//! Reads one review-bot finding from the root comment of its review thread.
//!
//! The format is the bot's own (thepastaclaw/review-system,
//! `reviewsys/publish.py`), with no stability promise, so this is tolerant:
//!
//! ```text
//! <!-- thepastaclaw-review v1 finding=b75cf39c3cc4 dedupe=e384482555294fe9 -->
//! <!-- thepastaclaw-thread-status v1 -->
//! > ✅ **Resolved** at `0daf80e5`; see the replies below.
//!
//! **🔴 Blocking: Normalize typed Kotlin DPNS names before reading or voting**
//! ```
//!
//! The status block is optional and sits in the run of HTML comments that
//! opens the body; it is read only from there, never from the body text.

use super::types::{GhFindingStatus, GhSeverity};

/// Starts the marker on every finding root, e.g.
/// `<!-- thepastaclaw-review v1 finding=<hex> dedupe=<id> [root=<id>] -->`.
const FINDING_MARKER: &str = "thepastaclaw-review v1";
/// Followed by the one quoted line the bot rewrites as the status changes.
const STATUS_MARKER: &str = "<!-- thepastaclaw-thread-status v1 -->";

/// Severity and status of a bot finding, from its root comment's body.
/// `None` when the body is not a finding: it carries neither the finding
/// marker nor a `**<icon> <Severity>: <title>**` headline.
pub(crate) fn parse_finding(body: &str) -> Option<(GhSeverity, GhFindingStatus)> {
    let headline = headline(body);
    let severity = if has_finding_marker(body) {
        // The bot reads an unknown or missing severity as a nitpick.
        headline.map_or(GhSeverity::Nitpick, |h| h.severity)
    } else {
        // Without the marker, only a headline in the bot's own shape counts.
        headline.filter(|h| h.titled && h.known)?.severity
    };
    Some((severity, status(body)))
}

fn has_finding_marker(body: &str) -> bool {
    body.match_indices(FINDING_MARKER).any(|(i, _)| {
        let rest = &body[i + FINDING_MARKER.len()..];
        let trimmed = rest.trim_start();
        trimmed.len() < rest.len()
            && trimmed
                .strip_prefix("finding=")
                .and_then(|id| id.chars().next())
                .is_some_and(|c| c.is_ascii_hexdigit())
    })
}

struct Headline {
    severity: GhSeverity,
    /// The label was one of the three severity words.
    known: bool,
    /// The line had a `label: title` shape.
    titled: bool,
}

/// The first bold line (`**...**`), as the bot's own `_finding_title` reads
/// it. A quoted line (`> ...`) is never a headline.
fn headline(body: &str) -> Option<Headline> {
    body.lines().find_map(|line| {
        let s = line.trim();
        if s.len() < 4 || !s.starts_with("**") || !s.ends_with("**") {
            return None;
        }
        let inner = s.trim_matches('*');
        let (label, titled) = match inner.split_once(':') {
            Some((label, title)) => (label, !title.trim().is_empty()),
            None => (inner, false),
        };
        let label = label.to_lowercase();
        let (severity, known) = if label.contains("blocking") {
            (GhSeverity::Blocking, true)
        } else if label.contains("suggestion") {
            (GhSeverity::Suggestion, true)
        } else if label.contains("nitpick") {
            (GhSeverity::Nitpick, true)
        } else {
            (GhSeverity::Nitpick, false)
        };
        Some(Headline {
            severity,
            known,
            titled,
        })
    })
}

/// The bot's word on the finding, from the quoted line right under the
/// status marker. The marker counts only in the run of HTML comments that
/// opens the body, which is where the bot puts it.
fn status(body: &str) -> GhFindingStatus {
    let mut lines = body.lines().skip_while(|l| l.trim().is_empty());
    while let Some(line) = lines.next() {
        let line = line.trim();
        if !line.starts_with("<!--") {
            break;
        }
        if line == STATUS_MARKER {
            return lines.next().map_or(GhFindingStatus::Stands, status_line);
        }
    }
    GhFindingStatus::Stands
}

/// `> ✅ **Resolved** at ...` -> Resolved. Anything else, "Needs a tracking
/// issue" included, leaves the finding standing.
fn status_line(line: &str) -> GhFindingStatus {
    let Some(quoted) = line.trim_start().strip_prefix('>') else {
        return GhFindingStatus::Stands;
    };
    let lead = quoted
        .split_once("**")
        .and_then(|(_, rest)| rest.split_once("**"))
        .map(|(lead, _)| lead.trim().to_lowercase());
    match lead.as_deref() {
        Some("resolved") => GhFindingStatus::Resolved,
        Some("withdrawn") => GhFindingStatus::Withdrawn,
        Some("no longer applies") => GhFindingStatus::NoLongerApplies,
        Some("deferred") => GhFindingStatus::Deferred,
        Some("disputed") => GhFindingStatus::Disputed,
        _ => GhFindingStatus::Stands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GhFindingStatus::*;
    use GhSeverity::*;

    const MARKER: &str =
        "<!-- thepastaclaw-review v1 finding=b75cf39c3cc4 dedupe=e384482555294fe9 -->";

    fn with_status(line: &str, headline: &str) -> String {
        format!(
            "{MARKER}\n{STATUS_MARKER}\n> {line} at `0daf80e5`; see the replies below.\n\n{headline}\n\nBody."
        )
    }

    #[test]
    fn every_status_line() {
        let h = "**🔴 Blocking: Do the thing**";
        let cases = [
            ("✅ **Resolved**", Resolved),
            ("✅ **Withdrawn**", Withdrawn),
            ("✅ **No longer applies**", NoLongerApplies),
            (
                "⏭️ **Deferred** (maintainer decision; not blocking)",
                Deferred,
            ),
            (
                "⏭️ **Deferred** (maintainer decision; not blocking; tracked in https://github.com/o/r/issues/1)",
                Deferred,
            ),
            (
                "⚖️ **Disputed** (left to the maintainers; not blocking)",
                Disputed,
            ),
            ("⏸️ **Needs a tracking issue** (not yet deferred)", Stands),
            ("🆕 **Something new**", Stands),
        ];
        for (line, want) in cases {
            assert_eq!(
                parse_finding(&with_status(line, h)),
                Some((Blocking, want)),
                "{line}"
            );
        }
    }

    #[test]
    fn no_status_block_stands() {
        let body = format!("{MARKER}\n**🟡 Suggestion: Do the thing**\n\nBody.");
        assert_eq!(parse_finding(&body), Some((Suggestion, Stands)));
    }

    #[test]
    fn severities() {
        let sev = |h: &str| parse_finding(&format!("{MARKER}\n{h}\n\nBody.")).map(|(s, _)| s);
        assert_eq!(sev("**🔴 Blocking: x**"), Some(Blocking));
        assert_eq!(sev("**🟡 Suggestion: x**"), Some(Suggestion));
        assert_eq!(sev("**💬 Nitpick: x**"), Some(Nitpick));
        // The bot reads an unknown severity as a nitpick.
        assert_eq!(sev("**💬 Critical: x**"), Some(Nitpick));
        // A marker with no headline at all is still a finding.
        assert_eq!(sev("Just text."), Some(Nitpick));
    }

    #[test]
    fn the_status_is_read_only_from_under_its_marker() {
        // Words that look like a status, lower in the body, do not count.
        let body = format!(
            "{MARKER}\n**🔴 Blocking: x**\n\nThe old code said:\n> ✅ **Resolved**\n\n{STATUS_MARKER}\n> ✅ **Withdrawn**"
        );
        assert_eq!(parse_finding(&body), Some((Blocking, Stands)));
        // A status marker not followed by a quote.
        let body = format!("{MARKER}\n{STATUS_MARKER}\n**🔴 Blocking: x**");
        assert_eq!(parse_finding(&body), Some((Blocking, Stands)));
    }

    #[test]
    fn a_root_id_and_windows_line_endings() {
        let body = with_status("✅ **No longer applies**", "**💬 Nitpick: x**")
            .replace(
                "dedupe=e384482555294fe9",
                "dedupe=e384482555294fe9 root=cause-1.a",
            )
            .replace('\n', "\r\n");
        assert_eq!(parse_finding(&body), Some((Nitpick, NoLongerApplies)));
    }

    #[test]
    fn without_the_marker_only_a_proper_headline_counts() {
        assert_eq!(
            parse_finding("**🔴 Blocking: Do the thing**\n\nBody."),
            Some((Blocking, Stands))
        );
        assert_eq!(parse_finding("**Critical: x**"), None);
        assert_eq!(parse_finding("**Blocking**"), None);
        assert_eq!(parse_finding("Thanks, fixed in abc123."), None);
        assert_eq!(parse_finding(""), None);
        // A broken marker is no marker.
        assert_eq!(
            parse_finding("<!-- thepastaclaw-review v1 finding= -->\nhello"),
            None
        );
    }
}
