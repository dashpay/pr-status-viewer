//! The PR Hygiene gate's self-review: whoever holds a PR (its author, or
//! anyone it is assigned to) states that they have read the final diff by
//! commenting `/self-reviewed <head commit>`.
//!
//! One place decides both what the snapshot shows (`self_review`) and what a
//! fresh check allows (`self_review_check`), and builds the one comment the
//! app ever posts (`self_review_comment`). Pure.
//!
//! The gate's rules, from dashpay/stale_prs_are_bad `pr_review/policy.py`:
//! the whole comment, trimmed, must match
//! `/self[- ]?review(?:ed)?(?:\s+(?P<head>[0-9a-fA-F]{40}))?` ignoring case
//! (`attestation` reads it the same way); an edited comment does not count;
//! only the author and the assignees count, never a bot; and a push that
//! changes the diff takes it back.

use chrono::{DateTime, Utc};

use super::next_step::next_step;
use crate::error::{AppError, AppResult};
use crate::github::types::{GhHygiene, GhPullRequest, GhSelfReviewCheck};
use crate::model::{SelfReview, SelfReviewCheck, SelfReviewState};

/// What the comment starts with.
pub const SELF_REVIEW_COMMAND: &str = "/self-reviewed";

/// Refusals, as the confirmation shows them.
pub const SELF_REVIEW_CLOSED: &str = "This pull request is closed.";
pub const SELF_REVIEW_DRAFT: &str = "Drafts are not reviewed. Mark it ready for review first.";
pub const SELF_REVIEW_NOT_HOLDING: &str = "Only the author or an assignee can post the self-review. Assign yourself on GitHub to take this pull request over.";
pub const SELF_REVIEW_NO_HEAD: &str =
    "GitHub did not report the pull request's head commit. Try again in a moment.";

/// Gate stages at which the self-review is not offered: the PR is a draft,
/// or the gate is past this step.
const PAST_OR_NOT_AT: [&str; 3] = ["draft", "ready-for-human", "ready-to-merge"];

/// A self-review the app posted. The engine keeps these in memory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedSelfReview {
    /// `owner/name`, as tracked or as GitHub names it; compared ignoring case.
    pub repo: String,
    pub number: u64,
    /// The head commit the comment named.
    pub head_sha: String,
    pub at: DateTime<Utc>,
}

impl PostedSelfReview {
    /// Whether this was posted on `number` in `repo` for `head_sha`.
    pub fn covers(&self, repo: &str, number: u64, head_sha: &str) -> bool {
        self.number == number
            && self.repo.eq_ignore_ascii_case(repo)
            && !head_sha.is_empty()
            && self.head_sha.eq_ignore_ascii_case(head_sha)
    }
}

/// The comment that attests to `head_sha`: exactly `/self-reviewed <sha>`.
/// Refuses anything but a full 40-character hexadecimal commit id, because
/// the gate ignores a comment naming anything else.
pub fn self_review_comment(head_sha: &str) -> AppResult<String> {
    if !is_full_sha(head_sha) {
        return Err(AppError::InvalidInput(SELF_REVIEW_NO_HEAD.to_owned()));
    }
    Ok(format!("{SELF_REVIEW_COMMAND} {head_sha}"))
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads a comment as the gate does. `None`: not a self-review. `Some(None)`:
/// a bare one. `Some(Some(sha))`: one naming a commit.
pub fn attestation(body: &str) -> Option<Option<&str>> {
    let rest = strip_prefix_ignore_case(body.trim(), "/self")?;
    let rest = rest.strip_prefix(['-', ' ']).unwrap_or(rest);
    let rest = strip_prefix_ignore_case(rest, "review")?;
    let rest = strip_prefix_ignore_case(rest, "ed").unwrap_or(rest);
    if rest.is_empty() {
        return Some(None);
    }
    let sha = rest.trim_start();
    // `\s+`: at least one space before the commit.
    (sha.len() < rest.len() && is_full_sha(sha)).then_some(Some(sha))
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

/// Whether `login` holds the PR: is its author or one of its assignees.
pub fn holds(login: &str, author: &str, assignees: &[String]) -> bool {
    !login.is_empty()
        && (login.eq_ignore_ascii_case(author)
            || assignees.iter().any(|a| a.eq_ignore_ascii_case(login)))
}

/// What the gate says, for the confirmation: the sentence after the stage,
/// or the bare description when there is no sentence.
pub fn gate_text(hygiene: Option<&GhHygiene>) -> Option<String> {
    let description = hygiene?.description.as_deref()?.trim();
    if description.is_empty() {
        return None;
    }
    let text = match description.split_once(':') {
        Some((_, sentence)) if !sentence.trim().is_empty() => sentence.trim(),
        Some((stage, _)) => stage.trim(),
        None => description,
    };
    Some(text.to_owned()).filter(|t| !t.is_empty())
}

/// The gate's word on the self-review, before anything the app posted.
fn gate_state(holds: bool, is_draft: bool, hygiene: Option<&GhHygiene>) -> SelfReviewState {
    let Some(hygiene) = hygiene else {
        return SelfReviewState::Unavailable;
    };
    if !holds || is_draft {
        return SelfReviewState::Unavailable;
    }
    let stage = next_step(hygiene, true).map(|s| s.stage);
    if stage.is_some_and(|s| PAST_OR_NOT_AT.contains(&s.as_str())) {
        return SelfReviewState::Unavailable;
    }
    let asks = hygiene
        .description
        .as_deref()
        .is_some_and(|d| d.to_ascii_lowercase().contains("/self-review"));
    if asks {
        SelfReviewState::Wanted
    } else {
        SelfReviewState::Possible
    }
}

/// The self-review state of one PR in the snapshot. `repo` is the name the
/// project is tracked under; `posted` is what the app has posted.
///
/// - `Unavailable`: the viewer does not hold the PR, it is a draft, its head
///   has no PR Hygiene status, or the gate's stage is `draft`,
///   `ready-for-human` or `ready-to-merge`.
/// - `Posted`: otherwise, when the app posted the self-review for the PR's
///   current head commit.
/// - `Wanted`: otherwise, when the gate's description asks for it (contains
///   "/self-review").
/// - `Possible`: otherwise.
pub fn self_review(
    pr: &GhPullRequest,
    repo: &str,
    viewer: Option<&str>,
    posted: &[PostedSelfReview],
) -> SelfReview {
    let holds = viewer.is_some_and(|v| holds(v, &pr.author, &pr.assignees));
    let state = gate_state(holds, pr.is_draft, pr.hygiene.as_ref());
    if state == SelfReviewState::Unavailable {
        return SelfReview::default();
    }
    let ours = posted
        .iter()
        .filter(|p| {
            p.covers(repo, pr.number, &pr.head_sha) || p.covers(&pr.repo, pr.number, &pr.head_sha)
        })
        .map(|p| p.at)
        .max();
    match ours {
        Some(at) => SelfReview {
            state: SelfReviewState::Posted,
            posted_at: Some(at),
        },
        None => SelfReview {
            state,
            posted_at: None,
        },
    }
}

/// The answer to the confirmation's "can I post the self-review now?". The
/// first thing in the way wins: closed, draft, not holding the PR, no usable
/// head commit.
pub fn self_review_check(id: String, check: &GhSelfReviewCheck) -> SelfReviewCheck {
    let comment = self_review_comment(&check.head_sha);
    let reason = if !check.is_open {
        Some(SELF_REVIEW_CLOSED)
    } else if check.is_draft {
        Some(SELF_REVIEW_DRAFT)
    } else if !holds(&check.viewer, &check.author, &check.assignees) {
        Some(SELF_REVIEW_NOT_HOLDING)
    } else if comment.is_err() {
        Some(SELF_REVIEW_NO_HEAD)
    } else {
        None
    };
    SelfReviewCheck {
        id,
        head_sha: check.head_sha.clone(),
        comment: comment.unwrap_or_default(),
        can_post: reason.is_none(),
        reason: reason.map(str::to_owned),
        already_posted_at: already_posted_at(check),
        gate: gate_text(check.hygiene.as_ref()),
        diff_url: format!("{}/files", check.url.trim_end_matches('/')),
    }
}

/// When the viewer last posted a self-review naming the head commit, in a
/// comment the gate would still read (not edited, not by a bot).
fn already_posted_at(check: &GhSelfReviewCheck) -> Option<DateTime<Utc>> {
    if check.head_sha.is_empty() {
        return None;
    }
    check
        .comments
        .iter()
        .filter(|c| !c.edited && !c.author_is_bot && c.author.eq_ignore_ascii_case(&check.viewer))
        .filter(|c| {
            attestation(&c.body)
                .flatten()
                .is_some_and(|sha| sha.eq_ignore_ascii_case(&check.head_sha))
        })
        .map(|c| c.created_at)
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::types::GhComment;
    use crate::merge::tests::{at, gh_pr};
    use pretty_assertions::assert_eq;

    const SHA: &str = "ce87c4ba91c01d646c7fa966b385ec7e215d90af";
    const OTHER: &str = "9cc7778487200a5f379f0cb47cfff854e1a520dd";

    // ---- The comment and the gate's pattern ----

    #[test]
    fn the_comment_is_exactly_the_command_and_the_sha() {
        assert_eq!(
            self_review_comment(SHA).unwrap(),
            format!("/self-reviewed {SHA}")
        );
        // It is a self-review naming that commit, read as the gate reads it.
        let comment = self_review_comment(SHA).unwrap();
        assert_eq!(attestation(&comment), Some(Some(SHA)));
        let upper = SHA.to_ascii_uppercase();
        assert_eq!(
            attestation(&self_review_comment(&upper).unwrap()),
            Some(Some(upper.as_str()))
        );
    }

    #[test]
    fn the_comment_refuses_anything_but_a_full_sha() {
        for bad in [
            "",
            "abc",
            &SHA[..39],
            &format!("{SHA}0"),
            &format!(" {SHA}"),
            &SHA.replace('c', "g"),
            "ce87c4ba91c01d646c7fa966b385ec7e215d90a\n",
        ] {
            assert!(
                matches!(self_review_comment(bad), Err(AppError::InvalidInput(_))),
                "{bad:?}"
            );
        }
    }

    /// Cases worked out against the gate's regular expression
    /// `/self[- ]?review(?:ed)?(?:\s+(?P<head>[0-9a-fA-F]{40}))?` with
    /// `re.IGNORECASE` and `fullmatch` on the trimmed body.
    #[test]
    fn attestation_reads_comments_as_the_gate_does() {
        let named = |s: &str| Some(Some(s.to_owned()));
        let cases: Vec<(String, Option<Option<String>>)> = vec![
            ("/self-reviewed".into(), Some(None)),
            ("/self-review".into(), Some(None)),
            ("/selfreview".into(), Some(None)),
            ("/self review".into(), Some(None)),
            ("/self reviewed".into(), Some(None)),
            ("/SELF-REVIEWED".into(), Some(None)),
            ("/self-reviewed\n".into(), Some(None)),
            ("  /self-reviewed  ".into(), Some(None)),
            (format!("/self-reviewed {SHA}"), named(SHA)),
            (format!("/self-review {SHA}"), named(SHA)),
            (format!("/selfreviewed\t\n {SHA}\n"), named(SHA)),
            (
                format!("/Self-Reviewed {}", SHA.to_ascii_uppercase()),
                named(&SHA.to_ascii_uppercase()),
            ),
            // Not a full match.
            ("/self--reviewed".into(), None),
            ("/self  reviewed".into(), None),
            ("/self-reviewe".into(), None),
            ("/self-reviewing".into(), None),
            ("/self-reviewed.".into(), None),
            ("self-reviewed".into(), None),
            ("I /self-reviewed".into(), None),
            ("/self-reviewed thanks".into(), None),
            (format!("/self-reviewed{SHA}"), None),
            (format!("/self-reviewed {}", &SHA[..39]), None),
            (format!("/self-reviewed {SHA}0"), None),
            (format!("/self-reviewed {SHA} ok"), None),
            (format!("/self-reviewed {SHA}\n/self-reviewed"), None),
            ("".into(), None),
            ("/self".into(), None),
        ];
        for (body, want) in cases {
            let got = attestation(&body).map(|o| o.map(str::to_owned));
            assert_eq!(got, want, "{body:?}");
        }
    }

    #[test]
    fn attestation_survives_multibyte_text() {
        assert_eq!(attestation("/sé"), None);
        assert_eq!(attestation("é/self-reviewed"), None);
        assert_eq!(attestation("/self-reviewed é"), None);
    }

    // ---- Holding a PR, and the gate's words ----

    #[test]
    fn the_author_and_the_assignees_hold_it() {
        let assignees = vec!["Bob".to_owned(), "carol".to_owned()];
        assert!(holds("alice", "Alice", &assignees));
        assert!(holds("bob", "alice", &assignees));
        assert!(holds("CAROL", "alice", &assignees));
        assert!(!holds("dave", "alice", &assignees));
        assert!(!holds("", "", &[]));
        assert!(!holds("bo", "alice", &assignees));
    }

    fn gate(description: Option<&str>) -> GhHygiene {
        GhHygiene {
            state: "PENDING".into(),
            description: description.map(Into::into),
        }
    }

    #[test]
    fn gate_text_is_the_sentence_or_the_bare_stage() {
        let text = |d: Option<&str>| gate_text(Some(&gate(d)));
        assert_eq!(
            text(Some(
                "waiting-self-review: Bots are done - your move: post /self-reviewed."
            ))
            .as_deref(),
            Some("Bots are done - your move: post /self-reviewed.")
        );
        assert_eq!(
            text(Some("ready-to-merge")).as_deref(),
            Some("ready-to-merge")
        );
        assert_eq!(
            text(Some("Evaluating current review policy")).as_deref(),
            Some("Evaluating current review policy")
        );
        assert_eq!(text(Some("draft:  ")).as_deref(), Some("draft"));
        assert_eq!(text(Some(": ")), None);
        assert_eq!(text(Some("  ")), None);
        assert_eq!(text(None), None);
        assert_eq!(gate_text(None), None);
    }

    // ---- The snapshot ----

    fn state(holds: bool, is_draft: bool, description: Option<Option<&str>>) -> SelfReviewState {
        let hygiene = description.map(gate);
        gate_state(holds, is_draft, hygiene.as_ref())
    }

    #[test]
    fn each_state_from_the_gate() {
        use SelfReviewState::*;
        let wanted = Some(Some(
            "waiting-self-review: Bots are done - your move: post /self-reviewed.",
        ));
        let cases: Vec<(bool, bool, Option<Option<&str>>, SelfReviewState)> = vec![
            (true, false, wanted, Wanted),
            (
                true,
                false,
                Some(Some(
                    "waiting-author: Address CodeRabbit's findings, then post /Self-Reviewed.",
                )),
                Wanted,
            ),
            (
                true,
                false,
                Some(Some(
                    "waiting-self-review: Author must post /self-reviewed ce87c4ba91c01d646c7fa966b385ec7e215d90af",
                )),
                Wanted,
            ),
            // Not holding it, a draft, no gate.
            (false, false, wanted, Unavailable),
            (true, true, wanted, Unavailable),
            (true, false, None, Unavailable),
            // Past this step, or not at it.
            (true, false, Some(Some("draft")), Unavailable),
            (
                true,
                false,
                Some(Some("ready-for-human: Waiting for alice to review.")),
                Unavailable,
            ),
            (true, false, Some(Some("READY-TO-MERGE")), Unavailable),
            // Some other step before human review.
            (
                true,
                false,
                Some(Some(
                    "waiting-bots: Waiting for bot review - thepastaclaw not yet.",
                )),
                Possible,
            ),
            (
                true,
                false,
                Some(Some(
                    "waiting-self-review: Your move: thepastaclaw requested changes on this head; dismiss the review or push a fix.",
                )),
                Possible,
            ),
            (true, false, Some(Some("waiting-build")), Possible),
            (
                true,
                false,
                Some(Some("waiting-author: Resolve the open threads.")),
                Possible,
            ),
            (
                true,
                false,
                Some(Some("Evaluating current review policy")),
                Possible,
            ),
            (true, false, Some(None), Possible),
        ];
        for (holds, draft, description, want) in cases {
            assert_eq!(
                state(holds, draft, description),
                want,
                "holds={holds} draft={draft} {description:?}"
            );
        }
    }

    fn pr(author: &str, assignees: &[&str], description: Option<&str>) -> GhPullRequest {
        let mut pr = gh_pr("o/r", 7);
        pr.author = author.into();
        pr.assignees = assignees.iter().map(|a| (*a).to_owned()).collect();
        pr.head_sha = SHA.into();
        pr.hygiene = description.map(|d| gate(Some(d)));
        pr
    }

    fn posted(repo: &str, number: u64, head: &str, at_: &str) -> PostedSelfReview {
        PostedSelfReview {
            repo: repo.into(),
            number,
            head_sha: head.into(),
            at: at(at_),
        }
    }

    const ASKS: &str = "waiting-self-review: Bots are done - your move: post /self-reviewed.";

    #[test]
    fn an_assignee_holds_it_in_the_snapshot() {
        let p = pr("someone", &["Alice"], Some(ASKS));
        assert_eq!(
            self_review(&p, "o/r", Some("alice"), &[]).state,
            SelfReviewState::Wanted
        );
        assert_eq!(
            self_review(&p, "o/r", Some("bob"), &[]),
            SelfReview::default()
        );
        assert_eq!(self_review(&p, "o/r", None, &[]), SelfReview::default());
    }

    #[test]
    fn posted_wins_for_the_current_head_only() {
        let p = pr("alice", &[], Some(ASKS));
        let ours = [posted(
            "O/R",
            7,
            &SHA.to_ascii_uppercase(),
            "2026-11-22T12:00:00Z",
        )];
        assert_eq!(
            self_review(&p, "o/r", Some("alice"), &ours),
            SelfReview {
                state: SelfReviewState::Posted,
                posted_at: Some(at("2026-11-22T12:00:00Z")),
            }
        );
        // Over Possible too.
        let waiting = pr("alice", &[], Some("waiting-bots: Waiting for bot review."));
        assert_eq!(
            self_review(&waiting, "o/r", Some("alice"), &ours).state,
            SelfReviewState::Posted
        );
        // The latest of several.
        let twice = [
            posted("o/r", 7, SHA, "2026-11-22T12:00:00Z"),
            posted("o/r", 7, SHA, "2026-11-22T12:05:00Z"),
        ];
        assert_eq!(
            self_review(&p, "o/r", Some("alice"), &twice).posted_at,
            Some(at("2026-11-22T12:05:00Z"))
        );
        // A new head, another PR or another repo: not ours.
        for other in [
            posted("o/r", 7, OTHER, "2026-11-22T12:00:00Z"),
            posted("o/r", 8, SHA, "2026-11-22T12:00:00Z"),
            posted("o/other", 7, SHA, "2026-11-22T12:00:00Z"),
        ] {
            assert_eq!(
                self_review(&p, "o/r", Some("alice"), std::slice::from_ref(&other)).state,
                SelfReviewState::Wanted,
                "{other:?}"
            );
        }
        // GitHub's current name of a renamed repo counts as well.
        assert_eq!(
            self_review(&p, "o/old-name", Some("alice"), &ours).state,
            SelfReviewState::Posted
        );
    }

    #[test]
    fn posted_ends_when_the_gate_moves_past_it() {
        let ours = [posted("o/r", 7, SHA, "2026-11-22T12:00:00Z")];
        for description in [
            Some("ready-for-human: Waiting for bob."),
            Some("ready-to-merge"),
            Some("draft"),
            None,
        ] {
            let p = pr("alice", &[], description);
            assert_eq!(
                self_review(&p, "o/r", Some("alice"), &ours),
                SelfReview::default(),
                "{description:?}"
            );
        }
        let mut draft = pr("alice", &[], Some(ASKS));
        draft.is_draft = true;
        assert_eq!(
            self_review(&draft, "o/r", Some("alice"), &ours),
            SelfReview::default()
        );
    }

    // ---- The fresh check ----

    fn comment(author: &str, body: &str, at_: &str) -> GhComment {
        GhComment {
            author: author.into(),
            author_is_bot: false,
            body: body.into(),
            created_at: at(at_),
            edited: false,
        }
    }

    fn gh_check() -> GhSelfReviewCheck {
        GhSelfReviewCheck {
            node_id: "PR_node".into(),
            url: "https://github.com/o/r/pull/7".into(),
            head_sha: SHA.into(),
            is_draft: false,
            is_open: true,
            author: "alice".into(),
            assignees: vec![],
            viewer: "Alice".into(),
            hygiene: Some(gate(Some(ASKS))),
            comments: vec![],
        }
    }

    #[test]
    fn check_allows_the_holder() {
        let c = self_review_check("o/r#7".into(), &gh_check());
        assert_eq!(
            c,
            SelfReviewCheck {
                id: "o/r#7".into(),
                head_sha: SHA.into(),
                comment: format!("/self-reviewed {SHA}"),
                can_post: true,
                reason: None,
                already_posted_at: None,
                gate: Some("Bots are done - your move: post /self-reviewed.".into()),
                diff_url: "https://github.com/o/r/pull/7/files".into(),
            }
        );
        // An assignee may too.
        let mut gh = gh_check();
        gh.author = "someone".into();
        gh.assignees = vec!["ALICE".into()];
        assert!(self_review_check("o/r#7".into(), &gh).can_post);
    }

    #[test]
    fn check_refusals_in_order() {
        let mut gh = gh_check();
        gh.is_open = false;
        gh.is_draft = true;
        gh.author = "someone".into();
        let mut seen = Vec::new();
        let mut look = |gh: &GhSelfReviewCheck| {
            let c = self_review_check("o/r#7".into(), gh);
            assert_eq!(c.can_post, c.reason.is_none());
            seen.push(c.reason.unwrap_or_else(|| "ok".into()));
        };
        look(&gh);
        gh.is_open = true;
        look(&gh);
        gh.is_draft = false;
        look(&gh);
        gh.head_sha = "abc".into();
        gh.author = "alice".into();
        look(&gh);
        gh.head_sha = SHA.into();
        look(&gh);
        assert_eq!(
            seen,
            vec![
                SELF_REVIEW_CLOSED,
                SELF_REVIEW_DRAFT,
                SELF_REVIEW_NOT_HOLDING,
                SELF_REVIEW_NO_HEAD,
                "ok"
            ]
        );
    }

    #[test]
    fn a_refused_check_still_shows_the_comment_and_gate() {
        let mut gh = gh_check();
        gh.is_draft = true;
        let c = self_review_check("o/r#7".into(), &gh);
        assert!(!c.can_post);
        assert_eq!(c.comment, format!("/self-reviewed {SHA}"));
        assert!(c.gate.is_some());
        // No usable head: nothing to post.
        gh.head_sha = String::new();
        assert_eq!(self_review_check("o/r#7".into(), &gh).comment, "");
    }

    #[test]
    fn already_posted_is_the_viewers_latest_countable_one_for_this_head() {
        let mut gh = gh_check();
        gh.comments = vec![
            comment(
                "alice",
                &format!("/self-reviewed {SHA}"),
                "2026-11-22T10:00:00Z",
            ),
            comment(
                "ALICE",
                &format!("/self-reviewed {SHA}"),
                "2026-11-22T11:00:00Z",
            ),
            // Another head, bare, wrapped in words, someone else, a bot,
            // edited: none of these.
            comment(
                "alice",
                &format!("/self-reviewed {OTHER}"),
                "2026-11-22T12:00:00Z",
            ),
            comment("alice", "/self-reviewed", "2026-11-22T12:00:00Z"),
            comment(
                "alice",
                &format!("Done: /self-reviewed {SHA}"),
                "2026-11-22T12:00:00Z",
            ),
            comment(
                "bob",
                &format!("/self-reviewed {SHA}"),
                "2026-11-22T12:00:00Z",
            ),
            GhComment {
                author_is_bot: true,
                ..comment(
                    "alice",
                    &format!("/self-reviewed {SHA}"),
                    "2026-11-22T12:00:00Z",
                )
            },
            GhComment {
                edited: true,
                ..comment(
                    "alice",
                    &format!("/self-reviewed {SHA}"),
                    "2026-11-22T12:00:00Z",
                )
            },
        ];
        let c = self_review_check("o/r#7".into(), &gh);
        assert_eq!(c.already_posted_at, Some(at("2026-11-22T11:00:00Z")));
        // It does not stop a renewed one.
        assert!(c.can_post);
    }
}
