//! The seams the sync engine talks through. The app plugs in the real GitHub
//! client, feed client, T3 Code's `t3` command, Tauri event emitter and config file;
//! the tests plug in fakes.

use std::future::Future;

use tauri::{AppHandle, Emitter};

use crate::config::{self, ConfigStore};
use crate::error::AppResult;
use crate::feed::{FeedClient, FeedSnapshot};
use crate::github::types::{GhAccount, GhMergeCheck, GhPrAuthor, GhProject, GhSelfReviewCheck};
use crate::github::{self, GitHubClient};
use crate::model::{Config, PrSummary, Snapshot, SyncState, Viewer};
use crate::t3::{T3Cli, T3Link};

/// Event carrying a full `Snapshot`. Must match `EVENTS.snapshot` in `src/api.ts`.
pub const EVENT_SNAPSHOT: &str = "snapshot";
/// Event carrying a `SyncState`. Must match `EVENTS.syncState` in `src/api.ts`.
pub const EVENT_SYNC_STATE: &str = "sync-state";

/// A signed-in GitHub client.
pub trait GitHubApi: Send + Sync + 'static {
    fn viewer(&self) -> impl Future<Output = AppResult<Viewer>> + Send;
    fn canonical_repo(&self, repo: &str) -> impl Future<Output = AppResult<String>> + Send;
    /// The account named `login`; `None` when there is no such user or
    /// organization.
    fn account(&self, login: &str) -> impl Future<Output = AppResult<Option<GhAccount>>> + Send;
    /// The open PRs in `repo` by the viewer and by `contributors`, plus the
    /// `followed` ones (by number) that are still open.
    fn fetch_project(
        &self,
        repo: &str,
        followed: &[u64],
        contributors: &[String],
    ) -> impl Future<Output = AppResult<GhProject>> + Send;
    fn list_open_prs(&self, repo: &str) -> impl Future<Output = AppResult<Vec<PrSummary>>> + Send;
    /// The author of each open PR in `repo`, from the same listing.
    fn open_pr_authors(
        &self,
        repo: &str,
    ) -> impl Future<Output = AppResult<Vec<GhPrAuthor>>> + Send;
    /// A fresh look at whether one PR can be squash-merged.
    fn merge_check(
        &self,
        repo: &str,
        number: u64,
    ) -> impl Future<Output = AppResult<GhMergeCheck>> + Send;
    /// Squash-merges the PR, refused by GitHub if its head is not
    /// `expected_head_oid`. Changes a real pull request: only ever called
    /// after a fresh `merge_check` says it is ready.
    fn squash_merge(
        &self,
        pr_node_id: &str,
        expected_head_oid: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
    /// A fresh look at whether the viewer can post the self-review on one
    /// PR. Read-only.
    fn self_review_check(
        &self,
        repo: &str,
        number: u64,
    ) -> impl Future<Output = AppResult<GhSelfReviewCheck>> + Send;
    /// Posts `body` as a comment on the PR with node id `subject_id`, as the
    /// user. Their own statement on GitHub: only ever called with the
    /// self-review comment, after a fresh `self_review_check` allows it and
    /// the user has confirmed.
    fn add_comment(
        &self,
        subject_id: &str,
        body: &str,
    ) -> impl Future<Output = AppResult<()>> + Send;
}

/// Finds credentials and builds a `GitHubApi` client.
pub trait GitHubConnector: Send + Sync + 'static {
    type Client: GitHubApi;
    fn connect(&self) -> impl Future<Output = AppResult<Self::Client>> + Send;
}

pub trait FeedSource: Send + Sync + 'static {
    fn fetch(&self) -> impl Future<Output = AppResult<FeedSnapshot>> + Send;
}

/// T3 Code's record of which threads belong to which PR.
pub trait T3Source: Send + Sync + 'static {
    /// The threads tied to pull requests, read from at least the T3 projects
    /// whose clone is one of `repos` (`owner/name`, any case). `Ok(None)`
    /// means T3 Code is not there. The engine runs it in its own task and
    /// never waits on it.
    fn read(&self, repos: &[String])
    -> impl Future<Output = AppResult<Option<Vec<T3Link>>>> + Send;
}

/// Where the engine sends what the UI should see.
pub trait EventSink: Send + Sync + 'static {
    fn snapshot(&self, snapshot: &Snapshot);
    fn sync_state(&self, state: SyncState);
}

/// Where the config lives between runs.
pub trait ConfigBackend: Send + Sync + 'static {
    fn load(&self) -> AppResult<Config>;
    fn save(&self, config: &Config) -> AppResult<()>;
}

/// The pure config rules from `crate::config`, injectable so the engine's
/// tests do not depend on another module's progress.
#[derive(Clone, Copy)]
pub struct ConfigRules {
    pub parse_repo: fn(&str) -> AppResult<String>,
    pub parse_pr_ref: fn(&str) -> AppResult<(String, u64)>,
    pub add_project: fn(&mut Config, &str) -> bool,
    pub remove_project: fn(&mut Config, &str) -> bool,
    pub follow: fn(&mut Config, &str, u64) -> bool,
    pub unfollow: fn(&mut Config, &str, u64) -> bool,
    pub parse_login: fn(&str) -> AppResult<String>,
    /// Ok(false) when already tracked; an error past the maximum.
    pub track_contributor: fn(&mut Config, &str) -> AppResult<bool>,
    pub untrack_contributor: fn(&mut Config, &str) -> bool,
}

impl ConfigRules {
    pub const REAL: Self = Self {
        parse_repo: config::parse_repo,
        parse_pr_ref: config::parse_pr_ref,
        add_project: config::add_project,
        remove_project: config::remove_project,
        follow: config::follow,
        unfollow: config::unfollow,
        parse_login: config::parse_login,
        track_contributor: config::track_contributor,
        untrack_contributor: config::untrack_contributor,
    };
}

// ---- Real implementations ----

impl GitHubApi for GitHubClient {
    async fn viewer(&self) -> AppResult<Viewer> {
        GitHubClient::viewer(self).await
    }

    async fn canonical_repo(&self, repo: &str) -> AppResult<String> {
        GitHubClient::canonical_repo(self, repo).await
    }

    async fn account(&self, login: &str) -> AppResult<Option<GhAccount>> {
        GitHubClient::account(self, login).await
    }

    async fn fetch_project(
        &self,
        repo: &str,
        followed: &[u64],
        contributors: &[String],
    ) -> AppResult<GhProject> {
        GitHubClient::fetch_project(self, repo, followed, contributors).await
    }

    async fn list_open_prs(&self, repo: &str) -> AppResult<Vec<PrSummary>> {
        GitHubClient::list_open_prs(self, repo).await
    }

    async fn open_pr_authors(&self, repo: &str) -> AppResult<Vec<GhPrAuthor>> {
        GitHubClient::open_pr_authors(self, repo).await
    }

    async fn merge_check(&self, repo: &str, number: u64) -> AppResult<GhMergeCheck> {
        GitHubClient::merge_check(self, repo, number).await
    }

    async fn squash_merge(&self, pr_node_id: &str, expected_head_oid: &str) -> AppResult<()> {
        GitHubClient::squash_merge(self, pr_node_id, expected_head_oid).await
    }

    async fn self_review_check(&self, repo: &str, number: u64) -> AppResult<GhSelfReviewCheck> {
        GitHubClient::self_review_check(self, repo, number).await
    }

    async fn add_comment(&self, subject_id: &str, body: &str) -> AppResult<()> {
        GitHubClient::add_comment(self, subject_id, body).await
    }
}

impl T3Source for T3Cli {
    async fn read(&self, repos: &[String]) -> AppResult<Option<Vec<T3Link>>> {
        T3Cli::read(self, repos).await
    }
}

/// Looks for a token the way `github::resolve_token` does, every time it is
/// asked, so signing in with `gh` while the app runs is picked up.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenConnector;

impl GitHubConnector for TokenConnector {
    type Client = GitHubClient;

    async fn connect(&self) -> AppResult<GitHubClient> {
        let token = github::resolve_token().await?;
        GitHubClient::new(token)
    }
}

impl FeedSource for FeedClient {
    async fn fetch(&self) -> AppResult<FeedSnapshot> {
        FeedClient::fetch(self).await
    }
}

impl ConfigBackend for ConfigStore {
    fn load(&self) -> AppResult<Config> {
        ConfigStore::load(self)
    }

    fn save(&self, config: &Config) -> AppResult<()> {
        ConfigStore::save(self, config)
    }
}

/// Emits to every webview of the app.
pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn snapshot(&self, snapshot: &Snapshot) {
        if let Err(e) = self.0.emit(EVENT_SNAPSHOT, snapshot) {
            log::warn!("could not emit {EVENT_SNAPSHOT}: {e}");
        }
    }

    fn sync_state(&self, state: SyncState) {
        if let Err(e) = self.0.emit(EVENT_SYNC_STATE, state) {
            log::warn!("could not emit {EVENT_SYNC_STATE}: {e}");
        }
    }
}
