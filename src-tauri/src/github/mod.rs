//! GitHub client: finds a token, then reads projects over the GraphQL API.
//!
//! The HTTP layer here is thin: it sends a query and hands the status,
//! rate-limit headers and body to the pure functions in `parse`.

mod coderabbit;
mod finding;
mod finding_cache;
mod findings_query;
mod parse;
mod rounds;
mod search;
mod token;
mod traits;
pub mod types;

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{AppError, AppResult};
use crate::model::{PrSummary, Viewer};
use finding_cache::{FindingCache, Known, PrState, Reading};
use findings_query::FindingsJob;
use parse::{
    AccountData, AddCommentData, Failure, FollowedData, Gql, MergeCheckData, MineData, OpenPrsData,
    OpenRow, PrThreads, QueryCost, RateHeaders, RepoAccess, RepoData, SelfReviewCheckData,
    SquashMergeData, ThreadsData, ViewerData,
};
use types::{GhAccount, GhMergeCheck, GhPrAuthor, GhProject, GhPullRequest, GhSelfReviewCheck};

/// Default GraphQL endpoint.
pub const GRAPHQL_URL: &str = "https://api.github.com/graphql";

const TIMEOUT: Duration = Duration::from_secs(20);
const USER_AGENT_VALUE: &str = "pr-status-viewer";

/// First page size for the viewer's PR search. Halved when GitHub rejects
/// or times out on the query, down to `MIN_PAGE`.
const MINE_PAGE: u32 = 50;
/// Followed PRs fetched per aliased query.
const FOLLOWED_BATCH: usize = 25;
const OPEN_PAGE: u32 = 100;
const OPEN_CAP: usize = 300;
const MIN_PAGE: u32 = 5;
/// Most PRs the searches that include tracked contributors may list for one
/// project. Past it the rest of the contributors' PRs (the least recently
/// updated, as the search is sorted) are left out; the viewer's own PRs are
/// then read with a search of their own, so they are never crowded out.
const CONTRIBUTOR_CAP: usize = 150;
/// Safety stop for pagination loops.
const MAX_PAGES: usize = 40;

/// Review threads are counted in pages of 100 (in `merge_fields.graphql`
/// and `threads.graphql`); this caps how many pages one PR may take.
const MAX_THREAD_PAGES: usize = 50;
/// Safety stop for the findings follow-up queries of one project.
const MAX_FINDINGS_QUERIES: usize = 100;

const PR_FIELDS: &str = concat!(
    include_str!("queries/pr_fields.graphql"),
    "\n",
    include_str!("queries/merge_fields.graphql"),
);
const MERGE_CHECK_QUERY: &str = concat!(
    include_str!("queries/merge_check.graphql"),
    "\n",
    include_str!("queries/merge_fields.graphql"),
);
const THREADS_QUERY: &str = include_str!("queries/threads.graphql");
const SQUASH_MERGE_MUTATION: &str = include_str!("queries/squash_merge.graphql");
const SELF_REVIEW_CHECK_QUERY: &str = include_str!("queries/self_review_check.graphql");
const POST_COMMENT_MUTATION: &str = include_str!("queries/post_comment.graphql");
const VIEWER_QUERY: &str = include_str!("queries/viewer.graphql");
const REPO_QUERY: &str = include_str!("queries/repo.graphql");
const MINE_QUERY: &str = include_str!("queries/mine.graphql");
const OPEN_PRS_QUERY: &str = include_str!("queries/open_prs.graphql");
const ACCOUNT_QUERY: &str = include_str!("queries/account.graphql");

/// Finds a GitHub token without asking the user: `GH_TOKEN`, then
/// `GITHUB_TOKEN`, then `gh auth token`. Returns `AppError::NotSignedIn`
/// with a helpful message when none is found.
pub async fn resolve_token() -> AppResult<String> {
    token::resolve().await
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    http: reqwest::Client,
    endpoint: String,
    token: Token,
    /// The last-round findings of each PR. Shared by clones.
    findings: Arc<Mutex<FindingCache>>,
    usage: Arc<UsageCounters>,
}

/// What this client's queries have cost so far, for diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueryUsage {
    /// Requests sent.
    pub requests: u64,
    /// Response bytes received.
    pub bytes: u64,
    /// Rate-limit points, summed over the queries that report their cost
    /// (`rateLimit { cost }`): the project, followed, thread and findings
    /// queries.
    pub cost: u64,
}

#[derive(Debug, Default)]
struct UsageCounters {
    requests: AtomicU64,
    bytes: AtomicU64,
    cost: AtomicU64,
}

/// Keeps the token out of `Debug` output.
#[derive(Clone)]
struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

impl GitHubClient {
    pub fn new(token: String) -> AppResult<Self> {
        Self::with_endpoint(token, GRAPHQL_URL)
    }

    /// Same as `new`, against another GraphQL endpoint (a test server, or
    /// GitHub Enterprise).
    pub fn with_endpoint(token: String, endpoint: &str) -> AppResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .default_headers(headers)
            .build()
            .map_err(|e| AppError::Other(format!("could not set up the HTTP client: {e}")))?;
        Ok(Self {
            http,
            endpoint: endpoint.to_owned(),
            token: Token(token),
            findings: Arc::default(),
            usage: Arc::default(),
        })
    }

    /// What this client's queries have cost so far.
    pub fn usage(&self) -> QueryUsage {
        QueryUsage {
            requests: self.usage.requests.load(Ordering::Relaxed),
            bytes: self.usage.bytes.load(Ordering::Relaxed),
            cost: self.usage.cost.load(Ordering::Relaxed),
        }
    }

    /// The account named `login`, in GitHub's casing, or `None` when there
    /// is no such user or organization (GitHub App bots are neither).
    pub async fn account(&self, login: &str) -> AppResult<Option<GhAccount>> {
        let result = self
            .query::<AccountData>(ACCOUNT_QUERY, json!({ "login": login }))
            .await;
        parse::parse_account(result)
    }

    pub async fn viewer(&self) -> AppResult<Viewer> {
        let resp: Gql<ViewerData> = self.query(VIEWER_QUERY, json!({})).await?;
        Ok(parse::parse_viewer(resp.data))
    }

    /// Confirms the repo exists and returns its name in GitHub's casing.
    pub async fn canonical_repo(&self, repo: &str) -> AppResult<String> {
        let (owner, name) = split_repo(repo)?;
        let resp: Gql<RepoData> = self
            .query(REPO_QUERY, json!({ "owner": owner, "name": name }))
            .await
            .map_err(|f| not_found_for(f, repo))?;
        parse::parse_repo_name(resp, repo)
    }

    /// The open PRs in `repo` by the viewer and by the `contributors`
    /// (logins), plus the PRs in `followed` (by number) that are still open.
    ///
    /// Review threads are counted exactly; a failure there fails the
    /// project. The findings of the last review rounds come from a cache
    /// and a follow-up query (`attach_findings`); a failure there never
    /// fails the project.
    pub async fn fetch_project(
        &self,
        repo: &str,
        followed: &[u64],
        contributors: &[String],
    ) -> AppResult<GhProject> {
        let (mine, followed_prs) = tokio::try_join!(
            self.fetch_authored(repo, contributors),
            self.fetch_followed(repo, followed)
        )?;
        let (mine_prs, mut threads) = mine.prs;
        let (followed_prs, followed_threads) = followed_prs;
        threads.extend(followed_threads);
        let mut prs = parse::merge_prs(mine_prs, followed_prs);
        let threads = self.count_remaining_threads(&mut prs, threads).await?;
        let canonical = mine.repo;
        for pr in &mut prs {
            pr.repo.clone_from(&canonical);
            mine.access.apply(&mut pr.merge);
        }
        self.attach_findings(&canonical, &mut prs, &threads).await;
        Ok(GhProject {
            repo: canonical,
            open_count: mine.open_count,
            prs,
        })
    }

    /// A fresh look at one PR: everything that decides whether it can be
    /// squash-merged now, with every review thread counted.
    pub async fn merge_check(&self, repo: &str, number: u64) -> AppResult<GhMergeCheck> {
        let (owner, name) = split_repo(repo)?;
        let vars = json!({ "owner": owner, "name": name, "number": number });
        let resp: Gql<MergeCheckData> = self
            .query(MERGE_CHECK_QUERY, vars)
            .await
            .map_err(|f| not_found_for(f, repo))?;
        let (mut check, threads) = parse::parse_merge_check(resp, repo, number)?;
        if threads.after.is_some() {
            self.finish_threads(threads)
                .await?
                .count
                .apply(&mut check.facts);
        }
        Ok(check)
    }

    /// Squash-merges a PR with GitHub's `mergePullRequest`. GitHub refuses
    /// if the head is no longer `expected_head_oid`. No commit title or body
    /// is sent, so the repository's own squash settings apply. Never retried.
    pub async fn squash_merge(&self, pr_node_id: &str, expected_head_oid: &str) -> AppResult<()> {
        if pr_node_id.is_empty() || expected_head_oid.is_empty() {
            return Err(AppError::InvalidInput(
                "The pull request or its head commit is unknown.".into(),
            ));
        }
        let vars = json!({ "id": pr_node_id, "head": expected_head_oid });
        let result = self
            .query::<SquashMergeData>(SQUASH_MERGE_MUTATION, vars)
            .await;
        parse::interpret_squash_merge(result)
    }

    /// A fresh look at one PR: everything that decides whether the viewer
    /// can post the PR Hygiene self-review on it now. Read-only.
    pub async fn self_review_check(&self, repo: &str, number: u64) -> AppResult<GhSelfReviewCheck> {
        let (owner, name) = split_repo(repo)?;
        let vars = json!({ "owner": owner, "name": name, "number": number });
        let resp: Gql<SelfReviewCheckData> = self
            .query(SELF_REVIEW_CHECK_QUERY, vars)
            .await
            .map_err(|f| not_found_for(f, repo))?;
        parse::parse_self_review_check(resp, repo, number)
    }

    /// Posts `body` as a comment on the PR (or issue) with node id
    /// `subject_id`, as the signed-in user, with GitHub's `addComment`. It
    /// is the user's own statement on GitHub: only ever called by the
    /// engine's `post_self_review`, after a fresh check, when the user has
    /// confirmed. Never retried.
    pub async fn add_comment(&self, subject_id: &str, body: &str) -> AppResult<()> {
        if subject_id.is_empty() || body.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "The pull request or the comment is missing.".into(),
            ));
        }
        let vars = json!({ "subjectId": subject_id, "body": body });
        let result = self
            .query::<AddCommentData>(POST_COMMENT_MUTATION, vars)
            .await;
        parse::interpret_add_comment(result)
    }

    /// Reads the review threads past the first page of each PR, and returns
    /// every PR's full thread list by number. Fails rather than leave a
    /// count short.
    async fn count_remaining_threads(
        &self,
        prs: &mut [GhPullRequest],
        threads: Vec<PrThreads>,
    ) -> AppResult<HashMap<u64, PrThreads>> {
        let mut by_number: HashMap<u64, PrThreads> = HashMap::with_capacity(threads.len());
        for t in threads {
            // A PR both mine and followed: the first copy wins, as in `merge_prs`.
            by_number.entry(t.number).or_insert(t);
        }
        let mut numbers: Vec<u64> = by_number
            .values()
            .filter(|t| t.after.is_some())
            .map(|t| t.number)
            .collect();
        numbers.sort_unstable();
        for number in numbers {
            let Some(t) = by_number.remove(&number) else {
                continue;
            };
            let done = self.finish_threads(t).await?;
            for pr in prs.iter_mut().filter(|p| p.number == number) {
                done.count.apply(&mut pr.merge);
            }
            by_number.insert(number, done);
        }
        Ok(by_number)
    }

    /// Fetches the remaining pages of one PR's review threads.
    async fn finish_threads(&self, mut more: PrThreads) -> AppResult<PrThreads> {
        for _ in 0..MAX_THREAD_PAGES {
            let Some(after) = more.after.take() else {
                return Ok(more);
            };
            let vars = json!({ "id": more.node_id, "after": after });
            let resp: Gql<ThreadsData> = self.query(THREADS_QUERY, vars).await?;
            more.after = parse::parse_threads_page(resp, &mut more)?;
        }
        if more.after.is_none() {
            return Ok(more);
        }
        Err(AppError::Other(format!(
            "pull request #{} has too many review threads to count",
            more.number
        )))
    }

    /// Sets each PR's last-round findings, round times, traits and the
    /// review bot's post count (its round number): from the
    /// cache when nothing about the PR changed, otherwise read again with
    /// the follow-up query. Never fails: a PR whose read failed shows what
    /// was cached, or nothing.
    async fn attach_findings(
        &self,
        repo: &str,
        prs: &mut [GhPullRequest],
        threads: &HashMap<u64, PrThreads>,
    ) {
        let now = chrono::Utc::now();
        let states: Vec<PrState> = prs
            .iter()
            .filter_map(|pr| threads.get(&pr.number).map(|t| PrState::new(pr, t)))
            .collect();
        let mut fetched: HashMap<u64, Result<Reading, String>> = HashMap::new();
        let mut jobs = Vec::new();
        {
            let cache = self.finding_cache();
            for s in states
                .iter()
                .filter(|s| cache.refetch(repo, s, now).is_some())
            {
                // Only time says to read again: the reviews and traits read
                // last time still stand.
                let known = cache.known(repo, s, now);
                let nothing_else = s.thread_count() == 0 && !s.reviewed();
                match (
                    known,
                    FindingsJob::new(s.number, &s.fingerprint.node_id, s.thread_count()),
                ) {
                    // No threads and no review: no findings, and the traits
                    // are known.
                    (
                        Some(Known {
                            reviews,
                            traits,
                            posts,
                        }),
                        _,
                    ) if nothing_else => {
                        let reading = Reading {
                            reviews,
                            traits,
                            posts: Some(posts),
                            ..Reading::default()
                        };
                        fetched.insert(s.number, Ok(reading));
                    }
                    (Some(known), Some(job)) => jobs.push(job.with_known(known)),
                    (None, Some(job)) => {
                        jobs.push(job.with_posts_before(cache.posts_before(repo, s)));
                    }
                    // No node id: nothing can be asked.
                    (_, None) => {
                        fetched.insert(s.number, Ok(Reading::default()));
                    }
                }
            }
        }
        if !jobs.is_empty() {
            log::debug!("{repo}: reading the findings of {} PRs", jobs.len());
            fetched.extend(self.read_findings(jobs).await);
        }
        let mut found = self.finding_cache().settle(repo, &states, fetched, now);
        for pr in prs.iter_mut() {
            (pr.findings, pr.last_rounds, pr.traits, pr.rounds) =
                found.remove(&pr.number).unwrap_or_default();
        }
    }

    /// Runs the findings follow-up queries for `jobs`, with the same
    /// page-size backoff as the other queries. Returns each job's outcome.
    async fn read_findings(
        &self,
        mut jobs: Vec<FindingsJob>,
    ) -> Vec<(u64, Result<Reading, String>)> {
        let mut budget = findings_query::BUDGET;
        for _ in 0..MAX_FINDINGS_QUERIES {
            let slots = findings_query::plan_batch(&jobs, budget, findings_query::MAX_ALIASES);
            if slots.is_empty() {
                break;
            }
            let (query, vars) = findings_query::build_query(&jobs, &slots);
            match self
                .query::<serde_json::Map<String, Value>>(&query, vars)
                .await
            {
                Ok(resp) => {
                    if let Some(e) = resp.errors.first() {
                        log::debug!("findings query came back with errors: {}", e.message);
                    }
                    findings_query::apply_response(&mut jobs, &slots, &resp.data);
                }
                Err(Failure::TooCostly(msg)) if budget > findings_query::MIN_BUDGET => {
                    budget = (budget / 2).max(findings_query::MIN_BUDGET);
                    log::info!("findings query too costly ({msg}); thread budget now {budget}");
                }
                Err(f) => {
                    let why = AppError::from(f).to_string();
                    for job in &mut jobs {
                        job.fail(why.clone());
                    }
                    break;
                }
            }
        }
        jobs.into_iter().map(FindingsJob::into_result).collect()
    }

    /// Diagnostics only: makes the cached findings (and traits) look `minutes` older,
    /// so the next pass re-reads what has expired (as it would after that
    /// long). Used by `examples/findings_cost.rs`.
    #[doc(hidden)]
    pub fn age_findings_cache(&self, minutes: i64) {
        self.finding_cache()
            .age(chrono::TimeDelta::minutes(minutes));
    }

    fn finding_cache(&self) -> MutexGuard<'_, FindingCache> {
        // A panic while holding the lock leaves the map whole; keep using it.
        self.findings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Every open PR in `repo`, newest update first, as light rows.
    /// `is_mine` and `is_followed` are left false for the caller to fill.
    pub async fn list_open_prs(&self, repo: &str) -> AppResult<Vec<PrSummary>> {
        let rows = self.open_rows(repo).await?;
        Ok(rows.into_iter().map(|r| r.summary).collect())
    }

    /// The author of every open PR in `repo` (one entry per PR, newest
    /// update first), from the same listing as `list_open_prs`.
    pub async fn open_pr_authors(&self, repo: &str) -> AppResult<Vec<GhPrAuthor>> {
        let rows = self.open_rows(repo).await?;
        Ok(rows.into_iter().map(|r| r.author).collect())
    }

    /// The open PRs of `repo`, up to `OPEN_CAP`.
    async fn open_rows(&self, repo: &str) -> AppResult<Vec<OpenRow>> {
        let (owner, name) = split_repo(repo)?;
        let mut rows: Vec<OpenRow> = Vec::new();
        let mut after: Option<String> = None;
        let mut page_size = OPEN_PAGE;
        for _ in 0..MAX_PAGES {
            let want = u32::try_from(OPEN_CAP - rows.len()).unwrap_or(OPEN_PAGE);
            let first = page_size.min(want);
            let vars = json!({ "owner": owner, "name": name, "first": first, "after": after });
            let resp: Gql<OpenPrsData> = match self.query(OPEN_PRS_QUERY, vars).await {
                Ok(resp) => resp,
                Err(Failure::TooCostly(msg)) if page_size > MIN_PAGE => {
                    page_size = (page_size / 2).max(MIN_PAGE);
                    log::info!("open PR list too costly ({msg}); page size now {page_size}");
                    continue;
                }
                Err(f) => return Err(not_found_for(f, repo).into()),
            };
            let page = parse::parse_open_page(resp, repo)?;
            for row in page.rows {
                if !rows.iter().any(|r| r.summary.number == row.summary.number) {
                    rows.push(row);
                }
            }
            if rows.len() >= OPEN_CAP || page.next.is_none() {
                break;
            }
            after = page.next;
        }
        rows.truncate(OPEN_CAP);
        Ok(rows)
    }

    /// The open PRs by the viewer and the `contributors`, with the repo's
    /// canonical name, open count and merge settings.
    ///
    /// One search covers the viewer and as many contributors as fit in a
    /// search string; the others go into further searches (`search`). The
    /// searches with contributors stop at `CONTRIBUTOR_CAP` PRs between them;
    /// if the one with the viewer is cut short, the viewer's PRs are read
    /// again with a search of their own, which has no such cap.
    async fn fetch_authored(&self, repo: &str, contributors: &[String]) -> AppResult<Mine> {
        let (owner, name) = split_repo(repo)?;
        let mut found = Found::default();
        let mut budget = CONTRIBUTOR_CAP;
        for s in search::author_searches(owner, name, contributors) {
            let cap = s.has_others.then_some(budget);
            if cap == Some(0) {
                log::info!("{repo}: contributors' PRs past {CONTRIBUTOR_CAP} are not listed");
                continue;
            }
            let page = self.search_prs(repo, &s.query, cap).await?;
            budget = budget.saturating_sub(page.prs.len());
            let cut_short = page.cut_short;
            found.add(page);
            if cut_short {
                log::info!("{repo}: contributors' PRs past {CONTRIBUTOR_CAP} are not listed");
                if s.has_viewer {
                    let viewer = search::viewer_search(owner, name);
                    found.add(self.search_prs(repo, &viewer, None).await?);
                }
            }
        }
        let (canonical, open_count, access) = found.meta.ok_or_else(|| {
            AppError::Other(format!("GitHub could not answer the query for {repo}"))
        })?;
        Ok(Mine {
            repo: canonical,
            open_count,
            access,
            prs: (found.prs, found.threads),
        })
    }

    /// Every PR one search string finds, page by page, up to `cap` PRs
    /// (`None`: as many as `MAX_PAGES` pages hold).
    async fn search_prs(&self, repo: &str, search: &str, cap: Option<usize>) -> AppResult<Found> {
        let (owner, name) = split_repo(repo)?;
        let query = format!("{MINE_QUERY}\n{PR_FIELDS}");
        let mut found = Found::default();
        let mut after: Option<String> = None;
        let mut page_size = MINE_PAGE;
        for _ in 0..MAX_PAGES {
            let left = cap.map(|c| c.saturating_sub(found.prs.len()));
            let first = match left {
                Some(left) => page_size.min(u32::try_from(left).unwrap_or(page_size)),
                None => page_size,
            };
            let vars = json!({
                "owner": owner, "name": name, "search": search,
                "first": first.max(1), "after": after,
            });
            let resp: Gql<MineData> = match self.query(&query, vars).await {
                Ok(resp) => resp,
                Err(Failure::TooCostly(msg)) if page_size > MIN_PAGE => {
                    page_size = (page_size / 2).max(MIN_PAGE);
                    log::info!("PR query too costly ({msg}); page size now {page_size}");
                    continue;
                }
                Err(f) => return Err(not_found_for(f, repo).into()),
            };
            let page = parse::parse_mine_page(resp, repo)?;
            found
                .meta
                .get_or_insert((page.repo, page.open_count, page.access));
            found.prs = parse::merge_prs(std::mem::take(&mut found.prs), page.prs);
            found.threads.extend(page.threads);
            let Some(cursor) = page.next else {
                break;
            };
            if cap.is_some_and(|c| found.prs.len() >= c) {
                found.cut_short = true;
                break;
            }
            after = Some(cursor);
        }
        Ok(found)
    }

    /// Followed PRs that are still open, fetched by number with aliases.
    async fn fetch_followed(
        &self,
        repo: &str,
        numbers: &[u64],
    ) -> AppResult<(Vec<GhPullRequest>, Vec<PrThreads>)> {
        let mut wanted: Vec<u64> = numbers.iter().copied().filter(|n| *n > 0).collect();
        wanted.sort_unstable();
        wanted.dedup();
        if wanted.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let (owner, name) = split_repo(repo)?;
        let mut out = Vec::new();
        let mut threads = Vec::new();
        let mut batch = FOLLOWED_BATCH;
        let mut rest: &[u64] = &wanted;
        while !rest.is_empty() {
            let take = batch.min(rest.len());
            let (chunk, tail) = rest.split_at(take);
            let query = followed_query(chunk);
            let vars = json!({ "owner": owner, "name": name });
            match self.query::<FollowedData>(&query, vars).await {
                Ok(resp) => {
                    let page = parse::parse_followed(resp, repo)?;
                    out.extend(page.prs);
                    threads.extend(page.threads);
                    rest = tail;
                }
                Err(Failure::TooCostly(msg)) if batch > 1 => {
                    batch = (batch / 2).max(1);
                    log::info!("followed PR query too costly ({msg}); batch now {batch}");
                }
                Err(f) => return Err(not_found_for(f, repo).into()),
            }
        }
        Ok((out, threads))
    }

    async fn query<T: DeserializeOwned + QueryCost>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Gql<T>, Failure> {
        let mut auth = HeaderValue::from_str(&format!("bearer {}", self.token.0))
            .map_err(|_| AppError::Unauthorized("the token contains invalid characters".into()))?;
        auth.set_sensitive(true);
        let resp = self
            .http
            .post(&self.endpoint)
            .header(AUTHORIZATION, auth)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| {
                // A transport timeout is the network, not query cost:
                // retrying with smaller pages would only multiply the wait.
                if e.is_timeout() {
                    Failure::App(AppError::Network("the request timed out".into()))
                } else {
                    Failure::App(AppError::from(e.without_url()))
                }
            })?;
        let status = resp.status().as_u16();
        let rate = rate_headers(resp.headers());
        let body = resp
            .bytes()
            .await
            .map_err(|e| Failure::App(AppError::from(e.without_url())))?;
        let usage = &self.usage;
        usage.requests.fetch_add(1, Ordering::Relaxed);
        usage.bytes.fetch_add(
            u64::try_from(body.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        let resp = parse::interpret::<T>(status, rate, &body, chrono::Utc::now())?;
        if let Some(cost) = resp.data.cost() {
            usage.cost.fetch_add(u64::from(cost), Ordering::Relaxed);
        }
        Ok(resp)
    }
}

/// The viewer's own PRs in a repo, and the repo facts that came with them.
struct Mine {
    repo: String,
    open_count: u32,
    access: RepoAccess,
    /// The PRs, and their review threads.
    prs: (Vec<GhPullRequest>, Vec<PrThreads>),
}

/// What one or more PR searches found in a repo.
#[derive(Default)]
struct Found {
    /// Canonical name, open count and merge settings, from the first page.
    meta: Option<(String, u32, RepoAccess)>,
    prs: Vec<GhPullRequest>,
    threads: Vec<PrThreads>,
    /// The search stopped at its cap with more results left.
    cut_short: bool,
}

impl Found {
    fn add(&mut self, other: Found) {
        if self.meta.is_none() {
            self.meta = other.meta;
        }
        self.prs = parse::merge_prs(std::mem::take(&mut self.prs), other.prs);
        self.threads.extend(other.threads);
    }
}

/// A GraphQL NOT_FOUND without data names the repo the user asked for.
fn not_found_for(f: Failure, repo: &str) -> Failure {
    match f {
        Failure::App(AppError::NotFound(_)) => Failure::App(AppError::NotFound(format!(
            "{repo} does not exist, or your GitHub account cannot see it"
        ))),
        other => other,
    }
}

fn rate_headers(headers: &HeaderMap) -> RateHeaders {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .map(str::to_owned)
    };
    RateHeaders {
        remaining: get("x-ratelimit-remaining").and_then(|v| v.parse().ok()),
        reset: get("x-ratelimit-reset").and_then(|v| v.parse().ok()),
        retry_after: get("retry-after").and_then(|v| v.parse().ok()),
    }
}

fn split_repo(repo: &str) -> AppResult<(&str, &str)> {
    match repo.split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => {
            Ok((owner, name))
        }
        _ => Err(AppError::InvalidInput(format!(
            "\"{repo}\" is not a repository. Use owner/name or a GitHub link."
        ))),
    }
}

/// One query that fetches each number under its own alias.
fn followed_query(numbers: &[u64]) -> String {
    let mut q = String::from(
        "query Followed($owner: String!, $name: String!) {\n  rateLimit { cost }\n  repository(owner: $owner, name: $name) {\n",
    );
    for n in numbers {
        q.push_str(&format!(
            "    pr{n}: pullRequest(number: {n}) {{ ...PrFields }}\n"
        ));
    }
    q.push_str("  }\n}\n");
    q.push_str(PR_FIELDS);
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repo_works() {
        assert_eq!(split_repo("a/b").unwrap(), ("a", "b"));
        assert!(split_repo("a").is_err());
        assert!(split_repo("a/b/c").is_err());
        assert!(split_repo("/b").is_err());
    }

    #[test]
    fn followed_query_has_one_alias_per_number() {
        let q = followed_query(&[12, 345]);
        assert!(q.contains("pr12: pullRequest(number: 12) { ...PrFields }"));
        assert!(q.contains("pr345: pullRequest(number: 345) { ...PrFields }"));
        assert!(q.contains("fragment PrFields on PullRequest"));
    }

    #[test]
    fn rate_headers_parse() {
        let mut h = HeaderMap::new();
        h.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h.insert("x-ratelimit-reset", HeaderValue::from_static("1795000000"));
        h.insert("retry-after", HeaderValue::from_static(" 30 "));
        let r = rate_headers(&h);
        assert_eq!(r.remaining, Some(0));
        assert_eq!(r.reset, Some(1_795_000_000));
        assert_eq!(r.retry_after, Some(30));
        let r = rate_headers(&HeaderMap::new());
        assert_eq!((r.remaining, r.reset, r.retry_after), (None, None, None));
    }

    #[test]
    fn debug_hides_token() {
        let client = GitHubClient::new("ghp_secret_value".into()).unwrap();
        let shown = format!("{client:?}");
        assert!(!shown.contains("ghp_secret_value"));
        assert!(shown.contains("redacted"));
    }
}
