//! Tolerant parsing of the feed JSON.
//!
//! The feed is read as a `serde_json::Value` and picked apart by hand rather
//! than through derived structs: a derived struct rejects a whole entry when
//! one field has an unexpected type, and the feed promises nothing about its
//! types. Here a field that is missing, null or mistyped simply reads as
//! absent, and an entry is dropped only when it lacks its identity
//! (`repo` and `number`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use super::{FeedActive, FeedQueued, FeedSnapshot, FeedStep, PathTiming};
use crate::error::{AppError, AppResult};
use crate::model::Findings;

/// Parses the feed JSON. Must tolerate missing and unknown fields: the feed
/// has no stability promise.
///
/// Errors only when the text is not JSON, is not a JSON object, or has
/// neither `live` nor `data_as_of`.
pub fn parse_feed(json: &str) -> AppResult<FeedSnapshot> {
    let doc: Value = serde_json::from_str(json)?;
    let Some(root) = doc.as_object() else {
        return Err(AppError::Decode(
            "the review feed is not a JSON object".to_string(),
        ));
    };
    let live = root.get("live").filter(|v| !v.is_null());
    let top_as_of = time(root.get("data_as_of"));
    if live.is_none() && top_as_of.is_none() {
        return Err(AppError::Decode(
            "the review feed has neither `live` nor `data_as_of`".to_string(),
        ));
    }
    let live = live.and_then(Value::as_object);

    let data_as_of = top_as_of
        .or_else(|| live.and_then(|l| time(l.get("ts"))))
        .or_else(|| time(root.get("generated_at")));

    let heads = live.and_then(|l| obj(l.get("heads")));
    let paths = obj(root.get("history"))
        .and_then(|h| obj(h.get("path_timing")))
        .and_then(|p| obj(p.get("paths")));

    Ok(FeedSnapshot {
        data_as_of,
        active: list(live.and_then(|l| l.get("active")), active_entry),
        queued: list(live.and_then(|l| l.get("queued")), queued_entry),
        heads_running: heads.and_then(|h| uint(h.get("running"))),
        heads_queued: heads.and_then(|h| uint(h.get("queued"))),
        median_run_minutes: live.and_then(|l| non_negative(l.get("median_run_minutes"))),
        typical_minutes_by_tier: live
            .and_then(|l| obj(l.get("typical_minutes_by_tier")))
            .map(minutes_by_tier)
            .unwrap_or_default(),
        degraded: live
            .and_then(|l| obj(l.get("degraded")))
            .and_then(|d| boolean(d.get("active")))
            .unwrap_or(false),
        normal_path: path_timing(paths.and_then(|p| obj(p.get("normal")))),
        priority_path: path_timing(paths.and_then(|p| obj(p.get("priority")))),
    })
}

fn active_entry(v: &Map<String, Value>) -> Option<FeedActive> {
    let (repo, number) = identity(v)?;
    Some(FeedActive {
        repo,
        number,
        sha: string(v.get("sha")),
        phase: string(v.get("phase")),
        status: string(v.get("status")),
        progress: number_f64(v.get("progress")).map(|p| p.clamp(0.0, 1.0)),
        remaining_seconds: non_negative(v.get("remaining_seconds")),
        elapsed_seconds: non_negative(v.get("elapsed_seconds")),
        started_at: time(v.get("started_at")),
        queued_at: time(v.get("queued_at")),
        wait_seconds: non_negative(v.get("wait_seconds")),
        overdue: boolean(v.get("overdue")).unwrap_or(false),
        tier: string(v.get("tier")),
        priority: boolean(v.get("priority")).unwrap_or(false),
        findings: findings(v.get("findings")),
        steps: list(v.get("steps"), step_entry),
        upcoming: v
            .get("upcoming")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| string(Some(s))).collect())
            .unwrap_or_default(),
    })
}

fn step_entry(v: &Map<String, Value>) -> Option<FeedStep> {
    Some(FeedStep {
        name: string(v.get("name"))?,
        status: string(v.get("status")),
        lanes_done: uint(v.get("lanes_done")),
        lanes_total: uint(v.get("lanes_total")),
        seconds: non_negative(v.get("seconds")),
        started_at: time(v.get("started_at")),
    })
}

fn queued_entry(v: &Map<String, Value>) -> Option<FeedQueued> {
    let (repo, number) = identity(v)?;
    Some(FeedQueued {
        repo,
        number,
        sha: string(v.get("sha")),
        position: uint(v.get("position")).filter(|p| *p > 0),
        eligible: boolean(v.get("eligible")),
        eligible_in_seconds: non_negative(v.get("eligible_in_seconds")),
        eligible_at: time(v.get("eligible_at")),
        reason: string(v.get("reason")),
        priority: boolean(v.get("priority")).unwrap_or(false),
        queued_at: time(v.get("queued_at")),
    })
}

fn identity(v: &Map<String, Value>) -> Option<(String, u64)> {
    let repo = string(v.get("repo")).filter(|r| r.contains('/'))?;
    let number = whole(v.get("number")).filter(|n| *n > 0)?;
    Some((repo, number))
}

fn findings(v: Option<&Value>) -> Option<Findings> {
    let o = obj(v)?;
    let blocking = uint(o.get("blocking"));
    let suggestion = uint(o.get("suggestion"));
    let nitpick = uint(o.get("nitpick"));
    if blocking.is_none() && suggestion.is_none() && nitpick.is_none() {
        return None;
    }
    Some(Findings {
        blocking: blocking.unwrap_or(0),
        suggestion: suggestion.unwrap_or(0),
        nitpick: nitpick.unwrap_or(0),
    })
}

fn path_timing(v: Option<&Map<String, Value>>) -> PathTiming {
    let median = |key: &str| {
        v.and_then(|p| obj(p.get(key)))
            .and_then(|m| non_negative(m.get("median_seconds")))
    };
    PathTiming {
        duration_median_seconds: median("duration"),
        slot_wait_median_seconds: median("slot_wait"),
        wait_median_seconds: median("wait"),
    }
}

fn minutes_by_tier(v: &Map<String, Value>) -> BTreeMap<String, f64> {
    v.iter()
        .filter_map(|(tier, m)| Some((tier.clone(), non_negative(Some(m))?)))
        .collect()
}

/// Parses every element of an array that is an object and passes `f`;
/// anything else (missing, null, not an array, bad elements) is skipped.
fn list<T>(v: Option<&Value>, f: impl Fn(&Map<String, Value>) -> Option<T>) -> Vec<T> {
    v.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .filter_map(&f)
                .collect()
        })
        .unwrap_or_default()
}

fn obj(v: Option<&Value>) -> Option<&Map<String, Value>> {
    v.and_then(Value::as_object)
}

/// A non-empty string.
fn string(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn boolean(v: Option<&Value>) -> Option<bool> {
    v.and_then(Value::as_bool)
}

fn number_f64(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|f| f.is_finite())
}

fn non_negative(v: Option<&Value>) -> Option<f64> {
    number_f64(v).filter(|f| *f >= 0.0)
}

/// A whole, non-negative number. Accepts `3` and `3.0`, not `3.5`.
fn whole(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64().or_else(|| {
        let f = number_f64(Some(v))?;
        // The range check makes the cast exact.
        (f >= 0.0 && f.fract() == 0.0 && f <= u32::MAX as f64).then_some(f as u64)
    })
}

/// A whole number that fits in a u32. Fractions are rounded down, so a
/// count like `2.0` or a seconds value like `60.5` still reads.
fn uint(v: Option<&Value>) -> Option<u32> {
    let v = v?;
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok();
    }
    let f = non_negative(Some(v))?;
    (f <= u32::MAX as f64).then_some(f.floor() as u32)
}

fn time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    let s = v.and_then(Value::as_str)?;
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/feed/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_a_real_feed() {
        let feed = parse_feed(&fixture("real_trimmed.json")).unwrap();
        assert_eq!(feed.data_as_of, Some(at("2026-11-21T20:06:01Z")));
        assert_eq!(feed.active.len(), 3);
        assert_eq!(feed.queued.len(), 3);
        assert_eq!(feed.heads_running, Some(5));
        assert_eq!(feed.heads_queued, Some(1));
        assert_eq!(feed.median_run_minutes, Some(23.1));
        assert_eq!(feed.typical_minutes_by_tier.get("normal"), Some(&28.6));
        assert!(!feed.degraded);
        assert_eq!(
            feed.normal_path,
            PathTiming {
                duration_median_seconds: Some(1914.0),
                slot_wait_median_seconds: Some(22.0),
                wait_median_seconds: Some(1815.0),
            }
        );
        assert_eq!(feed.priority_path.duration_median_seconds, Some(1363.0));

        let a = &feed.active[0];
        assert_eq!(a.repo, "dashpay/dashwallet-ios");
        assert_eq!(a.number, 1474);
        assert_eq!(
            a.sha.as_deref(),
            Some("84d64701abb8868b4b722f015921fb3fd2ca72de")
        );
        assert_eq!(a.phase.as_deref(), Some("phase1"));
        assert_eq!(a.status.as_deref(), Some("running"));
        assert_eq!(a.progress, Some(0.885));
        assert_eq!(a.remaining_seconds, Some(342.0));
        assert_eq!(a.elapsed_seconds, Some(2645.0));
        assert_eq!(a.started_at, Some(at("2026-11-21T19:21:56Z")));
        assert_eq!(a.queued_at, Some(at("2026-11-21T18:49:50Z")));
        assert_eq!(a.wait_seconds, Some(1926.0));
        assert!(a.overdue);
        assert_eq!(a.tier.as_deref(), Some("low"));
        assert!(!a.priority);
        assert_eq!(a.findings, None);
        let names: Vec<_> = a.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["worktree", "select", "triage", "context", "phase1"]);
        assert_eq!(a.steps[4].status.as_deref(), Some("running"));
        assert_eq!(a.steps[4].lanes_done, Some(0));
        assert_eq!(a.steps[4].lanes_total, Some(1));
        assert_eq!(
            a.upcoming,
            ["verify1", "gate", "phase2", "verify2", "publish"]
        );

        let b = &feed.active[1];
        assert_eq!(b.phase.as_deref(), Some("phase2"));
        assert_eq!(
            b.findings,
            Some(Findings {
                blocking: 1,
                suggestion: 1,
                nitpick: 0
            })
        );

        let q = &feed.queued[0];
        assert_eq!(q.repo, "dashpay/dashwallet-ios");
        assert_eq!(q.number, 1089);
        assert_eq!(q.position, Some(1));
        assert_eq!(q.eligible, Some(false));
        assert_eq!(q.eligible_in_seconds, Some(1675.0));
        assert_eq!(q.eligible_at, Some(at("2026-11-21T20:33:56Z")));
        assert_eq!(
            q.reason.as_deref(),
            Some("debounce (more pushes may follow)")
        );
        assert!(!q.priority);
        assert!(feed.queued[1].priority);
        assert_eq!(feed.queued[2].eligible_at, None);
        assert_eq!(feed.queued[2].eligible_in_seconds, Some(600.0));
    }

    #[test]
    fn minimal_document_is_empty_but_valid() {
        let feed = parse_feed(&fixture("minimal.json")).unwrap();
        assert_eq!(
            feed,
            FeedSnapshot {
                data_as_of: Some(at("2026-11-21T20:06:01Z")),
                ..FeedSnapshot::default()
            }
        );
    }

    #[test]
    fn live_only_document_takes_its_time_from_live_ts() {
        let feed = parse_feed(&fixture("live_only.json")).unwrap();
        assert_eq!(feed.data_as_of, Some(at("2026-11-21T20:06:01Z")));
        assert_eq!(feed.queued.len(), 1);
        assert_eq!(feed.queued[0].sha, None);
    }

    #[test]
    fn missing_and_null_sections_read_as_empty() {
        let feed = parse_feed(&fixture("missing_sections.json")).unwrap();
        assert!(feed.active.is_empty());
        assert!(feed.queued.is_empty());
        assert_eq!(feed.heads_running, None);
        assert_eq!(feed.median_run_minutes, None);
        assert_eq!(feed.typical_minutes_by_tier.get("normal"), Some(&28.6));
        assert_eq!(feed.normal_path, PathTiming::default());
    }

    #[test]
    fn a_malformed_entry_does_not_discard_the_rest() {
        let feed = parse_feed(&fixture("malformed_entry.json")).unwrap();
        let repos: Vec<_> = feed.active.iter().map(|a| a.repo.as_str()).collect();
        assert_eq!(repos, ["owner/good-one", "owner/good-two"]);

        let two = &feed.active[1];
        assert_eq!(two.sha, None);
        assert_eq!(two.progress, None, "a string progress reads as absent");
        assert_eq!(two.remaining_seconds, None);
        assert_eq!(two.findings, None, "no countable findings");
        let steps: Vec<_> = two.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            steps,
            ["phase1"],
            "nameless and non-object steps are skipped"
        );

        assert_eq!(feed.queued.len(), 1);
        assert_eq!(feed.queued[0].eligible_in_seconds, Some(60.5));
    }

    #[test]
    fn rejects_documents_that_are_not_a_feed() {
        for doc in [
            "[]",
            "42",
            "\"x\"",
            "null",
            "{}",
            r#"{"live": null}"#,
            "not json",
        ] {
            let err = parse_feed(doc).unwrap_err();
            assert!(matches!(err, AppError::Decode(_)), "{doc}: {err:?}");
        }
    }

    #[test]
    fn empty_live_object_is_accepted() {
        let feed = parse_feed(r#"{"live": {}}"#).unwrap();
        assert_eq!(feed, FeedSnapshot::default());
    }

    #[test]
    fn wrong_types_read_as_absent() {
        let doc = r#"{
            "data_as_of": 17,
            "generated_at": "2026-11-21T20:06:03Z",
            "live": {
                "heads": {"running": "5", "queued": -1},
                "degraded": true,
                "median_run_minutes": "slow",
                "active": [{"repo": "a/b", "number": 3.0, "progress": 1.7, "overdue": "yes",
                            "upcoming": ["publish", 7, null, ""]}],
                "queued": {"not": "a list"}
            }
        }"#;
        let feed = parse_feed(doc).unwrap();
        assert_eq!(feed.data_as_of, Some(at("2026-11-21T20:06:03Z")));
        assert_eq!(feed.heads_running, None);
        assert_eq!(feed.heads_queued, None);
        assert!(!feed.degraded, "degraded must be an object with `active`");
        assert_eq!(feed.median_run_minutes, None);
        assert_eq!(feed.active.len(), 1);
        assert_eq!(feed.active[0].number, 3);
        assert_eq!(feed.active[0].progress, Some(1.0), "clamped");
        assert!(!feed.active[0].overdue);
        assert_eq!(feed.active[0].upcoming, ["publish"]);
        assert!(feed.queued.is_empty());
    }
}
