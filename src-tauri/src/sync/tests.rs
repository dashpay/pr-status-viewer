//! Engine tests with fake sources and tokio's paused clock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use pretty_assertions::assert_eq;

use super::source::{
    ConfigBackend, ConfigRules, EventSink, FeedSource, GitHubApi, GitHubConnector, T3Source,
};
use super::{Deps, Engine, HEAD_MOVED, SELF_REVIEW_HEAD_MOVED, TRACK_SELF, rate_limit_wait};
use crate::error::{AppError, AppResult};
use crate::feed::FeedSnapshot;
use crate::github::types::{
    GhAccount, GhAccountKind, GhComment, GhHygiene, GhMergeCheck, GhMergeFacts, GhMergeable,
    GhPrAuthor, GhProject, GhPullRequest, GhSelfReviewCheck,
};
use crate::merge::{
    CLOSED_REASON, MergeInput, SELF_REVIEW_CLOSED, SELF_REVIEW_DRAFT, SELF_REVIEW_NOT_HOLDING,
};
use crate::model::{
    BotReview, CiState, Config, FeedHealth, KnownContributor, PrSummary, ProjectSnapshot,
    PullRequest, SelfReview, SelfReviewState, Snapshot, SyncState, T3ThreadState, Viewer, pr_id,
};
use crate::t3::T3Link;

// ---- Fakes ----

#[derive(Clone, Copy, Debug)]
enum Fail {
    Network,
    Unauthorized,
    NotSignedIn,
    RateLimited,
}

impl Fail {
    fn error(self) -> AppError {
        match self {
            Fail::Network => AppError::Network("connection reset".into()),
            Fail::Unauthorized => AppError::Unauthorized("bad credentials".into()),
            Fail::NotSignedIn => AppError::NotSignedIn("run gh auth login".into()),
            Fail::RateLimited => AppError::RateLimited {
                reset_at: (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
            },
        }
    }
}

/// A PR's head commit and its PR Hygiene description.
type Gate = (String, Option<String>);

#[derive(Default)]
struct GitHubWorld {
    /// When set, `connect` fails with it.
    connect_fail: Mutex<Option<Fail>>,
    /// Per-repo failure (lower-cased key). Absent repos succeed.
    repo_fail: Mutex<HashMap<String, Fail>>,
    /// PRs each repo returns: (number, author).
    repo_prs: Mutex<HashMap<String, Vec<(u64, String)>>>,
    /// Canonical casing for repos that exist (lower-cased key).
    canonical: Mutex<HashMap<String, String>>,
    /// `fetch_project` answers with this name instead (a renamed repo).
    renamed: Mutex<HashMap<String, String>>,
    open_prs: Mutex<Vec<PrSummary>>,
    fetch_delay: Mutex<Duration>,
    /// Fresh merge checks by (lower-cased repo, number).
    merge_checks: Mutex<HashMap<(String, u64), GhMergeCheck>>,
    merge_check_calls: AtomicUsize,
    /// Every `squash_merge` call: (node id, expected head).
    merges: Mutex<Vec<(String, String)>>,
    /// When set, `squash_merge` fails with it.
    merge_fail: Mutex<Option<String>>,
    /// Head commit and PR Hygiene description of PRs the project fetch
    /// returns, by (lower-cased repo, number). Others: head "abc", no gate.
    gates: Mutex<HashMap<(String, u64), Gate>>,
    /// Fresh self-review checks by (lower-cased repo, number).
    self_review_checks: Mutex<HashMap<(String, u64), GhSelfReviewCheck>>,
    self_review_check_calls: AtomicUsize,
    /// Every `add_comment` call: (subject id, body).
    comments: Mutex<Vec<(String, String)>>,
    /// When set, `add_comment` fails with it.
    comment_fail: Mutex<Option<String>>,
    /// Accounts that exist, by lower-cased login.
    accounts: Mutex<HashMap<String, GhAccount>>,
    account_calls: AtomicUsize,
    /// The authors of each repo's open PRs (lower-cased key).
    open_authors: Mutex<HashMap<String, Vec<GhPrAuthor>>>,
    /// The contributors each project fetch was asked for.
    asked_contributors: Mutex<Vec<Vec<String>>>,
    connects: AtomicUsize,
    fetches: AtomicUsize,
    /// Every network call of any kind.
    calls: AtomicUsize,
}

impl GitHubWorld {
    fn set_repo_fail(&self, repo: &str, fail: Option<Fail>) {
        let mut map = self.repo_fail.lock().unwrap();
        match fail {
            Some(f) => map.insert(repo.to_ascii_lowercase(), f),
            None => map.remove(&repo.to_ascii_lowercase()),
        };
    }

    fn set_prs(&self, repo: &str, prs: &[(u64, &str)]) {
        self.repo_prs.lock().unwrap().insert(
            repo.to_ascii_lowercase(),
            prs.iter().map(|(n, a)| (*n, (*a).to_string())).collect(),
        );
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    fn set_merge_check(&self, repo: &str, number: u64, check: GhMergeCheck) {
        self.merge_checks
            .lock()
            .unwrap()
            .insert((repo.to_ascii_lowercase(), number), check);
    }

    fn merges(&self) -> Vec<(String, String)> {
        self.merges.lock().unwrap().clone()
    }

    fn set_gate(&self, repo: &str, number: u64, head: &str, description: Option<&str>) {
        self.gates.lock().unwrap().insert(
            (repo.to_ascii_lowercase(), number),
            (head.to_owned(), description.map(str::to_owned)),
        );
    }

    fn set_self_review_check(&self, repo: &str, number: u64, check: GhSelfReviewCheck) {
        self.self_review_checks
            .lock()
            .unwrap()
            .insert((repo.to_ascii_lowercase(), number), check);
    }

    fn comments(&self) -> Vec<(String, String)> {
        self.comments.lock().unwrap().clone()
    }

    fn add_account(&self, login: &str, kind: GhAccountKind) {
        self.accounts.lock().unwrap().insert(
            login.to_ascii_lowercase(),
            GhAccount {
                login: login.to_owned(),
                avatar_url: Some(format!("https://avatars.test/{login}")),
                kind,
            },
        );
    }

    fn set_open_authors(&self, repo: &str, authors: &[(&str, bool)]) {
        self.open_authors.lock().unwrap().insert(
            repo.to_ascii_lowercase(),
            authors
                .iter()
                .map(|(login, is_bot)| GhPrAuthor {
                    login: (*login).to_owned(),
                    avatar_url: Some(format!("https://avatars.test/{login}")),
                    is_bot: *is_bot,
                })
                .collect(),
        );
    }

    fn account_calls(&self) -> usize {
        self.account_calls.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct FakeConnector(Arc<GitHubWorld>);

struct FakeClient(Arc<GitHubWorld>);

impl GitHubConnector for FakeConnector {
    type Client = FakeClient;

    async fn connect(&self) -> AppResult<FakeClient> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.connects.fetch_add(1, Ordering::SeqCst);
        if let Some(fail) = *self.0.connect_fail.lock().unwrap() {
            return Err(fail.error());
        }
        Ok(FakeClient(Arc::clone(&self.0)))
    }
}

impl GitHubApi for FakeClient {
    async fn viewer(&self) -> AppResult<Viewer> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Viewer {
            login: "Alice".into(),
            avatar_url: None,
        })
    }

    async fn canonical_repo(&self, repo: &str) -> AppResult<String> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .canonical
            .lock()
            .unwrap()
            .get(&repo.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("{repo} does not exist")))
    }

    async fn account(&self, login: &str) -> AppResult<Option<GhAccount>> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.account_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(fail) = *self.0.connect_fail.lock().unwrap() {
            return Err(fail.error());
        }
        Ok(self
            .0
            .accounts
            .lock()
            .unwrap()
            .get(&login.to_ascii_lowercase())
            .cloned())
    }

    async fn open_pr_authors(&self, repo: &str) -> AppResult<Vec<GhPrAuthor>> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        let key = repo.to_ascii_lowercase();
        if let Some(fail) = self.0.repo_fail.lock().unwrap().get(&key) {
            return Err(fail.error());
        }
        Ok(self
            .0
            .open_authors
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_default())
    }

    async fn fetch_project(
        &self,
        repo: &str,
        followed: &[u64],
        contributors: &[String],
    ) -> AppResult<GhProject> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.fetches.fetch_add(1, Ordering::SeqCst);
        self.0
            .asked_contributors
            .lock()
            .unwrap()
            .push(contributors.to_vec());
        let delay = *self.0.fetch_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let key = repo.to_ascii_lowercase();
        if let Some(fail) = self.0.repo_fail.lock().unwrap().get(&key) {
            return Err(fail.error());
        }
        let prs: Vec<GhPullRequest> = self
            .0
            .repo_prs
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|(n, author)| {
                author.eq_ignore_ascii_case("alice")
                    || followed.contains(n)
                    || contributors.iter().any(|c| c.eq_ignore_ascii_case(author))
            })
            .map(|(n, author)| {
                let mut pr = gh_pr(repo, n, &author);
                if let Some((head, description)) =
                    self.0.gates.lock().unwrap().get(&(key.clone(), n))
                {
                    pr.head_sha.clone_from(head);
                    pr.hygiene = Some(GhHygiene {
                        state: "PENDING".into(),
                        description: description.clone(),
                    });
                }
                pr
            })
            .collect();
        let renamed = self.0.renamed.lock().unwrap().get(&key).cloned();
        Ok(GhProject {
            repo: renamed.unwrap_or_else(|| repo.to_string()),
            open_count: 10,
            prs,
        })
    }

    async fn list_open_prs(&self, _repo: &str) -> AppResult<Vec<PrSummary>> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.0.open_prs.lock().unwrap().clone())
    }

    async fn merge_check(&self, repo: &str, number: u64) -> AppResult<GhMergeCheck> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.merge_check_calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .merge_checks
            .lock()
            .unwrap()
            .get(&(repo.to_ascii_lowercase(), number))
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("{repo}#{number} does not exist")))
    }

    async fn squash_merge(&self, pr_node_id: &str, expected_head_oid: &str) -> AppResult<()> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .merges
            .lock()
            .unwrap()
            .push((pr_node_id.to_string(), expected_head_oid.to_string()));
        match self.0.merge_fail.lock().unwrap().clone() {
            Some(message) => Err(AppError::Other(format!(
                "GitHub refused the merge: {message}"
            ))),
            None => Ok(()),
        }
    }

    async fn self_review_check(&self, repo: &str, number: u64) -> AppResult<GhSelfReviewCheck> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .self_review_check_calls
            .fetch_add(1, Ordering::SeqCst);
        self.0
            .self_review_checks
            .lock()
            .unwrap()
            .get(&(repo.to_ascii_lowercase(), number))
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("{repo}#{number} does not exist")))
    }

    async fn add_comment(&self, subject_id: &str, body: &str) -> AppResult<()> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0
            .comments
            .lock()
            .unwrap()
            .push((subject_id.to_string(), body.to_string()));
        match self.0.comment_fail.lock().unwrap().clone() {
            Some(message) => Err(AppError::Other(format!(
                "GitHub did not post the comment: {message}"
            ))),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
enum T3Mode {
    /// T3 Code is not installed.
    Absent,
    Links(Vec<T3Link>),
    Error,
    Panic,
}

struct T3World {
    mode: Mutex<T3Mode>,
    reads: AtomicUsize,
    /// The repos each read was asked for.
    asked: Mutex<Vec<Vec<String>>>,
}

impl Default for T3World {
    fn default() -> Self {
        Self {
            mode: Mutex::new(T3Mode::Absent),
            reads: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
        }
    }
}

impl T3World {
    fn set(&self, mode: T3Mode) {
        *self.mode.lock().unwrap() = mode;
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct FakeT3(Arc<T3World>);

impl T3Source for FakeT3 {
    async fn read(&self, repos: &[String]) -> AppResult<Option<Vec<T3Link>>> {
        self.0.reads.fetch_add(1, Ordering::SeqCst);
        self.0.asked.lock().unwrap().push(repos.to_vec());
        let mode = self.0.mode.lock().unwrap().clone();
        match mode {
            T3Mode::Absent => Ok(None),
            T3Mode::Links(links) => Ok(Some(links)),
            T3Mode::Error => Err(AppError::Other("no such table: projection_threads".into())),
            T3Mode::Panic => panic!("the T3 reader fell over"),
        }
    }
}

#[derive(Default)]
struct FeedWorld {
    fail: Mutex<bool>,
    fetches: AtomicUsize,
}

#[derive(Clone)]
struct FakeFeed(Arc<FeedWorld>);

impl FeedSource for FakeFeed {
    async fn fetch(&self) -> AppResult<FeedSnapshot> {
        self.0.fetches.fetch_add(1, Ordering::SeqCst);
        if *self.0.fail.lock().unwrap() {
            Err(AppError::Network("feed down".into()))
        } else {
            Ok(FeedSnapshot::default())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Snapshot(Box<Snapshot>),
    Syncing(bool),
}

#[derive(Default)]
struct Events(Mutex<Vec<Event>>);

#[derive(Clone)]
struct FakeSink(Arc<Events>);

impl EventSink for FakeSink {
    fn snapshot(&self, snapshot: &Snapshot) {
        self.0
            .0
            .lock()
            .unwrap()
            .push(Event::Snapshot(Box::new(snapshot.clone())));
    }

    fn sync_state(&self, state: SyncState) {
        self.0.0.lock().unwrap().push(Event::Syncing(state.syncing));
    }
}

impl Events {
    fn all(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    fn syncing(&self) -> Vec<bool> {
        self.all()
            .into_iter()
            .filter_map(|e| match e {
                Event::Syncing(s) => Some(s),
                Event::Snapshot(_) => None,
            })
            .collect()
    }

    fn snapshots(&self) -> usize {
        self.all()
            .iter()
            .filter(|e| matches!(e, Event::Snapshot(_)))
            .count()
    }

    fn last_snapshot(&self) -> Option<Snapshot> {
        self.all().into_iter().rev().find_map(|e| match e {
            Event::Snapshot(s) => Some(*s),
            Event::Syncing(_) => None,
        })
    }
}

#[derive(Default)]
struct Disk {
    config: Mutex<Config>,
    saves: AtomicUsize,
    /// When set, `load` fails with it (an unreadable config file).
    load_error: Mutex<Option<String>>,
}

#[derive(Clone)]
struct FakeStore(Arc<Disk>);

impl ConfigBackend for FakeStore {
    fn load(&self) -> AppResult<Config> {
        if let Some(reason) = self.0.load_error.lock().unwrap().clone() {
            return Err(AppError::Config(reason));
        }
        Ok(self.0.config.lock().unwrap().clone())
    }

    fn save(&self, config: &Config) -> AppResult<()> {
        self.0.saves.fetch_add(1, Ordering::SeqCst);
        *self.0.config.lock().unwrap() = config.clone();
        Ok(())
    }
}

// Simple stand-ins for `crate::config`'s rules.

fn fake_parse_repo(input: &str) -> AppResult<String> {
    let s = input.trim().trim_start_matches("https://github.com/");
    match s.split('/').collect::<Vec<_>>().as_slice() {
        [owner, name] if !owner.is_empty() && !name.is_empty() => Ok(format!("{owner}/{name}")),
        _ => Err(AppError::InvalidInput(format!("{input} is not a repo"))),
    }
}

fn fake_parse_pr_ref(input: &str) -> AppResult<(String, u64)> {
    super::split_pr_id(input.trim())
        .map(|(r, n)| (r.to_string(), n))
        .ok_or_else(|| AppError::InvalidInput(format!("{input} is not a PR")))
}

fn fake_add_project(cfg: &mut Config, repo: &str) -> bool {
    if cfg.projects.iter().any(|p| p.eq_ignore_ascii_case(repo)) {
        return false;
    }
    cfg.projects.push(repo.to_string());
    true
}

fn fake_remove_project(cfg: &mut Config, repo: &str) -> bool {
    let before = cfg.projects.len();
    cfg.projects.retain(|p| !p.eq_ignore_ascii_case(repo));
    cfg.followed
        .retain(|id| !super::split_pr_id(id).is_some_and(|(r, _)| r.eq_ignore_ascii_case(repo)));
    cfg.projects.len() != before
}

fn fake_follow(cfg: &mut Config, repo: &str, number: u64) -> bool {
    let id = pr_id(repo, number);
    if cfg.followed.iter().any(|f| f.eq_ignore_ascii_case(&id)) {
        return false;
    }
    cfg.followed.push(id);
    true
}

fn fake_unfollow(cfg: &mut Config, repo: &str, number: u64) -> bool {
    let id = pr_id(repo, number);
    let before = cfg.followed.len();
    cfg.followed.retain(|f| !f.eq_ignore_ascii_case(&id));
    cfg.followed.len() != before
}

fn fake_parse_login(input: &str) -> AppResult<String> {
    let s = input.trim();
    let s = s.strip_prefix('@').unwrap_or(s);
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::InvalidInput(format!("{input} is not a login")));
    }
    Ok(s.to_owned())
}

/// The fake rules allow three contributors.
const FAKE_MAX_CONTRIBUTORS: usize = 3;

fn fake_track_contributor(cfg: &mut Config, login: &str) -> AppResult<bool> {
    if cfg
        .contributors
        .iter()
        .any(|c| c.eq_ignore_ascii_case(login))
    {
        return Ok(false);
    }
    if cfg.contributors.len() >= FAKE_MAX_CONTRIBUTORS {
        return Err(AppError::InvalidInput(format!(
            "You can track at most {FAKE_MAX_CONTRIBUTORS} contributors."
        )));
    }
    cfg.contributors.push(login.to_owned());
    Ok(true)
}

fn fake_untrack_contributor(cfg: &mut Config, login: &str) -> bool {
    let before = cfg.contributors.len();
    cfg.contributors.retain(|c| !c.eq_ignore_ascii_case(login));
    cfg.contributors.len() != before
}

const FAKE_RULES: ConfigRules = ConfigRules {
    parse_repo: fake_parse_repo,
    parse_pr_ref: fake_parse_pr_ref,
    add_project: fake_add_project,
    remove_project: fake_remove_project,
    follow: fake_follow,
    unfollow: fake_unfollow,
    parse_login: fake_parse_login,
    track_contributor: fake_track_contributor,
    untrack_contributor: fake_untrack_contributor,
};

/// A stand-in for `merge::build_snapshot` that keeps the facts tests check.
fn fake_merge(input: MergeInput<'_>) -> Snapshot {
    let login = input.viewer.as_ref().map(|v| v.login.to_ascii_lowercase());
    let links = input.t3.unwrap_or_default();
    let posted = input.self_reviews;
    Snapshot {
        generated_at: input.now,
        sign_in_required: false,
        t3_available: input.t3.is_some(),
        viewer: input.viewer.clone(),
        projects: input
            .projects
            .into_iter()
            .map(|p| ProjectSnapshot {
                repo: p.repo,
                open_count: p.project.as_ref().map_or(0, |gh| gh.open_count),
                prs: p
                    .project
                    .map(|gh| {
                        gh.prs
                            .into_iter()
                            .map(|pr| {
                                let is_mine = login.as_deref()
                                    == Some(pr.author.to_ascii_lowercase().as_str());
                                let mut out = pull_request(&pr, is_mine);
                                out.t3_threads =
                                    crate::merge::t3_threads(links, &pr.repo, &pr.repo, pr.number);
                                out.self_review = crate::merge::self_review(
                                    &pr,
                                    &pr.repo,
                                    login.as_deref(),
                                    posted,
                                );
                                out
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                error: p.error,
            })
            .collect(),
        github: input.github,
        feed: FeedHealth {
            ok: input.feed_status.ok,
            last_success_at: input.feed_status.last_success_at,
            error: input.feed_status.error,
            // Records whether a feed was handed to merge at all.
            degraded: input.feed.is_some(),
            ..FeedHealth::default()
        },
    }
}

fn gh_pr(repo: &str, number: u64, author: &str) -> GhPullRequest {
    let t = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
    GhPullRequest {
        repo: repo.to_string(),
        number,
        title: format!("PR {number}"),
        url: format!("https://github.com/{repo}/pull/{number}"),
        author: author.to_string(),
        author_avatar_url: None,
        assignees: Vec::new(),
        milestone: None,
        is_draft: false,
        additions: 1,
        deletions: 1,
        created_at: t,
        updated_at: t,
        head_sha: "abc".into(),
        head_branch: None,
        head_from_fork: false,
        review_decision: None,
        reviews: vec![],
        review_requests: vec![],
        ci: CiState::None,
        bot_check: None,
        hygiene: None,
        findings: Vec::new(),
        last_rounds: Default::default(),
        traits: Vec::new(),
        rounds: None,
        merge: Default::default(),
    }
}

fn pull_request(pr: &GhPullRequest, is_mine: bool) -> PullRequest {
    PullRequest {
        id: pr_id(&pr.repo, pr.number),
        repo: pr.repo.clone(),
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        author: pr.author.clone(),
        author_avatar_url: None,
        milestone: None,
        is_draft: pr.is_draft,
        is_mine,
        is_followed: !is_mine,
        additions: pr.additions,
        deletions: pr.deletions,
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        head_sha: pr.head_sha.clone(),
        head_branch: pr.head_branch.clone(),
        ci: pr.ci,
        review_decision: None,
        humans: vec![],
        bot: BotReview::None,
        rounds: pr.rounds,
        next_step: None,
        last_review_at: None,
        finding_counts: Default::default(),
        traits: Vec::new(),
        base_branch: String::new(),
        stacked_on: None,
        unresolved_threads: 0,
        merge: Default::default(),
        has_conflicts: false,
        self_review: Default::default(),
        t3_threads: Vec::new(),
        needs_me: false,
        needs_me_reason: None,
    }
}

fn summary(repo: &str, number: u64, author: &str) -> PrSummary {
    PrSummary {
        id: pr_id(repo, number),
        repo: repo.to_string(),
        number,
        title: format!("PR {number}"),
        url: format!("https://github.com/{repo}/pull/{number}"),
        author: author.to_string(),
        is_draft: false,
        is_mine: false,
        is_followed: false,
        updated_at: Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap(),
    }
}

// ---- Harness ----

type TestEngine = Engine<FakeConnector, FakeFeed, FakeSink, FakeStore, FakeT3>;

struct Harness {
    engine: Arc<TestEngine>,
    gh: Arc<GitHubWorld>,
    feed: Arc<FeedWorld>,
    t3: Arc<T3World>,
    events: Arc<Events>,
    disk: Arc<Disk>,
}

impl Harness {
    fn new(projects: &[&str], followed: &[&str]) -> Self {
        Self::tracking(projects, followed, &[])
    }

    fn tracking(projects: &[&str], followed: &[&str], contributors: &[&str]) -> Self {
        let disk = Arc::new(Disk::default());
        *disk.config.lock().unwrap() = Config {
            projects: projects.iter().map(|s| s.to_string()).collect(),
            followed: followed.iter().map(|s| s.to_string()).collect(),
            contributors: contributors.iter().map(|s| s.to_string()).collect(),
        };
        Self::with_disk(disk)
    }

    fn with_disk(disk: Arc<Disk>) -> Self {
        let gh = Arc::new(GitHubWorld::default());
        let feed = Arc::new(FeedWorld::default());
        let t3 = Arc::new(T3World::default());
        let events = Arc::new(Events::default());
        let engine = Arc::new(Engine::new(Deps {
            github: FakeConnector(Arc::clone(&gh)),
            feed: FakeFeed(Arc::clone(&feed)),
            t3: FakeT3(Arc::clone(&t3)),
            events: FakeSink(Arc::clone(&events)),
            store: FakeStore(Arc::clone(&disk)),
            rules: FAKE_RULES,
            merge: fake_merge,
        }));
        Self {
            engine,
            gh,
            feed,
            t3,
            events,
            disk,
        }
    }

    fn start(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(Arc::clone(&self.engine).run())
    }

    fn snapshot(&self) -> Snapshot {
        self.engine.snapshot().expect("a snapshot")
    }

    fn project(&self, repo: &str) -> ProjectSnapshot {
        self.snapshot()
            .projects
            .into_iter()
            .find(|p| p.repo == repo)
            .expect("project in snapshot")
    }

    fn pr_numbers(&self, repo: &str) -> Vec<u64> {
        let mut n: Vec<u64> = self.project(repo).prs.iter().map(|p| p.number).collect();
        n.sort_unstable();
        n
    }
}

/// Lets every ready task run and the paused clock move on by `secs`.
async fn wait(secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

// ---- Tests ----

#[tokio::test(start_paused = true)]
async fn first_cycle_produces_a_snapshot_and_two_sync_states() {
    let h = Harness::new(&["acme/app", "acme/lib"], &["acme/lib#7"]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "bob")]);
    h.gh.set_prs("acme/lib", &[(7, "carol"), (8, "dave")]);
    assert_eq!(h.engine.snapshot(), None);

    let _run = h.start();
    wait(1).await;

    assert_eq!(h.events.syncing(), vec![true, false]);
    // The feed lands first and stays quiet; the GitHub pass publishes both.
    assert_eq!(h.events.snapshots(), 1);
    assert_eq!(h.events.last_snapshot(), h.engine.snapshot());

    let snap = h.snapshot();
    assert_eq!(snap.viewer.map(|v| v.login), Some("Alice".to_string()));
    assert!(snap.github.ok);
    assert!(snap.github.last_success_at.is_some());
    assert!(snap.feed.ok);
    assert_eq!(
        snap.projects
            .iter()
            .map(|p| p.repo.as_str())
            .collect::<Vec<_>>(),
        vec!["acme/app", "acme/lib"]
    );
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
    assert_eq!(h.pr_numbers("acme/lib"), vec![7]);
    assert_eq!(h.gh.connects.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn polls_feed_every_30s_and_github_every_90s() {
    let h = Harness::new(&["acme/app"], &[]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(
        (h.gh.fetches(), h.feed.fetches.load(Ordering::SeqCst)),
        (1, 1)
    );

    wait(60).await; // t = 61
    assert_eq!(
        (h.gh.fetches(), h.feed.fetches.load(Ordering::SeqCst)),
        (1, 3)
    );

    wait(30).await; // t = 91
    assert_eq!(
        (h.gh.fetches(), h.feed.fetches.load(Ordering::SeqCst)),
        (2, 4)
    );
    // Connects once and reuses the client.
    assert_eq!(h.gh.connects.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn feed_only_passes_do_not_announce_syncing() {
    let h = Harness::new(&["acme/app"], &[]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.events.syncing(), vec![true, false]);
    let snapshots = h.events.snapshots();

    wait(60).await; // t = 61: two feed-only passes
    assert_eq!(h.feed.fetches.load(Ordering::SeqCst), 3);
    assert_eq!(
        h.events.syncing(),
        vec![true, false],
        "feed passes are quiet"
    );
    assert_eq!(h.events.snapshots(), snapshots + 2, "but still publish");

    wait(30).await; // t = 91: the GitHub timer is due
    assert_eq!(h.gh.fetches(), 2);
    assert_eq!(h.events.syncing(), vec![true, false, true, false]);

    // A refresh is announced even right after a GitHub pass.
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(
        h.events.syncing(),
        vec![true, false, true, false, true, false]
    );
}

#[tokio::test(start_paused = true)]
async fn failing_repo_keeps_last_good_data_and_reports_error() {
    let h = Harness::new(&["acme/app", "acme/lib"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    h.gh.set_prs("acme/lib", &[(2, "alice")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/lib"), vec![2]);

    h.gh.set_repo_fail("acme/lib", Some(Fail::Network));
    h.gh.set_prs("acme/app", &[(1, "alice"), (3, "alice")]);
    h.engine.request_sync();
    wait(1).await;

    let lib = h.project("acme/lib");
    assert_eq!(h.pr_numbers("acme/lib"), vec![2]);
    assert_eq!(
        lib.error.as_deref(),
        Some("network error: connection reset")
    );
    let app = h.project("acme/app");
    assert_eq!(app.error, None);
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 3]);
    // One repo working means GitHub itself is fine.
    assert!(h.snapshot().github.ok);

    h.gh.set_repo_fail("acme/lib", None);
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(h.project("acme/lib").error, None);
}

#[tokio::test(start_paused = true)]
async fn every_repo_failing_marks_github_down() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_repo_fail("acme/app", Some(Fail::Network));
    let _run = h.start();
    wait(1).await;
    let snap = h.snapshot();
    assert!(!snap.github.ok);
    assert_eq!(
        snap.github.error.as_deref(),
        Some("acme/app: network error: connection reset")
    );
}

#[tokio::test(start_paused = true)]
async fn unauthorized_drops_client_and_viewer() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    let _run = h.start();
    wait(1).await;
    assert!(h.snapshot().viewer.is_some());

    h.gh.set_repo_fail("acme/app", Some(Fail::Unauthorized));
    h.engine.request_sync();
    wait(1).await;
    let snap = h.snapshot();
    assert_eq!(snap.viewer, None);
    assert!(!snap.github.ok);
    assert_eq!(
        snap.github.error.as_deref(),
        Some("GitHub rejected the token: bad credentials")
    );

    // The next pass looks for a token again.
    h.gh.set_repo_fail("acme/app", None);
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(h.gh.connects.load(Ordering::SeqCst), 2);
    assert!(h.snapshot().viewer.is_some());
    assert!(h.snapshot().github.ok);
}

#[tokio::test(start_paused = true)]
async fn not_signed_in_shows_no_viewer_and_retries() {
    let h = Harness::new(&["acme/app"], &[]);
    *h.gh.connect_fail.lock().unwrap() = Some(Fail::NotSignedIn);
    let _run = h.start();
    wait(1).await;
    let snap = h.snapshot();
    assert_eq!(snap.viewer, None);
    assert!(snap.sign_in_required);
    assert!(!snap.github.ok);
    assert_eq!(
        snap.github.error.as_deref(),
        Some("not signed in to GitHub: run gh auth login")
    );
    assert_eq!(h.gh.fetches(), 0);

    *h.gh.connect_fail.lock().unwrap() = None;
    wait(90).await;
    assert!(h.snapshot().viewer.is_some());
    assert!(!h.snapshot().sign_in_required);
    assert_eq!(h.gh.fetches(), 1);
}

#[tokio::test(start_paused = true)]
async fn offline_at_startup_is_not_reported_as_signed_out() {
    let h = Harness::new(&["acme/app"], &[]);
    *h.gh.connect_fail.lock().unwrap() = Some(Fail::Network);
    let _run = h.start();
    wait(1).await;
    let snap = h.snapshot();
    assert_eq!(snap.viewer, None);
    assert!(!snap.sign_in_required);
    assert!(!snap.github.ok);
}

#[tokio::test(start_paused = true)]
async fn nothing_is_published_before_the_first_github_attempt() {
    // The feed answers at once; GitHub takes five seconds. A snapshot built
    // from the feed alone would have no viewer and look signed out.
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    *h.gh.fetch_delay.lock().unwrap() = Duration::from_secs(5);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.feed.fetches.load(Ordering::SeqCst), 1);
    assert_eq!(h.events.snapshots(), 0);
    assert_eq!(h.engine.snapshot(), None);

    wait(5).await;
    let snap = h.snapshot();
    assert!(snap.viewer.is_some());
    assert!(
        snap.feed.ok,
        "the feed fetched earlier is in the first snapshot"
    );
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_config_is_never_overwritten() {
    let disk = Arc::new(Disk::default());
    *disk.load_error.lock().unwrap() =
        Some("/cfg/config.json is not valid JSON. Fix or delete the file.".into());
    let h = Harness::with_disk(disk);
    h.gh.canonical
        .lock()
        .unwrap()
        .insert("acme/app".into(), "Acme/App".into());
    assert!(h.engine.config().projects.is_empty());

    let err = h.engine.add_project("acme/app").await.unwrap_err();
    assert!(matches!(err, AppError::Config(_)));
    assert!(
        err.to_string()
            .contains("/cfg/config.json is not valid JSON")
    );
    assert!(err.to_string().contains("Nothing was saved"));
    assert!(h.engine.follow_pr("acme/app#1").is_err());
    assert!(h.engine.remove_project("acme/app").is_err());
    assert_eq!(h.disk.saves.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_pr_unfollowed_during_a_pass_does_not_come_back() {
    let h = Harness::new(&["acme/app"], &["acme/app#2"]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "bob")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2]);

    // A pass that started while #2 was still followed...
    *h.gh.fetch_delay.lock().unwrap() = Duration::from_secs(5);
    h.engine.request_sync();
    wait(1).await;
    h.engine.unfollow_pr("acme/app#2").unwrap();
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);

    // ...lands with #2 in its result. It must not put #2 back.
    wait(5).await;
    assert_eq!(h.gh.fetches(), 3, "the follow-up cycle is still in flight");
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn requests_during_a_cycle_coalesce_into_one() {
    let h = Harness::new(&["acme/app"], &[]);
    *h.gh.fetch_delay.lock().unwrap() = Duration::from_secs(5);
    let _run = h.start();
    wait(1).await; // first cycle is mid-fetch
    assert_eq!(h.gh.fetches(), 1);
    for _ in 0..5 {
        h.engine.request_sync();
    }
    wait(1).await;
    h.engine.request_sync();
    wait(30).await; // both cycles done, before any timer is due for GitHub

    assert_eq!(h.gh.fetches(), 2);
    assert_eq!(h.events.syncing(), vec![true, false, true, false]);
}

#[tokio::test(start_paused = true)]
async fn rate_limit_pauses_github_until_reset() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_repo_fail("acme/app", Some(Fail::RateLimited));
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.gh.fetches(), 1);
    let snap = h.snapshot();
    assert!(!snap.github.ok);
    assert!(
        snap.github
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("GitHub rate limit reached"))
    );

    h.gh.set_repo_fail("acme/app", None);
    h.engine.request_sync();
    wait(5 * 60).await; // several timer ticks and a refresh, all inside the window
    assert_eq!(h.gh.fetches(), 1);
    // The feed keeps going meanwhile.
    assert!(h.feed.fetches.load(Ordering::SeqCst) > 5);

    wait(6 * 60).await; // past the 10-minute reset
    assert_eq!(h.gh.fetches(), 2);
    assert!(h.snapshot().github.ok);
}

#[tokio::test(start_paused = true)]
async fn feed_failure_keeps_last_good_feed() {
    let h = Harness::new(&[], &[]);
    let _run = h.start();
    wait(1).await;
    let first = h.snapshot().feed;
    assert!(first.ok);
    assert!(first.degraded, "merge got a feed");

    *h.feed.fail.lock().unwrap() = true;
    wait(30).await;
    let feed = h.snapshot().feed;
    assert!(!feed.ok);
    assert_eq!(feed.error.as_deref(), Some("network error: feed down"));
    assert!(feed.degraded, "merge still gets the last good feed");
    assert_eq!(feed.last_success_at, first.last_success_at);
}

#[tokio::test(start_paused = true)]
async fn remove_project_updates_snapshot_without_network() {
    let h = Harness::new(&["acme/app", "acme/lib"], &["acme/lib#7"]);
    h.gh.set_prs("acme/lib", &[(7, "carol")]);
    let _run = h.start();
    wait(1).await;
    let calls = h.gh.calls();
    let emitted = h.events.snapshots();

    let config = h.engine.remove_project("acme/lib").unwrap();
    assert_eq!(config.projects, vec!["acme/app".to_string()]);
    assert!(config.followed.is_empty());
    assert_eq!(*h.disk.config.lock().unwrap(), config);

    assert_eq!(h.gh.calls(), calls);
    assert_eq!(h.events.snapshots(), emitted + 1);
    let repos: Vec<String> = h.snapshot().projects.into_iter().map(|p| p.repo).collect();
    assert_eq!(repos, vec!["acme/app".to_string()]);

    // Re-adding later does not resurrect the dropped cache.
    h.gh.canonical
        .lock()
        .unwrap()
        .insert("acme/lib".into(), "acme/lib".into());
    *h.gh.fetch_delay.lock().unwrap() = Duration::from_secs(5);
    h.engine.add_project("acme/lib").await.unwrap();
    assert_eq!(h.pr_numbers("acme/lib"), Vec::<u64>::new());
}

#[tokio::test(start_paused = true)]
async fn unfollow_updates_snapshot_without_network_then_syncs() {
    let h = Harness::new(&["acme/app"], &["acme/app#2", "acme/app#1"]);
    h.gh.set_prs("acme/app", &[(1, "ALICE"), (2, "bob")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2]);
    let calls = h.gh.calls();

    let config = h.engine.unfollow_pr("acme/app#2").unwrap();
    assert_eq!(config.followed, vec!["acme/app#1".to_string()]);
    assert_eq!(h.gh.calls(), calls, "no network before returning");
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);

    // Unfollowing your own PR keeps it: it is still yours.
    h.engine.unfollow_pr("acme/app#1").unwrap();
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);

    wait(1).await;
    assert_eq!(h.gh.fetches(), 2, "then a cycle runs");
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn add_project_validates_and_uses_github_casing() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.canonical
        .lock()
        .unwrap()
        .insert("acme/lib".into(), "Acme/Lib".into());
    let _run = h.start();
    wait(1).await;

    let err = h.engine.add_project("ACME/app").await.unwrap_err();
    assert_eq!(err.to_string(), "ACME/app is already in your projects.");

    let err = h.engine.add_project("acme/nope").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
    assert_eq!(h.engine.config().projects, vec!["acme/app".to_string()]);

    let err = h.engine.add_project("not a repo").await.unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));

    let fetches = h.gh.fetches();
    let config = h
        .engine
        .add_project("https://github.com/acme/lib")
        .await
        .unwrap();
    assert_eq!(
        config.projects,
        vec!["acme/app".to_string(), "Acme/Lib".to_string()]
    );
    assert_eq!(*h.disk.config.lock().unwrap(), config);
    wait(1).await;
    assert_eq!(h.gh.fetches(), fetches + 2, "a cycle fetched both projects");
    assert!(h.snapshot().projects.iter().any(|p| p.repo == "Acme/Lib"));
}

#[tokio::test(start_paused = true)]
async fn add_project_when_signed_out_fails() {
    let h = Harness::new(&[], &[]);
    *h.gh.connect_fail.lock().unwrap() = Some(Fail::NotSignedIn);
    let err = h.engine.add_project("acme/app").await.unwrap_err();
    assert!(matches!(err, AppError::NotSignedIn(_)));
    assert!(h.engine.config().projects.is_empty());
    assert_eq!(h.disk.saves.load(Ordering::SeqCst), 0);
    assert_eq!(h.snapshot().viewer, None);
}

#[tokio::test(start_paused = true)]
async fn follow_requires_tracked_project_and_uses_its_casing() {
    let h = Harness::new(&["Acme/App"], &[]);
    let err = h.engine.follow_pr("other/repo#3").unwrap_err();
    assert_eq!(
        err.to_string(),
        "other/repo is not one of your projects. Add it first."
    );
    let config = h.engine.follow_pr("acme/app#3").unwrap();
    assert_eq!(config.followed, vec!["Acme/App#3".to_string()]);
    assert_eq!(*h.disk.config.lock().unwrap(), config);
}

#[tokio::test(start_paused = true)]
async fn list_open_prs_marks_mine_and_followed() {
    let h = Harness::new(&["acme/app"], &["ACME/app#2"]);
    *h.gh.open_prs.lock().unwrap() = vec![
        summary("acme/app", 1, "alice"),
        summary("acme/app", 2, "bob"),
        summary("acme/app", 3, "carol"),
    ];
    let prs = h.engine.list_open_prs("acme/app").await.unwrap();
    let flags: Vec<(u64, bool, bool)> = prs
        .iter()
        .map(|p| (p.number, p.is_mine, p.is_followed))
        .collect();
    assert_eq!(
        flags,
        vec![(1, true, false), (2, false, true), (3, false, false)]
    );
}

#[tokio::test(start_paused = true)]
async fn focus_syncs_only_when_github_is_stale() {
    let h = Harness::new(&["acme/app"], &[]);
    let _run = h.start();
    wait(1).await;
    h.engine.on_focus();
    wait(1).await;
    assert_eq!(h.gh.fetches(), 1, "fresh: no sync");

    wait(20).await;
    h.engine.on_focus();
    wait(1).await;
    assert_eq!(h.gh.fetches(), 2, "stale: synced");
}

#[test]
fn rate_limit_wait_reads_rfc3339_and_unix_seconds() {
    let now = Utc.with_ymd_and_hms(2026, 11, 22, 12, 0, 0).unwrap();
    assert_eq!(
        rate_limit_wait("2026-11-22T12:05:00Z", now),
        Duration::from_secs(300)
    );
    let unix = (now.timestamp() + 90).to_string();
    assert_eq!(rate_limit_wait(&unix, now), Duration::from_secs(90));
    assert_eq!(rate_limit_wait("2026-11-22T11:00:00Z", now), Duration::ZERO);
    assert_eq!(rate_limit_wait("soon", now), Duration::from_secs(60));
    assert_eq!(
        rate_limit_wait("2026-11-23T12:00:00Z", now),
        Duration::from_secs(3600)
    );
}

// ---- Merging ----

/// A fresh check GitHub would let through.
fn ready_check(head: &str) -> GhMergeCheck {
    GhMergeCheck {
        head_sha: head.into(),
        is_draft: false,
        is_open: true,
        commit_title: "PR 1 (#1)".into(),
        facts: GhMergeFacts {
            node_id: "PR_fresh_node".into(),
            base_branch: "main".into(),
            review_threads: 2,
            unresolved_threads: 0,
            mergeable: GhMergeable::Mergeable,
            merge_state: "CLEAN".into(),
            squash_allowed: true,
            viewer_can_merge: true,
            merge_queue: false,
        },
    }
}

fn invalid_input(r: AppResult<()>) -> String {
    match r {
        Err(AppError::InvalidInput(m)) => m,
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn merge_check_on_an_untracked_repo_is_refused() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_merge_check("other/repo", 1, ready_check("abc"));
    let e = h.engine.merge_check("other/repo#1").await.unwrap_err();
    assert!(
        matches!(e, AppError::InvalidInput(ref m) if m.contains("other/repo")),
        "{e:?}"
    );
    let e = h.engine.squash_merge("other/repo#1", "abc").await;
    invalid_input(e);
    assert_eq!(h.gh.merge_check_calls.load(Ordering::SeqCst), 0);
    assert!(h.gh.merges().is_empty());
    // Garbage ids are refused too.
    assert!(h.engine.merge_check("not a pr").await.is_err());
}

#[tokio::test(start_paused = true)]
async fn merge_check_is_fresh_and_uses_the_tracked_name() {
    let h = Harness::new(&["Acme/App"], &[]);
    let mut check = ready_check("abc");
    check.facts.unresolved_threads = 2;
    h.gh.set_merge_check("acme/app", 1, check);
    let got = h.engine.merge_check("acme/app#1").await.unwrap();
    assert_eq!(got.id, "Acme/App#1");
    assert_eq!(got.head_sha, "abc");
    assert_eq!(got.base_branch, "main");
    assert_eq!(got.commit_title, "PR 1 (#1)");
    assert_eq!(got.unresolved_threads, 2);
    assert!(!got.merge.ready);
    assert_eq!(got.merge.reason.as_deref(), Some("2 unresolved comments"));

    h.gh.set_merge_check("acme/app", 1, ready_check("abc"));
    assert!(
        h.engine
            .merge_check("Acme/App#1")
            .await
            .unwrap()
            .merge
            .ready
    );

    let mut closed = ready_check("abc");
    closed.is_open = false;
    h.gh.set_merge_check("acme/app", 1, closed);
    let got = h.engine.merge_check("acme/app#1").await.unwrap();
    assert_eq!(got.merge.reason.as_deref(), Some(CLOSED_REASON));
    assert_eq!(h.gh.merge_check_calls.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn squash_merge_refuses_a_moved_head() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_merge_check("acme/app", 1, ready_check("new-head"));
    let m = invalid_input(h.engine.squash_merge("acme/app#1", "old-head").await);
    assert_eq!(m, HEAD_MOVED);
    let m = invalid_input(h.engine.squash_merge("acme/app#1", "  ").await);
    assert_eq!(m, HEAD_MOVED);
    assert!(h.gh.merges().is_empty());
}

#[tokio::test(start_paused = true)]
async fn squash_merge_refuses_unresolved_threads() {
    let h = Harness::new(&["acme/app"], &[]);
    let mut check = ready_check("abc");
    check.facts.unresolved_threads = 1;
    h.gh.set_merge_check("acme/app", 1, check);
    let m = invalid_input(h.engine.squash_merge("acme/app#1", "abc").await);
    assert_eq!(m, "1 unresolved comment");
    assert!(h.gh.merges().is_empty());
}

#[tokio::test(start_paused = true)]
async fn squash_merge_refuses_when_github_would_not_merge() {
    let h = Harness::new(&["acme/app"], &[]);
    let mut check = ready_check("abc");
    check.facts.merge_state = "BLOCKED".into();
    h.gh.set_merge_check("acme/app", 1, check);
    let m = invalid_input(h.engine.squash_merge("acme/app#1", "abc").await);
    assert!(
        m.starts_with("GitHub's branch rules block this merge"),
        "{m}"
    );
    assert!(h.gh.merges().is_empty());
}

#[tokio::test(start_paused = true)]
async fn squash_merge_refuses_a_closed_pr() {
    let h = Harness::new(&["acme/app"], &[]);
    let mut check = ready_check("abc");
    check.is_open = false;
    h.gh.set_merge_check("acme/app", 1, check);
    let m = invalid_input(h.engine.squash_merge("acme/app#1", "abc").await);
    assert_eq!(m, CLOSED_REASON);
    assert!(h.gh.merges().is_empty());
}

#[tokio::test(start_paused = true)]
async fn squash_merge_happy_path() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "alice")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2]);
    let fetches = h.gh.fetches();
    let snapshots = h.events.snapshots();

    // The UI passes the head it showed; the engine merges with what GitHub
    // says now (the same head, and the fresh node id).
    h.gh.set_merge_check("acme/app", 1, ready_check("ABC"));
    h.engine.squash_merge("acme/app#1", "abc").await.unwrap();
    assert_eq!(
        h.gh.merges(),
        vec![("PR_fresh_node".to_string(), "ABC".to_string())]
    );
    // Gone from the snapshot at once, before any sync.
    assert_eq!(h.events.snapshots(), snapshots + 1);
    assert_eq!(h.pr_numbers("acme/app"), vec![2]);
    assert_eq!(h.gh.fetches(), fetches);

    // And a sync follows.
    h.gh.set_prs("acme/app", &[(2, "alice")]);
    wait(1).await;
    assert_eq!(h.gh.fetches(), fetches + 1);
    assert_eq!(h.pr_numbers("acme/app"), vec![2]);
    assert_eq!(h.gh.merges().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_refused_merge_keeps_the_pr_and_says_why() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    let _run = h.start();
    wait(1).await;
    h.gh.set_merge_check("acme/app", 1, ready_check("abc"));
    *h.gh.merge_fail.lock().unwrap() = Some("Required status check missing".into());
    let e = h
        .engine
        .squash_merge("acme/app#1", "abc")
        .await
        .unwrap_err();
    assert!(
        e.to_string().contains("Required status check missing"),
        "{e}"
    );
    assert_eq!(h.gh.merges().len(), 1);
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
}

// ---- T3 Code ----

fn t3_link(repo: &str, number: u64, id: &str, state: T3ThreadState) -> T3Link {
    T3Link {
        repo: repo.into(),
        number,
        thread_id: id.into(),
        title: format!("Thread {id}"),
        state,
        watching: false,
        last_active_at: None,
    }
}

#[tokio::test(start_paused = true)]
async fn t3_links_reach_the_snapshot() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "alice")]);
    h.t3.set(T3Mode::Links(vec![
        t3_link("ACME/app", 1, "idle", T3ThreadState::Idle),
        t3_link("acme/app", 1, "waits", T3ThreadState::Waiting),
        t3_link("acme/app", 1, "busy", T3ThreadState::Working),
        t3_link("acme/other", 2, "elsewhere", T3ThreadState::Working),
    ]));
    let _run = h.start();
    wait(1).await;
    let snap = h.snapshot();
    assert!(snap.t3_available);
    let prs = &h.project("acme/app").prs;
    let pr1 = prs.iter().find(|p| p.number == 1).unwrap();
    let threads: Vec<_> = pr1
        .t3_threads
        .iter()
        .map(|t| (t.thread_id.as_str(), t.state))
        .collect();
    assert_eq!(
        threads,
        vec![
            ("busy", T3ThreadState::Working),
            ("waits", T3ThreadState::Waiting),
            ("idle", T3ThreadState::Idle)
        ]
    );
    assert!(
        prs.iter()
            .find(|p| p.number == 2)
            .unwrap()
            .t3_threads
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn t3_is_read_on_feed_only_passes_too() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.t3.reads(), 1);
    assert!(!h.snapshot().t3_available);
    let fetches = h.gh.fetches();

    // T3 Code starts working on the PR between GitHub passes.
    h.t3.set(T3Mode::Links(vec![t3_link(
        "acme/app",
        1,
        "t",
        T3ThreadState::Working,
    )]));
    wait(30).await; // t = 31: a feed-only pass
    assert_eq!(h.gh.fetches(), fetches);
    assert_eq!(h.t3.reads(), 2);
    let snap = h.snapshot();
    assert!(snap.t3_available);
    assert_eq!(snap.projects[0].prs[0].t3_threads.len(), 1);
    assert_eq!(h.events.last_snapshot(), Some(snap));

    // Unchanged T3 data adds no snapshot of its own: only the feed pass's.
    let snapshots = h.events.snapshots();
    wait(30).await; // t = 61
    assert_eq!(h.t3.reads(), 3);
    assert_eq!(h.events.snapshots(), snapshots + 1);
}

async fn a_broken_reader_leaves_t3_unavailable(mode: T3Mode) {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    h.t3.set(T3Mode::Links(vec![t3_link(
        "acme/app",
        1,
        "t",
        T3ThreadState::Idle,
    )]));
    let _run = h.start();
    wait(1).await;
    assert!(h.snapshot().t3_available);

    h.t3.set(mode);
    wait(30).await; // a feed-only pass
    let snap = h.snapshot();
    assert!(!snap.t3_available, "{snap:?}");
    assert!(snap.projects[0].prs[0].t3_threads.is_empty());
    assert!(snap.github.ok);
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);

    // The cycles go on as before.
    wait(60).await; // t = 91: GitHub again
    assert_eq!(h.gh.fetches(), 2);
    assert!(h.t3.reads() >= 4);
    assert!(h.snapshot().github.ok);
    assert!(!h.snapshot().t3_available);
}

#[tokio::test(start_paused = true)]
async fn a_failing_t3_reader_leaves_t3_unavailable() {
    a_broken_reader_leaves_t3_unavailable(T3Mode::Error).await;
}

#[tokio::test(start_paused = true)]
async fn a_panicking_t3_reader_leaves_t3_unavailable() {
    a_broken_reader_leaves_t3_unavailable(T3Mode::Panic).await;
}

#[tokio::test(start_paused = true)]
async fn a_t3_reader_that_panics_from_the_start_still_lets_github_publish() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    h.t3.set(T3Mode::Panic);
    let _run = h.start();
    wait(1).await;
    let snap = h.snapshot();
    assert!(!snap.t3_available);
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
    assert_eq!(h.events.syncing(), vec![true, false]);
}

#[tokio::test(start_paused = true)]
async fn t3_is_asked_for_tracked_followed_and_renamed_repos() {
    let h = Harness::new(&["acme/app", "acme/old"], &["Other/Lib#3", "ACME/APP#9"]);
    h.gh.renamed
        .lock()
        .unwrap()
        .insert("acme/old".into(), "acme/new".into());
    let _run = h.start();
    wait(1).await;
    // The first read starts before GitHub has answered.
    assert_eq!(
        h.t3.asked.lock().unwrap()[0],
        vec!["acme/app", "acme/old", "Other/Lib"]
    );
    wait(30).await;
    assert_eq!(
        h.t3.asked.lock().unwrap().last().cloned().unwrap(),
        vec!["acme/app", "acme/old", "Other/Lib", "acme/new"]
    );
}

// ---- Self-review ----

const HEAD: &str = "ce87c4ba91c01d646c7fa966b385ec7e215d90af";
const NEW_HEAD: &str = "9cc7778487200a5f379f0cb47cfff854e1a520dd";
const ASKS: &str = "waiting-self-review: Bots are done - your move: post /self-reviewed.";

/// A fresh self-review check that allows Alice to post on her own PR.
fn self_review_ready(head: &str) -> GhSelfReviewCheck {
    GhSelfReviewCheck {
        node_id: "PR_fresh_node".into(),
        url: "https://github.com/acme/app/pull/1".into(),
        head_sha: head.into(),
        is_draft: false,
        is_open: true,
        author: "alice".into(),
        assignees: vec![],
        viewer: "Alice".into(),
        hygiene: Some(GhHygiene {
            state: "PENDING".into(),
            description: Some(ASKS.into()),
        }),
        comments: vec![],
    }
}

fn self_review_of(h: &Harness, number: u64) -> SelfReview {
    h.project("acme/app")
        .prs
        .into_iter()
        .find(|p| p.number == number)
        .expect("the PR")
        .self_review
}

#[tokio::test(start_paused = true)]
async fn self_review_check_is_fresh_and_only_for_tracked_repos() {
    let h = Harness::new(&["Acme/App"], &[]);
    let mut check = self_review_ready(HEAD);
    check.comments = vec![GhComment {
        author: "alice".into(),
        author_is_bot: false,
        body: format!("/self-reviewed {HEAD}"),
        created_at: Utc.with_ymd_and_hms(2026, 11, 22, 9, 0, 0).unwrap(),
        edited: false,
    }];
    h.gh.set_self_review_check("acme/app", 1, check);
    let got = h.engine.self_review_check("acme/app#1").await.unwrap();
    assert_eq!(got.id, "Acme/App#1");
    assert_eq!(got.head_sha, HEAD);
    assert_eq!(got.comment, format!("/self-reviewed {HEAD}"));
    assert!(got.can_post);
    assert_eq!(
        got.already_posted_at,
        Some(Utc.with_ymd_and_hms(2026, 11, 22, 9, 0, 0).unwrap())
    );
    assert_eq!(
        got.gate.as_deref(),
        Some("Bots are done - your move: post /self-reviewed.")
    );
    assert_eq!(got.diff_url, "https://github.com/acme/app/pull/1/files");
    assert_eq!(h.gh.self_review_check_calls.load(Ordering::SeqCst), 1);

    h.gh.set_self_review_check("other/repo", 1, self_review_ready(HEAD));
    let e = h
        .engine
        .self_review_check("other/repo#1")
        .await
        .unwrap_err();
    assert!(
        matches!(e, AppError::InvalidInput(ref m) if m.contains("other/repo")),
        "{e:?}"
    );
    invalid_input(h.engine.post_self_review("other/repo#1", HEAD).await);
    assert!(h.engine.self_review_check("not a pr").await.is_err());
    assert_eq!(h.gh.self_review_check_calls.load(Ordering::SeqCst), 1);
    assert!(h.gh.comments().is_empty());
}

#[tokio::test(start_paused = true)]
async fn check_then_post_shows_posted_until_the_head_moves() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "alice")]);
    h.gh.set_gate("acme/app", 1, HEAD, Some(ASKS));
    let _run = h.start();
    wait(1).await;
    assert_eq!(self_review_of(&h, 1).state, SelfReviewState::Wanted);
    // No gate on #2: nothing to offer.
    assert_eq!(self_review_of(&h, 2), SelfReview::default());

    h.gh.set_self_review_check("acme/app", 1, self_review_ready(HEAD));
    let check = h.engine.self_review_check("acme/app#1").await.unwrap();
    assert!(check.can_post);
    let fetches = h.gh.fetches();
    let snapshots = h.events.snapshots();

    // The UI passes back the head it showed, in any case.
    h.engine
        .post_self_review("acme/app#1", &format!(" {} ", HEAD.to_ascii_uppercase()))
        .await
        .unwrap();
    assert_eq!(
        h.gh.comments(),
        vec![(
            "PR_fresh_node".to_string(),
            format!("/self-reviewed {HEAD}")
        )]
    );
    // Posted at once, before any sync.
    assert_eq!(h.events.snapshots(), snapshots + 1);
    assert_eq!(h.gh.fetches(), fetches);
    let shown = self_review_of(&h, 1);
    assert_eq!(shown.state, SelfReviewState::Posted);
    assert!(shown.posted_at.is_some());
    assert_eq!(self_review_of(&h, 2), SelfReview::default());

    // A sync follows; the gate has not re-evaluated yet, so still posted.
    wait(1).await;
    assert_eq!(h.gh.fetches(), fetches + 1);
    assert_eq!(self_review_of(&h, 1), shown);

    // A push: the self-review no longer covers the head.
    h.gh.set_gate("acme/app", 1, NEW_HEAD, Some(ASKS));
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(
        self_review_of(&h, 1),
        SelfReview {
            state: SelfReviewState::Wanted,
            posted_at: None
        }
    );
    // And it is forgotten: going back to the old head does not revive it.
    h.gh.set_gate("acme/app", 1, HEAD, Some(ASKS));
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(self_review_of(&h, 1).state, SelfReviewState::Wanted);
    assert_eq!(h.gh.comments().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn posted_ends_when_the_gate_moves_on_and_a_renewed_ask_shows_as_wanted() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    h.gh.set_gate("acme/app", 1, HEAD, Some(ASKS));
    let _run = h.start();
    wait(1).await;
    h.gh.set_self_review_check("acme/app", 1, self_review_ready(HEAD));
    h.engine.post_self_review("acme/app#1", HEAD).await.unwrap();
    assert_eq!(self_review_of(&h, 1).state, SelfReviewState::Posted);

    h.gh.set_gate(
        "acme/app",
        1,
        HEAD,
        Some("ready-for-human: Waiting for bob."),
    );
    wait(1).await;
    assert_eq!(self_review_of(&h, 1), SelfReview::default());

    // An objection answered: the gate asks again on the same head.
    h.gh.set_gate("acme/app", 1, HEAD, Some(ASKS));
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(self_review_of(&h, 1).state, SelfReviewState::Wanted);
}

#[tokio::test(start_paused = true)]
async fn post_self_review_refuses_a_moved_head() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_self_review_check("acme/app", 1, self_review_ready(NEW_HEAD));
    let m = invalid_input(h.engine.post_self_review("acme/app#1", HEAD).await);
    assert_eq!(m, SELF_REVIEW_HEAD_MOVED);
    let m = invalid_input(h.engine.post_self_review("acme/app#1", "  ").await);
    assert_eq!(m, SELF_REVIEW_HEAD_MOVED);
    assert!(h.gh.comments().is_empty());
}

#[tokio::test(start_paused = true)]
async fn post_self_review_refuses_whoever_does_not_hold_it() {
    let h = Harness::new(&["acme/app"], &[]);
    let mut check = self_review_ready(HEAD);
    check.author = "bob".into();
    check.assignees = vec!["carol".into()];
    h.gh.set_self_review_check("acme/app", 1, check.clone());
    let m = invalid_input(h.engine.post_self_review("acme/app#1", HEAD).await);
    assert_eq!(m, SELF_REVIEW_NOT_HOLDING);
    assert!(h.gh.comments().is_empty());

    // Assigned to Alice: allowed.
    check.assignees.push("alice".into());
    h.gh.set_self_review_check("acme/app", 1, check);
    h.engine.post_self_review("acme/app#1", HEAD).await.unwrap();
    assert_eq!(h.gh.comments().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn post_self_review_refuses_drafts_and_closed_prs() {
    let h = Harness::new(&["acme/app"], &[]);
    let mut draft = self_review_ready(HEAD);
    draft.is_draft = true;
    h.gh.set_self_review_check("acme/app", 1, draft);
    let m = invalid_input(h.engine.post_self_review("acme/app#1", HEAD).await);
    assert_eq!(m, SELF_REVIEW_DRAFT);

    let mut closed = self_review_ready(HEAD);
    closed.is_open = false;
    h.gh.set_self_review_check("acme/app", 1, closed.clone());
    let m = invalid_input(h.engine.post_self_review("acme/app#1", HEAD).await);
    assert_eq!(m, SELF_REVIEW_CLOSED);
    // Closed is said even when the head has moved too.
    closed.head_sha = NEW_HEAD.into();
    h.gh.set_self_review_check("acme/app", 1, closed);
    let m = invalid_input(h.engine.post_self_review("acme/app#1", HEAD).await);
    assert_eq!(m, SELF_REVIEW_CLOSED);
    assert!(h.gh.comments().is_empty());
}

#[tokio::test(start_paused = true)]
async fn post_self_review_refuses_a_head_that_is_not_a_full_sha() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_self_review_check("acme/app", 1, self_review_ready("abc"));
    let e = h.engine.post_self_review("acme/app#1", "abc").await;
    invalid_input(e);
    assert!(h.gh.comments().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_failed_post_records_nothing() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice")]);
    h.gh.set_gate("acme/app", 1, HEAD, Some(ASKS));
    let _run = h.start();
    wait(1).await;
    h.gh.set_self_review_check("acme/app", 1, self_review_ready(HEAD));
    *h.gh.comment_fail.lock().unwrap() = Some("Resource not accessible".into());
    let e = h
        .engine
        .post_self_review("acme/app#1", HEAD)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("Resource not accessible"), "{e}");
    assert_eq!(h.gh.comments().len(), 1);
    assert_eq!(self_review_of(&h, 1).state, SelfReviewState::Wanted);
}

// ---- Tracked contributors ----

fn saves(h: &Harness) -> usize {
    h.disk.saves.load(Ordering::SeqCst)
}

fn refusal(r: AppResult<Config>) -> String {
    match r {
        Err(AppError::InvalidInput(msg)) => msg,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn tracking_a_contributor_stores_github_casing_and_brings_their_prs() {
    let h = Harness::new(&["acme/app", "acme/lib"], &[]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "bob"), (3, "carol")]);
    h.gh.set_prs("acme/lib", &[(7, "BOB")]);
    h.gh.add_account("Bob", GhAccountKind::User);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
    let fetches = h.gh.fetches();

    let config = h.engine.track_contributor(" @bob ").await.unwrap();
    assert_eq!(config.contributors, vec!["Bob".to_string()]);
    assert_eq!(*h.disk.config.lock().unwrap(), config);
    assert_eq!(h.engine.config(), config);

    // A sync is asked for at once; their PRs arrive with it.
    wait(1).await;
    assert_eq!(h.gh.fetches(), fetches + 2);
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2]);
    assert_eq!(h.pr_numbers("acme/lib"), vec![7]);
    let asked = h.gh.asked_contributors.lock().unwrap().clone();
    assert_eq!(asked.last(), Some(&vec!["Bob".to_string()]));
    let pr2 = h
        .project("acme/app")
        .prs
        .into_iter()
        .find(|p| p.number == 2)
        .unwrap();
    assert!(!pr2.is_mine);
    assert_eq!(pr2.self_review.state, SelfReviewState::Unavailable);
}

#[tokio::test(start_paused = true)]
async fn tracking_someone_already_tracked_changes_nothing() {
    let h = Harness::tracking(&["acme/app"], &[], &["Bob"]);
    let _run = h.start();
    wait(1).await;
    let (calls, saved) = (h.gh.calls(), saves(&h));

    let config = h.engine.track_contributor("BOB").await.unwrap();
    assert_eq!(config.contributors, vec!["Bob".to_string()]);
    assert_eq!(h.gh.calls(), calls, "no network");
    assert_eq!(saves(&h), saved, "nothing saved");
    wait(1).await;
    assert_eq!(h.gh.fetches(), 1, "no sync asked for");
}

#[tokio::test(start_paused = true)]
async fn tracking_yourself_is_refused() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.add_account("Alice", GhAccountKind::User);
    // Before the first sync the viewer is not known yet: found on the way.
    assert_eq!(
        refusal(h.engine.track_contributor("alice").await),
        TRACK_SELF
    );
    let _run = h.start();
    wait(1).await;
    let calls = h.gh.calls();
    assert_eq!(
        refusal(h.engine.track_contributor("@ALICE").await),
        TRACK_SELF
    );
    assert_eq!(h.gh.calls(), calls, "known without asking GitHub");
    assert_eq!(h.gh.account_calls(), 0);
    assert_eq!(saves(&h), 0);
    assert!(h.engine.config().contributors.is_empty());
}

#[tokio::test(start_paused = true)]
async fn tracking_needs_a_real_person() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.add_account("DashPay", GhAccountKind::Organization);
    h.gh.add_account("Weird", GhAccountKind::Other);
    let _run = h.start();
    wait(1).await;

    assert_eq!(
        refusal(h.engine.track_contributor("zed").await),
        "There is no GitHub user named zed."
    );
    assert_eq!(
        refusal(h.engine.track_contributor("dashpay").await),
        "DashPay is an organization, not a person. Only people's pull requests can be tracked."
    );
    assert_eq!(
        refusal(h.engine.track_contributor("weird").await),
        "Weird is not a person's account. Only people's pull requests can be tracked."
    );
    let calls = h.gh.account_calls();
    let msg = refusal(h.engine.track_contributor("ThePastaClaw").await);
    assert!(msg.starts_with("thepastaclaw is the review bot"), "{msg}");
    assert_eq!(
        h.gh.account_calls(),
        calls,
        "the review bot is known by name"
    );
    assert!(matches!(
        h.engine.track_contributor("not a login").await,
        Err(AppError::InvalidInput(_))
    ));
    assert_eq!(saves(&h), 0);
}

#[tokio::test(start_paused = true)]
async fn tracking_stops_at_the_maximum_before_asking_github() {
    let h = Harness::tracking(&["acme/app"], &[], &["a", "b", "c"]);
    h.gh.add_account("d", GhAccountKind::User);
    let _run = h.start();
    wait(1).await;
    assert_eq!(
        refusal(h.engine.track_contributor("d").await),
        "You can track at most 3 contributors."
    );
    assert_eq!(h.gh.account_calls(), 0);
    assert_eq!(saves(&h), 0);
    // Someone already tracked still answers with the config.
    assert_eq!(
        h.engine.track_contributor("A").await.unwrap().contributors,
        vec!["a", "b", "c"]
    );
}

#[tokio::test(start_paused = true)]
async fn tracking_when_github_is_unreachable_fails_and_saves_nothing() {
    let h = Harness::new(&["acme/app"], &[]);
    *h.gh.connect_fail.lock().unwrap() = Some(Fail::Network);
    let err = h.engine.track_contributor("bob").await.unwrap_err();
    assert!(matches!(err, AppError::Network(_)), "{err:?}");
    assert_eq!(saves(&h), 0);
}

#[tokio::test(start_paused = true)]
async fn untracking_drops_their_prs_at_once_but_keeps_followed_ones() {
    let h = Harness::tracking(&["acme/app"], &["acme/app#3"], &["Bob", "Carol"]);
    h.gh.set_prs(
        "acme/app",
        &[(1, "alice"), (2, "bob"), (3, "carol"), (4, "carol")],
    );
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2, 3, 4]);
    let calls = h.gh.calls();

    let config = h.engine.untrack_contributor("CAROL").unwrap();
    assert_eq!(config.contributors, vec!["Bob".to_string()]);
    assert_eq!(*h.disk.config.lock().unwrap(), config);
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2, 3], "#3 is followed");

    h.engine.untrack_contributor(" bob ").unwrap();
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 3]);
    assert_eq!(h.gh.calls(), calls, "no network");

    // The next sync agrees.
    h.engine.request_sync();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 3]);
    assert_eq!(
        h.gh.asked_contributors.lock().unwrap().last(),
        Some(&Vec::<String>::new())
    );
}

#[tokio::test(start_paused = true)]
async fn untracking_someone_not_tracked_changes_nothing() {
    let h = Harness::tracking(&["acme/app"], &[], &["Bob"]);
    let _run = h.start();
    wait(1).await;
    let emitted = h.events.snapshots();
    let config = h.engine.untrack_contributor("nobody").unwrap();
    assert_eq!(config.contributors, vec!["Bob".to_string()]);
    assert_eq!(saves(&h), 0);
    assert_eq!(h.events.snapshots(), emitted);
}

#[tokio::test(start_paused = true)]
async fn a_contributor_untracked_during_a_pass_does_not_come_back() {
    let h = Harness::tracking(&["acme/app"], &[], &["bob"]);
    h.gh.set_prs("acme/app", &[(1, "alice"), (2, "bob")]);
    let _run = h.start();
    wait(1).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1, 2]);

    *h.gh.fetch_delay.lock().unwrap() = Duration::from_secs(5);
    h.engine.request_sync();
    wait(1).await;
    h.engine.untrack_contributor("bob").unwrap();
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
    // The pass that started while bob was tracked lands with #2.
    wait(5).await;
    assert_eq!(h.pr_numbers("acme/app"), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_config_is_not_overwritten_by_tracking() {
    let disk = Arc::new(Disk::default());
    *disk.load_error.lock().unwrap() =
        Some("/cfg/config.json is not valid JSON. Fix or delete the file.".into());
    let h = Harness::with_disk(disk);
    h.gh.add_account("Bob", GhAccountKind::User);

    let err = h.engine.track_contributor("bob").await.unwrap_err();
    assert!(matches!(err, AppError::Config(_)), "{err:?}");
    assert!(err.to_string().contains("Nothing was saved"));
    assert!(h.engine.untrack_contributor("bob").is_ok());
    assert_eq!(saves(&h), 0);
    assert!(h.engine.config().contributors.is_empty());
}

fn known(list: &[KnownContributor]) -> Vec<(&str, u32)> {
    list.iter()
        .map(|k| (k.login.as_str(), k.open_prs))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn known_contributors_are_counted_across_projects_and_filtered() {
    let h = Harness::tracking(&["acme/app", "acme/lib"], &[], &["Dave"]);
    h.gh.set_open_authors(
        "acme/app",
        &[
            ("bob", false),
            ("alice", false),
            ("Carol", false),
            ("dependabot", true),
            ("thepastaclaw", false),
            ("ghost", false),
            ("dave", false),
            ("erin", false),
        ],
    );
    h.gh.set_open_authors(
        "acme/lib",
        &[
            ("Bob", false),
            ("carol", false),
            ("ALICE", false),
            ("bob", false),
        ],
    );
    let _run = h.start();
    wait(1).await;

    let list = h.engine.known_contributors().await.unwrap();
    assert_eq!(known(&list), vec![("bob", 3), ("Carol", 2), ("erin", 1)]);
    assert_eq!(
        list[0].avatar_url.as_deref(),
        Some("https://avatars.test/bob")
    );
    assert_eq!(saves(&h), 0, "read-only");
}

#[tokio::test(start_paused = true)]
async fn ties_are_ordered_by_login() {
    let h = Harness::new(&["acme/app"], &[]);
    h.gh.set_open_authors(
        "acme/app",
        &[
            ("zoe", false),
            ("Bob", false),
            ("amy", false),
            ("zoe", false),
        ],
    );
    let list = h.engine.known_contributors().await.unwrap();
    assert_eq!(known(&list), vec![("zoe", 2), ("amy", 1), ("Bob", 1)]);
}

#[tokio::test(start_paused = true)]
async fn known_contributors_survive_one_project_failing_but_not_all() {
    let h = Harness::new(&["acme/app", "acme/lib"], &[]);
    h.gh.set_open_authors("acme/app", &[("bob", false)]);
    h.gh.set_open_authors("acme/lib", &[("carol", false)]);
    h.gh.set_repo_fail("acme/lib", Some(Fail::Network));
    let list = h.engine.known_contributors().await.unwrap();
    assert_eq!(known(&list), vec![("bob", 1)]);

    h.gh.set_repo_fail("acme/app", Some(Fail::Network));
    let err = h.engine.known_contributors().await.unwrap_err();
    assert!(matches!(err, AppError::Network(_)), "{err:?}");

    let empty = Harness::new(&[], &[]);
    assert!(empty.engine.known_contributors().await.unwrap().is_empty());
    assert_eq!(empty.gh.calls(), 0, "no projects, no network");
}
