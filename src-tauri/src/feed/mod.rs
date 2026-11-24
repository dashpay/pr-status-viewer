//! Client for the review bot's public status feed.
//!
//! The feed (`FEED_URL`) is regenerated about once a minute and has no
//! stability promise, so `parse_feed` reads it field by field and keeps
//! whatever it can: missing, null, wrongly typed and unknown fields are all
//! tolerated, and one bad entry in a list never discards the others.
//!
//! `FeedSnapshot` keeps the feed's own relative numbers (seconds as of
//! `data_as_of`). `merge` turns them into absolute timestamps against
//! `data_as_of`, never the local clock, so they do not drift between fetches.

mod parse;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};

use crate::error::{AppError, AppResult};
use crate::model::Findings;

pub use parse::parse_feed;

pub const FEED_URL: &str = "https://thepastaclaw.github.io/review-system/data/status.json";

const USER_AGENT: &str = "pr-status-viewer";
const TIMEOUT: Duration = Duration::from_secs(15);

/// The parts of the feed the app uses, already validated. Only `merge`
/// reads its fields.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeedSnapshot {
    /// When the bot generated the data. Every relative number in the feed is
    /// as of this instant. Falls back to `live.ts`, then `generated_at`.
    pub data_as_of: Option<DateTime<Utc>>,
    /// Running reviews (`live.active`).
    pub active: Vec<FeedActive>,
    /// Waiting reviews (`live.queued`).
    pub queued: Vec<FeedQueued>,
    /// `live.heads.running`.
    pub heads_running: Option<u32>,
    /// `live.heads.queued`.
    pub heads_queued: Option<u32>,
    /// `live.median_run_minutes`.
    pub median_run_minutes: Option<f64>,
    /// `live.typical_minutes_by_tier`, e.g. "normal" -> 28.6.
    pub typical_minutes_by_tier: BTreeMap<String, f64>,
    /// `live.degraded.active`.
    pub degraded: bool,
    /// `history.path_timing.paths.normal`.
    pub normal_path: PathTiming,
    /// `history.path_timing.paths.priority`.
    pub priority_path: PathTiming,
}

/// Median timings for one path through the bot's queue, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PathTiming {
    /// Time from start of the run to its end.
    pub duration_median_seconds: Option<f64>,
    /// Time from becoming eligible to getting a review slot.
    pub slot_wait_median_seconds: Option<f64>,
    /// Time from being queued to starting (debounce included).
    pub wait_median_seconds: Option<f64>,
}

/// One running review (`live.active[]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeedActive {
    /// `owner/name`, as the feed spells it.
    pub repo: String,
    pub number: u64,
    /// Head commit under review. `None` when the feed omits it.
    pub sha: Option<String>,
    /// e.g. "phase1", "phase2", "verify2".
    pub phase: Option<String>,
    pub status: Option<String>,
    /// 0.0 to 1.0.
    pub progress: Option<f64>,
    /// The bot's estimate, as of `data_as_of`.
    pub remaining_seconds: Option<f64>,
    pub elapsed_seconds: Option<f64>,
    pub started_at: Option<DateTime<Utc>>,
    pub queued_at: Option<DateTime<Utc>>,
    /// Seconds in the queue before the run started.
    pub wait_seconds: Option<f64>,
    pub overdue: bool,
    pub tier: Option<String>,
    pub priority: bool,
    pub findings: Option<Findings>,
    /// Steps started so far, in feed order.
    pub steps: Vec<FeedStep>,
    /// Step names not yet started.
    pub upcoming: Vec<String>,
}

/// One entry of `live.active[].steps[]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeedStep {
    pub name: String,
    /// "ok", "running", "failed", ...
    pub status: Option<String>,
    pub lanes_done: Option<u32>,
    pub lanes_total: Option<u32>,
    pub seconds: Option<f64>,
    pub started_at: Option<DateTime<Utc>>,
}

/// One waiting review (`live.queued[]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FeedQueued {
    pub repo: String,
    pub number: u64,
    pub sha: Option<String>,
    /// 1-based.
    pub position: Option<u32>,
    /// True once the debounce has passed and it only waits for a slot.
    pub eligible: Option<bool>,
    /// As of `data_as_of`.
    pub eligible_in_seconds: Option<f64>,
    pub eligible_at: Option<DateTime<Utc>>,
    /// e.g. "debounce (more pushes may follow)".
    pub reason: Option<String>,
    pub priority: bool,
    pub queued_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct FeedClient {
    http: reqwest::Client,
    url: String,
}

impl FeedClient {
    /// Client for `FEED_URL`.
    pub fn new() -> AppResult<Self> {
        Self::with_url(FEED_URL)
    }

    /// Client for another URL (tests).
    pub fn with_url(url: impl Into<String>) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            url: url.into(),
        })
    }

    pub async fn fetch(&self) -> AppResult<FeedSnapshot> {
        let url = cache_busted(&self.url, cache_buster());
        let response = self.http.get(&url).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Network(format!(
                "the review feed answered HTTP {}",
                status.as_u16()
            )));
        }
        let body = response.text().await?;
        parse_feed(&body)
    }
}

/// A value that changes on every request, so no cache between us and
/// GitHub Pages serves a stale copy.
fn cache_buster() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

fn cache_busted(url: &str, token: u128) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}_={token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_buster_appends_a_query_parameter() {
        assert_eq!(
            cache_busted("https://x.test/status.json", 42),
            "https://x.test/status.json?_=42"
        );
        assert_eq!(
            cache_busted("https://x.test/status.json?a=1", 42),
            "https://x.test/status.json?a=1&_=42"
        );
    }

    #[test]
    fn clients_build() {
        assert!(FeedClient::new().is_ok());
        let client = FeedClient::with_url("http://127.0.0.1:9/status.json");
        assert!(client.is_ok_and(|c| c.url == "http://127.0.0.1:9/status.json"));
    }
}
