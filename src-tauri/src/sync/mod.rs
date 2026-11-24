//! The sync engine: polls GitHub and the review feed, keeps the latest
//! snapshot, and emits it to the UI.
//!
//! All state sits behind one `std::sync::Mutex` that is never held across an
//! `.await`. Network calls work on clones taken out of the lock, and their
//! results are written back in one short critical section.
//!
//! Scheduling lives in [`Engine::run`]: a full cycle at startup, the feed
//! every [`FEED_INTERVAL`], GitHub every [`GITHUB_INTERVAL`], and a full
//! cycle whenever [`Engine::request_sync`] is called. T3 Code's threads are
//! read at the start of every cycle, beside the passes and never waited on. A request only sets a
//! flag that the next cycle clears as it starts, so any number of requests
//! made while a cycle runs collapse into exactly one follow-up cycle.

pub mod source;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::feed::FeedSnapshot;
use crate::github::types::{GhAccountKind, GhPrAuthor, GhProject};
use crate::merge::{self, MergeInput, PostedSelfReview, ProjectInput};
use crate::model::{
    BOT_LOGIN, Config, KnownContributor, MergeCheck, PrSummary, SelfReviewCheck, SelfReviewState,
    Snapshot, SourceHealth, SyncState, Viewer, pr_id,
};
use crate::t3::T3Link;
use source::{
    ConfigBackend, ConfigRules, EventSink, FeedSource, GitHubApi, GitHubConnector, T3Source,
};

/// How often the review feed is polled.
pub const FEED_INTERVAL: Duration = Duration::from_secs(30);
/// How often GitHub is polled.
pub const GITHUB_INTERVAL: Duration = Duration::from_secs(90);
/// Focusing the window syncs when the last GitHub pass is older than this.
pub const FOCUS_STALE_AFTER: Duration = Duration::from_secs(20);
/// Most project fetches in flight at once.
pub const MAX_CONCURRENT_FETCHES: usize = 4;
/// Wait used when a rate-limit reset time cannot be read.
const RATE_LIMIT_FALLBACK: Duration = Duration::from_secs(60);
/// Never wait longer than this for a rate-limit reset.
const RATE_LIMIT_MAX: Duration = Duration::from_secs(60 * 60);

/// Builds the snapshot from what the engine holds. `merge::build_snapshot`
/// in the app; a simple stand-in in tests.
pub type MergeFn = for<'a> fn(MergeInput<'a>) -> Snapshot;

/// Refusal when the head moved between the user's look and the merge.
pub const HEAD_MOVED: &str =
    "New commits were pushed since you looked. Review them, then try again.";
/// Refusal when the user tries to track themselves.
pub const TRACK_SELF: &str = "Your own pull requests are already under Mine.";

/// Refusal when the head moved between the user's look and the self-review.
pub const SELF_REVIEW_HEAD_MOVED: &str =
    "New commits were pushed since you looked. Read them, then try again.";

/// Everything the engine needs from the outside world.
pub struct Deps<G, F, E, S, T> {
    pub github: G,
    pub feed: F,
    /// T3 Code's threads.
    pub t3: T,
    pub events: E,
    pub store: S,
    pub rules: ConfigRules,
    pub merge: MergeFn,
}

#[derive(Debug, Default)]
struct ProjectCache {
    /// Last good fetch.
    project: Option<GhProject>,
    /// Why the most recent fetch failed.
    error: Option<String>,
}

struct State<C> {
    config: Config,
    client: Option<Arc<C>>,
    viewer: Option<Viewer>,
    /// Keyed by lower-cased `owner/name`.
    projects: HashMap<String, ProjectCache>,
    feed: Option<FeedSnapshot>,
    /// T3 Code's threads as last read; `None` when T3 Code is unavailable.
    t3: Option<Vec<T3Link>>,
    /// The last T3 read failure, so a lasting one is logged once.
    t3_error: Option<String>,
    github_health: SourceHealth,
    feed_health: SourceHealth,
    snapshot: Option<Snapshot>,
    /// No GitHub passes before this.
    rate_limited_until: Option<Instant>,
    /// When the most recent GitHub pass started.
    last_github_pass: Option<Instant>,
    /// The first GitHub pass has finished, well or badly. Until then no
    /// snapshot is published: one built from the feed alone would have no
    /// viewer and no PRs, and the UI would show it as signed out.
    github_attempted: bool,
    /// No usable token was found; the user has to sign in.
    sign_in_required: bool,
    /// Why the config file could not be read at startup. While set, nothing
    /// is saved, so the user's file is never overwritten with less.
    config_error: Option<String>,
    /// Self-reviews the app has posted, in memory only. One is dropped as
    /// soon as its PR no longer shows it as posted (a new head, or the gate
    /// moved past the step).
    self_reviews: Vec<PostedSelfReview>,
}

pub struct Engine<G: GitHubConnector, F, E, S, T> {
    github: G,
    feed: F,
    t3: Arc<T>,
    /// A T3 read is in flight; the next cycle does not start another.
    t3_reading: AtomicBool,
    events: E,
    store: S,
    rules: ConfigRules,
    merge: MergeFn,
    state: Mutex<State<G::Client>>,
    /// A full cycle was asked for and has not started yet.
    pending: AtomicBool,
    wake: Notify,
}

impl<G, F, E, S, T> Engine<G, F, E, S, T>
where
    G: GitHubConnector,
    F: FeedSource,
    E: EventSink,
    S: ConfigBackend,
    T: T3Source,
{
    /// Loads the config. If it cannot be read the app still starts, with no
    /// projects, and refuses to save until the file is fixed (see
    /// `update_config`).
    pub fn new(deps: Deps<G, F, E, S, T>) -> Self {
        let (config, config_error) = match deps.store.load() {
            Ok(config) => (config, None),
            Err(e) => {
                log::error!("{e}; starting with no projects");
                let reason = match e {
                    AppError::Config(reason) => reason,
                    other => other.to_string(),
                };
                (Config::default(), Some(reason))
            }
        };
        Self {
            github: deps.github,
            feed: deps.feed,
            t3: Arc::new(deps.t3),
            t3_reading: AtomicBool::new(false),
            events: deps.events,
            store: deps.store,
            rules: deps.rules,
            merge: deps.merge,
            state: Mutex::new(State {
                config,
                client: None,
                viewer: None,
                projects: HashMap::new(),
                feed: None,
                t3: None,
                t3_error: None,
                github_health: SourceHealth::default(),
                feed_health: SourceHealth::default(),
                snapshot: None,
                rate_limited_until: None,
                last_github_pass: None,
                github_attempted: false,
                sign_in_required: false,
                config_error,
                self_reviews: Vec::new(),
            }),
            pending: AtomicBool::new(false),
            wake: Notify::new(),
        }
    }

    /// A panic elsewhere must not wedge the app: recover a poisoned lock.
    fn lock(&self) -> MutexGuard<'_, State<G::Client>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ---- Scheduling ----

    /// Runs forever. Spawn it once on the async runtime.
    pub async fn run(self: Arc<Self>) {
        let mut next_github = Instant::now();
        let mut next_feed = Instant::now();
        loop {
            let due = next_github.min(next_feed);
            if !self.pending.load(Ordering::SeqCst) && due > Instant::now() {
                tokio::select! {
                    biased;
                    () = self.wake.notified() => {}
                    () = tokio::time::sleep_until(due) => {}
                }
            }
            // Every request made up to here is served by this one cycle.
            let requested = self.pending.swap(false, Ordering::SeqCst);
            let now = Instant::now();
            let github = requested || now >= next_github;
            let feed = requested || now >= next_feed;
            if !github && !feed {
                // A leftover wake-up for a request already served.
                continue;
            }
            Arc::clone(&self).cycle(github, feed).await;
            let done = Instant::now();
            if github {
                next_github = done + GITHUB_INTERVAL;
            }
            if feed {
                next_feed = done + FEED_INTERVAL;
            }
        }
    }

    /// Asks for a full cycle as soon as possible and returns at once.
    pub fn request_sync(&self) {
        self.pending.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// Call when the main window gains focus.
    pub fn on_focus(&self) {
        let stale = self
            .lock()
            .last_github_pass
            .is_none_or(|at| at.elapsed() >= FOCUS_STALE_AFTER);
        if stale {
            self.request_sync();
        }
    }

    /// One cycle. The two passes run side by side, each in its own task so a
    /// panic in one source cannot take down the other or the loop, and each
    /// publishes a fresh snapshot as soon as it lands.
    ///
    /// `sync-state` is emitted only around cycles with a GitHub pass (every
    /// requested cycle has one). The 30-second feed-only passes are quiet so
    /// the UI's "Syncing…" indicator does not blink twice a minute.
    async fn cycle(self: Arc<Self>, github: bool, feed: bool) {
        self.refresh_t3();
        let announce = github;
        if announce {
            self.events.sync_state(SyncState { syncing: true });
        }
        let github_task = github.then(|| {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                this.github_pass().await;
                this.lock().github_attempted = true;
                this.rebuild_and_emit();
            })
        });
        let feed_task = feed.then(|| {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                this.feed_pass().await;
                this.rebuild_and_emit();
            })
        });
        for task in [github_task, feed_task].into_iter().flatten() {
            if let Err(e) = task.await {
                log::error!("sync pass failed: {e}");
            }
        }
        if github && !std::mem::replace(&mut self.lock().github_attempted, true) {
            // The GitHub task died before it could publish; do it here so the
            // UI does not wait forever on its first snapshot.
            self.rebuild_and_emit();
        }
        if announce {
            self.events.sync_state(SyncState { syncing: false });
        }
    }

    // ---- Passes ----

    async fn github_pass(&self) {
        let started = Instant::now();
        let started_at = Utc::now();
        {
            let mut st = self.lock();
            if st.rate_limited_until.is_some_and(|until| started < until) {
                return;
            }
            st.rate_limited_until = None;
            st.last_github_pass = Some(started);
        }

        let client = match self.client().await {
            Ok(client) => client,
            Err(e) => {
                self.record_github_error(&e);
                return;
            }
        };

        let (jobs, contributors) = {
            let st = self.lock();
            let jobs: Vec<(String, Vec<u64>)> = st
                .config
                .projects
                .iter()
                .map(|repo| (repo.clone(), followed_numbers(&st.config, repo)))
                .collect();
            let contributors: Arc<[String]> = st.config.contributors.clone().into();
            (jobs, contributors)
        };
        let results = per_project(&client, jobs, |client, repo, followed| {
            let contributors = Arc::clone(&contributors);
            async move { client.fetch_project(&repo, &followed, &contributors).await }
        })
        .await;
        self.apply_github_results(results, started_at);
    }

    async fn feed_pass(&self) {
        let result = self.feed.fetch().await;
        let mut st = self.lock();
        match result {
            Ok(feed) => {
                st.feed = Some(feed);
                st.feed_health = SourceHealth {
                    ok: true,
                    last_success_at: Some(Utc::now()),
                    error: None,
                };
            }
            Err(e) => {
                st.feed_health.ok = false;
                st.feed_health.error = Some(e.to_string());
            }
        }
    }

    /// Reads T3 Code's threads in the background and publishes if they
    /// changed. Not awaited by the cycle: a slow or stuck read delays
    /// nothing, and while one is in flight no other starts. An error or a
    /// panic in the reader means "T3 Code unavailable" and is logged, once
    /// per kind.
    fn refresh_t3(self: &Arc<Self>) {
        if self.t3_reading.swap(true, Ordering::SeqCst) {
            return;
        }
        let repos = t3_repos(&self.lock(), self.rules);
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let source = Arc::clone(&this.t3);
            let joined = tokio::spawn(async move { source.read(&repos).await }).await;
            let (links, error) = match joined {
                Ok(Ok(links)) => (links, None),
                Ok(Err(e)) => (None, Some(format!("could not read T3 Code's threads: {e}"))),
                Err(e) => (None, Some(format!("reading T3 Code's threads failed: {e}"))),
            };
            let changed = {
                let mut st = this.lock();
                if error.is_some() && st.t3_error != error {
                    log::warn!(
                        "{}; T3 Code threads are not shown",
                        error.as_deref().unwrap_or("")
                    );
                }
                st.t3_error = error;
                let changed = st.t3 != links;
                st.t3 = links;
                changed
            };
            this.t3_reading.store(false, Ordering::SeqCst);
            if changed {
                this.rebuild_and_emit();
            }
        });
    }

    /// The current client, or a new one (with the viewer) if there is none.
    async fn client(&self) -> AppResult<Arc<G::Client>> {
        let existing = self.lock().client.clone();
        if let Some(client) = existing {
            return Ok(client);
        }
        let client = Arc::new(self.github.connect().await?);
        let viewer = client.viewer().await?;
        let mut st = self.lock();
        st.client = Some(Arc::clone(&client));
        st.viewer = Some(viewer);
        st.sign_in_required = false;
        Ok(client)
    }

    /// `started_at`: when the pass began asking GitHub.
    fn apply_github_results(
        &self,
        results: Vec<(String, AppResult<GhProject>)>,
        started_at: DateTime<Utc>,
    ) {
        let mut st = self.lock();
        let total = results.len();
        let mut succeeded = 0;
        let mut source_error: Option<AppError> = None;
        let mut first_repo_error: Option<String> = None;

        for (repo, result) in results {
            let tracked = st.config.projects.iter().any(|p| same_repo(p, &repo));
            match result {
                Ok(mut project) => {
                    succeeded += 1;
                    if tracked {
                        // The pass started with the followed and tracked
                        // lists of its time; drop PRs unfollowed, and
                        // contributors untracked, while it ran.
                        let State { viewer, config, .. } = &*st;
                        project.prs.retain(|pr| {
                            is_viewer(viewer.as_ref(), &pr.author)
                                || is_followed(config, &repo, pr.number)
                                || is_tracked(config, &pr.author)
                        });
                        forget_ended_self_reviews(&mut st, &repo, &project, started_at);
                        let cache = st.projects.entry(repo_key(&repo)).or_default();
                        cache.project = Some(project);
                        cache.error = None;
                    }
                }
                Err(e) if is_source_wide(&e) => {
                    source_error.get_or_insert(e);
                }
                Err(e) => {
                    let message = e.to_string();
                    first_repo_error.get_or_insert_with(|| format!("{repo}: {message}"));
                    if tracked {
                        st.projects.entry(repo_key(&repo)).or_default().error = Some(message);
                    }
                }
            }
        }

        if let Some(e) = source_error {
            note_source_error(&mut st, &e);
        } else if total > 0 && succeeded == 0 {
            st.github_health.ok = false;
            st.github_health.error = first_repo_error;
        } else {
            st.github_health = SourceHealth {
                ok: true,
                last_success_at: Some(Utc::now()),
                error: None,
            };
        }
    }

    fn record_github_error(&self, e: &AppError) {
        note_source_error(&mut self.lock(), e);
    }

    /// For errors met while serving a command: auth and rate-limit errors
    /// change what the app shows, so record them and publish at once.
    fn absorb(&self, e: AppError) -> AppError {
        if is_source_wide(&e) {
            self.record_github_error(&e);
            self.lock().github_attempted = true;
            self.rebuild_and_emit();
        }
        e
    }

    // ---- Snapshot ----

    fn rebuild_and_emit(&self) {
        let mut st = self.lock();
        if !st.github_attempted {
            return;
        }
        let snapshot = self.build(&st);
        // Emitting under the lock keeps snapshots in order on the wire.
        self.events.snapshot(&snapshot);
        st.snapshot = Some(snapshot);
    }

    fn build(&self, st: &State<G::Client>) -> Snapshot {
        let projects = st
            .config
            .projects
            .iter()
            .map(|repo| {
                let cache = st.projects.get(&repo_key(repo));
                ProjectInput {
                    repo: repo.clone(),
                    project: cache.and_then(|c| c.project.clone()),
                    error: cache.and_then(|c| c.error.clone()),
                }
            })
            .collect();
        let mut snapshot = (self.merge)(MergeInput {
            now: Utc::now(),
            viewer: st.viewer.clone(),
            config: &st.config,
            projects,
            github: st.github_health.clone(),
            feed: st.feed.as_ref(),
            feed_status: st.feed_health.clone(),
            t3: st.t3.as_deref(),
            self_reviews: &st.self_reviews,
        });
        snapshot.sign_in_required = st.sign_in_required;
        snapshot
    }

    // ---- Commands ----

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.lock().snapshot.clone()
    }

    pub fn config(&self) -> Config {
        self.lock().config.clone()
    }

    /// Applies `change` to a copy of the config, saves it, then commits it.
    /// The lock is held only for in-memory work and a small local file write.
    fn update_config<R>(
        &self,
        change: impl FnOnce(&mut Config) -> AppResult<R>,
    ) -> AppResult<(Config, R)> {
        let mut st = self.lock();
        if let Some(reason) = &st.config_error {
            return Err(AppError::Config(format!(
                "{reason} Nothing was saved, so the file is unchanged. Restart PR Status once it is fixed."
            )));
        }
        let mut next = st.config.clone();
        let result = change(&mut next)?;
        self.store.save(&next)?;
        st.config = next.clone();
        Ok((next, result))
    }

    pub async fn add_project(&self, input: &str) -> AppResult<Config> {
        let repo = (self.rules.parse_repo)(input)?;
        if self
            .lock()
            .config
            .projects
            .iter()
            .any(|p| same_repo(p, &repo))
        {
            return Err(already_tracked(&repo));
        }
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let canonical = client
            .canonical_repo(&repo)
            .await
            .map_err(|e| self.absorb(e))?;
        let (config, ()) = self.update_config(|cfg| {
            if cfg.projects.iter().any(|p| same_repo(p, &canonical)) {
                return Err(already_tracked(&canonical));
            }
            (self.rules.add_project)(cfg, &canonical);
            Ok(())
        })?;
        // Show the new project at once; its PRs follow with the sync.
        self.rebuild_and_emit();
        self.request_sync();
        Ok(config)
    }

    pub fn remove_project(&self, repo: &str) -> AppResult<Config> {
        let (config, _) = self.update_config(|cfg| Ok((self.rules.remove_project)(cfg, repo)))?;
        self.lock().projects.remove(&repo_key(repo));
        self.rebuild_and_emit();
        Ok(config)
    }

    pub fn follow_pr(&self, input: &str) -> AppResult<Config> {
        let (repo, number) = (self.rules.parse_pr_ref)(input)?;
        let (config, _) = self.update_config(|cfg| {
            let tracked = cfg
                .projects
                .iter()
                .find(|p| same_repo(p, &repo))
                .cloned()
                .ok_or_else(|| {
                    AppError::InvalidInput(format!(
                        "{repo} is not one of your projects. Add it first."
                    ))
                })?;
            Ok((self.rules.follow)(cfg, &tracked, number))
        })?;
        self.request_sync();
        Ok(config)
    }

    pub fn unfollow_pr(&self, id: &str) -> AppResult<Config> {
        let (repo, number) = (self.rules.parse_pr_ref)(id)?;
        let (config, _) = self.update_config(|cfg| {
            let tracked = cfg
                .projects
                .iter()
                .find(|p| same_repo(p, &repo))
                .cloned()
                .unwrap_or_else(|| repo.clone());
            Ok((self.rules.unfollow)(cfg, &tracked, number))
        })?;
        {
            let mut st = self.lock();
            let State {
                viewer, projects, ..
            } = &mut *st;
            let cached = projects
                .get_mut(&repo_key(&repo))
                .and_then(|c| c.project.as_mut());
            if let Some(project) = cached {
                project
                    .prs
                    .retain(|pr| pr.number != number || is_viewer(viewer.as_ref(), &pr.author));
            }
        }
        self.rebuild_and_emit();
        self.request_sync();
        Ok(config)
    }

    /// Starts tracking a contributor: their open PRs in the tracked projects
    /// join the snapshot with the next sync, which is asked for at once.
    /// Someone already tracked changes nothing.
    pub async fn track_contributor(&self, input: &str) -> AppResult<Config> {
        let login = (self.rules.parse_login)(input)?;
        if login.eq_ignore_ascii_case(BOT_LOGIN) {
            return Err(AppError::InvalidInput(format!(
                "{BOT_LOGIN} is the review bot, not a person. Its reviews already show on every pull request."
            )));
        }
        {
            let st = self.lock();
            // The rules on a copy: already tracked, or no room for more.
            let mut probe = st.config.clone();
            if !(self.rules.track_contributor)(&mut probe, &login)? {
                return Ok(st.config.clone());
            }
            if is_viewer(st.viewer.as_ref(), &login) {
                return Err(AppError::InvalidInput(TRACK_SELF.to_owned()));
            }
        }
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        if is_viewer(self.lock().viewer.as_ref(), &login) {
            return Err(AppError::InvalidInput(TRACK_SELF.to_owned()));
        }
        let account = client
            .account(&login)
            .await
            .map_err(|e| self.absorb(e))?
            .ok_or_else(|| {
                AppError::InvalidInput(format!("There is no GitHub user named {login}."))
            })?;
        match account.kind {
            GhAccountKind::User => {}
            GhAccountKind::Organization => {
                return Err(AppError::InvalidInput(format!(
                    "{} is an organization, not a person. Only people's pull requests can be tracked.",
                    account.login
                )));
            }
            GhAccountKind::Other => {
                return Err(AppError::InvalidInput(format!(
                    "{} is not a person's account. Only people's pull requests can be tracked.",
                    account.login
                )));
            }
        }
        let (config, added) =
            self.update_config(|cfg| (self.rules.track_contributor)(cfg, &account.login))?;
        if added {
            self.request_sync();
        }
        Ok(config)
    }

    /// Stops tracking a contributor (any casing). Their PRs leave the
    /// snapshot at once, except the ones the user follows or holds. A login
    /// that is not tracked changes nothing.
    pub fn untrack_contributor(&self, login: &str) -> AppResult<Config> {
        let login = login.trim();
        {
            let st = self.lock();
            if !is_tracked(&st.config, login) {
                return Ok(st.config.clone());
            }
        }
        let (config, _) =
            self.update_config(|cfg| Ok((self.rules.untrack_contributor)(cfg, login)))?;
        {
            let mut st = self.lock();
            let State {
                viewer,
                config,
                projects,
                ..
            } = &mut *st;
            for (key, cache) in projects.iter_mut() {
                let Some(project) = cache.project.as_mut() else {
                    continue;
                };
                project.prs.retain(|pr| {
                    !pr.author.eq_ignore_ascii_case(login)
                        || is_viewer(viewer.as_ref(), &pr.author)
                        || is_followed(config, key, pr.number)
                });
            }
        }
        self.rebuild_and_emit();
        Ok(config)
    }

    /// The authors of open PRs across the tracked projects, most open PRs
    /// first, then by login. Leaves out the viewer, GitHub App accounts, the
    /// review bot, deleted accounts and anyone already tracked. A project
    /// that cannot be listed is skipped, unless none can.
    pub async fn known_contributors(&self) -> AppResult<Vec<KnownContributor>> {
        let projects = self.lock().config.projects.clone();
        if projects.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let jobs = projects.into_iter().map(|repo| (repo, ())).collect();
        let results = per_project(&client, jobs, |client, repo, ()| async move {
            client.open_pr_authors(&repo).await
        })
        .await;
        let mut authors: Vec<GhPrAuthor> = Vec::new();
        let mut first_error: Option<AppError> = None;
        let mut listed = 0;
        for (repo, result) in results {
            match result {
                Ok(found) => {
                    listed += 1;
                    authors.extend(found);
                }
                Err(e) => {
                    log::warn!("could not list the open pull requests of {repo}: {e}");
                    first_error.get_or_insert(e);
                }
            }
        }
        if listed == 0
            && let Some(e) = first_error
        {
            return Err(self.absorb(e));
        }
        let st = self.lock();
        Ok(known_from(
            authors,
            st.viewer.as_ref(),
            &st.config.contributors,
        ))
    }

    /// Parses a PR id and finds its repo among the tracked projects, in the
    /// casing the user tracks it under.
    fn tracked_pr(&self, id: &str) -> AppResult<(String, u64)> {
        let (repo, number) = (self.rules.parse_pr_ref)(id)?;
        let tracked = self
            .lock()
            .config
            .projects
            .iter()
            .find(|p| same_repo(p, &repo))
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidInput(format!("{repo} is not one of your projects."))
            })?;
        Ok((tracked, number))
    }

    /// A fresh answer, straight from GitHub, to "can this be squash-merged
    /// now?". Only for PRs in tracked projects.
    pub async fn merge_check(&self, id: &str) -> AppResult<MergeCheck> {
        let (repo, number) = self.tracked_pr(id)?;
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let fresh = client
            .merge_check(&repo, number)
            .await
            .map_err(|e| self.absorb(e))?;
        Ok(merge::merge_check(pr_id(&repo, number), &fresh))
    }

    /// Squash-merges a PR, but only if a fresh check (never the snapshot,
    /// never the UI) finds it open, still at `expected_head_sha`, and ready.
    /// On success the PR leaves the snapshot at once and a sync follows.
    pub async fn squash_merge(&self, id: &str, expected_head_sha: &str) -> AppResult<()> {
        let (repo, number) = self.tracked_pr(id)?;
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let fresh = client
            .merge_check(&repo, number)
            .await
            .map_err(|e| self.absorb(e))?;
        let expected = expected_head_sha.trim();
        let check = merge::merge_check(pr_id(&repo, number), &fresh);
        let refusal = if !fresh.is_open {
            Some(merge::CLOSED_REASON.to_owned())
        } else if expected.is_empty() || !fresh.head_sha.eq_ignore_ascii_case(expected) {
            Some(HEAD_MOVED.to_owned())
        } else if !check.merge.ready {
            Some(
                check
                    .merge
                    .reason
                    .unwrap_or_else(|| "This pull request cannot be merged right now.".to_owned()),
            )
        } else {
            None
        };
        if let Some(reason) = refusal {
            // What the user saw is out of date; bring it up to date.
            self.request_sync();
            return Err(AppError::InvalidInput(reason));
        }
        let result = client
            .squash_merge(&fresh.facts.node_id, &fresh.head_sha)
            .await
            .map_err(|e| self.absorb(e));
        if result.is_ok() {
            let mut st = self.lock();
            let cached = st
                .projects
                .get_mut(&repo_key(&repo))
                .and_then(|c| c.project.as_mut());
            if let Some(project) = cached {
                project.prs.retain(|pr| pr.number != number);
            }
        }
        // Even a failed attempt may have changed things on GitHub.
        self.rebuild_and_emit();
        self.request_sync();
        result
    }

    /// A fresh answer, straight from GitHub, to "can I post the self-review
    /// on this PR now?". Only for PRs in tracked projects. Read-only.
    pub async fn self_review_check(&self, id: &str) -> AppResult<SelfReviewCheck> {
        let (repo, number) = self.tracked_pr(id)?;
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let fresh = client
            .self_review_check(&repo, number)
            .await
            .map_err(|e| self.absorb(e))?;
        Ok(merge::self_review_check(pr_id(&repo, number), &fresh))
    }

    /// Posts `/self-reviewed <head>` on a PR as the user, but only if a fresh
    /// check (never the snapshot, never the UI) finds it open, not a draft,
    /// held by the viewer, and still at `expected_head_sha`. On success the
    /// PR shows the self-review as posted at once, and a sync follows.
    pub async fn post_self_review(&self, id: &str, expected_head_sha: &str) -> AppResult<()> {
        let (repo, number) = self.tracked_pr(id)?;
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let fresh = client
            .self_review_check(&repo, number)
            .await
            .map_err(|e| self.absorb(e))?;
        let expected = expected_head_sha.trim();
        let check = merge::self_review_check(pr_id(&repo, number), &fresh);
        let refusal = if !fresh.is_open {
            Some(merge::SELF_REVIEW_CLOSED.to_owned())
        } else if expected.is_empty() || !fresh.head_sha.eq_ignore_ascii_case(expected) {
            Some(SELF_REVIEW_HEAD_MOVED.to_owned())
        } else if !check.can_post {
            Some(check.reason.clone().unwrap_or_else(|| {
                "The self-review cannot be posted on this pull request right now.".to_owned()
            }))
        } else {
            None
        };
        if let Some(reason) = refusal {
            // What the user saw is out of date; bring it up to date.
            self.request_sync();
            return Err(AppError::InvalidInput(reason));
        }
        // Built again from GitHub's head, never from what the UI sent.
        let comment = merge::self_review_comment(&fresh.head_sha)?;
        let result = client
            .add_comment(&fresh.node_id, &comment)
            .await
            .map_err(|e| self.absorb(e));
        if result.is_ok() {
            self.lock().self_reviews.push(PostedSelfReview {
                repo: repo.clone(),
                number,
                head_sha: fresh.head_sha.clone(),
                at: Utc::now(),
            });
        }
        // Even a failed attempt may have changed things on GitHub.
        self.rebuild_and_emit();
        self.request_sync();
        result
    }

    pub async fn list_open_prs(&self, repo: &str) -> AppResult<Vec<PrSummary>> {
        let client = self.client().await.map_err(|e| self.absorb(e))?;
        let mut prs = client
            .list_open_prs(repo)
            .await
            .map_err(|e| self.absorb(e))?;
        let st = self.lock();
        for pr in &mut prs {
            pr.is_mine = is_viewer(st.viewer.as_ref(), &pr.author);
            pr.is_followed = is_followed(&st.config, &pr.repo, pr.number);
        }
        Ok(prs)
    }
}

/// Runs `work` for every project, at most `MAX_CONCURRENT_FETCHES` at a
/// time. Each runs in its own task, so a panic is reported against its repo.
async fn per_project<C, J, T, W, Fut>(
    client: &Arc<C>,
    jobs: Vec<(String, J)>,
    work: W,
) -> Vec<(String, AppResult<T>)>
where
    C: GitHubApi,
    J: Send + 'static,
    T: Send + 'static,
    W: Fn(Arc<C>, String, J) -> Fut,
    Fut: Future<Output = AppResult<T>> + Send + 'static,
{
    let mut results = Vec::with_capacity(jobs.len());
    let mut names = HashMap::new();
    let mut set = JoinSet::new();
    let mut queue = jobs.into_iter();
    loop {
        while set.len() < MAX_CONCURRENT_FETCHES {
            let Some((repo, job)) = queue.next() else {
                break;
            };
            let handle = set.spawn(work(Arc::clone(client), repo.clone(), job));
            names.insert(handle.id(), repo);
        }
        let Some(joined) = set.join_next_with_id().await else {
            break;
        };
        match joined {
            Ok((id, result)) => {
                if let Some(repo) = names.remove(&id) {
                    results.push((repo, result));
                }
            }
            Err(e) => {
                if let Some(repo) = names.remove(&e.id()) {
                    log::error!("working on {repo} failed: {e}");
                    results.push((repo, Err(AppError::Other("internal error".into()))));
                }
            }
        }
    }
    results
}

/// Counts open PRs per author (one entry per PR) into the people the app
/// can offer to track.
fn known_from(
    authors: Vec<GhPrAuthor>,
    viewer: Option<&Viewer>,
    tracked: &[String],
) -> Vec<KnownContributor> {
    let mut by_login: HashMap<String, KnownContributor> = HashMap::new();
    for author in authors {
        let skip = author.is_bot
            || author.login.is_empty()
            || author.login.eq_ignore_ascii_case("ghost")
            || author.login.eq_ignore_ascii_case(BOT_LOGIN)
            || is_viewer(viewer, &author.login)
            || tracked
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&author.login));
        if skip {
            continue;
        }
        let entry = by_login
            .entry(author.login.to_ascii_lowercase())
            .or_insert_with(|| KnownContributor {
                login: author.login.clone(),
                avatar_url: None,
                open_prs: 0,
            });
        entry.open_prs += 1;
        if entry.avatar_url.is_none() {
            entry.avatar_url = author.avatar_url;
        }
    }
    let mut out: Vec<KnownContributor> = by_login.into_values().collect();
    out.sort_by(|a, b| {
        b.open_prs
            .cmp(&a.open_prs)
            .then_with(|| {
                a.login
                    .to_ascii_lowercase()
                    .cmp(&b.login.to_ascii_lowercase())
            })
            .then_with(|| a.login.cmp(&b.login))
    });
    out
}

/// Drops each self-review the app posted in `repo` that a fresh fetch of
/// the project no longer shows as posted: the head moved, the gate moved
/// past the step, or the PR left the list. Only a fetch that started after
/// the post can say so. Once dropped it stays dropped, so a gate that asks
/// again later is shown as asking.
fn forget_ended_self_reviews<C>(
    st: &mut State<C>,
    repo: &str,
    project: &GhProject,
    started_at: DateTime<Utc>,
) {
    let State {
        self_reviews,
        viewer,
        ..
    } = st;
    let viewer = viewer.as_ref().map(|v| v.login.as_str());
    self_reviews.retain(|posted| {
        if !same_repo(&posted.repo, repo) || posted.at >= started_at {
            return true;
        }
        project
            .prs
            .iter()
            .find(|pr| pr.number == posted.number)
            .is_some_and(|pr| {
                let shown = merge::self_review(pr, repo, viewer, std::slice::from_ref(posted));
                shown.state == SelfReviewState::Posted
            })
    });
}

/// Errors that are about GitHub as a whole rather than one repo.
fn is_source_wide(e: &AppError) -> bool {
    matches!(
        e,
        AppError::Unauthorized(_) | AppError::NotSignedIn(_) | AppError::RateLimited { .. }
    )
}

/// Records a failed GitHub call in the state. Auth errors drop the client so
/// the next pass looks for a token again; rate limits pause GitHub passes.
fn note_source_error<C>(st: &mut State<C>, e: &AppError) {
    match e {
        AppError::Unauthorized(_) | AppError::NotSignedIn(_) => {
            st.client = None;
            st.viewer = None;
            st.sign_in_required = true;
        }
        AppError::RateLimited { reset_at } => {
            st.rate_limited_until = Some(Instant::now() + rate_limit_wait(reset_at, Utc::now()));
        }
        _ => {}
    }
    st.github_health.ok = false;
    st.github_health.error = Some(e.to_string());
}

/// How long to wait for a rate-limit reset given as RFC 3339 or Unix seconds.
fn rate_limit_wait(reset_at: &str, now: DateTime<Utc>) -> Duration {
    let reset_at = reset_at.trim();
    let at = DateTime::parse_from_rfc3339(reset_at)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            reset_at
                .parse::<i64>()
                .ok()
                .and_then(|secs| DateTime::from_timestamp(secs, 0))
        });
    match at {
        Some(at) => (at - now)
            .to_std()
            .unwrap_or(Duration::ZERO)
            .min(RATE_LIMIT_MAX),
        None => RATE_LIMIT_FALLBACK,
    }
}

fn already_tracked(repo: &str) -> AppError {
    AppError::InvalidInput(format!("{repo} is already in your projects."))
}

/// The repos whose T3 threads matter: the tracked projects, the repos of
/// followed PRs, and each project's name on GitHub when it differs (after a
/// rename). Unique ignoring case, in that order.
fn t3_repos<C>(st: &State<C>, rules: ConfigRules) -> Vec<String> {
    let followed = st
        .config
        .followed
        .iter()
        .filter_map(|id| (rules.parse_pr_ref)(id).ok().map(|(repo, _)| repo));
    let canonical = st.config.projects.iter().filter_map(|repo| {
        let cache = st.projects.get(&repo_key(repo))?;
        cache.project.as_ref().map(|p| p.repo.clone())
    });
    let mut repos: Vec<String> = Vec::new();
    for repo in st
        .config
        .projects
        .iter()
        .cloned()
        .chain(followed)
        .chain(canonical)
    {
        if !repos.iter().any(|r| same_repo(r, &repo)) {
            repos.push(repo);
        }
    }
    repos
}

fn repo_key(repo: &str) -> String {
    repo.to_ascii_lowercase()
}

fn same_repo(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn is_viewer(viewer: Option<&Viewer>, login: &str) -> bool {
    viewer.is_some_and(|v| v.login.eq_ignore_ascii_case(login))
}

/// Splits a canonical PR id (`owner/name#123`).
fn split_pr_id(id: &str) -> Option<(&str, u64)> {
    let (repo, number) = id.rsplit_once('#')?;
    Some((repo, number.parse().ok()?))
}

fn is_tracked(config: &Config, login: &str) -> bool {
    config
        .contributors
        .iter()
        .any(|c| c.eq_ignore_ascii_case(login))
}

fn followed_numbers(config: &Config, repo: &str) -> Vec<u64> {
    config
        .followed
        .iter()
        .filter_map(|id| split_pr_id(id))
        .filter(|(r, _)| same_repo(r, repo))
        .map(|(_, n)| n)
        .collect()
}

fn is_followed(config: &Config, repo: &str, number: u64) -> bool {
    config
        .followed
        .iter()
        .filter_map(|id| split_pr_id(id))
        .any(|(r, n)| n == number && same_repo(r, repo))
}
