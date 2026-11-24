//! Reads one CodeRabbit finding from the root comment of its review thread.
//!
//! The format is CodeRabbit's own, with no stability promise, so this is
//! tolerant. Current comments look like:
//!
//! ```text
//! <!-- coderabbit-ai-generated:v1 -->
//!
//! **<picture>...</picture> Major** · **Security & Privacy** · **Quick win**
//!
//! **Reject symlinks before checking the bundle's file types.**
//! ...
//! <!-- cr-indicator-types:potential_issue -->
//! ...
//! ✅ Addressed in commits 39e8c9a to 17f5a8c
//! ```
//!
//! Older ones open with italic labels instead, e.g.
//! `_⚠️ Potential issue_ | _🟠 Major_` or
//! `_🎯 Functional Correctness_ | _🟡 Minor_ | _⚡ Quick win_`, and the
//! oldest name no severity at all (`_⚠️ Potential issue_`).
//!
//! The headline is the first line of the body that is neither blank nor an
//! HTML comment. Its segments are split on `|` and `·`, and the first that
//! reads Critical, Major, Minor or Trivial is the severity.

use super::types::{GhFindingStatus, GhSeverity};

/// CodeRabbit's login as GraphQL reports a comment's author.
pub(crate) const LOGIN: &str = "coderabbitai";
/// The same account as the `author` argument of `reviews` wants it: the
/// plain login matches nothing.
pub(crate) const REVIEW_AUTHOR: &str = "coderabbitai[bot]";

const INDICATOR_MARKER: &str = "<!-- cr-indicator-types:";
/// CodeRabbit appends this line to its own comment once it sees the fix.
const ADDRESSED: &str = "✅ Addressed in commit";
/// Labels of the oldest headlines, which carry no severity word.
const OLD_LABELS: [&str; 3] = ["potential issue", "refactor suggestion", "nitpick"];

/// Severity and status of a CodeRabbit finding, from its root comment.
/// `None` when the comment is not a finding: it has neither the
/// `cr-indicator-types` marker nor a headline in CodeRabbit's shape.
pub(crate) fn parse_coderabbit(body: &str) -> Option<(GhSeverity, GhFindingStatus)> {
    let types = indicator_types(body);
    let headline = headline(body);
    let word = headline.as_deref().and_then(severity_word);
    let old_label = headline.as_deref().is_some_and(has_old_label);
    if types.is_none() && word.is_none() && !old_label {
        return None;
    }
    let nitpick_type = types
        .iter()
        .flatten()
        .any(|t| t.eq_ignore_ascii_case("nitpick"));
    let severity = match word {
        _ if nitpick_type => GhSeverity::Nitpick,
        Some(Word::Critical | Word::Major) => GhSeverity::Blocking,
        Some(Word::Minor) => GhSeverity::Suggestion,
        // Trivial, or no severity at all.
        Some(Word::Trivial) | None => GhSeverity::Nitpick,
    };
    let status = if body.lines().any(|l| l.trim_start().starts_with(ADDRESSED)) {
        GhFindingStatus::Resolved
    } else {
        GhFindingStatus::Stands
    };
    Some((severity, status))
}

/// The types listed in `<!-- cr-indicator-types:a,b -->`, if the marker is
/// there.
fn indicator_types(body: &str) -> Option<Vec<String>> {
    let (_, rest) = body.split_once(INDICATOR_MARKER)?;
    let (list, _) = rest.split_once("-->")?;
    Some(
        list.split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect(),
    )
}

/// The first line that is not blank and not an HTML comment, with tags
/// (`<picture>`, `<img>`) removed.
fn headline(body: &str) -> Option<String> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("<!--"))?;
    Some(strip_tags(line))
}

fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The headline's segments, each without emphasis marks or a leading icon.
fn segments(headline: &str) -> impl Iterator<Item = String> + '_ {
    headline.split(['|', '·']).map(|seg| {
        seg.trim_matches(|c: char| c == '*' || c == '_' || c.is_whitespace())
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim_matches(|c: char| c == '*' || c == '_' || c.is_whitespace())
            .to_lowercase()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Word {
    Critical,
    Major,
    Minor,
    Trivial,
}

fn severity_word(headline: &str) -> Option<Word> {
    segments(headline).find_map(|s| match s.as_str() {
        "critical" => Some(Word::Critical),
        "major" => Some(Word::Major),
        "minor" => Some(Word::Minor),
        "trivial" => Some(Word::Trivial),
        _ => None,
    })
}

fn has_old_label(headline: &str) -> bool {
    segments(headline).any(|s| OLD_LABELS.iter().any(|l| s.starts_with(l)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use GhFindingStatus::*;
    use GhSeverity::*;

    const GENERATED: &str = "<!-- coderabbit-ai-generated:v1 -->";

    /// A comment in the current format with `word` as its severity.
    fn current(word: &str, types: &str, addressed: bool) -> String {
        let mut body = format!(
            "{GENERATED}\n\n**<picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"x.svg\"><img src=\"y.svg\" width=\"16\" height=\"16\" align=\"absmiddle\" alt=\"\"></picture> {word}** · **Security & Privacy** · **Quick win**\n\n**Do the thing.**\n\nBecause.\n\n<!-- cr-indicator-types:{types} -->\n\n<!-- cr-comment:v1:0af6a8f1 -->"
        );
        if addressed {
            body.push_str(
                "\n\n<!-- This is an auto-generated reply by CodeRabbit -->\n\n✅ Addressed in commits 39e8c9a to 17f5a8c",
            );
        }
        body
    }

    #[test]
    fn each_severity_word() {
        let cases = [
            ("Critical", Blocking),
            ("Major", Blocking),
            ("Minor", Suggestion),
            ("Trivial", Nitpick),
        ];
        for (word, want) in cases {
            assert_eq!(
                parse_coderabbit(&current(word, "potential_issue", false)),
                Some((want, Stands)),
                "{word}"
            );
        }
    }

    #[test]
    fn an_unknown_word_with_the_marker_is_a_nitpick() {
        assert_eq!(
            parse_coderabbit(&current("Severe", "potential_issue", false)),
            Some((Nitpick, Stands))
        );
    }

    #[test]
    fn nitpick_types_are_nitpicks() {
        assert_eq!(
            parse_coderabbit(&current("Major", "nitpick", false)),
            Some((Nitpick, Stands))
        );
        assert_eq!(
            parse_coderabbit(&current("Minor", "potential_issue, nitpick", false)),
            Some((Nitpick, Stands))
        );
        // A refactor suggestion keeps its word.
        assert_eq!(
            parse_coderabbit(&current("Minor", "refactor_suggestion", false)),
            Some((Suggestion, Stands))
        );
    }

    #[test]
    fn addressed_is_resolved() {
        assert_eq!(
            parse_coderabbit(&current("Major", "potential_issue", true)),
            Some((Blocking, Resolved))
        );
        let one = current("Minor", "potential_issue", false) + "\n\n✅ Addressed in commit f508b5f";
        assert_eq!(parse_coderabbit(&one), Some((Suggestion, Resolved)));
    }

    #[test]
    fn older_italic_headlines() {
        let cases = [
            ("_⚠️ Potential issue_ | _🟠 Major_", Blocking),
            ("_⚠️ Potential issue_ | _🔴 Critical_", Blocking),
            ("_🛠️ Refactor suggestion_ | _🟠 Major_", Blocking),
            ("_⚠️ Potential issue_ | _🟡 Minor_", Suggestion),
            (
                "_📐 Maintainability & Code Quality_ | _🟡 Minor_ | _⚡ Quick win_",
                Suggestion,
            ),
            ("_🧹 Nitpick_ | _🔵 Trivial_", Nitpick),
            // The oldest: a label but no severity.
            ("_⚠️ Potential issue_", Nitpick),
            ("_🛠️ Refactor suggestion_", Nitpick),
        ];
        for (headline, want) in cases {
            let body = format!("{headline}\n\n**Check the reference.**\n\nBody.");
            assert_eq!(parse_coderabbit(&body), Some((want, Stands)), "{headline}");
        }
        let addressed = "_⚠️ Potential issue_\n\n**Fix it.**\n\n✅ Addressed in commit f508b5f";
        assert_eq!(parse_coderabbit(addressed), Some((Nitpick, Resolved)));
    }

    #[test]
    fn replies_and_other_comments_are_not_findings() {
        assert_eq!(parse_coderabbit(""), None);
        assert_eq!(
            parse_coderabbit(
                "<!-- This is an auto-generated reply by CodeRabbit -->\n\n`@someone` Thanks, noted."
            ),
            None
        );
        assert_eq!(
            parse_coderabbit(
                "_💡 Verification agent_\n\n<details>\n<summary>❓ Verification inconclusive</summary>"
            ),
            None
        );
        // A severity word lower in the body is not a headline.
        assert_eq!(
            parse_coderabbit("Thanks!\n\n**<picture></picture> Major** · x"),
            None
        );
    }

    #[test]
    fn windows_line_endings() {
        let body = current("Minor", "potential_issue", true).replace('\n', "\r\n");
        assert_eq!(parse_coderabbit(&body), Some((Suggestion, Resolved)));
    }
}
