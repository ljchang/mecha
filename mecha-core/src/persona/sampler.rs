//! The day sampler (`docs/PERSONA-LIFE-DESIGN.md` §4.2): one day of hers,
//! built from her routine's anchors and the time-use table, with no model.
//!
//! A day runs from 4am to 4am, as an ATUS diary does, so a night's sleep
//! belongs to the day it ends. The routine's anchors are placed first, each
//! drawn against its chance; the hours around them are filled from the
//! table's chain for her cell and the date's day type, and each stretch is
//! given who was there. The draw is seeded by persona and date, so a day is
//! the same every time it is asked for, and a replay can recompute it.
//!
//! **How a thin cell is read.** The table holds counts, and this is where
//! they become draws. A cell's counts are shrunk toward the pool's shares
//! by [`PRIOR`] stretches, so a row with a few stretches mostly follows the
//! pool and a row with thousands follows its cell; an activity the cell
//! never saw at that hour keeps the pool's chance of it. Where the pool has
//! nothing either, the nearest hour (or band, for durations) of the pool
//! stands in. Each such step past the cell is counted on the day
//! ([`Day::backoffs`]), so how often the table runs thin is measured.

use chrono::{Datelike, NaiveDate, Weekday};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::activity::Activity;
use super::atus::{Band, Cell, Company, DayType, Table};
use super::routine::Routine;

/// When a day begins, in minutes after midnight: an ATUS diary's 4am.
pub const DAY_START: u32 = 4 * 60;
/// A day's length in minutes.
pub const MINUTES_PER_DAY: u32 = 24 * 60;
/// When a day ends: 4am the next morning, in the day's own minutes.
pub const DAY_END: u32 = DAY_START + MINUTES_PER_DAY;
/// How many stretches of the pool's shares a cell's counts are blended
/// with: what a cell needs to outweigh the pool.
pub const PRIOR: f64 = 5.0;
/// The hours a night's sleep may end in when the day opens asleep.
const WAKE_HOURS: std::ops::RangeInclusive<u8> = 4..=13;

/// One stretch of a sampled day.
#[derive(Debug, Clone, PartialEq)]
pub struct Stretch {
    pub activity: Activity,
    /// Minutes after the day's midnight; past 1440 is the next morning,
    /// before 4am.
    pub start: u32,
    pub end: u32,
    /// Who was there, or `None` where ATUS does not ask (sleep, grooming,
    /// private activities). `[Alone]` is never beside anyone.
    pub company: Option<Vec<Company>>,
    /// The routine anchor that placed it, by its label, or `None` when the
    /// chain drew it.
    pub anchor: Option<String>,
}

/// A sampled day.
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub cell: Cell,
    pub day_type: DayType,
    /// Back to back from [`DAY_START`] to [`DAY_END`], never empty.
    pub stretches: Vec<Stretch>,
    /// Draws that had to read past her cell's row at that hour: the pool
    /// alone, a nearer hour or band, or the last resort.
    pub backoffs: u32,
    /// The anchor that runs past this day's end, which the next day opens
    /// with. Stored with the day, so the next day reads what this one was.
    pub carries: Option<Carry>,
}

impl Day {
    /// The day as `mecha persona day --json` and the week view print it:
    /// minutes after the day's midnight, so one past 1439 is the next
    /// morning.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "date": self.date.to_string(),
            "cell": self.cell.id(),
            "day_type": self.day_type.id(),
            "backoffs": self.backoffs,
            "stretches": self.stretches.iter().map(|s| serde_json::json!({
                "activity": s.activity.id(),
                "start_minute": s.start,
                "end_minute": s.end,
                "company": s.company.as_ref().map(|c| c.iter().map(|k| k.id()).collect::<Vec<_>>()),
                "anchor": s.anchor,
            })).collect::<Vec<_>>(),
            "carries": self.carries.as_ref().map(|c| serde_json::json!({
                "activity": c.activity.id(),
                "anchor": c.label,
                "until_minute": c.until,
            })),
        })
    }
}

/// An anchor that ran past a day's 4am.
#[derive(Debug, Clone, PartialEq)]
pub struct Carry {
    pub activity: Activity,
    pub label: String,
    /// Where it ends, in the next day's minutes: past [`DAY_START`].
    pub until: u32,
}

/// The day type ATUS would file `date` under.
pub fn day_type(date: NaiveDate) -> DayType {
    match date.weekday() {
        Weekday::Sat | Weekday::Sun => DayType::Weekend,
        _ => DayType::Weekday,
    }
}

/// The seed for one persona's day: the same persona and date, the same day.
pub fn seed(persona: &str, date: NaiveDate) -> u64 {
    let digest = Sha256::digest(format!("mecha-day\0{persona}\0{date}").as_bytes());
    u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// Sample `persona`'s day on `date`, in `cell`, around `routine`'s anchors.
pub fn sample(table: &Table, cell: Cell, routine: &Routine, persona: &str, date: NaiveDate) -> Day {
    let before = date
        .pred_opt()
        .and_then(|prev| carried_from(routine, persona, prev));
    sample_after(table, cell, routine, persona, date, before)
}

/// The anchor `routine` would carry out of `persona`'s `date`, redrawn:
/// what a day before that is not stored passes on. Anchors are the first
/// draws of a day's stream, so this needs nothing else of the day. A tail
/// carried into `date` can knock out one of its anchors by covering its
/// start, so the day before's own tail is placed first; that one is
/// redrawn from its day's anchors alone, which is exact unless a tail two
/// days back knocked out the anchor that would have knocked out another.
pub fn carried_from(routine: &Routine, persona: &str, date: NaiveDate) -> Option<Carry> {
    let own = |d: NaiveDate, before: Option<Carry>| {
        let mut draw = Draw(seed(persona, d));
        carry_out(&anchors_of(routine, d, &mut draw, before))
    };
    let before = date.pred_opt().and_then(|prev| own(prev, None));
    own(date, before)
}

/// The day's anchors in start order: the tail `before` carried in, then
/// its own, each drawn against its chance, the earlier start winning an
/// overlap.
fn anchors_of(
    routine: &Routine,
    date: NaiveDate,
    draw: &mut Draw,
    before: Option<Carry>,
) -> Vec<Placed> {
    let tail = before.filter(|c| c.until > DAY_START).map(|c| Placed {
        activity: c.activity,
        label: c.label,
        start: DAY_START,
        end: c.until.min(DAY_END),
        reaches: c.until,
    });
    keep_first(
        tail.into_iter()
            .chain(place_anchors(routine, date, draw))
            .collect(),
    )
}

/// The anchor among `anchors` that runs past the day's end.
fn carry_out(anchors: &[Placed]) -> Option<Carry> {
    anchors.iter().find(|a| a.reaches > DAY_END).map(|a| Carry {
        activity: a.activity,
        label: a.label.clone(),
        until: a.reaches - MINUTES_PER_DAY,
    })
}

/// [`sample`], opening with `before`: what the day before carried, as
/// stored when it was drawn.
pub fn sample_after(
    table: &Table,
    cell: Cell,
    routine: &Routine,
    persona: &str,
    date: NaiveDate,
    before: Option<Carry>,
) -> Day {
    let mut s = Sampler {
        table,
        cell,
        day: day_type(date),
        draw: Draw(seed(persona, date)),
        backoffs: 0,
    };
    let anchors = anchors_of(routine, date, &mut s.draw, before);
    let carries = carry_out(&anchors);
    let mut out: Vec<Stretch> = Vec::new();
    let mut next_anchor = 0;
    let (mut t, mut from) = (DAY_START, Activity::Sleep);
    // The day opens asleep, unless an anchor opens it.
    if anchors.first().is_none_or(|a| a.start > DAY_START) {
        let limit = anchors.first().map_or(DAY_END, |a| a.start);
        let wake = s.wake().min(limit);
        if wake > DAY_START {
            out.push(Stretch {
                activity: Activity::Sleep,
                start: DAY_START,
                end: wake,
                company: None,
                anchor: None,
            });
            t = wake;
        }
    }
    while t < DAY_END {
        if let Some(a) = anchors.get(next_anchor).filter(|a| a.start <= t) {
            next_anchor += 1;
            // A drawn stretch of the same activity cut short by the anchor
            // is the anchor begun early, never two stretches in a row.
            let start = out
                .pop_if(|l| l.activity == a.activity && l.anchor.is_none() && l.end == t)
                .map_or(t, |l| l.start);
            out.push(Stretch {
                activity: a.activity,
                start,
                end: a.end,
                company: s.company(a.activity),
                anchor: Some(a.label.clone()),
            });
            (t, from) = (a.end, a.activity);
            continue;
        }
        let limit = anchors.get(next_anchor).map_or(DAY_END, |a| a.start);
        let hour = ((t / 60) % 24) as u8;
        let act = s.next(from, hour);
        let end = (t + s.duration(act, Band::of(hour)).max(1)).min(limit);
        out.push(Stretch {
            activity: act,
            start: t,
            end,
            company: s.company(act),
            anchor: None,
        });
        (t, from) = (end, act);
    }
    Day {
        date,
        cell,
        day_type: s.day,
        stretches: out,
        backoffs: s.backoffs,
        carries,
    }
}

/// An anchor as placed on one day.
struct Placed {
    activity: Activity,
    label: String,
    start: u32,
    /// Where the day ends it: at [`DAY_END`] for one that runs on.
    end: u32,
    /// Where it would end, past [`DAY_END`] for one the next day carries.
    reaches: u32,
}

/// The anchors that happen on `date`, each drawn against its chance, in
/// start order: its weekday's from 4am on, and the next weekday's before
/// 4am, which the day reaches into. Overlaps are settled by [`keep_first`].
fn place_anchors(routine: &Routine, date: NaiveDate, draw: &mut Draw) -> Vec<Placed> {
    let today = date.weekday().num_days_from_monday();
    let mut placed: Vec<Placed> = Vec::new();
    for a in &routine.anchors {
        for (weekday, offset) in [(today, 0), ((today + 1) % 7, MINUTES_PER_DAY)] {
            let start = a.start + offset;
            if !a.on(weekday) || !(DAY_START..DAY_END).contains(&start) {
                continue;
            }
            // Drawn even at a chance of 1, so one anchor's chance never
            // shifts the draws of the rest of the day.
            if draw.unit() >= a.chance {
                continue;
            }
            placed.push(Placed {
                activity: a.activity,
                label: a.what.clone().unwrap_or_else(|| a.activity.id().into()),
                start,
                end: (start + a.minutes).min(DAY_END),
                reaches: start + a.minutes,
            });
        }
    }
    keep_first(placed)
}

/// In start order, dropping any that overlaps one kept before it: the
/// earlier start wins, the earlier in the list on a tie.
fn keep_first(mut placed: Vec<Placed>) -> Vec<Placed> {
    placed.sort_by_key(|p| p.start);
    let mut kept: Vec<Placed> = Vec::new();
    for p in placed {
        if kept.last().is_none_or(|k| p.start >= k.end) {
            kept.push(p);
        }
    }
    kept
}

struct Sampler<'a> {
    table: &'a Table,
    cell: Cell,
    day: DayType,
    draw: Draw,
    backoffs: u32,
}

impl Sampler<'_> {
    /// When a night's sleep ends, for a day that opens asleep: an hour drawn
    /// from when the table's sleep stretches ended, a minute within it. The
    /// cell's counts across the hours are blended with the pool's shares
    /// across them, as a row is ([`Sampler::blend`]), so an hour the cell has
    /// no row for keeps the pool's chance of it at the cell's scale.
    fn wake(&mut self) -> u32 {
        let ended = |cell: Option<Cell>, hour: u8| -> Option<f64> {
            self.table
                .next(cell, self.day, hour, Activity::Sleep)
                .map(|w| w.values().sum())
        };
        let pool: BTreeMap<u8, f64> = WAKE_HOURS
            .map(|h| (h, ended(None, h).unwrap_or(0.0)))
            .collect();
        let pool_total: f64 = pool.values().sum();
        let cell: BTreeMap<u8, Option<f64>> =
            WAKE_HOURS.map(|h| (h, ended(Some(self.cell), h))).collect();
        if cell.values().all(Option::is_none) {
            self.backoffs += 1;
        }
        let by_hour: BTreeMap<u8, f64> = WAKE_HOURS
            .map(|h| {
                let share = if pool_total > 0.0 {
                    pool[&h] / pool_total
                } else {
                    0.0
                };
                (h, cell[&h].unwrap_or(0.0) + PRIOR * share)
            })
            .collect();
        let hour = match self.draw.pick(&by_hour) {
            Some(h) => h,
            None => {
                self.backoffs += 1;
                7
            }
        };
        u32::from(hour) * 60 + (self.draw.unit() * 60.0) as u32
    }

    /// What follows `from` at `hour`.
    fn next(&mut self, from: Activity, hour: u8) -> Activity {
        let day = self.day;
        if let Some((w, pooled)) = self.blend(|c| self.table.next(c, day, hour, from).cloned()) {
            if let Some(a) = self.draw.pick(&w) {
                self.backoffs += u32::from(pooled);
                return a;
            }
        }
        self.backoffs += 1;
        // The pool's nearest hour that saw `from` end, later first.
        for d in 1..=12u8 {
            for h in [(hour + d) % 24, (hour + 24 - d) % 24] {
                if let Some(a) = self
                    .table
                    .next(None, day, h, from)
                    .and_then(|w| self.draw.pick(w))
                {
                    return a;
                }
            }
        }
        // Every activity ends somewhere in the pool on either day type
        // (`persona::atus`'s tests), so this is a table that broke them.
        if from == Activity::Sleep {
            Activity::PersonalCare
        } else {
            Activity::Sleep
        }
    }

    /// How long `act` lasts, begun in `band`, in minutes.
    fn duration(&mut self, act: Activity, band: Band) -> u32 {
        let u = self.draw.unit();
        if let Some(d) = self.table.durations(Some(self.cell), self.day, band, act) {
            return d.at(u);
        }
        self.backoffs += 1;
        // The pool's: this band, then the nearest, later first.
        let (n, at) = (
            Band::ALL.len(),
            Band::ALL.iter().position(|b| *b == band).expect("a band"),
        );
        for d in 0..n {
            for i in [(at + d) % n, (at + n - d) % n] {
                if let Some(d) = self.table.durations(None, self.day, Band::ALL[i], act) {
                    return d.at(u);
                }
            }
        }
        30
    }

    /// Who was there for a stretch of `act`: alone, or a set of kinds of
    /// company, from the cell's shares blended with the pool's. `None` where
    /// ATUS does not ask.
    fn company(&mut self, act: Activity) -> Option<Vec<Company>> {
        let pool = self.table.company(None, self.day, act)?;
        let cell = self.table.company(Some(self.cell), self.day, act);
        if cell.is_none() {
            self.backoffs += 1;
        }
        let share = |who: Company| {
            let p = pool.share(who).unwrap_or(0.0);
            match cell {
                Some(c) => (c.present[who as usize] + PRIOR * p) / (c.weight + PRIOR),
                None => p,
            }
        };
        let alone = share(Company::Alone);
        let (u, others) = (self.draw.unit(), &Company::ALL[1..]);
        // Drawn for every kind whatever `u` says, so the stream's length
        // never depends on a draw.
        let rolls: Vec<f64> = others.iter().map(|_| self.draw.unit()).collect();
        if u < alone || alone >= 1.0 {
            return Some(vec![Company::Alone]);
        }
        let given = |who: Company| (share(who) / (1.0 - alone)).min(1.0);
        let mut with: Vec<Company> = others
            .iter()
            .zip(&rolls)
            .filter(|(who, roll)| **roll < given(**who))
            .map(|(who, _)| *who)
            .collect();
        if with.is_empty() {
            // Someone was there: the kind most likely to be. One more draw,
            // only here, so the stream's length depends on this stretch's
            // rolls; it still depends on nothing outside the day.
            let weights: BTreeMap<Company, f64> =
                others.iter().map(|who| (*who, share(*who))).collect();
            with.push(self.draw.pick(&weights).unwrap_or(Company::Public));
        }
        Some(with)
    }

    /// A transition row for her cell blended with the pool's shares, or the
    /// pool's alone when the cell has none, with whether it was the pool
    /// alone. `None` when neither has a row.
    fn blend(
        &self,
        row: impl Fn(Option<Cell>) -> Option<BTreeMap<Activity, f64>>,
    ) -> Option<(BTreeMap<Activity, f64>, bool)> {
        let pool = row(None)?;
        let total: f64 = pool.values().sum();
        if total <= 0.0 {
            return None;
        }
        let Some(mut cell) = row(Some(self.cell)) else {
            return Some((pool, true));
        };
        for (act, n) in &pool {
            *cell.entry(*act).or_default() += PRIOR * n / total;
        }
        Some((cell, false))
    }
}

/// Seeded uniform draws: splitmix64, like `sample::SeedStream`, kept here so
/// the sampler does not widen that module's role.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// In [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A key drawn by weight, in the map's order; `None` when the weights
    /// sum to nothing. One draw either way.
    fn pick<K: Copy>(&mut self, weights: &BTreeMap<K, f64>) -> Option<K> {
        let u = self.unit();
        let total: f64 = weights.values().filter(|w| **w > 0.0).sum();
        if total <= 0.0 {
            return None;
        }
        let mut left = u * total;
        let mut last = None;
        for (k, w) in weights.iter().filter(|(_, w)| **w > 0.0) {
            last = Some(*k);
            if left < *w {
                return last;
            }
            left -= w;
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::atus::table;
    use crate::persona::routine;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn empty() -> Routine {
        routine::parse("cell = \"full_time\"\n").unwrap()
    }

    fn check_shape(day: &Day) {
        let s = &day.stretches;
        assert_eq!(s.first().unwrap().start, DAY_START);
        assert_eq!(s.last().unwrap().end, DAY_END);
        for w in s.windows(2) {
            assert_eq!(w[0].end, w[1].start, "{:?}", day.date);
            assert!(
                w[0].activity != w[1].activity || w[0].anchor.is_some() || w[1].anchor.is_some(),
                "{} follows itself at {}",
                w[0].activity.id(),
                w[0].end
            );
        }
        for st in s {
            assert!(st.end > st.start, "{st:?}");
            if let Some(c) = &st.company {
                assert!(!c.is_empty());
                assert!(c.len() == 1 || !c.contains(&Company::Alone), "{c:?}");
            }
        }
    }

    #[test]
    fn a_day_is_the_same_every_time_and_differs_by_persona_and_date() {
        let r = empty();
        let d = date("2026-11-02");
        let a = sample(table(), Cell::FullTime, &r, "mara", d);
        assert_eq!(a, sample(table(), Cell::FullTime, &r, "mara", d));
        assert_ne!(a, sample(table(), Cell::FullTime, &r, "jun", d));
        assert_ne!(
            a,
            sample(table(), Cell::FullTime, &r, "mara", date("2026-11-03"))
        );
    }

    #[test]
    fn every_day_runs_back_to_back_from_4am_to_4am() {
        let r = empty();
        let mut d = date("2026-01-01");
        for _ in 0..60 {
            for cell in Cell::ALL {
                check_shape(&sample(table(), cell, &r, "mara", d));
            }
            d = d.succ_opt().unwrap();
        }
    }

    #[test]
    fn a_certain_anchor_is_placed_as_written_and_a_soft_one_by_its_chance() {
        let r = routine::parse(
            "cell = \"student\"\n\
             [[anchor]]\nwhat = \"lecture\"\nactivity = \"class\"\ndays = [\"weekdays\"]\nat = \"09:00\"\nminutes = 90\n\
             [[anchor]]\nwhat = \"run\"\nactivity = \"exercise\"\ndays = [\"daily\"]\nat = \"18:00\"\nminutes = 45\nchance = 0.5\n",
        )
        .unwrap();
        let (mut runs, mut days) = (0, 0);
        let mut d = date("2026-01-05"); // a Monday
        for _ in 0..140 {
            let day = sample(table(), Cell::Student, &r, "mara", d);
            check_shape(&day);
            let lecture = day
                .stretches
                .iter()
                .find(|s| s.anchor.as_deref() == Some("lecture"));
            if day.day_type == DayType::Weekday {
                let l = lecture.expect("a weekday has its lecture");
                assert_eq!(l.end, 9 * 60 + 90);
                assert!(l.start <= 9 * 60);
            } else {
                assert!(lecture.is_none());
            }
            days += 1;
            runs += day
                .stretches
                .iter()
                .filter(|s| s.anchor.as_deref() == Some("run"))
                .count();
            d = d.succ_opt().unwrap();
        }
        let rate = f64::from(runs as u32) / f64::from(days);
        assert!((0.38..=0.62).contains(&rate), "the run happened {rate}");
    }

    #[test]
    fn an_anchor_before_4am_belongs_to_the_day_before() {
        // Friday 01:00 is the end of Thursday's day.
        let r = routine::parse(
            "cell = \"student\"\n[[anchor]]\nwhat = \"late set\"\nactivity = \"socializing\"\ndays = [\"fri\"]\nat = \"01:00\"\nminutes = 60\n",
        )
        .unwrap();
        let thu = sample(table(), Cell::Student, &r, "mara", date("2026-01-08"));
        let fri = sample(table(), Cell::Student, &r, "mara", date("2026-01-09"));
        let late = |d: &Day| d.stretches.iter().any(|s| s.anchor.is_some());
        assert!(late(&thu) && !late(&fri));
        let s = thu.stretches.iter().find(|s| s.anchor.is_some()).unwrap();
        assert_eq!((s.end - 60), MINUTES_PER_DAY + 60);
        check_shape(&thu);
    }

    #[test]
    fn a_night_shift_past_4am_carries_into_the_next_day() {
        let r = routine::parse(
            "cell = \"full_time\"\n[[anchor]]\nwhat = \"ward\"\nactivity = \"work\"\ndays = [\"mon\"]\nat = \"22:00\"\nminutes = 480\n",
        )
        .unwrap();
        let mon = sample(table(), Cell::FullTime, &r, "mara", date("2026-01-05"));
        let tue = sample(table(), Cell::FullTime, &r, "mara", date("2026-01-06"));
        let wed = sample(table(), Cell::FullTime, &r, "mara", date("2026-01-07"));
        let ward = |d: &Day| -> Vec<(u32, u32)> {
            d.stretches
                .iter()
                .filter(|s| s.anchor.as_deref() == Some("ward"))
                .map(|s| (s.start, s.end))
                .collect()
        };
        assert_eq!(ward(&mon).last().map(|w| w.1), Some(DAY_END));
        assert_eq!(mon.carries.as_ref().map(|c| c.until), Some(6 * 60));
        assert_eq!(carried_from(&r, "mara", date("2026-01-05")), mon.carries);
        assert!(tue.carries.is_none());
        // Tuesday opens on the shift's last two hours, not asleep.
        assert_eq!(ward(&tue), vec![(DAY_START, 6 * 60)]);
        assert_eq!(tue.stretches[0].activity, Activity::Work);
        assert!(ward(&wed).is_empty());
        for d in [&mon, &tue, &wed] {
            check_shape(d);
        }
    }

    #[test]
    fn the_next_day_opens_on_what_was_carried_not_what_the_routine_says_now() {
        // The day before as stored carried two hours; the routine now says
        // the shift ends at 4am. The stored carry wins.
        let now = routine::parse("cell = \"full_time\"\n").unwrap();
        let carried = Carry {
            activity: Activity::Work,
            label: "ward".into(),
            until: 6 * 60,
        };
        let tue = sample_after(
            table(),
            Cell::FullTime,
            &now,
            "mara",
            date("2026-01-06"),
            Some(carried),
        );
        assert_eq!(tue.stretches[0].anchor.as_deref(), Some("ward"));
        assert_eq!(tue.stretches[0].end, 6 * 60);
        let plain = sample_after(
            table(),
            Cell::FullTime,
            &now,
            "mara",
            date("2026-01-06"),
            None,
        );
        assert_eq!(plain.stretches[0].activity, Activity::Sleep);
        check_shape(&tue);
    }

    #[test]
    fn a_carried_tail_that_covers_an_anchors_start_knocks_it_out_of_the_carry() {
        // Monday's day-long anchor carries into Tuesday past the start of
        // Tuesday's long one, which is dropped, so Tuesday carries nothing,
        // whether drawn whole or redrawn for Wednesday's sake.
        let r = routine::parse(
            "cell = \"full_time\"\n\
             [[anchor]]\nwhat = \"long\"\nactivity = \"work\"\ndays = [\"mon\"]\nat = \"20:00\"\nminutes = 1440\n\
             [[anchor]]\nwhat = \"late\"\nactivity = \"socializing\"\ndays = [\"tue\"]\nat = \"14:00\"\nminutes = 1000\nchance = 0.9\n",
        )
        .unwrap();
        let tue = date("2026-01-06");
        let drawn = sample(table(), Cell::FullTime, &r, "mara", tue);
        assert_eq!(drawn.stretches[0].anchor.as_deref(), Some("long"));
        assert!(drawn.carries.is_none());
        assert_eq!(carried_from(&r, "mara", tue), drawn.carries);
        check_shape(&drawn);
    }

    #[test]
    fn the_cell_moves_the_day() {
        // Coarse: over many days, a full-time worker works far more on a
        // weekday than a retiree, and a student is in class more than both.
        let r = empty();
        let minutes = |cell: Cell, act: Activity| -> u32 {
            let mut d = date("2026-03-02");
            let mut total = 0;
            for _ in 0..70 {
                if day_type(d) == DayType::Weekday {
                    total += sample(table(), cell, &r, "mara", d)
                        .stretches
                        .iter()
                        .filter(|s| s.activity == act)
                        .map(|s| s.end - s.start)
                        .sum::<u32>();
                }
                d = d.succ_opt().unwrap();
            }
            total
        };
        let work = minutes(Cell::FullTime, Activity::Work);
        assert!(
            work > 4 * minutes(Cell::Retired, Activity::Work) + 1,
            "{work}"
        );
        assert!(minutes(Cell::Student, Activity::Class) > minutes(Cell::FullTime, Activity::Class));
        // An average full-time weekday holds hours of work, not minutes.
        assert!(work / 50 > 180, "{} a weekday", work / 50);
    }

    #[test]
    fn a_thin_row_follows_the_pool_and_a_thick_one_its_cell() {
        let s = Sampler {
            table: table(),
            cell: Cell::Student,
            day: DayType::Weekday,
            draw: Draw(1),
            backoffs: 0,
        };
        let mut cell = BTreeMap::new();
        cell.insert(Activity::Work, 1.0);
        let mut pool = BTreeMap::new();
        pool.insert(Activity::Work, 10.0);
        pool.insert(Activity::Eating, 990.0);
        let (w, pooled) = s
            .blend(|c| {
                Some(if c.is_some() {
                    cell.clone()
                } else {
                    pool.clone()
                })
            })
            .unwrap();
        assert!(!pooled);
        // One stretch of work against PRIOR stretches of the pool's shares.
        assert!((w[&Activity::Work] - (1.0 + PRIOR * 0.01)).abs() < 1e-9);
        assert!((w[&Activity::Eating] - PRIOR * 0.99).abs() < 1e-9);
        let only_pool = s
            .blend(|c| {
                if c.is_some() {
                    None
                } else {
                    Some(pool.clone())
                }
            })
            .unwrap();
        assert_eq!(only_pool, (pool, true));
        assert!(s.blend(|_| None).is_none());
    }

    #[test]
    fn a_draw_by_weight_lands_in_proportion() {
        let mut d = Draw(42);
        let mut w = BTreeMap::new();
        w.insert(1, 1.0);
        w.insert(2, 3.0);
        w.insert(3, 0.0);
        let mut n = [0u32; 4];
        for _ in 0..4000 {
            n[d.pick(&w).unwrap()] += 1;
        }
        assert_eq!(n[3], 0);
        assert!((2800..3200).contains(&n[2]), "{n:?}");
        assert!(d.pick(&BTreeMap::<u8, f64>::new()).is_none());
    }
}
