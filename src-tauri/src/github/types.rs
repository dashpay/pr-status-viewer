//! Raw facts about a project as GitHub reports them, before they are merged
//! with the review feed. This is the hand-off between the GitHub client
//! (which produces these) and `merge` (which consumes them).
//!
//! Frozen contract: do not change without the coordinator's say-so.

use chrono::{DateTime, Utc};

use crate::model::{CiState, PrTrait, ReviewDecision};

#[derive(Debug, Clone, PartialEq)]
pub struct GhProject {
    /// `owner/name`, in GitHub's canonical casing.
    pub repo: String,
    /// Total open PRs in the repo.
    pub open_count: u32,
    /// The viewer's open PRs, the tracked contributors' open PRs, and the
    /// followed PRs that are still open.
    pub prs: Vec<GhPullRequest>,
}

/// A GitHub account looked up by login (`repositoryOwner`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhAccount {
    /// In GitHub's casing.
    pub login: String,
    pub avatar_url: Option<String>,
    pub kind: GhAccountKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhAccountKind {
    User,
    Organization,
    /// Any other `__typename` GitHub may add.
    Other,
}

/// The author of one open PR, from the light open-PR listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhPrAuthor {
    /// "ghost" for a deleted account.
    pub login: String,
    pub avatar_url: Option<String>,
    /// A GitHub App account (GraphQL type `Bot`), e.g. "dependabot".
    pub is_bot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GhPullRequest {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub author_avatar_url: Option<String>,
    /// Logins the PR is assigned to (the first 10). With the author, they
    /// are who holds the PR for the PR Hygiene gate's self-review.
    pub assignees: Vec<String>,
    /// Title of the PR's milestone, e.g. "v5.1.0".
    pub milestone: Option<String>,
    pub is_draft: bool,
    pub additions: u32,
    pub deletions: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub head_sha: String,
    /// Name of the PR's head branch (`headRefName`), e.g. "fix/sdk-timeout".
    /// A fork's branch has the same form; it is not prefixed with the owner.
    pub head_branch: Option<String>,
    /// The head branch lives in a fork (`isCrossRepository`), so it can
    /// never be the base of another PR in the project. False when GitHub
    /// did not say.
    pub head_from_fork: bool,
    pub review_decision: Option<ReviewDecision>,
    /// Latest opinionated review per reviewer. Includes the bot's.
    pub reviews: Vec<GhReview>,
    /// People asked to review who have not yet done so.
    pub review_requests: Vec<GhReviewer>,
    /// CI on the head commit, computed WITHOUT the bot's check run. (GitHub's
    /// own rollup counts the bot's "changes requested" as a CI failure.)
    pub ci: CiState,
    /// The bot's check run on the head commit, if there is one.
    pub bot_check: Option<GhBotCheck>,
    /// The `model::HYGIENE_CONTEXT` status on the head commit, if there is
    /// one. Not counted in `ci`.
    pub hygiene: Option<GhHygiene>,
    /// What GitHub says about merging this PR.
    pub merge: GhMergeFacts,
    /// The findings of the last review round of the review bot and of
    /// CodeRabbit: one per review thread in that round.
    pub findings: Vec<GhFinding>,
    /// When each reviewer's last round was submitted, and on which commit.
    pub last_rounds: GhLastRounds,
    /// What the workflow comments on the PR report it does
    /// (`github::traits`): at most one per kind, GroveDB first.
    pub traits: Vec<PrTrait>,
    /// How many review posts the review bot has made on the PR: its
    /// reviews that carry its round marker (`github::rounds::PostCount`).
    /// `None` until its reviews have been read.
    pub rounds: Option<u32>,
}

/// A reviewer whose findings are counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GhFindingReviewer {
    /// The review bot, `model::BOT_LOGIN`.
    ReviewBot,
    /// CodeRabbit, the GitHub App `coderabbitai`.
    CodeRabbit,
}

/// When each reviewer's last review round was submitted, and on which
/// commit; `None` when it has not reviewed the PR (or its reviews could not
/// be read).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GhLastRounds {
    pub review_bot: Option<DateTime<Utc>>,
    pub coderabbit: Option<DateTime<Utc>>,
    /// The commit the review bot's last round was submitted on (the
    /// review's `commit.oid`), when GitHub reports it.
    pub review_bot_commit: Option<String>,
    /// The commit CodeRabbit's last round was submitted on, when known.
    pub coderabbit_commit: Option<String>,
}

impl GhLastRounds {
    /// The later of the two rounds.
    pub fn latest(&self) -> Option<DateTime<Utc>> {
        self.review_bot.max(self.coderabbit)
    }

    /// The commit `reviewer`'s last round was submitted on, when known.
    pub fn commit(&self, reviewer: GhFindingReviewer) -> Option<&str> {
        match reviewer {
            GhFindingReviewer::ReviewBot => self.review_bot_commit.as_deref(),
            GhFindingReviewer::CodeRabbit => self.coderabbit_commit.as_deref(),
        }
    }
}

/// One finding of a reviewer's last round, read from the root comment of
/// its review thread. `merge` turns these into `model::FindingCounts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GhFinding {
    pub reviewer: GhFindingReviewer,
    pub severity: GhSeverity,
    pub status: GhFindingStatus,
    /// The thread is marked resolved on GitHub (by anyone).
    pub thread_resolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhSeverity {
    Blocking,
    Suggestion,
    Nitpick,
}

/// The reviewer's own word on a finding. The review bot keeps a status line
/// in the root comment (`<!-- thepastaclaw-thread-status v1 -->`);
/// CodeRabbit appends "✅ Addressed in commit(s) ..." when it confirms a fix,
/// which reads as `Resolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhFindingStatus {
    /// No status line, or one that does not lift the finding ("Needs a
    /// tracking issue"): the finding stands.
    Stands,
    Resolved,
    Withdrawn,
    NoLongerApplies,
    Deferred,
    Disputed,
}

/// The raw facts that decide whether a squash merge is possible. `merge`
/// turns them into `model::MergeReadiness`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GhMergeFacts {
    /// GraphQL node id of the PR, needed by the merge mutation.
    pub node_id: String,
    /// The branch the PR merges into (`baseRefName`).
    pub base_branch: String,
    /// All review threads on the PR.
    pub review_threads: u32,
    /// Review threads with `isResolved` false, outdated ones included.
    pub unresolved_threads: u32,
    pub mergeable: GhMergeable,
    /// GitHub's `mergeStateStatus`, upper case: CLEAN, BLOCKED, BEHIND,
    /// DIRTY, DRAFT, HAS_HOOKS, UNSTABLE, UNKNOWN.
    pub merge_state: String,
    /// The repo allows squash merging.
    pub squash_allowed: bool,
    /// The viewer has write access or better to the repo.
    pub viewer_can_merge: bool,
    /// The repo uses a merge queue for the PR's base branch.
    pub merge_queue: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GhMergeable {
    Mergeable,
    Conflicting,
    /// GitHub has not finished computing it.
    #[default]
    Unknown,
}

/// A fresh look at one PR for the merge confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhMergeCheck {
    pub head_sha: String,
    pub is_draft: bool,
    /// False once the PR is merged or closed.
    pub is_open: bool,
    /// `viewerMergeHeadlineText(mergeMethod: SQUASH)`.
    pub commit_title: String,
    pub facts: GhMergeFacts,
}

/// A fresh look at one PR for the self-review confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct GhSelfReviewCheck {
    /// GraphQL node id of the PR, the subject of the comment mutation.
    pub node_id: String,
    /// The PR's page, e.g. "https://github.com/o/r/pull/7".
    pub url: String,
    pub head_sha: String,
    pub is_draft: bool,
    /// False once the PR is merged or closed.
    pub is_open: bool,
    pub author: String,
    /// Logins the PR is assigned to (the first 10).
    pub assignees: Vec<String>,
    /// The signed-in user's login.
    pub viewer: String,
    /// The `model::HYGIENE_CONTEXT` status on the head commit, if any.
    pub hygiene: Option<GhHygiene>,
    /// The PR's most recent top-level comments (the last 50), oldest first.
    pub comments: Vec<GhComment>,
}

/// A top-level comment on a PR.
#[derive(Debug, Clone, PartialEq)]
pub struct GhComment {
    /// "ghost" for a deleted account.
    pub author: String,
    /// Written by a GitHub App account (GraphQL `__typename` "Bot").
    pub author_is_bot: bool,
    pub body: String,
    pub created_at: DateTime<Utc>,
    /// The comment has been edited since it was posted.
    pub edited: bool,
}

/// The commit status named `model::HYGIENE_CONTEXT`.
#[derive(Debug, Clone, PartialEq)]
pub struct GhHygiene {
    /// GitHub's status state, upper case: PENDING, SUCCESS, FAILURE, ERROR.
    pub state: String,
    /// The raw description, e.g. "waiting-self-review: Bots are done - ...".
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GhReviewer {
    pub login: String,
    pub avatar_url: Option<String>,
    /// A GitHub App account (GraphQL `__typename` "Bot"), e.g. "coderabbitai".
    pub is_bot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GhReview {
    pub login: String,
    pub avatar_url: Option<String>,
    pub state: GhReviewState,
    pub submitted_at: Option<DateTime<Utc>>,
    /// Authored by a GitHub App account (GraphQL `__typename` "Bot").
    pub is_bot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

/// The check run named `model::BOT_CHECK_NAME`.
#[derive(Debug, Clone, PartialEq)]
pub struct GhBotCheck {
    pub status: GhCheckStatus,
    /// GitHub's conclusion, upper case: SUCCESS, FAILURE, NEUTRAL, CANCELLED...
    pub conclusion: Option<String>,
    /// The check's title, e.g. "Approved", "Changes requested",
    /// "Reviewing — Phase 1 review", "Not reviewed: the PR is a draft".
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhCheckStatus {
    Queued,
    InProgress,
    Completed,
}
