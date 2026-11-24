//! The environment builds run in. Launched from Finder the app gets a
//! minimal PATH, so the user's login shell is asked for theirs, and what is
//! still missing (the Android SDK, Java) is filled in from the usual places.
//!
//! The environment can hold tokens: it is never logged, and the GitHub
//! tokens are removed before anything runs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::host::{Cmd, Host};

/// How long the login shell gets to print its environment.
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(6);
const JAVA_HOME_TIMEOUT: Duration = Duration::from_secs(3);

const MARKER: &str = "__PR_STATUS_ENV__";

/// Never passed to a build.
const DROPPED: &[&str] = &["GH_TOKEN", "GITHUB_TOKEN"];

/// Searched after the shell's PATH.
const FALLBACK_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildEnv {
    /// The complete environment for commands.
    pub vars: Arc<HashMap<String, String>>,
    pub android_home: Option<PathBuf>,
    pub java_home: Option<PathBuf>,
}

impl BuildEnv {
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        self.vars
            .get("PATH")
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default()
    }

    /// The first executable called `program` on PATH.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        self.path_dirs()
            .into_iter()
            .map(|d| d.join(program))
            .find(|p| is_executable(p))
    }

    pub fn adb(&self) -> Option<PathBuf> {
        self.android_tool("platform-tools/adb")
    }

    pub fn emulator(&self) -> Option<PathBuf> {
        self.android_tool("emulator/emulator")
    }

    fn android_tool(&self, rel: &str) -> Option<PathBuf> {
        let p = self.android_home.as_ref()?.join(rel);
        is_executable(&p).then_some(p)
    }

    pub fn home(&self) -> Option<PathBuf> {
        self.vars
            .get("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    }
}

/// What [`resolve`] works from; gathered by [`gather`], injected in tests.
#[derive(Debug, Clone, Default)]
pub struct EnvInputs {
    /// The login shell's environment, when it could be read.
    pub shell: Option<HashMap<String, String>>,
    /// The app's own environment.
    pub process: HashMap<String, String>,
    pub macos: bool,
    /// What `/usr/libexec/java_home -v 17` printed.
    pub java_home_tool: Option<String>,
}

/// Builds the environment. `exists` answers whether a path exists, so tests
/// need no real SDKs.
pub fn resolve(inputs: EnvInputs, exists: &dyn Fn(&Path) -> bool) -> BuildEnv {
    let EnvInputs {
        shell,
        process,
        macos,
        java_home_tool,
    } = inputs;
    let mut vars = shell.unwrap_or_else(|| process.clone());
    for key in DROPPED {
        vars.remove(*key);
    }
    let home = vars
        .get("HOME")
        .or_else(|| process.get("HOME"))
        .filter(|h| !h.is_empty())
        .cloned();
    if let Some(h) = &home {
        vars.insert("HOME".into(), h.clone());
    }
    let home = home.map(PathBuf::from);
    let usable = |v: Option<&String>| v.filter(|s| !s.trim().is_empty()).map(PathBuf::from);

    // PATH: the shell's, then the usual places.
    let mut dirs: Vec<PathBuf> = vars
        .get("PATH")
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    if let Some(h) = &home {
        dirs.push(h.join(".cargo/bin"));
    }
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));

    // The Android SDK.
    let mut sdk_candidates: Vec<PathBuf> = Vec::new();
    sdk_candidates.extend(usable(vars.get("ANDROID_HOME")));
    sdk_candidates.extend(usable(vars.get("ANDROID_SDK_ROOT")));
    if let Some(h) = &home {
        sdk_candidates.push(if macos {
            h.join("Library/Android/sdk")
        } else {
            h.join("Android/Sdk")
        });
    }
    let android_home = sdk_candidates.into_iter().find(|p| exists(p));
    if let Some(sdk) = &android_home {
        vars.insert("ANDROID_HOME".into(), sdk.to_string_lossy().into_owned());
        dirs.push(sdk.join("platform-tools"));
        dirs.push(sdk.join("emulator"));
    }

    // Java 17 for Gradle.
    let mut java_candidates: Vec<PathBuf> = Vec::new();
    java_candidates.extend(usable(vars.get("JAVA_HOME")));
    java_candidates.extend(usable(java_home_tool.map(|s| s.trim().to_owned()).as_ref()));
    for prefix in [
        "/opt/homebrew/opt",
        "/usr/local/opt",
        "/home/linuxbrew/.linuxbrew/opt",
    ] {
        for formula in ["openjdk@17", "openjdk"] {
            let base = Path::new(prefix).join(formula);
            java_candidates.push(base.join("libexec/openjdk.jdk/Contents/Home"));
            java_candidates.push(base.join("libexec"));
        }
    }
    if macos {
        java_candidates.push(PathBuf::from(
            "/Applications/Android Studio.app/Contents/jbr/Contents/Home",
        ));
        if let Some(h) = &home {
            java_candidates
                .push(h.join("Applications/Android Studio.app/Contents/jbr/Contents/Home"));
        }
    } else {
        java_candidates.push(PathBuf::from("/usr/lib/jvm/java-17-openjdk-amd64"));
        java_candidates.push(PathBuf::from("/usr/lib/jvm/java-17-openjdk"));
        java_candidates.push(PathBuf::from("/opt/android-studio/jbr"));
        if let Some(h) = &home {
            java_candidates.push(h.join("android-studio/jbr"));
        }
    }
    let java_home = java_candidates
        .into_iter()
        .find(|p| exists(&p.join("bin/java")));
    if let Some(j) = &java_home {
        vars.insert("JAVA_HOME".into(), j.to_string_lossy().into_owned());
    }

    let mut unique: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !d.as_os_str().is_empty() && !unique.contains(&d) {
            unique.push(d);
        }
    }
    if let Ok(joined) = std::env::join_paths(&unique) {
        vars.insert("PATH".into(), joined.to_string_lossy().into_owned());
    }
    // Ruby, Xcode's tools and Gradle all want a UTF-8 locale.
    vars.entry("LANG".into())
        .or_insert_with(|| "en_US.UTF-8".into());

    BuildEnv {
        vars: Arc::new(vars),
        android_home,
        java_home,
    }
}

/// Parses what `printf MARKER; env -0` printed. Anything an rc file printed
/// before the marker is ignored.
pub fn parse_env_dump(out: &str) -> Option<HashMap<String, String>> {
    let (_, rest) = out.split_once(MARKER)?;
    let vars: HashMap<String, String> = rest
        .split('\0')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            let ok = !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            ok.then(|| (k.to_owned(), v.to_owned()))
        })
        .collect();
    vars.contains_key("PATH").then_some(vars)
}

/// Asks the login shell and `java_home`, then resolves. Never fails: a
/// shell that does not answer means the app's own environment.
pub async fn gather<H: Host>(host: &H) -> BuildEnv {
    let process: HashMap<String, String> = std::env::vars().collect();
    let macos = cfg!(target_os = "macos");
    let shell = process
        .get("SHELL")
        .filter(|s| s.starts_with('/'))
        .cloned()
        .unwrap_or_else(|| if macos { "/bin/zsh" } else { "/bin/bash" }.to_owned());
    let script = format!("printf '%s' '{MARKER}'; /usr/bin/env -0");
    // Interactive and login, so both .zprofile and .zshrc are read.
    let dump = Cmd::new(shell).args(["-l", "-i", "-c", &script]);
    let shell_env = match host.output(dump, SHELL_TIMEOUT).await {
        Ok(out) => parse_env_dump(&out.stdout),
        Err(e) => {
            log::info!("run: login shell environment unavailable: {e}");
            None
        }
    };
    if shell_env.is_none() {
        log::info!("run: using the app's own environment for builds");
    }
    let java_home_tool = if macos && Path::new("/usr/libexec/java_home").exists() {
        let cmd = Cmd::new("/usr/libexec/java_home").args(["-v", "17"]);
        match host.output(cmd, JAVA_HOME_TIMEOUT).await {
            Ok(out) if out.success() => Some(out.stdout.trim().to_owned()),
            _ => None,
        }
    } else {
        None
    };
    resolve(
        EnvInputs {
            shell: shell_env,
            process,
            macos,
            java_home_tool,
        },
        &|p| p.exists(),
    )
}

pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use pretty_assertions::assert_eq;

    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn existing(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p: &Path| set.contains(p)
    }

    #[test]
    fn env_dump_ignores_rc_noise_and_needs_path() {
        let out =
            format!("Welcome!\n{MARKER}PATH=/a:/b\0HOME=/Users/me\0WEIRD\0=x\0MULTI=one\ntwo\0");
        let vars = parse_env_dump(&out).unwrap();
        assert_eq!(vars.get("PATH").unwrap(), "/a:/b");
        assert_eq!(vars.get("MULTI").unwrap(), "one\ntwo");
        assert_eq!(vars.len(), 3);
        assert_eq!(parse_env_dump("no marker"), None);
        assert_eq!(parse_env_dump(&format!("{MARKER}HOME=/x\0")), None);
    }

    #[test]
    fn shell_env_wins_and_tokens_are_dropped() {
        let env = resolve(
            EnvInputs {
                shell: Some(map(&[
                    ("PATH", "/Users/me/.local/bin:/usr/bin"),
                    ("HOME", "/Users/me"),
                    ("GH_TOKEN", "secret"),
                    ("GITHUB_TOKEN", "secret"),
                    ("LANG", "de_DE.UTF-8"),
                ])),
                process: map(&[("PATH", "/usr/bin"), ("HOME", "/Users/me")]),
                macos: true,
                java_home_tool: None,
            },
            &existing(&[]),
        );
        assert!(!env.vars.contains_key("GH_TOKEN"));
        assert!(!env.vars.contains_key("GITHUB_TOKEN"));
        assert_eq!(env.vars.get("LANG").unwrap(), "de_DE.UTF-8");
        let dirs: Vec<String> = env
            .path_dirs()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            dirs,
            vec![
                "/Users/me/.local/bin",
                "/usr/bin",
                "/Users/me/.cargo/bin",
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/bin",
                "/usr/sbin",
                "/sbin",
            ]
        );
        assert_eq!(env.android_home, None);
        assert_eq!(env.java_home, None);
    }

    #[test]
    fn without_a_shell_the_app_environment_is_used() {
        let env = resolve(
            EnvInputs {
                shell: None,
                process: map(&[
                    ("PATH", "/usr/bin:/bin"),
                    ("HOME", "/Users/me"),
                    ("GITHUB_TOKEN", "t"),
                ]),
                macos: true,
                java_home_tool: None,
            },
            &existing(&[]),
        );
        assert!(
            env.path_dirs()
                .starts_with(&[PathBuf::from("/usr/bin"), PathBuf::from("/bin")])
        );
        assert!(
            env.path_dirs()
                .contains(&PathBuf::from("/Users/me/.cargo/bin"))
        );
        assert!(!env.vars.contains_key("GITHUB_TOKEN"));
        assert_eq!(env.vars.get("LANG").unwrap(), "en_US.UTF-8");
    }

    #[test]
    fn android_sdk_and_java_are_filled_in() {
        let sdk = "/Users/me/Library/Android/sdk";
        let jdk = "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home";
        let env = resolve(
            EnvInputs {
                shell: Some(map(&[("PATH", "/usr/bin"), ("HOME", "/Users/me")])),
                process: HashMap::new(),
                macos: true,
                java_home_tool: None,
            },
            &existing(&[sdk, &format!("{jdk}/bin/java")]),
        );
        assert_eq!(env.android_home, Some(PathBuf::from(sdk)));
        assert_eq!(env.vars.get("ANDROID_HOME").unwrap(), sdk);
        assert_eq!(env.java_home, Some(PathBuf::from(jdk)));
        assert_eq!(env.vars.get("JAVA_HOME").unwrap(), jdk);
        let dirs = env.path_dirs();
        assert!(dirs.contains(&PathBuf::from(format!("{sdk}/platform-tools"))));
        assert!(dirs.contains(&PathBuf::from(format!("{sdk}/emulator"))));
    }

    #[test]
    fn set_values_beat_defaults_but_only_when_they_exist() {
        let env = resolve(
            EnvInputs {
                shell: Some(map(&[
                    ("PATH", "/usr/bin"),
                    ("HOME", "/home/me"),
                    ("ANDROID_HOME", "/gone"),
                    ("ANDROID_SDK_ROOT", "/sdk"),
                    ("JAVA_HOME", "/jdk-broken"),
                ])),
                process: HashMap::new(),
                macos: false,
                java_home_tool: Some("/ignored-on-linux-but-valid\n".into()),
            },
            &existing(&[
                "/sdk",
                "/home/me/Android/Sdk",
                "/ignored-on-linux-but-valid/bin/java",
            ]),
        );
        assert_eq!(env.android_home, Some(PathBuf::from("/sdk")));
        assert_eq!(
            env.java_home,
            Some(PathBuf::from("/ignored-on-linux-but-valid"))
        );

        let env = resolve(
            EnvInputs {
                shell: Some(map(&[("PATH", "/usr/bin"), ("HOME", "/home/me")])),
                process: HashMap::new(),
                macos: false,
                java_home_tool: None,
            },
            &existing(&[
                "/home/me/Android/Sdk",
                "/home/me/android-studio/jbr/bin/java",
            ]),
        );
        assert_eq!(
            env.android_home,
            Some(PathBuf::from("/home/me/Android/Sdk"))
        );
        assert_eq!(
            env.java_home,
            Some(PathBuf::from("/home/me/android-studio/jbr"))
        );
    }

    #[test]
    fn java_home_tool_comes_before_homebrew() {
        let env = resolve(
            EnvInputs {
                shell: Some(map(&[("PATH", "/usr/bin"), ("HOME", "/Users/me")])),
                process: HashMap::new(),
                macos: true,
                java_home_tool: Some(
                    "/Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home\n".into(),
                ),
            },
            &existing(&[
                "/Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home/bin/java",
                "/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home/bin/java",
            ]),
        );
        assert_eq!(
            env.java_home,
            Some(PathBuf::from(
                "/Library/Java/JavaVirtualMachines/zulu-17.jdk/Contents/Home"
            ))
        );
    }
}
