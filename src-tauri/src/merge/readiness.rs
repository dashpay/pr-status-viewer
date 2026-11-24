//! Whether a PR can be squash-merged from the app, and if not, why.
//!
//! One rule list serves both the snapshot (from the facts of the last sync)
//! and the merge confirmation (from a fresh `merge_check`). First match wins.

use crate::github::types::{GhMergeCheck, GhMergeFacts, GhMergeable};
use crate::model::{MergeCheck, MergeReadiness};

/// Shown when a fresh check finds the PR merged or closed.
pub const CLOSED_REASON: &str = "This pull request is already merged or closed.";

/// The first thing in the way of a squash merge, or ready. In order:
///
/// 1. the viewer lacks write access;
/// 2. the repo does not allow squash merges;
/// 3. the PR is a draft;
/// 4. review threads are unresolved;
/// 5. merge conflicts;
/// 6. the repo uses a merge queue (merging must go through it, on GitHub);
/// 7. the branch is behind its base and the rules require it up to date;
/// 8. branch protection blocks it (a required review or check is missing);
/// 9. GitHub has not worked it out yet, or says something we do not know;
/// 10. otherwise ready: CLEAN, HAS_HOOKS or UNSTABLE, and mergeable.
///     UNSTABLE means only checks that are not required are failing, which
///     GitHub allows, so the app does too.
pub fn readiness(facts: &GhMergeFacts, is_draft: bool) -> MergeReadiness {
    let state = facts.merge_state.as_str();
    let base = if facts.base_branch.is_empty() {
        "the base branch"
    } else {
        facts.base_branch.as_str()
    };
    let reason = if !facts.viewer_can_merge {
        "You do not have permission to merge in this repository.".to_owned()
    } else if !facts.squash_allowed {
        "This repository does not allow squash merges.".to_owned()
    } else if is_draft || state == "DRAFT" {
        "Draft pull requests cannot be merged.".to_owned()
    } else if facts.unresolved_threads > 0 {
        unresolved_comments(facts.unresolved_threads)
    } else if has_conflicts(facts) {
        format!("Merge conflicts with {base}.")
    } else if facts.merge_queue {
        "This repository uses a merge queue. Merge it on GitHub.".to_owned()
    } else if state == "BEHIND" {
        format!("The branch is behind {base}. Update it first.")
    } else if state == "BLOCKED" {
        "GitHub's branch rules block this merge: a required review or check is missing.".to_owned()
    } else if facts.mergeable == GhMergeable::Mergeable
        && matches!(state, "CLEAN" | "HAS_HOOKS" | "UNSTABLE")
    {
        return MergeReadiness {
            ready: true,
            reason: None,
        };
    } else {
        "GitHub is still working out whether this can merge.".to_owned()
    };
    MergeReadiness {
        ready: false,
        reason: Some(reason),
    }
}

/// GitHub reports merge conflicts with the base branch: `mergeable` is
/// CONFLICTING or the merge state is DIRTY. False while GitHub has not
/// worked it out (UNKNOWN), whatever comes before it in `readiness`.
pub(crate) fn has_conflicts(facts: &GhMergeFacts) -> bool {
    facts.mergeable == GhMergeable::Conflicting || facts.merge_state == "DIRTY"
}

fn unresolved_comments(n: u32) -> String {
    if n == 1 {
        "1 unresolved comment".to_owned()
    } else {
        format!("{n} unresolved comments")
    }
}

/// The answer to the merge confirmation's "can this merge now?". A PR that
/// is no longer open is never ready.
pub fn merge_check(id: String, check: &GhMergeCheck) -> MergeCheck {
    let merge = if check.is_open {
        readiness(&check.facts, check.is_draft)
    } else {
        MergeReadiness {
            ready: false,
            reason: Some(CLOSED_REASON.to_owned()),
        }
    };
    MergeCheck {
        id,
        head_sha: check.head_sha.clone(),
        base_branch: check.facts.base_branch.clone(),
        commit_title: check.commit_title.clone(),
        unresolved_threads: check.facts.unresolved_threads,
        merge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A PR GitHub would merge right now.
    fn clean() -> GhMergeFacts {
        GhMergeFacts {
            node_id: "PR_1".into(),
            base_branch: "main".into(),
            review_threads: 3,
            unresolved_threads: 0,
            mergeable: GhMergeable::Mergeable,
            merge_state: "CLEAN".into(),
            squash_allowed: true,
            viewer_can_merge: true,
            merge_queue: false,
        }
    }

    fn reason(facts: &GhMergeFacts, draft: bool) -> Option<String> {
        let r = readiness(facts, draft);
        assert_eq!(r.ready, r.reason.is_none(), "{facts:?}");
        r.reason
    }

    fn with(f: impl FnOnce(&mut GhMergeFacts)) -> GhMergeFacts {
        let mut facts = clean();
        f(&mut facts);
        facts
    }

    #[test]
    fn ready_states() {
        for state in ["CLEAN", "HAS_HOOKS", "UNSTABLE"] {
            let r = readiness(&with(|f| f.merge_state = state.into()), false);
            assert_eq!(
                r,
                MergeReadiness {
                    ready: true,
                    reason: None
                },
                "{state}"
            );
        }
    }

    #[test]
    fn conflicts_whatever_comes_first() {
        assert!(!has_conflicts(&clean()));
        assert!(has_conflicts(&with(
            |f| f.mergeable = GhMergeable::Conflicting
        )));
        assert!(has_conflicts(&with(|f| {
            f.mergeable = GhMergeable::Unknown;
            f.merge_state = "DIRTY".into();
        })));
        // Not worked out yet, or in the way for another reason: no conflicts.
        assert!(!has_conflicts(&with(|f| {
            f.mergeable = GhMergeable::Unknown;
            f.merge_state = "UNKNOWN".into();
        })));
        assert!(!has_conflicts(&with(|f| f.merge_state = "BEHIND".into())));
        // The merge reason names the first rule; the flag still says it.
        let both = with(|f| {
            f.unresolved_threads = 2;
            f.mergeable = GhMergeable::Conflicting;
        });
        assert_eq!(
            reason(&both, false).as_deref(),
            Some("2 unresolved comments")
        );
        assert!(has_conflicts(&both));
    }

    #[test]
    fn each_rule_alone() {
        let s = |x: &str| Some(x.to_owned());
        let cases: Vec<(GhMergeFacts, bool, Option<String>)> = vec![
            (
                with(|f| f.viewer_can_merge = false),
                false,
                s("You do not have permission to merge in this repository."),
            ),
            (
                with(|f| f.squash_allowed = false),
                false,
                s("This repository does not allow squash merges."),
            ),
            (clean(), true, s("Draft pull requests cannot be merged.")),
            (
                with(|f| f.merge_state = "DRAFT".into()),
                false,
                s("Draft pull requests cannot be merged."),
            ),
            (
                with(|f| f.unresolved_threads = 1),
                false,
                s("1 unresolved comment"),
            ),
            (
                with(|f| f.unresolved_threads = 17),
                false,
                s("17 unresolved comments"),
            ),
            (
                with(|f| f.mergeable = GhMergeable::Conflicting),
                false,
                s("Merge conflicts with main."),
            ),
            (
                with(|f| f.merge_state = "DIRTY".into()),
                false,
                s("Merge conflicts with main."),
            ),
            (
                with(|f| f.merge_queue = true),
                false,
                s("This repository uses a merge queue. Merge it on GitHub."),
            ),
            (
                with(|f| f.merge_state = "BEHIND".into()),
                false,
                s("The branch is behind main. Update it first."),
            ),
            (
                with(|f| f.merge_state = "BLOCKED".into()),
                false,
                s("GitHub's branch rules block this merge: a required review or check is missing."),
            ),
            (
                with(|f| f.mergeable = GhMergeable::Unknown),
                false,
                s("GitHub is still working out whether this can merge."),
            ),
            (
                with(|f| f.merge_state = "UNKNOWN".into()),
                false,
                s("GitHub is still working out whether this can merge."),
            ),
            (
                with(|f| f.merge_state = "SOMETHING_NEW".into()),
                false,
                s("GitHub is still working out whether this can merge."),
            ),
            (
                with(|f| f.merge_state = String::new()),
                false,
                s("GitHub is still working out whether this can merge."),
            ),
        ];
        for (facts, draft, want) in cases {
            assert_eq!(reason(&facts, draft), want, "{facts:?} draft={draft}");
        }
    }

    #[test]
    fn rules_apply_in_order() {
        // Everything wrong at once: each fix reveals the next rule.
        let mut facts = GhMergeFacts {
            node_id: "PR_1".into(),
            base_branch: "v5.2-dev".into(),
            review_threads: 4,
            unresolved_threads: 2,
            mergeable: GhMergeable::Conflicting,
            merge_state: "BEHIND".into(),
            squash_allowed: false,
            viewer_can_merge: false,
            merge_queue: true,
        };
        let mut seen = vec![reason(&facts, true)];
        facts.viewer_can_merge = true;
        seen.push(reason(&facts, true));
        facts.squash_allowed = true;
        seen.push(reason(&facts, true));
        seen.push(reason(&facts, false));
        facts.unresolved_threads = 0;
        seen.push(reason(&facts, false));
        facts.mergeable = GhMergeable::Mergeable;
        seen.push(reason(&facts, false));
        facts.merge_queue = false;
        seen.push(reason(&facts, false));
        facts.merge_state = "BLOCKED".into();
        seen.push(reason(&facts, false));
        facts.merge_state = "UNKNOWN".into();
        seen.push(reason(&facts, false));
        facts.merge_state = "CLEAN".into();
        seen.push(reason(&facts, false));
        let seen: Vec<String> = seen
            .into_iter()
            .map(|r| r.unwrap_or_else(|| "ready".into()))
            .collect();
        assert_eq!(
            seen,
            vec![
                "You do not have permission to merge in this repository.",
                "This repository does not allow squash merges.",
                "Draft pull requests cannot be merged.",
                "2 unresolved comments",
                "Merge conflicts with v5.2-dev.",
                "This repository uses a merge queue. Merge it on GitHub.",
                "The branch is behind v5.2-dev. Update it first.",
                "GitHub's branch rules block this merge: a required review or check is missing.",
                "GitHub is still working out whether this can merge.",
                "ready",
            ]
        );
    }

    #[test]
    fn unresolved_comments_beat_blocked_clean_and_unknown() {
        for state in ["BLOCKED", "CLEAN", "UNSTABLE", "UNKNOWN", "BEHIND", "DIRTY"] {
            let facts = with(|f| {
                f.merge_state = state.into();
                f.unresolved_threads = 3;
            });
            assert_eq!(
                reason(&facts, false).as_deref(),
                Some("3 unresolved comments"),
                "{state}"
            );
        }
    }

    #[test]
    fn blocked_beats_unknown_mergeable() {
        let facts = with(|f| {
            f.merge_state = "BLOCKED".into();
            f.mergeable = GhMergeable::Unknown;
        });
        assert!(
            reason(&facts, false)
                .unwrap()
                .starts_with("GitHub's branch rules")
        );
    }

    #[test]
    fn missing_base_branch_still_reads() {
        let facts = with(|f| {
            f.base_branch = String::new();
            f.merge_state = "BEHIND".into();
        });
        assert_eq!(
            reason(&facts, false).as_deref(),
            Some("The branch is behind the base branch. Update it first.")
        );
    }

    fn gh_check(is_open: bool) -> GhMergeCheck {
        GhMergeCheck {
            head_sha: "abc123".into(),
            is_draft: false,
            is_open,
            commit_title: "feat: thing (#7)".into(),
            facts: with(|f| f.unresolved_threads = 0),
        }
    }

    #[test]
    fn merge_check_carries_the_facts() {
        let check = merge_check("o/r#7".into(), &gh_check(true));
        assert_eq!(
            check,
            MergeCheck {
                id: "o/r#7".into(),
                head_sha: "abc123".into(),
                base_branch: "main".into(),
                commit_title: "feat: thing (#7)".into(),
                unresolved_threads: 0,
                merge: MergeReadiness {
                    ready: true,
                    reason: None
                },
            }
        );
    }

    #[test]
    fn a_closed_pr_is_never_ready() {
        let mut gh = gh_check(false);
        // Even with every other fact in order, and before the permission rule.
        gh.facts.viewer_can_merge = false;
        let check = merge_check("o/r#7".into(), &gh);
        assert!(!check.merge.ready);
        assert_eq!(check.merge.reason.as_deref(), Some(CLOSED_REASON));
    }
}
