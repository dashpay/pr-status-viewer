//! Learns from T3 Code which of its threads are tied to which pull request,
//! and whether each is working, waiting or idle.
//!
//! Where it comes from:
//! - Threads: T3 Code's own command, `t3 thread list --json --project
//!   <dir>` ([`cli`]), one call per T3 project whose clone's `origin` is a
//!   repo the app tracks or follows. The command asks the running T3 Code
//!   server; when T3 Code is not running, or the command fails for every
//!   project, the read is an error and the app shows no threads.
//! - Projects: the command cannot list them, so the project directories
//!   still come from T3 Code's database, opened read-only ([`projects`]).
//!   `run/` uses the same list to find a project's local clone.
//!
//! The output has no stability promise and is parsed tolerantly
//! ([`parse`]). Each thread gives its `status` (`running` is working,
//! `waiting` is waiting on a background task such as a pull request watch,
//! anything else is idle), `settled`, `updatedAt`, and its GitHub pull
//! requests from `pullRequests` (each with `watching`) and
//! `linkedPullRequest`.
//!
//! What is missed:
//! - Delegated and subagent threads: the command lists a project's
//!   top-level threads only. A parent's own status covers the work it
//!   delegated, but a pull request linked only on a child thread is not
//!   seen.
//! - Threads in a T3 project whose directory is not a clone of a tracked or
//!   followed GitHub repo (a scratch folder, an unrelated repo), even when
//!   they link a tracked pull request.
//!
//! Nothing here writes to T3 Code: the only command run is `thread list`,
//! and the database is opened read-only.

pub mod cli;
pub mod parse;
pub mod projects;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};

use crate::model::T3ThreadState;

pub use cli::{T3Cli, T3Project, default_cli_path};
pub use projects::{T3Reader, default_db_path};

/// One T3 Code thread tied to one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T3Link {
    /// `owner/name` as T3 Code recorded it.
    pub repo: String,
    pub number: u64,
    pub thread_id: String,
    pub title: String,
    pub state: T3ThreadState,
    /// T3 Code watches this pull request for the thread.
    pub watching: bool,
    /// When the thread last changed.
    pub last_active_at: Option<DateTime<Utc>>,
}
