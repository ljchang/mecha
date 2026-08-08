//! Busy intervals, and the arithmetic on them. Pure — the providers hand in
//! raw `(start, end)` stamp pairs and this module parses, sorts and
//! coalesces, so the merge is testable without a credential anywhere.
//!
//! Intervals only, never events: a free/busy answer carries no titles, no
//! attendees, no locations. That is the point of asking the free/busy
//! endpoints instead of reading the calendar — the availability engine (and
//! anything downstream of it, like a published booking page) learns *when*
//! the user is busy and nothing else.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// One busy interval, UTC. Serialises as RFC 3339 with seconds, which is the
/// contract with the slot pipeline downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Interval {
    #[serde(serialize_with = "rfc3339")]
    pub start: DateTime<Utc>,
    #[serde(serialize_with = "rfc3339")]
    pub end: DateTime<Utc>,
}

fn rfc3339<S: serde::Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Parse a provider stamp into UTC. Graph pads to seven fractional digits
/// and sometimes omits the zone (stating it beside the stamp instead), so a
/// zoneless stamp is taken as UTC — both call sites only ever request UTC.
pub fn parse_stamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(t.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|t| t.and_utc())
}

/// Sort and coalesce: overlapping or touching intervals become one. Zero- or
/// negative-length input intervals are dropped rather than merged into
/// nonsense.
pub fn merge(intervals: Vec<Interval>) -> Vec<Interval> {
    let mut intervals: Vec<Interval> = intervals.into_iter().filter(|i| i.start < i.end).collect();
    intervals.sort_by_key(|i| (i.start, i.end));
    let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
    for next in intervals {
        match merged.last_mut() {
            Some(last) if next.start <= last.end => last.end = last.end.max(next.end),
            _ => merged.push(next),
        }
    }
    merged
}

/// Split `[start, end)` into windows of at most `max_days`, for APIs with a
/// bounded query span (Graph's `getSchedule` refuses more than 62 days).
/// Windows abut exactly; an empty or inverted span yields nothing.
pub fn chunk_windows(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    max_days: i64,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let mut windows = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let next = (cursor + Duration::days(max_days)).min(end);
        windows.push((cursor, next));
        cursor = next;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        parse_stamp(s).unwrap()
    }

    fn iv(start: &str, end: &str) -> Interval {
        Interval {
            start: t(start),
            end: t(end),
        }
    }

    #[test]
    fn provider_stamps_parse_including_graphs_zoneless_shape() {
        assert!(parse_stamp("2026-08-10T14:00:00Z").is_some());
        assert!(parse_stamp("2026-08-10T14:00:00.0000000Z").is_some());
        // Graph getSchedule: zone stated beside the stamp, not inside it.
        assert!(parse_stamp("2026-08-10T14:00:00.0000000").is_some());
        assert_eq!(
            parse_stamp("2026-08-10T10:00:00-04:00").unwrap(),
            t("2026-08-10T14:00:00Z"),
            "an offset stamp lands on the same instant in UTC"
        );
        assert!(parse_stamp("2026-08-10").is_none());
        assert!(parse_stamp("").is_none());
    }

    #[test]
    fn merge_coalesces_overlap_and_touching_and_sorts() {
        let merged = merge(vec![
            iv("2026-08-10T15:00:00Z", "2026-08-10T16:00:00Z"),
            iv("2026-08-10T09:00:00Z", "2026-08-10T10:00:00Z"),
            // Overlaps the 9–10.
            iv("2026-08-10T09:30:00Z", "2026-08-10T10:30:00Z"),
            // Touches the 15–16: back-to-back busy is one busy stretch.
            iv("2026-08-10T16:00:00Z", "2026-08-10T17:00:00Z"),
        ]);
        assert_eq!(
            merged,
            vec![
                iv("2026-08-10T09:00:00Z", "2026-08-10T10:30:00Z"),
                iv("2026-08-10T15:00:00Z", "2026-08-10T17:00:00Z"),
            ]
        );
    }

    #[test]
    fn merge_keeps_a_contained_interval_inside_its_container() {
        let merged = merge(vec![
            iv("2026-08-10T09:00:00Z", "2026-08-10T17:00:00Z"),
            iv("2026-08-10T10:00:00Z", "2026-08-10T11:00:00Z"),
        ]);
        assert_eq!(
            merged,
            vec![iv("2026-08-10T09:00:00Z", "2026-08-10T17:00:00Z")]
        );
    }

    #[test]
    fn degenerate_intervals_are_dropped() {
        let merged = merge(vec![
            iv("2026-08-10T10:00:00Z", "2026-08-10T10:00:00Z"),
            iv("2026-08-10T12:00:00Z", "2026-08-10T11:00:00Z"),
        ]);
        assert!(merged.is_empty());
    }

    #[test]
    fn intervals_serialise_as_rfc3339_utc() {
        let json =
            serde_json::to_value(iv("2026-08-10T09:00:00Z", "2026-08-10T10:30:00Z")).unwrap();
        assert_eq!(json["start"], "2026-08-10T09:00:00Z");
        assert_eq!(json["end"], "2026-08-10T10:30:00Z");
    }

    #[test]
    fn windows_chunk_at_the_cap_and_abut_exactly() {
        let windows = chunk_windows(t("2026-01-01T00:00:00Z"), t("2026-06-01T00:00:00Z"), 62);
        assert!(windows.len() > 1, "151 days must not fit one 62-day window");
        assert_eq!(windows.first().unwrap().0, t("2026-01-01T00:00:00Z"));
        assert_eq!(windows.last().unwrap().1, t("2026-06-01T00:00:00Z"));
        for pair in windows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "windows must abut with no gap");
        }
        for (a, b) in &windows {
            assert!(*b - *a <= Duration::days(62));
        }
    }

    #[test]
    fn a_short_or_empty_span_is_one_or_zero_windows() {
        let one = chunk_windows(t("2026-08-01T00:00:00Z"), t("2026-08-15T00:00:00Z"), 62);
        assert_eq!(one.len(), 1);
        assert!(chunk_windows(t("2026-08-15T00:00:00Z"), t("2026-08-01T00:00:00Z"), 62).is_empty());
    }
}
