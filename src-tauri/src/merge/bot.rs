//! Where the bot's review of a PR's head commit stands.
//!
//! The feed knows about live work (running and queued reviews) before
//! GitHub does; the check run is the source of truth for verdicts. So: a
//! feed entry for the head commit wins, then the check run.

use chrono::{DateTime, Utc};

use super::eta;
use super::steps;
use crate::feed::{FeedActive, FeedQueued, FeedSnapshot};
use crate::github::types::{GhBotCheck, GhCheckStatus, GhPullRequest};
use crate::model::BotReview;

const UNKNOWN_PHASE: &str = "unknown";
const REVIEWING: &str = "Reviewing";

pub fn bot_review(pr: &GhPullRequest, feed: Option<&FeedSnapshot>) -> BotReview {
    // The feed lags: once GitHub shows the check finished after the feed was
    // generated, the feed's "running" or "queued" entry is out of date.
    let feed = feed.filter(|f| !finished_since(pr.bot_check.as_ref(), f.data_as_of));
    if let Some(feed) = feed {
        if let Some(a) = feed
            .active
            .iter()
            .find(|a| is_for(pr, &a.repo, a.number, a.sha.as_deref()))
        {
            return reviewing_from_feed(a, feed.data_as_of);
        }
        if let Some(q) = feed
            .queued
            .iter()
            .find(|q| is_for(pr, &q.repo, q.number, q.sha.as_deref()))
        {
            return queued_from_feed(q, feed);
        }
    }
    match &pr.bot_check {
        Some(check) => from_check(check),
        None => BotReview::None,
    }
}

/// The check completed after the feed's data was generated.
fn finished_since(check: Option<&GhBotCheck>, feed_as_of: Option<DateTime<Utc>>) -> bool {
    match (check, feed_as_of) {
        (Some(check), Some(as_of)) => {
            check.status == GhCheckStatus::Completed
                && check.completed_at.is_some_and(|done| done > as_of)
        }
        _ => false,
    }
}

/// A feed entry is about this PR's head commit. An entry for another sha is
/// stale; one without a sha is taken at its word.
fn is_for(pr: &GhPullRequest, repo: &str, number: u64, sha: Option<&str>) -> bool {
    number == pr.number
        && repo.eq_ignore_ascii_case(&pr.repo)
        && sha.is_none_or(|s| s.eq_ignore_ascii_case(&pr.head_sha))
}

fn reviewing_from_feed(a: &FeedActive, as_of: Option<DateTime<Utc>>) -> BotReview {
    let phase = a.phase.clone().unwrap_or_else(|| UNKNOWN_PHASE.to_string());
    let phase_label = if phase == UNKNOWN_PHASE {
        REVIEWING.to_string()
    } else {
        steps::label(&phase)
    };
    let started_at = a
        .started_at
        .or_else(|| as_of.and_then(|t| a.elapsed_seconds.and_then(|e| eta::add_seconds(t, -e))));
    BotReview::Reviewing {
        phase,
        phase_label,
        progress: a.progress.map(|p| p as f32),
        finishes_at: as_of.and_then(|t| a.remaining_seconds.and_then(|r| eta::add_seconds(t, r))),
        started_at,
        overdue: a.overdue,
        tier: a.tier.clone(),
        priority: a.priority,
        waited_seconds: a.wait_seconds.map(|w| w.round() as u64),
        steps: steps::review_steps(a),
        findings: a.findings,
    }
}

fn queued_from_feed(q: &FeedQueued, feed: &FeedSnapshot) -> BotReview {
    let starts_at = eta::queued_starts_at(q, feed.data_as_of);
    BotReview::Queued {
        position: q.position,
        starts_at,
        finishes_at: eta::queued_finishes_at(q, starts_at, feed),
        reason: q.reason.clone(),
        priority: q.priority,
    }
}

fn from_check(check: &GhBotCheck) -> BotReview {
    match check.status {
        GhCheckStatus::Queued => BotReview::Queued {
            position: None,
            starts_at: None,
            finishes_at: None,
            reason: None,
            priority: false,
        },
        GhCheckStatus::InProgress => BotReview::Reviewing {
            phase: UNKNOWN_PHASE.to_string(),
            phase_label: check
                .title
                .as_deref()
                .and_then(phase_from_title)
                .unwrap_or_else(|| REVIEWING.to_string()),
            progress: None,
            finishes_at: None,
            started_at: check.started_at,
            overdue: false,
            tier: None,
            priority: false,
            waited_seconds: None,
            steps: Vec::new(),
            findings: None,
        },
        GhCheckStatus::Completed => completed(check),
    }
}

fn completed(check: &GhBotCheck) -> BotReview {
    let at = check.completed_at;
    let conclusion = check
        .conclusion
        .as_deref()
        .map(str::to_ascii_uppercase)
        .unwrap_or_default();
    let title_mentions_draft = check
        .title
        .as_deref()
        .is_some_and(|t| t.to_lowercase().contains("draft"));
    match conclusion.as_str() {
        "SUCCESS" => BotReview::Approved { at },
        "FAILURE" => BotReview::ChangesRequested { at },
        "NEUTRAL" => BotReview::Commented { at },
        "CANCELLED" | "SKIPPED" if title_mentions_draft => BotReview::Draft,
        _ => BotReview::Other {
            title: check
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .or_else(|| check.conclusion.clone())
                .unwrap_or_else(|| "Review finished".to_string()),
        },
    }
}

/// "Reviewing — Phase 1 review" -> "Phase 1". `None` when the title does
/// not have that shape.
fn phase_from_title(title: &str) -> Option<String> {
    let (_, rest) = title
        .split_once('—')
        .or_else(|| title.split_once('–'))
        .or_else(|| title.split_once(" - "))?;
    let rest = rest.trim();
    const SUFFIX: &str = " review";
    let cut = rest.len().saturating_sub(SUFFIX.len());
    let phase = match (rest.get(..cut), rest.get(cut..)) {
        (Some(head), Some(tail)) if tail.eq_ignore_ascii_case(SUFFIX) => head.trim_end(),
        _ => rest,
    };
    (!phase.is_empty()).then(|| phase.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::{FeedStep, PathTiming};
    use crate::merge::tests::{at, check, gh_pr};
    use crate::model::{Findings, ReviewStep, StepStatus};
    use pretty_assertions::assert_eq;

    const SHA: &str = "abc123";

    fn feed_with(active: Vec<FeedActive>, queued: Vec<FeedQueued>) -> FeedSnapshot {
        FeedSnapshot {
            data_as_of: Some(at("2026-11-21T20:00:00Z")),
            active,
            queued,
            normal_path: PathTiming {
                duration_median_seconds: Some(1800.0),
                slot_wait_median_seconds: Some(60.0),
                wait_median_seconds: None,
            },
            ..FeedSnapshot::default()
        }
    }

    fn active(repo: &str, number: u64, sha: Option<&str>) -> FeedActive {
        FeedActive {
            repo: repo.into(),
            number,
            sha: sha.map(Into::into),
            phase: Some("phase2".into()),
            progress: Some(0.5),
            remaining_seconds: Some(600.0),
            elapsed_seconds: Some(1200.0),
            overdue: true,
            tier: Some("critical".into()),
            priority: true,
            wait_seconds: Some(30.4),
            findings: Some(Findings {
                blocking: 1,
                suggestion: 2,
                nitpick: 3,
            }),
            steps: vec![FeedStep {
                name: "context".into(),
                status: Some("ok".into()),
                ..FeedStep::default()
            }],
            upcoming: vec!["publish".into()],
            ..FeedActive::default()
        }
    }

    fn queued(repo: &str, number: u64, sha: Option<&str>) -> FeedQueued {
        FeedQueued {
            repo: repo.into(),
            number,
            sha: sha.map(Into::into),
            position: Some(2),
            eligible_in_seconds: Some(300.0),
            reason: Some("debounce (more pushes may follow)".into()),
            ..FeedQueued::default()
        }
    }

    fn pr() -> GhPullRequest {
        let mut pr = gh_pr("Owner/Repo", 7);
        pr.head_sha = SHA.into();
        pr.bot_check = Some(check(
            GhCheckStatus::Completed,
            Some("SUCCESS"),
            Some("Approved"),
        ));
        pr
    }

    #[test]
    fn a_check_that_finished_after_the_feed_was_generated_wins() {
        // Feed as of 20:00 still says "running"; GitHub says approved at 20:10.
        let feed = feed_with(
            vec![active("owner/repo", 7, Some(SHA))],
            vec![queued("owner/repo", 7, Some(SHA))],
        );
        let mut done = pr();
        let finished = at("2026-11-21T20:10:00Z");
        done.bot_check.as_mut().unwrap().completed_at = Some(finished);
        assert_eq!(
            bot_review(&done, Some(&feed)),
            BotReview::Approved { at: Some(finished) }
        );
    }

    #[test]
    fn a_check_that_finished_before_the_feed_is_an_earlier_run() {
        // Approved at 19:50, then re-reviewed: the feed (20:00) is newer.
        let feed = feed_with(vec![active("owner/repo", 7, Some(SHA))], vec![]);
        let mut rerun = pr();
        rerun.bot_check.as_mut().unwrap().completed_at = Some(at("2026-11-21T19:50:00Z"));
        assert!(matches!(
            bot_review(&rerun, Some(&feed)),
            BotReview::Reviewing { .. }
        ));
    }

    #[test]
    fn active_entry_for_the_head_is_reviewing_with_every_detail() {
        let feed = feed_with(vec![active("owner/repo", 7, Some(SHA))], vec![]);
        assert_eq!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Reviewing {
                phase: "phase2".into(),
                phase_label: "Phase 2".into(),
                progress: Some(0.5),
                finishes_at: Some(at("2026-11-21T20:10:00Z")),
                started_at: Some(at("2026-11-21T19:40:00Z")),
                overdue: true,
                tier: Some("critical".into()),
                priority: true,
                waited_seconds: Some(30),
                steps: vec![
                    ReviewStep {
                        key: "triage".into(),
                        label: "Triage".into(),
                        status: StepStatus::Done,
                        detail: None
                    },
                    ReviewStep {
                        key: "publish".into(),
                        label: "Publish".into(),
                        status: StepStatus::Pending,
                        detail: None
                    },
                ],
                findings: Some(Findings {
                    blocking: 1,
                    suggestion: 2,
                    nitpick: 3
                }),
            }
        );
    }

    #[test]
    fn feed_started_at_wins_over_elapsed() {
        let mut a = active("owner/repo", 7, None);
        a.started_at = Some(at("2026-11-21T19:00:00Z"));
        let feed = feed_with(vec![a], vec![]);
        let BotReview::Reviewing { started_at, .. } = bot_review(&pr(), Some(&feed)) else {
            panic!("expected reviewing");
        };
        assert_eq!(started_at, Some(at("2026-11-21T19:00:00Z")));
    }

    #[test]
    fn active_entry_without_sha_matches() {
        let feed = feed_with(vec![active("owner/repo", 7, None)], vec![]);
        assert!(matches!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Reviewing { .. }
        ));
    }

    #[test]
    fn active_entry_without_phase_is_unknown() {
        let mut a = active("owner/repo", 7, None);
        a.phase = None;
        let feed = feed_with(vec![a], vec![]);
        let BotReview::Reviewing {
            phase, phase_label, ..
        } = bot_review(&pr(), Some(&feed))
        else {
            panic!("expected reviewing");
        };
        assert_eq!(
            (phase.as_str(), phase_label.as_str()),
            ("unknown", "Reviewing")
        );
    }

    #[test]
    fn stale_active_entry_falls_through_to_the_check() {
        let feed = feed_with(vec![active("owner/repo", 7, Some("old-sha"))], vec![]);
        assert_eq!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Approved { at: None }
        );
    }

    #[test]
    fn other_prs_in_the_feed_do_not_match() {
        let feed = feed_with(
            vec![
                active("owner/repo", 8, None),
                active("owner/other", 7, None),
            ],
            vec![queued("owner/repo", 70, None)],
        );
        assert_eq!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Approved { at: None }
        );
    }

    #[test]
    fn queued_entry_is_queued_with_our_estimate() {
        let feed = feed_with(vec![], vec![queued("OWNER/REPO", 7, Some(SHA))]);
        assert_eq!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Queued {
                position: Some(2),
                starts_at: Some(at("2026-11-21T20:05:00Z")),
                finishes_at: Some(at("2026-11-21T20:36:00Z")),
                reason: Some("debounce (more pushes may follow)".into()),
                priority: false,
            }
        );
    }

    #[test]
    fn active_wins_over_queued() {
        let feed = feed_with(
            vec![active("owner/repo", 7, Some(SHA))],
            vec![queued("owner/repo", 7, Some(SHA))],
        );
        assert!(matches!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Reviewing { .. }
        ));
    }

    #[test]
    fn stale_queued_entry_falls_through_to_the_check() {
        let feed = feed_with(vec![], vec![queued("owner/repo", 7, Some("old"))]);
        assert_eq!(
            bot_review(&pr(), Some(&feed)),
            BotReview::Approved { at: None }
        );
    }

    fn with_check(
        status: GhCheckStatus,
        conclusion: Option<&str>,
        title: Option<&str>,
    ) -> BotReview {
        let mut pr = pr();
        let mut c = check(status, conclusion, title);
        c.completed_at = Some(at("2026-11-21T18:00:00Z"));
        c.started_at = Some(at("2026-11-21T17:30:00Z"));
        pr.bot_check = Some(c);
        bot_review(&pr, None)
    }

    #[test]
    fn completed_check_conclusions() {
        use GhCheckStatus::Completed;
        let done = Some(at("2026-11-21T18:00:00Z"));
        assert_eq!(
            with_check(Completed, Some("SUCCESS"), None),
            BotReview::Approved { at: done }
        );
        assert_eq!(
            with_check(Completed, Some("FAILURE"), Some("Changes requested")),
            BotReview::ChangesRequested { at: done }
        );
        assert_eq!(
            with_check(Completed, Some("neutral"), None),
            BotReview::Commented { at: done }
        );
        assert_eq!(
            with_check(
                Completed,
                Some("SKIPPED"),
                Some("Not reviewed: the PR is a draft")
            ),
            BotReview::Draft
        );
        assert_eq!(
            with_check(Completed, Some("CANCELLED"), Some("Draft PR")),
            BotReview::Draft
        );
        assert_eq!(
            with_check(
                Completed,
                Some("CANCELLED"),
                Some("Superseded by a newer push")
            ),
            BotReview::Other {
                title: "Superseded by a newer push".into()
            }
        );
        assert_eq!(
            with_check(Completed, Some("TIMED_OUT"), None),
            BotReview::Other {
                title: "TIMED_OUT".into()
            }
        );
        assert_eq!(
            with_check(Completed, None, None),
            BotReview::Other {
                title: "Review finished".into()
            }
        );
    }

    #[test]
    fn in_progress_check_without_feed_is_reviewing_unknown() {
        let r = with_check(
            GhCheckStatus::InProgress,
            None,
            Some("Reviewing — Phase 1 review"),
        );
        assert_eq!(
            r,
            BotReview::Reviewing {
                phase: "unknown".into(),
                phase_label: "Phase 1".into(),
                progress: None,
                finishes_at: None,
                started_at: Some(at("2026-11-21T17:30:00Z")),
                overdue: false,
                tier: None,
                priority: false,
                waited_seconds: None,
                steps: vec![],
                findings: None,
            }
        );
        let BotReview::Reviewing { phase_label, .. } =
            with_check(GhCheckStatus::InProgress, None, Some("In progress"))
        else {
            panic!("expected reviewing");
        };
        assert_eq!(phase_label, "Reviewing");
    }

    #[test]
    fn queued_check_is_queued_with_nulls() {
        assert_eq!(
            with_check(GhCheckStatus::Queued, None, None),
            BotReview::Queued {
                position: None,
                starts_at: None,
                finishes_at: None,
                reason: None,
                priority: false
            }
        );
    }

    #[test]
    fn no_check_and_no_feed_is_none() {
        let mut pr = pr();
        pr.bot_check = None;
        assert_eq!(bot_review(&pr, None), BotReview::None);
        let empty = feed_with(vec![], vec![]);
        assert_eq!(bot_review(&pr, Some(&empty)), BotReview::None);
    }

    #[test]
    fn phase_titles() {
        assert_eq!(
            phase_from_title("Reviewing — Phase 1 review").as_deref(),
            Some("Phase 1")
        );
        assert_eq!(
            phase_from_title("Reviewing – Verify").as_deref(),
            Some("Verify")
        );
        assert_eq!(
            phase_from_title("Reviewing - Phase 2 Review").as_deref(),
            Some("Phase 2")
        );
        assert_eq!(phase_from_title("Reviewing"), None);
        assert_eq!(phase_from_title("Reviewing — "), None);
        assert_eq!(
            phase_from_title("Reviewing — review").as_deref(),
            Some("review")
        );
    }
}
