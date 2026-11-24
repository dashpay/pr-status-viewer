//! Time estimates for reviews still in the bot's queue.
//!
//! The feed gives relative seconds as of its `data_as_of`; everything here
//! anchors on that instant, never the local clock, so an estimate does not
//! drift between fetches of the same feed.
//!
//! `finishes_at` for a queued PR is our own estimate, not the bot's: when it
//! can start, plus the median wait for a review slot, plus the median review
//! duration, for the PR's path (priority or normal).

use chrono::{DateTime, TimeDelta, Utc};

use crate::feed::{FeedQueued, FeedSnapshot};

/// Longest offset we accept from the feed, so an absurd value cannot
/// overflow a timestamp. One year.
const MAX_OFFSET_SECONDS: f64 = 366.0 * 24.0 * 3600.0;

/// `t` plus a (possibly fractional) number of seconds. `None` when the
/// offset is not a sane, finite number.
pub fn add_seconds(t: DateTime<Utc>, seconds: f64) -> Option<DateTime<Utc>> {
    if !seconds.is_finite() || seconds.abs() > MAX_OFFSET_SECONDS {
        return None;
    }
    let delta = TimeDelta::try_milliseconds((seconds * 1000.0).round() as i64)?;
    t.checked_add_signed(delta)
}

/// When a queued review can start: the feed's `eligible_at`, else
/// `data_as_of + eligible_in_seconds`, else unknown.
pub fn queued_starts_at(
    q: &FeedQueued,
    data_as_of: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    q.eligible_at
        .or_else(|| add_seconds(data_as_of?, q.eligible_in_seconds?))
}

/// Typical seconds from becoming eligible to the review being done.
///
/// Prefers the path's median slot wait plus median duration from
/// `history.path_timing` (a missing slot wait counts as zero), then
/// `live.median_run_minutes`, then the normal tier's typical minutes.
pub fn typical_review_seconds(feed: &FeedSnapshot, priority: bool) -> Option<f64> {
    let path = if priority {
        &feed.priority_path
    } else {
        &feed.normal_path
    };
    let positive = |v: Option<f64>| v.filter(|s| s.is_finite() && *s > 0.0);
    if let Some(duration) = positive(path.duration_median_seconds) {
        let slot_wait = path
            .slot_wait_median_seconds
            .filter(|s| s.is_finite() && *s >= 0.0)
            .unwrap_or(0.0);
        return Some(slot_wait + duration);
    }
    positive(feed.median_run_minutes)
        .or_else(|| positive(feed.typical_minutes_by_tier.get("normal").copied()))
        .map(|minutes| minutes * 60.0)
}

/// Our estimate of when a queued review finishes.
///
/// It starts at `starts_at`, or at `data_as_of` if that start is already
/// past or the PR is already eligible. `None` when we do not know when it
/// can start, or have no typical duration to add.
pub fn queued_finishes_at(
    q: &FeedQueued,
    starts_at: Option<DateTime<Utc>>,
    feed: &FeedSnapshot,
) -> Option<DateTime<Utc>> {
    let as_of = feed.data_as_of;
    let base = match (starts_at, as_of) {
        (Some(s), Some(a)) => s.max(a),
        (Some(s), None) => s,
        // Unknown start: only usable when it may start right away.
        (None, Some(a)) if q.eligible != Some(false) => a,
        (None, _) => return None,
    };
    add_seconds(base, typical_review_seconds(feed, q.priority)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::PathTiming;
    use pretty_assertions::assert_eq;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    const AS_OF: &str = "2026-11-21T20:00:00Z";

    fn feed() -> FeedSnapshot {
        FeedSnapshot {
            data_as_of: Some(at(AS_OF)),
            normal_path: PathTiming {
                duration_median_seconds: Some(1800.0),
                slot_wait_median_seconds: Some(60.0),
                wait_median_seconds: Some(1815.0),
            },
            priority_path: PathTiming {
                duration_median_seconds: Some(1200.0),
                slot_wait_median_seconds: Some(0.0),
                wait_median_seconds: None,
            },
            median_run_minutes: Some(23.0),
            typical_minutes_by_tier: [("normal".to_string(), 28.0)].into_iter().collect(),
            ..FeedSnapshot::default()
        }
    }

    fn queued() -> FeedQueued {
        FeedQueued {
            repo: "o/r".into(),
            number: 1,
            ..FeedQueued::default()
        }
    }

    #[test]
    fn starts_at_prefers_eligible_at() {
        let q = FeedQueued {
            eligible_at: Some(at("2026-11-21T20:30:00Z")),
            eligible_in_seconds: Some(10.0),
            ..queued()
        };
        assert_eq!(
            queued_starts_at(&q, Some(at(AS_OF))),
            Some(at("2026-11-21T20:30:00Z"))
        );
    }

    #[test]
    fn starts_at_falls_back_to_relative_seconds_from_data_as_of() {
        let q = FeedQueued {
            eligible_in_seconds: Some(90.5),
            ..queued()
        };
        assert_eq!(
            queued_starts_at(&q, Some(at(AS_OF))),
            Some(at("2026-11-21T20:01:30.500Z"))
        );
        assert_eq!(queued_starts_at(&q, None), None);
        assert_eq!(queued_starts_at(&queued(), Some(at(AS_OF))), None);
    }

    #[test]
    fn typical_seconds_uses_the_path_then_the_fallbacks() {
        let f = feed();
        assert_eq!(typical_review_seconds(&f, false), Some(1860.0));
        assert_eq!(typical_review_seconds(&f, true), Some(1200.0));

        let mut no_slot_wait = feed();
        no_slot_wait.normal_path.slot_wait_median_seconds = None;
        assert_eq!(typical_review_seconds(&no_slot_wait, false), Some(1800.0));

        let mut no_paths = feed();
        no_paths.normal_path = PathTiming::default();
        assert_eq!(typical_review_seconds(&no_paths, false), Some(23.0 * 60.0));

        no_paths.median_run_minutes = None;
        assert_eq!(typical_review_seconds(&no_paths, false), Some(28.0 * 60.0));

        no_paths.typical_minutes_by_tier.clear();
        assert_eq!(typical_review_seconds(&no_paths, false), None);
        assert_eq!(typical_review_seconds(&FeedSnapshot::default(), true), None);
    }

    #[test]
    fn zero_duration_is_not_a_duration() {
        let mut f = feed();
        f.normal_path.duration_median_seconds = Some(0.0);
        f.median_run_minutes = Some(0.0);
        assert_eq!(typical_review_seconds(&f, false), Some(28.0 * 60.0));
    }

    #[test]
    fn finish_is_start_plus_typical_review() {
        let f = feed();
        let start = at("2026-11-21T20:30:00Z");
        assert_eq!(
            queued_finishes_at(&queued(), Some(start), &f),
            Some(at("2026-11-21T21:01:00Z"))
        );
        let priority = FeedQueued {
            priority: true,
            ..queued()
        };
        assert_eq!(
            queued_finishes_at(&priority, Some(start), &f),
            Some(at("2026-11-21T20:50:00Z"))
        );
    }

    #[test]
    fn a_start_in_the_past_counts_from_data_as_of() {
        let f = feed();
        assert_eq!(
            queued_finishes_at(&queued(), Some(at("2026-11-21T19:00:00Z")), &f),
            Some(at("2026-11-21T20:31:00Z"))
        );
    }

    #[test]
    fn eligible_without_start_counts_from_data_as_of() {
        let f = feed();
        let eligible = FeedQueued {
            eligible: Some(true),
            ..queued()
        };
        assert_eq!(
            queued_finishes_at(&eligible, None, &f),
            Some(at("2026-11-21T20:31:00Z"))
        );
        let waiting = FeedQueued {
            eligible: Some(false),
            ..queued()
        };
        assert_eq!(queued_finishes_at(&waiting, None, &f), None);
    }

    #[test]
    fn no_timing_data_means_no_estimate() {
        let f = FeedSnapshot {
            data_as_of: Some(at(AS_OF)),
            ..FeedSnapshot::default()
        };
        assert_eq!(queued_finishes_at(&queued(), Some(at(AS_OF)), &f), None);
    }

    #[test]
    fn add_seconds_rejects_absurd_offsets() {
        let t = at(AS_OF);
        assert_eq!(add_seconds(t, f64::NAN), None);
        assert_eq!(add_seconds(t, 1e18), None);
        assert_eq!(add_seconds(t, -60.0), Some(at("2026-11-21T19:59:00Z")));
    }
}
