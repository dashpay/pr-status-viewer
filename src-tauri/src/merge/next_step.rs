//! Reads the repo's "PR Hygiene" gate into a `NextStep`. Pure.
//!
//! The gate's description is `<stage>: <sentence>`, or a bare stage code
//! such as "ready-to-merge". Stage codes we do not know are still shown,
//! as a cleaned-up version of the code.

use crate::github::types::GhHygiene;
use crate::model::{NextActor, NextStep};

/// The next step the gate reports, or `None` when it has no description.
/// `is_mine` picks "Your move" over "Author's move".
pub(crate) fn next_step(hygiene: &GhHygiene, is_mine: bool) -> Option<NextStep> {
    let description = hygiene.description.as_deref()?.trim();
    if description.is_empty() {
        return None;
    }
    let (stage, detail) = match description.split_once(':') {
        Some((stage, detail)) => (stage, Some(detail.trim()).filter(|d| !d.is_empty())),
        None => (description, None),
    };
    let stage = stage.trim().to_lowercase();
    let (actor, label) = match stage.as_str() {
        "waiting-self-review" => (
            NextActor::Author,
            if is_mine {
                "Your move"
            } else {
                "Author's move"
            }
            .to_owned(),
        ),
        "waiting-bots" => (NextActor::Bots, "Waiting for bots".to_owned()),
        "ready-for-human" => (NextActor::Reviewers, "Ready for human review".to_owned()),
        "waiting-build" => (NextActor::Ci, "Waiting for CI".to_owned()),
        "draft" => (NextActor::Author, "Draft".to_owned()),
        "ready-to-merge" => (NextActor::Merge, "Ready to merge".to_owned()),
        other => (NextActor::Unknown, humanize(other)),
    };
    Some(NextStep {
        stage,
        label,
        detail: detail.map(str::to_owned),
        actor,
    })
}

/// "waiting-for-thing" -> "Waiting for thing".
fn humanize(stage: &str) -> String {
    let spaced = stage.replace('-', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// True when `text` names `login` as a whole word, ignoring case. A word
/// boundary is anything that cannot be part of a GitHub login, so "shum"
/// does not match "shumkov" and "bob" does not match "bob-two".
pub(crate) fn names_login(text: &str, login: &str) -> bool {
    if login.is_empty() {
        return false;
    }
    let text = text.to_ascii_lowercase();
    let login = login.to_ascii_lowercase();
    let is_login_char = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(&login).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + login.len()..].chars().next();
        !before.is_some_and(is_login_char) && !after.is_some_and(is_login_char)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn gate(description: Option<&str>) -> GhHygiene {
        GhHygiene {
            state: "PENDING".into(),
            description: description.map(Into::into),
        }
    }

    fn step(description: &str, is_mine: bool) -> NextStep {
        next_step(&gate(Some(description)), is_mine).expect("a next step")
    }

    fn summary(s: &NextStep) -> (&str, &str, Option<&str>, NextActor) {
        (&s.stage, &s.label, s.detail.as_deref(), s.actor)
    }

    #[test]
    fn every_known_stage() {
        use NextActor::*;
        let cases = [
            (
                "waiting-self-review: Bots are done - your move: post /self-reviewed.",
                true,
                (
                    "waiting-self-review",
                    "Your move",
                    Some("Bots are done - your move: post /self-reviewed."),
                    Author,
                ),
            ),
            (
                "waiting-self-review: Your move: thepastaclaw requested changes on this head; \
                 dismiss the review or push a fix.",
                false,
                (
                    "waiting-self-review",
                    "Author's move",
                    Some(
                        "Your move: thepastaclaw requested changes on this head; \
                         dismiss the review or push a fix.",
                    ),
                    Author,
                ),
            ),
            (
                "waiting-bots: Waiting for bot review - thepastaclaw not yet.",
                true,
                (
                    "waiting-bots",
                    "Waiting for bots",
                    Some("Waiting for bot review - thepastaclaw not yet."),
                    Bots,
                ),
            ),
            (
                "ready-for-human: Ready for review - dpp: QuantumExplorer or shumkov",
                false,
                (
                    "ready-for-human",
                    "Ready for human review",
                    Some("Ready for review - dpp: QuantumExplorer or shumkov"),
                    Reviewers,
                ),
            ),
            (
                "waiting-build: Waiting for required checks.",
                true,
                (
                    "waiting-build",
                    "Waiting for CI",
                    Some("Waiting for required checks."),
                    Ci,
                ),
            ),
            (
                "draft: The PR is a draft.",
                true,
                ("draft", "Draft", Some("The PR is a draft."), Author),
            ),
            (
                "ready-to-merge",
                false,
                ("ready-to-merge", "Ready to merge", None, Merge),
            ),
        ];
        for (description, is_mine, want) in cases {
            assert_eq!(summary(&step(description, is_mine)), want, "{description}");
        }
    }

    #[test]
    fn unknown_stage_is_humanized() {
        let got = step("needs-owner-sign-off: Ask an owner.", true);
        assert_eq!(
            summary(&got),
            (
                "needs-owner-sign-off",
                "Needs owner sign off",
                Some("Ask an owner."),
                NextActor::Unknown
            )
        );
    }

    #[test]
    fn stage_is_trimmed_and_lower_cased() {
        let got = step("  Ready-To-Merge  ", false);
        assert_eq!(
            summary(&got),
            ("ready-to-merge", "Ready to merge", None, NextActor::Merge)
        );
        let got = step(" WAITING-BOTS :   ", false);
        assert_eq!(
            summary(&got),
            ("waiting-bots", "Waiting for bots", None, NextActor::Bots)
        );
    }

    #[test]
    fn no_description_means_no_step() {
        assert_eq!(next_step(&gate(None), true), None);
        assert_eq!(next_step(&gate(Some("  ")), true), None);
    }

    #[test]
    fn names_login_whole_words_only() {
        let text = "Ready for review - dpp: QuantumExplorer or shumkov | wallet: bob-two";
        assert!(names_login(text, "shumkov"));
        assert!(names_login(text, "SHUMKOV"));
        assert!(names_login(text, "quantumexplorer"));
        assert!(names_login(text, "bob-two"));
        // Prefixes and pieces of other logins.
        assert!(!names_login(text, "shum"));
        assert!(!names_login(text, "Quantum"));
        assert!(!names_login(text, "bob"));
        assert!(!names_login(text, "two"));
        assert!(!names_login(text, ""));
        // A later whole-word hit counts even after a partial one.
        assert!(names_login("alicea, alice", "alice"));
        assert!(names_login("alice", "alice"));
    }
}
