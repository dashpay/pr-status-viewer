//! Picks each reviewer's last review round on a PR, and the review threads
//! that belong to it.
//!
//! The review bot (`model::BOT_LOGIN`):
//! - A round is one of its reviews whose body carries `ROUND_MARKER`. Its
//!   other reviews have an empty body: they are replies in threads.
//! - The round's findings are the threads it opened (the thread's first
//!   comment belongs to the round's review) plus the threads its body links
//!   to as carried from earlier rounds ("Still open from earlier rounds",
//!   "Disputed", ...): every `#discussion_r<id>` link outside `<details>`
//!   blocks, `<id>` being the database id of the thread's first comment.
//!   Links inside `<details>` (follow-up candidates, the agent prompt) do
//!   not count.
//!
//! CodeRabbit (`coderabbit::LOGIN`):
//! - Its full reviews have a body; its replies come as reviews with an
//!   empty body. On real data every review that opened a thread had a body,
//!   but some full reviews open no thread (their findings sit outside the
//!   diff, in the body). A round is therefore a review with a body, or one
//!   that opened a thread.
//! - The round's findings are the threads it opened. CodeRabbit does not
//!   carry earlier findings forward.
//!
//! Each round keeps the commit its review was submitted on, which `merge`
//! compares with the PR's current head (`merge::finding_counts`).
//!
//! Only the reviews read are considered (`findings_query::REVIEWS_READ` per
//! reviewer, newest last), so a round older than that many later reviews
//! by the same reviewer is not seen.
//!
//! The PR's round number (`PostCount`) is how many review posts the bot has
//! made on it: its reviews that carry `ROUND_MARKER`. Its replies in threads
//! (empty bodies) and its "CI failed after approval" follow-ups (bodies that
//! open with `<!-- thepastaclaw-review-update v1 ...>` or
//! `<!-- thepastaclaw-ci-update v1 ...>`) are reviews too, and are not
//! counted. `count_posts` counts from the reviews read with the findings
//! when they reach back far enough, or on from an earlier count; otherwise
//! every review of the bot must be read (`findings_query`).

use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::finding_cache::{Found, Verdict};
use super::types::GhFindingReviewer;

/// Opens the body of each of the review bot's rounds.
const ROUND_MARKER: &str = "<!-- thepastaclaw-review v1 -->";
/// Precedes the comment id in a link to a review thread.
const DISCUSSION_ANCHOR: &str = "#discussion_r";

/// How many review posts the review bot has made on a PR, counted through
/// one of its reviews.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PostCount {
    /// Its reviews that carry `ROUND_MARKER`.
    pub posts: u32,
    /// Its reviews counted through, replies and follow-ups included.
    pub reviews: u32,
    /// Database id of the newest review counted, if known; a later count
    /// goes on from it.
    pub newest: Option<u64>,
}

impl PostCount {
    /// Adds the next of the bot's reviews, in GitHub's order.
    pub fn add(&mut self, database_id: Option<u64>, body: &str) {
        self.reviews = self.reviews.saturating_add(1);
        if is_post(body) {
            self.posts = self.posts.saturating_add(1);
        }
        self.newest = database_id.or(self.newest);
    }
}

/// Whether a review body by the bot is one of its review posts.
pub(crate) fn is_post(body: &str) -> bool {
    body.contains(ROUND_MARKER)
}

/// The bot's post count from its latest reviews `window` (oldest first)
/// and `total`, the number of reviews it has on the PR (GitHub's
/// `totalCount`, read with the window). Exact when the window holds every
/// review; otherwise counted on from `before`, an earlier count, when its
/// newest review is in the window and the reviews after it make up the
/// total. `None` when neither holds: every review must then be read.
pub(crate) fn count_posts(
    window: &[ReviewInfo],
    total: u32,
    before: Option<PostCount>,
) -> Option<PostCount> {
    let held = u32::try_from(window.len()).unwrap_or(u32::MAX);
    if total <= held {
        let mut count = PostCount::default();
        for r in window {
            count.add(Some(r.database_id), &r.body);
        }
        return Some(count);
    }
    let before = before?;
    let anchor = before.newest?;
    let at = window.iter().position(|r| r.database_id == anchor)?;
    let after = &window[at + 1..];
    let added = u32::try_from(after.len()).ok()?;
    if before.reviews.checked_add(added) != Some(total) {
        return None;
    }
    let mut count = before;
    for r in after {
        count.add(Some(r.database_id), &r.body);
    }
    Some(count)
}

/// One review by one of the reviewers, as the findings query reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewInfo {
    pub database_id: u64,
    /// `None` for a review not yet submitted; such a review is no round.
    pub submitted_at: Option<DateTime<Utc>>,
    /// The commit the review was submitted on (`commit.oid`); `None` when
    /// GitHub does not report it.
    pub commit: Option<String>,
    pub body: String,
}

/// The latest reviews of each reviewer, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reviews {
    pub review_bot: Vec<ReviewInfo>,
    pub coderabbit: Vec<ReviewInfo>,
}

/// One review thread, as the findings query reads its first comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadInfo {
    pub id: String,
    /// Database id of the first comment: what `#discussion_r<id>` names.
    pub root_id: Option<u64>,
    /// Database id of the review the first comment was posted with.
    pub root_review: Option<u64>,
    /// Which of the two reviewers wrote the first comment, if either.
    pub author: Option<GhFindingReviewer>,
    /// The finding the first comment states, if it is one.
    pub verdict: Option<Verdict>,
}

/// The last-round findings and round times of one PR.
pub(crate) fn last_rounds(
    review_bot: &[ReviewInfo],
    coderabbit: &[ReviewInfo],
    threads: &[ThreadInfo],
) -> Found {
    let mut found = Found::default();

    if let Some(round) = latest(review_bot.iter().filter(|r| is_post(&r.body))) {
        found.rounds.review_bot = round.submitted_at;
        found.rounds.review_bot_commit.clone_from(&round.commit);
        let carried = linked_comment_ids(&round.body);
        for t in threads {
            let new = t.root_review == Some(round.database_id);
            let linked = t.root_id.is_some_and(|id| carried.contains(&id));
            if new || linked {
                add(&mut found, t, GhFindingReviewer::ReviewBot);
            }
        }
    }

    let opened: HashSet<u64> = threads
        .iter()
        .filter(|t| t.author == Some(GhFindingReviewer::CodeRabbit))
        .filter_map(|t| t.root_review)
        .collect();
    let rounds = coderabbit
        .iter()
        .filter(|r| !r.body.trim().is_empty() || opened.contains(&r.database_id));
    if let Some(round) = latest(rounds) {
        found.rounds.coderabbit = round.submitted_at;
        found.rounds.coderabbit_commit.clone_from(&round.commit);
        for t in threads
            .iter()
            .filter(|t| t.root_review == Some(round.database_id))
        {
            add(&mut found, t, GhFindingReviewer::CodeRabbit);
        }
    }
    found
}

/// Records `t` as `reviewer`'s finding when that reviewer opened it with a
/// finding.
fn add(found: &mut Found, t: &ThreadInfo, reviewer: GhFindingReviewer) {
    if t.author != Some(reviewer) {
        return;
    }
    if let Some(verdict) = t.verdict {
        found.findings.insert(t.id.clone(), (reviewer, verdict));
    }
}

/// The most recently submitted review; on a tie, the later id.
fn latest<'a>(reviews: impl Iterator<Item = &'a ReviewInfo>) -> Option<&'a ReviewInfo> {
    reviews
        .filter(|r| r.submitted_at.is_some())
        .max_by_key(|r| (r.submitted_at, r.database_id))
}

/// The comment ids of every `#discussion_r<id>` link outside `<details>`
/// blocks (nested ones included).
fn linked_comment_ids(body: &str) -> HashSet<u64> {
    let mut ids = HashSet::new();
    for text in outside_details(body) {
        for (i, _) in text.match_indices(DISCUSSION_ANCHOR) {
            let digits: String = text[i + DISCUSSION_ANCHOR.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(id) = digits.parse() {
                ids.insert(id);
            }
        }
    }
    ids
}

/// The stretches of `body` not inside any `<details>` element. An unclosed
/// `<details>` hides the rest of the body.
fn outside_details(body: &str) -> Vec<&str> {
    const OPEN: &str = "<details";
    const CLOSE: &str = "</details>";
    let lower = body.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut pos = 0usize;
    while pos < lower.len() {
        let next_open = lower[pos..].find(OPEN).map(|i| i + pos);
        let next_close = lower[pos..].find(CLOSE).map(|i| i + pos);
        let (at, opening) = match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => (o, true),
            (Some(o), None) => (o, true),
            (_, Some(c)) => (c, false),
            (None, None) => break,
        };
        if opening {
            if depth == 0 {
                out.push(&body[start..at]);
            }
            depth += 1;
            pos = at + OPEN.len();
        } else {
            pos = at + CLOSE.len();
            if depth > 0 {
                depth -= 1;
                if depth == 0 {
                    start = pos;
                }
            }
        }
    }
    if depth == 0 && start < body.len() {
        out.push(&body[start..]);
    }
    out
}

/// Thread ids per reviewer, for tests and diagnostics.
#[cfg(test)]
pub(crate) fn by_reviewer(
    found: &Found,
) -> std::collections::HashMap<GhFindingReviewer, Vec<String>> {
    let mut out: std::collections::HashMap<GhFindingReviewer, Vec<String>> =
        std::collections::HashMap::new();
    for (id, (reviewer, _)) in &found.findings {
        out.entry(*reviewer).or_default().push(id.clone());
    }
    for ids in out.values_mut() {
        ids.sort();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::types::{GhFindingStatus::*, GhLastRounds, GhSeverity::*};
    use GhFindingReviewer::*;
    use pretty_assertions::assert_eq;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn review(id: u64, when: &str, body: &str) -> ReviewInfo {
        ReviewInfo {
            database_id: id,
            submitted_at: Some(at(when)),
            commit: Some(format!("c{id}")),
            body: body.into(),
        }
    }

    fn round_body(rest: &str) -> String {
        format!(
            "{ROUND_MARKER}\n<!-- thepastaclaw-review-phase v1 phase=final sha=abc -->\n## Review\n\n{rest}"
        )
    }

    fn thread(
        id: &str,
        root_id: u64,
        review: u64,
        author: Option<GhFindingReviewer>,
        verdict: Option<Verdict>,
    ) -> ThreadInfo {
        ThreadInfo {
            id: id.into(),
            root_id: Some(root_id),
            root_review: Some(review),
            author,
            verdict,
        }
    }

    fn bot(id: &str, root_id: u64, review: u64) -> ThreadInfo {
        thread(
            id,
            root_id,
            review,
            Some(ReviewBot),
            Some((Blocking, Stands)),
        )
    }

    fn rabbit(id: &str, root_id: u64, review: u64) -> ThreadInfo {
        thread(
            id,
            root_id,
            review,
            Some(CodeRabbit),
            Some((Suggestion, Stands)),
        )
    }

    fn ids(found: &Found, who: GhFindingReviewer) -> Vec<String> {
        by_reviewer(found).remove(&who).unwrap_or_default()
    }

    #[test]
    fn the_last_bot_round_wins_over_later_replies() {
        let reviews = [
            review(1, "2026-11-21T10:00:00Z", &round_body("first")),
            review(2, "2026-11-21T12:00:00Z", &round_body("second")),
            // Replies in threads: empty bodies, later than the round.
            review(3, "2026-11-21T13:00:00Z", ""),
            review(4, "2026-11-21T14:00:00Z", ""),
        ];
        let threads = [bot("A", 10, 1), bot("B", 20, 2), bot("C", 30, 3)];
        let found = last_rounds(&reviews, &[], &threads);
        assert_eq!(found.rounds.review_bot, Some(at("2026-11-21T12:00:00Z")));
        assert_eq!(found.rounds.review_bot_commit.as_deref(), Some("c2"));
        assert_eq!(found.rounds.coderabbit, None);
        assert_eq!(found.rounds.coderabbit_commit, None);
        // A, from the first round and not linked, is not counted. C was
        // posted with a reply review: not a round.
        assert_eq!(ids(&found, ReviewBot), vec!["B"]);
    }

    #[test]
    fn carried_threads_are_linked_outside_details() {
        let body = round_body(
            "### Still open from earlier rounds (1)\n\n- 🔴 **Blocking**: x ([thread](https://github.com/o/r/pull/1#discussion_r10))\n\n### Disputed — left to maintainers (1)\n\n- 🔴 **Blocking**: y ([thread](https://github.com/o/r/pull/1#discussion_r20))\n\n<details>\n<summary>Follow-up candidates (1)</summary>\n\nSee https://github.com/o/r/pull/1#discussion_r30\n<details><summary>nested</summary>#discussion_r40</details>\nstill inside #discussion_r50\n</details>\n\nAfter: ([thread](https://github.com/o/r/pull/1#discussion_r60))",
        );
        let reviews = [
            review(1, "2026-11-21T10:00:00Z", &round_body("first")),
            review(2, "2026-11-21T12:00:00Z", &body),
        ];
        let threads = [
            bot("carried-open", 10, 1),
            bot("carried-disputed", 20, 1),
            bot("in-details", 30, 1),
            bot("nested", 40, 1),
            bot("deeper", 50, 1),
            bot("after-details", 60, 1),
            bot("old", 70, 1),
            bot("new", 80, 2),
        ];
        let found = last_rounds(&reviews, &[], &threads);
        assert_eq!(
            ids(&found, ReviewBot),
            vec!["after-details", "carried-disputed", "carried-open", "new"]
        );
    }

    #[test]
    fn links_to_threads_that_are_not_the_bots_findings_do_not_count() {
        let body = round_body("([thread](#discussion_r10)) ([thread](#discussion_r20))");
        let reviews = [review(2, "2026-11-21T12:00:00Z", &body)];
        let threads = [thread("person", 10, 1, None, None), rabbit("rabbit", 20, 7)];
        assert!(last_rounds(&reviews, &[], &threads).findings.is_empty());
    }

    #[test]
    fn a_round_without_findings_counts_nothing_but_has_a_time() {
        let reviews = [
            review(1, "2026-11-21T10:00:00Z", &round_body("first")),
            review(2, "2026-11-21T12:00:00Z", &round_body("**Approved**")),
        ];
        let found = last_rounds(&reviews, &[], &[bot("A", 10, 1)]);
        assert!(found.findings.is_empty());
        assert_eq!(found.rounds.review_bot, Some(at("2026-11-21T12:00:00Z")));
    }

    #[test]
    fn never_reviewed() {
        let found = last_rounds(&[], &[], &[thread("person", 1, 1, None, None)]);
        assert_eq!(found, Found::default());
        // Only replies, or a review never submitted: no round either.
        let reviews = [
            review(1, "2026-11-21T10:00:00Z", ""),
            ReviewInfo {
                database_id: 2,
                submitted_at: None,
                commit: None,
                body: round_body("pending"),
            },
        ];
        assert_eq!(last_rounds(&reviews, &[], &[]), Found::default());
    }

    #[test]
    fn the_last_coderabbit_round_among_replies() {
        let reviews = [
            review(
                1,
                "2026-11-20T21:17:11Z",
                "**Actionable comments posted: 2**",
            ),
            review(
                2,
                "2026-11-21T16:22:12Z",
                "**Actionable comments posted: 2**",
            ),
            review(3, "2026-11-21T16:26:10Z", ""),
            review(4, "2026-11-21T18:41:43Z", ""),
        ];
        let threads = [
            rabbit("R1a", 11, 1),
            rabbit("R1b", 12, 1),
            rabbit("R2a", 21, 2),
            rabbit("R2b", 22, 2),
            // A review that opened a thread is a round, even without a
            // body.
            thread("reply", 31, 3, Some(CodeRabbit), None),
        ];
        let found = last_rounds(&[], &reviews, &threads[..4]);
        assert_eq!(found.rounds.coderabbit, Some(at("2026-11-21T16:22:12Z")));
        assert_eq!(found.rounds.coderabbit_commit.as_deref(), Some("c2"));
        assert_eq!(ids(&found, CodeRabbit), vec!["R2a", "R2b"]);

        let found = last_rounds(&[], &reviews, &threads);
        assert_eq!(found.rounds.coderabbit, Some(at("2026-11-21T16:26:10Z")));
        assert_eq!(found.rounds.coderabbit_commit.as_deref(), Some("c3"));
        assert!(found.findings.is_empty());
    }

    #[test]
    fn a_coderabbit_round_with_findings_only_in_its_body() {
        let reviews = [
            review(
                1,
                "2026-11-21T10:00:00Z",
                "**Actionable comments posted: 1**",
            ),
            review(
                2,
                "2026-11-21T12:00:00Z",
                "> [!CAUTION]\n> Outside diff range comments (1)",
            ),
        ];
        let found = last_rounds(&[], &reviews, &[rabbit("R1", 11, 1)]);
        assert_eq!(found.rounds.coderabbit, Some(at("2026-11-21T12:00:00Z")));
        assert!(found.findings.is_empty());
    }

    #[test]
    fn both_reviewers() {
        let bot_reviews = [review(5, "2026-11-21T12:00:00Z", &round_body("x"))];
        let rabbit_reviews = [review(
            6,
            "2026-11-21T13:00:00Z",
            "**Actionable comments posted: 1**",
        )];
        let threads = [
            bot("B", 50, 5),
            rabbit("R", 60, 6),
            thread("person", 70, 6, None, None),
        ];
        let found = last_rounds(&bot_reviews, &rabbit_reviews, &threads);
        assert_eq!(ids(&found, ReviewBot), vec!["B"]);
        assert_eq!(ids(&found, CodeRabbit), vec!["R"]);
        assert_eq!(
            found.rounds,
            GhLastRounds {
                review_bot: Some(at("2026-11-21T12:00:00Z")),
                coderabbit: Some(at("2026-11-21T13:00:00Z")),
                review_bot_commit: Some("c5".into()),
                coderabbit_commit: Some("c6".into()),
            }
        );
    }

    #[test]
    fn a_round_without_a_known_commit_has_none() {
        let mut bot_round = review(5, "2026-11-21T12:00:00Z", &round_body("x"));
        bot_round.commit = None;
        // An older round on a known commit does not lend it its commit.
        let reviews = [
            review(4, "2026-11-21T10:00:00Z", &round_body("y")),
            bot_round,
        ];
        let found = last_rounds(&reviews, &[], &[]);
        assert_eq!(found.rounds.review_bot, Some(at("2026-11-21T12:00:00Z")));
        assert_eq!(found.rounds.review_bot_commit, None);
    }

    /// The bodies the bot gives its reviews, as on dashpay/platform.
    const REPLY: &str = "";
    const CI_UPDATE: &str =
        "<!-- thepastaclaw-ci-update v1 sha=abc -->\n**CI failed after approval**";
    const REVIEW_UPDATE: &str =
        "<!-- thepastaclaw-review-update v1 sha=abc -->\nThe approval no longer holds.";

    /// Bot reviews 1..=n: posts at the ids in `posts`, replies elsewhere.
    fn bot_reviews(ids: std::ops::RangeInclusive<u64>, posts: &[u64]) -> Vec<ReviewInfo> {
        ids.map(|id| {
            let body = if posts.contains(&id) {
                round_body("x")
            } else {
                REPLY.to_owned()
            };
            review(id, "2026-11-21T10:00:00Z", &body)
        })
        .collect()
    }

    fn counted(posts: u32, reviews: u32, newest: u64) -> Option<PostCount> {
        Some(PostCount {
            posts,
            reviews,
            newest: Some(newest),
        })
    }

    #[test]
    fn only_review_posts_count_as_rounds() {
        let reviews = [
            review(1, "2026-11-21T10:00:00Z", &round_body("first")),
            review(2, "2026-11-21T10:10:00Z", REPLY),
            review(3, "2026-11-21T10:20:00Z", "   "),
            review(4, "2026-11-21T11:00:00Z", CI_UPDATE),
            review(5, "2026-11-21T11:01:00Z", REVIEW_UPDATE),
            review(6, "2026-11-21T12:00:00Z", &round_body("second")),
            // Two posts on one commit count twice.
            review(7, "2026-11-21T12:30:00Z", &round_body("third")),
        ];
        assert_eq!(count_posts(&reviews, 7, None), counted(3, 7, 7));
        assert!(!is_post(CI_UPDATE));
        assert!(!is_post(REVIEW_UPDATE));
        assert!(!is_post("<!-- thepastaclaw-review v2 -->"));
        // Another author's reviews never reach this count: the query asks
        // for the bot's only (`author:`).
    }

    #[test]
    fn a_window_holding_every_review_is_exact() {
        assert_eq!(count_posts(&[], 0, None), Some(PostCount::default()));
        let w = bot_reviews(1..=4, &[1, 3]);
        assert_eq!(count_posts(&w, 4, None), counted(2, 4, 4));
        // An earlier count is not needed, and a wrong one is ignored.
        assert_eq!(count_posts(&w, 4, counted(9, 9, 99)), counted(2, 4, 4));
    }

    #[test]
    fn a_window_short_of_the_total_needs_an_earlier_count() {
        // 30 reviews; the window holds the last 10 (21..=30).
        let w = bot_reviews(21..=30, &[22, 25, 30]);
        assert_eq!(count_posts(&w, 30, None), None);
        // Counted through review 27 before: 12 posts in 27 reviews.
        assert_eq!(
            count_posts(&w, 30, counted(12, 27, 27)),
            counted(13, 30, 30)
        );
        // Nothing new since.
        assert_eq!(
            count_posts(&w, 30, counted(14, 30, 30)),
            counted(14, 30, 30)
        );
    }

    #[test]
    fn a_gap_means_every_review_must_be_read() {
        let w = bot_reviews(21..=30, &[22, 25, 30]);
        // The earlier count's newest review is older than the window.
        assert_eq!(count_posts(&w, 30, counted(5, 15, 15)), None);
        // It is in the window, but the reviews do not add up to the total.
        assert_eq!(count_posts(&w, 31, counted(12, 27, 27)), None);
        assert_eq!(count_posts(&w, 30, counted(12, 26, 27)), None);
        // An earlier count without a newest review.
        let none = PostCount {
            posts: 1,
            reviews: 1,
            newest: None,
        };
        assert_eq!(count_posts(&w, 30, Some(none)), None);
    }

    #[test]
    fn counting_review_by_review() {
        let mut c = PostCount::default();
        c.add(Some(1), &round_body("x"));
        c.add(None, REPLY);
        assert_eq!(c, counted(1, 2, 1).unwrap());
        c.add(Some(3), CI_UPDATE);
        assert_eq!(c, counted(1, 3, 3).unwrap());
    }

    #[test]
    fn details_are_skipped_whatever_their_case_and_nesting() {
        assert_eq!(outside_details("a<details>b</details>c"), vec!["a", "c"]);
        assert_eq!(
            outside_details("a<DETAILS open>b<details>c</details>d</Details>e"),
            vec!["a", "e"]
        );
        assert_eq!(outside_details("a<details>never closed"), vec!["a"]);
        assert_eq!(outside_details("a</details>b"), vec!["a</details>b"]);
        assert_eq!(outside_details(""), Vec::<&str>::new());
    }
}
