//! Rendering calendar times in the user's zone.
//!
//! Both providers hand back UTC (Graph as `2026-08-04T16:00:00.0000000Z`,
//! Google as RFC 3339), and a model reading UTC reports UTC — so a noon
//! meeting is announced at four in the afternoon. It is not obviously wrong
//! on the page, which is what makes it worth fixing here rather than hoping
//! the model converts.
//!
//! The zone comes from `MECHA_TZ`, set on the server in the `[[mcp]]` block's
//! `env`, falling back to `TZ` and then to leaving the stamp alone.
//!
//! **It also resolves a query window, for the same reason and the opposite
//! direction.** A model has no clock, so the window it asks for is only as
//! good as the date it was told. On 2026-09-14 a run whose prompt carried a
//! stale date asked `calendar_list_events` for
//! `time_min: 2026-09-13T00:00:00-04:00` and the calendar answered that
//! window faithfully — the tool confirmed the wrong premise instead of
//! contradicting it, and the owner had to correct the date twice. #238 made
//! the harness's own clock unable to go stale; [`resolve_bound`] is the other
//! half, letting the model name `today` and have the *server* decide which
//! day that is, where the clock is real. [`as_of`] is the third: every
//! time-scoped answer says what instant and zone it was resolved against, so
//! a wrong premise is contradicted by the result rather than confirmed by it.

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;

/// The zone to render in, if one is configured.
pub fn configured_zone() -> Option<Tz> {
    for var in ["MECHA_TZ", "TZ"] {
        if let Ok(name) = std::env::var(var) {
            if let Ok(tz) = name.parse::<Tz>() {
                return Some(tz);
            }
        }
    }
    None
}

/// Render one timestamp in `tz`. Returns the input unchanged when it does not
/// parse or carries no zone — an all-day event's bare `2026-08-10` is a date,
/// not an instant, and converting it would move the day.
pub fn in_zone(raw: &str, tz: Option<Tz>) -> String {
    let Some(tz) = tz else { return raw.to_string() };
    let trimmed = raw.trim();
    // Graph pads to seven fractional digits, which is valid RFC 3339.
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) else {
        return raw.to_string();
    };
    parsed
        .with_timezone(&tz)
        .format("%Y-%m-%d %H:%M %Z")
        .to_string()
}

/// Which end of a window a bare day name means.
///
/// `today` is a day, not an instant, so it resolves to local midnight as a
/// `Start` and to the last moment before the next local midnight as an `End`.
/// That is what makes `time_min = today, time_max = today` mean all of today,
/// which is what a model writes when it means that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Start,
    End,
}

/// What [`resolve_bound`] decided, with the three cases kept apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Not a term this resolves — an RFC 3339 stamp, or anything else. The
    /// caller passes it through untouched, so every window that worked
    /// before still works.
    Passthrough,
    /// Resolved against the configured zone.
    At(String),
    /// A relative term, and no zone configured to resolve it in.
    ///
    /// **Refused rather than resolved against the machine's clock**, because
    /// this server runs where `TZ` is UTC and the whole incident was a UTC
    /// day standing in for a local one. Answering "today" with the wrong day
    /// is the failure; saying which variable is missing is recoverable.
    NeedsZone,
}

/// Resolve a relative window term in the configured zone.
///
/// The set is deliberately small and unambiguous: `now`, `today`,
/// `tomorrow`, `yesterday`, and signed day offsets (`+3d`, `-1d`).
///
/// **`this week` and `this month` are deliberately absent.** Where a week
/// starts is a convention this server cannot read off anything — ISO says
/// Monday, US calendars say Sunday — and resolving it would put a
/// confidently wrong window in exactly the place this function exists to
/// stop one. They fall through as [`Resolved::Passthrough`], so a model that
/// writes one gets the provider's own parse error rather than a silently
/// shifted week. If a week term is ever wanted, it needs a stated
/// convention, not a guess.
pub fn resolve_bound(raw: &str, tz: Option<Tz>, now: DateTime<Utc>, bound: Bound) -> Resolved {
    let term = raw.trim().to_ascii_lowercase();
    let offset_days = match term.as_str() {
        "now" => {
            return match tz {
                // `now` is an instant, not a day, so it needs no zone to be
                // correct — but it is still refused without one, so the whole
                // relative vocabulary behaves alike rather than one term
                // working and its neighbours not. Rendered in the zone for
                // the same reason the day bounds are: the window is echoed
                // back for a reader to check, and one term answering in UTC
                // while its neighbours answer local is the inconsistency that
                // makes a stamp hard to read at a glance.
                Some(tz) => Resolved::At(now.with_timezone(&tz).to_rfc3339()),
                None => Resolved::NeedsZone,
            };
        }
        "today" => 0,
        "tomorrow" => 1,
        "yesterday" => -1,
        other => match parse_day_offset(other) {
            Some(n) => n,
            None => return Resolved::Passthrough,
        },
    };
    let Some(tz) = tz else {
        return Resolved::NeedsZone;
    };
    let day = now.with_timezone(&tz).date_naive() + Duration::days(offset_days);
    let (h, m, sec) = match bound {
        Bound::Start => (0, 0, 0),
        // 23:59:59, not the next local midnight: Google treats `timeMax` as
        // exclusive and Graph does not, so the inclusive end of the day is
        // the spelling that means the same thing to both. It is also what a
        // model writes by hand — the 2026-09-14 call said `T23:59:59-04:00`.
        Bound::End => (23, 59, 59),
    };
    // Rendered in the configured zone, not UTC. Providers accept any
    // offset, and the window is echoed back to the model in `as_of` and in
    // the empty-result line — `2026-09-16T00:00:00-04:00` is checkable
    // against a calendar at a glance and `…T04:00:00Z` is not, which matters
    // for a value whose whole job is contradicting a wrong premise.
    Resolved::At(local_instant(tz, day, h, m, sec, bound).to_rfc3339())
}

/// One wall-clock time on a local calendar day, as a real instant.
///
/// **A local day is not always 24 hours long and its edges are not always
/// there.** A fall-back day has two 01:30s and a spring-forward day has no
/// 02:30, so `from_local_datetime` answers with an ambiguity or with nothing.
/// Widened rather than narrowed at both ends — earliest for a start, latest
/// for an end — so an ambiguous hour is inside the window rather than half
/// outside it, and a window never silently loses an event that happened.
///
/// When the wall time does not exist at all, the clock is walked *into* the
/// day until it does — forward from a start, backward from an end (bounded; a
/// transition is at most a couple of hours). Walking one direction for both
/// was the shape defect a review caught: at `hour + step` an end bound of
/// 23:59:59 asks for hour 24, `and_hms_opt` returns `None`, the loop breaks
/// immediately and the end falls to the noon fallback — *earlier* than the
/// start, so a widening fallback produced an inverted window that silently
/// answers nothing. Unreachable with any zone in the database, and fixed
/// anyway, because "widen, never narrow" is the property being relied on.
fn local_instant(
    tz: Tz,
    day: chrono::NaiveDate,
    hour: u32,
    min: u32,
    sec: u32,
    bound: Bound,
) -> DateTime<Tz> {
    for step in 0..4 {
        // Into the day from whichever edge this is, so neither bound can
        // cross the other.
        let walked = match bound {
            Bound::Start => hour.checked_add(step),
            Bound::End => hour.checked_sub(step),
        };
        let Some(naive) = walked.and_then(|h| day.and_hms_opt(h, min, sec)) else {
            break;
        };
        let mapped = tz.from_local_datetime(&naive);
        let picked = match bound {
            Bound::Start => mapped.earliest(),
            Bound::End => mapped.latest(),
        };
        if let Some(dt) = picked {
            return dt;
        }
    }
    // Every candidate wall time on this day was unrepresentable, which no
    // real zone does. Fall back to the day's noon offset rather than to a
    // guess about the zone: noon exists everywhere.
    let noon = day
        .and_hms_opt(12, 0, 0)
        .expect("noon is a valid wall time on every date");
    tz.from_local_datetime(&noon)
        .earliest()
        .unwrap_or_else(|| Utc.from_utc_datetime(&noon).with_timezone(&tz))
}

/// `+3d` / `-1d` / `3d`, and nothing cleverer.
///
/// **Bounded, because the two chrono calls downstream panic rather than
/// return.** `Duration::days` is `expect(try_days(n), …)` and
/// `NaiveDate + TimeDelta` is `checked_add_signed(…).expect(…)`, so
/// `+999999999d` aborted the *process*: `mcp::serve` awaits `provider.call`
/// inline with no `catch_unwind` anywhere in this crate, so the unwind leaves
/// `main` and the run loses mail and calendar for the rest of the session —
/// from a model-supplied string, which under this repo's threat model is
/// reachable from untrusted content steering a calendar query. Reproduced
/// before fixing: `NaiveDate + TimeDelta overflowed`.
///
/// Anything sillier than a couple of centuries is a typo, and falls through
/// as [`Resolved::Passthrough`] to the provider's own parse error — the same
/// place `this week` lands, which keeps the three-outcome shape intact rather
/// than inventing a fourth for "accepted, then crashed".
fn parse_day_offset(term: &str) -> Option<i64> {
    let rest = term.strip_suffix('d')?;
    if rest.is_empty() {
        return None;
    }
    let n = rest.parse::<i64>().ok()?;
    // Range containment, not `n.abs() <= MAX`: `i64::MIN.abs()` panics, so
    // the obvious spelling of this guard reintroduces the same class of bug
    // one level down — caught by the test below feeding it
    // `-9223372036854775808d`.
    (-MAX_OFFSET_DAYS..=MAX_OFFSET_DAYS)
        .contains(&n)
        .then_some(n)
}

/// Roughly 274 years either way — past any window a calendar query means and
/// far inside `NaiveDate`'s range, so the arithmetic cannot reach its panic.
const MAX_OFFSET_DAYS: i64 = 100_000;

/// What a time-scoped answer was resolved against, for its own record.
///
/// Attached to every window result so a wrong date premise is contradicted by
/// the data. The instant is the server's, which is the point: the model
/// cannot check its own clock, and this is the one in the answer.
pub fn as_of(now: DateTime<Utc>, tz: Option<Tz>) -> String {
    match tz {
        Some(tz) => format!(
            "as of {} (server clock, {tz})",
            now.with_timezone(&tz).format("%A %Y-%m-%d %H:%M %Z")
        ),
        None => format!(
            "as of {} (server clock, no MECHA_TZ configured)",
            now.format("%A %Y-%m-%d %H:%M UTC")
        ),
    }
}

/// A weekday/date pair computed from the source instant, in the configured
/// mailbox zone when present. Unknown timestamps stay unknown, never guessed.
pub fn calendar_date(raw: &str, tz: Option<Tz>) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(raw.trim())
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(raw.trim()))
        .ok()?;
    Some(match tz {
        Some(tz) => parsed
            .with_timezone(&tz)
            .format("%A %Y-%m-%d %:z")
            .to_string(),
        None => parsed.format("%A %Y-%m-%d %:z").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_weekdays_come_from_the_source_instant_in_the_display_zone() {
        assert_eq!(
            calendar_date("2026-10-11T16:12:00Z", None).as_deref(),
            Some("Sunday 2026-10-11 +00:00")
        );
        assert_eq!(
            calendar_date("Mon, 12 Oct 2026 00:30:00 +0000", eastern()).as_deref(),
            Some("Sunday 2026-10-11 -04:00")
        );
        assert_eq!(calendar_date("unknown", eastern()), None);
    }

    fn eastern() -> Option<Tz> {
        Some("America/New_York".parse().unwrap())
    }

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// **The incident, as a test.** 02:37Z on the 14th is 22:37 on the 13th
    /// in the owner's zone, so a run told "today" that evening means the
    /// 13th; the same word at 13:21Z the next day means the 14th. The model
    /// writes the same token either way and the server decides, which is the
    /// whole point — it is the one party in the exchange holding a clock.
    #[test]
    fn today_is_the_local_day_not_the_utc_one() {
        let before_midnight = at("2026-09-14T02:37:12Z");
        let next_morning = at("2026-09-14T13:21:33Z");

        let Resolved::At(a) = resolve_bound("today", eastern(), before_midnight, Bound::Start)
        else {
            panic!("today must resolve with a zone configured")
        };
        let Resolved::At(b) = resolve_bound("today", eastern(), next_morning, Bound::Start) else {
            panic!("today must resolve with a zone configured")
        };
        assert!(a.starts_with("2026-09-13T00:00:00"), "{a}");
        assert!(b.starts_with("2026-09-14T00:00:00"), "{b}");
        assert_ne!(a, b, "the same word an hour apart is a different day");

        // Both ends of one word, which is how a model asks for a whole day.
        let Resolved::At(end) = resolve_bound("today", eastern(), next_morning, Bound::End) else {
            panic!("an end bound resolves too")
        };
        assert!(end.starts_with("2026-09-14T23:59:59"), "{end}");
    }

    #[test]
    fn the_small_vocabulary_resolves_and_everything_else_passes_through() {
        let now = at("2026-09-16T13:21:33Z");
        for (term, expect) in [
            ("today", "2026-09-16"),
            ("tomorrow", "2026-09-17"),
            ("yesterday", "2026-09-15"),
            ("+3d", "2026-09-19"),
            ("-1d", "2026-09-15"),
            ("2d", "2026-09-18"),
            ("TOMORROW", "2026-09-17"),
        ] {
            let Resolved::At(got) = resolve_bound(term, eastern(), now, Bound::Start) else {
                panic!("`{term}` should resolve")
            };
            assert!(got.starts_with(expect), "{term} → {got}, wanted {expect}");
        }
        let Resolved::At(n) = resolve_bound("now", eastern(), now, Bound::Start) else {
            panic!("`now` should resolve")
        };
        // The same instant, rendered in the configured zone like every other
        // term — see `the_whole_vocabulary_answers_in_the_configured_zone`.
        assert_eq!(at(&n), now);
        assert!(n.ends_with("-04:00"), "{n}");

        // An RFC 3339 window is what every caller sent before this existed
        // and must still reach the provider untouched.
        for passthrough in [
            "2026-09-13T00:00:00-04:00",
            "this week",
            "next month",
            "",
            "d",
            "soon",
        ] {
            assert_eq!(
                resolve_bound(passthrough, eastern(), now, Bound::Start),
                Resolved::Passthrough,
                "`{passthrough}` must not be resolved"
            );
        }
    }

    /// Refused, not answered against the machine's clock. This server runs
    /// where `TZ` is UTC, and a UTC day standing in for a local one is the
    /// failure the whole module is about.
    #[test]
    fn a_relative_term_with_no_zone_is_refused_rather_than_guessed() {
        let now = at("2026-09-14T02:37:12Z");
        for term in ["today", "tomorrow", "now", "+1d"] {
            assert_eq!(
                resolve_bound(term, None, now, Bound::Start),
                Resolved::NeedsZone,
                "`{term}` with no zone"
            );
        }
        // And a real stamp still works with no zone, because it needs none.
        assert_eq!(
            resolve_bound("2026-09-13T00:00:00Z", None, now, Bound::Start),
            Resolved::Passthrough
        );
    }

    /// A local day is not always 24 hours and its edges are not always there.
    /// Both directions, because they fail differently: spring-forward can
    /// delete a wall time, fall-back can duplicate one.
    #[test]
    fn a_day_boundary_survives_both_dst_transitions() {
        // Spring forward: 2026-03-08, clocks jump 02:00 → 03:00 Eastern.
        let spring = at("2026-03-08T12:00:00Z");
        let Resolved::At(start) = resolve_bound("today", eastern(), spring, Bound::Start) else {
            panic!()
        };
        let Resolved::At(end) = resolve_bound("today", eastern(), spring, Bound::End) else {
            panic!()
        };
        assert!(start.starts_with("2026-03-08T00:00:00-05:00"), "{start}");
        assert!(end.starts_with("2026-03-08T23:59:59-04:00"), "{end}");
        // The window really does span the short day, offsets and all.
        assert!(at(&end) > at(&start));
        assert_eq!((at(&end) - at(&start)).num_hours(), 22);

        // Fall back: 2026-11-01, 02:00 → 01:00, so the day is 25 hours.
        let fall = at("2026-11-01T12:00:00Z");
        let Resolved::At(start) = resolve_bound("today", eastern(), fall, Bound::Start) else {
            panic!()
        };
        let Resolved::At(end) = resolve_bound("today", eastern(), fall, Bound::End) else {
            panic!()
        };
        assert_eq!((at(&end) - at(&start)).num_hours(), 24);
    }

    /// **A term the parser accepted and then crashed on.** `+999999999d`
    /// panicked with `NaiveDate + TimeDelta overflowed`, and `mcp::serve`
    /// awaits `provider.call` inline with no `catch_unwind` in the crate, so
    /// the unwind left `main` and the session lost mail *and* calendar. The
    /// input is a model-supplied string. Panics today without the clamp.
    #[test]
    fn an_absurd_day_offset_falls_through_instead_of_killing_the_server() {
        let now = at("2026-09-17T11:15:00Z");
        for absurd in [
            "+999999999d",
            "-999999999d",
            "9223372036854775807d",
            "-9223372036854775808d",
        ] {
            for bound in [Bound::Start, Bound::End] {
                assert_eq!(
                    resolve_bound(absurd, eastern(), now, bound),
                    Resolved::Passthrough,
                    "`{absurd}` must fall through, not panic"
                );
            }
        }
        // And the bound itself is where the line sits, not an arbitrary cliff.
        let Resolved::At(ok) = resolve_bound("100000d", eastern(), now, Bound::Start) else {
            panic!("the largest accepted offset still resolves")
        };
        assert!(ok.starts_with("2300-"), "{ok}");
        assert_eq!(
            resolve_bound("100001d", eastern(), now, Bound::Start),
            Resolved::Passthrough
        );
    }

    /// Every term answers in one format, so a window is readable at a glance.
    /// `now` rendered in UTC while its neighbours rendered local.
    #[test]
    fn the_whole_vocabulary_answers_in_the_configured_zone() {
        let now = at("2026-09-17T11:15:00Z");
        for term in ["now", "today", "tomorrow", "+2d"] {
            let Resolved::At(got) = resolve_bound(term, eastern(), now, Bound::Start) else {
                panic!("`{term}` should resolve")
            };
            assert!(
                got.ends_with("-04:00"),
                "`{term}` answered {got}, not in the configured zone"
            );
        }
    }

    /// The fallback must widen at both ends. Walking one direction for both
    /// sent an end bound to the noon fallback — earlier than its own start,
    /// an inverted window that answers nothing rather than everything.
    #[test]
    fn a_bound_never_crosses_its_partner() {
        let now = at("2026-09-17T11:15:00Z");
        for tz in [eastern(), Some("Australia/Lord_Howe".parse().unwrap())] {
            for term in ["today", "tomorrow", "yesterday"] {
                let (Resolved::At(start), Resolved::At(end)) = (
                    resolve_bound(term, tz, now, Bound::Start),
                    resolve_bound(term, tz, now, Bound::End),
                ) else {
                    panic!("`{term}` should resolve")
                };
                assert!(
                    at(&end) > at(&start),
                    "{term} in {tz:?}: end {end} is not after start {start}"
                );
            }
        }
    }

    /// The stamp names the instant *and* the zone it was read in, because
    /// either alone is the ambiguity this module exists to remove.
    #[test]
    fn the_as_of_stamp_names_the_instant_and_the_zone() {
        let stamp = as_of(at("2026-09-16T13:21:33Z"), eastern());
        assert!(stamp.contains("Wednesday 2026-09-16"), "{stamp}");
        assert!(stamp.contains("09:21"), "{stamp}");
        assert!(stamp.contains("America/New_York"), "{stamp}");

        // With no zone it says so rather than implying the owner's.
        let bare = as_of(at("2026-09-16T13:21:33Z"), None);
        assert!(bare.contains("no MECHA_TZ"), "{bare}");
        assert!(bare.contains("13:21 UTC"), "{bare}");
    }

    /// The bug this exists for: a noon Eastern meeting was announced as 4pm.
    #[test]
    fn a_utc_stamp_renders_in_the_users_zone() {
        assert_eq!(
            in_zone("2026-08-04T16:00:00Z", eastern()),
            "2026-08-04 12:00 EDT"
        );
        // Graph's seven-digit fraction must parse too.
        assert_eq!(
            in_zone("2026-08-04T16:00:00.0000000Z", eastern()),
            "2026-08-04 12:00 EDT"
        );
    }

    /// An IANA zone, not an offset, so this is right on both sides of the
    /// DST boundary.
    #[test]
    fn winter_and_summer_differ() {
        assert!(in_zone("2026-01-15T17:00:00Z", eastern()).contains("12:00 EST"));
        assert!(in_zone("2026-07-15T16:00:00Z", eastern()).contains("12:00 EDT"));
    }

    #[test]
    fn all_day_dates_and_unparseable_input_pass_through() {
        // A date is not an instant; converting it could move the day.
        assert_eq!(in_zone("2026-08-10", eastern()), "2026-08-10");
        assert_eq!(in_zone("", eastern()), "");
        // No zone configured means no change.
        assert_eq!(
            in_zone("2026-08-04T16:00:00Z", None),
            "2026-08-04T16:00:00Z"
        );
    }
}
