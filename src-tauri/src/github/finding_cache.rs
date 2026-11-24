//! Remembers the findings of each reviewer's last round on each PR, so the
//! review and comment bodies are read only when something may have changed.
//!
//! The page queries return, every pass, each thread's id and resolved flag,
//! and each reviewer's latest review time (`latestReviews`). Which threads
//! belong to the last rounds, and each one's verdict `(severity, status)`,
//! come from the reviews' and the threads' root comments' bodies, which are
//! costly to read (`findings_query`, `rounds`). This cache keeps them per PR
//! and decides when they must be read again:
//!
//! - the PR is not cached;
//! - its node id, `updatedAt`, head commit, thread count or set of thread
//!   ids changed;
//! - the latest review of the review bot or of CodeRabbit changed: a new
//!   round (or a reply, which may change a status) was posted;
//! - its verdicts are older than `MAX_AGE`. The bot rewrites the status line
//!   inside an existing comment, and it is not established that such an edit
//!   bumps the PR's `updatedAt`, so time alone also triggers a re-read.
//!
//! Otherwise the cached verdicts are combined with the fresh resolved flags.
//!
//! The last rounds come back with the commit each was submitted on, cached
//! like the rest. Whether a round was on the PR's current head is not
//! decided here: `merge` compares that commit with the head of the pass
//! (`GhPullRequest::head_sha`), so a cached reading, or one kept after a
//! failed read, is always judged against the fresh head.
//!
//! When only time triggers the re-read, the reviews cannot have changed
//! (a new review changes the fingerprint), so the reviews read last time
//! are reused and only the threads are read again, which keeps the answer
//! small. Reviews are read afresh at least every `REVIEWS_MAX_AGE`, in case
//! a reviewer edited a round's body.
//!
//! The PR's traits (`traits`) are read from its comments together with the
//! reviews, and reused with them. A comment edit bumps the PR's `updatedAt`
//! (checked on dashpay/platform, November 2026), so a workflow rewriting
//! its trait comment changes the fingerprint; `REVIEWS_MAX_AGE` bounds
//! the rest.
//!
//! The bot's post count (`rounds::PostCount`, the PR's round number) is
//! read with the reviews and kept with them; a reading that reuses the
//! reviews reuses it too. Every fresh reading is handed the count read
//! last time (`posts_before`), so it only has to count the reviews posted
//! since. A failed reading keeps the old entry, count included, and a
//! reading that could not count (no node id) keeps the count it had.
//!
//! The cache lives inside the GitHub client, so it lasts as long as the
//! signed-in client does.

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};

use super::coderabbit;
use super::parse::{PrThreads, ThreadRef};
use super::rounds::{PostCount, Reviews};
use super::types::{
    GhFinding, GhFindingReviewer, GhFindingStatus, GhLastRounds, GhPullRequest, GhSeverity,
};
use crate::model::{BOT_LOGIN, PrTrait};

/// A reviewer's word on one thread.
pub(crate) type Verdict = (GhSeverity, GhFindingStatus);

/// What one reading of a PR found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Found {
    /// The threads of the last rounds that carry a finding, by thread id:
    /// who raised it and what it says.
    pub findings: HashMap<String, (GhFindingReviewer, Verdict)>,
    pub rounds: GhLastRounds,
}

/// One successful reading of a PR.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reading {
    pub found: Found,
    /// The reviews the rounds were picked from.
    pub reviews: Reviews,
    /// The reviews (and comments) were read in this reading, not reused
    /// from the cache.
    pub reviews_fresh: bool,
    /// The bot's post count, counted with the reviews or reused with them.
    /// `None` only when nothing could be read.
    pub posts: Option<PostCount>,
    /// The PR's traits, from its comments, or reused with the reviews.
    pub traits: Vec<PrTrait>,
}

/// What a reading may reuse instead of asking again: the reviews, and the
/// traits and post count read with them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Known {
    pub reviews: Reviews,
    pub traits: Vec<PrTrait>,
    pub posts: PostCount,
}

/// What a pass hands back per PR: its findings, its last rounds, its
/// traits and the bot's post count.
pub(crate) type Settled = (Vec<GhFinding>, GhLastRounds, Vec<PrTrait>, Option<u32>);

/// Verdicts older than this are read again.
pub(crate) const MAX_AGE: TimeDelta = TimeDelta::minutes(5);
/// Reviews (and the traits read with them) read longer ago than this are
/// not reused.
pub(crate) const REVIEWS_MAX_AGE: TimeDelta = TimeDelta::minutes(30);
/// An entry not listed by any pass for this long is dropped (its project
/// was removed, or never fetched again).
const FORGET_AFTER: TimeDelta = TimeDelta::hours(1);

/// What has to stay the same for cached verdicts to be reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    pub node_id: String,
    pub updated_at: DateTime<Utc>,
    pub head_sha: String,
    /// GitHub's thread `totalCount`.
    pub total: u32,
    /// Sorted.
    pub thread_ids: Vec<String>,
    /// The latest review of the review bot and of CodeRabbit, from the
    /// page query's `latestReviews`: lower-case login and submission time,
    /// sorted. Empty when neither has reviewed.
    pub reviews: Vec<(String, Option<DateTime<Utc>>)>,
}

/// One PR as this pass sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrState {
    pub number: u64,
    pub fingerprint: Fingerprint,
    /// In GitHub's order, with fresh resolved flags.
    pub threads: Vec<ThreadRef>,
}

impl PrState {
    pub fn new(pr: &GhPullRequest, threads: &PrThreads) -> Self {
        let mut thread_ids: Vec<String> = threads.threads.iter().map(|t| t.id.clone()).collect();
        thread_ids.sort_unstable();
        thread_ids.dedup();
        let mut reviews: Vec<(String, Option<DateTime<Utc>>)> = pr
            .reviews
            .iter()
            .filter(|r| is_counted_reviewer(&r.login))
            .map(|r| (r.login.to_ascii_lowercase(), r.submitted_at))
            .collect();
        reviews.sort_unstable();
        Self {
            number: pr.number,
            fingerprint: Fingerprint {
                node_id: pr.merge.node_id.clone(),
                updated_at: pr.updated_at,
                head_sha: pr.head_sha.clone(),
                total: threads.count.total,
                thread_ids,
                reviews,
            },
            threads: threads.threads.clone(),
        }
    }

    /// Either reviewer has reviewed the PR, so it may have a last round
    /// even without threads.
    pub fn reviewed(&self) -> bool {
        !self.fingerprint.reviews.is_empty()
    }

    /// Threads to read: the larger of the count and the listed threads.
    pub fn thread_count(&self) -> u32 {
        let listed = u32::try_from(self.threads.len()).unwrap_or(u32::MAX);
        self.fingerprint.total.max(listed)
    }
}

/// Why a PR's verdicts must be read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refetch {
    NotCached,
    Changed,
    Expired,
}

#[derive(Debug, Clone)]
struct Entry {
    fingerprint: Fingerprint,
    fetched_at: DateTime<Utc>,
    /// Last time a pass listed the PR.
    listed_at: DateTime<Utc>,
    found: Found,
    reviews: Reviews,
    traits: Vec<PrTrait>,
    /// The bot's post count; `None` until one reading could count it.
    posts: Option<PostCount>,
    /// When the reviews and traits were read.
    reviews_read_at: DateTime<Utc>,
}

/// Verdicts per PR, keyed by repo (lower case) and number.
#[derive(Debug, Default)]
pub(crate) struct FindingCache {
    entries: HashMap<(String, u64), Entry>,
}

fn key(repo: &str, number: u64) -> (String, u64) {
    (repo.to_ascii_lowercase(), number)
}

impl FindingCache {
    /// Whether, and why, `pr`'s verdicts must be read again at `now`.
    pub fn refetch(&self, repo: &str, pr: &PrState, now: DateTime<Utc>) -> Option<Refetch> {
        let Some(entry) = self.entries.get(&key(repo, pr.number)) else {
            return Some(Refetch::NotCached);
        };
        if entry.fingerprint != pr.fingerprint {
            Some(Refetch::Changed)
        } else if now - entry.fetched_at >= MAX_AGE {
            Some(Refetch::Expired)
        } else {
            None
        }
    }

    /// The reviews, traits and post count read last time, when they can
    /// stand in for reading them again: nothing about the PR changed, they
    /// are younger than `REVIEWS_MAX_AGE`, and the posts were counted.
    pub fn known(&self, repo: &str, pr: &PrState, now: DateTime<Utc>) -> Option<Known> {
        let entry = self.entries.get(&key(repo, pr.number))?;
        let posts = entry.posts?;
        (entry.fingerprint == pr.fingerprint && now - entry.reviews_read_at < REVIEWS_MAX_AGE).then(
            || Known {
                reviews: entry.reviews.clone(),
                traits: entry.traits.clone(),
                posts,
            },
        )
    }

    /// The bot's post count read last time for this PR (the same node),
    /// for a fresh reading to count on from.
    pub fn posts_before(&self, repo: &str, pr: &PrState) -> Option<PostCount> {
        self.entries
            .get(&key(repo, pr.number))
            .filter(|e| e.fingerprint.node_id == pr.fingerprint.node_id)
            .and_then(|e| e.posts)
    }

    /// Settles every PR of one pass of `repo`. `fetched` holds the outcome
    /// of reading each PR that needed it. A PR read successfully is cached
    /// anew. One whose read failed keeps its old entry, so it is tried
    /// again next pass, and shows the cached findings meanwhile (none when
    /// there are none). The rest use their cached findings. Entries of PRs
    /// of `repo` not in `prs` are dropped. Returns the findings, last
    /// rounds and traits per number.
    pub fn settle(
        &mut self,
        repo: &str,
        prs: &[PrState],
        mut fetched: HashMap<u64, Result<Reading, String>>,
        now: DateTime<Utc>,
    ) -> HashMap<u64, Settled> {
        let mut out = HashMap::with_capacity(prs.len());
        for pr in prs {
            let k = key(repo, pr.number);
            match fetched.remove(&pr.number) {
                Some(Ok(reading)) => {
                    let old = self.entries.get(&k);
                    let reviews_read_at = match old {
                        Some(old) if !reading.reviews_fresh => old.reviews_read_at,
                        _ => now,
                    };
                    let posts = reading.posts.or_else(|| {
                        old.filter(|o| o.fingerprint.node_id == pr.fingerprint.node_id)
                            .and_then(|o| o.posts)
                    });
                    self.entries.insert(
                        k.clone(),
                        Entry {
                            fingerprint: pr.fingerprint.clone(),
                            fetched_at: now,
                            listed_at: now,
                            found: reading.found,
                            reviews: reading.reviews,
                            traits: reading.traits,
                            posts,
                            reviews_read_at,
                        },
                    );
                }
                Some(Err(why)) => {
                    let kept = if self.entries.contains_key(&k) {
                        "showing the findings read earlier"
                    } else {
                        "showing none"
                    };
                    log::warn!(
                        "could not read the review findings of {repo}#{}: {why}; {kept}",
                        pr.number
                    );
                }
                None => {}
            }
            let settled = match self.entries.get_mut(&k) {
                Some(entry) => {
                    entry.listed_at = now;
                    (
                        combine(&pr.threads, &entry.found.findings),
                        entry.found.rounds.clone(),
                        entry.traits.clone(),
                        entry.posts.map(|p| p.posts),
                    )
                }
                None => (Vec::new(), GhLastRounds::default(), Vec::new(), None),
            };
            out.insert(pr.number, settled);
        }
        let repo_key = repo.to_ascii_lowercase();
        self.entries.retain(|(r, number), entry| {
            let listed = *r != repo_key || prs.iter().any(|p| p.number == *number);
            listed && now - entry.listed_at < FORGET_AFTER
        });
        out
    }

    /// Moves every entry `by` into the past, as if read that much earlier.
    /// For measuring what a later pass costs (`examples/findings_cost.rs`).
    pub fn age(&mut self, by: TimeDelta) {
        for entry in self.entries.values_mut() {
            entry.fetched_at -= by;
            entry.listed_at -= by;
            entry.reviews_read_at -= by;
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The findings of a PR: one per listed thread of a last round, in thread
/// order, with the thread's fresh resolved flag.
pub(crate) fn combine(
    threads: &[ThreadRef],
    findings: &HashMap<String, (GhFindingReviewer, Verdict)>,
) -> Vec<GhFinding> {
    threads
        .iter()
        .filter_map(|t| {
            findings
                .get(&t.id)
                .map(|&(reviewer, (severity, status))| GhFinding {
                    reviewer,
                    severity,
                    status,
                    thread_resolved: t.resolved,
                })
        })
        .collect()
}

/// The review bot or CodeRabbit, by login.
pub(crate) fn is_counted_reviewer(login: &str) -> bool {
    reviewer_of(login).is_some()
}

/// Which counted reviewer `login` is, if either.
pub(crate) fn reviewer_of(login: &str) -> Option<GhFindingReviewer> {
    if login.eq_ignore_ascii_case(BOT_LOGIN) {
        Some(GhFindingReviewer::ReviewBot)
    } else if login.eq_ignore_ascii_case(coderabbit::LOGIN)
        || login.eq_ignore_ascii_case(coderabbit::REVIEW_AUTHOR)
    {
        Some(GhFindingReviewer::CodeRabbit)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::rounds::Reviews;
    use crate::merge::finding_counts;
    use GhFindingReviewer::*;
    use GhFindingStatus::*;
    use GhSeverity::*;
    use pretty_assertions::assert_eq;

    const REPO: &str = "dashpay/platform";

    fn at(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + TimeDelta::minutes(min)
    }

    fn t(id: &str, resolved: bool) -> ThreadRef {
        ThreadRef {
            id: id.into(),
            resolved,
        }
    }

    fn state(number: u64, threads: &[ThreadRef]) -> PrState {
        let mut ids: Vec<String> = threads.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        PrState {
            number,
            fingerprint: Fingerprint {
                node_id: format!("PR_{number}"),
                updated_at: at(-60),
                head_sha: "aaaa".into(),
                total: u32::try_from(threads.len()).unwrap(),
                thread_ids: ids,
                reviews: vec![],
            },
            threads: threads.to_vec(),
        }
    }

    /// The review bot's findings on the given threads.
    fn verdicts(pairs: &[(&str, Verdict)]) -> Found {
        Found {
            findings: pairs
                .iter()
                .map(|(id, v)| ((*id).to_owned(), (ReviewBot, *v)))
                .collect(),
            rounds: GhLastRounds::default(),
        }
    }

    fn reading(found: Found) -> Reading {
        Reading {
            found,
            reviews: Reviews::default(),
            reviews_fresh: true,
            posts: Some(PostCount::default()),
            traits: vec![],
        }
    }

    fn ok(pairs: &[(u64, Found)]) -> HashMap<u64, Result<Reading, String>> {
        pairs
            .iter()
            .map(|(n, v)| (*n, Ok(reading(v.clone()))))
            .collect()
    }

    /// A cache holding PR 1 (threads A, B; A a blocking finding), read at 0.
    fn filled() -> (FindingCache, PrState) {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false), t("B", false)]);
        cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            ok(&[(1, verdicts(&[("A", (Blocking, Stands))]))]),
            at(0),
        );
        (cache, pr)
    }

    #[test]
    fn nothing_changed_means_no_refetch() {
        let (cache, pr) = filled();
        assert_eq!(cache.refetch(REPO, &pr, at(0)), None);
        assert_eq!(cache.refetch(REPO, &pr, at(4)), None);
        // Resolving a thread is not a reason to read the comments again.
        let mut resolved = pr.clone();
        resolved.threads[0].resolved = true;
        assert_eq!(cache.refetch(REPO, &resolved, at(4)), None);
        // The repo name's casing does not matter.
        assert_eq!(cache.refetch("DashPay/Platform", &pr, at(4)), None);
    }

    #[test]
    fn each_trigger_alone_refetches() {
        let (cache, pr) = filled();
        assert_eq!(
            cache.refetch(REPO, &state(2, &[]), at(1)),
            Some(Refetch::NotCached)
        );
        assert_eq!(
            cache.refetch("other/repo", &pr, at(1)),
            Some(Refetch::NotCached)
        );

        let changed = |f: &dyn Fn(&mut Fingerprint)| {
            let mut p = pr.clone();
            f(&mut p.fingerprint);
            cache.refetch(REPO, &p, at(1))
        };
        assert_eq!(
            changed(&|f| f.updated_at = at(1)),
            Some(Refetch::Changed),
            "updatedAt"
        );
        assert_eq!(
            changed(&|f| f.head_sha = "bbbb".into()),
            Some(Refetch::Changed),
            "head"
        );
        assert_eq!(changed(&|f| f.total = 3), Some(Refetch::Changed), "count");
        assert_eq!(
            changed(&|f| f.thread_ids = vec!["A".into(), "C".into()]),
            Some(Refetch::Changed),
            "same count, other threads"
        );
        assert_eq!(
            changed(&|f| f.node_id = "PR_other".into()),
            Some(Refetch::Changed),
            "node id"
        );
    }

    #[test]
    fn a_new_review_by_either_reviewer_refetches() {
        let (cache, pr) = filled();
        let mut pr = pr;
        let mut gh = crate::merge::tests::gh_pr(REPO, 1);
        gh.merge.node_id = pr.fingerprint.node_id.clone();
        gh.updated_at = pr.fingerprint.updated_at;
        gh.head_sha = pr.fingerprint.head_sha.clone();
        let threads = PrThreads {
            number: 1,
            node_id: gh.merge.node_id.clone(),
            count: crate::github::parse::ThreadCount {
                total: 2,
                resolved: 0,
                unresolved: 2,
            },
            threads: pr.threads.clone(),
            after: None,
        };
        // A person's review is not a reason to read again.
        gh.reviews = vec![review("alice", Some(at(-10)))];
        let s = PrState::new(&gh, &threads);
        assert_eq!(s.fingerprint, pr.fingerprint);
        assert!(!s.reviewed());
        assert_eq!(cache.refetch(REPO, &s, at(1)), None);

        // The bot's latest review, then CodeRabbit's, then a newer one.
        for reviews in [
            vec![review("thepastaclaw", Some(at(-5)))],
            vec![review("coderabbitai", Some(at(-5)))],
            vec![
                review("ThePastaClaw", Some(at(-5))),
                review("coderabbitai", Some(at(-1))),
            ],
        ] {
            gh.reviews = reviews;
            let s = PrState::new(&gh, &threads);
            assert!(s.reviewed());
            assert_eq!(cache.refetch(REPO, &s, at(1)), Some(Refetch::Changed));
            pr = s;
        }
        let mut cache = cache;
        cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            ok(&[(1, Found::default())]),
            at(2),
        );
        assert_eq!(cache.refetch(REPO, &pr, at(3)), None);
    }

    fn review(login: &str, submitted_at: Option<DateTime<Utc>>) -> crate::github::types::GhReview {
        crate::github::types::GhReview {
            login: login.into(),
            avatar_url: None,
            state: crate::github::types::GhReviewState::Commented,
            submitted_at,
            is_bot: login == "coderabbitai",
        }
    }

    #[test]
    fn last_rounds_come_back_with_the_findings() {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false)]);
        let mut found = verdicts(&[("A", (Blocking, Stands))]);
        found.rounds.coderabbit = Some(at(-3));
        let out = cache.settle(REPO, std::slice::from_ref(&pr), ok(&[(1, found)]), at(0));
        assert_eq!(out[&1].1.coderabbit, Some(at(-3)));
        // From the cache too.
        let out = cache.settle(REPO, std::slice::from_ref(&pr), HashMap::new(), at(1));
        assert_eq!(out[&1].1.latest(), Some(at(-3)));
        assert_eq!(out[&1].0.len(), 1);
    }

    #[test]
    fn reviews_are_reused_only_while_nothing_changed() {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false)]);
        let reviews = Reviews {
            review_bot: vec![crate::github::rounds::ReviewInfo {
                database_id: 7,
                submitted_at: Some(at(-10)),
                commit: Some("aaaa".into()),
                body: "<!-- thepastaclaw-review v1 -->".into(),
            }],
            coderabbit: vec![],
        };
        let fresh = Reading {
            found: Found::default(),
            reviews: reviews.clone(),
            reviews_fresh: true,
            posts: Some(PostCount::default()),
            traits: vec![],
        };
        cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Ok(fresh))].into(),
            at(0),
        );
        assert_eq!(
            cache.known(REPO, &pr, at(5)).map(|k| k.reviews),
            Some(reviews.clone())
        );
        assert_eq!(cache.known(REPO, &state(2, &[]), at(5)), None);
        let mut changed = pr.clone();
        changed.fingerprint.reviews = vec![("thepastaclaw".into(), Some(at(4)))];
        assert_eq!(cache.known(REPO, &changed, at(5)), None);

        // Re-read at 5 and 10 with the reused reviews: their age still
        // counts from 0, so at 30 they are read again.
        for minute in [5, 10] {
            let reused = Reading {
                found: Found::default(),
                reviews: reviews.clone(),
                reviews_fresh: false,
                posts: Some(PostCount::default()),
                traits: vec![],
            };
            cache.settle(
                REPO,
                std::slice::from_ref(&pr),
                [(1, Ok(reused))].into(),
                at(minute),
            );
        }
        assert!(cache.known(REPO, &pr, at(29)).is_some());
        assert_eq!(cache.known(REPO, &pr, at(30)), None);
    }

    #[test]
    fn round_commits_are_cached_and_judged_against_the_fresh_head() {
        // The bot's round on "aaaa", the head at the time; A stands and is
        // resolved by hand.
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", true)]);
        let mut found = verdicts(&[("A", (Blocking, Stands))]);
        found.rounds.review_bot = Some(at(-10));
        found.rounds.review_bot_commit = Some("aaaa".into());
        let out = cache.settle(REPO, std::slice::from_ref(&pr), ok(&[(1, found)]), at(0));
        let (findings, rounds, _, _) = &out[&1];
        assert_eq!(rounds.review_bot_commit.as_deref(), Some("aaaa"));
        let c = finding_counts(findings, rounds, "aaaa");
        assert_eq!((c.blocking.not_agreed, c.blocking.awaiting), (1, 0));

        // From the cache, the round keeps its commit.
        let out = cache.settle(REPO, std::slice::from_ref(&pr), HashMap::new(), at(1));
        assert_eq!(out[&1].1.review_bot_commit.as_deref(), Some("aaaa"));

        // The author pushes "bbbb". The read that follows fails, so the
        // cached reading is shown; judged against the new head, the finding
        // awaits the re-review.
        let mut pushed = pr.clone();
        pushed.fingerprint.head_sha = "bbbb".into();
        assert_eq!(cache.refetch(REPO, &pushed, at(2)), Some(Refetch::Changed));
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pushed),
            [(1, Err("HTTP 502".to_owned()))].into(),
            at(2),
        );
        let (findings, rounds, _, _) = &out[&1];
        let c = finding_counts(findings, rounds, &pushed.fingerprint.head_sha);
        assert_eq!((c.blocking.not_agreed, c.blocking.awaiting), (0, 1));
    }

    #[test]
    fn verdicts_expire_after_five_minutes() {
        let (cache, pr) = filled();
        let just_before = at(5) - TimeDelta::seconds(1);
        assert_eq!(cache.refetch(REPO, &pr, just_before), None);
        assert_eq!(cache.refetch(REPO, &pr, at(5)), Some(Refetch::Expired));
        assert_eq!(cache.refetch(REPO, &pr, at(90)), Some(Refetch::Expired));
    }

    #[test]
    fn cached_verdicts_take_fresh_resolved_flags() {
        let (mut cache, pr) = filled();
        let before = cache.settle(REPO, std::slice::from_ref(&pr), HashMap::new(), at(1));
        let head = &pr.fingerprint.head_sha;
        assert_eq!(
            finding_counts(&before[&1].0, &before[&1].1, head)
                .blocking
                .not_agreed,
            0
        );
        assert_eq!(
            before[&1].0,
            vec![GhFinding {
                reviewer: ReviewBot,
                severity: Blocking,
                status: Stands,
                thread_resolved: false
            }]
        );

        // Thread A was resolved on GitHub since: no refetch, and the finding
        // goes from open to awaiting (the round's commit is not known).
        let mut now_resolved = pr.clone();
        now_resolved.threads[0].resolved = true;
        assert_eq!(cache.refetch(REPO, &now_resolved, at(2)), None);
        let after = cache.settle(REPO, &[now_resolved], HashMap::new(), at(2));
        let counts = finding_counts(&after[&1].0, &after[&1].1, head);
        assert_eq!(
            (
                counts.blocking.total,
                counts.blocking.not_agreed,
                counts.blocking.awaiting
            ),
            (1, 0, 1)
        );
        assert!(after[&1].0[0].thread_resolved);
    }

    #[test]
    fn combine_keeps_thread_order_and_skips_other_threads() {
        let mut v = verdicts(&[
            ("C", (Nitpick, Deferred)),
            ("A", (Suggestion, Resolved)),
            ("gone", (Blocking, Stands)),
        ])
        .findings;
        v.insert("C".into(), (CodeRabbit, (Nitpick, Deferred)));
        let found = combine(&[t("A", true), t("B", false), t("C", false)], &v);
        assert_eq!(
            found,
            vec![
                GhFinding {
                    reviewer: ReviewBot,
                    severity: Suggestion,
                    status: Resolved,
                    thread_resolved: true
                },
                GhFinding {
                    reviewer: CodeRabbit,
                    severity: Nitpick,
                    status: Deferred,
                    thread_resolved: false
                },
            ]
        );
    }

    #[test]
    fn a_refetch_replaces_the_entry() {
        let (mut cache, pr) = filled();
        let mut pushed = pr.clone();
        pushed.fingerprint.head_sha = "bbbb".into();
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pushed),
            ok(&[(1, verdicts(&[("A", (Blocking, Resolved))]))]),
            at(3),
        );
        assert_eq!(out[&1].0[0].status, Resolved);
        assert_eq!(cache.refetch(REPO, &pushed, at(3)), None);
        assert_eq!(cache.refetch(REPO, &pushed, at(8)), Some(Refetch::Expired));
    }

    #[test]
    fn prs_no_longer_listed_are_dropped() {
        let mut cache = FindingCache::default();
        let one = state(1, &[t("A", false)]);
        let two = state(2, &[t("B", false)]);
        let v = verdicts(&[("A", (Blocking, Stands)), ("B", (Nitpick, Stands))]);
        cache.settle(
            REPO,
            &[one.clone(), two.clone()],
            ok(&[(1, v.clone()), (2, v.clone())]),
            at(0),
        );
        let other = state(7, &[t("Z", false)]);
        cache.settle("o/r", std::slice::from_ref(&other), ok(&[(7, v)]), at(0));
        assert_eq!(cache.len(), 3);

        // PR 2 merged: gone from the next pass of its repo. The other
        // repo's entry is untouched.
        cache.settle(REPO, std::slice::from_ref(&one), HashMap::new(), at(1));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.refetch(REPO, &two, at(1)), Some(Refetch::NotCached));
        assert_eq!(cache.refetch("o/r", &other, at(1)), None);

        // A repo not fetched for an hour (removed from the projects) is
        // forgotten on the next pass of any repo.
        cache.settle(REPO, std::slice::from_ref(&one), HashMap::new(), at(61));
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.refetch("o/r", &other, at(61)),
            Some(Refetch::NotCached)
        );
    }

    #[test]
    fn a_failed_read_falls_back_to_the_cache_or_to_nothing() {
        let (mut cache, pr) = filled();
        let fresh = state(2, &[t("X", false)]);
        let fetched: HashMap<u64, Result<Reading, String>> = [
            (1, Err("HTTP 502".to_owned())),
            (2, Err("HTTP 502".to_owned())),
        ]
        .into_iter()
        .collect();
        // PR 1 expired and its read failed; PR 2 was never read.
        let out = cache.settle(REPO, &[pr.clone(), fresh.clone()], fetched, at(6));
        assert_eq!(out[&1].0.len(), 1, "cached findings are kept");
        assert_eq!(out[&1].0[0].severity, Blocking);
        assert_eq!(out[&2].0, vec![], "nothing cached: none shown");
        // Both are tried again next pass.
        assert_eq!(cache.refetch(REPO, &pr, at(7)), Some(Refetch::Expired));
        assert_eq!(cache.refetch(REPO, &fresh, at(7)), Some(Refetch::NotCached));
    }

    #[test]
    fn findings_never_cross_prs() {
        // Two PRs sharing a thread id (never on GitHub, but it must not
        // matter): each shows only its own verdicts.
        let mut cache = FindingCache::default();
        let one = state(1, &[t("A", false)]);
        let two = state(2, &[t("A", false)]);
        let out = cache.settle(
            REPO,
            &[one.clone(), two.clone()],
            ok(&[
                (1, verdicts(&[("A", (Blocking, Stands))])),
                (2, Found::default()),
            ]),
            at(0),
        );
        assert_eq!(out[&1].0.len(), 1);
        assert_eq!(out[&2].0, vec![]);
    }

    fn counted(posts: u32, reviews: u32, newest: u64) -> PostCount {
        PostCount {
            posts,
            reviews,
            newest: Some(newest),
        }
    }

    #[test]
    fn the_post_count_is_kept_reused_and_handed_on() {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false)]);
        // Never read: no count, and nothing to count on from.
        let out = cache.settle(REPO, std::slice::from_ref(&pr), HashMap::new(), at(0));
        assert_eq!(out[&1].3, None);
        assert_eq!(cache.posts_before(REPO, &pr), None);

        let first = Reading {
            posts: Some(counted(7, 27, 900)),
            ..reading(Found::default())
        };
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Ok(first))].into(),
            at(0),
        );
        assert_eq!(out[&1].3, Some(7));
        // From the cache.
        let out = cache.settle(REPO, std::slice::from_ref(&pr), HashMap::new(), at(1));
        assert_eq!(out[&1].3, Some(7));
        // Reused with the reviews on a time-only re-read.
        let known = cache.known(REPO, &pr, at(6)).unwrap();
        assert_eq!(known.posts, counted(7, 27, 900));
        let reused = Reading {
            reviews_fresh: false,
            posts: Some(known.posts),
            ..reading(Found::default())
        };
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Ok(reused))].into(),
            at(6),
        );
        assert_eq!(out[&1].3, Some(7));
        // Handed to a fresh reading of the same PR to count on from, even
        // when the PR changed; never to another node under that number.
        let mut pushed = pr.clone();
        pushed.fingerprint.head_sha = "bbbb".into();
        assert_eq!(cache.posts_before(REPO, &pushed), Some(counted(7, 27, 900)));
        let mut other = pr.clone();
        other.fingerprint.node_id = "PR_other".into();
        assert_eq!(cache.posts_before(REPO, &other), None);
    }

    #[test]
    fn a_failed_or_countless_reading_keeps_the_post_count() {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false)]);
        let first = Reading {
            posts: Some(counted(4, 9, 50)),
            ..reading(Found::default())
        };
        cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Ok(first))].into(),
            at(0),
        );
        let mut pushed = pr.clone();
        pushed.fingerprint.head_sha = "bbbb".into();
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pushed),
            [(1, Err("HTTP 502".to_owned()))].into(),
            at(1),
        );
        assert_eq!(out[&1].3, Some(4));
        assert_eq!(cache.posts_before(REPO, &pushed), Some(counted(4, 9, 50)));
        // A reading that counted nothing (no node id to ask with) keeps it.
        let countless = Reading {
            posts: None,
            ..reading(Found::default())
        };
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pushed),
            [(1, Ok(countless.clone()))].into(),
            at(2),
        );
        assert_eq!(out[&1].3, Some(4));
        // But not for another node under that number.
        let mut other = pushed.clone();
        other.fingerprint.node_id = "PR_other".into();
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&other),
            [(1, Ok(countless))].into(),
            at(3),
        );
        assert_eq!(out[&1].3, None);
        // Without a count the reviews are not reused, so they and the
        // count are read afresh.
        assert_eq!(cache.known(REPO, &other, at(4)), None);
    }

    #[test]
    fn traits_come_back_with_the_findings_and_survive_a_failed_read() {
        let mut cache = FindingCache::default();
        let pr = state(1, &[t("A", false)]);
        let grove = PrTrait {
            kind: crate::model::TraitKind::GrovedbChange,
            summary: "5 nodes changed".into(),
            links: vec![],
        };
        let first = Reading {
            traits: vec![grove.clone()],
            ..reading(Found::default())
        };
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Ok(first))].into(),
            at(0),
        );
        assert_eq!(out[&1].2, vec![grove.clone()]);
        // Reused with the reviews while nothing changed.
        assert_eq!(
            cache.known(REPO, &pr, at(6)).unwrap().traits,
            vec![grove.clone()]
        );
        // A failed read shows the cached traits.
        let out = cache.settle(
            REPO,
            std::slice::from_ref(&pr),
            [(1, Err("HTTP 502".to_owned()))].into(),
            at(6),
        );
        assert_eq!(out[&1].2, vec![grove]);
        // A PR never read has none.
        let out = cache.settle(REPO, &[state(2, &[])], HashMap::new(), at(7));
        assert_eq!(out[&2].2, vec![]);
    }
}
