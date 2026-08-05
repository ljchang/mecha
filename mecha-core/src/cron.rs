//! Five-field cron expressions, resolved in a named timezone.
//!
//! Hand-rolled rather than pulled in, for two reasons. The available crates
//! speak Quartz's six-or-seven-field dialect where the *first* field is
//! seconds, so `0 7 * * *` — what a person types, and what every crontab on
//! this machine means by "seven in the morning" — parses as something else
//! entirely rather than failing. A scheduler that silently fires at the wrong
//! time is the worst shape of bug this project keeps finding. And the whole
//! engine is two functions over a parsed bitfield, which is less code than the
//! wrapper that would have made a crate's dialect safe.
//!
//! Two things are load-bearing beyond the parsing:
//!
//! **[`Schedule::prev_at_or_before`] is the primitive, not `next_after`.**
//! "Is this due?" is answered by asking for the most recent slot at or before
//! now and comparing it to the last one that fired — which means a scheduler
//! that was asleep for a week wakes up owing exactly *one* run, not a week of
//! them, and a tick that arrives late has lost nothing. Iterating forward from
//! the last fire would have to enumerate every missed slot to find out how many
//! it was going to throw away.
//!
//! **Wall-clock time is not monotonic, and both discontinuities are handled
//! deliberately.** In the spring-forward gap a daily 02:30 job has no 02:30 to
//! run at, so it fires at the first instant that exists after the gap — a job
//! that silently skips a day twice a year is a job you cannot trust. In the
//! autumn fall-back the local time happens twice, and the *earlier* instant
//! wins, so the job runs once rather than twice. This is why the timezone is an
//! IANA name throughout and never an offset.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// How far the search will look before giving up. A schedule like
/// `0 0 30 2 *` — the 30th of February — matches nothing ever, and the search
/// has to terminate on something other than the heat death of the universe.
/// Four years covers every leap-year interaction a cron expression can express.
const HORIZON_DAYS: i64 = 366 * 4;

/// A parsed cron expression: minute, hour, day-of-month, month, day-of-week.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Schedule {
    /// The expression as written, so `mecha trigger list` shows what the user
    /// typed rather than a normalised rendering of it.
    source: String,
    minutes: u64,
    hours: u64,
    /// Bit 1..=31.
    days: u64,
    /// Bit 1..=12.
    months: u64,
    /// Bit 0..=6, Sunday is 0.
    weekdays: u64,
    /// Vixie cron's rule: when *both* day-of-month and day-of-week are
    /// restricted, a day matches if *either* does. Recording which fields were
    /// literally `*` is the only way to reproduce it, because a restriction
    /// that happens to name every value is not the same as `*`.
    dom_restricted: bool,
    dow_restricted: bool,
}

impl std::fmt::Display for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.source)
    }
}

impl From<Schedule> for String {
    fn from(s: Schedule) -> String {
        s.source
    }
}

impl TryFrom<String> for Schedule {
    type Error = anyhow::Error;
    fn try_from(s: String) -> anyhow::Result<Self> {
        Schedule::parse(&s)
    }
}

impl std::str::FromStr for Schedule {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        Schedule::parse(s)
    }
}

impl Schedule {
    /// Parse `minute hour day-of-month month day-of-week`, or one of the
    /// `@daily`-style aliases.
    ///
    /// `@reboot` is rejected rather than accepted-and-ignored: it means
    /// something in a crontab and nothing here, and a schedule that parses but
    /// never fires is exactly the failure this whole module is shaped to avoid.
    pub fn parse(expr: &str) -> anyhow::Result<Self> {
        let expr = expr.trim();
        let expanded = match expr.to_ascii_lowercase().as_str() {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            "@reboot" => anyhow::bail!(
                "`@reboot` has no meaning for a mecha trigger — there is no boot to hang \
                 it on. Use an explicit schedule."
            ),
            other if other.starts_with('@') => {
                anyhow::bail!(
                    "unknown schedule alias `{expr}` (known: @hourly, @daily, @midnight, \
                     @weekly, @monthly, @yearly)"
                )
            }
            _ => expr,
        };

        let fields: Vec<&str> = expanded.split_whitespace().collect();
        anyhow::ensure!(
            fields.len() == 5,
            "a cron schedule has five fields — minute hour day-of-month month day-of-week \
             — but `{expr}` has {}. (Seconds are not a field here: `0 7 * * *` is 7am.)",
            fields.len()
        );

        let minutes =
            parse_field(fields[0], 0, 59, &[]).map_err(|e| ctx("minute", fields[0], e))?;
        let hours = parse_field(fields[1], 0, 23, &[]).map_err(|e| ctx("hour", fields[1], e))?;
        let days =
            parse_field(fields[2], 1, 31, &[]).map_err(|e| ctx("day-of-month", fields[2], e))?;
        let months =
            parse_field(fields[3], 1, 12, MONTHS).map_err(|e| ctx("month", fields[3], e))?;
        let weekdays =
            parse_field(fields[4], 0, 7, WEEKDAYS).map_err(|e| ctx("day-of-week", fields[4], e))?;

        // Cron numbers Sunday as both 0 and 7; fold so matching only checks 0.
        let weekdays = if weekdays & (1 << 7) != 0 {
            (weekdays | 1) & !(1 << 7)
        } else {
            weekdays
        };

        Ok(Schedule {
            source: expr.to_string(),
            minutes,
            hours,
            days,
            months,
            weekdays,
            dom_restricted: fields[2] != "*",
            dow_restricted: fields[4] != "*",
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Does this local wall-clock date match the day fields?
    fn matches_day(&self, date: NaiveDate) -> bool {
        if self.months & (1 << date.month()) == 0 {
            return false;
        }
        let dom = self.days & (1 << date.day()) != 0;
        let dow = self.weekdays & (1 << date.weekday().num_days_from_sunday()) != 0;
        match (self.dom_restricted, self.dow_restricted) {
            // Vixie's rule: two restrictions are a union, not an intersection.
            // `0 0 13 * 5` is "the 13th, and every Friday", not "Friday the 13th".
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The first instant strictly after `after` at which this schedule fires.
    ///
    /// `None` only when the expression matches no date within four years —
    /// February 30th and friends.
    pub fn next_after(&self, after: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
        // Start from the minute after `after`, in local time: the search space
        // is wall-clock, which is the whole reason this is not arithmetic.
        let local = after.with_timezone(&tz);
        let mut date = local.date_naive();
        let mut from_minute = local.hour() * 60 + local.minute() + 1;

        for _ in 0..HORIZON_DAYS {
            if self.matches_day(date) {
                for minute in from_minute..24 * 60 {
                    if !self.matches_minute(minute) {
                        continue;
                    }
                    if let Some(utc) = self.resolve(date, minute, tz) {
                        // A fall-back hour repeats local times, so a resolved
                        // instant can land at or before where we started even
                        // though the local clock moved forward.
                        if utc > after {
                            return Some(utc);
                        }
                    }
                }
            }
            date = date.succ_opt()?;
            from_minute = 0;
        }
        None
    }

    /// The most recent instant at or before `at` at which this schedule fired.
    ///
    /// This is what answers "is it due?": compare it against the last slot that
    /// actually ran. A scheduler that missed forty slots owes one run, and this
    /// is the function that makes that true without enumerating the forty.
    pub fn prev_at_or_before(&self, at: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
        let local = at.with_timezone(&tz);
        let mut date = local.date_naive();
        let mut to_minute = local.hour() * 60 + local.minute();

        for _ in 0..HORIZON_DAYS {
            if self.matches_day(date) {
                for minute in (0..=to_minute).rev() {
                    if !self.matches_minute(minute) {
                        continue;
                    }
                    if let Some(utc) = self.resolve(date, minute, tz) {
                        if utc <= at {
                            return Some(utc);
                        }
                    }
                }
            }
            date = date.pred_opt()?;
            to_minute = 24 * 60 - 1;
        }
        None
    }

    fn matches_minute(&self, minute_of_day: u32) -> bool {
        self.hours & (1 << (minute_of_day / 60)) != 0
            && self.minutes & (1 << (minute_of_day % 60)) != 0
    }

    /// Turn a local wall-clock slot into a real instant.
    ///
    /// The two DST cases, each decided rather than defaulted:
    ///
    /// * **Ambiguous** (the hour ran twice): take the earlier. The job runs
    ///   once, on the first pass, and `next_after`'s "must be strictly later"
    ///   check keeps the second pass from firing it again.
    /// * **Gap** (the hour never happened): walk forward to the first minute
    ///   that exists. A 02:30 daily job fires at 03:00 on the spring-forward
    ///   day rather than silently skipping it — a scheduled run that vanishes
    ///   twice a year is worse than one that is half an hour late once.
    fn resolve(&self, date: NaiveDate, minute_of_day: u32, tz: Tz) -> Option<DateTime<Utc>> {
        let naive = date.and_hms_opt(minute_of_day / 60, minute_of_day % 60, 0)?;
        match tz.from_local_datetime(&naive) {
            LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
            LocalResult::Ambiguous(earlier, _) => Some(earlier.with_timezone(&Utc)),
            LocalResult::None => {
                // Gaps are an hour at most in every zone anyone has shipped;
                // search a little past that and give up rather than loop.
                let mut probe = naive;
                for _ in 0..180 {
                    probe += Duration::minutes(1);
                    match tz.from_local_datetime(&probe) {
                        LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
                        LocalResult::Ambiguous(earlier, _) => {
                            return Some(earlier.with_timezone(&Utc))
                        }
                        LocalResult::None => continue,
                    }
                }
                None
            }
        }
    }
}

const MONTHS: &[(&str, u32)] = &[
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

const WEEKDAYS: &[(&str, u32)] = &[
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];

fn ctx(field: &str, text: &str, e: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("{field} field `{text}`: {e}")
}

/// One field into a bitmask: `*`, `a`, `a-b`, `*/n`, `a-b/n`, and comma lists
/// of any of those. Names are accepted where cron accepts them.
fn parse_field(text: &str, min: u32, max: u32, names: &[(&str, u32)]) -> anyhow::Result<u64> {
    anyhow::ensure!(!text.is_empty(), "is empty");
    let mut mask = 0u64;

    for part in text.split(',') {
        let part = part.trim();
        anyhow::ensure!(!part.is_empty(), "has an empty item (a stray comma?)");

        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| anyhow::anyhow!("step `{s}` is not a number"))?;
                anyhow::ensure!(step > 0, "a step of 0 matches nothing");
                (r, step)
            }
            None => (part, 1),
        };

        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a, min, max, names)?, value(b, min, max, names)?)
        } else {
            let v = value(range, min, max, names)?;
            // `5/15` means "from 5, stepping" — same as `5-max/15`, as cron has
            // it. A bare `5` is just 5.
            if step > 1 {
                (v, max)
            } else {
                (v, v)
            }
        };
        anyhow::ensure!(lo <= hi, "range {lo}-{hi} runs backwards");

        let mut v = lo;
        while v <= hi {
            mask |= 1 << v;
            v += step;
        }
    }
    Ok(mask)
}

fn value(text: &str, min: u32, max: u32, names: &[(&str, u32)]) -> anyhow::Result<u32> {
    let text = text.trim();
    let n = match text.parse::<u32>() {
        Ok(n) => n,
        Err(_) => {
            let lower = text.to_ascii_lowercase();
            *names
                .iter()
                .find(|(name, _)| lower.starts_with(name))
                .map(|(_, v)| v)
                .ok_or_else(|| anyhow::anyhow!("`{text}` is not a number or a known name"))?
        }
    };
    anyhow::ensure!(n >= min && n <= max, "{n} is outside {min}-{max}");
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn ny() -> Tz {
        chrono_tz::America::New_York
    }

    #[test]
    fn five_fields_are_five_fields() {
        // The whole reason this module is hand-rolled: the crates' first field
        // is seconds, so this expression means 07:00 there only by accident.
        let s = Schedule::parse("0 7 * * *").unwrap();
        let next = s.next_after(utc("2026-08-05T00:00:00Z"), ny()).unwrap();
        assert_eq!(
            next.with_timezone(&ny()).to_string(),
            "2026-08-05 07:00:00 EDT"
        );

        // A six-field expression is an error, not a reinterpretation.
        let err = Schedule::parse("0 0 7 * * *").unwrap_err().to_string();
        assert!(err.contains("five fields"), "{err}");
        assert!(
            err.contains("7am"),
            "the message has to say what the user meant: {err}"
        );
    }

    #[test]
    fn steps_ranges_lists_and_names_all_parse() {
        let s = Schedule::parse("*/15 9-17 * * mon-fri").unwrap();
        let start = utc("2026-08-05T12:07:00Z"); // Wednesday, 08:07 EDT
        let next = s.next_after(start, ny()).unwrap();
        assert_eq!(
            next.with_timezone(&ny()).to_string(),
            "2026-08-05 09:00:00 EDT"
        );

        let s = Schedule::parse("30 3 1,15 jan,jul *").unwrap();
        let next = s.next_after(utc("2026-08-05T00:00:00Z"), ny()).unwrap();
        assert_eq!(
            next.with_timezone(&ny()).to_string(),
            "2027-01-01 03:30:00 EST"
        );

        // Weekend-only, by name, crossing a week boundary.
        let s = Schedule::parse("0 10 * * sat,sun").unwrap();
        let next = s.next_after(utc("2026-08-05T00:00:00Z"), ny()).unwrap();
        assert_eq!(next.with_timezone(&ny()).weekday(), chrono::Weekday::Sat);
    }

    #[test]
    fn aliases_expand_and_reboot_is_refused() {
        assert_eq!(Schedule::parse("@daily").unwrap().minutes, 1);
        assert_eq!(Schedule::parse("@hourly").unwrap().hours, u64::MAX >> 40);
        let err = Schedule::parse("@reboot").unwrap_err().to_string();
        assert!(err.contains("no meaning"), "{err}");
        assert!(Schedule::parse("@yesterday").is_err());
    }

    #[test]
    fn a_bad_field_says_which_field_and_what_was_wrong() {
        let err = Schedule::parse("0 25 * * *").unwrap_err().to_string();
        assert!(err.contains("hour"), "{err}");
        assert!(err.contains("outside 0-23"), "{err}");

        let err = Schedule::parse("0 7 * * funday").unwrap_err().to_string();
        assert!(err.contains("day-of-week"), "{err}");

        let err = Schedule::parse("*/0 * * * *").unwrap_err().to_string();
        assert!(err.contains("step of 0"), "{err}");
    }

    /// Vixie's rule, and the reason `dom_restricted`/`dow_restricted` exist:
    /// two day restrictions are a union, not an intersection.
    #[test]
    fn day_of_month_and_day_of_week_are_a_union_when_both_are_set() {
        // "the 13th, or any Friday" — not "Friday the 13th".
        let s = Schedule::parse("0 0 13 * fri").unwrap();
        let after = utc("2026-08-05T00:00:00Z"); // Wednesday
        let first = s.next_after(after, ny()).unwrap();
        assert_eq!(
            first.with_timezone(&ny()).day(),
            7,
            "Friday the 7th comes first"
        );
        let second = s.next_after(first, ny()).unwrap();
        assert_eq!(
            second.with_timezone(&ny()).day(),
            13,
            "then the 13th, itself a Thursday"
        );

        // With only one of them restricted, it is just that one.
        let s = Schedule::parse("0 0 13 * *").unwrap();
        let only = s.next_after(after, ny()).unwrap();
        assert_eq!(only.with_timezone(&ny()).day(), 13);
    }

    #[test]
    fn an_impossible_date_terminates_instead_of_searching_forever() {
        let s = Schedule::parse("0 0 30 2 *").unwrap();
        assert_eq!(s.next_after(utc("2026-08-05T00:00:00Z"), ny()), None);
        assert_eq!(s.prev_at_or_before(utc("2026-08-05T00:00:00Z"), ny()), None);
    }

    /// The spring-forward gap. A daily 02:30 job has no 02:30 to run at on the
    /// day the clocks jump; it must still run.
    #[test]
    fn a_job_inside_the_spring_forward_gap_still_fires() {
        // 2027-03-14: America/New_York jumps 02:00 EST → 03:00 EDT.
        let s = Schedule::parse("30 2 * * *").unwrap();
        let next = s.next_after(utc("2027-03-13T12:00:00Z"), ny()).unwrap();
        let local = next.with_timezone(&ny());
        assert_eq!(local.date_naive().to_string(), "2027-03-14");
        assert_eq!(
            local.to_string(),
            "2027-03-14 03:00:00 EDT",
            "the run is late, not lost — a schedule that silently skips a day twice a \
             year is a schedule you cannot build on"
        );
    }

    /// The fall-back hour happens twice. The job must not.
    #[test]
    fn a_job_inside_the_repeated_hour_fires_once() {
        // 2026-11-01: 02:00 EDT → 01:00 EST, so 01:30 happens twice.
        let s = Schedule::parse("30 1 * * *").unwrap();
        let first = s.next_after(utc("2026-10-31T12:00:00Z"), ny()).unwrap();
        assert_eq!(
            first.to_rfc3339(),
            "2026-11-01T05:30:00+00:00",
            "the earlier 01:30, EDT"
        );

        let second = s.next_after(first, ny()).unwrap();
        assert_eq!(
            second.with_timezone(&ny()).date_naive().to_string(),
            "2026-11-02",
            "the next fire is the following day, not the repeated 01:30 in EST"
        );

        // And the due check agrees: asked at the *second* 01:30, the most
        // recent slot is still the first one, which already fired.
        let during = utc("2026-11-01T06:30:00Z");
        assert_eq!(s.prev_at_or_before(during, ny()).unwrap(), first);
    }

    /// The property the whole scheduler rests on: a missed week owes one run.
    #[test]
    fn the_most_recent_slot_is_one_slot_however_long_the_gap() {
        let s = Schedule::parse("0 7 * * *").unwrap();
        let now = utc("2026-08-05T12:30:00Z"); // 08:30 EDT
        let prev = s.prev_at_or_before(now, ny()).unwrap();
        assert_eq!(
            prev.with_timezone(&ny()).to_string(),
            "2026-08-05 07:00:00 EDT"
        );

        // A month asleep does not change the answer, and costs no more work.
        let long_ago = utc("2026-07-01T00:00:00Z");
        assert!(prev > long_ago, "one slot owed, not thirty-five");
        assert_eq!(s.prev_at_or_before(now, ny()).unwrap(), prev);
    }

    #[test]
    fn prev_and_next_agree_on_a_slot_boundary() {
        let s = Schedule::parse("*/10 * * * *").unwrap();
        let exactly = utc("2026-08-05T12:30:00Z");
        // At the instant of a slot, that slot is the most recent one...
        assert_eq!(s.prev_at_or_before(exactly, ny()).unwrap(), exactly);
        // ...and the next is strictly later, so nothing fires twice.
        assert_eq!(
            s.next_after(exactly, ny()).unwrap(),
            utc("2026-08-05T12:40:00Z")
        );
    }

    #[test]
    fn the_timezone_is_the_users_not_the_machines() {
        let s = Schedule::parse("0 7 * * *").unwrap();
        let at = utc("2026-08-05T00:00:00Z");
        let in_ny = s.next_after(at, ny()).unwrap();
        let in_utc = s.next_after(at, chrono_tz::UTC).unwrap();
        assert_ne!(in_ny, in_utc, "07:00 is a wall-clock claim, not an instant");
        assert_eq!(in_utc.to_rfc3339(), "2026-08-05T07:00:00+00:00");
        assert_eq!(in_ny.to_rfc3339(), "2026-08-05T11:00:00+00:00");
    }

    #[test]
    fn a_schedule_round_trips_through_serde_as_what_the_user_typed() {
        let s = Schedule::parse("*/15 9-17 * * mon-fri").unwrap();
        let toml = toml::to_string(&serde_json::json!({"schedule": s.clone()})).unwrap();
        assert!(
            toml.contains(r#"schedule = "*/15 9-17 * * mon-fri""#),
            "{toml}"
        );
        let back: Schedule = serde_json::from_str(r#""*/15 9-17 * * mon-fri""#).unwrap();
        assert_eq!(back, s);
        assert!(serde_json::from_str::<Schedule>(r#""nonsense""#).is_err());
    }
}
