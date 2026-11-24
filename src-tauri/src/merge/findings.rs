//! How the findings of the last review rounds on a PR stand, and when the
//! last round was.

use chrono::{DateTime, Utc};

use crate::github::types::{
    GhFinding, GhFindingReviewer, GhFindingStatus, GhLastRounds, GhPullRequest, GhSeverity,
};
use crate::model::{FindingCounts, SeverityCount};

/// Counts findings by severity. Agreed: the reviewer itself lifted the
/// finding (resolved, withdrawn, no longer applies; for CodeRabbit,
/// addressed). Not agreed: disputed or deferred; or the thread was resolved
/// on GitHub while the finding still stands and that finding's reviewer's
/// last round (`rounds`) was submitted on `head_sha`, the PR's current head,
/// so it has seen the code and did not lift the finding. Awaiting: the
/// thread was resolved on GitHub while the finding still stands and that
/// reviewer's last round was on another commit, or on one not known, so the
/// fix is not re-checked yet. The rest are open. `coderabbit` counts how
/// many of each total are CodeRabbit's.
pub(crate) fn finding_counts(
    findings: &[GhFinding],
    rounds: &GhLastRounds,
    head_sha: &str,
) -> FindingCounts {
    let mut counts = FindingCounts::default();
    for f in findings {
        let slot: &mut SeverityCount = match f.severity {
            GhSeverity::Blocking => &mut counts.blocking,
            GhSeverity::Suggestion => &mut counts.suggestion,
            GhSeverity::Nitpick => &mut counts.nitpick,
        };
        slot.total += 1;
        if f.reviewer == GhFindingReviewer::CodeRabbit {
            slot.coderabbit += 1;
        }
        match f.status {
            GhFindingStatus::Resolved
            | GhFindingStatus::Withdrawn
            | GhFindingStatus::NoLongerApplies => slot.agreed += 1,
            GhFindingStatus::Disputed | GhFindingStatus::Deferred => slot.not_agreed += 1,
            GhFindingStatus::Stands if f.thread_resolved => {
                if reviewed_head(rounds.commit(f.reviewer), head_sha) {
                    slot.not_agreed += 1;
                } else {
                    slot.awaiting += 1;
                }
            }
            GhFindingStatus::Stands => {}
        }
    }
    counts
}

/// Whether a round on `commit` was on `head_sha`. An unknown commit or head
/// is not.
fn reviewed_head(commit: Option<&str>, head_sha: &str) -> bool {
    commit.is_some_and(|c| !c.is_empty() && c.eq_ignore_ascii_case(head_sha))
}

/// When the later of the two last rounds (the review bot's, CodeRabbit's)
/// was submitted; failing both, when the bot's check run last completed.
pub(crate) fn last_review_at(pr: &GhPullRequest) -> Option<DateTime<Utc>> {
    pr.last_rounds
        .latest()
        .or_else(|| pr.bot_check.as_ref().and_then(|c| c.completed_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::types::{GhBotCheck, GhCheckStatus, GhLastRounds};
    use crate::merge::tests::{at, gh_pr};
    use GhFindingReviewer::*;
    use GhFindingStatus::*;
    use pretty_assertions::assert_eq;

    fn f(severity: GhSeverity, status: GhFindingStatus, thread_resolved: bool) -> GhFinding {
        GhFinding {
            reviewer: ReviewBot,
            severity,
            status,
            thread_resolved,
        }
    }

    fn cr(severity: GhSeverity, status: GhFindingStatus, thread_resolved: bool) -> GhFinding {
        GhFinding {
            reviewer: CodeRabbit,
            ..f(severity, status, thread_resolved)
        }
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

    const HEAD: &str = "4dd9ab0c3051534db0feffa32d1d8bde98413d7a";
    const OLDER: &str = "9363df29b4fcc7cadda0b27b418cc71e201b69f6";

    /// Both reviewers' last rounds on `commit`.
    fn on(commit: Option<&str>) -> GhLastRounds {
        GhLastRounds {
            review_bot: Some(at("2026-11-22T11:41:38Z")),
            coderabbit: Some(at("2026-11-22T11:00:00Z")),
            review_bot_commit: commit.map(str::to_owned),
            coderabbit_commit: commit.map(str::to_owned),
        }
    }

    /// What one finding counts as: (agreed, not agreed, awaiting); open
    /// otherwise.
    fn outcome(f: GhFinding, rounds: &GhLastRounds, head: &str) -> (u32, u32, u32) {
        let c = finding_counts(&[f], rounds, head);
        assert_eq!(c.blocking, count(0, 0, 0, 0, 0));
        assert_eq!(c.nitpick, count(0, 0, 0, 0, 0));
        let s = c.suggestion;
        assert_eq!(s.total, 1);
        assert_eq!(s.coderabbit, u32::from(f.reviewer == CodeRabbit));
        (s.agreed, s.not_agreed, s.awaiting)
    }

    #[test]
    fn every_status_resolved_or_not_reviewed_on_the_head_or_not() {
        // The last round on the head, on an older commit, on an unknown
        // commit, and no round at all.
        let on_head = on(Some(HEAD));
        let older = on(Some(OLDER));
        let unknown = on(None);
        let none = GhLastRounds::default();
        let all = [
            (&on_head, true),
            (&older, false),
            (&unknown, false),
            (&none, false),
        ];
        for make in [f, cr] {
            for (rounds, reviewed_head) in all {
                // (status, thread resolved) -> outcome, whatever the round.
                let same = [
                    (Resolved, false, (1, 0, 0)),
                    (Resolved, true, (1, 0, 0)),
                    (Withdrawn, false, (1, 0, 0)),
                    (Withdrawn, true, (1, 0, 0)),
                    (NoLongerApplies, false, (1, 0, 0)),
                    (NoLongerApplies, true, (1, 0, 0)),
                    (Disputed, false, (0, 1, 0)),
                    (Disputed, true, (0, 1, 0)),
                    (Deferred, false, (0, 1, 0)),
                    (Deferred, true, (0, 1, 0)),
                    (Stands, false, (0, 0, 0)),
                ];
                for (status, resolved, want) in same {
                    let finding = make(GhSeverity::Suggestion, status, resolved);
                    assert_eq!(
                        outcome(finding, rounds, HEAD),
                        want,
                        "{finding:?} {rounds:?}"
                    );
                }
                // Stands, resolved by hand: not agreed only when the round
                // was on the current head.
                let finding = make(GhSeverity::Suggestion, Stands, true);
                let want = if reviewed_head { (0, 1, 0) } else { (0, 0, 1) };
                assert_eq!(
                    outcome(finding, rounds, HEAD),
                    want,
                    "{finding:?} {rounds:?}"
                );
            }
        }
    }

    #[test]
    fn each_finding_is_judged_by_its_own_reviewers_round() {
        use GhSeverity::*;
        // The bot re-reviewed the head; CodeRabbit's last round is older.
        let rounds = GhLastRounds {
            review_bot_commit: Some(HEAD.into()),
            coderabbit_commit: Some(OLDER.into()),
            ..on(None)
        };
        let findings = [f(Blocking, Stands, true), cr(Blocking, Stands, true)];
        assert_eq!(
            finding_counts(&findings, &rounds, HEAD).blocking,
            count(2, 0, 1, 1, 1)
        );
        // And the other way round.
        let rounds = GhLastRounds {
            review_bot_commit: Some(OLDER.into()),
            coderabbit_commit: Some(HEAD.into()),
            ..on(None)
        };
        assert_eq!(
            finding_counts(&findings, &rounds, HEAD).blocking,
            count(2, 0, 1, 1, 1)
        );
    }

    #[test]
    fn the_head_is_compared_whole_and_without_case() {
        let finding = f(GhSeverity::Suggestion, Stands, true);
        let upper = on(Some(&HEAD.to_ascii_uppercase()));
        assert_eq!(outcome(finding, &upper, HEAD), (0, 1, 0));
        // A short sha is another commit as far as this is concerned.
        assert_eq!(outcome(finding, &on(Some(&HEAD[..8])), HEAD), (0, 0, 1));
        // An unknown head matches nothing, not even an empty commit.
        assert_eq!(outcome(finding, &on(Some("")), ""), (0, 0, 1));
        assert_eq!(outcome(finding, &on(Some(HEAD)), ""), (0, 0, 1));
    }

    #[test]
    fn platform_6056_before_and_after_the_re_review() {
        use GhSeverity::*;
        // dashpay/platform#6056: the bot's round on 9363df29 opened 5
        // blocking and 5 suggestion threads. The author pushed 4dd9ab0c,
        // replied "Fixed in ..." and resolved all ten; the bot had not
        // re-reviewed yet.
        let mut findings = vec![f(Blocking, Stands, true); 5];
        findings.extend(vec![f(Suggestion, Stands, true); 5]);
        let mut rounds = GhLastRounds {
            coderabbit: None,
            coderabbit_commit: None,
            ..on(Some(OLDER))
        };
        let c = finding_counts(&findings, &rounds, HEAD);
        assert_eq!(c.blocking, count(5, 0, 0, 5, 0));
        assert_eq!(c.suggestion, count(5, 0, 0, 5, 0));
        assert_eq!(c.nitpick, count(0, 0, 0, 0, 0));

        // Had that round been on the head, the same threads are not agreed.
        rounds.review_bot_commit = Some(HEAD.into());
        let c = finding_counts(&findings, &rounds, HEAD);
        assert_eq!(c.blocking, count(5, 0, 5, 0, 0));
        assert_eq!(c.suggestion, count(5, 0, 5, 0, 0));
    }

    #[test]
    fn counts_add_up_by_severity() {
        use GhSeverity::*;
        let findings = [
            f(Blocking, Resolved, true),
            f(Blocking, Disputed, true),
            f(Blocking, Stands, true),
            f(Blocking, Stands, false),
            f(Suggestion, Withdrawn, false),
            f(Suggestion, Stands, false),
            f(Nitpick, Deferred, false),
            f(Nitpick, Stands, true),
        ];
        assert_eq!(
            finding_counts(&findings, &on(Some(HEAD)), HEAD),
            FindingCounts {
                blocking: count(4, 1, 2, 0, 0),
                suggestion: count(2, 1, 0, 0, 0),
                nitpick: count(2, 0, 2, 0, 0),
            }
        );
        assert_eq!(
            finding_counts(&findings, &on(Some(OLDER)), HEAD),
            FindingCounts {
                blocking: count(4, 1, 1, 1, 0),
                suggestion: count(2, 1, 0, 0, 0),
                nitpick: count(2, 0, 1, 1, 0),
            }
        );
        assert_eq!(
            finding_counts(&[], &on(Some(HEAD)), HEAD),
            FindingCounts::default()
        );
    }

    #[test]
    fn coderabbit_findings_join_the_totals_and_are_counted_apart() {
        use GhSeverity::*;
        let rounds = on(Some(HEAD));
        let c = finding_counts(
            &[
                f(Blocking, Stands, false),
                // Addressed.
                cr(Blocking, Resolved, true),
                // Resolved by a person without "Addressed", after
                // CodeRabbit reviewed the head: not agreed.
                cr(Blocking, Stands, true),
                // Open.
                cr(Suggestion, Stands, false),
                f(Suggestion, Resolved, false),
                cr(Nitpick, Stands, false),
            ],
            &rounds,
            HEAD,
        );
        assert_eq!(
            c,
            FindingCounts {
                blocking: count(3, 1, 1, 0, 2),
                suggestion: count(2, 1, 0, 0, 1),
                nitpick: count(1, 0, 0, 0, 1),
            }
        );
        // Only CodeRabbit.
        let c = finding_counts(
            &[cr(Blocking, Stands, false), cr(Blocking, Resolved, false)],
            &rounds,
            HEAD,
        );
        assert_eq!(c.blocking, count(2, 1, 0, 0, 2));
    }

    fn completed_check(at_: Option<&str>) -> GhBotCheck {
        GhBotCheck {
            status: GhCheckStatus::Completed,
            conclusion: Some("SUCCESS".into()),
            title: Some("Approved".into()),
            started_at: None,
            completed_at: at_.map(at),
        }
    }

    fn rounds(bot: Option<&str>, rabbit: Option<&str>) -> GhLastRounds {
        GhLastRounds {
            review_bot: bot.map(at),
            coderabbit: rabbit.map(at),
            ..GhLastRounds::default()
        }
    }

    #[test]
    fn last_review_is_the_later_last_round() {
        let mut pr = gh_pr("o/r", 1);
        // A round beats the check run, even a later one.
        pr.bot_check = Some(completed_check(Some("2026-11-21T23:00:00Z")));
        let cases = [
            (
                rounds(Some("2026-11-21T08:00:00Z"), Some("2026-11-21T09:00:00Z")),
                "2026-11-21T09:00:00Z",
            ),
            (
                rounds(Some("2026-11-21T10:00:00Z"), Some("2026-11-21T09:00:00Z")),
                "2026-11-21T10:00:00Z",
            ),
            (
                rounds(Some("2026-11-21T08:00:00Z"), None),
                "2026-11-21T08:00:00Z",
            ),
            (
                rounds(None, Some("2026-11-21T09:00:00Z")),
                "2026-11-21T09:00:00Z",
            ),
        ];
        for (r, want) in cases {
            pr.last_rounds = r.clone();
            assert_eq!(last_review_at(&pr), Some(at(want)), "{r:?}");
        }
    }

    #[test]
    fn last_review_falls_back_to_the_check_run() {
        let mut pr = gh_pr("o/r", 1);
        // Neither reviewer has a round.
        pr.last_rounds = GhLastRounds::default();
        pr.bot_check = Some(completed_check(Some("2026-11-21T11:00:00Z")));
        assert_eq!(last_review_at(&pr), Some(at("2026-11-21T11:00:00Z")));

        // A check run still going has no completion time.
        pr.bot_check = Some(completed_check(None));
        assert_eq!(last_review_at(&pr), None);
        pr.bot_check = None;
        assert_eq!(last_review_at(&pr), None);
    }
}
