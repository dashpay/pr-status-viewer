//! The IPC contract: everything the UI receives from the backend.
//!
//! These types are mirrored by hand in `src/types.ts`. Change both files
//! together, and never change either without the coordinator's say-so:
//! four sessions build against them in parallel.
//!
//! All JSON is camelCase. Timestamps are RFC 3339 strings in UTC.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// GitHub login of the review bot. Its reviews and check run are reported
/// as `PullRequest::bot`, never as a human review.
pub const BOT_LOGIN: &str = "thepastaclaw";
/// Name of the check run the bot attaches to each PR head commit.
pub const BOT_CHECK_NAME: &str = "PastaClaw review";
/// Name of the commit status the tracked repos use as a workflow gate. Its
/// description says whose move the PR is; it is not CI and is never counted
/// as CI.
pub const HYGIENE_CONTEXT: &str = "PR Hygiene";

/// Canonical PR id used across the app and in the config file.
pub fn pr_id(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

/// One complete picture of everything the UI shows. The backend replaces
/// the whole snapshot on every sync; the UI never patches it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generated_at: DateTime<Utc>,
    /// `None` until GitHub has told us who the user is.
    pub viewer: Option<Viewer>,
    /// True when T3 Code's threads were read (through its `t3` command), so
    /// `t3Threads` on each PR is meaningful. False when T3 Code is not
    /// installed, not running, or could not be read.
    #[serde(default)]
    pub t3_available: bool,
    /// True when no usable GitHub token could be found: the user has to sign
    /// in. False for every other failure (offline, rate limit, ...).
    #[serde(default)]
    pub sign_in_required: bool,
    /// One entry per configured project, in config order.
    pub projects: Vec<ProjectSnapshot>,
    pub github: SourceHealth,
    pub feed: FeedHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub login: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHealth {
    /// False when the most recent fetch failed.
    pub ok: bool,
    pub last_success_at: Option<DateTime<Utc>>,
    /// Human-readable reason for the most recent failure.
    pub error: Option<String>,
}

/// Health of the review bot's public status feed, plus its headline numbers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedHealth {
    pub ok: bool,
    pub last_success_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// When the bot generated the data (its `data_as_of`).
    pub data_as_of: Option<DateTime<Utc>>,
    pub running: u32,
    pub queued: u32,
    pub median_review_minutes: Option<f64>,
    /// True when the bot reports it is running on stand-in models.
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    /// `owner/name`.
    pub repo: String,
    /// Total open PRs in the repo, not only the ones listed in `prs`.
    pub open_count: u32,
    /// The viewer's own open PRs, the PRs they follow, and the open PRs of the
    /// contributors they track. Unordered. A tracked contributor's PR has
    /// `is_mine` and `is_followed` false unless it is also followed.
    pub prs: Vec<PullRequest>,
    /// Set when this project could not be fetched; `prs` is then the last
    /// good list, or empty.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    /// `owner/name#123`. Unique across the snapshot.
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub author_avatar_url: Option<String>,
    /// Title of the PR's milestone as GitHub shows it, e.g. "v5.1.0". `None`
    /// when the PR has no milestone.
    pub milestone: Option<String>,
    pub is_draft: bool,
    /// Authored by the viewer.
    pub is_mine: bool,
    /// In the config's followed list.
    pub is_followed: bool,
    pub additions: u32,
    pub deletions: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub head_sha: String,
    /// Name of the PR's head branch, e.g. "fix/sdk-timeout". `None` when
    /// GitHub did not report one.
    #[serde(default)]
    pub head_branch: Option<String>,
    /// CI on the head commit, not counting the bot's own check run.
    pub ci: CiState,
    pub review_decision: Option<ReviewDecision>,
    /// Human reviewers only: latest review per person, plus pending requests.
    pub humans: Vec<HumanReview>,
    pub bot: BotReview,
    /// How many reviews the review bot has posted on this PR: its reviews
    /// that carry its round marker. Its replies in threads and its "CI
    /// failed after approval" follow-ups are not counted. `None` until the
    /// PR's reviews have been read.
    pub rounds: Option<u32>,
    /// What the repo's "PR Hygiene" gate says happens next. `None` when the
    /// repo does not use the gate or it has said nothing yet.
    pub next_step: Option<NextStep>,
    /// When the most recent last review round of the review bot or
    /// CodeRabbit was submitted; failing that, when the bot's check run last
    /// completed.
    pub last_review_at: Option<DateTime<Utc>>,
    /// The findings of the last review round of the review bot and of
    /// CodeRabbit on this PR, by severity.
    pub finding_counts: FindingCounts,
    /// Notable things the PR does, as reported by workflow comments on it:
    /// at most one per kind, GroveDB first. Empty when there are none.
    #[serde(default)]
    pub traits: Vec<PrTrait>,
    /// The branch the PR merges into.
    pub base_branch: String,
    /// The number of the PR this one is stacked on: the PR in the same
    /// project, in this snapshot, whose head branch (not from a fork) is this
    /// PR's base branch. `None` when there is none, or when that branch is
    /// shared rather than a stack (`merge/stack.rs` decides).
    #[serde(default)]
    pub stacked_on: Option<u64>,
    /// Review threads ("conversations") not yet resolved, outdated ones
    /// included. Top-level PR comments cannot be resolved and do not count.
    pub unresolved_threads: u32,
    /// Whether the squash-merge button is enabled, and if not, why.
    pub merge: MergeReadiness,
    /// GitHub reports merge conflicts with the base branch. False while
    /// GitHub has not worked it out yet, which it does lazily.
    #[serde(default)]
    pub has_conflicts: bool,
    /// Whether the viewer can post the self-review the PR Hygiene gate asks
    /// of whoever holds the PR, and whether the gate is asking now.
    #[serde(default)]
    pub self_review: SelfReview,
    /// T3 Code threads tied to this PR, working ones first. Empty when there
    /// are none or T3 Code is absent.
    pub t3_threads: Vec<T3Thread>,
    /// True when the viewer is the one who has to act next.
    pub needs_me: bool,
    /// Short reason, e.g. "Changes requested" or "CI failing".
    pub needs_me_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CiState {
    Passing,
    Failing,
    Pending,
    /// No checks reported.
    None,
}

/// The next step in the PR's workflow, read from the "PR Hygiene" status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextStep {
    /// Stage code, lower case, e.g. "waiting-self-review".
    pub stage: String,
    /// Short display text, e.g. "Your move".
    pub label: String,
    /// The sentence after the stage code, as the gate wrote it.
    pub detail: Option<String>,
    pub actor: NextActor,
}

/// Who the next step is waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NextActor {
    Author,
    Bots,
    Reviewers,
    Ci,
    Merge,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanReview {
    pub login: String,
    pub avatar_url: Option<String>,
    pub state: HumanReviewState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HumanReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    /// Review requested, nothing submitted yet.
    Requested,
}

/// Where the bot's review of the PR's head commit stands.
///
/// Serialized as an object tagged by `state`, e.g.
/// `{"state":"reviewing","phase":"phase1",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BotReview {
    /// The bot has no check on this commit (repo not covered, or not yet seen).
    None,
    /// The bot skipped the PR because it is a draft.
    Draft,
    Queued {
        /// 1-based position in the bot's queue, when the feed lists the PR.
        position: Option<u32>,
        /// When the review can start (the feed's debounce deadline).
        starts_at: Option<DateTime<Utc>>,
        /// Our estimate, from the bot's median review time. Not the bot's.
        finishes_at: Option<DateTime<Utc>>,
        /// Why it is waiting, e.g. "debounce (more pushes may follow)".
        reason: Option<String>,
        priority: bool,
    },
    Reviewing {
        /// Feed phase key, e.g. "phase1".
        phase: String,
        /// Display form, e.g. "Phase 1".
        phase_label: String,
        /// 0.0 to 1.0, when the feed reports it.
        progress: Option<f32>,
        /// The bot's own estimate.
        finishes_at: Option<DateTime<Utc>>,
        started_at: Option<DateTime<Utc>>,
        /// The bot says this run is taking longer than usual.
        overdue: bool,
        /// Triage tier, e.g. "normal", "critical".
        tier: Option<String>,
        priority: bool,
        /// Seconds the PR sat in the queue before the run started.
        waited_seconds: Option<u64>,
        /// The whole pipeline in order: done, then the running step, then pending.
        steps: Vec<ReviewStep>,
        findings: Option<Findings>,
    },
    Approved {
        at: Option<DateTime<Utc>>,
    },
    ChangesRequested {
        at: Option<DateTime<Utc>>,
    },
    /// Reviewed with comments, no changes requested.
    Commented {
        at: Option<DateTime<Utc>>,
    },
    /// A check-run state we do not recognise; show `title` as is.
    Other {
        title: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStep {
    /// Feed step name, e.g. "phase1", "verify1", "publish".
    pub key: String,
    /// Display form, e.g. "Phase 1".
    pub label: String,
    pub status: StepStatus,
    /// Extra progress text, e.g. "0 of 2 lanes".
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Done,
    Running,
    Pending,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Findings {
    pub blocking: u32,
    pub suggestion: u32,
    pub nitpick: u32,
}

/// The findings of the last review rounds on a PR (the review bot's and
/// CodeRabbit's), by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingCounts {
    pub blocking: SeverityCount,
    pub suggestion: SeverityCount,
    pub nitpick: SeverityCount,
}

/// How the findings of one severity stand. Open = total - agreed -
/// not_agreed - awaiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeverityCount {
    pub total: u32,
    /// Settled with the reviewer's agreement: the bot marked the finding
    /// resolved, withdrawn, or no longer applicable; CodeRabbit marked it
    /// addressed.
    pub agreed: u32,
    /// Closed without agreement: disputed or deferred, or the thread was
    /// resolved on GitHub while the finding still stands although its
    /// reviewer's last round was on the PR's current head (it has seen the
    /// code and did not lift the finding).
    pub not_agreed: u32,
    /// Awaiting re-review: the thread was resolved on GitHub while the
    /// finding still stands, and its reviewer's last round was on an older
    /// commit than the PR's current head, so the fix is not re-checked yet.
    #[serde(default)]
    pub awaiting: u32,
    /// How many of `total` are CodeRabbit's findings (the rest are the review bot's).
    pub coderabbit: u32,
}

/// Something notable a pull request does, reported by a workflow comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrTrait {
    pub kind: TraitKind,
    /// One short line for a tooltip, e.g. "5 nodes changed" or
    /// "token-history-contract v3: new version".
    pub summary: String,
    /// Where to look at it. First is the one a plain click opens. Every
    /// link is an https URL on `dashpay.github.io` under the viewer of its
    /// kind; empty when the comment had none that qualified.
    pub links: Vec<TraitLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TraitKind {
    /// The PR changes the described GroveDB structure.
    GrovedbChange,
    /// The PR adds, changes or removes a system contract's document schemas.
    ContractChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraitLink {
    /// Short label, e.g. "Structure viewer" or "token-history-contract v3
    /// (compare with v2)".
    pub label: String,
    pub url: String,
}

/// Whether a PR can be squash-merged from the app right now.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReadiness {
    /// True only when every review thread is resolved AND GitHub itself
    /// would allow a squash merge by the viewer.
    pub ready: bool,
    /// When not ready: the first thing in the way, as a short sentence the
    /// button's tooltip can show, e.g. "3 unresolved comments".
    pub reason: Option<String>,
}

/// A T3 Code thread tied to a pull request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct T3Thread {
    pub thread_id: String,
    pub title: String,
    pub state: T3ThreadState,
    /// When the thread last ran or changed, if known.
    pub last_active_at: Option<DateTime<Utc>>,
    /// T3 Code watches this pull request for the thread (CI, reviews,
    /// merge) and wakes the thread when it changes.
    #[serde(default)]
    pub watching: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum T3ThreadState {
    /// An agent run is in flight right now.
    Working,
    /// The thread's turn has ended while it waits on something it started
    /// (a pull request watch, a background command, a delegated task); it
    /// resumes on its own when that finishes or changes.
    Waiting,
    /// The thread exists and is not settled, but nothing is running or
    /// pending.
    Idle,
}

/// The self-review attestation of the PR Hygiene gate: whoever holds a PR
/// (its author, or anyone it is assigned to) states that they have read the
/// final diff themselves, by commenting `/self-reviewed <head commit>`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfReview {
    pub state: SelfReviewState,
    /// For `Posted`: when the app posted it.
    pub posted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelfReviewState {
    /// Nothing to offer: the viewer does not hold the PR, it is a draft, the
    /// repo has no PR Hygiene gate, or the gate is past this step.
    #[default]
    Unavailable,
    /// The viewer could post it, but the gate is not asking for it right now
    /// (it is waiting on something else first, or may already have it).
    Possible,
    /// The gate says the next step is to post it.
    Wanted,
    /// The app posted it for the current head commit and the gate has not
    /// moved on yet (it re-evaluates a minute or two after the comment).
    Posted,
}

/// A fresh answer, straight from GitHub, to "can I post the self-review on
/// this PR now?". Fetched when the user opens the confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfReviewCheck {
    /// `owner/name#123`.
    pub id: String,
    /// The commit the self-review would cover. Pass it back to
    /// `post_self_review`, which refuses if the head has moved since.
    pub head_sha: String,
    /// Exactly what would be posted: `/self-reviewed <head_sha>`.
    pub comment: String,
    pub can_post: bool,
    /// When `can_post` is false: why, as a sentence for the user.
    pub reason: Option<String>,
    /// When the viewer last posted a self-review naming this commit, if they
    /// have. Posting again is allowed (the gate asks for a renewed one after
    /// an objection is answered).
    pub already_posted_at: Option<DateTime<Utc>>,
    /// What the gate says right now, e.g. "Bots are done - your move: post
    /// /self-reviewed.". `None` when it says nothing.
    pub gate: Option<String>,
    /// The PR's "Files changed" page, for a last look at the diff.
    pub diff_url: String,
}

/// A fresh answer, straight from GitHub, to "can this PR be squash-merged
/// now?". Fetched when the user opens the merge confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeCheck {
    /// `owner/name#123`.
    pub id: String,
    /// The commit that would be merged. Pass it back to `squash_merge`, which
    /// refuses if the head has moved since.
    pub head_sha: String,
    pub base_branch: String,
    /// The title GitHub will give the squash commit.
    pub commit_title: String,
    pub unresolved_threads: u32,
    pub merge: MergeReadiness,
}

/// A light row for the "browse a project's open PRs and follow some" list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrSummary {
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub is_draft: bool,
    pub is_mine: bool,
    pub is_followed: bool,
    pub updated_at: DateTime<Utc>,
}

/// What the user has chosen to track. Persisted as JSON in the app config dir.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Repos as `owner/name`, in the order the user added them.
    pub projects: Vec<String>,
    /// PR ids (`owner/name#123`) the user follows.
    pub followed: Vec<String>,
    /// GitHub logins of the contributors the user tracks, in the order they
    /// were added, in GitHub's own casing. The snapshot lists their open PRs
    /// in the tracked projects alongside the user's.
    pub contributors: Vec<String>,
}

/// Someone the app can offer to track: an author of open PRs in the tracked
/// projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownContributor {
    pub login: String,
    pub avatar_url: Option<String>,
    /// Their open PRs across the tracked projects.
    pub open_prs: u32,
}

/// Emitted on the `sync-state` event while a sync runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub syncing: bool,
}

// ---------------------------------------------------------------------------
// Running a project's example apps (`run/`).
// ---------------------------------------------------------------------------

/// Which example app to build and open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunPlatform {
    Ios,
    Android,
}

/// What can be run for one tracked project. One entry per configured
/// project, in config order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProject {
    /// `owner/name`.
    pub repo: String,
    /// The local clone of the repo, when one is known.
    pub clone_path: Option<String>,
    /// True when the user chose `clone_path`; false when it was found
    /// automatically (through T3 Code's projects).
    pub clone_is_set: bool,
    /// The example apps found in the clone. Empty when there is no clone or
    /// it has none, and then the UI offers nothing to run.
    pub platforms: Vec<RunPlatform>,
}

/// Everything the "run" dialog needs for one PR. Asked fresh each time the
/// dialog opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOptions {
    /// `owner/name#123`.
    pub id: String,
    pub clone_path: String,
    /// Where the code can come from. Never empty; the first is the default.
    pub sources: Vec<RunSource>,
    /// One entry per example app the project has.
    pub platforms: Vec<RunPlatformOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunSourceKind {
    /// The PR's head commit, in a checkout the app keeps for itself.
    PrHead,
    /// One of the user's own worktrees that is on the PR's branch, built in
    /// place with whatever is in it.
    Worktree,
}

/// One place the code to build can come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSource {
    pub kind: RunSourceKind,
    /// The worktree's directory; for `PrHead`, the app's own checkout.
    pub path: String,
    /// The branch checked out there. `None` for `PrHead` and for a detached
    /// worktree.
    pub branch: Option<String>,
    /// The commit that would be built: the PR's head for `PrHead`, the
    /// worktree's HEAD otherwise.
    pub sha: String,
    /// The commit is the PR's head commit. Always true for `PrHead`.
    pub at_pr_head: bool,
    /// The worktree has uncommitted changes. Always false for `PrHead`.
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPlatformOptions {
    pub platform: RunPlatform,
    /// `None` when it can run. Otherwise why it cannot, as a sentence for
    /// the user, e.g. "Xcode is not installed.".
    pub unavailable: Option<String>,
    /// Where it can run, the best choice first (running ones first).
    pub devices: Vec<RunDevice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunDeviceKind {
    /// An iOS simulator.
    Simulator,
    /// An Android virtual device.
    Emulator,
    /// A physical Android device connected over adb.
    Device,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDevice {
    /// Stable across runs: a simulator's UDID, an emulator's AVD name
    /// prefixed `avd:`, a physical device's adb serial prefixed `adb:`.
    pub id: String,
    /// e.g. "iPhone 18 Pro Max" or "palmatum_api35".
    pub name: String,
    /// e.g. "iOS 27.2". `None` when nothing useful is known.
    pub detail: Option<String>,
    pub kind: RunDeviceKind,
    /// Booted or connected right now.
    pub running: bool,
}

/// What the user chose in the run dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    /// `owner/name#123`.
    pub id: String,
    pub platform: RunPlatform,
    pub source_kind: RunSourceKind,
    /// For `Worktree`: the `path` of the chosen `RunSource`. Ignored for
    /// `PrHead`.
    #[serde(default)]
    pub source_path: Option<String>,
    /// A `RunDevice::id`.
    pub device_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// One build-and-open run. Only one runs at a time; the last one is kept
/// until the next starts. Emitted whole on the `run-job` event whenever it
/// changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunJob {
    /// Unique within one launch of the app; later jobs have higher ids.
    pub job_id: u64,
    /// `owner/name#123`.
    pub pr_id: String,
    pub platform: RunPlatform,
    /// e.g. "iPhone 18 Pro Max".
    pub device_name: String,
    pub source_kind: RunSourceKind,
    /// The commit being built, when known.
    pub sha: Option<String>,
    pub state: RunState,
    /// Every step in order: done, then the running one, then pending.
    pub steps: Vec<RunStep>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// When `state` is `Failed`: one sentence saying what went wrong.
    pub error: Option<String>,
    /// Where the complete log is kept.
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStep {
    /// Stable name, e.g. "checkout", "buildSdk", "buildApp", "device",
    /// "install", "open".
    pub key: String,
    /// Display text, e.g. "Build the SDK".
    pub label: String,
    pub status: StepStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// New output of the running job, emitted on the `run-log` event in batches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLog {
    pub job_id: u64,
    pub lines: Vec<String>,
}
