//! The pipeline of a running review, as the UI shows it.
//!
//! The feed's setup steps (worktree, select, triage, context) are folded
//! into one "Triage" step; the rest follow in pipeline order, then the steps
//! not yet started.

use crate::feed::FeedActive;
use crate::model::{ReviewStep, StepStatus};

/// Setup steps shown as the single step `TRIAGE_KEY`.
const SETUP: [&str; 4] = ["worktree", "select", "triage", "context"];
const TRIAGE_KEY: &str = "triage";

/// Known steps in the order the bot runs them. Seen in the live feed;
/// steps not listed here keep the place the feed gives them.
const PIPELINE: [&str; 8] = [
    "phase1", "verify1", "gate", "phase2", "verify2", "stepback", "confirm", "publish",
];

/// Display form of a feed step or phase name: "phase1" -> "Phase 1".
pub fn label(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "triage" => "Triage".to_string(),
        "phase1" => "Phase 1".to_string(),
        "verify1" => "Verify".to_string(),
        "gate" => "Gate".to_string(),
        "phase2" => "Phase 2".to_string(),
        "verify2" => "Verify 2".to_string(),
        "publish" => "Publish".to_string(),
        _ => title_case(key),
    }
}

/// "step_back" -> "Step Back", "phase3" -> "Phase 3".
fn title_case(key: &str) -> String {
    key.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let digits = word.len() - word.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            let (letters, number) = word.split_at(word.len() - digits);
            let mut out = String::new();
            let mut chars = letters.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(&chars.as_str().to_lowercase());
            }
            if !number.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(number);
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn step_status(status: Option<&str>) -> StepStatus {
    match status.map(str::to_ascii_lowercase).as_deref() {
        Some("ok" | "done" | "completed" | "complete" | "success" | "skipped") => StepStatus::Done,
        Some("running" | "in_progress" | "active") => StepStatus::Running,
        Some(
            "failed" | "failure" | "error" | "errored" | "timeout" | "timed_out" | "cancelled"
            | "canceled" | "aborted",
        ) => StepStatus::Failed,
        _ => StepStatus::Pending,
    }
}

fn is_setup(name: &str) -> bool {
    SETUP.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// Position in `PIPELINE`, if known.
fn rank(name: &str) -> Option<usize> {
    PIPELINE.iter().position(|s| s.eq_ignore_ascii_case(name))
}

/// The whole pipeline of a running review: triage, then the started steps
/// in pipeline order, then the upcoming ones.
pub fn review_steps(active: &FeedActive) -> Vec<ReviewStep> {
    let mut out = Vec::new();

    // Setup statuses: started ones as reported, upcoming ones as pending.
    let setup: Vec<StepStatus> = active
        .steps
        .iter()
        .filter(|s| is_setup(&s.name))
        .map(|s| step_status(s.status.as_deref()))
        .chain(
            active
                .upcoming
                .iter()
                .filter(|n| is_setup(n))
                .map(|_| StepStatus::Pending),
        )
        .collect();
    if !setup.is_empty() {
        out.push(ReviewStep {
            key: TRIAGE_KEY.to_string(),
            label: label(TRIAGE_KEY),
            status: fold(&setup),
            detail: None,
        });
    }

    // Started steps, one per name (the latest report wins), sorted into
    // pipeline order. A step we do not know keeps its place after the step
    // the feed listed before it.
    let mut started: Vec<(usize, ReviewStep)> = Vec::new();
    let mut last_rank = 0;
    for step in active.steps.iter().filter(|s| !is_setup(&s.name)) {
        let status = step_status(step.status.as_deref());
        let detail = match (status, step.lanes_total) {
            (StepStatus::Running, Some(total)) if total > 0 => {
                let done = step.lanes_done.unwrap_or(0).min(total);
                let noun = if total == 1 { "lane" } else { "lanes" };
                Some(format!("{done} of {total} {noun}"))
            }
            _ => None,
        };
        let entry = ReviewStep {
            key: step.name.clone(),
            label: label(&step.name),
            status,
            detail,
        };
        if let Some(existing) = started
            .iter_mut()
            .find(|(_, s)| s.key.eq_ignore_ascii_case(&step.name))
        {
            existing.1 = entry;
            continue;
        }
        last_rank = rank(&step.name).unwrap_or(last_rank);
        started.push((last_rank, entry));
    }
    started.sort_by_key(|(r, _)| *r);
    out.extend(started.into_iter().map(|(_, s)| s));

    for name in active.upcoming.iter().filter(|n| !is_setup(n)) {
        if out.iter().any(|s| s.key.eq_ignore_ascii_case(name)) {
            continue;
        }
        out.push(ReviewStep {
            key: name.clone(),
            label: label(name),
            status: StepStatus::Pending,
            detail: None,
        });
    }
    out
}

/// One status for a group of steps: failed if any failed, done when all
/// are done, pending when none has started, otherwise running.
fn fold(statuses: &[StepStatus]) -> StepStatus {
    if statuses.contains(&StepStatus::Failed) {
        StepStatus::Failed
    } else if statuses.iter().all(|s| *s == StepStatus::Done) {
        StepStatus::Done
    } else if statuses.iter().all(|s| *s == StepStatus::Pending) {
        StepStatus::Pending
    } else {
        StepStatus::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::FeedStep;
    use pretty_assertions::assert_eq;

    fn step(name: &str, status: &str) -> FeedStep {
        FeedStep {
            name: name.into(),
            status: Some(status.into()),
            ..FeedStep::default()
        }
    }

    fn lanes(mut s: FeedStep, done: u32, total: u32) -> FeedStep {
        s.lanes_done = Some(done);
        s.lanes_total = Some(total);
        s
    }

    fn active(steps: Vec<FeedStep>, upcoming: &[&str]) -> FeedActive {
        FeedActive {
            repo: "o/r".into(),
            number: 1,
            steps,
            upcoming: upcoming.iter().map(|s| s.to_string()).collect(),
            ..FeedActive::default()
        }
    }

    fn summary(steps: &[ReviewStep]) -> Vec<(String, String, StepStatus, Option<String>)> {
        steps
            .iter()
            .map(|s| (s.key.clone(), s.label.clone(), s.status, s.detail.clone()))
            .collect()
    }

    fn row(
        key: &str,
        label: &str,
        status: StepStatus,
        detail: Option<&str>,
    ) -> (String, String, StepStatus, Option<String>) {
        (key.into(), label.into(), status, detail.map(Into::into))
    }

    #[test]
    fn folds_setup_and_lists_the_pipeline_in_order() {
        let a = active(
            vec![
                step("worktree", "ok"),
                step("select", "ok"),
                step("triage", "ok"),
                step("context", "ok"),
                lanes(step("phase1", "running"), 0, 2),
            ],
            &["verify1", "gate", "phase2", "verify2", "publish"],
        );
        use StepStatus::*;
        assert_eq!(
            summary(&review_steps(&a)),
            vec![
                row("triage", "Triage", Done, None),
                row("phase1", "Phase 1", Running, Some("0 of 2 lanes")),
                row("verify1", "Verify", Pending, None),
                row("gate", "Gate", Pending, None),
                row("phase2", "Phase 2", Pending, None),
                row("verify2", "Verify 2", Pending, None),
                row("publish", "Publish", Pending, None),
            ]
        );
    }

    #[test]
    fn setup_is_running_while_any_setup_step_runs() {
        let a = active(
            vec![step("worktree", "ok"), step("select", "running")],
            &["triage", "context", "phase1"],
        );
        let steps = review_steps(&a);
        assert_eq!(steps[0].key, "triage");
        assert_eq!(steps[0].status, StepStatus::Running);
        assert_eq!(steps.len(), 2, "upcoming setup names fold into triage");
        assert_eq!(steps[1].key, "phase1");
    }

    #[test]
    fn setup_folding_statuses() {
        use StepStatus::*;
        assert_eq!(fold(&[Done, Done]), Done);
        assert_eq!(fold(&[Done, Running]), Running);
        assert_eq!(fold(&[Done, Pending]), Running);
        assert_eq!(fold(&[Pending, Pending]), Pending);
        assert_eq!(fold(&[Done, Failed, Running]), Failed);
    }

    #[test]
    fn only_upcoming_setup_is_pending_triage() {
        let a = active(vec![], &["worktree", "select", "phase1"]);
        let steps = review_steps(&a);
        assert_eq!(steps[0].status, StepStatus::Pending);
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn no_steps_at_all_is_an_empty_pipeline() {
        assert!(review_steps(&active(vec![], &[])).is_empty());
    }

    #[test]
    fn started_steps_are_sorted_into_pipeline_order() {
        // As seen in the live feed for a critical-tier run.
        let a = active(
            vec![
                step("context", "ok"),
                lanes(step("phase2", "ok"), 6, 6),
                lanes(step("phase1", "ok"), 2, 2),
                step("verify2", "running"),
            ],
            &["stepback", "publish"],
        );
        let keys: Vec<_> = review_steps(&a)
            .into_iter()
            .map(|s| (s.key, s.label, s.detail))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("triage".into(), "Triage".into(), None),
                ("phase1".into(), "Phase 1".into(), None),
                ("phase2".into(), "Phase 2".into(), None),
                ("verify2".into(), "Verify 2".into(), None),
                ("stepback".into(), "Stepback".into(), None),
                ("publish".into(), "Publish".into(), None),
            ]
        );
    }

    #[test]
    fn statuses_and_unknown_steps() {
        let a = active(
            vec![
                step("phase1", "failed"),
                step("lint_pass", "weird-new-status"),
                lanes(step("phase2", "running"), 1, 1),
            ],
            &["phase2", "publish"],
        );
        use StepStatus::*;
        assert_eq!(
            summary(&review_steps(&a)),
            vec![
                row("phase1", "Phase 1", Failed, None),
                row("lint_pass", "Lint Pass", Pending, None),
                row("phase2", "Phase 2", Running, Some("1 of 1 lane")),
                row("publish", "Publish", Pending, None),
            ]
        );
    }

    #[test]
    fn confirm_step_sits_before_publish() {
        // As seen in the live feed near the end of a run.
        let a = active(
            vec![
                step("context", "ok"),
                step("verify2", "ok"),
                step("confirm", "running"),
            ],
            &["publish"],
        );
        let keys: Vec<_> = review_steps(&a)
            .into_iter()
            .map(|s| (s.key, s.label, s.status))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("triage".into(), "Triage".into(), StepStatus::Done),
                ("verify2".into(), "Verify 2".into(), StepStatus::Done),
                ("confirm".into(), "Confirm".into(), StepStatus::Running),
                ("publish".into(), "Publish".into(), StepStatus::Pending),
            ]
        );
    }

    #[test]
    fn a_repeated_step_keeps_its_latest_report() {
        let a = active(
            vec![step("phase1", "failed"), step("phase1", "running")],
            &[],
        );
        let steps = review_steps(&a);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].status, StepStatus::Running);
    }

    #[test]
    fn labels() {
        assert_eq!(label("phase1"), "Phase 1");
        assert_eq!(label("verify1"), "Verify");
        assert_eq!(label("verify2"), "Verify 2");
        assert_eq!(label("gate"), "Gate");
        assert_eq!(label("publish"), "Publish");
        assert_eq!(label("phase3"), "Phase 3");
        assert_eq!(label("deep-dive"), "Deep Dive");
        assert_eq!(label("CI_wait"), "Ci Wait");
        assert_eq!(label("42"), "42");
    }
}
