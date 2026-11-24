//! Lists T3 Code's threads through its own `t3` command:
//! `t3 thread list --json --project <dir>`, once per T3 project worth
//! reading, side by side.
//!
//! The command is a launcher T3 Code writes to `~/.t3/bin/t3`; it runs the
//! desktop app's bundled server code as node, which asks the running T3
//! Code server and handles its own sign-in. One call costs about 0.3 s and
//! 0.4 s of CPU, so only the projects whose clone's `origin` is one of the
//! repos the app cares about are read, and the project list (one database
//! read and a `git remote get-url` per project) is kept for a few minutes.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::task::JoinSet;

use crate::error::{AppError, AppResult};
use crate::run::git;
use crate::run::host::RealHost;

use super::T3Link;
use super::parse::{links_of, parse_thread_list};
use super::projects::{T3Reader, default_db_path};

/// Overrides where the `t3` command is.
pub const CLI_PATH_ENV: &str = "PR_STATUS_T3_CLI";
/// A call that has not answered by then is stopped.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the project list and each project's origin are reused.
pub const PROJECTS_TTL: Duration = Duration::from_secs(5 * 60);
/// Never handed to the command: it has no use for a GitHub token.
const HIDDEN_VARS: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];
/// How much of a failed call's output reaches the log.
const EXCERPT_CHARS: usize = 200;

pub(super) const MSG_CLI: &str = "T3 Code's t3 command could not list its threads.";

/// Where T3 Code's `t3` command is: `PR_STATUS_T3_CLI`, else
/// `$T3CODE_HOME/bin/t3`, else `~/.t3/bin/t3`. Never looked up on `PATH`,
/// where another tool may be called `t3`. Does not check that it exists.
pub fn default_cli_path() -> Option<PathBuf> {
    cli_path_from(
        std::env::var_os(CLI_PATH_ENV),
        std::env::var_os("T3CODE_HOME"),
        std::env::var_os("HOME"),
    )
}

/// [`default_cli_path`] with the environment passed in.
pub(super) fn cli_path_from(
    override_path: Option<std::ffi::OsString>,
    t3_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(t3_home) = t3_home.filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(t3_home).join("bin").join("t3"));
    }
    let home = home.filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".t3").join("bin").join("t3"))
}

/// One T3 Code project and the GitHub repo its clone's `origin` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T3Project {
    pub root: PathBuf,
    /// `owner/name`; `None` when the directory is not a clone of a GitHub
    /// repo (or its origin could not be read).
    pub repo: Option<String>,
}

/// The projects whose origin is one of `repos` (`owner/name`, any case), in
/// T3 Code's order. A project with no GitHub origin is never read.
pub fn projects_to_read<'a>(projects: &'a [T3Project], repos: &[String]) -> Vec<&'a Path> {
    projects
        .iter()
        .filter(|p| {
            p.repo
                .as_deref()
                .is_some_and(|r| repos.iter().any(|w| w.trim().eq_ignore_ascii_case(r)))
        })
        .map(|p| p.root.as_path())
        .collect()
}

/// Every project's threads put together. Partial results are fine: a
/// failed call is logged and skipped, and only when every call failed is
/// the whole read an error.
pub(super) fn combine(results: Vec<(PathBuf, Result<String, String>)>) -> AppResult<Vec<T3Link>> {
    let total = results.len();
    let mut threads = Vec::new();
    let mut read = 0;
    for (root, result) in results {
        let parsed = result.and_then(|stdout| {
            parse_thread_list(&stdout)
                .ok_or_else(|| format!("its output is not a list of threads: {}", excerpt(&stdout)))
        });
        match parsed {
            Ok(listed) => {
                read += 1;
                threads.extend(listed);
            }
            Err(why) => log::debug!("t3: thread list for {} failed: {why}", root.display()),
        }
    }
    if total > 0 && read == 0 {
        return Err(AppError::Other(MSG_CLI.into()));
    }
    Ok(links_of(threads))
}

/// The start of a command's output, on one line, for the log.
pub(super) fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(EXCERPT_CHARS).collect();
    if flat.chars().count() > EXCERPT_CHARS {
        out.push('…');
    }
    out
}

/// Reads T3 Code's threads through its `t3` command. Where the command and
/// the database are is worked out on every read unless given, so installing
/// T3 Code while the app runs is picked up.
#[derive(Debug, Default)]
pub struct T3Cli {
    cli: Option<PathBuf>,
    db: Option<PathBuf>,
    projects: Mutex<Option<(Instant, Vec<T3Project>)>>,
}

impl T3Cli {
    /// T3 Code where it installs itself.
    pub fn new() -> Self {
        Self::default()
    }

    /// The command and the database at these places instead (`None`: the
    /// default place).
    pub fn with_paths(cli: Option<PathBuf>, db: Option<PathBuf>) -> Self {
        Self {
            cli,
            db,
            projects: Mutex::default(),
        }
    }

    pub fn cli_path(&self) -> Option<PathBuf> {
        self.cli.clone().or_else(default_cli_path)
    }

    /// The live threads that are tied to a pull request, from the T3
    /// projects whose origin is one of `repos`. `Ok(None)` when T3 Code is
    /// not installed (no `t3` command, or no database to list projects
    /// from). An error when no project could be read.
    pub async fn read(&self, repos: &[String]) -> AppResult<Option<Vec<T3Link>>> {
        let Some(cli) = self.cli_path().filter(|p| p.is_file()) else {
            return Ok(None);
        };
        let Some(projects) = self.projects().await? else {
            return Ok(None);
        };
        let roots: Vec<PathBuf> = projects_to_read(&projects, repos)
            .into_iter()
            .map(Path::to_path_buf)
            .collect();

        let mut calls = JoinSet::new();
        for root in roots {
            let cli = cli.clone();
            calls.spawn(async move {
                let result = list_threads(&cli, &root).await;
                (root, result)
            });
        }
        let mut results = Vec::new();
        while let Some(joined) = calls.join_next().await {
            match joined {
                Ok(result) => results.push(result),
                Err(e) => log::debug!("t3: a thread list call failed: {e}"),
            }
        }
        // Same input, same order, whichever call finished first.
        results.sort_by(|a, b| a.0.cmp(&b.0));
        combine(results).map(Some)
    }

    /// T3 Code's projects with their origins, reused for [`PROJECTS_TTL`].
    /// `Ok(None)` when T3 Code's database is not there.
    pub async fn projects(&self) -> AppResult<Option<Vec<T3Project>>> {
        if let Some((at, projects)) = self.lock().as_ref()
            && at.elapsed() < PROJECTS_TTL
        {
            return Ok(Some(projects.clone()));
        }
        let Some(db) = self.db.clone().or_else(default_db_path) else {
            return Ok(None);
        };
        let roots = tokio::task::spawn_blocking(move || T3Reader::new(db).read_project_roots())
            .await
            .map_err(|e| AppError::Other(format!("Reading T3 Code's projects failed: {e}")))??;
        let Some(roots) = roots else {
            return Ok(None);
        };

        let mut lookups = JoinSet::new();
        for (i, root) in roots.into_iter().enumerate() {
            lookups.spawn(async move {
                let repo = git::origin_url(&RealHost, &root)
                    .await
                    .and_then(|url| git::github_repo_of(&url));
                (i, T3Project { root, repo })
            });
        }
        let mut found = Vec::new();
        while let Some(joined) = lookups.join_next().await {
            if let Ok(project) = joined {
                found.push(project);
            }
        }
        found.sort_by_key(|(i, _)| *i);
        let projects: Vec<T3Project> = found.into_iter().map(|(_, p)| p).collect();
        *self.lock() = Some((Instant::now(), projects.clone()));
        Ok(Some(projects))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(Instant, Vec<T3Project>)>> {
        self.projects
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One `t3 thread list --json --project <root>`: its stdout, or why it
/// failed. Read-only. Stopped, with anything it started, after
/// [`CALL_TIMEOUT`].
async fn list_threads(cli: &Path, root: &Path) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(cli);
    cmd.args(["thread", "list", "--json", "--project"])
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for var in HIDDEN_VARS {
        cmd.env_remove(var);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn().map_err(|e| format!("could not start: {e}"))?;
    let pgid = child.id();
    let output = child.wait_with_output();
    tokio::pin!(output);
    tokio::select! {
        done = &mut output => match done {
            Ok(out) if out.status.success() => {
                String::from_utf8(out.stdout).map_err(|_| "its output is not UTF-8".to_owned())
            }
            Ok(out) => {
                let said = if out.stderr.iter().any(|b| !b.is_ascii_whitespace()) {
                    out.stderr
                } else {
                    out.stdout
                };
                Err(format!(
                    "{}: {}",
                    out.status,
                    excerpt(&String::from_utf8_lossy(&said))
                ))
            }
            Err(e) => Err(format!("lost track of it: {e}")),
        },
        () = tokio::time::sleep(CALL_TIMEOUT) => {
            // The leader is not reaped yet, so its group id still names
            // only what it started. Dropping `output` then kills and reaps
            // the leader itself.
            kill_group(pgid);
            Err(format!("no answer within {} s; stopped", CALL_TIMEOUT.as_secs()))
        }
    }
}

#[cfg(unix)]
fn kill_group(pgid: Option<u32>) {
    let Some(pgid) = pgid.and_then(|p| libc::pid_t::try_from(p).ok()) else {
        return;
    };
    // SAFETY: `kill` takes plain integers and has no memory effects. A
    // negative pid addresses the process group created for this call.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(_pgid: Option<u32>) {}
