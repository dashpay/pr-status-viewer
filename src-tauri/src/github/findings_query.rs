//! The follow-up query that reads what each review thread says, and the
//! reviews that tell which threads belong to each reviewer's last round.
//!
//! The page queries ask only for each thread's id and resolved flag, which
//! is cheap. GitHub charges for what a query *could* return, so asking every
//! PR for `reviewThreads(first: 100) { comments(first: 1) { body } }` costs
//! as if every PR had 100 threads. This query instead asks only the PRs that
//! need it (see `finding_cache`), each for exactly as many threads as it
//! has, batched under aliases (`p0`, `p1`, ...). A PR with more than 100
//! threads is read in pages, as the page queries do. The first page of each
//! PR also reads the last `REVIEWS_READ` reviews of the review bot and of
//! CodeRabbit (`bot`, `rabbit`), with their bodies and the commit each was
//! submitted on (`commit { oid }`, an object, so it adds nothing to the
//! cost), unless the cache still holds them (`FindingsJob::with_known`);
//! `rounds` picks the last round of each from them.
//!
//! Along with the reviews, the first page also reads the PR's top-level
//! comments (`comments`), with their bodies, for its traits (`traits`).
//! They are paged `COMMENTS_FIRST` at a time, up to `MAX_COMMENT_PAGES`
//! pages (1,000 comments); past that the traits come from the comments
//! read. Only the workflow comments that carry a trait marker are kept
//! while paging. Comments are a top-level connection of the PR, so they
//! add one request per PR to GitHub's cost, not one per thread; they are
//! not counted in the thread budget, but their page shrinks with it.
//!
//! The PR's round number, how many review posts the bot has made on it
//! (`rounds::PostCount`), is counted from the same `bot` reviews: their
//! `totalCount` (free) tells whether the window holds every review, or else
//! whether the count read last time can be carried on (`rounds::
//! count_posts`). When neither works (a PR first read with more bot reviews
//! than the window, or one whose earlier count is too old), every review of
//! the bot is read once more, `posts`, a page at a time with only its id and
//! body; the posts are counted as the pages arrive and the bodies dropped.
//! Those pages count against the thread budget, a review for a thread.
//!
//! Everything here is pure: the client plans a batch, sends the query built
//! here, and hands the response back to `apply_response`.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::coderabbit::{self, parse_coderabbit};
use super::finding::parse_finding;
use super::finding_cache::{Known, Reading, reviewer_of};
use super::rounds::{PostCount, ReviewInfo, Reviews, ThreadInfo, count_posts, last_rounds};
use super::traits::{TraitComment, is_candidate, read_traits};
use super::types::GhFindingReviewer;
use crate::model::{BOT_LOGIN, PrTrait};

/// GitHub caps `first` at 100.
pub(crate) const MAX_FIRST: u32 = 100;
/// Thread slots asked for in one query, summed over its aliases. Halved
/// when GitHub refuses or times out on the query, down to `MIN_BUDGET`.
pub(crate) const BUDGET: u32 = 500;
pub(crate) const MIN_BUDGET: u32 = 5;
/// PRs asked for in one query.
pub(crate) const MAX_ALIASES: usize = 50;
/// Pages of threads, or of the bot's reviews, one PR may take before it is
/// given up on.
const MAX_PAGES: usize = 50;
/// Top-level comments asked for per page, at the full thread budget.
pub(crate) const COMMENTS_FIRST: u32 = 100;
/// Pages of comments read per PR; the traits come from those read.
pub(crate) const MAX_COMMENT_PAGES: usize = 10;
/// Latest reviews read per reviewer. A reviewer's replies in threads come
/// as reviews of their own (with an empty body), so this many must cover
/// the replies posted since its last round. On dashpay/platform and
/// dashpay/dash-evo-tool (70 PRs, November 2026) at most 3 replies
/// followed a last round. Reply reviews have no body, so the size of the
/// answer grows with the rounds read, not with the replies.
pub(crate) const REVIEWS_READ: u32 = 10;

/// One PR whose findings are being read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindingsJob {
    pub number: u64,
    pub node_id: String,
    /// Threads believed still unread; sizes the next page.
    pub remaining: u32,
    /// Cursor of the next page of threads.
    pub after: Option<String>,
    /// Threads are still to be read.
    threads_pending: bool,
    /// Responses folded in so far.
    pages: usize,
    thread_pages: usize,
    threads: Vec<ThreadInfo>,
    /// The latest reviews of each reviewer: from the first page, or given.
    reviews: Reviews,
    /// The reviews and traits were given, so they are not asked for.
    reviews_given: bool,
    /// Comments are still to be read.
    comments_pending: bool,
    /// Cursor of the next page of comments.
    comments_after: Option<String>,
    comment_pages: usize,
    /// The comments read so far that may carry a trait.
    trait_comments: Vec<TraitComment>,
    /// Given with the reviews.
    traits: Vec<PrTrait>,
    /// The bot's post count read last time, to count on from.
    posts_before: Option<PostCount>,
    /// The bot's post count, once known (or given with the reviews).
    posts: Option<PostCount>,
    /// Every review of the bot is being read to count its posts.
    posts_pending: bool,
    /// Cursor of the next page of the bot's reviews.
    posts_after: Option<String>,
    /// The bot's reviews believed still unread; sizes the next page.
    posts_remaining: u32,
    post_pages: usize,
    /// The count over the pages read so far.
    posts_read: PostCount,
    pub state: JobState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JobState {
    Pending,
    Done,
    Failed(String),
}

impl FindingsJob {
    /// A job that reads `threads` review threads, the reviews and the
    /// comments. `None` without a node id.
    pub fn new(number: u64, node_id: &str, threads: u32) -> Option<Self> {
        (!node_id.is_empty()).then(|| Self {
            number,
            node_id: node_id.to_owned(),
            remaining: threads,
            after: None,
            threads_pending: threads > 0,
            pages: 0,
            thread_pages: 0,
            threads: Vec::new(),
            reviews: Reviews::default(),
            reviews_given: false,
            comments_pending: true,
            comments_after: None,
            comment_pages: 0,
            trait_comments: Vec::new(),
            traits: Vec::new(),
            posts_before: None,
            posts: None,
            posts_pending: false,
            posts_after: None,
            posts_remaining: 0,
            post_pages: 0,
            posts_read: PostCount::default(),
            state: JobState::Pending,
        })
    }

    /// Counts the bot's posts on from `before`, the count read last time,
    /// when the reviews read allow it.
    #[must_use]
    pub fn with_posts_before(mut self, before: Option<PostCount>) -> Self {
        self.posts_before = before;
        self
    }

    /// Uses the reviews, traits and post count read earlier instead of
    /// asking for them (and for the comments).
    #[must_use]
    pub fn with_known(mut self, known: Known) -> Self {
        self.reviews = known.reviews;
        self.traits = known.traits;
        self.posts = Some(known.posts);
        self.reviews_given = true;
        self.comments_pending = false;
        self.settle_state();
        self
    }

    /// Whether the next page asks for the reviews.
    fn asks_reviews(&self) -> bool {
        self.pages == 0 && !self.reviews_given
    }

    /// Done once nothing is left to ask.
    fn settle_state(&mut self) {
        if self.state == JobState::Pending
            && !self.threads_pending
            && !self.comments_pending
            && !self.posts_pending
            && !self.asks_reviews()
        {
            self.state = JobState::Done;
        }
    }

    pub fn fail(&mut self, why: impl Into<String>) {
        if self.state == JobState::Pending {
            self.state = JobState::Failed(why.into());
        }
    }

    /// What came of the job: the last rounds and their findings once every
    /// page was read.
    pub fn into_result(self) -> (u64, Result<Reading, String>) {
        let result = match self.state {
            JobState::Done => Ok(Reading {
                found: last_rounds(
                    &self.reviews.review_bot,
                    &self.reviews.coderabbit,
                    &self.threads,
                ),
                reviews: self.reviews,
                reviews_fresh: !self.reviews_given,
                posts: self.posts,
                traits: if self.reviews_given {
                    self.traits
                } else {
                    read_traits(&self.trait_comments)
                },
            }),
            JobState::Failed(why) => Err(why),
            JobState::Pending => Err("not every page was read".to_owned()),
        };
        (self.number, result)
    }
}

/// One alias in a query: which job, and how many threads, comments and
/// reviews of the bot (to count its posts) to ask it for (0: none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    pub job: usize,
    pub first: u32,
    pub comments: u32,
    pub posts: u32,
}

/// Comments asked for per page under a thread `budget`: `COMMENTS_FIRST`
/// at the full budget, less as it is halved, down to `MIN_BUDGET`.
fn comments_first(budget: u32) -> u32 {
    (budget / (BUDGET / COMMENTS_FIRST)).clamp(MIN_BUDGET, COMMENTS_FIRST)
}

/// The next query: pending jobs in order, each asking for its remaining
/// threads and bot reviews to count (at most `MAX_FIRST` of each), while
/// the thread `budget` (shared by both) and `max_aliases` allow. A job that
/// does not fit waits for the next query, unless the query would otherwise
/// be empty; then it asks for what fits, threads first. A job with comments
/// to read asks for a page of them too; one with only comments left takes
/// no thread budget.
pub(crate) fn plan_batch(jobs: &[FindingsJob], budget: u32, max_aliases: usize) -> Vec<Slot> {
    let mut left = budget.max(1);
    let comments = comments_first(budget);
    let mut slots = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        if slots.len() >= max_aliases {
            break;
        }
        if job.state != JobState::Pending {
            continue;
        }
        let want = if job.threads_pending {
            job.remaining.clamp(1, MAX_FIRST)
        } else {
            0
        };
        let want_posts = if job.posts_pending {
            job.posts_remaining.clamp(1, MAX_FIRST)
        } else {
            0
        };
        let (first, posts) = if want + want_posts <= left {
            (want, want_posts)
        } else if slots.is_empty() {
            let first = want.min(left);
            (first, want_posts.min(left - first))
        } else {
            continue;
        };
        slots.push(Slot {
            job: i,
            first,
            comments: if job.comments_pending { comments } else { 0 },
            posts,
        });
        left -= first + posts;
    }
    slots
}

/// The query text and variables for `slots`. Node ids and cursors go in as
/// variables; only the page sizes are written into the text. A job's first
/// page also asks for the reviewers' latest reviews, unless it was given
/// them; later pages may ask for every review of the bot, to count posts.
pub(crate) fn build_query(jobs: &[FindingsJob], slots: &[Slot]) -> (String, Value) {
    let mut params = Vec::with_capacity(slots.len() * 2);
    let mut body = String::new();
    let mut vars = Map::new();
    for (k, slot) in slots.iter().enumerate() {
        let job = &jobs[slot.job];
        params.push(format!("$id{k}: ID!"));
        vars.insert(format!("id{k}"), json!(job.node_id));
        let reviews = if job.asks_reviews() {
            format!(
                " bot: reviews(last: {REVIEWS_READ}, author: \"{BOT_LOGIN}\") {{ totalCount nodes {{ databaseId submittedAt commit {{ oid }} body }} }} rabbit: reviews(last: {REVIEWS_READ}, author: \"{}\") {{ nodes {{ databaseId submittedAt commit {{ oid }} body }} }}",
                coderabbit::REVIEW_AUTHOR
            )
        } else {
            String::new()
        };
        // GitHub refuses a query that declares a variable it does not use.
        let threads = if slot.first > 0 {
            params.push(format!("$after{k}: String"));
            vars.insert(format!("after{k}"), json!(job.after));
            format!(
                " reviewThreads(first: {first}, after: $after{k}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ id comments(first: 1) {{ nodes {{ databaseId author {{ login }} pullRequestReview {{ databaseId }} body }} }} }} }}",
                first = slot.first
            )
        } else {
            String::new()
        };
        let comments = if slot.comments > 0 {
            params.push(format!("$comments{k}: String"));
            vars.insert(format!("comments{k}"), json!(job.comments_after));
            format!(
                " comments(first: {n}, after: $comments{k}) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ author {{ __typename login }} createdAt lastEditedAt body }} }}",
                n = slot.comments
            )
        } else {
            String::new()
        };
        let posts = if slot.posts > 0 {
            params.push(format!("$posts{k}: String"));
            vars.insert(format!("posts{k}"), json!(job.posts_after));
            format!(
                " posts: reviews(first: {n}, after: $posts{k}, author: \"{BOT_LOGIN}\") {{ pageInfo {{ hasNextPage endCursor }} nodes {{ databaseId body }} }}",
                n = slot.posts
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            body,
            "  p{k}: node(id: $id{k}) {{ ... on PullRequest {{ number{reviews}{threads}{comments}{posts} }} }}"
        );
    }
    let query = format!(
        "query Findings({}) {{\n  rateLimit {{ cost }}\n{body}}}\n",
        params.join(", ")
    );
    (query, Value::Object(vars))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindingsNode {
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    review_threads: Option<FindingsConnection>,
    #[serde(default)]
    bot: Option<ReviewConnection>,
    #[serde(default)]
    rabbit: Option<ReviewConnection>,
    #[serde(default)]
    comments: Option<IssueComments>,
    #[serde(default)]
    posts: Option<PostsConnection>,
}

/// A page of every review of the bot, read to count its posts.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostsConnection {
    page_info: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<PostNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostNode {
    #[serde(default)]
    database_id: Option<u64>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueComments {
    page_info: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<IssueComment>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueComment {
    #[serde(default)]
    author: Option<TypedAuthor>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    last_edited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TypedAuthor {
    #[serde(rename = "__typename", default)]
    typename: String,
    #[serde(default)]
    login: String,
}

impl IssueComment {
    fn into_trait_comment(self) -> TraitComment {
        let (author_type, author_login) = self
            .author
            .map(|a| (a.typename, a.login))
            .unwrap_or_default();
        TraitComment {
            author_type,
            author_login,
            created_at: self.created_at,
            last_edited_at: self.last_edited_at,
            body: self.body.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FindingsConnection {
    page_info: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<FindingsThread>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewConnection {
    /// Asked for the bot's reviews only.
    #[serde(default)]
    total_count: Option<u32>,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<ReviewNode>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewNode {
    #[serde(default)]
    database_id: Option<u64>,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
    /// Null when GitHub no longer has the commit.
    #[serde(default)]
    commit: Option<CommitRef>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommitRef {
    #[serde(default)]
    oid: Option<String>,
}

impl ReviewConnection {
    fn into_infos(self) -> Vec<ReviewInfo> {
        self.nodes
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some(ReviewInfo {
                    database_id: r.database_id?,
                    submitted_at: r.submitted_at,
                    commit: r.commit.and_then(|c| c.oid).filter(|oid| !oid.is_empty()),
                    body: r.body.unwrap_or_default(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct FindingsThread {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    comments: Option<Comments>,
}

#[derive(Debug, Deserialize)]
struct Comments {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<Comment>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comment {
    #[serde(default)]
    database_id: Option<u64>,
    author: Option<Author>,
    #[serde(default)]
    pull_request_review: Option<ReviewRef>,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct Author {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRef {
    #[serde(default)]
    database_id: Option<u64>,
}

impl FindingsThread {
    /// What the thread's first comment says, when the thread has an id.
    fn into_info(self) -> Option<ThreadInfo> {
        let id = self.id.filter(|id| !id.is_empty())?;
        let root = self
            .comments
            .and_then(|c| c.nodes.into_iter().next().flatten());
        let Some(root) = root else {
            return Some(ThreadInfo {
                id,
                root_id: None,
                root_review: None,
                author: None,
                verdict: None,
            });
        };
        let author = root.author.as_ref().and_then(|a| reviewer_of(&a.login));
        let verdict = match author {
            Some(GhFindingReviewer::ReviewBot) => parse_finding(&root.body),
            Some(GhFindingReviewer::CodeRabbit) => parse_coderabbit(&root.body),
            None => None,
        };
        Some(ThreadInfo {
            id,
            root_id: root.database_id,
            root_review: root.pull_request_review.and_then(|r| r.database_id),
            author,
            verdict,
        })
    }
}

/// Folds one response into the jobs in `slots`. An alias that is missing,
/// null, unreadable or about another PR fails its job only, as does a first
/// page without the reviews, or a page without the threads or comments it
/// asked for.
pub(crate) fn apply_response(jobs: &mut [FindingsJob], slots: &[Slot], data: &Map<String, Value>) {
    for (k, slot) in slots.iter().enumerate() {
        let Some(job) = jobs.get_mut(slot.job) else {
            continue;
        };
        let node = data
            .get(&format!("p{k}"))
            .filter(|v| !v.is_null())
            .and_then(|v| FindingsNode::deserialize(v).ok());
        let Some(node) = node else {
            job.fail("GitHub did not return its review threads");
            continue;
        };
        if node.number != Some(job.number) {
            job.fail("GitHub returned another pull request");
            continue;
        }
        if job.asks_reviews() {
            let (Some(bot), Some(rabbit)) = (node.bot, node.rabbit) else {
                job.fail("GitHub did not return its reviews");
                continue;
            };
            let total = bot.total_count;
            job.reviews = Reviews {
                review_bot: bot.into_infos(),
                coderabbit: rabbit.into_infos(),
            };
            // Without a total, nothing says the window holds every review.
            let counted =
                total.and_then(|t| count_posts(&job.reviews.review_bot, t, job.posts_before));
            match counted {
                Some(count) => job.posts = Some(count),
                None => {
                    job.posts_pending = true;
                    job.posts_remaining = total.unwrap_or(MAX_FIRST).max(1);
                }
            }
        }
        if slot.first > 0 {
            let Some(page) = node.review_threads else {
                job.fail("GitHub did not return its review threads");
                continue;
            };
            let got = u32::try_from(page.nodes.len()).unwrap_or(u32::MAX);
            job.threads.extend(
                page.nodes
                    .into_iter()
                    .flatten()
                    .filter_map(FindingsThread::into_info),
            );
            job.thread_pages += 1;
            match page.page_info.end_cursor {
                Some(cursor) if page.page_info.has_next_page => {
                    if job.thread_pages >= MAX_PAGES {
                        job.fail("it has too many review threads to read");
                        continue;
                    }
                    job.after = Some(cursor);
                    job.remaining = job.remaining.saturating_sub(got).max(1);
                }
                _ => job.threads_pending = false,
            }
        }
        if slot.comments > 0 {
            let Some(page) = node.comments else {
                job.fail("GitHub did not return its comments");
                continue;
            };
            job.trait_comments.extend(
                page.nodes
                    .into_iter()
                    .flatten()
                    .map(IssueComment::into_trait_comment)
                    .filter(is_candidate),
            );
            job.comment_pages += 1;
            match page.page_info.end_cursor {
                Some(cursor) if page.page_info.has_next_page => {
                    if job.comment_pages >= MAX_COMMENT_PAGES {
                        log::info!(
                            "#{}: more than {} pages of comments; reading traits from those read",
                            job.number,
                            MAX_COMMENT_PAGES
                        );
                        job.comments_pending = false;
                    } else {
                        job.comments_after = Some(cursor);
                    }
                }
                _ => job.comments_pending = false,
            }
        }
        if slot.posts > 0 {
            let Some(page) = node.posts else {
                job.fail("GitHub did not return the review bot's reviews");
                continue;
            };
            let got = u32::try_from(page.nodes.len()).unwrap_or(u32::MAX);
            for review in page.nodes.into_iter().flatten() {
                job.posts_read
                    .add(review.database_id, review.body.as_deref().unwrap_or(""));
            }
            job.post_pages += 1;
            match page.page_info.end_cursor {
                Some(cursor) if page.page_info.has_next_page => {
                    if job.post_pages >= MAX_PAGES {
                        job.fail("the review bot has too many reviews to count");
                        continue;
                    }
                    job.posts_after = Some(cursor);
                    job.posts_remaining = job.posts_remaining.saturating_sub(got).max(1);
                }
                _ => {
                    job.posts_pending = false;
                    job.posts = Some(job.posts_read);
                }
            }
        }
        job.pages += 1;
        job.settle_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::finding_cache::Found;
    use crate::github::rounds::by_reviewer;
    use crate::github::types::{GhFindingStatus::*, GhSeverity::*};
    use crate::merge::finding_counts;
    use crate::model::SeverityCount;
    use GhFindingReviewer::*;
    use pretty_assertions::assert_eq;

    fn job(number: u64, threads: u32) -> FindingsJob {
        FindingsJob::new(number, &format!("PR_{number}"), threads).unwrap()
    }

    /// A slot at the full budget: `first` threads and a page of comments.
    fn slot(job: usize, first: u32) -> Slot {
        Slot {
            job,
            first,
            comments: COMMENTS_FIRST,
            posts: 0,
        }
    }

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/github/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_slice(&bytes).unwrap()
    }

    const ROUND: &str = "<!-- thepastaclaw-review v1 -->\n## Review";

    /// A response page for alias `p{k}`: `ids` threads, the bot's finding on
    /// those listed in `bot`, all opened by its review 1, which is a round.
    /// `first` adds the reviews, as on a job's first page.
    fn page(number: u64, ids: &[&str], bot: &[&str], next: Option<&str>, first: bool) -> Value {
        let nodes: Vec<Value> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let (login, body) = if bot.contains(id) {
                    (
                        "thepastaclaw",
                        "<!-- thepastaclaw-review v1 finding=ab12 dedupe=1 -->\n**🔴 Blocking: x**",
                    )
                } else {
                    ("someone", "Looks good.")
                };
                json!({"id": id, "comments": {"nodes": [{
                    "databaseId": 1000 + i,
                    "author": {"login": login},
                    "pullRequestReview": {"databaseId": 1},
                    "body": body
                }]}})
            })
            .collect();
        let mut v = json!({
            "number": number,
            "reviewThreads": {
                "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
                "nodes": nodes
            }
        });
        if first {
            v["bot"] = json!({"totalCount": 1, "nodes": [
                {"databaseId": 1, "submittedAt": "2026-11-21T10:00:00Z", "commit": {"oid": "r1"}, "body": ROUND}
            ]});
            v["rabbit"] = json!({"nodes": []});
            v["comments"] = json!({
                "pageInfo": {"hasNextPage": false, "endCursor": null},
                "nodes": []
            });
        }
        v
    }

    fn data(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn count(
        total: u32,
        agreed: u32,
        not_agreed: u32,
        awaiting: u32,
        coderabbit: u32,
    ) -> SeverityCount {
        SeverityCount {
            total,
            agreed,
            not_agreed,
            awaiting,
            coderabbit,
        }
    }

    #[test]
    fn a_pr_without_threads_asks_its_reviews_and_comments_only() {
        assert_eq!(FindingsJob::new(1, "", 4), None);
        let jobs = vec![FindingsJob::new(1, "PR_1", 0).unwrap()];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(slots, vec![slot(0, 0)]);
        let (query, vars) = build_query(&jobs, &slots);
        assert!(!query.contains("reviewThreads"));
        assert!(!query.contains("$after0"));
        assert!(query.contains("bot: reviews("));
        assert!(query.contains("comments(first: 100, after: $comments0)"));
        assert!(query.contains("query Findings($id0: ID!, $comments0: String)"));
        assert_eq!(vars["comments0"], Value::Null);
        // Known reviews and traits, no threads: nothing to ask.
        let known = FindingsJob::new(1, "PR_1", 0)
            .unwrap()
            .with_known(Known::default());
        assert_eq!(known.state, JobState::Done);
        assert_eq!(plan_batch(&[known], BUDGET, MAX_ALIASES), vec![]);
    }

    #[test]
    fn each_pr_asks_for_its_own_thread_count() {
        let jobs = vec![job(1, 48), job(2, 3), job(3, 100)];
        assert_eq!(
            plan_batch(&jobs, BUDGET, MAX_ALIASES),
            vec![slot(0, 48), slot(1, 3), slot(2, 100)]
        );
        let (query, vars) = build_query(&jobs, &plan_batch(&jobs, BUDGET, MAX_ALIASES));
        assert!(query.contains("p0: node(id: $id0)"));
        assert!(query.contains("reviewThreads(first: 48, after: $after0)"));
        assert!(query.contains("reviewThreads(first: 3, after: $after1)"));
        assert!(query.contains("reviewThreads(first: 100, after: $after2)"));
        assert!(
            query.contains(
                "query Findings($id0: ID!, $after0: String, $comments0: String, $id1: ID!"
            )
        );
        assert!(query.contains(
            "comments(first: 100, after: $comments0) { pageInfo { hasNextPage endCursor } nodes { author { __typename login } createdAt lastEditedAt body } }"
        ));
        assert!(query.contains("rateLimit { cost }"));
        assert!(query.contains(
            r#"bot: reviews(last: 10, author: "thepastaclaw") { totalCount nodes { databaseId submittedAt commit { oid } body } }"#
        ));
        assert!(query.contains(
            r#"rabbit: reviews(last: 10, author: "coderabbitai[bot]") { nodes { databaseId submittedAt commit { oid } body } }"#
        ));
        assert!(query.contains(
            "comments(first: 1) { nodes { databaseId author { login } pullRequestReview { databaseId } body } }"
        ));
        assert_eq!(query.matches("bot: reviews").count(), 3);
        assert_eq!(vars["id1"], "PR_2");
        assert_eq!(vars["after1"], Value::Null);
    }

    #[test]
    fn batches_respect_the_budget_and_alias_cap() {
        let jobs = vec![job(1, 80), job(2, 60), job(3, 30), job(4, 10)];
        // 80 fits; 60 does not fit the 20 left and waits; 10 does fit.
        // The comment page shrinks with the budget.
        let at = |job, first, comments| Slot {
            job,
            first,
            comments,
            posts: 0,
        };
        assert_eq!(
            plan_batch(&jobs, 100, MAX_ALIASES),
            vec![at(0, 80, 20), at(3, 10, 20)]
        );
        assert_eq!(plan_batch(&jobs, BUDGET, 2), vec![slot(0, 80), slot(1, 60)]);
        // A budget smaller than the first job: it asks for what fits.
        assert_eq!(plan_batch(&jobs, 5, MAX_ALIASES), vec![at(0, 5, 5)]);
        // Finished and failed jobs are not asked again.
        let mut jobs = jobs;
        jobs[0].state = JobState::Done;
        jobs[1].fail("x");
        assert_eq!(
            plan_batch(&jobs, BUDGET, MAX_ALIASES),
            vec![slot(2, 30), slot(3, 10)]
        );
    }

    #[test]
    fn a_pr_with_130_threads_takes_two_pages() {
        let mut jobs = vec![job(5113, 130)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(slots, vec![slot(0, 100)]);
        let ids: Vec<String> = (0..100).map(|i| format!("T{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", page(5113, &ids, &["T7"], Some("CUR"), true))]),
        );
        assert_eq!(jobs[0].state, JobState::Pending);
        assert_eq!(jobs[0].after.as_deref(), Some("CUR"));
        assert_eq!(jobs[0].remaining, 30);

        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(
            slots,
            vec![Slot {
                job: 0,
                first: 30,
                comments: 0,
                posts: 0,
            }]
        );
        let (query, vars) = build_query(&jobs, &slots);
        assert_eq!(vars["after0"], "CUR");
        // The reviews were read with the first page.
        assert!(!query.contains("reviews("));
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", page(5113, &["U1", "U2"], &["U2"], None, false))]),
        );
        assert_eq!(plan_batch(&jobs, BUDGET, MAX_ALIASES), vec![]);
        let (number, result) = jobs.remove(0).into_result();
        assert_eq!(number, 5113);
        let found = result.unwrap().found;
        assert_eq!(found.findings.len(), 2);
        assert_eq!(found.findings["T7"], (ReviewBot, (Blocking, Stands)));
        assert_eq!(found.findings["U2"], (ReviewBot, (Blocking, Stands)));
    }

    #[test]
    fn more_threads_than_counted_keeps_paging() {
        // The count said 3; a thread was added since. The cursor rules.
        let mut jobs = vec![job(1, 3)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", page(1, &["A", "B", "C"], &[], Some("N"), true))]),
        );
        assert_eq!(jobs[0].state, JobState::Pending);
        assert_eq!(plan_batch(&jobs, BUDGET, MAX_ALIASES)[0].first, 1);
    }

    #[test]
    fn a_bad_alias_fails_only_its_own_pr() {
        let mut jobs = vec![job(1, 2), job(2, 2), job(3, 2), job(4, 2), job(5, 2)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[
                ("p0", page(1, &["A"], &["A"], None, true)),
                ("p1", Value::Null),
                // Another PR's answer under this alias is never used.
                ("p2", page(99, &["Z"], &["Z"], None, true)),
                // p3 missing.
                // A first page without the reviews.
                ("p4", page(5, &["Y"], &["Y"], None, false)),
            ]),
        );
        let results: Vec<_> = jobs.into_iter().map(FindingsJob::into_result).collect();
        assert!(
            results[0]
                .1
                .as_ref()
                .is_ok_and(|r| r.found.findings.contains_key("A"))
        );
        assert!(results[1].1.is_err());
        assert!(results[2].1.is_err());
        assert!(results[3].1.is_err());
        assert!(results[4].1.is_err());
    }

    #[test]
    fn given_reviews_are_not_asked_for_again() {
        let mut first = vec![job(1, 2)];
        let slots = plan_batch(&first, BUDGET, MAX_ALIASES);
        apply_response(
            &mut first,
            &slots,
            &data(&[("p0", page(1, &["A", "B"], &["A"], None, true))]),
        );
        let reading = first.remove(0).into_result().1.unwrap();
        assert!(reading.reviews_fresh);
        assert_eq!(reading.reviews.review_bot.len(), 1);
        assert_eq!(
            reading.found.rounds.review_bot_commit.as_deref(),
            Some("r1")
        );

        // Later, nothing changed: the threads are read again, not the reviews.
        let mut again = vec![job(1, 2).with_known(Known {
            reviews: reading.reviews.clone(),
            traits: reading.traits.clone(),
            posts: reading.posts.unwrap(),
        })];
        let slots = plan_batch(&again, BUDGET, MAX_ALIASES);
        let (query, _) = build_query(&again, &slots);
        assert!(!query.contains("reviews("));
        assert!(!query.contains("comments(first: 100"));
        // The bot has since resolved A.
        let mut p = page(1, &["A", "B"], &[], None, false);
        p["reviewThreads"]["nodes"][0]["comments"]["nodes"][0] = json!({
            "databaseId": 1000, "author": {"login": "thepastaclaw"},
            "pullRequestReview": {"databaseId": 1},
            "body": "<!-- thepastaclaw-review v1 finding=ab12 dedupe=1 -->\n<!-- thepastaclaw-thread-status v1 -->\n> ✅ **Resolved** at `abc`\n**🔴 Blocking: x**"
        });
        apply_response(&mut again, &slots, &data(&[("p0", p)]));
        let later = again.remove(0).into_result().1.unwrap();
        assert!(!later.reviews_fresh);
        assert_eq!(later.reviews, reading.reviews);
        assert_eq!(later.posts, reading.posts);
        assert_eq!(later.found.rounds, reading.found.rounds);
        assert_eq!(later.found.findings["A"], (ReviewBot, (Blocking, Resolved)));
    }

    #[test]
    fn real_threads_read_as_findings() {
        // findings_6034.json: the first 11 threads are the real threads of
        // dashpay/platform#6034 (bodies trimmed after the headline), then a
        // real coderabbitai thread, a real human-started one, and hand-made
        // ones (10 findings, then a thread with no comments). It has no
        // reviews, so this reads each thread on its own.
        let resp = fixture("findings_6034.json");
        assert_eq!(resp["data"]["rateLimit"]["cost"], 1);
        let nodes = resp["data"]["p0"]["reviewThreads"]["nodes"]
            .as_array()
            .unwrap();
        let read: Vec<Option<(GhFindingReviewer, (_, _))>> = nodes
            .iter()
            .map(|n| {
                let t: FindingsThread = serde_json::from_value(n.clone()).unwrap();
                let info = t.into_info().unwrap();
                info.author.zip(info.verdict)
            })
            .collect();
        let b = |v| Some((ReviewBot, v));
        assert_eq!(
            read,
            vec![
                b((Blocking, Resolved)),
                b((Blocking, Resolved)),
                b((Blocking, Disputed)),
                b((Blocking, Resolved)),
                b((Blocking, Stands)),
                b((Suggestion, Resolved)),
                b((Suggestion, Resolved)),
                b((Suggestion, Resolved)),
                b((Suggestion, Stands)),
                b((Suggestion, Stands)),
                b((Suggestion, Stands)),
                // CodeRabbit's "Minor", not addressed.
                Some((CodeRabbit, (Suggestion, Stands))),
                // A person.
                None,
                b((Suggestion, Withdrawn)),
                b((Blocking, NoLongerApplies)),
                b((Blocking, Deferred)),
                // Needs a tracking issue: still stands.
                b((Blocking, Stands)),
                b((Nitpick, Stands)),
                // Unknown severity word.
                b((Nitpick, Stands)),
                // `root=` in the marker.
                b((Suggestion, Resolved)),
                // "**Resolved**" quoted in the body, no status block.
                b((Blocking, Stands)),
                // Windows line endings.
                b((Suggestion, Disputed)),
                // No comments: not a finding.
                None,
            ]
        );
    }

    /// Reads a one-PR findings response saved from GitHub (before comments
    /// and the bot's `totalCount` were asked for; an empty page of comments
    /// is added, and the bot's reviews read are taken as all it has).
    fn read_fixture(name: &str, number: u64) -> Found {
        let mut resp = fixture(name);
        resp["data"]["p0"]["comments"] = json!({
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": []
        });
        let bot = &mut resp["data"]["p0"]["bot"];
        if bot.get("totalCount").is_none() {
            bot["totalCount"] = json!(bot["nodes"].as_array().unwrap().len());
        }
        let data = resp["data"].as_object().unwrap();
        let threads = data["p0"]["reviewThreads"]["nodes"]
            .as_array()
            .unwrap()
            .len();
        let mut jobs = vec![job(number, u32::try_from(threads).unwrap())];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(&mut jobs, &slots, data);
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert!(reading.reviews_fresh);
        reading.found
    }

    /// The findings with the fixture's resolved flags.
    fn findings_of(name: &str, found: &Found) -> Vec<crate::github::types::GhFinding> {
        let resp = fixture(name);
        let threads: Vec<crate::github::parse::ThreadRef> =
            resp["data"]["p0"]["reviewThreads"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| crate::github::parse::ThreadRef {
                    id: n["id"].as_str().unwrap().to_owned(),
                    resolved: n["isResolved"].as_bool().unwrap(),
                })
                .collect();
        crate::github::finding_cache::combine(&threads, &found.findings)
    }

    #[test]
    fn the_last_round_of_platform_6034() {
        // rounds_6034.json: the real answer for dashpay/platform#6034
        // (review and comment bodies trimmed, `isResolved` added from the
        // page query). Three bot rounds; the last, 2026-11-21T16:25:22Z,
        // opened 3 suggestion threads and carries 2 blockers from the
        // first: discussion_r4651948881 (still open, resolved on GitHub)
        // and discussion_r4651948872 (disputed).
        let found = read_fixture("rounds_6034.json", 6034);
        assert_eq!(
            found.rounds.review_bot,
            Some(crate::merge::tests::at("2026-11-21T16:25:22Z"))
        );
        assert_eq!(found.rounds.coderabbit, None);
        assert_eq!(
            by_reviewer(&found)[&ReviewBot],
            vec![
                "PRRT_kwDOGUlHz866VAnf",
                "PRRT_kwDOGUlHz866VAnm",
                "PRRT_kwDOGUlHz866ZJE4",
                "PRRT_kwDOGUlHz866ZJE9",
                "PRRT_kwDOGUlHz866ZJFB",
            ]
        );
        // The fixture predates the reviews' commits: the round's commit is
        // unknown, so the blocker resolved by hand awaits a re-review.
        assert_eq!(found.rounds.review_bot_commit, None);
        let findings = findings_of("rounds_6034.json", &found);
        let counts = finding_counts(&findings, &found.rounds, "head");
        assert_eq!(counts.blocking, count(2, 0, 1, 1, 0));
        assert_eq!(counts.suggestion, count(3, 0, 0, 0, 0));
        assert_eq!(counts.nitpick, count(0, 0, 0, 0, 0));
    }

    #[test]
    fn the_last_rounds_of_evo_tool_1161() {
        // rounds_1161.json: the real answer for dashpay/dash-evo-tool#1161
        // (bodies trimmed). CodeRabbit's rounds were at 21:17:11 (7
        // threads), 16:22:12 (2) and 18:48:18 (1 Major, resolved by a
        // person without "Addressed": not agreed), each followed by reply
        // reviews. The bot's last round, 19:32:04, opened 2 blockers and
        // links 3 from earlier rounds (2 "still open", 1 "awaiting tracking
        // link"); all five stand while resolved on GitHub. Its finding "not
        // shown inline" has no thread and is not counted.
        let found = read_fixture("rounds_1161.json", 1161);
        assert_eq!(
            found.rounds.coderabbit,
            Some(crate::merge::tests::at("2026-11-21T18:48:18Z"))
        );
        assert_eq!(
            found.rounds.review_bot,
            Some(crate::merge::tests::at("2026-11-21T19:32:04Z"))
        );
        let by = by_reviewer(&found);
        assert_eq!(by[&CodeRabbit], vec!["PRRT_kwDOM8GK3c66a5LK"]);
        assert_eq!(
            by[&ReviewBot],
            vec![
                "PRRT_kwDOM8GK3c66LbK0",
                "PRRT_kwDOM8GK3c66LbKm",
                "PRRT_kwDOM8GK3c66ZkdJ",
                "PRRT_kwDOM8GK3c66bZ9O",
                "PRRT_kwDOM8GK3c66bZ9S",
            ]
        );
        // No commits in this fixture either: all six await a re-review.
        let findings = findings_of("rounds_1161.json", &found);
        let counts = finding_counts(&findings, &found.rounds, "head");
        assert_eq!(counts.blocking, count(6, 0, 0, 6, 1));
        assert_eq!(counts.suggestion, count(0, 0, 0, 0, 0));
        assert_eq!(counts.nitpick, count(0, 0, 0, 0, 0));
        // Had both last rounds been on the head, they are not agreed.
        let mut on_head = found.rounds.clone();
        on_head.review_bot_commit = Some("head".into());
        on_head.coderabbit_commit = Some("head".into());
        let counts = finding_counts(&findings, &on_head, "head");
        assert_eq!(counts.blocking, count(6, 0, 6, 0, 1));
    }

    #[test]
    fn platform_6056_awaits_the_re_review_of_the_pushed_fix() {
        // rounds_6056.json: the real answer for dashpay/platform#6056 at
        // 12:2x UTC on 2026-11-22 (bodies trimmed; `isResolved` and
        // `headRefOid` added from the page query). The bot's round at
        // 11:41:38 on 9363df29 opened 5 blocking and 5 suggestion threads.
        // The author then pushed 4dd9ab0c, replied "Fixed in ..." on all
        // ten and resolved them; the bot had not re-reviewed yet.
        let found = read_fixture("rounds_6056.json", 6056);
        let resp = fixture("rounds_6056.json");
        let head = resp["data"]["p0"]["headRefOid"].as_str().unwrap();
        assert_eq!(head, "4dd9ab0c3051534db0feffa32d1d8bde98413d7a");
        assert_eq!(
            found.rounds.review_bot,
            Some(crate::merge::tests::at("2026-11-22T11:41:38Z"))
        );
        assert_eq!(
            found.rounds.review_bot_commit.as_deref(),
            Some("9363df29b4fcc7cadda0b27b418cc71e201b69f6")
        );
        assert_eq!(found.rounds.coderabbit, None);
        assert_eq!(by_reviewer(&found)[&ReviewBot].len(), 10);

        let findings = findings_of("rounds_6056.json", &found);
        assert!(findings.iter().all(|f| f.thread_resolved));
        let counts = finding_counts(&findings, &found.rounds, head);
        assert_eq!(counts.blocking, count(5, 0, 0, 5, 0));
        assert_eq!(counts.suggestion, count(5, 0, 0, 5, 0));
        assert_eq!(counts.nitpick, count(0, 0, 0, 0, 0));

        // The same threads, had the head still been the round's commit:
        // the bot saw that code and did not lift them.
        let counts = finding_counts(
            &findings,
            &found.rounds,
            "9363df29b4fcc7cadda0b27b418cc71e201b69f6",
        );
        assert_eq!(counts.blocking, count(5, 0, 5, 0, 0));
        assert_eq!(counts.suggestion, count(5, 0, 5, 0, 0));
    }

    #[test]
    fn a_review_commit_may_be_null_or_missing() {
        let reviews: ReviewConnection = serde_json::from_value(json!({"nodes": [
            {"databaseId": 1, "submittedAt": "2026-11-21T10:00:00Z", "commit": null, "body": ROUND},
            {"databaseId": 2, "submittedAt": "2026-11-21T11:00:00Z", "body": ROUND},
            {"databaseId": 3, "submittedAt": "2026-11-21T12:00:00Z", "commit": {"oid": null}, "body": ROUND},
            {"databaseId": 4, "submittedAt": "2026-11-21T13:00:00Z", "commit": {}, "body": ROUND},
            {"databaseId": 5, "submittedAt": "2026-11-21T14:00:00Z", "commit": {"oid": ""}, "body": ROUND},
            {"databaseId": 6, "submittedAt": "2026-11-21T15:00:00Z", "commit": {"oid": "abc"}, "body": ROUND},
            null
        ]}))
        .unwrap();
        let commits: Vec<(u64, Option<String>)> = reviews
            .into_infos()
            .into_iter()
            .map(|r| (r.database_id, r.commit))
            .collect();
        assert_eq!(
            commits,
            vec![
                (1, None),
                (2, None),
                (3, None),
                (4, None),
                (5, None),
                (6, Some("abc".into())),
            ]
        );

        // A whole first page with a null commit reads, and its round has
        // no commit.
        let mut jobs = vec![job(1, 1)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        let mut p = page(1, &["A"], &["A"], None, true);
        p["bot"]["nodes"][0]["commit"] = Value::Null;
        apply_response(&mut jobs, &slots, &data(&[("p0", p)]));
        let found = jobs.remove(0).into_result().1.unwrap().found;
        assert!(found.rounds.review_bot.is_some());
        assert_eq!(found.rounds.review_bot_commit, None);
    }

    #[test]
    fn only_the_two_reviewers_start_findings() {
        let bot =
            "<!-- thepastaclaw-review v1 finding=b75cf39c3cc4 dedupe=e3 -->\n**🔴 Blocking: x**";
        let rabbit = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture></picture> Major** · **x**\n<!-- cr-indicator-types:potential_issue -->";
        let thread = |login: Option<&str>, body: &str| -> Option<(GhFindingReviewer, (_, _))> {
            let t: FindingsThread = serde_json::from_value(json!({
                "id": "T",
                "comments": {"nodes": [{
                    "databaseId": 5,
                    "author": login.map(|l| json!({"login": l})),
                    "pullRequestReview": {"databaseId": 9},
                    "body": body
                }]}
            }))
            .unwrap();
            let info = t.into_info().unwrap();
            assert_eq!((info.root_id, info.root_review), (Some(5), Some(9)));
            info.author.zip(info.verdict)
        };
        assert_eq!(
            thread(Some("thepastaclaw"), bot),
            Some((ReviewBot, (Blocking, Stands)))
        );
        assert!(thread(Some("ThePastaClaw"), bot).is_some());
        assert_eq!(
            thread(Some("coderabbitai"), rabbit),
            Some((CodeRabbit, (Blocking, Stands)))
        );
        // Each reviewer's format counts only from that reviewer.
        assert_eq!(thread(Some("coderabbitai"), bot), None);
        assert_eq!(thread(Some("thepastaclaw"), rabbit), None);
        assert_eq!(thread(Some("someone"), bot), None);
        assert_eq!(thread(Some("someone"), rabbit), None);
        // A deleted account.
        assert_eq!(thread(None, bot), None);
        let bare: FindingsThread = serde_json::from_value(json!({"id": "T"})).unwrap();
        assert_eq!(bare.into_info().unwrap().verdict, None);
        let no_id: FindingsThread = serde_json::from_value(json!({"id": ""})).unwrap();
        assert_eq!(no_id.into_info(), None);
    }

    /// A comments page for alias `p{k}`: `bodies` by the workflow bot.
    fn comments_page(number: u64, bodies: &[(&str, &str)], next: Option<&str>) -> Value {
        let nodes: Vec<Value> = bodies
            .iter()
            .map(|(created, body)| {
                json!({
                    "author": {"__typename": "Bot", "login": "github-actions"},
                    "createdAt": created,
                    "lastEditedAt": null,
                    "body": body
                })
            })
            .collect();
        json!({
            "number": number,
            "comments": {
                "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
                "nodes": nodes
            }
        })
    }

    const GROVE: &str = "<!-- grovedb-structure-preview -->\nThis pull request changes the described GroveDB structure. **[Open it in the structure viewer](https://dashpay.github.io/grovedb-structure-viewer/?head=x)**\n\n**Added** (2 nodes)\n";
    const STATE: &str = "<!-- platform-pr-review-state-v1 {} -->";

    #[test]
    fn real_comments_of_platform_5526_give_both_traits() {
        // traits_5526.json: the 62 real comments of dashpay/platform#5526
        // (bodies other than the two trait comments cut to their first
        // line), the trait comments sitting among 57 review-state comments
        // of the same bot.
        let mut resp = fixture("traits_5526.json");
        // Saved before the bot's `totalCount` was asked for; it has none.
        resp["data"]["p0"]["bot"]["totalCount"] = json!(0);
        let data = resp["data"].as_object().unwrap();
        let mut jobs = vec![job(5526, 0)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(&mut jobs, &slots, data);
        assert_eq!(
            jobs[0].trait_comments.len(),
            2,
            "only marked workflow comments are kept"
        );
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert_eq!(
            reading
                .traits
                .iter()
                .map(|t| (t.kind, t.summary.as_str(), t.links.len()))
                .collect::<Vec<_>>(),
            vec![
                (
                    crate::model::TraitKind::GrovedbChange,
                    "5 added, 2 changed",
                    1
                ),
                (
                    crate::model::TraitKind::ContractChange,
                    "token-market-contract v1: new contract",
                    1
                ),
            ]
        );
    }

    #[test]
    fn comments_are_paged_after_the_threads_are_done() {
        // Two threads, read on the first page; the comments take three
        // pages, and the newer GroveDB comment is on the last.
        let mut jobs = vec![job(7, 2)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(slots, vec![slot(0, 2)]);
        let mut first = page(7, &["A", "B"], &["A"], None, true);
        first["comments"] =
            comments_page(7, &[("2026-11-01T00:00:00Z", STATE)], Some("C1"))["comments"].clone();
        apply_response(&mut jobs, &slots, &data(&[("p0", first)]));
        assert_eq!(jobs[0].state, JobState::Pending);

        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(
            slots,
            vec![Slot {
                job: 0,
                first: 0,
                comments: COMMENTS_FIRST,
                posts: 0,
            }]
        );
        let (query, vars) = build_query(&jobs, &slots);
        assert!(!query.contains("reviewThreads"));
        assert!(!query.contains("reviews("));
        assert_eq!(vars["comments0"], "C1");
        let changed = GROVE.replace("**Added** (2 nodes)", "**Changed** (9 nodes)");
        apply_response(
            &mut jobs,
            &slots,
            &data(&[(
                "p0",
                comments_page(7, &[("2026-11-02T00:00:00Z", &changed)], Some("C2")),
            )]),
        );
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[(
                "p0",
                comments_page(
                    7,
                    &[
                        ("2026-11-03T00:00:00Z", GROVE),
                        ("2026-11-04T00:00:00Z", STATE),
                    ],
                    None,
                ),
            )]),
        );
        assert_eq!(jobs[0].state, JobState::Done);
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert_eq!(reading.found.findings.len(), 1);
        assert_eq!(reading.traits.len(), 1);
        assert_eq!(reading.traits[0].summary, "2 nodes added");
    }

    #[test]
    fn comments_stop_after_the_page_cap() {
        let mut jobs = vec![job(8, 0)];
        let mut asked = 0;
        loop {
            let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
            if slots.is_empty() {
                break;
            }
            asked += 1;
            assert!(asked <= MAX_COMMENT_PAGES, "must stop at the cap");
            let mut p = comments_page(
                8,
                &[(
                    "2026-11-01T00:00:00Z",
                    if asked == 1 { GROVE } else { STATE },
                )],
                Some("more"),
            );
            if asked == 1 {
                p["bot"] = json!({"totalCount": 0, "nodes": []});
                p["rabbit"] = json!({"nodes": []});
            }
            apply_response(&mut jobs, &slots, &data(&[("p0", p)]));
        }
        assert_eq!(asked, MAX_COMMENT_PAGES);
        let reading = jobs.remove(0).into_result().1.unwrap();
        // The traits come from what was read.
        assert_eq!(reading.traits.len(), 1);
    }

    #[test]
    fn a_page_without_the_comments_it_asked_for_fails_the_pr() {
        let mut jobs = vec![job(9, 1), job(10, 1)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        let mut missing = page(9, &["A"], &[], None, true);
        missing.as_object_mut().unwrap().remove("comments");
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", missing), ("p1", page(10, &["B"], &[], None, true))]),
        );
        assert!(matches!(jobs[0].state, JobState::Failed(_)));
        assert_eq!(jobs[1].state, JobState::Done);
        assert_eq!(jobs.remove(1).into_result().1.unwrap().traits, vec![]);
    }

    /// The real reviews of the review bot on dashpay/platform#5749
    /// (posts_5749.json: bodies cut to their first two lines, replies
    /// empty). 53 reviews: 30 posts, 2 "CI failed after approval"
    /// follow-ups, 21 replies.
    fn reviews_5749() -> Vec<Value> {
        let resp = fixture("posts_5749.json");
        let reviews = &resp["data"]["repository"]["pullRequest"]["reviews"];
        assert_eq!(reviews["totalCount"], 53);
        reviews["nodes"].as_array().unwrap().clone()
    }

    /// A first page with no threads: the bot's `window` of reviews and its
    /// `total`, no CodeRabbit reviews, no comments.
    fn reviews_page(number: u64, total: u64, window: &[Value]) -> Value {
        json!({
            "number": number,
            "bot": {"totalCount": total, "nodes": window},
            "rabbit": {"nodes": []},
            "comments": {"pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": []}
        })
    }

    /// A page of every review of the bot, as `posts` reads it.
    fn posts_page(number: u64, reviews: &[Value], next: Option<&str>) -> Value {
        let nodes: Vec<Value> = reviews
            .iter()
            .map(|r| json!({"databaseId": r["databaseId"], "body": r["body"]}))
            .collect();
        json!({
            "number": number,
            "posts": {
                "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
                "nodes": nodes
            }
        })
    }

    fn bot_review(id: u64, body: &str) -> Value {
        json!({"databaseId": id, "submittedAt": "2026-11-21T10:00:00Z", "commit": {"oid": "c"}, "body": body})
    }

    fn posts(posts: u32, reviews: u32, newest: u64) -> Option<PostCount> {
        Some(PostCount {
            posts,
            reviews,
            newest: Some(newest),
        })
    }

    /// Runs `jobs` to the end against `answer`, which gives each alias's
    /// response for a slot. Returns how many queries it took.
    fn run(
        jobs: &mut [FindingsJob],
        budget: u32,
        answer: impl Fn(&FindingsJob, &Slot) -> Value,
    ) -> usize {
        let mut queries = 0;
        loop {
            let slots = plan_batch(jobs, budget, MAX_ALIASES);
            if slots.is_empty() {
                return queries;
            }
            queries += 1;
            assert!(queries <= 20, "must finish");
            let (query, _) = build_query(jobs, &slots);
            for (k, slot) in slots.iter().enumerate() {
                let asks = query
                    .lines()
                    .find(|l| l.contains(&format!("p{k}: node")))
                    .unwrap();
                assert_eq!(asks.contains(" posts: reviews("), slot.posts > 0);
            }
            let d: Map<String, Value> = slots
                .iter()
                .enumerate()
                .map(|(k, slot)| (format!("p{k}"), answer(&jobs[slot.job], slot)))
                .collect();
            apply_response(jobs, &slots, &d);
        }
    }

    #[test]
    fn rounds_come_from_the_window_when_it_holds_every_review() {
        let window = [
            bot_review(1, ROUND),
            bot_review(2, ""),
            bot_review(3, "<!-- thepastaclaw-ci-update v1 sha=abc -->\nCI failed"),
            bot_review(4, ROUND),
        ];
        let mut jobs = vec![job(1, 0)];
        let queries = run(&mut jobs, BUDGET, |_, _| reviews_page(1, 4, &window));
        assert_eq!(queries, 1);
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert_eq!(reading.posts, posts(2, 4, 4));
        // Never reviewed by the bot: zero, known.
        let mut jobs = vec![job(2, 0)];
        run(&mut jobs, BUDGET, |_, _| reviews_page(2, 0, &[]));
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert_eq!(reading.posts, Some(PostCount::default()));
    }

    #[test]
    fn rounds_count_on_from_the_count_read_before() {
        let all = reviews_5749();
        let window = &all[43..];
        // Counted through the 45th review before; the window holds the
        // last 10 (44th to 53rd).
        let mut before = PostCount::default();
        for r in &all[..45] {
            before.add(r["databaseId"].as_u64(), r["body"].as_str().unwrap());
        }
        let mut jobs = vec![job(5749, 0).with_posts_before(Some(before))];
        let queries = run(&mut jobs, BUDGET, |_, _| reviews_page(5749, 53, window));
        assert_eq!(queries, 1, "no review is read twice");
        let reading = jobs.remove(0).into_result().1.unwrap();
        let newest = all[52]["databaseId"].as_u64().unwrap();
        assert_eq!(reading.posts, posts(30, 53, newest));
    }

    #[test]
    fn rounds_of_platform_5749_are_counted_across_pages() {
        // First read of a PR with more bot reviews than the window: every
        // review is read, here 25 at a time (a thread budget of 25), after
        // the window.
        let all = reviews_5749();
        let mut jobs = vec![job(5749, 0)];
        let queries = run(&mut jobs, 25, |job, slot| {
            if slot.posts == 0 {
                return reviews_page(5749, 53, &all[43..]);
            }
            let from: usize = job.posts_after.as_deref().map_or(0, |c| c.parse().unwrap());
            let to = (from + slot.posts as usize).min(all.len());
            let next = (to < all.len()).then(|| to.to_string());
            posts_page(5749, &all[from..to], next.as_deref())
        });
        // The window, then 25 + 25 + 3.
        assert_eq!(queries, 4);
        let reading = jobs.remove(0).into_result().1.unwrap();
        let newest = all[52]["databaseId"].as_u64().unwrap();
        assert_eq!(reading.posts, posts(30, 53, newest));
        // The window still gives the last round.
        assert_eq!(reading.reviews.review_bot.len(), 10);
        assert!(reading.found.rounds.review_bot.is_some());
    }

    #[test]
    fn a_gap_after_the_count_read_before_reads_every_review() {
        let all = reviews_5749();
        // Counted through the 20th review: older than the window.
        let mut before = PostCount::default();
        for r in &all[..20] {
            before.add(r["databaseId"].as_u64(), r["body"].as_str().unwrap());
        }
        let mut jobs = vec![job(5749, 0).with_posts_before(Some(before))];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", reviews_page(5749, 53, &all[43..]))]),
        );
        assert_eq!(jobs[0].state, JobState::Pending);

        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(
            slots,
            vec![Slot {
                job: 0,
                first: 0,
                comments: 0,
                posts: 53
            }]
        );
        let (query, vars) = build_query(&jobs, &slots);
        assert_eq!(
            query,
            "query Findings($id0: ID!, $posts0: String) {\n  rateLimit { cost }\n  p0: node(id: $id0) { ... on PullRequest { number posts: reviews(first: 53, after: $posts0, author: \"thepastaclaw\") { pageInfo { hasNextPage endCursor } nodes { databaseId body } } } }\n}\n"
        );
        assert_eq!(vars["posts0"], Value::Null);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", posts_page(5749, &all, None))]),
        );
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert_eq!(reading.posts.map(|p| p.posts), Some(30));
    }

    #[test]
    fn counting_pages_share_the_thread_budget() {
        // Two PRs each with 80 bot reviews to count, one with 30 threads.
        let mut a = job(1, 0);
        let mut b = job(2, 30);
        for j in [&mut a, &mut b] {
            j.pages = 1;
            j.reviews_given = true;
            j.comments_pending = false;
            j.posts_pending = true;
            j.posts_remaining = 80;
        }
        let jobs = vec![a, b];
        let at = |job, first, posts| Slot {
            job,
            first,
            comments: 0,
            posts,
        };
        assert_eq!(
            plan_batch(&jobs, BUDGET, MAX_ALIASES),
            vec![at(0, 0, 80), at(1, 30, 80)]
        );
        // 100: the second does not fit and waits.
        assert_eq!(plan_batch(&jobs, 100, MAX_ALIASES), vec![at(0, 0, 80)]);
        // A budget smaller than the first job: threads first, then reviews.
        assert_eq!(plan_batch(&jobs[1..], 50, MAX_ALIASES), vec![at(0, 30, 20)]);
        assert_eq!(plan_batch(&jobs[1..], 20, MAX_ALIASES), vec![at(0, 20, 0)]);
        // Never more than 100 reviews in one page.
        let mut big = jobs[0].clone();
        big.posts_remaining = 1000;
        assert_eq!(
            plan_batch(&[big], BUDGET, MAX_ALIASES),
            vec![at(0, 0, MAX_FIRST)]
        );
    }

    #[test]
    fn a_counting_page_that_does_not_come_back_fails_the_pr() {
        let all = reviews_5749();
        let mut jobs = vec![job(5749, 0), job(6056, 0)];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[
                ("p0", reviews_page(5749, 53, &all[43..])),
                ("p1", reviews_page(6056, 53, &all[43..])),
            ]),
        );
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        let mut without = posts_page(6056, &all, None);
        without.as_object_mut().unwrap().remove("posts");
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", posts_page(5749, &all, None)), ("p1", without)]),
        );
        let results: Vec<_> = jobs.into_iter().map(FindingsJob::into_result).collect();
        assert_eq!(
            results[0].1.as_ref().unwrap().posts.map(|p| p.posts),
            Some(30)
        );
        assert!(results[1].1.is_err());
    }

    #[test]
    fn a_window_without_its_total_is_counted_in_full() {
        // Nothing says the window holds every review.
        let mut jobs = vec![job(1, 0)];
        let mut first = reviews_page(1, 0, &[bot_review(1, ROUND)]);
        first["bot"].as_object_mut().unwrap().remove("totalCount");
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        apply_response(&mut jobs, &slots, &data(&[("p0", first)]));
        assert_eq!(jobs[0].state, JobState::Pending);
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(slots[0].posts, MAX_FIRST);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", posts_page(1, &[bot_review(1, ROUND)], None))]),
        );
        assert_eq!(
            jobs.remove(0).into_result().1.unwrap().posts,
            posts(1, 1, 1)
        );
    }

    #[test]
    fn known_traits_are_handed_back_without_reading_comments() {
        let known = Known {
            reviews: Reviews::default(),
            posts: PostCount::default(),
            traits: vec![crate::model::PrTrait {
                kind: crate::model::TraitKind::ContractChange,
                summary: "dpns-contract v3: changed".into(),
                links: vec![],
            }],
        };
        let mut jobs = vec![job(11, 1).with_known(known.clone())];
        let slots = plan_batch(&jobs, BUDGET, MAX_ALIASES);
        assert_eq!(slots[0].comments, 0);
        apply_response(
            &mut jobs,
            &slots,
            &data(&[("p0", page(11, &["A"], &[], None, false))]),
        );
        let reading = jobs.remove(0).into_result().1.unwrap();
        assert!(!reading.reviews_fresh);
        assert_eq!(reading.traits, known.traits);
    }
}
