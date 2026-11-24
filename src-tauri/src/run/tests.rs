//! Whole jobs on fakes: a scripted host, recorded events, a paused clock.
//! Nothing here starts a real build, simulator, emulator or network call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::env::BuildEnv;
use super::host::{CancelSignal, Captured, Cmd, Exit, Host, LineSink, cancelled};
use super::source::{Clock, Lookups, RunEvents};
use super::*;
use crate::error::AppResult;
use crate::model::{RunLog, RunState, StepStatus};

// ---- Fakes ----

#[derive(Clone)]
enum Act {
    Lines(Vec<String>),
    Sleep(u64),
    WaitCancel,
    Hook(Arc<dyn Fn() + Send + Sync>),
}

struct RunRule {
    pat: String,
    acts: Vec<Act>,
    exit: Exit,
}

struct OutRule {
    pat: String,
    /// Answers in turn; the last one repeats.
    answers: Vec<Result<Captured, String>>,
    asked: usize,
}

#[derive(Default)]
struct FakeHost {
    runs: Mutex<Vec<RunRule>>,
    outs: Mutex<Vec<OutRule>>,
    calls: Mutex<Vec<String>>,
}

impl FakeHost {
    fn on_run(&self, pat: &str, acts: Vec<Act>, exit: Exit) {
        self.runs.lock().unwrap().insert(
            0,
            RunRule {
                pat: pat.into(),
                acts,
                exit,
            },
        );
    }

    fn on_output(&self, pat: &str, answers: Vec<Result<Captured, String>>) {
        self.outs.lock().unwrap().insert(
            0,
            OutRule {
                pat: pat.into(),
                answers,
                asked: 0,
            },
        );
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Host for FakeHost {
    async fn run(&self, cmd: Cmd, out: LineSink, mut cancel: CancelSignal) -> Exit {
        let line = cmd.to_string();
        self.calls.lock().unwrap().push(format!("run {line}"));
        let rule = {
            let runs = self.runs.lock().unwrap();
            runs.iter()
                .find(|r| line.contains(&r.pat))
                .map(|r| (r.acts.clone(), r.exit.clone()))
        };
        let Some((acts, exit)) = rule else {
            return Exit::Code(0);
        };
        for act in acts {
            match act {
                Act::Lines(lines) => lines.into_iter().for_each(|l| out.push(l)),
                Act::Sleep(ms) => {
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_millis(ms)) => {}
                        () = cancelled(&mut cancel) => return Exit::Cancelled,
                    }
                }
                Act::WaitCancel => {
                    cancelled(&mut cancel).await;
                    return Exit::Cancelled;
                }
                Act::Hook(f) => f(),
            }
        }
        exit
    }

    async fn output(&self, cmd: Cmd, _timeout: Duration) -> AppResult<Captured> {
        let line = cmd.to_string();
        self.calls.lock().unwrap().push(format!("out {line}"));
        let mut outs = self.outs.lock().unwrap();
        let Some(rule) = outs.iter_mut().find(|r| line.contains(&r.pat)) else {
            return Err(crate::error::AppError::Other(format!("no rule for {line}")));
        };
        let i = rule.asked.min(rule.answers.len() - 1);
        rule.asked += 1;
        rule.answers[i]
            .clone()
            .map_err(crate::error::AppError::Other)
    }

    fn spawn_detached(&self, cmd: Cmd, _log: Option<PathBuf>) -> AppResult<()> {
        self.calls.lock().unwrap().push(format!("spawn {cmd}"));
        Ok(())
    }
}

/// Forwards to a shared host so the test keeps a handle on it.
#[derive(Clone)]
struct SharedHost(Arc<FakeHost>);

impl Host for SharedHost {
    async fn run(&self, cmd: Cmd, out: LineSink, cancel: CancelSignal) -> Exit {
        self.0.run(cmd, out, cancel).await
    }
    async fn output(&self, cmd: Cmd, timeout: Duration) -> AppResult<Captured> {
        self.0.output(cmd, timeout).await
    }
    fn spawn_detached(&self, cmd: Cmd, log: Option<PathBuf>) -> AppResult<()> {
        self.0.spawn_detached(cmd, log)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Job(RunJob),
    Log(RunLog),
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Event>>>);

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
    fn jobs(&self) -> Vec<RunJob> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Job(j) => Some(j),
                Event::Log(_) => None,
            })
            .collect()
    }
    fn log_batches(&self) -> Vec<Vec<String>> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Log(l) => Some(l.lines),
                Event::Job(_) => None,
            })
            .collect()
    }
}

impl RunEvents for Recorder {
    fn job(&self, job: &RunJob) {
        self.0.lock().unwrap().push(Event::Job(job.clone()));
    }
    fn log(&self, log: &RunLog) {
        self.0.lock().unwrap().push(Event::Log(log.clone()));
    }
}

struct PausedClock {
    base: DateTime<Utc>,
    start: tokio::time::Instant,
}

impl Clock for PausedClock {
    fn now(&self) -> DateTime<Utc> {
        let elapsed = chrono::Duration::from_std(self.start.elapsed()).unwrap_or_default();
        self.base + elapsed
    }
}

struct FakeLookups {
    workspaces: Vec<PathBuf>,
    env: BuildEnv,
}

impl Lookups for FakeLookups {
    async fn t3_workspaces(&self) -> Vec<PathBuf> {
        self.workspaces.clone()
    }
    async fn build_env(&self) -> BuildEnv {
        self.env.clone()
    }
}

// ---- The world ----

const REPO: &str = "dashpay/platform";
const NUMBER: u64 = 6002;
const HEAD: &str = "9cc7778487200a5f379f0cb47cfff854e1a520dd";
const FETCHED: &str = "aaaa778487200a5f379f0cb47cfff854e1a520dd";
const BRANCH: &str = "fix/dashmate-local";
const SIM: &str = "BBBB0000-0000-0000-0000-000000000002";

type TestRunner = Runner<SharedHost, Recorder, PausedClock, FakeLookups>;

struct World {
    _tmp: TempDir,
    root: PathBuf,
    clone: PathBuf,
    worktree: PathBuf,
    host: Arc<FakeHost>,
    events: Recorder,
    runner: TestRunner,
}

fn touch(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn executable(path: &Path) {
    touch(path, "#!/bin/sh\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A source tree with both example apps.
fn source_tree(root: &Path, ios: bool, android: bool) {
    if ios {
        touch(&root.join("packages/swift-sdk/build_ios.sh"), "");
        std::fs::create_dir_all(
            root.join("packages/swift-sdk/SwiftExampleApp/SwiftExampleApp.xcodeproj"),
        )
        .unwrap();
    }
    if android {
        touch(&root.join("packages/kotlin-sdk/build_android.sh"), "");
        touch(&root.join("packages/kotlin-sdk/gradlew"), "");
        touch(
            &root.join("packages/kotlin-sdk/KotlinExampleApp/app/build.gradle.kts"),
            "android {\n  defaultConfig {\n    applicationId = \"org.example.app\"\n  }\n}\n",
        );
    }
}

fn ok(s: &str) -> Result<Captured, String> {
    Ok(Captured::ok(s))
}

fn porcelain(clone: &Path, worktree: &Path, checkout: &Path) -> String {
    format!(
        "worktree {}\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/master\n\n\
         worktree {}\nHEAD {HEAD}\nbranch refs/heads/{BRANCH}\n\n\
         worktree {}\nHEAD {HEAD}\ndetached\n\n",
        clone.display(),
        worktree.display(),
        checkout.display()
    )
}

fn world_with(ios_supported: bool) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let clone = root.join("src/platform");
    let worktree = root.join("src/platform-wt");
    source_tree(&clone, true, true);
    source_tree(&worktree, true, true);

    let bin = root.join("bin");
    executable(&bin.join("cargo"));
    executable(&bin.join("cargo-ndk"));
    let sdk = root.join("sdk");
    executable(&sdk.join("platform-tools/adb"));
    executable(&sdk.join("emulator/emulator"));
    let home = root.join("home");
    touch(
        &home.join(".android/avd/stopped_avd.ini"),
        "target=android-35\n",
    );
    touch(
        &home.join(".android/avd/stopped_avd.avd/config.ini"),
        "abi.type=x86_64\n",
    );
    let vars: HashMap<String, String> = [
        ("PATH".to_owned(), bin.to_string_lossy().into_owned()),
        ("HOME".to_owned(), home.to_string_lossy().into_owned()),
    ]
    .into();
    let env = BuildEnv {
        vars: Arc::new(vars),
        android_home: Some(sdk),
        java_home: Some(root.join("jdk")),
    };

    let paths = RunPaths {
        cache_dir: root.join("cache"),
        settings_file: root.join("config/run.json"),
    };
    let checkout = paths.checkout(REPO);
    // The built iOS app, where Xcode would put it.
    for source in [
        job::PlannedSource::PrHead {
            clone: clone.clone(),
            checkout: checkout.clone(),
        },
        job::PlannedSource::Worktree {
            path: worktree.clone(),
        },
    ] {
        let app = paths
            .derived(REPO, &source)
            .join("Build/Products/Debug-iphonesimulator/SwiftExampleApp.app");
        touch(&app.join("Info.plist"), "");
    }

    let host = Arc::new(FakeHost::default());
    host.on_output(
        "remote get-url origin",
        vec![ok("git@github.com:dashpay/platform.git\n")],
    );
    host.on_output(
        "worktree list --porcelain",
        vec![ok(&porcelain(&clone, &worktree, &checkout))],
    );
    host.on_output("status --porcelain", vec![ok("")]);
    host.on_output("rev-parse --show-toplevel", vec![Err("not a repo".into())]);
    host.on_output("rev-parse HEAD", vec![ok(&format!("{FETCHED}\n"))]);
    host.on_output("rev-parse FETCH_HEAD", vec![ok(&format!("{FETCHED}\n"))]);
    host.on_output(
        "--find xcodebuild",
        vec![ok("/Applications/Xcode.app/xcodebuild\n")],
    );
    host.on_output(
        "simctl list devices available --json",
        vec![ok(&std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/run/simctl_devices_multi.json"),
        )
        .unwrap())],
    );
    host.on_output("plutil", vec![ok("org.example.app\n")]);
    host.on_output(
        "devices -l",
        vec![ok(
            "List of devices attached\nemulator-5554 device product:x model:sdk transport_id:1\n\n",
        )],
    );
    host.on_output("-list-avds", vec![ok("running_avd\nstopped_avd\n")]);
    host.on_output("emu avd name", vec![ok("running_avd\r\nOK\r\n")]);
    host.on_output("getprop ro.product.cpu.abi", vec![ok("arm64-v8a\n")]);
    host.on_output("getprop sys.boot_completed", vec![ok("1\n")]);
    host.on_output(
        "resolve-activity",
        vec![ok("priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\norg.example.app/.MainActivity\n")],
    );
    host.on_output(
        "am start",
        vec![ok(
            "Starting: Intent { cmp=org.example.app/.MainActivity }\n",
        )],
    );

    let events = Recorder::default();
    let runner = Runner::new(Deps {
        host: SharedHost(Arc::clone(&host)),
        events: events.clone(),
        clock: PausedClock {
            base: Utc.with_ymd_and_hms(2026, 11, 22, 10, 0, 0).unwrap(),
            start: tokio::time::Instant::now(),
        },
        lookups: FakeLookups {
            workspaces: vec![root.join("src/elsewhere"), clone.clone()],
            env,
        },
        paths,
        ios_supported,
    });
    World {
        _tmp: tmp,
        root,
        clone,
        worktree,
        host,
        events,
        runner,
    }
}

fn world() -> World {
    world_with(true)
}

impl World {
    fn pr(&self) -> PrTarget {
        PrTarget {
            repo: REPO.into(),
            number: NUMBER,
            head_sha: HEAD.into(),
            head_branch: Some(BRANCH.into()),
        }
    }

    fn checkout(&self) -> PathBuf {
        self.runner.paths().checkout(REPO)
    }

    /// `git worktree add` makes the checkout, with the given apps.
    fn checkout_gets(&self, ios: bool, android: bool) {
        let checkout = self.checkout();
        self.host.on_run(
            "worktree add",
            vec![Act::Hook(Arc::new(move || {
                std::fs::create_dir_all(&checkout).unwrap();
                source_tree(&checkout, ios, android);
            }))],
            Exit::Code(0),
        );
    }

    fn request(&self, platform: RunPlatform, kind: RunSourceKind, device: &str) -> RunRequest {
        RunRequest {
            id: format!("{REPO}#{NUMBER}"),
            platform,
            source_kind: kind,
            source_path: (kind == RunSourceKind::Worktree)
                .then(|| self.worktree.to_string_lossy().into_owned()),
            device_id: device.into(),
        }
    }

    async fn run_to_end(&self, req: RunRequest) -> RunJob {
        self.runner.start(&self.pr(), &req).await.unwrap();
        self.runner.wait().await;
        self.runner.job().unwrap()
    }

    /// The streamed commands, by the part that names them.
    fn ran(&self) -> Vec<String> {
        const NAMES: &[&str] = &[
            "worktree add",
            "fetch --no-tags origin pull/6002/head",
            "checkout --detach --force FETCH_HEAD",
            "submodule update",
            "build_ios.sh --target sim",
            "build_android.sh --abi arm64",
            "build_android.sh --abi x86_64",
            "xcodebuild",
            "simctl bootstatus",
            "simctl boot",
            "open -a Simulator",
            "simctl install",
            "simctl launch",
            "gradlew --no-daemon --console=plain :app:mergeDebugNativeLibs :app:installDebug",
            "-avd stopped_avd -port 5556",
        ];
        self.host
            .calls()
            .iter()
            .filter(|c| c.starts_with("run ") || c.starts_with("spawn "))
            .map(|c| {
                NAMES
                    .iter()
                    .find(|n| c.contains(*n))
                    .map(|n| (*n).to_owned())
                    .unwrap_or_else(|| c.clone())
            })
            .collect()
    }
}

/// Each step's status in every emitted job, as "key:status" strings.
fn transitions(jobs: &[RunJob]) -> Vec<String> {
    let mut out = Vec::new();
    let mut last: HashMap<String, StepStatus> = HashMap::new();
    for job in jobs {
        for step in &job.steps {
            if last.get(&step.key) != Some(&step.status) {
                last.insert(step.key.clone(), step.status);
                if step.status != StepStatus::Pending
                    || out
                        .iter()
                        .any(|s: &String| s.starts_with(&format!("{}:", step.key)))
                {
                    out.push(format!("{}:{:?}", step.key, step.status));
                }
            }
        }
    }
    out
}

fn keys(job: &RunJob) -> Vec<&str> {
    job.steps.iter().map(|s| s.key.as_str()).collect()
}

// ---- Jobs ----

#[tokio::test(start_paused = true)]
async fn ios_pr_head_runs_every_step_in_order() {
    let w = world();
    w.checkout_gets(true, true);
    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM);
    let started = w.runner.start(&w.pr(), &req).await.unwrap();
    assert_eq!(started.state, RunState::Running);
    assert_eq!(started.job_id, 1);
    assert_eq!(started.sha, None);
    assert_eq!(started.device_name, "iPhone 18");
    assert_eq!(
        keys(&started),
        vec![
            "checkout", "buildSdk", "buildApp", "device", "install", "open"
        ]
    );
    assert!(
        started
            .steps
            .iter()
            .all(|s| s.status == StepStatus::Pending)
    );
    assert_eq!(started.steps[0].label, "Get the code");
    assert_eq!(started.steps[3].label, "Start the simulator");

    w.runner.wait().await;
    let job = w.runner.job().unwrap();
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    assert_eq!(job.sha.as_deref(), Some(FETCHED));
    assert!(job.finished_at.is_some());
    assert!(job.steps.iter().all(|s| s.status == StepStatus::Done));
    assert_eq!(
        w.ran(),
        vec![
            "worktree add",
            "fetch --no-tags origin pull/6002/head",
            "checkout --detach --force FETCH_HEAD",
            "build_ios.sh --target sim",
            "xcodebuild",
            "simctl boot",
            "open -a Simulator",
            "simctl bootstatus",
            "simctl install",
            "simctl launch",
        ]
    );
    assert_eq!(
        transitions(&w.events.jobs()),
        vec![
            "checkout:Running",
            "checkout:Done",
            "buildSdk:Running",
            "buildSdk:Done",
            "buildApp:Running",
            "buildApp:Done",
            "device:Running",
            "device:Done",
            "install:Running",
            "install:Done",
            "open:Running",
            "open:Done",
        ]
    );
    // The app is built for the chosen simulator, into the app's own
    // derived data.
    let calls = w.host.calls();
    let xcode = calls
        .iter()
        .find(|c| c.contains("xcodebuild -project"))
        .unwrap();
    assert!(
        xcode.contains(&format!("'platform=iOS Simulator,id={SIM}'")),
        "{xcode}"
    );
    assert!(xcode.contains("cache/xcode/dashpay-platform"), "{xcode}");
    let launch = calls.iter().find(|c| c.contains("simctl launch")).unwrap();
    assert!(
        launch.ends_with(&format!("{SIM} org.example.app")),
        "{launch}"
    );
    // The worktree add is for the app's checkout, from the user's clone.
    let add = calls.iter().find(|c| c.contains("worktree add")).unwrap();
    assert!(
        add.contains(&format!(
            "-C {} worktree add --detach {}",
            w.clone.display(),
            w.checkout().display()
        )),
        "{add}"
    );
    // The last event is the finished job, after every log line.
    assert!(
        matches!(w.events.events().last(), Some(Event::Job(j)) if j.state == RunState::Succeeded)
    );
}

#[tokio::test(start_paused = true)]
async fn an_existing_checkout_is_reused_and_submodules_are_updated() {
    let w = world();
    let checkout = w.checkout();
    source_tree(&checkout, true, false);
    touch(&checkout.join(".git"), "gitdir: elsewhere\n");
    touch(&checkout.join(".gitmodules"), "");
    w.host.on_output(
        "rev-parse --show-toplevel",
        vec![ok(&format!("{}\n", checkout.display()))],
    );
    // At an older commit before the fetch, then at the fetched one.
    w.host.on_output(
        "rev-parse HEAD",
        vec![ok(&format!("{HEAD}\n")), ok(&format!("{FETCHED}\n"))],
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM))
        .await;
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    let ran = w.ran();
    assert_eq!(
        &ran[..3],
        &[
            "fetch --no-tags origin pull/6002/head",
            "checkout --detach --force FETCH_HEAD",
            "submodule update",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn android_worktree_on_a_running_emulator() {
    let w = world();
    let job = w
        .run_to_end(w.request(
            RunPlatform::Android,
            RunSourceKind::Worktree,
            "avd:running_avd",
        ))
        .await;
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    assert_eq!(job.sha.as_deref(), Some(HEAD));
    assert_eq!(keys(&job), vec!["buildSdk", "device", "install", "open"]);
    assert_eq!(job.steps[1].label, "Start the emulator");
    assert_eq!(job.steps[2].label, "Build and install the app");
    assert_eq!(
        w.ran(),
        vec![
            "build_android.sh --abi arm64",
            "gradlew --no-daemon --console=plain :app:mergeDebugNativeLibs :app:installDebug",
        ]
    );
    let calls = w.host.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("-s emulator-5554 shell am start -n org.example.app/.MainActivity"))
    );
    // No git command that writes ran for a worktree.
    assert!(
        !calls.iter().any(|c| c.contains("worktree add")
            || c.contains("fetch")
            || c.contains("checkout --"))
    );
    assert_eq!(
        transitions(&w.events.jobs()),
        vec![
            "buildSdk:Running",
            "buildSdk:Done",
            "device:Running",
            "device:Done",
            "install:Running",
            "install:Done",
            "open:Running",
            "open:Done",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn android_pr_head_starts_a_stopped_emulator_and_waits_for_it() {
    let w = world();
    w.checkout_gets(false, true);
    w.host.on_output(
        "getprop sys.boot_completed",
        vec![Err("device offline".into()), ok("\n"), ok("1\n")],
    );
    let before = tokio::time::Instant::now();
    let job = w
        .run_to_end(w.request(
            RunPlatform::Android,
            RunSourceKind::PrHead,
            "avd:stopped_avd",
        ))
        .await;
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    assert_eq!(
        keys(&job),
        vec!["checkout", "buildSdk", "device", "install", "open"]
    );
    assert_eq!(
        w.ran(),
        vec![
            "worktree add",
            "fetch --no-tags origin pull/6002/head",
            "checkout --detach --force FETCH_HEAD",
            // A stopped AVD's ABI comes from its config.ini.
            "build_android.sh --abi x86_64",
            // emulator-5554 is taken, so the next even port.
            "-avd stopped_avd -port 5556",
            "gradlew --no-daemon --console=plain :app:mergeDebugNativeLibs :app:installDebug",
        ]
    );
    let calls = w.host.calls();
    let gradle = calls.iter().position(|c| c.contains("gradlew")).unwrap();
    let boots = calls
        .iter()
        .filter(|c| c.contains("-s emulator-5556 shell getprop sys.boot_completed"))
        .count();
    assert_eq!(boots, 3);
    assert!(
        calls
            .iter()
            .rposition(|c| c.contains("boot_completed"))
            .unwrap()
            < gradle
    );
    // Two polls two seconds apart.
    assert!(before.elapsed() >= Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn a_pr_head_without_the_app_fails_the_first_step() {
    let w = world();
    w.checkout_gets(true, false);
    let job = w
        .run_to_end(w.request(
            RunPlatform::Android,
            RunSourceKind::PrHead,
            "avd:running_avd",
        ))
        .await;
    assert_eq!(job.state, RunState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("This commit has no Android example app.")
    );
    assert_eq!(job.steps[0].status, StepStatus::Failed);
    assert!(
        job.steps[1..]
            .iter()
            .all(|s| s.status == StepStatus::Pending)
    );
    assert!(!w.ran().iter().any(|c| c.contains("build_android")));
}

#[tokio::test(start_paused = true)]
async fn a_failing_step_ends_the_job() {
    let w = world();
    w.host.on_run(
        "xcodebuild",
        vec![Act::Lines(vec!["error: no such module".into()])],
        Exit::Code(65),
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM))
        .await;
    assert_eq!(job.state, RunState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("Build the app failed (exit code 65).")
    );
    let status: Vec<StepStatus> = job.steps.iter().map(|s| s.status).collect();
    assert_eq!(
        status,
        vec![
            StepStatus::Done,
            StepStatus::Failed,
            StepStatus::Pending,
            StepStatus::Pending,
            StepStatus::Pending,
        ]
    );
    assert!(job.steps[1].finished_at.is_some());
    assert_eq!(w.ran().last().map(String::as_str), Some("xcodebuild"));
    let log = w.runner.log();
    assert!(log.contains(&"error: no such module".to_owned()));
    assert!(log.contains(&"Build the app failed (exit code 65).".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn a_command_that_cannot_start_is_a_sentence() {
    let w = world();
    w.host.on_run(
        "build_ios.sh",
        vec![],
        Exit::NotStarted("Could not start ./build_ios.sh: permission was denied.".into()),
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM))
        .await;
    assert_eq!(job.state, RunState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("Could not start ./build_ios.sh: permission was denied.")
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_in_the_middle_stops_the_step_and_runs_nothing_more() {
    let w = world();
    w.host.on_run(
        "build_ios.sh",
        vec![Act::Lines(vec!["Compiling".into()]), Act::WaitCancel],
        Exit::Code(0),
    );
    w.runner
        .start(
            &w.pr(),
            &w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM),
        )
        .await
        .unwrap();
    // Let it reach the SDK build.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(w.runner.job().unwrap().steps[0].status, StepStatus::Running);
    w.runner.cancel();
    w.runner.wait().await;
    let job = w.runner.job().unwrap();
    assert_eq!(job.state, RunState::Cancelled);
    assert_eq!(job.error, None);
    assert!(job.steps.iter().all(|s| s.status == StepStatus::Pending));
    assert!(job.steps[0].started_at.is_some() && job.steps[0].finished_at.is_some());
    assert_eq!(w.ran(), vec!["build_ios.sh --target sim"]);
    assert_eq!(
        w.runner.log().last().map(String::as_str),
        Some("==> Stopped.")
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_while_waiting_for_the_emulator() {
    let w = world();
    w.host
        .on_output("getprop sys.boot_completed", vec![ok("\n")]);
    w.runner
        .start(
            &w.pr(),
            &w.request(
                RunPlatform::Android,
                RunSourceKind::Worktree,
                "avd:stopped_avd",
            ),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(w.runner.job().unwrap().steps[1].status, StepStatus::Running);
    w.runner.cancel();
    w.runner.wait().await;
    assert_eq!(w.runner.job().unwrap().state, RunState::Cancelled);
    assert!(!w.ran().iter().any(|c| c.contains("gradlew")));
}

#[tokio::test(start_paused = true)]
async fn an_emulator_that_never_boots_fails_after_the_timeout() {
    let w = world();
    w.host
        .on_output("getprop sys.boot_completed", vec![ok("\n")]);
    let job = w
        .run_to_end(w.request(
            RunPlatform::Android,
            RunSourceKind::Worktree,
            "avd:stopped_avd",
        ))
        .await;
    assert_eq!(job.state, RunState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("The emulator did not finish starting within 6 minutes.")
    );
}

#[tokio::test(start_paused = true)]
async fn only_one_job_at_a_time() {
    let w = world();
    w.host
        .on_run("build_ios.sh", vec![Act::WaitCancel], Exit::Code(0));
    let req = w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM);
    w.runner.start(&w.pr(), &req).await.unwrap();
    let err = w.runner.start(&w.pr(), &req).await.unwrap_err();
    assert_eq!(err.to_string(), MSG_BUSY);
    w.runner.cancel();
    w.runner.wait().await;
    // After it ends, the next one starts with a higher id, and the old job
    // stays readable until then.
    assert_eq!(w.runner.job().unwrap().job_id, 1);
    w.host.on_run("build_ios.sh", vec![], Exit::Code(0));
    let next = w.runner.start(&w.pr(), &req).await.unwrap();
    assert_eq!(next.job_id, 2);
    w.runner.wait().await;
    assert_eq!(w.runner.job().unwrap().state, RunState::Succeeded);
}

#[tokio::test(start_paused = true)]
async fn log_lines_arrive_in_batches_in_order() {
    let w = world();
    let first: Vec<String> = (0..500).map(|i| format!("a{i}")).collect();
    let second: Vec<String> = (0..300).map(|i| format!("b{i}")).collect();
    w.host.on_run(
        "build_ios.sh",
        vec![
            Act::Lines(first.clone()),
            Act::Sleep(1000),
            Act::Lines(second.clone()),
            Act::Sleep(1000),
        ],
        Exit::Code(0),
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM))
        .await;
    assert_eq!(job.state, RunState::Succeeded);
    let batches = w.events.log_batches();
    // A handful of batches, never one per line.
    assert!(batches.len() <= 6, "{} batches", batches.len());
    assert!(batches.iter().any(|b| b.len() >= 500));
    let all: Vec<String> = batches.concat();
    let pos = |s: &str| all.iter().position(|l| l == s).unwrap();
    assert!(pos("a0") < pos("a499") && pos("a499") < pos("b0") && pos("b0") < pos("b299"));
    assert_eq!(all, w.runner.log());
    // Every batch belongs to this job.
    assert!(w.events.events().iter().all(|e| match e {
        Event::Log(l) => l.job_id == 1,
        Event::Job(j) => j.job_id == 1,
    }));
    // Each step starts with a line naming it and its command.
    let i = pos("==> Build the SDK");
    assert!(
        all[i + 1].starts_with("$ ./build_ios.sh --target sim    (in "),
        "{}",
        all[i + 1]
    );
}

#[tokio::test(start_paused = true)]
async fn the_log_keeps_the_last_2000_lines_and_the_whole_file() {
    let w = world();
    let lines: Vec<String> = (0..2600)
        .map(|i| format!("\u{1b}[32mline {i}\u{1b}[0m"))
        .collect();
    w.host
        .on_run("build_ios.sh", vec![Act::Lines(lines)], Exit::Code(0));
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM))
        .await;
    let log = w.runner.log();
    assert_eq!(log.len(), 2000);
    assert!(!log.iter().any(|l| l.contains('\u{1b}')));
    let file = std::fs::read_to_string(job.log_path.unwrap()).unwrap();
    assert!(file.contains("line 0\n"));
    assert!(file.contains("line 2599\n"));
    assert!(file.starts_with("Run dashpay/platform#6002 on iPhone 18 (iOS), from the worktree"));
    assert!(
        w.runner
            .log_path()
            .unwrap()
            .starts_with(w.root.join("cache/logs"))
    );
}

// ---- Validation in run_start ----

#[tokio::test(start_paused = true)]
async fn start_rejects_what_options_would_not_offer() {
    let w = world();
    let pr = w.pr();
    let msg = |r: AppResult<RunJob>| r.unwrap_err().to_string();

    let mut req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM);
    req.id = "dashpay/platform#1".into();
    assert_eq!(
        msg(w.runner.start(&pr, &req).await),
        "The request does not match the pull request."
    );

    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, "NOPE");
    assert_eq!(
        msg(w.runner.start(&pr, &req).await),
        "That device is not available any more. Choose another."
    );

    // The user's main tree is on another branch: not a source for this PR.
    let mut req = w.request(RunPlatform::Ios, RunSourceKind::Worktree, SIM);
    req.source_path = Some(w.clone.to_string_lossy().into_owned());
    assert_eq!(
        msg(w.runner.start(&pr, &req).await),
        "That worktree is not on this pull request's branch."
    );
    // The app's own checkout is never a worktree source either.
    req.source_path = Some(w.checkout().to_string_lossy().into_owned());
    assert!(w.runner.start(&pr, &req).await.is_err());
    req.source_path = None;
    assert_eq!(
        msg(w.runner.start(&pr, &req).await),
        "Choose a worktree to build."
    );

    // A physical device id that adb does not list.
    let req = w.request(RunPlatform::Android, RunSourceKind::PrHead, "adb:GONE");
    assert!(w.runner.start(&pr, &req).await.is_err());
    // An Android id for iOS.
    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, "avd:running_avd");
    assert!(w.runner.start(&pr, &req).await.is_err());

    assert!(w.runner.job().is_none(), "nothing started");
    assert!(w.ran().is_empty());
}

#[tokio::test(start_paused = true)]
async fn start_rejects_a_worktree_without_the_app() {
    let w = world();
    std::fs::remove_dir_all(w.worktree.join("packages/kotlin-sdk")).unwrap();
    let req = w.request(
        RunPlatform::Android,
        RunSourceKind::Worktree,
        "avd:running_avd",
    );
    assert_eq!(
        w.runner.start(&w.pr(), &req).await.unwrap_err().to_string(),
        "This worktree has no Android example app."
    );
}

#[tokio::test(start_paused = true)]
async fn start_rejects_an_unavailable_platform() {
    let w = world();
    w.host
        .on_output("--find xcodebuild", vec![Err("not found".into())]);
    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM);
    assert_eq!(
        w.runner.start(&w.pr(), &req).await.unwrap_err().to_string(),
        "Xcode is not installed."
    );

    let w = world_with(false);
    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM);
    assert_eq!(
        w.runner.start(&w.pr(), &req).await.unwrap_err().to_string(),
        "iOS apps can only be built on a Mac."
    );
}

#[tokio::test(start_paused = true)]
async fn start_needs_a_known_clone() {
    let w = world();
    w.host.on_output(
        "remote get-url origin",
        vec![ok("git@github.com:someone/else.git\n")],
    );
    let req = w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM);
    assert_eq!(
        w.runner.start(&w.pr(), &req).await.unwrap_err().to_string(),
        "No local clone of dashpay/platform is known. Choose its folder first."
    );
}

// ---- The run dialog ----

#[tokio::test(start_paused = true)]
async fn options_list_worktrees_first_then_the_pr_head() {
    let w = world();
    let extra = w.root.join("src/platform-old");
    std::fs::create_dir_all(&extra).unwrap();
    let text = format!(
        "worktree {}\nHEAD 2222222222222222222222222222222222222222\nbranch refs/heads/{BRANCH}\n\n{}",
        extra.display(),
        porcelain(&w.clone, &w.worktree, &w.checkout())
    );
    w.host
        .on_output("worktree list --porcelain", vec![ok(&text)]);
    w.host.on_output(
        &format!("-C {} status --porcelain", extra.display()),
        vec![ok(" M Cargo.toml\n")],
    );
    let o = w.runner.options(&w.pr()).await.unwrap();
    assert_eq!(o.id, "dashpay/platform#6002");
    assert_eq!(o.clone_path, w.clone.to_string_lossy());
    let got: Vec<(RunSourceKind, String, bool, bool)> = o
        .sources
        .iter()
        .map(|s| (s.kind, s.path.clone(), s.at_pr_head, s.dirty))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                RunSourceKind::Worktree,
                w.worktree.to_string_lossy().into_owned(),
                true,
                false
            ),
            (
                RunSourceKind::Worktree,
                extra.to_string_lossy().into_owned(),
                false,
                true
            ),
            (
                RunSourceKind::PrHead,
                w.checkout().to_string_lossy().into_owned(),
                true,
                false
            ),
        ]
    );
    assert_eq!(o.sources[0].branch.as_deref(), Some(BRANCH));
    assert_eq!(o.sources[2].sha, HEAD);
    assert_eq!(o.sources[2].branch, None);

    let platforms: Vec<RunPlatform> = o.platforms.iter().map(|p| p.platform).collect();
    assert_eq!(platforms, vec![RunPlatform::Ios, RunPlatform::Android]);
    assert_eq!(o.platforms[0].unavailable, None);
    assert_eq!(o.platforms[0].devices[0].name, "iPad Air");
    let android: Vec<(&str, bool)> = o.platforms[1]
        .devices
        .iter()
        .map(|d| (d.id.as_str(), d.running))
        .collect();
    assert_eq!(
        android,
        vec![("avd:running_avd", true), ("avd:stopped_avd", false)]
    );
    assert_eq!(o.platforms[1].devices[1].detail.as_deref(), Some("API 35"));
}

#[tokio::test(start_paused = true)]
async fn options_explain_what_is_missing() {
    let w = world();
    std::fs::remove_file(w.root.join("bin/cargo-ndk")).unwrap();
    w.host.on_output(
        "simctl list devices available --json",
        vec![ok("{\"devices\":{}}")],
    );
    let o = w.runner.options(&w.pr()).await.unwrap();
    assert_eq!(
        o.platforms[0].unavailable.as_deref(),
        Some("No iOS simulator is available.")
    );
    assert_eq!(
        o.platforms[1].unavailable.as_deref(),
        Some("cargo-ndk was not found. Install it with \"cargo install cargo-ndk\".")
    );
    // On Linux iOS is not offered at all.
    let w = world_with(false);
    let o = w.runner.options(&w.pr()).await.unwrap();
    let platforms: Vec<RunPlatform> = o.platforms.iter().map(|p| p.platform).collect();
    assert_eq!(platforms, vec![RunPlatform::Android]);
}

// ---- Clones ----

#[tokio::test(start_paused = true)]
async fn targets_find_the_clone_through_t3_then_the_users_choice() {
    let w = world();
    let projects = vec![REPO.to_owned(), "dashpay/grovedb".to_owned()];
    let t = w.runner.targets(&projects).await;
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].clone_path.as_deref(), Some(w.clone.to_str().unwrap()));
    assert!(!t[0].clone_is_set);
    assert_eq!(t[0].platforms, vec![RunPlatform::Ios, RunPlatform::Android]);
    assert_eq!(t[1].clone_path, None);
    assert!(t[1].platforms.is_empty());

    // The user picks a subfolder of another clone: its top is stored.
    let mine = w.root.join("src/mine");
    source_tree(&mine, false, true);
    std::fs::create_dir_all(mine.join("packages")).unwrap();
    w.host.on_output(
        &format!(
            "-C {} rev-parse --show-toplevel",
            mine.join("packages").display()
        ),
        vec![ok(&format!("{}\n", mine.display()))],
    );
    let t = w
        .runner
        .set_clone(
            &projects,
            "DashPay/Platform",
            Some(mine.join("packages").to_str().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(t[0].clone_path.as_deref(), Some(mine.to_str().unwrap()));
    assert!(t[0].clone_is_set);
    assert_eq!(t[0].platforms, vec![RunPlatform::Android]);
    let saved = std::fs::read_to_string(w.root.join("config/run.json")).unwrap();
    assert!(saved.contains(mine.to_str().unwrap()));

    // Forgetting it goes back to T3 Code's project.
    let t = w.runner.set_clone(&projects, REPO, None).await.unwrap();
    assert_eq!(t[0].clone_path.as_deref(), Some(w.clone.to_str().unwrap()));
    assert!(!t[0].clone_is_set);
}

#[tokio::test(start_paused = true)]
async fn set_clone_rejects_the_wrong_repository() {
    let w = world();
    let projects = vec![REPO.to_owned()];
    let other = w.root.join("src/other");
    std::fs::create_dir_all(&other).unwrap();
    w.host.on_output(
        &format!("-C {} rev-parse --show-toplevel", other.display()),
        vec![ok(&format!("{}\n", other.display()))],
    );
    w.host.on_output(
        &format!("-C {} remote get-url origin", other.display()),
        vec![ok("https://github.com/someone/fork.git\n")],
    );
    let err = w
        .runner
        .set_clone(&projects, REPO, Some(other.to_str().unwrap()))
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "That folder is a clone of someone/fork, not dashpay/platform."
    );
    assert!(!w.root.join("config/run.json").exists());
    let err = w
        .runner
        .set_clone(&projects, "someone/untracked", None)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "someone/untracked is not one of your projects."
    );
}

#[test]
fn paths_are_per_project_and_per_worktree() {
    let paths = RunPaths {
        cache_dir: PathBuf::from("/cache"),
        settings_file: PathBuf::from("/config/run.json"),
    };
    assert_eq!(
        paths.checkout("DashPay/Platform"),
        PathBuf::from("/cache/checkouts/dashpay-platform")
    );
    let a = paths.derived(REPO, &job::PlannedSource::Worktree { path: "/a".into() });
    let b = paths.derived(REPO, &job::PlannedSource::Worktree { path: "/b".into() });
    assert_ne!(a, b);
    assert!(a.starts_with("/cache/xcode"));
    assert_eq!(
        paths.derived(
            REPO,
            &job::PlannedSource::PrHead {
                clone: "/c".into(),
                checkout: "/k".into()
            }
        ),
        PathBuf::from("/cache/xcode/dashpay-platform")
    );
}

#[tokio::test(start_paused = true)]
async fn an_app_without_a_launcher_activity_fails_to_open() {
    let w = world();
    w.host
        .on_output("resolve-activity", vec![ok("No activity found\n")]);
    let job = w
        .run_to_end(w.request(
            RunPlatform::Android,
            RunSourceKind::Worktree,
            "avd:running_avd",
        ))
        .await;
    assert_eq!(job.state, RunState::Failed);
    assert_eq!(
        job.error.as_deref(),
        Some("Open the app failed: org.example.app has no launcher activity on the device.")
    );
}

#[test]
fn launcher_component_parsing() {
    let text = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\norg.dashfoundation.example/.MainActivity\n";
    assert_eq!(
        job::launcher_component(text, "org.dashfoundation.example").as_deref(),
        Some("org.dashfoundation.example/.MainActivity")
    );
    assert_eq!(job::launcher_component("No activity found\n", "a.b"), None);
    assert_eq!(job::launcher_component("other.app/.Main\n", "a.b"), None);
}

#[tokio::test(start_paused = true)]
async fn a_checkout_already_at_the_commit_is_left_alone() {
    let w = world();
    let checkout = w.checkout();
    source_tree(&checkout, true, false);
    touch(&checkout.join(".git"), "gitdir: elsewhere\n");
    w.host.on_output(
        "rev-parse --show-toplevel",
        vec![ok(&format!("{}\n", checkout.display()))],
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM))
        .await;
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    assert_eq!(job.sha.as_deref(), Some(FETCHED));
    assert!(!w.ran().iter().any(|c| c.contains("checkout --detach")));
    assert!(
        w.runner
            .log()
            .contains(&"The checkout is already at that commit.".to_owned())
    );
    // The clean check never takes git's optional locks.
    let calls = w.host.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.contains("status --porcelain --untracked-files=no"))
    );

    // With changes to tracked files (a build script edited one), it checks out.
    let w = world();
    let checkout = w.checkout();
    source_tree(&checkout, true, false);
    touch(&checkout.join(".git"), "gitdir: elsewhere\n");
    w.host.on_output(
        "rev-parse --show-toplevel",
        vec![ok(&format!("{}\n", checkout.display()))],
    );
    w.host.on_output(
        &format!("-C {} status --porcelain", checkout.display()),
        vec![ok(" M packages/swift-sdk/Package.swift\n")],
    );
    let job = w
        .run_to_end(w.request(RunPlatform::Ios, RunSourceKind::PrHead, SIM))
        .await;
    assert_eq!(job.state, RunState::Succeeded, "{:?}", job.error);
    assert!(w.ran().iter().any(|c| c.contains("checkout --detach")));
}
