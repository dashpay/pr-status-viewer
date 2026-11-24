//! Turns the output of `t3 thread list --json` into [`T3Link`]s. Pure.
//!
//! The output has no stability promise, so the parse is tolerant: unknown
//! fields are ignored, a thread or pull request entry that cannot be read is
//! skipped, and only output that is not a list of threads at all is a
//! failure.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::model::T3ThreadState;

use super::T3Link;

/// Statuses that mean a run is in flight. Only `running` has been seen; the
/// others are names a run on its way could plausibly be given.
const WORKING_STATUSES: &[&str] = &[
    "running",
    "queued",
    "starting",
    "preparing",
    "pending",
    "in_progress",
    "in-progress",
    "streaming",
];

/// The thread's turn ended while it waits on a background task it started
/// (a pull request watch, a command, a delegated task).
const WAITING_STATUSES: &[&str] = &["waiting"];

/// One thread from the list, reduced to what the app uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedThread {
    pub thread_id: String,
    pub title: String,
    pub state: T3ThreadState,
    pub settled: bool,
    pub updated_at: Option<DateTime<Utc>>,
    /// Unique by repository (ignoring case) and number, first seen first.
    pub prs: Vec<ListedPr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedPr {
    /// `owner/name` as T3 Code recorded it.
    pub repo: String,
    pub number: u64,
    /// T3 Code watches this PR for the thread.
    pub watching: bool,
}

/// The threads in one `thread list --json` output. `None` when the output
/// is not a list of threads (an error message, a changed format).
pub fn parse_thread_list(stdout: &str) -> Option<Vec<ListedThread>> {
    let value: Value = serde_json::from_str(stdout.trim()).ok()?;
    // A later version might wrap the list in an object.
    let items = match &value {
        Value::Array(items) => items,
        Value::Object(o) => o.get("threads")?.as_array()?,
        _ => return None,
    };
    Some(items.iter().filter_map(parse_thread).collect())
}

/// What `status` means for the app. Anything unknown is idle: the thread
/// exists, and nothing says it is busy.
pub fn state_of(status: Option<&str>) -> T3ThreadState {
    let Some(status) = status.map(|s| s.trim().to_ascii_lowercase()) else {
        return T3ThreadState::Idle;
    };
    if WORKING_STATUSES.contains(&status.as_str()) {
        T3ThreadState::Working
    } else if WAITING_STATUSES.contains(&status.as_str()) {
        T3ThreadState::Waiting
    } else {
        T3ThreadState::Idle
    }
}

fn parse_thread(value: &Value) -> Option<ListedThread> {
    let obj = value.as_object()?;
    let thread_id = obj.get("threadId")?.as_str()?.trim();
    if thread_id.is_empty() || is_gone(obj) {
        return None;
    }
    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    let settled = match obj.get("settled").and_then(Value::as_bool) {
        Some(settled) => settled,
        None => non_empty_text(obj.get("settledAt")),
    };
    let explicit = obj
        .get("pullRequests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let linked = obj.get("linkedPullRequest").filter(|v| v.is_object());
    let mut prs: Vec<ListedPr> = Vec::new();
    for pr in explicit.chain(linked).filter_map(listed_pr) {
        match prs
            .iter_mut()
            .find(|p| p.number == pr.number && p.repo.eq_ignore_ascii_case(&pr.repo))
        {
            Some(seen) => seen.watching |= pr.watching,
            None => prs.push(pr),
        }
    }
    Some(ListedThread {
        thread_id: thread_id.to_owned(),
        title,
        state: state_of(obj.get("status").and_then(Value::as_str)),
        settled,
        updated_at: obj
            .get("updatedAt")
            .and_then(Value::as_str)
            .and_then(parse_time),
        prs,
    })
}

/// The list leaves archived and deleted threads out; should a later version
/// list them with a mark, they are still left out.
fn is_gone(obj: &Map<String, Value>) -> bool {
    ["archivedAt", "deletedAt"]
        .iter()
        .any(|k| non_empty_text(obj.get(*k)))
        || ["archived", "deleted"]
            .iter()
            .any(|k| obj.get(*k).and_then(Value::as_bool) == Some(true))
}

fn non_empty_text(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

/// One GitHub pull request entry. Its `host` decides when present; without
/// one, its `url` must point at github.com.
fn listed_pr(pr: &Value) -> Option<ListedPr> {
    let on_github = match pr.get("host") {
        Some(Value::String(host)) => host.trim().eq_ignore_ascii_case("github.com"),
        None | Some(Value::Null) => pr
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(is_github_url),
        Some(_) => false,
    };
    if !on_github {
        return None;
    }
    let (repo, number) = pr_ref(pr)?;
    Some(ListedPr {
        repo,
        number,
        watching: pr.get("watching").and_then(Value::as_bool) == Some(true),
    })
}

/// `repository` and `number` of one PR entry, when both are usable.
fn pr_ref(pr: &Value) -> Option<(String, u64)> {
    let repo = pr.get("repository")?.as_str()?.trim();
    let number = pr.get("number")?.as_u64().filter(|n| *n > 0)?;
    let (owner, name) = repo.split_once('/')?;
    let usable = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (usable(owner) && usable(name)).then(|| (repo.to_owned(), number))
}

fn is_github_url(url: &str) -> bool {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"));
    rest.and_then(|r| r.split('/').next())
        .is_some_and(|host| host.eq_ignore_ascii_case("github.com"))
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// One link per (thread, pull request), sorted by repository, number and
/// thread. A thread listed more than once (under two projects) counts once,
/// as its most recently updated copy. A settled thread is left out unless it
/// is working or waiting.
pub fn links_of(threads: impl IntoIterator<Item = ListedThread>) -> Vec<T3Link> {
    let mut unique: Vec<ListedThread> = Vec::new();
    for thread in threads {
        match unique.iter_mut().find(|t| t.thread_id == thread.thread_id) {
            Some(seen) => {
                if thread.updated_at > seen.updated_at {
                    *seen = thread;
                }
            }
            None => unique.push(thread),
        }
    }

    let mut links = Vec::new();
    for thread in unique {
        if thread.settled && thread.state == T3ThreadState::Idle {
            continue;
        }
        for pr in &thread.prs {
            links.push(T3Link {
                repo: pr.repo.clone(),
                number: pr.number,
                thread_id: thread.thread_id.clone(),
                title: thread.title.clone(),
                state: thread.state,
                watching: pr.watching,
                last_active_at: thread.updated_at,
            });
        }
    }
    links.sort_by(|a, b| {
        (a.repo.to_ascii_lowercase(), a.number, &a.thread_id).cmp(&(
            b.repo.to_ascii_lowercase(),
            b.number,
            &b.thread_id,
        ))
    });
    links
}
