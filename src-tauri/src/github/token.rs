//! Finding a GitHub token: environment first, then the GitHub CLI.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{AppError, AppResult};

/// Directories where `gh` usually lives. A GUI app launched from Finder
/// gets a minimal PATH, so these are searched after PATH.
const GH_FALLBACK_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

const GH_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) const NO_GH_MESSAGE: &str = "No GitHub token found. Install the GitHub CLI and run \
     `gh auth login` in a terminal, or set the GITHUB_TOKEN environment variable.";
pub(crate) const GH_FAILED_MESSAGE: &str = "The GitHub CLI is not signed in. Run `gh auth login` \
     in a terminal, or set the GITHUB_TOKEN environment variable.";

pub(crate) async fn resolve() -> AppResult<String> {
    if let Some(token) = token_from_env(|key| std::env::var(key).ok()) {
        return Ok(token);
    }
    let candidates = gh_candidates(
        std::env::var_os("PATH"),
        std::env::var_os("HOME").map(PathBuf::from),
    );
    let gh = candidates
        .into_iter()
        .find(|p| is_executable(p))
        .ok_or_else(|| AppError::NotSignedIn(NO_GH_MESSAGE.to_owned()))?;
    token_from_gh(&gh).await
}

/// `GH_TOKEN`, then `GITHUB_TOKEN`; blank values do not count.
pub(crate) fn token_from_env(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .filter_map(|key| get(key))
        .map(|v| v.trim().to_owned())
        .find(|v| !v.is_empty())
}

/// Every place `gh` might be, in search order, without duplicates.
pub(crate) fn gh_candidates(path_var: Option<OsString>, home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(GH_FALLBACK_DIRS.iter().map(PathBuf::from));
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join("gh");
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn is_executable(path: &Path) -> bool {
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

async fn token_from_gh(gh: &Path) -> AppResult<String> {
    let mut cmd = tokio::process::Command::new(gh);
    cmd.args(["auth", "token"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(GH_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            log::warn!("could not run {}: {e}", gh.display());
            return Err(AppError::NotSignedIn(NO_GH_MESSAGE.to_owned()));
        }
        Err(_) => {
            log::warn!("{} auth token timed out", gh.display());
            return Err(AppError::NotSignedIn(GH_FAILED_MESSAGE.to_owned()));
        }
    };
    // stdout holds the token: never log it.
    let token = String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if output.status.success() && !token.is_empty() {
        Ok(token)
    } else {
        log::info!("gh auth token exited with {}", output.status);
        Err(AppError::NotSignedIn(GH_FAILED_MESSAGE.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn env_prefers_gh_token_and_skips_blank() {
        let env: HashMap<&str, &str> = [("GH_TOKEN", "  "), ("GITHUB_TOKEN", "abc\n")].into();
        assert_eq!(
            token_from_env(|k| env.get(k).map(|v| v.to_string())),
            Some("abc".into())
        );
        let env: HashMap<&str, &str> = [("GH_TOKEN", "one"), ("GITHUB_TOKEN", "two")].into();
        assert_eq!(
            token_from_env(|k| env.get(k).map(|v| v.to_string())),
            Some("one".into())
        );
        assert_eq!(token_from_env(|_| None), None);
    }

    #[test]
    fn candidates_search_path_then_fallbacks() {
        let path = std::env::join_paths(["/custom/bin", "/usr/local/bin"]).unwrap();
        let got = gh_candidates(Some(path), Some(PathBuf::from("/home/me")));
        let want: Vec<PathBuf> = [
            "/custom/bin/gh",
            "/usr/local/bin/gh",
            "/opt/homebrew/bin/gh",
            "/usr/bin/gh",
            "/home/linuxbrew/.linuxbrew/bin/gh",
            "/home/me/.local/bin/gh",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn candidates_without_path_or_home() {
        let got = gh_candidates(None, None);
        assert_eq!(got.first(), Some(&PathBuf::from("/opt/homebrew/bin/gh")));
        assert_eq!(got.len(), GH_FALLBACK_DIRS.len());
    }

    #[test]
    fn messages_tell_the_user_what_to_do() {
        assert!(NO_GH_MESSAGE.contains("gh auth login"));
        assert!(NO_GH_MESSAGE.contains("GITHUB_TOKEN"));
        assert!(GH_FAILED_MESSAGE.contains("gh auth login"));
    }
}
