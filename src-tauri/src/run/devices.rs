//! Where an example app can run: iOS simulators and Android devices. The
//! parsers are pure and tested on captured output; the probes ask the tools
//! with short timeouts so the run dialog opens quickly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::model::{RunDevice, RunDeviceKind, RunPlatform, RunPlatformOptions};

use super::env::BuildEnv;
use super::host::{Cmd, Host};
use super::recipes::Tool;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

pub const AVD_PREFIX: &str = "avd:";
pub const ADB_PREFIX: &str = "adb:";

pub const MSG_NOT_MAC: &str = "iOS apps can only be built on a Mac.";
pub const MSG_NO_XCODE: &str = "Xcode is not installed.";
pub const MSG_NO_SIMULATOR: &str = "No iOS simulator is available.";
pub const MSG_NO_ANDROID_SDK: &str = "The Android SDK was not found. Set ANDROID_HOME.";
pub const MSG_NO_JAVA: &str = "Java 17 was not found.";
pub const MSG_NO_ANDROID_DEVICE: &str = "No Android device or emulator is set up.";

// ---- iOS ----

/// Available iPhone and iPad simulators from
/// `xcrun simctl list devices available --json`: booted first, then the
/// newest runtime, iPhones before iPads, then by name.
pub fn parse_simctl(json: &str) -> Option<Vec<RunDevice>> {
    let root: Value = serde_json::from_str(json).ok()?;
    let runtimes = root.get("devices")?.as_object()?;
    let mut found: Vec<(bool, Vec<u32>, bool, RunDevice)> = Vec::new();
    for (runtime, devices) in runtimes {
        let Some((label, version)) = runtime_label(runtime) else {
            continue;
        };
        for d in devices.as_array().into_iter().flatten() {
            let text = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
            let (udid, name, state, kind) = (
                text("udid"),
                text("name"),
                text("state"),
                text("deviceTypeIdentifier"),
            );
            if udid.is_empty() || name.is_empty() {
                continue;
            }
            if d.get("isAvailable").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let is_iphone = kind.contains("iPhone") || name.starts_with("iPhone");
            let is_ipad = kind.contains("iPad") || name.starts_with("iPad");
            if !is_iphone && !is_ipad {
                continue;
            }
            let running = state.eq_ignore_ascii_case("Booted");
            found.push((
                running,
                version.clone(),
                is_iphone,
                RunDevice {
                    id: udid.to_owned(),
                    name: name.to_owned(),
                    detail: Some(label.clone()),
                    kind: RunDeviceKind::Simulator,
                    running,
                },
            ));
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.name.cmp(&b.3.name))
    });
    Some(found.into_iter().map(|f| f.3).collect())
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-27-2` -> ("iOS 27.2", [27, 2]).
/// Only iOS runtimes.
fn runtime_label(id: &str) -> Option<(String, Vec<u32>)> {
    let rest = id.rsplit('.').next()?;
    let mut parts = rest.split('-');
    if parts.next()? != "iOS" {
        return None;
    }
    let version: Vec<u32> = parts.map_while(|p| p.parse().ok()).collect();
    if version.is_empty() {
        return None;
    }
    let dotted: Vec<String> = version.iter().map(u32::to_string).collect();
    Some((format!("iOS {}", dotted.join(".")), version))
}

pub async fn probe_ios<H: Host>(host: &H, env: &BuildEnv, requires: &[Tool]) -> RunPlatformOptions {
    let unavailable = |msg: &str, devices: Vec<RunDevice>| RunPlatformOptions {
        platform: RunPlatform::Ios,
        unavailable: Some(msg.to_owned()),
        devices,
    };
    // Whether this is a Mac is the runner's call (`ios_supported`).
    let xcrun = || Cmd::new("/usr/bin/xcrun").env(env.vars.clone());
    let find = host.output(xcrun().args(["--find", "xcodebuild"]), PROBE_TIMEOUT);
    let list = host.output(
        xcrun().args(["simctl", "list", "devices", "available", "--json"]),
        PROBE_TIMEOUT,
    );
    let (find, list) = tokio::join!(find, list);
    if !find.is_ok_and(|o| o.success()) {
        return unavailable(MSG_NO_XCODE, Vec::new());
    }
    let devices = match list {
        Ok(out) if out.success() => parse_simctl(&out.stdout).unwrap_or_default(),
        _ => Vec::new(),
    };
    if devices.is_empty() {
        return unavailable(MSG_NO_SIMULATOR, devices);
    }
    if let Some(tool) = missing_tool(env, requires) {
        return unavailable(tool.missing, devices);
    }
    RunPlatformOptions {
        platform: RunPlatform::Ios,
        unavailable: None,
        devices,
    }
}

/// Booted state of one simulator, asked fresh.
pub async fn simulator_booted<H: Host>(host: &H, env: &BuildEnv, udid: &str) -> Option<bool> {
    let cmd = Cmd::new("/usr/bin/xcrun").env(env.vars.clone()).args([
        "simctl",
        "list",
        "devices",
        "available",
        "--json",
    ]);
    let out = host.output(cmd, PROBE_TIMEOUT).await.ok()?;
    parse_simctl(&out.stdout)?
        .into_iter()
        .find(|d| d.id == udid)
        .map(|d| d.running)
}

// ---- Android ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    /// `device`, `offline`, `unauthorized`, ...
    pub state: String,
    pub model: Option<String>,
}

impl AdbDevice {
    pub fn is_emulator(&self) -> bool {
        self.serial.starts_with("emulator-")
    }
}

/// `adb devices -l`.
pub fn parse_adb_devices(text: &str) -> Vec<AdbDevice> {
    text.lines()
        .filter(|l| !l.starts_with("List of devices") && !l.starts_with('*'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let serial = words.next()?.to_owned();
            let state = words.next()?.to_owned();
            let model = words
                .find_map(|w| w.strip_prefix("model:"))
                .map(|m| m.replace('_', " "));
            Some(AdbDevice {
                serial,
                state,
                model,
            })
        })
        .collect()
}

/// `emulator -list-avds`: one AVD name per line, without the emulator's
/// own chatter.
pub fn parse_avd_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| is_avd_name(l))
        .map(str::to_owned)
        .collect()
}

pub fn is_avd_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// `adb -s emulator-5554 emu avd name`: the name, then `OK`.
pub fn parse_emu_avd_name(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .filter(|l| *l != "OK" && is_avd_name(l))
        .map(str::to_owned)
}

/// `key=value` lines of an AVD's `.ini` files.
pub fn parse_ini(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// `android-35` -> `API 35`.
fn api_label(target: &str) -> Option<String> {
    let level = target.strip_prefix("android-")?;
    (!level.is_empty()).then(|| format!("API {level}"))
}

/// Where AVDs are kept.
pub fn avd_home(env: &BuildEnv) -> Option<PathBuf> {
    let var = |k: &str| env.vars.get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("ANDROID_AVD_HOME")
        .or_else(|| var("ANDROID_USER_HOME").map(|p| p.join("avd")))
        .or_else(|| env.home().map(|h| h.join(".android/avd")))
}

/// What an AVD's files say: its API level and ABI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvdInfo {
    pub detail: Option<String>,
    pub abi: Option<String>,
}

pub fn avd_info(avd_home: &Path, name: &str) -> AvdInfo {
    let read = |p: PathBuf| std::fs::read_to_string(p).ok().map(|t| parse_ini(&t));
    let top = read(avd_home.join(format!("{name}.ini"))).unwrap_or_default();
    let dir = top
        .get("path")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| avd_home.join(format!("{name}.avd")));
    let config = read(dir.join("config.ini")).unwrap_or_default();
    AvdInfo {
        detail: top.get("target").and_then(|t| api_label(t)),
        abi: config.get("abi.type").cloned().filter(|s| !s.is_empty()),
    }
}

/// Android devices: the running ones (in adb's order) first, then the AVDs
/// that are not running.
pub fn android_devices(
    connected: &[AdbDevice],
    emulator_names: &HashMap<String, String>,
    avds: &[String],
    details: &HashMap<String, Option<String>>,
) -> Vec<RunDevice> {
    let mut out = Vec::new();
    let mut running_avds = Vec::new();
    for d in connected {
        if d.is_emulator() {
            let Some(name) = emulator_names.get(&d.serial) else {
                continue;
            };
            running_avds.push(name.clone());
            out.push(RunDevice {
                id: format!("{AVD_PREFIX}{name}"),
                name: name.clone(),
                detail: details.get(name).cloned().flatten(),
                kind: RunDeviceKind::Emulator,
                running: true,
            });
        } else if d.state == "device" {
            out.push(RunDevice {
                id: format!("{ADB_PREFIX}{}", d.serial),
                name: d.model.clone().unwrap_or_else(|| d.serial.clone()),
                detail: None,
                kind: RunDeviceKind::Device,
                running: true,
            });
        }
    }
    for name in avds {
        if running_avds.contains(name) {
            continue;
        }
        out.push(RunDevice {
            id: format!("{AVD_PREFIX}{name}"),
            name: name.clone(),
            detail: details.get(name).cloned().flatten(),
            kind: RunDeviceKind::Emulator,
            running: false,
        });
    }
    out
}

/// The Android side of the run dialog, plus the adb serial of each running
/// device by its `RunDevice::id`.
#[derive(Debug, Clone, PartialEq)]
pub struct AndroidProbe {
    pub options: RunPlatformOptions,
    pub serials: HashMap<String, String>,
}

pub async fn probe_android<H: Host>(host: &H, env: &BuildEnv, requires: &[Tool]) -> AndroidProbe {
    let fail = |msg: &str, devices: Vec<RunDevice>, serials| AndroidProbe {
        options: RunPlatformOptions {
            platform: RunPlatform::Android,
            unavailable: Some(msg.to_owned()),
            devices,
        },
        serials,
    };
    let Some(adb) = env.adb() else {
        return fail(MSG_NO_ANDROID_SDK, Vec::new(), HashMap::new());
    };
    let (devices, serials) = list_android(host, env, &adb).await;
    if env.java_home.is_none() {
        return fail(MSG_NO_JAVA, devices, serials);
    }
    if devices.is_empty() {
        return fail(MSG_NO_ANDROID_DEVICE, devices, serials);
    }
    if let Some(tool) = missing_tool(env, requires) {
        return fail(tool.missing, devices, serials);
    }
    AndroidProbe {
        options: RunPlatformOptions {
            platform: RunPlatform::Android,
            unavailable: None,
            devices,
        },
        serials,
    }
}

async fn list_android<H: Host>(
    host: &H,
    env: &BuildEnv,
    adb: &Path,
) -> (Vec<RunDevice>, HashMap<String, String>) {
    let adb_cmd = || Cmd::new(adb.to_string_lossy().into_owned()).env(env.vars.clone());
    let connected = host.output(adb_cmd().args(["devices", "-l"]), PROBE_TIMEOUT);
    let avds = async {
        match env.emulator() {
            Some(emu) => {
                let cmd = Cmd::new(emu.to_string_lossy().into_owned())
                    .env(env.vars.clone())
                    .arg("-list-avds");
                match host.output(cmd, PROBE_TIMEOUT).await {
                    Ok(out) => parse_avd_list(&out.stdout),
                    Err(_) => Vec::new(),
                }
            }
            None => Vec::new(),
        }
    };
    let (connected, avds) = tokio::join!(connected, avds);
    let connected = match connected {
        Ok(out) if out.success() => parse_adb_devices(&out.stdout),
        _ => Vec::new(),
    };
    let names = futures_join(connected.iter().filter(|d| d.is_emulator()).map(|d| {
        let cmd = adb_cmd().args(["-s", &d.serial, "emu", "avd", "name"]);
        let serial = d.serial.clone();
        async move {
            let name = host
                .output(cmd, PROBE_TIMEOUT)
                .await
                .ok()
                .and_then(|o| parse_emu_avd_name(&o.stdout));
            (serial, name)
        }
    }))
    .await;
    let emulator_names: HashMap<String, String> = names
        .into_iter()
        .filter_map(|(s, n)| n.map(|n| (s, n)))
        .collect();
    let home = avd_home(env);
    let mut details = HashMap::new();
    for name in avds.iter().chain(emulator_names.values()) {
        let info = home
            .as_deref()
            .map(|h| avd_info(h, name))
            .unwrap_or_default();
        details.insert(name.clone(), info.detail);
    }
    let devices = android_devices(&connected, &emulator_names, &avds, &details);
    let mut serials = HashMap::new();
    for (serial, name) in &emulator_names {
        serials.insert(format!("{AVD_PREFIX}{name}"), serial.clone());
    }
    for d in &connected {
        if !d.is_emulator() && d.state == "device" {
            serials.insert(format!("{ADB_PREFIX}{}", d.serial), d.serial.clone());
        }
    }
    (devices, serials)
}

/// Runs the futures concurrently and returns their results in order.
async fn futures_join<F: std::future::Future>(futs: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<_> = futs.map(|f| Some(Box::pin(f))).collect();
    let mut results: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut done = true;
        for (slot, result) in pending.iter_mut().zip(results.iter_mut()) {
            if let Some(f) = slot {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => {
                        *result = Some(v);
                        *slot = None;
                    }
                    std::task::Poll::Pending => done = false,
                }
            }
        }
        if done {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
    results.into_iter().flatten().collect()
}

/// The ABI name of a device as `ro.product.cpu.abi` reports it.
pub fn parse_abi(text: &str) -> Option<String> {
    let abi = text.trim();
    (!abi.is_empty()
        && abi
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| abi.to_owned())
}

pub fn missing_tool(env: &BuildEnv, requires: &[Tool]) -> Option<Tool> {
    requires
        .iter()
        .copied()
        .find(|t| env.find(t.program).is_none())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/run")
            .join(name);
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn simctl_fixture_sorts_booted_then_runtime_then_iphones() {
        let devices = parse_simctl(&fixture("simctl_devices.json")).unwrap();
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names[0], "iPhone 18 Pro Max");
        assert!(devices[0].running);
        assert_eq!(devices[0].id, "3A9B2AFC-68B4-41D3-A49A-07BEE3A0F52F");
        assert_eq!(devices[0].detail.as_deref(), Some("iOS 27.2"));
        assert_eq!(devices[0].kind, RunDeviceKind::Simulator);
        assert_eq!(devices.len(), 12);
        let first_ipad = names.iter().position(|n| n.starts_with("iPad")).unwrap();
        assert!(names[first_ipad..].iter().all(|n| n.starts_with("iPad")));
        assert!(devices[1..].iter().all(|d| !d.running));
    }

    #[test]
    fn simctl_multi_runtime_order_and_filtering() {
        let devices = parse_simctl(&fixture("simctl_devices_multi.json")).unwrap();
        let got: Vec<(&str, &str, bool)> = devices
            .iter()
            .map(|d| (d.name.as_str(), d.detail.as_deref().unwrap(), d.running))
            .collect();
        assert_eq!(
            got,
            vec![
                ("iPad Air", "iOS 17.5", true),
                ("iPhone 18", "iOS 27.2", false),
                ("iPad Pro", "iOS 27.2", false),
                ("iPhone 15", "iOS 17.5", false),
            ]
        );
        assert_eq!(parse_simctl("not json"), None);
    }

    #[test]
    fn adb_devices_fixture() {
        let devices = parse_adb_devices(&fixture("adb_devices_emulator.txt"));
        assert_eq!(
            devices,
            vec![AdbDevice {
                serial: "emulator-5554".into(),
                state: "device".into(),
                model: Some("sdk gphone64 arm64".into()),
            }]
        );
    }

    #[test]
    fn android_devices_running_first_and_physical_by_model() {
        let connected = parse_adb_devices(&fixture("adb_devices_mixed.txt"));
        assert_eq!(connected.len(), 4);
        let avds = parse_avd_list(&fixture("emulator_list_avds.txt"));
        assert_eq!(avds, vec!["palmatum_api35", "Pixel_9_API_36"]);
        let names: HashMap<String, String> =
            [("emulator-5554".to_owned(), "palmatum_api35".to_owned())].into();
        let details: HashMap<String, Option<String>> =
            [("palmatum_api35".to_owned(), Some("API 35".to_owned()))].into();
        let devices = android_devices(&connected, &names, &avds, &details);
        let got: Vec<(&str, &str, RunDeviceKind, bool)> = devices
            .iter()
            .map(|d| (d.id.as_str(), d.name.as_str(), d.kind, d.running))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "avd:palmatum_api35",
                    "palmatum_api35",
                    RunDeviceKind::Emulator,
                    true
                ),
                ("adb:1A2B3C4D5E6F", "Pixel 7", RunDeviceKind::Device, true),
                (
                    "avd:Pixel_9_API_36",
                    "Pixel_9_API_36",
                    RunDeviceKind::Emulator,
                    false
                ),
            ]
        );
        assert_eq!(devices[0].detail.as_deref(), Some("API 35"));
    }

    #[test]
    fn emu_avd_name_and_abi() {
        assert_eq!(
            parse_emu_avd_name("palmatum_api35\r\nOK\r\n").as_deref(),
            Some("palmatum_api35")
        );
        assert_eq!(parse_emu_avd_name("OK\r\n"), None);
        assert_eq!(parse_emu_avd_name("error: no emulator\n"), None);
        assert_eq!(parse_abi("arm64-v8a\r\n").as_deref(), Some("arm64-v8a"));
        assert_eq!(parse_abi(""), None);
    }

    #[test]
    fn avd_info_reads_both_ini_files() {
        let dir = tempfile::tempdir().unwrap();
        let avd_dir = dir.path().join("elsewhere/test.avd");
        std::fs::create_dir_all(&avd_dir).unwrap();
        std::fs::write(
            dir.path().join("test.ini"),
            format!(
                "avd.ini.encoding=UTF-8\npath={}\ntarget=android-35\n",
                avd_dir.display()
            ),
        )
        .unwrap();
        std::fs::write(
            avd_dir.join("config.ini"),
            "abi.type=arm64-v8a\nimage.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n",
        )
        .unwrap();
        assert_eq!(
            avd_info(dir.path(), "test"),
            AvdInfo {
                detail: Some("API 35".into()),
                abi: Some("arm64-v8a".into())
            }
        );
        assert_eq!(avd_info(dir.path(), "missing"), AvdInfo::default());
    }

    #[test]
    fn runtime_labels() {
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.iOS-27-2-iPhone19-4"),
            Some(("iOS 27.2".into(), vec![27, 2]))
        );
        assert_eq!(
            runtime_label("com.apple.CoreSimulator.SimRuntime.watchOS-11-0"),
            None
        );
    }
}
