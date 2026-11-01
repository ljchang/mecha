//! Her world's store: `<persona>/world.db` (`docs/PERSONA-LIFE-DESIGN.md`
//! §5), machine-written, beside `memory.db`.
//!
//! Two tables so far:
//!
//! - **`days`**: sampled days (`persona::sampler`), one row a stretch, kept
//!   about two weeks. A day pruned past that leaves its week's minutes per
//!   activity in `weeks`, so the long view keeps a shape without the hours.
//! - **`state`**: slow facts about her life, superseded by a later row and
//!   never decaying.
//!
//! A day is sampled when first asked for and stored, so what a chat was told
//! about a day stays what the day was. Today's stored day is drawn again
//! when what it was drawn from has changed (her routine, her cell, the
//! table); a past one is kept, because the past does not change
//! when the routine does, and is read back even when the routine no longer
//! loads. A day older than [`KEEP_DAYS`] is drawn and returned, never
//! stored, so it is never folded into its week twice, and a day ahead is drawn
//! and never stored, since it draws the same way until it comes. Every row names its
//! source, and an unknown source is never clean; `days.source` is for
//! `forget` and the today-block's fold to read, and the sampler is its only
//! writer so far.

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Days, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use super::activity::Activity;
use super::atus::{self, Cell, Company};
use super::memory::{connect, now, uid};
use super::routine;
use super::sampler::{self, Carry, Day, Stretch};

pub const WORLD_DB: &str = "world.db";
/// How long a sampled day is kept whole.
pub const KEEP_DAYS: u64 = 14;
/// Bumped when the sampler would draw a different day from the same inputs,
/// so today's stored day is drawn again.
const SAMPLER_VERSION: &str = "1";
/// A state value is a few words: "nurse at the county hospital".
pub const MAX_STATE: usize = 200;

/// Where a world row came from (§5). A closed set on disk; a value this
/// binary does not know loads as `Unknown`, which is never clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Drawn by the day sampler from her routine and the time-use table.
    Sampler,
    /// Written by the owner.
    Owner,
    #[serde(other)]
    Unknown,
}

impl Source {
    pub fn is_clean(self) -> bool {
        matches!(self, Source::Sampler | Source::Owner)
    }
}

/// What a `state` row is about (§5: job, relationship status, home, health).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateKey {
    Job,
    Relationship,
    Home,
    Health,
}

/// A pruned week: the days folded into it, and minutes per activity.
#[derive(Debug, Clone, PartialEq)]
pub struct Week {
    pub days: u32,
    /// In [`Activity`] order.
    pub minutes: Vec<(Activity, u32)>,
    /// Minutes under an activity this binary does not know: counted here,
    /// so the week still adds up to its days.
    pub unreadable: u32,
}

/// A slow fact about her life, current until a later row supersedes it.
#[derive(Debug, Clone, PartialEq)]
pub struct StateRow {
    pub key: StateKey,
    pub value: String,
    pub source: Source,
    pub written_at: String,
}

/// One persona's `world.db`.
pub struct World {
    conn: Connection,
}

fn wire<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("a unit variant serialises as a string"),
    }
}

fn unwire<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > 1 {
        bail!("world.db is version {version}; this binary reads up to 1");
    }
    if version == 0 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE days (
                 date TEXT NOT NULL,
                 seq INTEGER NOT NULL,
                 activity TEXT NOT NULL,
                 start INTEGER NOT NULL,
                 end INTEGER NOT NULL CHECK (end > start),
                 company TEXT,
                 anchor TEXT,
                 source TEXT NOT NULL,
                 PRIMARY KEY (date, seq)
             );
             CREATE TABLE sampled (
                 date TEXT PRIMARY KEY,
                 cell TEXT NOT NULL,
                 inputs TEXT NOT NULL,
                 backoffs INTEGER NOT NULL,
                 carry_activity TEXT,
                 carry_label TEXT,
                 carry_until INTEGER,
                 sampled_at TEXT NOT NULL
             );
             CREATE TABLE weeks (
                 week TEXT NOT NULL,
                 activity TEXT NOT NULL,
                 minutes INTEGER NOT NULL,
                 PRIMARY KEY (week, activity)
             );
             CREATE TABLE week_days (
                 week TEXT PRIMARY KEY,
                 days INTEGER NOT NULL
             );
             CREATE TABLE state (
                 uid TEXT PRIMARY KEY,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 source TEXT NOT NULL,
                 written_at TEXT NOT NULL,
                 superseded_at TEXT
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    Ok(())
}

/// What a day is drawn from, as one digest: the routine's text, the table,
/// and the sampler's own version.
fn inputs(routine_text: &str) -> String {
    let mut h = Sha256::new();
    for part in [SAMPLER_VERSION, atus::TEXT_SHA256, routine_text] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The Monday a date's week starts on.
fn week_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

impl World {
    /// The persona's world, created (owner-only) if it is not there yet.
    pub fn open(pdir: &Path) -> Result<World> {
        let conn = connect(&pdir.join(WORLD_DB), true)?.expect("created");
        migrate(&conn)?;
        Ok(World { conn })
    }

    /// The persona's world if it has one, read-only and never created.
    fn open_existing(pdir: &Path) -> Result<Option<World>> {
        let Some(conn) = connect(&pdir.join(WORLD_DB), false)? else {
            return Ok(None);
        };
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            // Created and never migrated (an open that failed its checks):
            // nothing in it, and the next writable open says why.
            0 => Ok(None),
            1 => Ok(Some(World { conn })),
            v => bail!("world.db is version {v}; this binary reads up to 1"),
        }
    }

    /// The day stored for `date`, if there is one this binary can read
    /// whole, with the digest of what it was drawn from.
    fn stored(&self, date: NaiveDate) -> Result<Option<(Day, String)>> {
        let key = date.to_string();
        let head = self
            .conn
            .query_row(
                "SELECT cell, inputs, backoffs, carry_activity, carry_label, carry_until
                 FROM sampled WHERE date = ?1",
                [&key],
                |r| {
                    Ok((
                        (
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, u32>(2)?,
                        ),
                        (
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<u32>>(5)?,
                        ),
                    ))
                },
            )
            .optional()?;
        let Some(((cell, digest, backoffs), carry)) = head else {
            return Ok(None);
        };
        let Some(cell) = Cell::from_id(&cell) else {
            return Ok(None);
        };
        let carries = match carry {
            (None, None, None) => None,
            (Some(act), Some(label), Some(until)) => match Activity::from_id(&act) {
                Some(activity) => Some(Carry {
                    activity,
                    label,
                    until,
                }),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let mut stmt = self.conn.prepare(
            "SELECT activity, start, end, company, anchor FROM days WHERE date = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([&key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut stretches = Vec::new();
        for row in rows {
            let (activity, start, end, company, anchor) = row?;
            // A row this binary cannot read makes the day unreadable, and
            // an unreadable day is drawn again rather than shown in part.
            let Some(activity) = Activity::from_id(&activity) else {
                return Ok(None);
            };
            let company = match company {
                None => None,
                Some(c) => {
                    let kinds: Option<Vec<Company>> = c
                        .split(',')
                        .map(|id| Company::ALL.iter().copied().find(|k| k.id() == id))
                        .collect();
                    match kinds {
                        Some(k) => Some(k),
                        None => return Ok(None),
                    }
                }
            };
            stretches.push(Stretch {
                activity,
                start,
                end,
                company,
                anchor,
            });
        }
        // A day that is not back to back from 4am to 4am is not one this
        // binary can read whole either: drawn again, never shown in part.
        let contiguous = stretches
            .windows(2)
            .all(|w| w[0].end == w[1].start && w[0].start < w[0].end);
        if stretches
            .first()
            .is_none_or(|s| s.start != sampler::DAY_START)
            || stretches.last().is_none_or(|s| s.end != sampler::DAY_END)
            || !contiguous
        {
            return Ok(None);
        }
        let day = Day {
            date,
            cell,
            day_type: sampler::day_type(date),
            stretches,
            backoffs,
            carries,
        };
        Ok(Some((day, digest)))
    }

    /// Replace whatever is stored for the day with `day`.
    fn store(&mut self, day: &Day, digest: &str) -> Result<()> {
        let key = day.date.to_string();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM days WHERE date = ?1", [&key])?;
        for (seq, s) in day.stretches.iter().enumerate() {
            let company = s
                .company
                .as_ref()
                .map(|c| c.iter().map(|k| k.id()).collect::<Vec<_>>().join(","));
            tx.execute(
                "INSERT INTO days (date, seq, activity, start, end, company, anchor, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    key,
                    seq as i64,
                    s.activity.id(),
                    s.start,
                    s.end,
                    company,
                    s.anchor,
                    wire(&Source::Sampler)
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO sampled
                 (date, cell, inputs, backoffs, carry_activity, carry_label, carry_until, sampled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (date) DO UPDATE SET cell = ?2, inputs = ?3, backoffs = ?4,
                 carry_activity = ?5, carry_label = ?6, carry_until = ?7, sampled_at = ?8",
            params![
                key,
                day.cell.id(),
                digest,
                day.backoffs,
                day.carries.as_ref().map(|c| c.activity.id()),
                day.carries.as_ref().map(|c| c.label.as_str()),
                day.carries.as_ref().map(|c| c.until),
                now()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Fold every day before `today` less [`KEEP_DAYS`] into its week's
    /// minutes per activity, then delete it.
    pub fn prune(&mut self, today: NaiveDate) -> Result<usize> {
        let cutoff = (today - Days::new(KEEP_DAYS)).to_string();
        let tx = self.conn.transaction()?;
        let old: Vec<String> = {
            let mut stmt = tx.prepare("SELECT date FROM sampled WHERE date < ?1 ORDER BY date")?;
            let rows = stmt.query_map([&cutoff], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for date in &old {
            let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                continue;
            };
            let week = week_of(d).to_string();
            tx.execute(
                "INSERT INTO weeks (week, activity, minutes)
                 SELECT ?1, activity, SUM(end - start) FROM days WHERE date = ?2 GROUP BY activity
                 ON CONFLICT (week, activity) DO UPDATE SET minutes = minutes + excluded.minutes",
                params![week, date],
            )?;
            tx.execute(
                "INSERT INTO week_days (week, days) VALUES (?1, 1)
                 ON CONFLICT (week) DO UPDATE SET days = days + 1",
                [&week],
            )?;
        }
        tx.execute("DELETE FROM days WHERE date < ?1", [&cutoff])?;
        tx.execute("DELETE FROM sampled WHERE date < ?1", [&cutoff])?;
        tx.commit()?;
        Ok(old.len())
    }

    /// The week that starts on `monday`, once its days have been pruned.
    pub fn week(&self, monday: NaiveDate) -> Result<Option<Week>> {
        let key = monday.to_string();
        let days: Option<u32> = self
            .conn
            .query_row("SELECT days FROM week_days WHERE week = ?1", [&key], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(days) = days else {
            return Ok(None);
        };
        let mut stmt = self
            .conn
            .prepare("SELECT activity, minutes FROM weeks WHERE week = ?1")?;
        let rows = stmt.query_map([&key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        let (mut out, mut unreadable) = (Vec::new(), 0);
        for row in rows {
            let (act, minutes) = row?;
            match Activity::from_id(&act) {
                Some(a) => out.push((a, minutes)),
                None => unreadable += minutes,
            }
        }
        out.sort();
        Ok(Some(Week {
            days,
            minutes: out,
            unreadable,
        }))
    }

    /// Write a state row, superseding the current one for its key.
    pub fn set_state(&mut self, key: StateKey, value: &str, source: Source) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            bail!("a state value is empty");
        }
        if value.chars().count() > MAX_STATE {
            bail!("a state value is at most {MAX_STATE} characters");
        }
        if value.chars().any(char::is_control) {
            bail!("a state value holds a control character");
        }
        if source == Source::Unknown {
            bail!("a state row needs a known source");
        }
        let at = now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE state SET superseded_at = ?1 WHERE key = ?2 AND superseded_at IS NULL",
            params![at, wire(&key)],
        )?;
        tx.execute(
            "INSERT INTO state (uid, key, value, source, written_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uid(), wire(&key), value, wire(&source), at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The current state, one row a key, in [`StateKey`] order. A key this
    /// binary does not know is left out.
    pub fn state(&self) -> Result<Vec<StateRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, value, source, written_at FROM state WHERE superseded_at IS NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, value, source, written_at) = row?;
            if let Some(key) = unwire(&key) {
                out.push(StateRow {
                    key,
                    value,
                    source: unwire(&source).unwrap_or(Source::Unknown),
                    written_at,
                });
            }
        }
        out.sort_by_key(|r| r.key);
        Ok(out)
    }
}

/// Her day on `date`: the stored one, or one sampled and stored now, with
/// days past [`KEEP_DAYS`] folded into their weeks. Refused by name when
/// `[life] world` is off, when she has no routine, or when the routine names
/// no cell.
pub fn day(store_dir: &Path, name: &str, date: NaiveDate, today: NaiveDate) -> Result<Day> {
    let store = super::Store::load(store_dir);
    let Some(persona) = store.get(name) else {
        bail!("no persona named `{name}`");
    };
    if !persona.settings.life.world {
        bail!("[life] world is off for `{name}`: she has no simulated days");
    }
    let pdir = store_dir.join(&persona.name);
    let existing = World::open_existing(&pdir)?;
    // A past day already stored is what it was, whatever the routine says
    // now, or whether it still loads.
    if date < today {
        if let Some((stored, _)) = existing
            .as_ref()
            .map(|w| w.stored(date))
            .transpose()?
            .flatten()
        {
            return Ok(stored);
        }
    }
    let Some(text) = routine::read(store_dir, &persona.name)?.routine else {
        bail!(
            "`{name}` has no routine.toml, so no cell to draw her days from; \
             `mecha persona routine edit {name}` writes one"
        );
    };
    let routine =
        routine::parse(&text).with_context(|| format!("{} does not load", routine::FILE))?;
    let Some(cell) = routine.cell else {
        bail!(
            "`{name}`'s routine.toml names no cell, so there is no kind of week to draw her \
             days from; add `cell = \"…\"` (`mecha persona routine edit {name}`)"
        );
    };
    let digest = inputs(&text);
    // The day before opens this one with what it carried past 4am, as it
    // was stored; redrawn only when it is not stored, or is a stale
    // future day.
    let prev = date.pred_opt();
    let stored_prev = match (&existing, prev) {
        (Some(w), Some(p)) => w.stored(p)?.filter(|(_, was)| p < today || *was == digest),
        _ => None,
    };
    let before = match stored_prev {
        Some((d, _)) => d.carries,
        None => prev.and_then(|p| sampler::carried_from(&routine, &persona.name, p)),
    };
    let day = sampler::sample_after(atus::table(), cell, &routine, &persona.name, date, before);
    if date < today - Days::new(KEEP_DAYS) || date > today {
        // Outside what the store keeps. A day past it would be folded into
        // its week at once and again on every asking. A day ahead is drawn
        // the same way every time until it comes, and stored early it would
        // be kept as history once its date passed, drawn from a routine
        // that may have changed since and told to no one.
        return Ok(day);
    }
    let mut world = World::open(&pdir)?;
    if let Some((stored, was)) = world.stored(date)? {
        if was == digest {
            return Ok(stored);
        }
    }
    world.store(&day, &digest)?;
    world.prune(today)?;
    Ok(day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A store with one persona, `mara`, `[life] world` as given and
    /// `routine` hers.
    fn store(routine: Option<&str>, world: bool) -> std::path::PathBuf {
        use crate::persona::tests_support::*;
        let dir = scratch();
        crate::persona::create(&dir, &no_lib(), new("mara")).unwrap();
        fill_core(&dir, "mara");
        let toml = dir.join("mara/persona.toml");
        let text = std::fs::read_to_string(&toml).unwrap();
        assert!(text.contains("world     = false"));
        let text = text.replace("world     = false", &format!("world     = {world}"));
        std::fs::write(&toml, text).unwrap();
        if let Some(r) = routine {
            std::fs::write(dir.join("mara").join(routine::FILE), r).unwrap();
        }
        dir
    }

    #[test]
    fn a_day_is_refused_by_name_without_its_switch_routine_or_cell() {
        let (d, today) = (date("2026-11-02"), date("2026-11-02"));
        let off = store(Some("cell = \"student\"\n"), false);
        let why = format!("{:#}", day(off.as_path(), "mara", d, today).unwrap_err());
        assert!(why.contains("[life] world is off"), "{why}");
        let none = store(None, true);
        let why = format!("{:#}", day(none.as_path(), "mara", d, today).unwrap_err());
        assert!(why.contains("no routine.toml"), "{why}");
        let no_cell = store(Some(routine::TEMPLATE), true);
        let why = format!(
            "{:#}",
            day(no_cell.as_path(), "mara", d, today).unwrap_err()
        );
        assert!(why.contains("names no cell"), "{why}");
        assert!(!no_cell.as_path().join("mara").join(WORLD_DB).exists());
    }

    #[test]
    fn a_stored_day_is_read_back_as_drawn() {
        let dir = store(Some("cell = \"part_time\"\n"), true);
        let d = date("2026-11-02");
        let first = day(dir.as_path(), "mara", d, d).unwrap();
        let world = World::open(&dir.as_path().join("mara")).unwrap();
        let (back, _) = world.stored(d).unwrap().unwrap();
        assert_eq!(back, first);
        assert_eq!(day(dir.as_path(), "mara", d, d).unwrap(), first);
    }

    #[test]
    fn a_routine_change_redraws_today_and_keeps_the_past() {
        let dir = store(Some("cell = \"full_time\"\n"), true);
        let (yesterday, today) = (date("2026-11-01"), date("2026-11-02"));
        let past = day(dir.as_path(), "mara", yesterday, yesterday).unwrap();
        let now = day(dir.as_path(), "mara", today, today).unwrap();
        std::fs::write(
            dir.as_path().join("mara").join(routine::FILE),
            "cell = \"retired\"\n",
        )
        .unwrap();
        assert_eq!(day(dir.as_path(), "mara", yesterday, today).unwrap(), past);
        let redrawn = day(dir.as_path(), "mara", today, today).unwrap();
        assert_eq!(redrawn.cell, Cell::Retired);
        assert_ne!(redrawn, now);
    }

    #[test]
    fn an_unreadable_row_redraws_the_day_rather_than_showing_part_of_it() {
        let dir = store(Some("cell = \"student\"\n"), true);
        let d = date("2026-11-02");
        let first = day(dir.as_path(), "mara", d, d).unwrap();
        let world = World::open(&dir.as_path().join("mara")).unwrap();
        world
            .conn
            .execute("UPDATE days SET activity = 'juggling' WHERE seq = 0", [])
            .unwrap();
        assert!(world.stored(d).unwrap().is_none());
        assert_eq!(day(dir.as_path(), "mara", d, d).unwrap(), first);
    }

    #[test]
    fn days_past_two_weeks_leave_their_week_behind() {
        let dir = store(Some("cell = \"at_home\"\n"), true);
        let monday = date("2026-10-05");
        let mut drawn = Vec::new();
        for i in 0..7 {
            let d = monday + Days::new(i);
            drawn.push(day(dir.as_path(), "mara", d, d).unwrap());
        }
        let mut world = World::open(&dir.as_path().join("mara")).unwrap();
        assert_eq!(world.prune(monday + Days::new(30)).unwrap(), 7);
        assert!(world.stored(monday).unwrap().is_none());
        let Week {
            days,
            minutes,
            unreadable,
        } = world.week(monday).unwrap().unwrap();
        assert_eq!(unreadable, 0);
        assert_eq!(days, 7);
        let total: u32 = minutes.iter().map(|(_, m)| m).sum();
        assert_eq!(total, 7 * 24 * 60);
        let sleep: u32 = drawn
            .iter()
            .flat_map(|d| &d.stretches)
            .filter(|s| s.activity == Activity::Sleep)
            .map(|s| s.end - s.start)
            .sum();
        assert!(minutes.contains(&(Activity::Sleep, sleep)));
        assert_eq!(world.prune(monday + Days::new(30)).unwrap(), 0);
    }

    #[test]
    fn a_day_past_the_store_is_drawn_never_stored_or_folded() {
        let dir = store(Some("cell = \"at_home\"\n"), true);
        let (old, today) = (date("2026-10-05"), date("2026-11-02"));
        let first = day(dir.as_path(), "mara", old, today).unwrap();
        assert_eq!(day(dir.as_path(), "mara", old, today).unwrap(), first);
        let world = World::open(&dir.join("mara")).unwrap();
        assert!(world.stored(old).unwrap().is_none());
        assert!(world.week(old).unwrap().is_none(), "folded on an asking");
    }

    #[test]
    fn a_stored_past_day_is_read_back_when_the_routine_no_longer_loads() {
        let dir = store(Some("cell = \"student\"\n"), true);
        let (yesterday, today) = (date("2026-11-01"), date("2026-11-02"));
        let past = day(dir.as_path(), "mara", yesterday, yesterday).unwrap();
        std::fs::write(dir.join("mara").join(routine::FILE), "cell = \"nurse\"\n").unwrap();
        assert_eq!(day(dir.as_path(), "mara", yesterday, today).unwrap(), past);
        assert!(day(dir.as_path(), "mara", today, today).is_err());
    }

    #[test]
    fn a_night_shift_carries_what_the_stored_day_ran_not_the_edited_routine() {
        let shift = |minutes: u32| {
            format!(
                "cell = \"full_time\"\n[[anchor]]\nwhat = \"ward\"\nactivity = \"work\"\n\
                 days = [\"mon\"]\nat = \"22:00\"\nminutes = {minutes}\n"
            )
        };
        let dir = store(Some(&shift(480)), true);
        let (mon, tue) = (date("2026-01-05"), date("2026-01-06"));
        let monday = day(dir.as_path(), "mara", mon, mon).unwrap();
        assert_eq!(monday.carries.as_ref().map(|c| c.until), Some(6 * 60));
        // The owner lengthens the shift after Monday was drawn and told.
        std::fs::write(dir.join("mara").join(routine::FILE), shift(600)).unwrap();
        let tuesday = day(dir.as_path(), "mara", tue, tue).unwrap();
        let first = &tuesday.stretches[0];
        assert_eq!(first.anchor.as_deref(), Some("ward"));
        assert_eq!((first.start, first.end), (sampler::DAY_START, 6 * 60));
        assert_eq!(day(dir.as_path(), "mara", mon, tue).unwrap(), monday);
    }

    #[test]
    fn a_day_ahead_is_drawn_never_stored() {
        let dir = store(Some("cell = \"retired\"\n"), true);
        let today = date("2026-11-02");
        for ahead in [date("2026-11-03"), date("2099-01-01")] {
            let drawn = day(dir.as_path(), "mara", ahead, today).unwrap();
            assert_eq!(day(dir.as_path(), "mara", ahead, today).unwrap(), drawn);
            let world = World::open(&dir.join("mara")).unwrap();
            assert!(world.stored(ahead).unwrap().is_none());
        }
    }

    #[test]
    fn a_stored_day_with_a_hole_is_drawn_again_never_shown_in_part() {
        let dir = store(Some("cell = \"student\"\n"), true);
        let d = date("2026-11-02");
        let first = day(dir.as_path(), "mara", d, d).unwrap();
        let world = World::open(&dir.join("mara")).unwrap();
        world
            .conn
            .execute("DELETE FROM days WHERE seq = 1", [])
            .unwrap();
        assert!(world.stored(d).unwrap().is_none());
        assert_eq!(day(dir.as_path(), "mara", d, d).unwrap(), first);
    }

    #[test]
    fn a_week_counts_minutes_it_cannot_name_rather_than_dropping_them() {
        let dir = store(None, true);
        let world = World::open(&dir.join("mara")).unwrap();
        world
            .conn
            .execute_batch(
                "INSERT INTO week_days VALUES ('2026-10-05', 1);
                 INSERT INTO weeks VALUES ('2026-10-05', 'sleep', 480), ('2026-10-05', 'juggling', 960);",
            )
            .unwrap();
        let week = world.week(date("2026-10-05")).unwrap().unwrap();
        assert_eq!(week.minutes, vec![(Activity::Sleep, 480)]);
        assert_eq!(week.unreadable, 960);
    }

    #[test]
    fn state_is_superseded_never_decayed_and_unknown_is_never_clean() {
        let dir = store(None, true);
        let mut world = World::open(&dir.as_path().join("mara")).unwrap();
        world
            .set_state(StateKey::Job, "nurse on nights", Source::Owner)
            .unwrap();
        world
            .set_state(StateKey::Job, "nurse on days", Source::Owner)
            .unwrap();
        world
            .set_state(StateKey::Home, "a flat by the river", Source::Owner)
            .unwrap();
        let now = world.state().unwrap();
        assert_eq!(now.len(), 2);
        assert_eq!(now[0].key, StateKey::Job);
        assert_eq!(now[0].value, "nurse on days");
        for bad in ["", "   ", "a\u{1b}b"] {
            assert!(world
                .set_state(StateKey::Health, bad, Source::Owner)
                .is_err());
        }
        assert!(world
            .set_state(StateKey::Health, &"x".repeat(MAX_STATE + 1), Source::Owner)
            .is_err());
        assert!(world
            .set_state(StateKey::Health, "fine", Source::Unknown)
            .is_err());
        world
            .conn
            .execute("UPDATE state SET source = 'rumour' WHERE key = 'home'", [])
            .unwrap();
        world
            .conn
            .execute(
                "INSERT INTO state (uid, key, value, source, written_at) VALUES ('x', 'pets', 'a cat', 'owner', 'now')",
                [],
            )
            .unwrap();
        let now = world.state().unwrap();
        assert_eq!(now.len(), 2, "a key this binary does not know is left out");
        let home = now.iter().find(|r| r.key == StateKey::Home).unwrap();
        assert_eq!(home.source, Source::Unknown);
        assert!(!home.source.is_clean());
        assert!(Source::Sampler.is_clean() && Source::Owner.is_clean());
    }
}
