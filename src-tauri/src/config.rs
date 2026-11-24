//! The user's tracked projects, followed PRs and tracked contributors,
//! persisted as JSON.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::model::{Config, pr_id};

/// Reads and writes the config file. Takes its path so tests can use a
/// temp dir; the app passes `<app config dir>/config.json`.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// A missing file is an empty config, not an error.
    pub fn load(&self) -> AppResult<Config> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(config_error(&self.path, &e)),
        };
        serde_json::from_str(&text).map_err(|e| {
            AppError::Config(format!(
                "{} is not valid JSON ({e}). Fix or delete the file.",
                self.path.display()
            ))
        })
    }

    /// Writes atomically (temp file + rename), creating parent dirs.
    pub fn save(&self, config: &Config) -> AppResult<()> {
        let dir = match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        fs::create_dir_all(&dir).map_err(|e| config_error(&dir, &e))?;

        let mut json = serde_json::to_string_pretty(config)
            .map_err(|e| AppError::Config(format!("could not encode the config: {e}")))?;
        json.push('\n');

        let file_name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.json".to_owned());
        let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));

        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            fs::rename(&tmp, &self.path)
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            config_error(&self.path, &e)
        })
    }
}

fn config_error(path: &Path, e: &std::io::Error) -> AppError {
    AppError::Config(format!("{}: {e}", path.display()))
}

/// Accepts `owner/name`, a github.com repo URL, or a URL to anything inside
/// the repo. Returns `owner/name`.
pub fn parse_repo(input: &str) -> AppResult<String> {
    let bad = || {
        AppError::InvalidInput(format!(
            "\"{}\" is not a repository. Use owner/name or a GitHub link.",
            input.trim()
        ))
    };
    let (segments, from_url) = split_input(input).ok_or_else(bad)?;
    let take = if from_url {
        segments.len().min(2)
    } else {
        segments.len()
    };
    if take != 2 {
        return Err(bad());
    }
    repo_from(segments[0], segments[1]).ok_or_else(bad)
}

/// Accepts `owner/name#123` or a github.com pull request URL.
pub fn parse_pr_ref(input: &str) -> AppResult<(String, u64)> {
    let bad = || {
        AppError::InvalidInput(format!(
            "\"{}\" is not a pull request. Use owner/name#123 or a pull request link.",
            input.trim()
        ))
    };
    let trimmed = input.trim();

    if !looks_like_url(trimmed) {
        // `owner/name#123`
        let (repo, number) = trimmed.split_once('#').ok_or_else(bad)?;
        let repo = parse_repo(repo).map_err(|_| bad())?;
        let number = parse_number(number).ok_or_else(bad)?;
        return Ok((repo, number));
    }

    let (segments, _) = split_input(trimmed).ok_or_else(bad)?;
    match segments.as_slice() {
        [owner, name, "pull", number, ..] => {
            let repo = repo_from(owner, name).ok_or_else(bad)?;
            let number = parse_number(number).ok_or_else(bad)?;
            Ok((repo, number))
        }
        _ => Err(bad()),
    }
}

/// Returns false when the project was already there (compared case-insensitively).
pub fn add_project(config: &mut Config, repo: &str) -> bool {
    if config.projects.iter().any(|p| p.eq_ignore_ascii_case(repo)) {
        return false;
    }
    config.projects.push(repo.to_owned());
    true
}

/// Also drops the followed PRs that belong to the removed project.
pub fn remove_project(config: &mut Config, repo: &str) -> bool {
    let before = config.projects.len();
    config.projects.retain(|p| !p.eq_ignore_ascii_case(repo));
    config.followed.retain(|id| match id.rsplit_once('#') {
        Some((id_repo, _)) => !id_repo.eq_ignore_ascii_case(repo),
        None => true,
    });
    config.projects.len() != before
}

/// Returns false when the PR was already followed (compared case-insensitively).
pub fn follow(config: &mut Config, repo: &str, number: u64) -> bool {
    let id = pr_id(repo, number);
    if config.followed.iter().any(|f| f.eq_ignore_ascii_case(&id)) {
        return false;
    }
    config.followed.push(id);
    true
}

/// Returns false when the PR was not followed.
pub fn unfollow(config: &mut Config, repo: &str, number: u64) -> bool {
    let id = pr_id(repo, number);
    let before = config.followed.len();
    config.followed.retain(|f| !f.eq_ignore_ascii_case(&id));
    config.followed.len() != before
}

/// Most contributors one config may track. Each tracked login widens the
/// per-project search; past about ten typical logins the search is split in
/// two (see `github::search`), and every extra search costs a request per
/// project on every sync, as do their PRs' findings.
pub const MAX_CONTRIBUTORS: usize = 20;

/// Accepts `login`, `@login`, or a github.com profile link (trailing slash
/// and extra path allowed). Returns the login as typed; GitHub's casing is
/// looked up later.
pub fn parse_login(input: &str) -> AppResult<String> {
    let trimmed = input.trim();
    let bad = || {
        AppError::InvalidInput(format!(
            "\"{trimmed}\" is not a GitHub username. Use a username, @username or a GitHub profile link."
        ))
    };
    let candidate = if looks_like_url(trimmed) {
        let (segments, _) = split_input(trimmed).ok_or_else(bad)?;
        // `git@github.com:owner/...` is a repo, not a profile.
        if trimmed.to_ascii_lowercase().starts_with("git@") {
            return Err(bad());
        }
        segments.first().copied().ok_or_else(bad)?
    } else {
        trimmed.strip_prefix('@').unwrap_or(trimmed)
    };
    if let Some(app) = candidate.strip_suffix("[bot]")
        && is_login(app)
    {
        return Err(AppError::InvalidInput(format!(
            "{candidate} is a GitHub App, not a person. Only people's pull requests can be tracked."
        )));
    }
    if is_login(candidate) {
        Ok(candidate.to_owned())
    } else {
        Err(bad())
    }
}

/// GitHub's username rules: 1 to 39 characters, ASCII letters, digits and
/// single hyphens, not starting or ending with a hyphen.
pub fn is_login(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Tracks `login` (already in GitHub's casing). Returns false when it was
/// already tracked (compared case-insensitively). Refuses past
/// `MAX_CONTRIBUTORS`.
pub fn track_contributor(config: &mut Config, login: &str) -> AppResult<bool> {
    if config
        .contributors
        .iter()
        .any(|c| c.eq_ignore_ascii_case(login))
    {
        return Ok(false);
    }
    if config.contributors.len() >= MAX_CONTRIBUTORS {
        return Err(too_many_contributors());
    }
    config.contributors.push(login.to_owned());
    Ok(true)
}

/// Returns false when `login` was not tracked.
pub fn untrack_contributor(config: &mut Config, login: &str) -> bool {
    let before = config.contributors.len();
    config
        .contributors
        .retain(|c| !c.eq_ignore_ascii_case(login.trim()));
    config.contributors.len() != before
}

pub fn too_many_contributors() -> AppError {
    AppError::InvalidInput(format!(
        "You can track at most {MAX_CONTRIBUTORS} contributors."
    ))
}

fn looks_like_url(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("github.com/")
        || lower.starts_with("www.github.com/")
        || lower.starts_with("git@github.com:")
}

/// Splits input into path segments. The flag says whether it was a GitHub
/// link (where trailing segments such as `/pull/12` are allowed). Returns
/// `None` for links to hosts other than github.com.
fn split_input(input: &str) -> Option<(Vec<&str>, bool)> {
    let s = input.trim();
    let lower = s.to_ascii_lowercase();
    let (path, from_url) = if let Some(rest) = lower.strip_prefix("git@github.com:") {
        (&s[s.len() - rest.len()..], true)
    } else if looks_like_url(s) {
        let after_scheme = match lower.find("://") {
            Some(i) => &s[i + 3..],
            None => s,
        };
        let (host, path) = after_scheme.split_once('/').unwrap_or((after_scheme, ""));
        let host = host.to_ascii_lowercase();
        if host != "github.com" && host != "www.github.com" {
            return None;
        }
        (path, true)
    } else {
        (s, false)
    };
    // Drop query string and fragment.
    let path = path.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<&str> = path.split('/').filter(|seg| !seg.is_empty()).collect();
    Some((segments, from_url))
}

fn repo_from(owner: &str, name: &str) -> Option<String> {
    let name = name.strip_suffix(".git").unwrap_or(name);
    let owner_ok = !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let name_ok = !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (owner_ok && name_ok).then(|| format!("{owner}/{name}"))
}

fn parse_number(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn cfg(projects: &[&str], followed: &[&str]) -> Config {
        Config {
            projects: projects.iter().map(|s| s.to_string()).collect(),
            followed: followed.iter().map(|s| s.to_string()).collect(),
            contributors: vec![],
        }
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().join("nope/config.json"));
        assert_eq!(store.load().unwrap(), Config::default());
    }

    #[test]
    fn load_corrupt_file_names_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let err = ConfigStore::new(path.clone()).load().unwrap_err();
        match err {
            AppError::Config(msg) => assert!(msg.contains(&path.display().to_string()), "{msg}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn load_tolerates_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"projects":["a/b"]}"#).unwrap();
        assert_eq!(ConfigStore::new(path).load().unwrap(), cfg(&["a/b"], &[]));
    }

    #[test]
    fn save_creates_dirs_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/config.json");
        let store = ConfigStore::new(path.clone());
        let config = cfg(&["dashpay/platform"], &["dashpay/platform#12"]);
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"projects\""));
        // Overwrite works and leaves no temp file behind.
        store.save(&Config::default()).unwrap();
        assert_eq!(store.load().unwrap(), Config::default());
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.json")]);
    }

    #[test]
    fn parse_repo_accepts_common_forms() {
        let ok = [
            "dashpay/platform",
            "  dashpay/platform  ",
            "dashpay/platform/",
            "dashpay/platform.git",
            "github.com/dashpay/platform",
            "www.github.com/dashpay/platform",
            "https://github.com/dashpay/platform",
            "http://github.com/dashpay/platform",
            "https://github.com/dashpay/platform/",
            "https://github.com/dashpay/platform.git",
            "https://GitHub.com/dashpay/platform/pull/12",
            "https://github.com/dashpay/platform/issues",
            "https://github.com/dashpay/platform/tree/main/src?x=1#L3",
            "https://github.com/dashpay/platform#readme",
            "git@github.com:dashpay/platform.git",
        ];
        for input in ok {
            assert_eq!(parse_repo(input).unwrap(), "dashpay/platform", "{input}");
        }
        assert_eq!(
            parse_repo("Some-Org/My_Repo.rs").unwrap(),
            "Some-Org/My_Repo.rs"
        );
    }

    #[test]
    fn parse_repo_rejects_junk() {
        let bad = [
            "",
            "foo",
            "foo/",
            "/foo",
            "a/b/c",
            "https://github.com/dashpay",
            "https://gitlab.com/dashpay/platform",
            "https://github.com.evil.com/dashpay/platform",
            "owner/na me",
            "own er/name",
            "-owner/name",
            "owner/..",
        ];
        for input in bad {
            let err = parse_repo(input).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "{input}: {err:?}");
        }
        assert_eq!(
            parse_repo(" foo ").unwrap_err().to_string(),
            "\"foo\" is not a repository. Use owner/name or a GitHub link."
        );
    }

    #[test]
    fn parse_pr_ref_accepts_common_forms() {
        let ok = [
            "dashpay/platform#6038",
            " dashpay/platform#6038 ",
            "https://github.com/dashpay/platform/pull/6038",
            "https://github.com/dashpay/platform/pull/6038/",
            "https://github.com/dashpay/platform/pull/6038/files",
            "https://github.com/dashpay/platform/pull/6038/commits/abc123",
            "https://github.com/dashpay/platform/pull/6038#issuecomment-123",
            "https://github.com/dashpay/platform/pull/6038/files?w=1",
            "github.com/dashpay/platform/pull/6038",
        ];
        for input in ok {
            assert_eq!(
                parse_pr_ref(input).unwrap(),
                ("dashpay/platform".to_string(), 6038),
                "{input}"
            );
        }
    }

    #[test]
    fn parse_pr_ref_rejects_junk() {
        let bad = [
            "",
            "dashpay/platform",
            "dashpay/platform#",
            "dashpay/platform#0",
            "dashpay/platform#12a",
            "dashpay/platform#-1",
            "#12",
            "https://github.com/dashpay/platform",
            "https://github.com/dashpay/platform/issues/12",
            "https://github.com/dashpay/platform/pull/",
            "https://github.com/dashpay/platform/pull/abc",
            "https://example.com/dashpay/platform/pull/12",
        ];
        for input in bad {
            let err = parse_pr_ref(input).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "{input}: {err:?}");
        }
        assert_eq!(
            parse_pr_ref("foo").unwrap_err().to_string(),
            "\"foo\" is not a pull request. Use owner/name#123 or a pull request link."
        );
    }

    #[test]
    fn add_project_is_case_insensitive_and_keeps_casing() {
        let mut c = Config::default();
        assert!(add_project(&mut c, "DashPay/Platform"));
        assert!(!add_project(&mut c, "dashpay/platform"));
        assert!(add_project(&mut c, "dashpay/dips"));
        assert_eq!(c.projects, vec!["DashPay/Platform", "dashpay/dips"]);
    }

    #[test]
    fn remove_project_drops_its_followed_prs() {
        let mut c = cfg(
            &["dashpay/platform", "dashpay/dips"],
            &[
                "DashPay/Platform#1",
                "dashpay/dips#2",
                "dashpay/platform-x#3",
            ],
        );
        assert!(remove_project(&mut c, "dashpay/PLATFORM"));
        assert_eq!(
            c,
            cfg(
                &["dashpay/dips"],
                &["dashpay/dips#2", "dashpay/platform-x#3"]
            )
        );
        assert!(!remove_project(&mut c, "dashpay/platform"));
    }

    #[test]
    fn parse_login_accepts_common_forms() {
        let ok = [
            "shumkov",
            "  shumkov ",
            "@shumkov",
            " @shumkov",
            "https://github.com/shumkov",
            "https://github.com/shumkov/",
            "http://www.github.com/shumkov",
            "github.com/shumkov",
            "https://GitHub.com/shumkov?tab=repositories",
            "https://github.com/shumkov/platform/pulls",
            "https://github.com/shumkov#top",
        ];
        for input in ok {
            assert_eq!(parse_login(input).unwrap(), "shumkov", "{input}");
        }
        // Casing is kept as typed; GitHub's is looked up later.
        assert_eq!(parse_login("Quantum-Explorer").unwrap(), "Quantum-Explorer");
        assert_eq!(parse_login("a").unwrap(), "a");
        assert_eq!(parse_login(&"a".repeat(39)).unwrap(), "a".repeat(39));
        assert_eq!(parse_login("0x1-2").unwrap(), "0x1-2");
    }

    #[test]
    fn parse_login_rejects_junk() {
        let bad = [
            "",
            "   ",
            "@",
            "-shumkov",
            "shumkov-",
            "shu--mkov",
            "shu_mkov",
            "shu.mkov",
            "shu mkov",
            "@@shumkov",
            "dashpay/platform",
            "https://github.com/",
            "https://gitlab.com/shumkov",
            "https://github.com.evil.com/shumkov",
            "git@github.com:shumkov/platform.git",
            "[bot]",
        ];
        for input in bad {
            let err = parse_login(input).unwrap_err();
            assert!(matches!(err, AppError::InvalidInput(_)), "{input}: {err:?}");
        }
        assert!(parse_login(&"a".repeat(40)).is_err());
        assert_eq!(
            parse_login(" shu mkov ").unwrap_err().to_string(),
            "\"shu mkov\" is not a GitHub username. Use a username, @username or a GitHub profile link."
        );
        for input in ["coderabbitai[bot]", "@github-actions[bot]"] {
            let msg = parse_login(input).unwrap_err().to_string();
            assert!(msg.contains("is a GitHub App, not a person"), "{msg}");
        }
    }

    #[test]
    fn track_and_untrack_contributors() {
        let mut c = Config::default();
        assert!(track_contributor(&mut c, "Shumkov").unwrap());
        assert!(!track_contributor(&mut c, "shumkov").unwrap());
        assert!(track_contributor(&mut c, "lklimek").unwrap());
        assert_eq!(c.contributors, vec!["Shumkov", "lklimek"]);
        assert!(untrack_contributor(&mut c, "SHUMKOV"));
        assert!(!untrack_contributor(&mut c, "shumkov"));
        assert!(!untrack_contributor(&mut c, "nobody"));
        assert_eq!(c.contributors, vec!["lklimek"]);
    }

    #[test]
    fn tracking_stops_at_the_maximum() {
        let mut c = Config::default();
        for i in 0..MAX_CONTRIBUTORS {
            assert!(track_contributor(&mut c, &format!("user{i}")).unwrap());
        }
        // Someone already tracked is still fine: nothing changes.
        assert!(!track_contributor(&mut c, "USER0").unwrap());
        let err = track_contributor(&mut c, "one-more").unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("You can track at most {MAX_CONTRIBUTORS} contributors.")
        );
        assert_eq!(c.contributors.len(), MAX_CONTRIBUTORS);
    }

    #[test]
    fn contributors_survive_a_round_trip_and_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let store = ConfigStore::new(path.clone());
        let mut config = cfg(&["dashpay/platform"], &[]);
        config.contributors = vec!["shumkov".into(), "QuantumExplorer".into()];
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("\"contributors\"")
        );
        // A file written before contributors existed loads with none.
        fs::write(&path, r#"{"projects":["a/b"],"followed":[]}"#).unwrap();
        assert!(store.load().unwrap().contributors.is_empty());
    }

    #[test]
    fn follow_and_unfollow() {
        let mut c = Config::default();
        assert!(follow(&mut c, "Dashpay/Platform", 12));
        assert!(!follow(&mut c, "dashpay/platform", 12));
        assert!(follow(&mut c, "dashpay/platform", 13));
        assert_eq!(
            c.followed,
            vec!["Dashpay/Platform#12", "dashpay/platform#13"]
        );
        assert!(unfollow(&mut c, "dashpay/platform", 12));
        assert!(!unfollow(&mut c, "dashpay/platform", 12));
        assert_eq!(c.followed, vec!["dashpay/platform#13"]);
    }
}
