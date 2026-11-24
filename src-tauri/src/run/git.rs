//! Reading the user's clone and worktrees. Every command here only reads:
//! `GIT_OPTIONAL_LOCKS=0` keeps even `git status` from refreshing the index.
//! The only writes the run feature makes to git are in `job.rs`, and only
//! for the app's own checkout.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{AppError, AppResult};

use super::host::{Cmd, Host};

pub const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// A read-only git command in `dir`.
pub fn read_cmd(dir: &Path) -> Cmd {
    Cmd::new("git")
        .arg("-C")
        .arg(dir.to_string_lossy().into_owned())
        .var("GIT_OPTIONAL_LOCKS", "0")
        .var("GIT_TERMINAL_PROMPT", "0")
}

/// `owner/name` of a GitHub remote URL, as written in the URL.
pub fn github_repo_of(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let prefixes = [
        "git@github.com:",
        "ssh://git@github.com/",
        "ssh://git@github.com:22/",
        "https://github.com/",
        "http://github.com/",
        "git://github.com/",
        "https://www.github.com/",
    ];
    let rest = prefixes
        .iter()
        .find_map(|p| lower.starts_with(p).then(|| &url[p.len()..]))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    let part_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (part_ok(owner) && part_ok(name)).then(|| format!("{owner}/{name}"))
}

/// The remote URL points at the GitHub repo `repo` (`owner/name`), ignoring
/// case.
pub fn origin_matches(url: &str, repo: &str) -> bool {
    github_repo_of(url).is_some_and(|r| r.eq_ignore_ascii_case(repo))
}

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: Option<String>,
    /// Short branch name, without `refs/heads/`.
    pub branch: Option<String>,
    pub bare: bool,
    pub prunable: bool,
}

pub fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    let mut out = Vec::new();
    let mut cur: Option<Worktree> = None;
    for line in porcelain.lines() {
        if line.is_empty() {
            out.extend(cur.take());
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "worktree" {
            out.extend(cur.take());
            cur = Some(Worktree {
                path: PathBuf::from(value),
                ..Worktree::default()
            });
            continue;
        }
        let Some(wt) = cur.as_mut() else { continue };
        match key {
            "HEAD" => wt.head = Some(value.to_owned()),
            "branch" => {
                wt.branch = Some(
                    value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_owned(),
                )
            }
            "bare" => wt.bare = true,
            "prunable" => wt.prunable = true,
            _ => {}
        }
    }
    out.extend(cur);
    out
}

/// A worktree that matches the PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matching {
    pub worktree: Worktree,
    pub at_pr_head: bool,
}

/// The user's worktrees on the PR's branch or at its head commit, those at
/// the head first, otherwise in git's order. Leaves out bare and prunable
/// entries, directories that are gone, and the app's own checkout.
pub fn matching_worktrees(
    all: Vec<Worktree>,
    head_branch: Option<&str>,
    head_sha: &str,
    own_checkout: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Matching> {
    let own = canonical(own_checkout);
    let mut found: Vec<Matching> = all
        .into_iter()
        .filter(|w| !w.bare && !w.prunable && exists(&w.path))
        .filter(|w| canonical(&w.path) != own)
        .filter_map(|w| {
            let at_pr_head = w
                .head
                .as_deref()
                .is_some_and(|h| !head_sha.is_empty() && h.eq_ignore_ascii_case(head_sha));
            let on_branch = matches!((head_branch, w.branch.as_deref()), (Some(b), Some(wb)) if !b.is_empty() && b == wb);
            (at_pr_head || on_branch).then_some(Matching {
                worktree: w,
                at_pr_head,
            })
        })
        .collect();
    // Stable: git's order is kept within each group.
    found.sort_by_key(|m| !m.at_pr_head);
    found
}

pub fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ---- Queries ----

async fn query<H: Host>(host: &H, dir: &Path, args: &[&str]) -> Option<String> {
    let cmd = read_cmd(dir).args(args.iter().copied());
    match host.output(cmd, GIT_TIMEOUT).await {
        Ok(out) if out.success() => Some(out.stdout),
        _ => None,
    }
}

pub async fn origin_url<H: Host>(host: &H, dir: &Path) -> Option<String> {
    query(host, dir, &["remote", "get-url", "origin"])
        .await
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub async fn toplevel<H: Host>(host: &H, dir: &Path) -> Option<PathBuf> {
    query(host, dir, &["rev-parse", "--show-toplevel"])
        .await
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty())
}

pub async fn list_worktrees<H: Host>(host: &H, clone: &Path) -> AppResult<Vec<Worktree>> {
    query(host, clone, &["worktree", "list", "--porcelain"])
        .await
        .map(|s| parse_worktrees(&s))
        .ok_or_else(|| AppError::Other("Could not list the clone's worktrees.".into()))
}

/// Uncommitted changes to tracked files. Untracked files do not count.
pub async fn is_dirty<H: Host>(host: &H, dir: &Path) -> bool {
    query(
        host,
        dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .await
    .is_some_and(|s| !s.trim().is_empty())
}

pub async fn head_of<H: Host>(host: &H, dir: &Path) -> Option<String> {
    query(host, dir, &["rev-parse", "HEAD"])
        .await
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The top of the git repo at `dir` when its `origin` is the GitHub repo
/// `repo`; otherwise a sentence saying why not.
pub async fn validate_clone<H: Host>(host: &H, dir: &Path, repo: &str) -> AppResult<PathBuf> {
    if !dir.is_dir() {
        return Err(AppError::InvalidInput("That folder does not exist.".into()));
    }
    let Some(top) = toplevel(host, dir).await else {
        return Err(AppError::InvalidInput(
            "That folder is not a git repository.".into(),
        ));
    };
    let Some(url) = origin_url(host, &top).await else {
        return Err(AppError::InvalidInput(format!(
            "That repository has no remote called origin, so it cannot be told apart from other clones of {repo}."
        )));
    };
    if origin_matches(&url, repo) {
        return Ok(top);
    }
    Err(AppError::InvalidInput(match github_repo_of(&url) {
        Some(other) => format!("That folder is a clone of {other}, not {repo}."),
        None => format!("That folder's origin remote is not {repo} on GitHub."),
    }))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::super::host::RealHost;
    use super::*;

    #[test]
    fn origin_url_forms() {
        for url in [
            "git@github.com:dashpay/platform.git",
            "git@github.com:dashpay/platform",
            "https://github.com/dashpay/platform",
            "https://github.com/dashpay/platform.git",
            "https://github.com/dashpay/platform/",
            "ssh://git@github.com/dashpay/platform.git",
            "ssh://git@github.com/dashpay/platform",
            "HTTPS://GitHub.com/DashPay/Platform.git",
            "  git@github.com:DASHPAY/platform.git\n",
        ] {
            assert!(origin_matches(url, "dashpay/platform"), "{url}");
        }
        for url in [
            "git@github.com:dashpay/platform-fork.git",
            "git@gitlab.com:dashpay/platform.git",
            "https://github.com/dashpay",
            "https://github.com/dashpay/platform/tree/main",
            "/Users/me/src/platform",
            "",
        ] {
            assert!(!origin_matches(url, "dashpay/platform"), "{url}");
        }
        assert_eq!(
            github_repo_of("git@github.com:QuantumExplorer/platform.git").as_deref(),
            Some("QuantumExplorer/platform")
        );
    }

    const PORCELAIN: &str = "\
worktree /src/platform
HEAD 62245a5c94aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
branch refs/heads/fix/sdk-start

worktree /tmp/scratch/wt-clippy
HEAD c895441e5baaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
detached
prunable gitdir file points to non-existent location

worktree /src/platform/.claude/worktrees/a
HEAD 9cc7778487200a5f379f0cb47cfff854e1a520dd
detached

worktree /src/platform/.claude/worktrees/b
HEAD 1111111111111111111111111111111111111111
branch refs/heads/fix/dashmate-local

worktree /cache/checkouts/dashpay-platform
HEAD 9cc7778487200a5f379f0cb47cfff854e1a520dd
detached

worktree /src/platform/.claude/worktrees/c
HEAD 9cc7778487200a5f379f0cb47cfff854e1a520dd
branch refs/heads/fix/dashmate-local
locked reason

";

    #[test]
    fn porcelain_parses() {
        let all = parse_worktrees(PORCELAIN);
        assert_eq!(all.len(), 6);
        assert_eq!(all[0].branch.as_deref(), Some("fix/sdk-start"));
        assert!(all[1].prunable);
        assert_eq!(all[2].branch, None);
        assert_eq!(
            all[2].head.as_deref(),
            Some("9cc7778487200a5f379f0cb47cfff854e1a520dd")
        );
        // No trailing blank line still yields the last entry.
        let trimmed = parse_worktrees("worktree /a\nHEAD abc\nbare");
        assert_eq!(trimmed.len(), 1);
        assert!(trimmed[0].bare);
    }

    #[test]
    fn matching_puts_head_first_and_skips_the_app_checkout() {
        let all = parse_worktrees(PORCELAIN);
        let got = matching_worktrees(
            all,
            Some("fix/dashmate-local"),
            "9CC7778487200a5f379f0cb47cfff854e1a520dd",
            Path::new("/cache/checkouts/dashpay-platform"),
            &|p| !p.starts_with("/tmp"),
        );
        let paths: Vec<(&str, bool)> = got
            .iter()
            .map(|m| (m.worktree.path.to_str().unwrap(), m.at_pr_head))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("/src/platform/.claude/worktrees/a", true),
                ("/src/platform/.claude/worktrees/c", true),
                ("/src/platform/.claude/worktrees/b", false),
            ]
        );
    }

    #[test]
    fn matching_without_a_branch_uses_the_commit_only() {
        let got = matching_worktrees(
            parse_worktrees(PORCELAIN),
            None,
            "62245a5c94aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Path::new("/elsewhere"),
            &|_| true,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].worktree.path, PathBuf::from("/src/platform"));
        // A missing directory is left out.
        let got = matching_worktrees(
            parse_worktrees(PORCELAIN),
            Some("fix/sdk-start"),
            "",
            Path::new("/elsewhere"),
            &|_| false,
        );
        assert!(got.is_empty());
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn repo(origin: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(
            dir.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        if let Some(url) = origin {
            git(dir.path(), &["remote", "add", "origin", url]);
        }
        dir
    }

    #[tokio::test]
    async fn validate_clone_explains_each_failure() {
        let host = RealHost;
        let good = repo(Some("git@github.com:dashpay/platform.git"));
        let sub = good.path().join("packages");
        std::fs::create_dir_all(&sub).unwrap();
        let top = validate_clone(&host, &sub, "DashPay/Platform")
            .await
            .unwrap();
        assert_eq!(canonical(&top), canonical(good.path()));

        let other = repo(Some("https://github.com/someone/platform"));
        assert_eq!(
            validate_clone(&host, other.path(), "dashpay/platform")
                .await
                .unwrap_err()
                .to_string(),
            "That folder is a clone of someone/platform, not dashpay/platform."
        );
        let none = repo(None);
        assert!(
            validate_clone(&host, none.path(), "dashpay/platform")
                .await
                .unwrap_err()
                .to_string()
                .starts_with("That repository has no remote called origin")
        );
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            validate_clone(&host, plain.path(), "dashpay/platform")
                .await
                .unwrap_err()
                .to_string(),
            "That folder is not a git repository."
        );
        assert_eq!(
            validate_clone(&host, &plain.path().join("nope"), "dashpay/platform")
                .await
                .unwrap_err()
                .to_string(),
            "That folder does not exist."
        );
    }

    #[tokio::test]
    async fn dirty_ignores_untracked_files_and_does_not_touch_the_index() {
        let host = RealHost;
        let r = repo(Some("git@github.com:dashpay/platform.git"));
        std::fs::write(r.path().join("a.txt"), "1").unwrap();
        git(r.path(), &["add", "a.txt"]);
        git(
            r.path(),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "a",
            ],
        );
        std::fs::write(r.path().join("untracked.txt"), "x").unwrap();
        assert!(!is_dirty(&host, r.path()).await);
        let index = r.path().join(".git/index");
        let before = std::fs::metadata(&index).unwrap().modified().unwrap();
        std::fs::write(r.path().join("a.txt"), "2").unwrap();
        assert!(is_dirty(&host, r.path()).await);
        let after = std::fs::metadata(&index).unwrap().modified().unwrap();
        assert_eq!(before, after, "git status rewrote the index");

        let wts = list_worktrees(&host, r.path()).await.unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert_eq!(wts[0].head, head_of(&host, r.path()).await);
    }
}
