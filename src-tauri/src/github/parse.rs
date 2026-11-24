//! Pure parsing of GitHub GraphQL responses into `types.rs` values.
//!
//! Nothing here does I/O: the HTTP layer in `mod.rs` hands over the status,
//! a few headers and the body, and gets back typed data or an `AppError`.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::types::{
    GhAccount, GhAccountKind, GhBotCheck, GhCheckStatus, GhComment, GhHygiene, GhMergeCheck,
    GhMergeFacts, GhMergeable, GhPrAuthor, GhPullRequest, GhReview, GhReviewState, GhReviewer,
    GhSelfReviewCheck,
};
use crate::error::{AppError, AppResult};
use crate::model::{
    BOT_CHECK_NAME, CiState, HYGIENE_CONTEXT, PrSummary, ReviewDecision, Viewer, pr_id,
};

// ---------------------------------------------------------------------------
// Envelope and error classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GqlError {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

/// A response with data. `errors` holds partial errors (for example a
/// followed PR number that does not exist) that came alongside it.
#[derive(Debug)]
pub(crate) struct Gql<T> {
    pub data: T,
    pub errors: Vec<GqlError>,
}

/// `rateLimit { cost }`: what GitHub charged for the query. Free to ask for.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct RateLimit {
    pub cost: u32,
}

/// Response data that may carry `rateLimit { cost }`, so the client can
/// keep count of what its queries cost.
pub(crate) trait QueryCost {
    fn cost(&self) -> Option<u32> {
        None
    }
}

impl QueryCost for serde_json::Map<String, Value> {
    fn cost(&self) -> Option<u32> {
        self.get("rateLimit")?
            .get("cost")?
            .as_u64()
            .and_then(|c| u32::try_from(c).ok())
    }
}

/// Why a request failed. `TooCostly` means GitHub refused or timed out on the
/// query itself; the caller can retry with a smaller page.
#[derive(Debug)]
pub(crate) enum Failure {
    TooCostly(String),
    App(AppError),
}

impl From<AppError> for Failure {
    fn from(e: AppError) -> Self {
        Failure::App(e)
    }
}

impl From<Failure> for AppError {
    fn from(f: Failure) -> Self {
        match f {
            Failure::App(e) => e,
            Failure::TooCostly(msg) => {
                AppError::Other(format!("GitHub could not answer the query: {msg}"))
            }
        }
    }
}

/// The rate-limit headers of a response.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RateHeaders {
    /// `x-ratelimit-remaining`
    pub remaining: Option<u64>,
    /// `x-ratelimit-reset`, epoch seconds.
    pub reset: Option<i64>,
    /// `retry-after`, seconds.
    pub retry_after: Option<u64>,
}

impl RateHeaders {
    fn reset_at(&self, now: DateTime<Utc>) -> String {
        let at = self
            .retry_after
            .and_then(|s| i64::try_from(s).ok())
            .map(|s| now + chrono::Duration::seconds(s))
            .or_else(|| self.reset.and_then(|t| DateTime::from_timestamp(t, 0)));
        match at {
            Some(at) => at.to_rfc3339_opts(SecondsFormat::Secs, true),
            None => "an unknown time".to_owned(),
        }
    }
}

/// Turns an HTTP response into data or a classified error.
pub(crate) fn interpret<T: DeserializeOwned>(
    status: u16,
    rate: RateHeaders,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<Gql<T>, Failure> {
    let body_message = || {
        serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"))
    };

    match status {
        200..=299 => {}
        401 => return Err(AppError::Unauthorized(body_message()).into()),
        403 | 429 => {
            let msg = body_message();
            let limited = rate.remaining == Some(0)
                || rate.retry_after.is_some()
                || status == 429
                || msg.to_ascii_lowercase().contains("rate limit");
            return Err(if limited {
                AppError::RateLimited {
                    reset_at: rate.reset_at(now),
                }
            } else {
                AppError::Unauthorized(msg)
            }
            .into());
        }
        502 | 504 => return Err(Failure::TooCostly(format!("HTTP {status}"))),
        _ => {
            return Err(AppError::Network(format!(
                "GitHub returned {}",
                body_message_or(status, body)
            ))
            .into());
        }
    }

    // Now and then GitHub gives up on a heavy query after it has sent a 2xx:
    // the body is then empty or ends early. That is the query timing out, as
    // a 502 would say, so callers retry it with a smaller page.
    if body.iter().all(u8::is_ascii_whitespace) {
        return Err(Failure::TooCostly(format!(
            "HTTP {status} with an empty body"
        )));
    }
    let env: Envelope<T> = match serde_json::from_slice(body) {
        Ok(env) => env,
        Err(e) if e.is_eof() => {
            return Err(Failure::TooCostly(format!(
                "HTTP {status} with a body that ends early"
            )));
        }
        Err(e) => return Err(AppError::from(e).into()),
    };
    let errors = env.errors.unwrap_or_default();

    if errors
        .iter()
        .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
    {
        return Err(AppError::RateLimited {
            reset_at: rate.reset_at(now),
        }
        .into());
    }

    match env.data {
        Some(data) => Ok(Gql { data, errors }),
        None => Err(classify_errors(&errors)),
    }
}

fn body_message_or(status: u16, body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned))
        .map(|m| format!("HTTP {status}: {m}"))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

fn classify_errors(errors: &[GqlError]) -> Failure {
    let Some(first) = errors.first() else {
        return AppError::Decode("GitHub sent neither data nor errors".into()).into();
    };
    if let Some(e) = errors
        .iter()
        .find(|e| e.kind.as_deref() == Some("NOT_FOUND"))
    {
        return AppError::NotFound(e.message.clone()).into();
    }
    if errors.iter().any(is_cost_error) {
        return Failure::TooCostly(first.message.clone());
    }
    AppError::Other(first.message.clone()).into()
}

fn is_cost_error(e: &GqlError) -> bool {
    if matches!(
        e.kind.as_deref(),
        Some("MAX_NODE_LIMIT_EXCEEDED" | "RESOURCE_LIMITS_EXCEEDED" | "EXCESSIVE_PAGINATION")
    ) {
        return true;
    }
    let m = e.message.to_ascii_lowercase();
    [
        "timeout",
        "timed out",
        "node limit",
        "too complex",
        "complexity",
        "in time",
    ]
    .iter()
    .any(|needle| m.contains(needle))
}

/// What to say when `repository` came back null.
pub(crate) fn repo_missing(errors: &[GqlError], repo: &str) -> AppError {
    match errors
        .iter()
        .find(|e| e.kind.as_deref() != Some("NOT_FOUND"))
    {
        Some(other) => AppError::Other(other.message.clone()),
        None => AppError::NotFound(format!(
            "{repo} does not exist, or your GitHub account cannot see it"
        )),
    }
}

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct Connection<T> {
    #[serde(default = "Vec::new")]
    pub nodes: Vec<Option<T>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Actor {
    pub login: String,
    pub avatar_url: Option<String>,
    /// "User", "Bot", "Mannequin"... Only asked for where it matters.
    #[serde(rename = "__typename", default)]
    pub typename: Option<String>,
}

impl Actor {
    /// A GitHub App account, such as a code-review app.
    fn is_bot(&self) -> bool {
        self.typename.as_deref() == Some("Bot")
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ViewerData {
    viewer: Actor,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepoName {
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepoData {
    repository: Option<RepoName>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Count {
    total_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MineRepo {
    name_with_owner: String,
    #[serde(flatten)]
    access: RepoAccess,
    pull_requests: Count,
}

/// What the repository allows the viewer when it comes to merging.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepoAccess {
    #[serde(default)]
    squash_merge_allowed: bool,
    /// ADMIN, MAINTAIN, WRITE, TRIAGE or READ; null for anonymous access.
    #[serde(default, deserialize_with = "permission")]
    viewer_permission: bool,
}

impl RepoAccess {
    /// Copies the repo-wide facts onto one PR's merge facts.
    pub fn apply(self, facts: &mut GhMergeFacts) {
        facts.squash_allowed = self.squash_merge_allowed;
        facts.viewer_can_merge = self.viewer_permission;
    }
}

/// `viewerPermission` -> "can merge": WRITE, MAINTAIN or ADMIN.
fn permission<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let level = Option::<String>::deserialize(d)?;
    Ok(matches!(
        level.as_deref(),
        Some("WRITE" | "MAINTAIN" | "ADMIN")
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Search {
    page_info: PageInfo,
    #[serde(default)]
    nodes: Vec<Option<Value>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MineData {
    repository: Option<MineRepo>,
    search: Search,
    #[serde(default, rename = "rateLimit")]
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FollowedData {
    repository: Option<serde_json::Map<String, Value>>,
    #[serde(default, rename = "rateLimit")]
    rate_limit: Option<RateLimit>,
}

impl QueryCost for MineData {
    fn cost(&self) -> Option<u32> {
        self.rate_limit.map(|r| r.cost)
    }
}

impl QueryCost for FollowedData {
    fn cost(&self) -> Option<u32> {
        self.rate_limit.map(|r| r.cost)
    }
}

impl QueryCost for ThreadsData {
    fn cost(&self) -> Option<u32> {
        self.rate_limit.map(|r| r.cost)
    }
}

impl QueryCost for ViewerData {}
impl QueryCost for RepoData {}
impl QueryCost for OpenPrsData {
    fn cost(&self) -> Option<u32> {
        self.rate_limit.map(|r| r.cost)
    }
}
impl QueryCost for AccountData {}
impl QueryCost for MergeCheckData {}
impl QueryCost for SquashMergeData {}
impl QueryCost for SelfReviewCheckData {}
impl QueryCost for AddCommentData {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryNode {
    number: u64,
    title: String,
    url: String,
    is_draft: bool,
    updated_at: DateTime<Utc>,
    author: Option<Actor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenPrsConnection {
    page_info: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<SummaryNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenPrsRepo {
    name_with_owner: String,
    pull_requests: OpenPrsConnection,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenPrsData {
    repository: Option<OpenPrsRepo>,
    #[serde(default, rename = "rateLimit")]
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountData {
    repository_owner: Option<Actor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrNode {
    number: u64,
    title: String,
    url: String,
    state: String,
    is_draft: bool,
    additions: u32,
    deletions: u32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    head_ref_oid: String,
    #[serde(default)]
    head_ref_name: Option<String>,
    /// The head branch lives in a fork. Missing or null (older captures)
    /// reads as false.
    #[serde(default)]
    is_cross_repository: Option<bool>,
    #[serde(default)]
    milestone: Option<MilestoneNode>,
    #[serde(flatten)]
    merge: MergeNode,
    review_decision: Option<String>,
    author: Option<Actor>,
    /// Tolerated missing: older captures of the page query lack it.
    #[serde(default)]
    assignees: Option<Connection<Login>>,
    latest_reviews: Option<Connection<ReviewNode>>,
    review_requests: Option<Connection<ReviewRequestNode>>,
    commits: Connection<CommitNode>,
}

/// An account where only its login is asked for.
#[derive(Debug, Deserialize)]
pub(crate) struct Login {
    login: String,
}

/// The logins of an `assignees` connection, blanks and nulls dropped.
fn logins(connection: Option<Connection<Login>>) -> Vec<String> {
    connection
        .map(|c| c.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .map(|l| l.login)
        .filter(|l| !l.is_empty())
        .collect()
}

#[derive(Debug, Deserialize)]
pub(crate) struct MilestoneNode {
    title: String,
}

/// The `MergeFields` fragment.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeNode {
    id: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    merge_state_status: Option<String>,
    #[serde(default)]
    is_merge_queue_enabled: bool,
    /// Required: without it the count would read as zero unresolved.
    review_threads: ThreadConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadConnection {
    total_count: u32,
    page_info: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<ThreadNode>>,
}

/// One review thread as the page query sees it: no comments, so it stays
/// cheap. What the thread says is fetched separately (`findings_query`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadNode {
    /// Tolerated missing: such a thread is counted but cannot carry a finding.
    #[serde(default)]
    id: Option<String>,
    is_resolved: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewNode {
    state: String,
    submitted_at: Option<DateTime<Utc>>,
    author: Option<Actor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewRequestNode {
    requested_reviewer: Option<RequestedReviewer>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
pub(crate) enum RequestedReviewer {
    User(Actor),
    Bot(Actor),
    Mannequin(Actor),
    Team {
        slug: String,
        organization: Org,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Org {
    login: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CommitNode {
    commit: Commit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Rollup {
    contexts: Connection<CheckContext>,
}

/// One entry of `statusCheckRollup.contexts`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "__typename")]
pub(crate) enum CheckContext {
    #[serde(rename_all = "camelCase")]
    CheckRun {
        name: String,
        status: String,
        conclusion: Option<String>,
        title: Option<String>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    },
    StatusContext {
        #[serde(default)]
        context: String,
        state: String,
        #[serde(default)]
        description: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeCheckData {
    repository: Option<MergeCheckRepo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeCheckRepo {
    #[serde(flatten)]
    access: RepoAccess,
    pull_request: Option<MergeCheckPr>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeCheckPr {
    state: String,
    is_draft: bool,
    head_ref_oid: String,
    #[serde(default)]
    viewer_merge_headline_text: Option<String>,
    #[serde(flatten)]
    merge: MergeNode,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThreadsData {
    node: Option<ThreadsNode>,
    #[serde(default, rename = "rateLimit")]
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadsNode {
    review_threads: Option<ThreadConnection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SquashMergeData {
    merge_pull_request: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SelfReviewCheckData {
    viewer: Login,
    repository: Option<SelfReviewCheckRepo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelfReviewCheckRepo {
    pull_request: Option<SelfReviewCheckPr>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelfReviewCheckPr {
    id: String,
    url: String,
    state: String,
    is_draft: bool,
    head_ref_oid: String,
    author: Option<Actor>,
    #[serde(default)]
    assignees: Option<Connection<Login>>,
    #[serde(default)]
    commits: Option<Connection<CommitNode>>,
    #[serde(default)]
    comments: Option<Connection<CommentNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommentNode {
    author: Option<Actor>,
    #[serde(default)]
    body: String,
    created_at: DateTime<Utc>,
    #[serde(default)]
    last_edited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddCommentData {
    add_comment: Option<AddCommentPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddCommentPayload {
    comment_edge: Option<CommentEdge>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CommentEdge {
    node: Option<NewComment>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NewComment {
    #[serde(default)]
    id: Option<String>,
}

// ---------------------------------------------------------------------------
// Review threads
// ---------------------------------------------------------------------------

/// Running count of a PR's review threads across pages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ThreadCount {
    /// GitHub's `totalCount`.
    pub total: u32,
    pub resolved: u32,
    pub unresolved: u32,
}

impl ThreadCount {
    /// Adds one page, and its threads to `threads`; returns the cursor of
    /// the next page, if there is one.
    pub fn add_page(
        &mut self,
        page: ThreadConnection,
        threads: &mut Vec<ThreadRef>,
    ) -> Option<String> {
        self.total = self.total.max(page.total_count);
        for node in page.nodes {
            match node {
                Some(node) => {
                    if node.is_resolved {
                        self.resolved += 1;
                    } else {
                        self.unresolved += 1;
                    }
                    if let Some(id) = node.id.filter(|id| !id.is_empty()) {
                        threads.push(ThreadRef {
                            id,
                            resolved: node.is_resolved,
                        });
                    }
                }
                // A thread we cannot see is not known to be resolved.
                None => self.unresolved += 1,
            }
        }
        if page.page_info.has_next_page {
            page.page_info.end_cursor
        } else {
            None
        }
    }

    /// Every thread not seen resolved counts as unresolved, so a count cut
    /// short (unseen pages, a short page) never reads as zero.
    pub fn unresolved(&self) -> u32 {
        self.unresolved
            .max(self.total.saturating_sub(self.resolved))
    }

    pub fn threads(&self) -> u32 {
        self.total.max(self.resolved + self.unresolved)
    }

    pub fn apply(&self, facts: &mut GhMergeFacts) {
        facts.review_threads = self.threads();
        facts.unresolved_threads = self.unresolved();
    }
}

/// One review thread: its id and whether it is marked resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadRef {
    pub id: String,
    pub resolved: bool,
}

/// A PR's review threads as the page queries see them (ids and resolved
/// flags, no comments), with the count, and the cursor of the next page
/// while there is more to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrThreads {
    pub number: u64,
    pub node_id: String,
    pub count: ThreadCount,
    /// In GitHub's order. A thread that came back null or without an id is
    /// counted in `count` but not listed.
    pub threads: Vec<ThreadRef>,
    /// Set while pages remain.
    pub after: Option<String>,
}

/// One further page of a PR's review threads.
pub(crate) fn parse_threads_page(
    resp: Gql<ThreadsData>,
    more: &mut PrThreads,
) -> Result<Option<String>, AppError> {
    let page = resp
        .data
        .node
        .and_then(|n| n.review_threads)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "the review threads of pull request #{} could not be read",
                more.number
            ))
        })?;
    Ok(more.count.add_page(page, &mut more.threads))
}

/// Merge facts of one PR (repo-wide facts not yet applied), and its review
/// threads from the first page, with the cursor of the next if any.
fn merge_facts(node: MergeNode, number: u64) -> (GhMergeFacts, PrThreads) {
    let mut count = ThreadCount::default();
    let mut threads = Vec::new();
    let after = count.add_page(node.review_threads, &mut threads);
    let mut facts = GhMergeFacts {
        node_id: node.id,
        base_branch: node.base_ref_name,
        review_threads: 0,
        unresolved_threads: 0,
        mergeable: match node.mergeable.as_deref() {
            Some("MERGEABLE") => GhMergeable::Mergeable,
            Some("CONFLICTING") => GhMergeable::Conflicting,
            _ => GhMergeable::Unknown,
        },
        merge_state: node
            .merge_state_status
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "UNKNOWN".to_owned()),
        squash_allowed: false,
        viewer_can_merge: false,
        merge_queue: node.is_merge_queue_enabled,
    };
    count.apply(&mut facts);
    let threads = PrThreads {
        number,
        node_id: facts.node_id.clone(),
        count,
        threads,
        after,
    };
    (facts, threads)
}

/// A fresh merge check of one PR, and its review threads so far.
pub(crate) fn parse_merge_check(
    resp: Gql<MergeCheckData>,
    repo: &str,
    number: u64,
) -> Result<(GhMergeCheck, PrThreads), AppError> {
    let Gql { data, errors } = resp;
    let repository = data.repository.ok_or_else(|| repo_missing(&errors, repo))?;
    let pr = repository
        .pull_request
        .ok_or_else(|| AppError::NotFound(format!("{} does not exist", pr_id(repo, number))))?;
    let (mut facts, threads) = merge_facts(pr.merge, number);
    repository.access.apply(&mut facts);
    Ok((
        GhMergeCheck {
            head_sha: pr.head_ref_oid,
            is_draft: pr.is_draft,
            is_open: pr.state == "OPEN",
            commit_title: pr.viewer_merge_headline_text.unwrap_or_default(),
            facts,
        },
        threads,
    ))
}

/// What came back from the `mergePullRequest` mutation. GitHub's own
/// message is kept in every refusal.
pub(crate) fn interpret_squash_merge(
    result: Result<Gql<SquashMergeData>, Failure>,
) -> AppResult<()> {
    match result {
        Ok(Gql { data, errors }) => match data.merge_pull_request {
            Some(payload) if !payload.is_null() => Ok(()),
            _ => Err(merge_refused(
                errors
                    .first()
                    .map(|e| e.message.as_str())
                    .unwrap_or("GitHub did not merge the pull request."),
            )),
        },
        // Never retried: GitHub may have merged before giving up.
        Err(Failure::TooCostly(_)) => Err(AppError::Other(
            "GitHub did not finish answering the merge request. Check the pull request on GitHub before trying again.".into(),
        )),
        Err(Failure::App(AppError::Network(m))) => Err(AppError::Network(format!(
            "{m}. The merge may have gone through; check the pull request on GitHub before trying again."
        ))),
        Err(Failure::App(AppError::Other(m))) => Err(merge_refused(&m)),
        Err(Failure::App(e)) => Err(e),
    }
}

/// GitHub's words when `expectedHeadOid` no longer matches the branch.
fn is_head_moved(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("head branch was modified")
        || m.contains("expected head")
        || m.contains("expectedheadoid")
        || (m.contains("head") && m.contains("modified"))
}

fn merge_refused(message: &str) -> AppError {
    let message = message.trim();
    if is_head_moved(message) {
        AppError::InvalidInput(format!(
            "New commits were pushed since you looked. Review them, then try again. (GitHub: {message})"
        ))
    } else {
        AppError::Other(format!("GitHub refused the merge: {message}"))
    }
}

/// A fresh self-review check of one PR.
pub(crate) fn parse_self_review_check(
    resp: Gql<SelfReviewCheckData>,
    repo: &str,
    number: u64,
) -> Result<GhSelfReviewCheck, AppError> {
    let Gql { data, errors } = resp;
    let repository = data.repository.ok_or_else(|| repo_missing(&errors, repo))?;
    let pr = repository
        .pull_request
        .ok_or_else(|| AppError::NotFound(format!("{} does not exist", pr_id(repo, number))))?;
    let contexts = head_contexts(pr.commits);
    let comments = pr
        .comments
        .map(|c| c.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .map(|c| GhComment {
            author: login_or_ghost(c.author.as_ref()),
            author_is_bot: c.author.as_ref().is_some_and(Actor::is_bot),
            body: c.body,
            created_at: c.created_at,
            edited: c.last_edited_at.is_some(),
        })
        .collect();
    Ok(GhSelfReviewCheck {
        node_id: pr.id,
        url: pr.url,
        head_sha: pr.head_ref_oid,
        is_draft: pr.is_draft,
        is_open: pr.state == "OPEN",
        author: login_or_ghost(pr.author.as_ref()),
        assignees: logins(pr.assignees),
        viewer: data.viewer.login,
        hygiene: pick_hygiene(&contexts),
        comments,
    })
}

/// What came back from the `addComment` mutation. Any GraphQL error counts
/// as a failure, even beside data. Never retried: the comment may have been
/// posted before GitHub gave up.
pub(crate) fn interpret_add_comment(result: Result<Gql<AddCommentData>, Failure>) -> AppResult<()> {
    const CHECK: &str = "Check the pull request on GitHub before trying again.";
    match result {
        Ok(Gql { data, errors }) => {
            let posted = data
                .add_comment
                .and_then(|p| p.comment_edge)
                .and_then(|e| e.node)
                .and_then(|n| n.id)
                .is_some_and(|id| !id.is_empty());
            match errors.first() {
                Some(e) if posted => Err(AppError::Other(format!(
                    "GitHub posted the comment but also reported an error: {}. {CHECK}",
                    e.message.trim()
                ))),
                Some(e) => Err(AppError::Other(format!(
                    "GitHub did not post the comment: {}",
                    e.message.trim()
                ))),
                None if posted => Ok(()),
                None => Err(AppError::Other(format!(
                    "GitHub did not confirm the comment. {CHECK}"
                ))),
            }
        }
        Err(Failure::TooCostly(_)) => Err(AppError::Other(format!(
            "GitHub did not finish answering. {CHECK}"
        ))),
        Err(Failure::App(AppError::Network(m))) => Err(AppError::Network(format!(
            "{m}. The comment may have been posted; check the pull request on GitHub before trying again."
        ))),
        Err(Failure::App(AppError::Other(m))) => Err(AppError::Other(format!(
            "GitHub did not post the comment: {m}"
        ))),
        Err(Failure::App(e)) => Err(e),
    }
}

/// The status and check contexts on the last commit of a `commits(last: 1)`
/// connection.
fn head_contexts(commits: Option<Connection<CommitNode>>) -> Vec<CheckContext> {
    commits
        .map(|c| c.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .next_back()
        .and_then(|c| c.commit.status_check_rollup)
        .map(|r| r.contexts.nodes.into_iter().flatten().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

pub(crate) fn parse_viewer(data: ViewerData) -> Viewer {
    Viewer {
        login: data.viewer.login,
        avatar_url: data.viewer.avatar_url,
    }
}

pub(crate) fn parse_repo_name(resp: Gql<RepoData>, repo: &str) -> Result<String, AppError> {
    resp.data
        .repository
        .map(|r| r.name_with_owner)
        .ok_or_else(|| repo_missing(&resp.errors, repo))
}

/// One page of the viewer's own open PRs.
#[derive(Debug)]
pub(crate) struct MinePage {
    pub repo: String,
    pub open_count: u32,
    pub access: RepoAccess,
    pub prs: Vec<GhPullRequest>,
    /// The review threads of each PR in `prs`.
    pub threads: Vec<PrThreads>,
    /// Cursor for the next page, when there is one.
    pub next: Option<String>,
}

pub(crate) fn parse_mine_page(resp: Gql<MineData>, repo: &str) -> Result<MinePage, AppError> {
    let Gql { data, errors } = resp;
    let repository = data.repository.ok_or_else(|| repo_missing(&errors, repo))?;
    let canonical = repository.name_with_owner;
    let mut prs = Vec::new();
    let mut threads = Vec::new();
    for node in data.search.nodes.into_iter().flatten() {
        // Search returns `{}` for result types the fragment does not cover.
        if node.get("number").is_none() {
            continue;
        }
        let node: PrNode = serde_json::from_value(node)?;
        if node.state == "OPEN" {
            let (pr, t) = pr_from_node(node, &canonical);
            prs.push(pr);
            threads.push(t);
        }
    }
    let info = data.search.page_info;
    Ok(MinePage {
        repo: canonical,
        open_count: repository.pull_requests.total_count,
        access: repository.access,
        prs,
        threads,
        next: if info.has_next_page {
            info.end_cursor
        } else {
            None
        },
    })
}

/// Followed PRs and their review threads.
#[derive(Debug)]
pub(crate) struct FollowedPage {
    pub prs: Vec<GhPullRequest>,
    pub threads: Vec<PrThreads>,
}

/// The followed PRs that are still open. Aliases that came back null
/// (no such PR) are dropped, as are closed and merged PRs.
pub(crate) fn parse_followed(
    resp: Gql<FollowedData>,
    repo: &str,
) -> Result<FollowedPage, AppError> {
    let Gql { data, errors } = resp;
    let aliases = data.repository.ok_or_else(|| repo_missing(&errors, repo))?;
    let mut prs = Vec::new();
    let mut threads = Vec::new();
    for (_, node) in aliases {
        if node.is_null() {
            continue;
        }
        let node: PrNode = serde_json::from_value(node)?;
        if node.state == "OPEN" {
            let (pr, t) = pr_from_node(node, repo);
            prs.push(pr);
            threads.push(t);
        }
    }
    prs.sort_by_key(|p| p.number);
    Ok(FollowedPage { prs, threads })
}

/// One page of a repo's open PRs as light rows.
#[derive(Debug)]
pub(crate) struct OpenPage {
    pub rows: Vec<OpenRow>,
    pub next: Option<String>,
}

/// One open PR, and who wrote it.
#[derive(Debug)]
pub(crate) struct OpenRow {
    pub summary: PrSummary,
    pub author: GhPrAuthor,
}

pub(crate) fn parse_open_page(resp: Gql<OpenPrsData>, repo: &str) -> Result<OpenPage, AppError> {
    let Gql { data, errors } = resp;
    let repository = data.repository.ok_or_else(|| repo_missing(&errors, repo))?;
    let canonical = repository.name_with_owner;
    let rows = repository
        .pull_requests
        .nodes
        .into_iter()
        .flatten()
        .map(|n| {
            let author = GhPrAuthor {
                login: login_or_ghost(n.author.as_ref()),
                avatar_url: n.author.as_ref().and_then(|a| a.avatar_url.clone()),
                is_bot: n.author.as_ref().is_some_and(Actor::is_bot),
            };
            let summary = PrSummary {
                id: pr_id(&canonical, n.number),
                repo: canonical.clone(),
                number: n.number,
                title: n.title,
                url: n.url,
                author: author.login.clone(),
                is_draft: n.is_draft,
                is_mine: false,
                is_followed: false,
                updated_at: n.updated_at,
            };
            OpenRow { summary, author }
        })
        .collect();
    let info = repository.pull_requests.page_info;
    Ok(OpenPage {
        rows,
        next: if info.has_next_page {
            info.end_cursor
        } else {
            None
        },
    })
}

/// The account `login` names, or `None` when there is none. A GraphQL
/// NOT_FOUND is "none" too; any other error with no account is an error.
pub(crate) fn parse_account(
    result: Result<Gql<AccountData>, Failure>,
) -> AppResult<Option<GhAccount>> {
    let resp = match result {
        Ok(resp) => resp,
        Err(Failure::App(AppError::NotFound(_))) => return Ok(None),
        Err(f) => return Err(f.into()),
    };
    let Some(owner) = resp.data.repository_owner else {
        return match resp
            .errors
            .iter()
            .find(|e| e.kind.as_deref() != Some("NOT_FOUND"))
        {
            Some(other) => Err(AppError::Other(other.message.clone())),
            None => Ok(None),
        };
    };
    let kind = match owner.typename.as_deref() {
        Some("User") => GhAccountKind::User,
        Some("Organization") => GhAccountKind::Organization,
        _ => GhAccountKind::Other,
    };
    Ok(Some(GhAccount {
        login: owner.login,
        avatar_url: owner.avatar_url,
        kind,
    }))
}

/// The viewer's PRs, then the followed ones that are not already listed.
pub(crate) fn merge_prs(
    mine: Vec<GhPullRequest>,
    followed: Vec<GhPullRequest>,
) -> Vec<GhPullRequest> {
    let mut out: Vec<GhPullRequest> = Vec::with_capacity(mine.len() + followed.len());
    for pr in mine.into_iter().chain(followed) {
        if !out.iter().any(|p| p.number == pr.number) {
            out.push(pr);
        }
    }
    out
}

fn login_or_ghost(actor: Option<&Actor>) -> String {
    // GitHub returns a null author for deleted accounts and shows them as "ghost".
    actor.map_or_else(|| "ghost".to_owned(), |a| a.login.clone())
}

/// One PR, with repo-wide merge facts not yet applied and no findings yet
/// (they come from a separate query), plus its review threads.
pub(crate) fn pr_from_node(node: PrNode, repo: &str) -> (GhPullRequest, PrThreads) {
    let (merge, threads) = merge_facts(node.merge, node.number);
    let contexts = head_contexts(Some(node.commits));

    let reviews = node
        .latest_reviews
        .map(|c| c.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let state = match r.state.as_str() {
                "APPROVED" => GhReviewState::Approved,
                "CHANGES_REQUESTED" => GhReviewState::ChangesRequested,
                "COMMENTED" => GhReviewState::Commented,
                "DISMISSED" => GhReviewState::Dismissed,
                // PENDING is an unsubmitted draft review; nobody else sees it.
                _ => return None,
            };
            Some(GhReview {
                login: login_or_ghost(r.author.as_ref()),
                is_bot: r.author.as_ref().is_some_and(Actor::is_bot),
                avatar_url: r.author.and_then(|a| a.avatar_url),
                state,
                submitted_at: r.submitted_at,
            })
        })
        .collect();

    let review_requests = node
        .review_requests
        .map(|c| c.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter_map(|r| match r.requested_reviewer? {
            RequestedReviewer::User(a) | RequestedReviewer::Mannequin(a) => Some(GhReviewer {
                login: a.login,
                avatar_url: a.avatar_url,
                is_bot: false,
            }),
            RequestedReviewer::Bot(a) => Some(GhReviewer {
                login: a.login,
                avatar_url: a.avatar_url,
                is_bot: true,
            }),
            RequestedReviewer::Team { slug, organization } => Some(GhReviewer {
                login: format!("{}/{slug}", organization.login),
                avatar_url: None,
                is_bot: false,
            }),
            RequestedReviewer::Unknown => None,
        })
        .collect();

    let review_decision = match node.review_decision.as_deref() {
        Some("APPROVED") => Some(ReviewDecision::Approved),
        Some("CHANGES_REQUESTED") => Some(ReviewDecision::ChangesRequested),
        Some("REVIEW_REQUIRED") => Some(ReviewDecision::ReviewRequired),
        _ => None,
    };

    let pr = GhPullRequest {
        repo: repo.to_owned(),
        number: node.number,
        title: node.title,
        url: node.url,
        author: login_or_ghost(node.author.as_ref()),
        author_avatar_url: node.author.and_then(|a| a.avatar_url),
        assignees: logins(node.assignees),
        milestone: node
            .milestone
            .map(|m| m.title.trim().to_owned())
            .filter(|t| !t.is_empty()),
        is_draft: node.is_draft,
        additions: node.additions,
        deletions: node.deletions,
        created_at: node.created_at,
        updated_at: node.updated_at,
        head_sha: node.head_ref_oid,
        head_branch: node.head_ref_name.filter(|b| !b.is_empty()),
        head_from_fork: node.is_cross_repository.unwrap_or(false),
        review_decision,
        reviews,
        review_requests,
        ci: compute_ci(&contexts),
        bot_check: pick_bot_check(&contexts),
        hygiene: pick_hygiene(&contexts),
        findings: Vec::new(),
        last_rounds: Default::default(),
        traits: Vec::new(),
        rounds: None,
        merge,
    };
    (pr, threads)
}

/// CI state of a commit, ignoring the bot's check run and the
/// `HYGIENE_CONTEXT` status (a workflow gate, not a build).
///
/// - Failing: any check run concluded FAILURE, TIMED_OUT, ACTION_REQUIRED or
///   STARTUP_FAILURE, or any status context is FAILURE or ERROR.
/// - Pending: otherwise, any check run not yet completed, or any status
///   context PENDING or EXPECTED.
/// - Passing: otherwise, at least one check counted.
/// - None: no checks besides those two.
///
/// SUCCESS, NEUTRAL, SKIPPED, STALE and CANCELLED count as passing.
/// CANCELLED is not a failure here: on a PR head it almost always means a
/// newer run in the same concurrency group superseded it, or someone stopped
/// a job on purpose. Neither says anything is broken, and showing it as
/// failing would cry wolf. An unknown conclusion also counts as passing.
pub(crate) fn compute_ci(contexts: &[CheckContext]) -> CiState {
    let mut failing = false;
    let mut pending = false;
    let mut counted = 0usize;
    for ctx in contexts {
        match ctx {
            CheckContext::CheckRun {
                name,
                status,
                conclusion,
                ..
            } => {
                if name == BOT_CHECK_NAME {
                    continue;
                }
                counted += 1;
                if status != "COMPLETED" {
                    pending = true;
                    continue;
                }
                match conclusion.as_deref() {
                    Some("FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE") => {
                        failing = true
                    }
                    None => pending = true,
                    _ => {}
                }
            }
            CheckContext::StatusContext { context, state, .. } => {
                if context == HYGIENE_CONTEXT {
                    continue;
                }
                counted += 1;
                match state.as_str() {
                    "FAILURE" | "ERROR" => failing = true,
                    "PENDING" | "EXPECTED" => pending = true,
                    _ => {}
                }
            }
            CheckContext::Unknown => {}
        }
    }
    if failing {
        CiState::Failing
    } else if pending {
        CiState::Pending
    } else if counted > 0 {
        CiState::Passing
    } else {
        CiState::None
    }
}

/// The `HYGIENE_CONTEXT` status, if the commit has one. GitHub reports only
/// the latest status per context, so there is at most one.
pub(crate) fn pick_hygiene(contexts: &[CheckContext]) -> Option<GhHygiene> {
    contexts.iter().find_map(|ctx| match ctx {
        CheckContext::StatusContext {
            context,
            state,
            description,
        } if context == HYGIENE_CONTEXT => Some(GhHygiene {
            state: state.to_ascii_uppercase(),
            description: description.clone().filter(|d| !d.trim().is_empty()),
        }),
        _ => None,
    })
}

/// The bot's check run; the most recently started one when there are several.
pub(crate) fn pick_bot_check(contexts: &[CheckContext]) -> Option<GhBotCheck> {
    let mut best: Option<GhBotCheck> = None;
    for ctx in contexts {
        let CheckContext::CheckRun {
            name,
            status,
            conclusion,
            title,
            started_at,
            completed_at,
        } = ctx
        else {
            continue;
        };
        if name != BOT_CHECK_NAME {
            continue;
        }
        let candidate = GhBotCheck {
            status: match status.as_str() {
                "IN_PROGRESS" => GhCheckStatus::InProgress,
                "COMPLETED" => GhCheckStatus::Completed,
                // QUEUED, PENDING, WAITING, REQUESTED, and anything new.
                _ => GhCheckStatus::Queued,
            },
            conclusion: conclusion.as_ref().map(|c| c.to_ascii_uppercase()),
            title: title.clone().filter(|t| !t.trim().is_empty()),
            started_at: *started_at,
            completed_at: *completed_at,
        };
        let newer = match &best {
            None => true,
            Some(b) => candidate.started_at >= b.started_at,
        };
        if newer {
            best = Some(candidate);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/tests/fixtures/github/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn ok<T: DeserializeOwned>(name: &str) -> Gql<T> {
        match interpret(200, RateHeaders::default(), &fixture(name), now()) {
            Ok(g) => g,
            Err(Failure::App(e)) => panic!("{name}: {e}"),
            Err(Failure::TooCostly(m)) => panic!("{name}: too costly: {m}"),
        }
    }

    fn app_err<T: DeserializeOwned + std::fmt::Debug>(
        status: u16,
        rate: RateHeaders,
        body: &str,
    ) -> AppError {
        match interpret::<T>(status, rate, body.as_bytes(), now()) {
            Err(Failure::App(e)) => e,
            other => panic!("expected an AppError, got {other:?}"),
        }
    }

    fn mine() -> MinePage {
        parse_mine_page(ok("mine.json"), "dashpay/platform").unwrap()
    }

    fn pr(page: &MinePage, number: u64) -> &GhPullRequest {
        page.prs
            .iter()
            .find(|p| p.number == number)
            .unwrap_or_else(|| panic!("PR {number} missing"))
    }

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    #[test]
    fn viewer() {
        let v = parse_viewer(ok::<ViewerData>("viewer.json").data);
        assert_eq!(v.login, "QuantumExplorer");
        assert!(v.avatar_url.unwrap().starts_with("https://"));
    }

    #[test]
    fn milestone_title_is_read_trimmed_and_optional() {
        let page = mine();
        assert_eq!(pr(&page, 6038).milestone.as_deref(), Some("v5.2.0"));
        assert_eq!(pr(&page, 6034).milestone.as_deref(), Some("v5.1.0"));
        // Surrounding spaces in a title are dropped.
        assert_eq!(pr(&page, 6022).milestone.as_deref(), Some("v5.1.0"));
        // An explicit null and a missing key both mean no milestone.
        assert_eq!(pr(&page, 6002).milestone, None);
        assert_eq!(pr(&page, 5967).milestone, None);
    }

    #[test]
    fn mine_page_basics() {
        let page = mine();
        assert_eq!(page.repo, "dashpay/platform");
        assert_eq!(page.open_count, 127);
        assert_eq!(page.next, None);
        let numbers: Vec<u64> = page.prs.iter().map(|p| p.number).collect();
        assert_eq!(numbers, vec![6038, 6034, 6022, 6002, 5967, 5113, 4776]);

        let p = pr(&page, 6038);
        assert_eq!(p.repo, "dashpay/platform");
        assert_eq!(p.author, "QuantumExplorer");
        assert_eq!(p.url, "https://github.com/dashpay/platform/pull/6038");
        assert_eq!(p.head_sha.len(), 40);
        assert!(p.head_sha.starts_with("ce87c4ba"));
        assert!(!p.is_draft);
        assert!(p.additions > 0);
        assert_eq!(p.review_decision, Some(ReviewDecision::ReviewRequired));
        assert!(p.created_at <= p.updated_at);
    }

    #[test]
    fn bot_check_in_progress_and_newest_wins() {
        // 6038 also carries an older, completed bot run that must be ignored.
        let p = pr(&mine(), 6038).clone();
        let bot = p.bot_check.unwrap();
        assert_eq!(bot.status, GhCheckStatus::InProgress);
        assert_eq!(bot.conclusion, None);
        assert_eq!(bot.title.as_deref(), Some("Reviewing — verifying findings"));
        assert!(bot.started_at.is_some());
        assert_eq!(bot.completed_at, None);
        // An in-progress run, a pending status context.
        assert_eq!(p.ci, CiState::Pending);
    }

    #[test]
    fn bot_check_completed_variants() {
        let page = mine();
        let cases = [
            (6002, "SUCCESS", "Approved"),
            (6034, "FAILURE", "Changes requested"),
            (6022, "NEUTRAL", "Reviewed — comments, no changes requested"),
            (5967, "CANCELLED", "Not reviewed: the PR is a draft"),
        ];
        for (number, conclusion, title) in cases {
            let bot = pr(&page, number).bot_check.clone().unwrap();
            assert_eq!(bot.status, GhCheckStatus::Completed, "{number}");
            assert_eq!(bot.conclusion.as_deref(), Some(conclusion), "{number}");
            assert_eq!(bot.title.as_deref(), Some(title), "{number}");
            assert!(bot.completed_at.is_some(), "{number}");
        }
        assert!(pr(&page, 5967).is_draft);
    }

    #[test]
    fn no_bot_check() {
        let page = mine();
        let p = pr(&page, 5113);
        assert_eq!(p.bot_check, None);
        assert!(p.is_draft);
        // A real CI failure.
        assert_eq!(p.ci, CiState::Failing);
    }

    #[test]
    fn bot_failure_alone_does_not_fail_ci() {
        // 6034: the only failing check is the bot's "Changes requested".
        assert_eq!(pr(&mine(), 6034).ci, CiState::Passing);
    }

    #[test]
    fn real_failure_beats_cancelled_and_pending() {
        // 6022: a failed E2E job and a cancelled job.
        assert_eq!(pr(&mine(), 6022).ci, CiState::Failing);
        // 4776: a status context (not the hygiene gate) in ERROR.
        assert_eq!(pr(&mine(), 4776).ci, CiState::Failing);
    }

    #[test]
    fn hygiene_gate_is_not_ci() {
        // 6002: every check passed; the only pending context is PR Hygiene.
        let page = mine();
        let p = pr(&page, 6002);
        assert_eq!(p.ci, CiState::Passing);
        assert_eq!(
            p.hygiene,
            Some(GhHygiene {
                state: "PENDING".into(),
                description: Some(
                    "ready-for-human: Ready for review - files with no dedicated owner: \
                     QuantumExplorer or shumkov | dpp: QuantumExplorer or shumkov"
                        .into()
                ),
            })
        );
    }

    #[test]
    fn hygiene_from_fixtures() {
        let page = mine();
        let stage = |n: u64| {
            pr(&page, n).hygiene.as_ref().map(|h| {
                let d = h.description.as_deref().unwrap_or_default();
                let code = d.split(':').next().unwrap_or_default();
                (h.state.clone(), code.to_owned())
            })
        };
        let s = |state: &str, stage: &str| Some((state.to_owned(), stage.to_owned()));
        assert_eq!(stage(6038), s("PENDING", "waiting-bots"));
        assert_eq!(stage(6034), s("PENDING", "waiting-self-review"));
        assert_eq!(stage(6022), s("PENDING", "waiting-self-review"));
        assert_eq!(stage(5967), s("PENDING", "draft"));
        assert_eq!(stage(4776), s("SUCCESS", "ready-to-merge"));
        // A repo or commit without the gate.
        assert_eq!(stage(5113), None);

        let followed = parse_followed(ok("followed.json"), "dashpay/platform")
            .unwrap()
            .prs;
        let p6041 = followed.iter().find(|p| p.number == 6041).unwrap();
        assert_eq!(p6041.ci, CiState::Passing);
        assert_eq!(
            p6041.hygiene.as_ref().map(|h| h.state.as_str()),
            Some("PENDING")
        );
        let p6036 = followed.iter().find(|p| p.number == 6036).unwrap();
        assert_eq!(p6036.hygiene, None);
    }

    #[test]
    fn head_branch_and_fork_flag_are_read_and_missing_means_not_a_fork() {
        let followed = parse_followed(ok("followed.json"), "dashpay/platform")
            .unwrap()
            .prs;
        let pr = |n: u64| followed.iter().find(|p| p.number == n).unwrap();
        assert_eq!(
            pr(6041).head_branch.as_deref(),
            Some("ci/hygiene-dashboard")
        );
        assert!(!pr(6041).head_from_fork);
        assert_eq!(pr(6036).head_branch.as_deref(), Some("v6.0-dev"));
        assert!(pr(6036).head_from_fork);

        // An older capture without either field.
        for p in &mine().prs {
            assert_eq!(p.head_branch, None, "PR {}", p.number);
            assert!(!p.head_from_fork, "PR {}", p.number);
        }

        // Null reads as not a fork.
        let mut v: Value = serde_json::from_slice(&fixture("followed.json")).unwrap();
        v["data"]["repository"]["pr6036"]["isCrossRepository"] = Value::Null;
        let body = serde_json::to_vec(&v).unwrap();
        let resp = interpret::<FollowedData>(200, RateHeaders::default(), &body, now()).unwrap();
        let followed = parse_followed(resp, "dashpay/platform").unwrap().prs;
        let p6036 = followed.iter().find(|p| p.number == 6036).unwrap();
        assert!(!p6036.head_from_fork);
    }

    #[test]
    fn hygiene_pick_tolerates_odd_values() {
        let got = pick_hygiene(&[
            status("SUCCESS"),
            hygiene("pending", Some("   ")),
            run(HYGIENE_CONTEXT, "COMPLETED", Some("FAILURE")),
        ]);
        assert_eq!(
            got,
            Some(GhHygiene {
                state: "PENDING".into(),
                description: None
            })
        );
        assert_eq!(pick_hygiene(&[status("PENDING")]), None);
        assert_eq!(
            pick_hygiene(&[hygiene("SUCCESS", Some("ready-to-merge"))]).and_then(|h| h.description),
            Some("ready-to-merge".into())
        );
    }

    #[test]
    fn reviews_and_requests() {
        let page = mine();
        let p = pr(&page, 6002);
        assert_eq!(
            p.reviews,
            vec![
                GhReview {
                    login: "shumkov".into(),
                    avatar_url: Some("https://avatars.githubusercontent.com/u/24296?v=4".into()),
                    state: GhReviewState::Approved,
                    submitted_at: ts("2026-11-20T10:00:00Z"),
                    is_bot: false,
                },
                GhReview {
                    login: "thepastaclaw".into(),
                    avatar_url: p.reviews[1].avatar_url.clone(),
                    state: GhReviewState::Approved,
                    submitted_at: p.reviews[1].submitted_at,
                    // The review bot is a plain User account.
                    is_bot: false,
                },
            ]
        );
        assert_eq!(
            p.review_requests,
            vec![GhReviewer {
                login: "dashpay/platform-reviewers".into(),
                avatar_url: None,
                is_bot: false,
            }]
        );

        let p = pr(&page, 6022);
        let states: Vec<_> = p
            .reviews
            .iter()
            .map(|r| (r.login.as_str(), r.state, r.is_bot))
            .collect();
        assert_eq!(
            states,
            vec![
                ("coderabbitai", GhReviewState::Commented, true),
                ("thepastaclaw", GhReviewState::Commented, false)
            ]
        );

        let p = pr(&page, 4776);
        let logins: Vec<_> = p
            .review_requests
            .iter()
            .map(|r| (r.login.as_str(), r.is_bot))
            .collect();
        assert_eq!(
            logins,
            vec![
                ("HashEngineering", false),
                ("llbartekll", false),
                ("coderabbitai", true)
            ]
        );
        assert!(p.review_requests[0].avatar_url.is_some());
    }

    #[test]
    fn followed_keeps_open_drops_merged_and_missing() {
        // The fixture has a NOT_FOUND error for one alias alongside data.
        let resp: Gql<FollowedData> = ok("followed.json");
        assert_eq!(resp.errors.len(), 1);
        let prs = parse_followed(resp, "dashpay/platform").unwrap().prs;
        let numbers: Vec<u64> = prs.iter().map(|p| p.number).collect();
        assert_eq!(numbers, vec![6036, 6041]);
        let draft = prs.iter().find(|p| p.number == 6036).unwrap();
        assert!(draft.is_draft);
        assert_eq!(draft.bot_check, None);
        let p6041 = prs.iter().find(|p| p.number == 6041).unwrap();
        assert_eq!(p6041.author, "shumkov");
        assert_eq!(
            p6041.bot_check.as_ref().unwrap().conclusion.as_deref(),
            Some("SUCCESS")
        );
    }

    #[test]
    fn merge_dedupes_by_number() {
        let page = mine();
        let mine_prs = page.prs.clone();
        let mut followed = vec![pr(&page, 6038).clone()];
        followed[0].title = "from the followed query".into();
        let extra = parse_followed(ok("followed.json"), "dashpay/platform")
            .unwrap()
            .prs;
        followed.extend(extra);
        let merged = merge_prs(mine_prs.clone(), followed);
        assert_eq!(merged.len(), mine_prs.len() + 2);
        // The viewer's copy wins.
        assert_ne!(
            merged.iter().find(|p| p.number == 6038).unwrap().title,
            "from the followed query"
        );
    }

    #[test]
    fn open_prs_page() {
        let page = parse_open_page(ok("open_prs.json"), "dashpay/platform").unwrap();
        assert_eq!(page.rows.len(), 4);
        assert!(page.next.is_some());
        let first = &page.rows[0].summary;
        assert_eq!(first.id, format!("dashpay/platform#{}", first.number));
        assert_eq!(first.repo, "dashpay/platform");
        assert!(!first.is_mine && !first.is_followed);
        assert!(
            page.rows
                .windows(2)
                .all(|w| w[0].summary.updated_at >= w[1].summary.updated_at)
        );
        // An older capture, without the author's type and avatar.
        let author = &page.rows[0].author;
        assert_eq!(author.login, first.author);
        assert_eq!((author.avatar_url.as_deref(), author.is_bot), (None, false));
    }

    #[test]
    fn open_prs_carry_their_authors() {
        let resp: Gql<OpenPrsData> = ok("open_prs_authors.json");
        assert_eq!(resp.data.cost(), Some(1));
        let page = parse_open_page(resp, "dashpay/dips").unwrap();
        assert_eq!(page.rows.len(), 15);
        assert_eq!(page.next, None);
        for row in &page.rows {
            assert_eq!(row.author.login, row.summary.author);
            assert!(!row.author.is_bot, "{}", row.author.login);
            assert!(
                row.author
                    .avatar_url
                    .as_deref()
                    .is_some_and(|u| u.starts_with("https://avatars.githubusercontent.com/")),
                "{}",
                row.author.login
            );
        }
        assert!(page.rows.iter().any(|r| r.author.login == "shumkov"));
    }

    #[test]
    fn open_pr_authors_mark_apps_and_deleted_accounts() {
        let body = br#"{"data":{"rateLimit":{"cost":1},"repository":{"nameWithOwner":"o/r","pullRequests":{
            "pageInfo":{"hasNextPage":false,"endCursor":null},
            "nodes":[
              {"number":1,"title":"a","url":"u1","isDraft":false,"updatedAt":"2026-11-20T10:00:00Z",
               "author":{"__typename":"Bot","login":"dependabot","avatarUrl":"https://avatars.test/d"}},
              {"number":2,"title":"b","url":"u2","isDraft":true,"updatedAt":"2026-11-19T10:00:00Z",
               "author":null},
              {"number":3,"title":"c","url":"u3","isDraft":false,"updatedAt":"2026-11-18T10:00:00Z",
               "author":{"__typename":"User","login":"Bob","avatarUrl":null}}
            ]}}}}"#;
        let resp = interpret::<OpenPrsData>(200, RateHeaders::default(), body, now()).unwrap();
        let page = parse_open_page(resp, "o/r").unwrap();
        let authors: Vec<_> = page
            .rows
            .iter()
            .map(|r| {
                (
                    r.author.login.as_str(),
                    r.author.is_bot,
                    r.author.avatar_url.is_some(),
                )
            })
            .collect();
        assert_eq!(
            authors,
            vec![
                ("dependabot", true, true),
                ("ghost", false, false),
                ("Bob", false, false)
            ]
        );
    }

    #[test]
    fn accounts_are_users_organizations_or_nothing() {
        let account = |name: &str| {
            parse_account(interpret::<AccountData>(
                200,
                RateHeaders::default(),
                &fixture(name),
                now(),
            ))
        };
        let user = account("account_user.json").unwrap().unwrap();
        assert_eq!(user.login, "shumkov");
        assert_eq!(user.kind, GhAccountKind::User);
        assert!(user.avatar_url.is_some());
        let org = account("account_org.json").unwrap().unwrap();
        assert_eq!(
            (org.login.as_str(), org.kind),
            ("dashpay", GhAccountKind::Organization)
        );
        assert_eq!(account("account_missing.json").unwrap(), None);
    }

    #[test]
    fn account_errors() {
        let parse = |body: &str| {
            parse_account(interpret::<AccountData>(
                200,
                RateHeaders::default(),
                body.as_bytes(),
                now(),
            ))
        };
        // A NOT_FOUND, with or without data, is "no such account".
        let not_found = r#"{"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a User with the login of 'x'."}]}"#;
        assert_eq!(parse(not_found).unwrap(), None);
        let null_with_not_found =
            r#"{"data":{"repositoryOwner":null},"errors":[{"type":"NOT_FOUND","message":"nope"}]}"#;
        assert_eq!(parse(null_with_not_found).unwrap(), None);
        // Anything else is an error, not a missing user.
        let other = r#"{"data":{"repositoryOwner":null},"errors":[{"type":"FORBIDDEN","message":"SAML enforcement"}]}"#;
        assert!(matches!(parse(other), Err(AppError::Other(m)) if m == "SAML enforcement"));
        let typed = r#"{"data":{"repositoryOwner":{"__typename":"Enterprise","login":"e","avatarUrl":null}}}"#;
        assert_eq!(parse(typed).unwrap().unwrap().kind, GhAccountKind::Other);
        let unauthorized = parse_account(interpret::<AccountData>(
            401,
            RateHeaders::default(),
            br#"{"message":"Bad credentials"}"#,
            now(),
        ));
        assert!(matches!(unauthorized, Err(AppError::Unauthorized(_))));
    }

    #[test]
    fn missing_repo_is_not_found() {
        let resp: Gql<RepoData> = ok("repo_missing.json");
        let err = parse_repo_name(resp, "dashpay/does-not-exist-xyz").unwrap_err();
        assert!(
            matches!(err, AppError::NotFound(ref m) if m.contains("dashpay/does-not-exist-xyz"))
        );
    }

    #[test]
    fn repo_name_is_canonical() {
        let resp = interpret::<RepoData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"repository":{"nameWithOwner":"dashpay/platform"}}}"#,
            now(),
        )
        .unwrap();
        assert_eq!(
            parse_repo_name(resp, "DASHPAY/Platform").unwrap(),
            "dashpay/platform"
        );
    }

    #[test]
    fn http_401_is_unauthorized() {
        let e = app_err::<ViewerData>(
            401,
            RateHeaders::default(),
            r#"{"message":"Bad credentials","documentation_url":"x"}"#,
        );
        assert!(matches!(e, AppError::Unauthorized(ref m) if m == "Bad credentials"));
    }

    #[test]
    fn http_403_with_headers_is_rate_limited() {
        let rate = RateHeaders {
            remaining: Some(0),
            reset: Some(1_795_000_000),
            retry_after: None,
        };
        let e = app_err::<ViewerData>(403, rate, r#"{"message":"API rate limit exceeded"}"#);
        match e {
            AppError::RateLimited { reset_at } => assert_eq!(reset_at, "2026-11-18T11:06:40Z"),
            other => panic!("{other:?}"),
        }
        let rate = RateHeaders {
            retry_after: Some(60),
            ..Default::default()
        };
        let e = app_err::<ViewerData>(429, rate, "");
        assert!(
            matches!(e, AppError::RateLimited { ref reset_at } if reset_at == "2026-11-22T12:01:00Z")
        );
    }

    #[test]
    fn http_403_without_rate_headers_is_unauthorized() {
        let e = app_err::<ViewerData>(
            403,
            RateHeaders {
                remaining: Some(4000),
                ..Default::default()
            },
            r#"{"message":"Resource protected by organization SAML enforcement."}"#,
        );
        assert!(matches!(e, AppError::Unauthorized(ref m) if m.contains("SAML")));
    }

    #[test]
    fn graphql_rate_limited() {
        let body = r#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#;
        let e = app_err::<ViewerData>(200, RateHeaders::default(), body);
        assert!(
            matches!(e, AppError::RateLimited { ref reset_at } if reset_at == "an unknown time")
        );
    }

    #[test]
    fn graphql_errors_without_data() {
        let body =
            r#"{"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a Repository"}]}"#;
        let e = app_err::<RepoData>(200, RateHeaders::default(), body);
        assert!(matches!(e, AppError::NotFound(_)));

        let body = r#"{"errors":[{"message":"Field 'x' doesn't exist on type 'Query'"},{"message":"second"}]}"#;
        let e = app_err::<RepoData>(200, RateHeaders::default(), body);
        assert!(matches!(e, AppError::Other(ref m) if m.contains("Field 'x'")));
    }

    #[test]
    fn cost_errors_are_retryable() {
        let body = r#"{"errors":[{"type":"MAX_NODE_LIMIT_EXCEEDED","message":"too many nodes"}]}"#;
        let r = interpret::<RepoData>(200, RateHeaders::default(), body.as_bytes(), now());
        assert!(matches!(r, Err(Failure::TooCostly(_))));
        let body = r#"{"data":null,"errors":[{"message":"Something went wrong while executing your query. This may be the result of a timeout"}]}"#;
        let r = interpret::<RepoData>(200, RateHeaders::default(), body.as_bytes(), now());
        assert!(matches!(r, Err(Failure::TooCostly(_))));
        let r = interpret::<RepoData>(502, RateHeaders::default(), b"<html>", now());
        assert!(matches!(r, Err(Failure::TooCostly(_))));
    }

    /// GitHub gave up after a 200: an empty body (seen as "EOF while parsing
    /// a value at line 1 column 0") or one cut off part way.
    #[test]
    fn an_empty_or_cut_off_200_is_retryable() {
        for body in ["", " \n", r#"{"data":{"repository":{"pullRequests":"#] {
            let r = interpret::<RepoData>(200, RateHeaders::default(), body.as_bytes(), now());
            assert!(matches!(r, Err(Failure::TooCostly(_))), "{body:?}");
        }
        // Not retried forever: a caller at its smallest page reports it plainly.
        let e = AppError::from(
            interpret::<RepoData>(200, RateHeaders::default(), b"", now()).unwrap_err(),
        );
        assert_eq!(
            e.to_string(),
            "GitHub could not answer the query: HTTP 200 with an empty body"
        );
    }

    #[test]
    fn other_http_errors_are_network() {
        let e = app_err::<RepoData>(503, RateHeaders::default(), "unavailable");
        assert!(matches!(e, AppError::Network(ref m) if m.contains("503")));
    }

    #[test]
    fn garbage_body_is_decode_error() {
        let e = app_err::<RepoData>(200, RateHeaders::default(), "<html>");
        assert!(matches!(e, AppError::Decode(_)));
    }

    fn run(name: &str, status: &str, conclusion: Option<&str>) -> CheckContext {
        CheckContext::CheckRun {
            name: name.into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_owned),
            title: None,
            started_at: None,
            completed_at: None,
        }
    }

    fn status(state: &str) -> CheckContext {
        CheckContext::StatusContext {
            context: "ci/legacy".into(),
            state: state.into(),
            description: None,
        }
    }

    fn hygiene(state: &str, description: Option<&str>) -> CheckContext {
        CheckContext::StatusContext {
            context: HYGIENE_CONTEXT.into(),
            state: state.into(),
            description: description.map(str::to_owned),
        }
    }

    #[test]
    fn ci_rules() {
        use CiState::{Failing, Passing, Pending};
        let none = CiState::None;
        let bot_fail = run(BOT_CHECK_NAME, "COMPLETED", Some("FAILURE"));
        let cases: Vec<(Vec<CheckContext>, CiState)> = vec![
            (vec![], none),
            (vec![bot_fail.clone()], none),
            (vec![CheckContext::Unknown], none),
            (
                vec![bot_fail.clone(), run("a", "COMPLETED", Some("SUCCESS"))],
                Passing,
            ),
            (
                vec![
                    run("a", "COMPLETED", Some("NEUTRAL")),
                    run("b", "COMPLETED", Some("SKIPPED")),
                    run("c", "COMPLETED", Some("CANCELLED")),
                    run("d", "COMPLETED", Some("STALE")),
                    status("SUCCESS"),
                ],
                Passing,
            ),
            (vec![run("a", "QUEUED", None)], Pending),
            (vec![run("a", "IN_PROGRESS", None)], Pending),
            (vec![run("a", "WAITING", None)], Pending),
            (vec![run("a", "COMPLETED", None)], Pending),
            (vec![status("PENDING")], Pending),
            (vec![status("EXPECTED")], Pending),
            (
                vec![run("a", "IN_PROGRESS", None), status("ERROR")],
                Failing,
            ),
            (vec![status("FAILURE")], Failing),
            (vec![run("a", "COMPLETED", Some("TIMED_OUT"))], Failing),
            (
                vec![run("a", "COMPLETED", Some("ACTION_REQUIRED"))],
                Failing,
            ),
            (
                vec![run("a", "COMPLETED", Some("STARTUP_FAILURE"))],
                Failing,
            ),
            (
                vec![
                    run("a", "IN_PROGRESS", None),
                    run("b", "COMPLETED", Some("FAILURE")),
                ],
                Failing,
            ),
            // The bot running does not make CI pending.
            (
                vec![run(BOT_CHECK_NAME, "IN_PROGRESS", None), status("SUCCESS")],
                Passing,
            ),
            // Nor does the hygiene gate, in any state.
            (
                vec![
                    hygiene("PENDING", Some("waiting-bots: x")),
                    status("SUCCESS"),
                ],
                Passing,
            ),
            (
                vec![
                    hygiene("FAILURE", None),
                    run("a", "COMPLETED", Some("SUCCESS")),
                ],
                Passing,
            ),
            (vec![hygiene("ERROR", None)], none),
            (vec![hygiene("PENDING", None), status("PENDING")], Pending),
        ];
        for (contexts, want) in cases {
            assert_eq!(compute_ci(&contexts), want, "{contexts:?}");
        }
    }

    #[test]
    fn bot_check_status_mapping() {
        for (status, want) in [
            ("QUEUED", GhCheckStatus::Queued),
            ("PENDING", GhCheckStatus::Queued),
            ("WAITING", GhCheckStatus::Queued),
            ("IN_PROGRESS", GhCheckStatus::InProgress),
            ("COMPLETED", GhCheckStatus::Completed),
        ] {
            let got = pick_bot_check(&[run(BOT_CHECK_NAME, status, None)]).unwrap();
            assert_eq!(got.status, want);
        }
        assert_eq!(pick_bot_check(&[run("other", "COMPLETED", None)]), None);
        let lower = pick_bot_check(&[run(BOT_CHECK_NAME, "COMPLETED", Some("success"))]).unwrap();
        assert_eq!(lower.conclusion.as_deref(), Some("SUCCESS"));
    }

    #[test]
    fn pr_node_tolerates_nulls() {
        // Deleted author, no rollup, no reviews, unknown reviewer type.
        let node: PrNode = serde_json::from_value(serde_json::json!({
            "number": 7, "title": "t", "url": "u", "state": "OPEN", "isDraft": false,
            "additions": 1, "deletions": 2,
            "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
            "headRefOid": "abc", "reviewDecision": null, "author": null,
            "id": "PR_7", "baseRefName": "main", "mergeable": null, "mergeStateStatus": null,
            "isMergeQueueEnabled": false,
            "reviewThreads": {"totalCount": 0, "pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": []},
            "latestReviews": {"nodes": [
                {"state": "PENDING", "submittedAt": null, "author": {"login": "me", "avatarUrl": null}},
                {"state": "DISMISSED", "submittedAt": null, "author": null}
            ]},
            "reviewRequests": {"nodes": [
                {"requestedReviewer": null},
                {"requestedReviewer": {"__typename": "Something"}}
            ]},
            "commits": {"nodes": [{"commit": {"oid": "abc", "statusCheckRollup": null}}]}
        }))
        .unwrap();
        let (p, threads) = pr_from_node(node, "o/r");
        assert_eq!(threads.after, None);
        assert_eq!(p.merge.mergeable, GhMergeable::Unknown);
        assert_eq!(p.merge.merge_state, "UNKNOWN");
        assert_eq!(p.author, "ghost");
        assert_eq!(p.author_avatar_url, None);
        assert_eq!(p.ci, CiState::None);
        assert_eq!(p.bot_check, None);
        assert_eq!(p.review_decision, None);
        assert_eq!(p.reviews.len(), 1);
        assert_eq!(p.reviews[0].login, "ghost");
        assert_eq!(p.reviews[0].state, GhReviewState::Dismissed);
        assert!(!p.reviews[0].is_bot);
        assert!(p.review_requests.is_empty());
        assert_eq!(p.hygiene, None);
    }

    // ---- Merge facts ----

    fn facts(page: &MinePage, number: u64) -> GhMergeFacts {
        pr(page, number).merge.clone()
    }

    #[test]
    fn repo_access_from_the_mine_query() {
        let page = mine();
        assert!(page.access.squash_merge_allowed);
        assert!(page.access.viewer_permission);
        // Repo-wide facts are applied by the client, not per node.
        assert!(!facts(&page, 6038).viewer_can_merge);
        let mut f = facts(&page, 6038);
        page.access.apply(&mut f);
        assert!(f.squash_allowed && f.viewer_can_merge);
    }

    #[test]
    fn viewer_permission_levels() {
        for (level, can) in [
            (r#""ADMIN""#, true),
            (r#""MAINTAIN""#, true),
            (r#""WRITE""#, true),
            (r#""TRIAGE""#, false),
            (r#""READ""#, false),
            ("null", false),
        ] {
            let json = format!(r#"{{"squashMergeAllowed": false, "viewerPermission": {level}}}"#);
            let access: RepoAccess = serde_json::from_str(&json).unwrap();
            assert_eq!(access.viewer_permission, can, "{level}");
            assert!(!access.squash_merge_allowed);
        }
    }

    #[test]
    fn merge_facts_from_the_mine_fixture() {
        let page = mine();
        let row = |n: u64| {
            let f = facts(&page, n);
            (
                f.merge_state,
                f.mergeable,
                f.review_threads,
                f.unresolved_threads,
            )
        };
        use GhMergeable::{Conflicting, Mergeable};
        // Real: 48 threads, all resolved, blocked by branch rules.
        assert_eq!(row(6038), ("BLOCKED".into(), Mergeable, 48, 0));
        // Real: 3 of 11 threads open.
        assert_eq!(row(6034), ("BLOCKED".into(), Mergeable, 11, 3));
        // Hand-edited to CLEAN with 5 of 25 open.
        assert_eq!(row(6022), ("CLEAN".into(), Mergeable, 25, 5));
        assert_eq!(row(6002), ("CLEAN".into(), Mergeable, 0, 0));
        // Real: conflicts.
        assert_eq!(row(4776), ("DIRTY".into(), Conflicting, 1, 0));

        let f = facts(&page, 6038);
        assert_eq!(f.base_branch, "v5.2-dev");
        assert!(f.node_id.starts_with("PR_"));
        assert!(!f.merge_queue);
    }

    #[test]
    fn a_long_thread_list_is_never_zero_before_every_page_is_seen() {
        // 5113: 130 threads, the first 100 all resolved, a next page pending.
        let page = mine();
        let f = facts(&page, 5113);
        assert_eq!(f.review_threads, 130);
        assert_eq!(f.unresolved_threads, 30);
        let pending: Vec<&PrThreads> = page.threads.iter().filter(|t| t.after.is_some()).collect();
        assert_eq!(pending.len(), 1);
        let more = pending[0];
        assert_eq!(more.number, 5113);
        assert_eq!(more.node_id, f.node_id);
        assert_eq!(
            more.count,
            ThreadCount {
                total: 130,
                resolved: 100,
                unresolved: 0
            }
        );
        assert_eq!(more.threads.len(), 100);
    }

    #[test]
    fn every_pr_lists_its_thread_ids_and_resolved_flags() {
        let page = mine();
        assert_eq!(page.threads.len(), page.prs.len());
        let t = page.threads.iter().find(|t| t.number == 6034).unwrap();
        assert_eq!(t.node_id, facts(&page, 6034).node_id);
        assert_eq!(t.threads.len(), 11);
        assert_eq!(t.after, None);
        assert_eq!(t.threads[0].id, "PRRT_fixture_6034_0");
        // 3 of 11 open, as in the merge facts.
        assert_eq!(t.threads.iter().filter(|t| !t.resolved).count(), 3);
        // The project query reports what it cost.
        assert_eq!(ok::<MineData>("mine.json").data.cost(), Some(3));
        let none = page.threads.iter().find(|t| t.number == 6002).unwrap();
        assert!(none.threads.is_empty());
        // No findings yet: they come from the follow-up query.
        assert!(page.prs.iter().all(|p| p.findings.is_empty()));
    }

    #[test]
    fn thread_pages_add_up() {
        // The real second page of 6038's threads (first 40 skipped).
        let mut more = PrThreads {
            number: 6038,
            node_id: "PR_x".into(),
            count: ThreadCount {
                total: 48,
                resolved: 38,
                unresolved: 2,
            },
            threads: vec![],
            after: Some("c".into()),
        };
        let resp = ok::<ThreadsData>("threads_page.json");
        assert_eq!(resp.data.cost(), Some(1));
        let next = parse_threads_page(resp, &mut more).unwrap();
        assert_eq!(next, None);
        assert_eq!(more.threads.len(), 8);
        assert_eq!(more.threads[0].id, "PRRT_fixture_6038_40");
        assert_eq!(
            more.count,
            ThreadCount {
                total: 48,
                resolved: 46,
                unresolved: 2
            }
        );
        assert_eq!(more.count.unresolved(), 2);
        assert_eq!(more.count.threads(), 48);

        let missing = interpret::<ThreadsData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"node":null}}"#,
            now(),
        )
        .unwrap();
        assert!(matches!(
            parse_threads_page(missing, &mut more),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn the_first_page_lists_its_threads() {
        let merge: MergeNode = serde_json::from_value(serde_json::json!({
            "id": "PR_x",
            "reviewThreads": {
                "totalCount": 3,
                "pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": [
                    {"id": "A", "isResolved": true},
                    {"id": "B", "isResolved": false},
                    // No id: counted, not listed.
                    {"isResolved": false}
                ]
            }
        }))
        .unwrap();
        let (facts, threads) = merge_facts(merge, 6034);
        assert_eq!((facts.review_threads, facts.unresolved_threads), (3, 2));
        assert_eq!(
            threads.threads,
            vec![
                ThreadRef {
                    id: "A".into(),
                    resolved: true
                },
                ThreadRef {
                    id: "B".into(),
                    resolved: false
                },
            ]
        );
        assert_eq!(threads.after, None);
        assert_eq!(threads.node_id, "PR_x");
    }

    #[test]
    fn hidden_or_null_threads_count_as_unresolved() {
        let page: ThreadConnection = serde_json::from_value(serde_json::json!({
            "totalCount": 4,
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": [{"isResolved": true}, null, {"isResolved": true}]
        }))
        .unwrap();
        let mut count = ThreadCount::default();
        assert_eq!(count.add_page(page, &mut vec![]), None);
        // One null node, one never returned.
        assert_eq!(count.unresolved(), 2);
        assert_eq!(count.threads(), 4);
    }

    #[test]
    fn followed_merge_facts() {
        let page = parse_followed(ok("followed.json"), "dashpay/platform").unwrap();
        assert!(page.threads.iter().all(|t| t.after.is_none()));
        assert_eq!(page.threads.len(), page.prs.len());
        let f = |n: u64| {
            page.prs
                .iter()
                .find(|p| p.number == n)
                .unwrap()
                .merge
                .clone()
        };
        // Hand-edited: behind its base, its one thread resolved.
        assert_eq!(f(6041).merge_state, "BEHIND");
        assert_eq!((f(6041).review_threads, f(6041).unresolved_threads), (1, 0));
        assert_eq!(f(6041).base_branch, "v6.0-dev");
        // Real: only non-required checks failing.
        assert_eq!(f(6036).merge_state, "UNSTABLE");
    }

    fn check(name: &str) -> (GhMergeCheck, PrThreads) {
        parse_merge_check(ok(name), "dashpay/platform", 5750).unwrap()
    }

    #[test]
    fn merge_check_real_response() {
        // Real: 21 threads, 17 unresolved, CLEAN.
        let (c, threads) = check("merge_check.json");
        assert_eq!(threads.after, None);
        assert!(c.is_open);
        assert!(!c.is_draft);
        assert_eq!(c.head_sha, "cff4236bac374011900aef76985bee6b48213e8e");
        assert_eq!(
            c.commit_title,
            "feat(platform)!: random token transfer and mint amounts (#5750)"
        );
        assert_eq!(c.facts.merge_state, "CLEAN");
        assert_eq!(c.facts.mergeable, GhMergeable::Mergeable);
        assert_eq!(
            (c.facts.review_threads, c.facts.unresolved_threads),
            (21, 17)
        );
        assert!(c.facts.squash_allowed && c.facts.viewer_can_merge);
        assert!(c.facts.node_id.starts_with("PR_"));
        assert_eq!(
            c.facts.base_branch,
            "claude/platform-tenderdash-randomness-149e4e"
        );
    }

    #[test]
    fn merge_check_variants() {
        let (c, _) = check("merge_check_unstable.json");
        assert_eq!(c.facts.merge_state, "UNSTABLE");
        assert_eq!(c.facts.unresolved_threads, 0);
        assert!(!c.facts.merge_queue);

        let (c, _) = check("merge_check_queue.json");
        assert!(c.facts.merge_queue);

        let (c, _) = check("merge_check_closed.json");
        assert!(!c.is_open);
        assert_eq!(c.facts.mergeable, GhMergeable::Unknown);
    }

    #[test]
    fn merge_check_missing_pr_or_repo() {
        let resp = interpret::<MergeCheckData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"repository":{"squashMergeAllowed":true,"viewerPermission":"ADMIN","pullRequest":null}},
                "errors":[{"type":"NOT_FOUND","message":"Could not resolve to a PullRequest"}]}"#,
            now(),
        )
        .unwrap();
        let e = parse_merge_check(resp, "o/r", 9).unwrap_err();
        assert!(
            matches!(e, AppError::NotFound(ref m) if m.contains("o/r#9")),
            "{e:?}"
        );

        let resp = interpret::<MergeCheckData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"repository":null}}"#,
            now(),
        )
        .unwrap();
        assert!(matches!(
            parse_merge_check(resp, "o/r", 9),
            Err(AppError::NotFound(_))
        ));
    }

    fn squash(name: &str) -> AppResult<()> {
        interpret_squash_merge(interpret(
            200,
            RateHeaders::default(),
            &fixture(name),
            now(),
        ))
    }

    #[test]
    fn squash_merge_success() {
        assert!(squash("squash_merge_ok.json").is_ok());
    }

    #[test]
    fn squash_merge_head_moved() {
        let e = squash("squash_merge_head_moved.json").unwrap_err();
        match e {
            AppError::InvalidInput(m) => {
                assert!(
                    m.starts_with("New commits were pushed since you looked."),
                    "{m}"
                );
                assert!(m.contains("Head branch was modified"), "{m}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn squash_merge_refusal_keeps_githubs_message() {
        let e = squash("squash_merge_refused.json").unwrap_err();
        assert!(
            matches!(e, AppError::Other(ref m)
                if m.starts_with("GitHub refused the merge: ") && m.contains("approving review")),
            "{e:?}"
        );
        // Refused without any data at all.
        let r = interpret::<SquashMergeData>(
            200,
            RateHeaders::default(),
            br#"{"errors":[{"type":"FORBIDDEN","message":"Viewer does not have permission"}]}"#,
            now(),
        );
        let e = interpret_squash_merge(r).unwrap_err();
        assert!(
            matches!(e, AppError::Other(ref m) if m.contains("Viewer does not have permission")),
            "{e:?}"
        );
        // Null payload, no message.
        let r = interpret::<SquashMergeData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"mergePullRequest":null}}"#,
            now(),
        );
        assert!(matches!(interpret_squash_merge(r), Err(AppError::Other(_))));
    }

    #[test]
    fn squash_merge_unclear_outcomes_say_to_check_github() {
        let r = interpret::<SquashMergeData>(502, RateHeaders::default(), b"<html>", now());
        let e = interpret_squash_merge(r).unwrap_err();
        assert!(
            e.to_string().contains("Check the pull request on GitHub"),
            "{e}"
        );
        let e = interpret_squash_merge(Err(Failure::App(AppError::Network("timed out".into()))))
            .unwrap_err();
        assert!(
            e.to_string().contains("check the pull request on GitHub"),
            "{e}"
        );
        // Auth errors pass through untouched.
        let r = interpret::<SquashMergeData>(
            401,
            RateHeaders::default(),
            br#"{"message":"Bad credentials"}"#,
            now(),
        );
        assert!(matches!(
            interpret_squash_merge(r),
            Err(AppError::Unauthorized(_))
        ));
    }

    // ---- Self-review ----

    fn self_review_fixture(name: &str, number: u64) -> GhSelfReviewCheck {
        parse_self_review_check(ok(name), "dashpay/platform", number).unwrap()
    }

    #[test]
    fn self_review_check_real_response() {
        // Real: the viewer's own PR, the gate asking for the self-review.
        let c = self_review_fixture("self_review_check_wanted.json", 6038);
        assert!(c.node_id.starts_with("PR_"), "{}", c.node_id);
        assert_eq!(c.url, "https://github.com/dashpay/platform/pull/6038");
        assert_eq!(c.head_sha, "ce87c4ba91c01d646c7fa966b385ec7e215d90af");
        assert!(c.is_open && !c.is_draft);
        assert_eq!(c.author, "QuantumExplorer");
        assert_eq!(c.viewer, "QuantumExplorer");
        assert!(c.assignees.is_empty());
        assert_eq!(
            c.hygiene,
            Some(GhHygiene {
                state: "PENDING".into(),
                description: Some(
                    "waiting-self-review: Bots are done - your move: post /self-reviewed.".into()
                ),
            })
        );
        assert_eq!(c.comments.len(), 26);
        assert!(c.comments.iter().any(|c| c.author_is_bot));
        assert!(c.comments.iter().any(|c| !c.author_is_bot));
        assert!(
            c.comments
                .windows(2)
                .all(|w| w[0].created_at <= w[1].created_at)
        );
    }

    #[test]
    fn self_review_check_variants() {
        let merged = self_review_fixture("self_review_check_merged.json", 6034);
        assert!(!merged.is_open);
        // A description without a stage code.
        assert_eq!(
            merged.hygiene.and_then(|h| h.description).as_deref(),
            Some("Evaluating current review policy")
        );

        let draft = self_review_fixture("self_review_check_draft.json", 5967);
        assert!(draft.is_draft && draft.is_open);

        let assigned = self_review_fixture("self_review_check_assigned.json", 5896);
        assert_eq!(assigned.assignees, vec!["HashEngineering".to_owned()]);
        assert!(
            assigned
                .comments
                .iter()
                .any(|c| c.body == "/self-reviewed\n")
        );

        // Someone else's PR, with the author's self-reviews on it.
        let theirs = self_review_fixture("self_review_check_not_holding.json", 5793);
        assert_eq!(theirs.author, "PastaPastaPasta");
        assert_eq!(theirs.viewer, "QuantumExplorer");
        let named: Vec<&str> = theirs
            .comments
            .iter()
            .filter(|c| c.body.starts_with("/self-reviewed "))
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(named.len(), 2);
    }

    #[test]
    fn real_checks_decide_as_expected() {
        use crate::merge::{
            SELF_REVIEW_CLOSED, SELF_REVIEW_DRAFT, SELF_REVIEW_NOT_HOLDING, self_review_check,
        };
        let decide = |name: &str, number: u64| {
            self_review_check(
                pr_id("dashpay/platform", number),
                &self_review_fixture(name, number),
            )
        };
        let wanted = decide("self_review_check_wanted.json", 6038);
        assert!(wanted.can_post, "{wanted:?}");
        assert_eq!(
            wanted.comment,
            "/self-reviewed ce87c4ba91c01d646c7fa966b385ec7e215d90af"
        );
        assert_eq!(
            wanted.gate.as_deref(),
            Some("Bots are done - your move: post /self-reviewed.")
        );
        assert_eq!(wanted.already_posted_at, None);
        assert_eq!(
            wanted.diff_url,
            "https://github.com/dashpay/platform/pull/6038/files"
        );

        let merged = decide("self_review_check_merged.json", 6034);
        assert_eq!(merged.reason.as_deref(), Some(SELF_REVIEW_CLOSED));
        assert_eq!(
            merged.gate.as_deref(),
            Some("Evaluating current review policy")
        );
        let draft = decide("self_review_check_draft.json", 5967);
        assert_eq!(draft.reason.as_deref(), Some(SELF_REVIEW_DRAFT));
        assert_eq!(draft.gate.as_deref(), Some("draft"));

        // The author's self-reviews are theirs: the viewer cannot post, and
        // has posted nothing.
        let theirs = decide("self_review_check_not_holding.json", 5793);
        assert_eq!(theirs.reason.as_deref(), Some(SELF_REVIEW_NOT_HOLDING));
        assert_eq!(theirs.already_posted_at, None);

        // The same PR seen by its author (viewer login changed in the
        // capture): their later self-review names the current head; the
        // earlier one named an older head.
        let own = decide("self_review_check_posted.json", 5793);
        assert!(own.can_post);
        assert_eq!(own.already_posted_at, ts("2026-11-09T17:25:23Z"));

        // Assigned to its author, not to the viewer: still not theirs.
        let assigned = decide("self_review_check_assigned.json", 5896);
        assert_eq!(assigned.reason.as_deref(), Some(SELF_REVIEW_NOT_HOLDING));
        assert_eq!(assigned.already_posted_at, None);
    }

    #[test]
    fn self_review_check_reads_edits_and_tolerates_nulls() {
        let resp = interpret::<SelfReviewCheckData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"viewer":{"login":"me"},"repository":{"pullRequest":{
                "id":"PR_x","number":7,"url":"https://github.com/o/r/pull/7","state":"OPEN",
                "isDraft":false,"headRefOid":"abc","author":null,
                "assignees":{"nodes":[null,{"login":"me"},{"login":""}]},
                "commits":{"nodes":[]},
                "comments":{"nodes":[null,
                  {"author":{"__typename":"User","login":"me"},"body":"/self-reviewed",
                   "createdAt":"2026-11-22T10:00:00Z","lastEditedAt":"2026-11-22T10:01:00Z"},
                  {"author":null,"body":"hi","createdAt":"2026-11-22T10:02:00Z","lastEditedAt":null}
                ]}}}}}"#,
            now(),
        )
        .unwrap();
        let c = parse_self_review_check(resp, "o/r", 7).unwrap();
        assert_eq!(c.author, "ghost");
        assert_eq!(c.assignees, vec!["me".to_owned()]);
        assert_eq!(c.hygiene, None);
        assert_eq!(c.comments.len(), 2);
        assert!(c.comments[0].edited);
        assert!(!c.comments[1].edited);
        assert_eq!(c.comments[1].author, "ghost");
    }

    #[test]
    fn self_review_check_missing_pr_or_repo() {
        let resp = interpret::<SelfReviewCheckData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"viewer":{"login":"me"},"repository":{"pullRequest":null}},
                "errors":[{"type":"NOT_FOUND","message":"Could not resolve to a PullRequest"}]}"#,
            now(),
        )
        .unwrap();
        let e = parse_self_review_check(resp, "o/r", 9).unwrap_err();
        assert!(
            matches!(e, AppError::NotFound(ref m) if m.contains("o/r#9")),
            "{e:?}"
        );
        let resp = interpret::<SelfReviewCheckData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"viewer":{"login":"me"},"repository":null}}"#,
            now(),
        )
        .unwrap();
        assert!(matches!(
            parse_self_review_check(resp, "o/r", 9),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn assignees_are_read_from_the_page_query() {
        let mut v: Value = serde_json::from_slice(&fixture("mine.json")).unwrap();
        let nodes = v["data"]["search"]["nodes"].as_array_mut().unwrap();
        nodes[0]["assignees"] =
            serde_json::json!({"nodes":[{"login":"alice"},null,{"login":"Bob"}]});
        let number = nodes[0]["number"].as_u64().unwrap();
        let body = serde_json::to_vec(&v).unwrap();
        let resp = interpret::<MineData>(200, RateHeaders::default(), &body, now()).unwrap();
        let page = parse_mine_page(resp, "dashpay/platform").unwrap();
        for p in &page.prs {
            if p.number == number {
                assert_eq!(p.assignees, vec!["alice".to_owned(), "Bob".to_owned()]);
            } else {
                // Older captures have no assignees at all.
                assert!(p.assignees.is_empty());
            }
        }
    }

    fn add_comment(name: &str) -> AppResult<()> {
        interpret_add_comment(interpret(
            200,
            RateHeaders::default(),
            &fixture(name),
            now(),
        ))
    }

    #[test]
    fn add_comment_success() {
        assert!(add_comment("add_comment_ok.json").is_ok());
    }

    #[test]
    fn add_comment_errors_are_failures() {
        let e = add_comment("add_comment_refused.json").unwrap_err();
        assert!(
            matches!(e, AppError::Other(ref m)
                if m.starts_with("GitHub did not post the comment: ")
                    && m.contains("Resource not accessible")),
            "{e:?}"
        );
        // Errors beside a posted comment still fail, and say to look.
        let r = interpret::<AddCommentData>(
            200,
            RateHeaders::default(),
            br#"{"data":{"addComment":{"commentEdge":{"node":{"id":"IC_1"}}}},
                "errors":[{"message":"Something odd"}]}"#,
            now(),
        );
        let e = interpret_add_comment(r).unwrap_err();
        assert!(e.to_string().contains("Something odd"), "{e}");
        assert!(
            e.to_string().contains("Check the pull request on GitHub"),
            "{e}"
        );
        // No errors, but no comment either.
        for body in [
            br#"{"data":{"addComment":null}}"#.as_slice(),
            br#"{"data":{"addComment":{"commentEdge":null}}}"#.as_slice(),
            br#"{"data":{"addComment":{"commentEdge":{"node":{"id":""}}}}}"#.as_slice(),
        ] {
            let r = interpret::<AddCommentData>(200, RateHeaders::default(), body, now());
            assert!(matches!(interpret_add_comment(r), Err(AppError::Other(_))));
        }
        // Refused without any data at all.
        let r = interpret::<AddCommentData>(
            200,
            RateHeaders::default(),
            br#"{"errors":[{"type":"FORBIDDEN","message":"Viewer cannot comment"}]}"#,
            now(),
        );
        let e = interpret_add_comment(r).unwrap_err();
        assert!(e.to_string().contains("Viewer cannot comment"), "{e}");
    }

    #[test]
    fn add_comment_unclear_outcomes_say_to_check_github() {
        let r = interpret::<AddCommentData>(502, RateHeaders::default(), b"<html>", now());
        let e = interpret_add_comment(r).unwrap_err();
        assert!(
            e.to_string().contains("Check the pull request on GitHub"),
            "{e}"
        );
        let e = interpret_add_comment(Err(Failure::App(AppError::Network("timed out".into()))))
            .unwrap_err();
        assert!(e.to_string().contains("may have been posted"), "{e}");
        let r = interpret::<AddCommentData>(
            401,
            RateHeaders::default(),
            br#"{"message":"Bad credentials"}"#,
            now(),
        );
        assert!(matches!(
            interpret_add_comment(r),
            Err(AppError::Unauthorized(_))
        ));
    }
}
