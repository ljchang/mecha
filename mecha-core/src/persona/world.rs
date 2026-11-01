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
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, Timelike};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use super::activity::Activity;
use super::atus::{self, Cell, Company};
use super::memory::{connect, now as stamp, uid};
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
/// The `weeks` activity a day no binary could read folds under: never an
/// [`Activity`] id, so [`World::week`] counts it as unreadable.
const UNREADABLE: &str = "?";

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

/// A stored day, the digest of what it was drawn from, and each
/// stretch's source, in stretch order.
struct Stored {
    day: Day,
    digest: String,
    sources: Vec<Source>,
}

/// What she is doing at one moment of her day.
#[derive(Debug, Clone, PartialEq)]
pub struct Now {
    /// The day it falls in, which runs 4am to 4am.
    pub date: NaiveDate,
    /// Minutes after that day's midnight; past 1440 before 4am.
    pub minute: u32,
    pub stretch: Stretch,
    /// Where the stretch came from; unknown is never clean.
    pub source: Source,
}

/// Whether a label read back from the store is one the routine's parser
/// would have let in (`routine::parse`'s `what`).
fn label_ok(label: &str) -> bool {
    !label.trim().is_empty()
        && label.trim() == label
        && label.chars().count() <= routine::MAX_WHAT
        && !label.chars().any(char::is_control)
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
    inputs_of(SAMPLER_VERSION, atus::TEXT_SHA256, routine_text)
}

/// [`inputs`] over its parts, so a test can vary each one.
fn inputs_of(version: &str, table: &str, routine_text: &str) -> String {
    let mut h = Sha256::new();
    for part in [version, table, routine_text] {
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
    /// whole, with the digest of what it was drawn from and each stretch's
    /// source.
    fn stored(&self, date: NaiveDate) -> Result<Option<Stored>> {
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
                Some(activity) if label_ok(&label) => Some(Carry {
                    activity,
                    label,
                    until,
                }),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let mut stmt = self.conn.prepare(
            "SELECT activity, start, end, company, anchor, source FROM days WHERE date = ?1
             ORDER BY seq",
        )?;
        let rows = stmt.query_map([&key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let (mut stretches, mut sources) = (Vec::new(), Vec::new());
        for row in rows {
            let (activity, start, end, company, anchor, source) = row?;
            // A row this binary cannot read makes the day unreadable, and
            // an unreadable day is drawn again rather than shown in part.
            let Some(activity) = Activity::from_id(&activity) else {
                return Ok(None);
            };
            let company = match company {
                None => None,
                Some(c) => {
                    let kinds: Option<Vec<Company>> = c.split(',').map(Company::from_id).collect();
                    match kinds {
                        // Alone is never beside anyone (`Stretch::company`).
                        Some(k) if !(k.len() > 1 && k.contains(&Company::Alone)) => Some(k),
                        _ => return Ok(None),
                    }
                }
            };
            // An anchor's label reaches her chat (`persona::today`), so it
            // is held to what the routine's own parser lets in.
            if anchor.as_deref().is_some_and(|a| !label_ok(a)) {
                return Ok(None);
            }
            stretches.push(Stretch {
                activity,
                start,
                end,
                company,
                anchor,
            });
            sources.push(unwire(&source).unwrap_or(Source::Unknown));
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
        Ok(Some(Stored {
            day,
            digest,
            sources,
        }))
    }

    /// Replace whatever is stored for the day with `day`, each stretch
    /// under its source in `sources` (stretch order; a missing one is
    /// unknown).
    fn store(&mut self, day: &Day, digest: &str, sources: &[Source]) -> Result<()> {
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
                    wire(&sources.get(seq).copied().unwrap_or(Source::Unknown))
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
                stamp()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Fold every day before `today` less [`KEEP_DAYS`] into its week's
    /// minutes per activity, then delete it.
    ///
    /// A day this binary cannot read whole folds as a whole day of
    /// unreadable minutes, so a week always adds up to its days.
    pub fn prune(&mut self, today: NaiveDate) -> Result<usize> {
        let cutoff = (today - Days::new(KEEP_DAYS)).to_string();
        // The write lock before the first read: the fold adds to a week and
        // deletes what it added from, so two processes folding from one
        // read (serve's chat turn and the CLI, on the first draw of a day)
        // would count a day twice, once as the minutes it held and once as a
        // day gone unreadable. `transaction()` would hold `conn` mutably
        // across `stored`, so the lock is taken by hand.
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match self.fold(&cutoff) {
            Ok(n) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(n)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// [`World::prune`]'s reads and writes, inside its transaction.
    fn fold(&self, cutoff: &str) -> Result<usize> {
        let old: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT date FROM sampled WHERE date < ?1 ORDER BY date")?;
            let rows = stmt.query_map([cutoff], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut folds = Vec::new();
        for date in &old {
            let Ok(d) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                continue;
            };
            let mut minutes: Vec<(&str, u32)> = Vec::new();
            match self.stored(d)? {
                Some(st) => {
                    for s in &st.day.stretches {
                        let id = s.activity.id();
                        match minutes.iter_mut().find(|(a, _)| *a == id) {
                            Some((_, m)) => *m += s.end - s.start,
                            None => minutes.push((id, s.end - s.start)),
                        }
                    }
                }
                None => minutes.push((UNREADABLE, sampler::MINUTES_PER_DAY)),
            }
            folds.push((week_of(d).to_string(), minutes));
        }
        for (week, minutes) in &folds {
            for (activity, m) in minutes {
                self.conn.execute(
                    "INSERT INTO weeks (week, activity, minutes) VALUES (?1, ?2, ?3)
                     ON CONFLICT (week, activity) DO UPDATE SET minutes = minutes + excluded.minutes",
                    params![week, activity, m],
                )?;
            }
            self.conn.execute(
                "INSERT INTO week_days (week, days) VALUES (?1, 1)
                 ON CONFLICT (week) DO UPDATE SET days = days + 1",
                [week],
            )?;
        }
        self.conn
            .execute("DELETE FROM days WHERE date < ?1", [cutoff])?;
        self.conn
            .execute("DELETE FROM sampled WHERE date < ?1", [cutoff])?;
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
        let at = stamp();
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
    match draw(store_dir, name, date, today)? {
        Drawn::Day(day, _) => Ok(day),
        Drawn::Absent(why) => bail!(why),
    }
}

/// What she is doing at `at`, the owner's local time: the stretch of her day
/// it falls in, read and stored as [`day`] does. A time before 4am is the
/// tail of the day before. `None` when she has no days to be in: `[life]
/// world` off, no routine, or a routine naming no cell.
pub fn now(store_dir: &Path, name: &str, at: NaiveDateTime) -> Result<Option<Now>> {
    let today = at.date();
    let minute = at.hour() * 60 + at.minute();
    let (date, minute) = if minute < sampler::DAY_START {
        match today.pred_opt() {
            Some(d) => (d, minute + sampler::MINUTES_PER_DAY),
            None => return Ok(None),
        }
    } else {
        (today, minute)
    };
    let (day, sources) = match draw(store_dir, name, date, today)? {
        Drawn::Day(day, sources) => (day, sources),
        Drawn::Absent(_) => return Ok(None),
    };
    let Some(i) = day
        .stretches
        .iter()
        .position(|s| s.start <= minute && minute < s.end)
    else {
        bail!("{name}'s day on {date} has no stretch at minute {minute}");
    };
    Ok(Some(Now {
        date,
        minute,
        stretch: day.stretches[i].clone(),
        source: sources.get(i).copied().unwrap_or(Source::Unknown),
    }))
}

/// A day, or why she has none.
enum Drawn {
    /// The day, with each stretch's source in stretch order.
    Day(Day, Vec<Source>),
    /// The refusal [`day`] gives, naming what is missing.
    Absent(String),
}

fn draw(store_dir: &Path, name: &str, date: NaiveDate, today: NaiveDate) -> Result<Drawn> {
    let store = super::Store::load(store_dir);
    let Some(persona) = store.get(name) else {
        bail!("no persona named `{name}`");
    };
    if !persona.settings.life.world {
        return Ok(Drawn::Absent(format!(
            "[life] world is off for `{name}`: she has no simulated days"
        )));
    }
    let pdir = store_dir.join(&persona.name);
    let existing = World::open_existing(&pdir)?;
    // A past day already stored is what it was, whatever the routine says
    // now, or whether it still loads.
    if date < today {
        if let Some(st) = existing
            .as_ref()
            .map(|w| w.stored(date))
            .transpose()?
            .flatten()
        {
            return Ok(Drawn::Day(st.day, st.sources));
        }
    }
    let Some(text) = routine::read(store_dir, &persona.name)?.routine else {
        return Ok(Drawn::Absent(format!(
            "`{name}` has no routine.toml, so no cell to draw her days from; \
             `mecha persona routine edit {name}` writes one"
        )));
    };
    let routine =
        routine::parse(&text).with_context(|| format!("{} does not load", routine::FILE))?;
    let Some(cell) = routine.cell else {
        return Ok(Drawn::Absent(format!(
            "`{name}`'s routine.toml names no cell, so there is no kind of week to draw her \
             days from; add `cell = \"…\"` (`mecha persona routine edit {name}`)"
        )));
    };
    let digest = inputs(&text);
    // The day before opens this one with what it carried past 4am, as it
    // was stored; redrawn only when it is not stored, or is a stale
    // future day.
    let prev = date.pred_opt();
    let stored_prev = match (&existing, prev) {
        (Some(w), Some(p)) => w.stored(p)?.filter(|st| p < today || st.digest == digest),
        _ => None,
    };
    // What the day before carried keeps the source of the stretch it ran
    // as, its last: a label read out of a row that is not clean is never
    // stamped clean by being carried past 4am.
    let (before, carried) = match stored_prev {
        Some(st) => {
            let from = st.sources.last().copied().unwrap_or(Source::Unknown);
            (st.day.carries, from)
        }
        None => (
            prev.and_then(|p| sampler::carried_from(&routine, &persona.name, p)),
            Source::Sampler,
        ),
    };
    let label = before.as_ref().map(|c| c.label.clone());
    let day = sampler::sample_after(atus::table(), cell, &routine, &persona.name, date, before);
    let mut drawn = vec![Source::Sampler; day.stretches.len()];
    if let (Some(label), Some(first)) = (label, day.stretches.first()) {
        if first.start == sampler::DAY_START && first.anchor.as_deref() == Some(label.as_str()) {
            drawn[0] = carried;
        }
    }
    if date < today - Days::new(KEEP_DAYS) || date > today {
        // Outside what the store keeps. A day past it would be folded into
        // its week at once and again on every asking. A day ahead is drawn
        // the same way every time until it comes, and stored early it would
        // be kept as history once its date passed, drawn from a routine
        // that may have changed since and told to no one.
        return Ok(Drawn::Day(day, drawn));
    }
    let mut world = World::open(&pdir)?;
    if let Some(st) = world.stored(date)? {
        if st.digest == digest {
            return Ok(Drawn::Day(st.day, st.sources));
        }
    }
    world.store(&day, &digest, &drawn)?;
    world.prune(today)?;
    Ok(Drawn::Day(day, drawn))
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
        let back = world.stored(d).unwrap().unwrap();
        assert_eq!(back.day, first);
        assert!(back.sources.iter().all(|s| *s == Source::Sampler));
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

    const NIGHT_SHIFT: &str = "cell = \"full_time\"\n[[anchor]]\nwhat = \"ward\"\n\
        activity = \"work\"\ndays = [\"mon\"]\nat = \"22:00\"\nminutes = 480\n";

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn now_puts_a_time_before_4am_in_the_day_before_and_carries_its_source() {
        let dir = store(Some(NIGHT_SHIFT), true);
        // Monday 2026-01-05's shift, at half past two on Tuesday.
        let n = now(dir.as_path(), "mara", at("2026-01-06 02:30"))
            .unwrap()
            .unwrap();
        assert_eq!(n.date, date("2026-01-05"));
        assert_eq!(n.minute, 26 * 60 + 30);
        assert_eq!(n.stretch.anchor.as_deref(), Some("ward"));
        assert_eq!(n.source, Source::Sampler);
        // At 4am the day is Tuesday's, which opens with the rest of the shift.
        let n = now(dir.as_path(), "mara", at("2026-01-06 04:00"))
            .unwrap()
            .unwrap();
        assert_eq!((n.date, n.minute), (date("2026-01-06"), sampler::DAY_START));
        assert_eq!(n.stretch.anchor.as_deref(), Some("ward"));
        // A row some other writer left, under a source this binary does not
        // know, is never clean.
        World::open(&dir.join("mara"))
            .unwrap()
            .conn
            .execute(
                "UPDATE days SET source = 'forged' WHERE date = '2026-01-05'",
                [],
            )
            .unwrap();
        let n = now(dir.as_path(), "mara", at("2026-01-06 02:30"))
            .unwrap()
            .unwrap();
        assert_eq!(n.source, Source::Unknown);
        // No switch, no routine or no cell: no day to be in, and no error.
        for (routine, world) in [(Some(NIGHT_SHIFT), false), (None, true), (Some(""), true)] {
            let dir = store(routine, world);
            assert_eq!(
                now(dir.as_path(), "mara", at("2026-01-06 12:00")).unwrap(),
                None
            );
        }
    }

    #[test]
    fn a_label_carried_out_of_a_day_that_is_not_clean_stays_unclean() {
        let dir = store(Some(NIGHT_SHIFT), true);
        let (mon, tue) = (date("2026-01-05"), date("2026-01-06"));
        day(dir.as_path(), "mara", mon, mon).unwrap();
        // Another writer's rows, under a source this binary does not know.
        World::open(&dir.join("mara"))
            .unwrap()
            .conn
            .execute(
                "UPDATE days SET source = 'model' WHERE date = '2026-01-05'",
                [],
            )
            .unwrap();
        let carried = now(dir.as_path(), "mara", at("2026-01-06 05:00"))
            .unwrap()
            .unwrap();
        assert_eq!(carried.date, tue);
        assert_eq!(carried.stretch.anchor.as_deref(), Some("ward"));
        assert_eq!(carried.source, Source::Unknown);
        // Stored that way, so a later read of Tuesday says the same; the rest
        // of Tuesday is the sampler's own draw.
        let world = World::open(&dir.join("mara")).unwrap();
        let st = world.stored(tue).unwrap().unwrap();
        assert_eq!(st.sources[0], Source::Unknown);
        assert!(st.sources[1..].iter().all(|s| *s == Source::Sampler));
    }

    #[test]
    fn alone_beside_anyone_makes_the_stored_day_unreadable() {
        let dir = store(Some("cell = \"student\"\n"), true);
        let d = date("2026-11-02");
        day(dir.as_path(), "mara", d, d).unwrap();
        let world = World::open(&dir.join("mara")).unwrap();
        world
            .conn
            .execute(
                "UPDATE days SET company = 'alone,friend'
                 WHERE seq = (SELECT MIN(seq) FROM days WHERE company IS NOT NULL)",
                [],
            )
            .unwrap();
        assert!(world.stored(d).unwrap().is_none());
    }

    #[test]
    fn a_stored_label_the_routine_would_refuse_makes_the_day_unreadable() {
        let dir = store(Some(NIGHT_SHIFT), true);
        let mon = date("2026-01-05");
        day(dir.as_path(), "mara", mon, mon).unwrap();
        let world = World::open(&dir.join("mara")).unwrap();
        assert!(world.stored(mon).unwrap().is_some());
        for (sql, back) in [
            (
                "UPDATE days SET anchor = 'ward\u{1b}[2J' WHERE anchor IS NOT NULL",
                "ward",
            ),
            (
                "UPDATE days SET anchor = '' WHERE anchor IS NOT NULL",
                "ward",
            ),
            (
                "UPDATE sampled SET carry_label = ' ward' WHERE carry_label IS NOT NULL",
                "ward",
            ),
        ] {
            world.conn.execute(sql, []).unwrap();
            assert!(world.stored(mon).unwrap().is_none(), "{sql}");
            world
                .conn
                .execute_batch(&format!(
                    "UPDATE days SET anchor = '{back}' WHERE anchor IS NOT NULL;
                     UPDATE sampled SET carry_label = '{back}' WHERE carry_label IS NOT NULL;"
                ))
                .unwrap();
            assert!(world.stored(mon).unwrap().is_some());
        }
        let long = "w".repeat(routine::MAX_WHAT + 1);
        world
            .conn
            .execute(
                "UPDATE days SET anchor = ?1 WHERE anchor IS NOT NULL",
                [&long],
            )
            .unwrap();
        assert!(world.stored(mon).unwrap().is_none());
    }

    #[test]
    fn a_day_nothing_can_read_folds_as_a_whole_day_of_unreadable_minutes() {
        let dir = store(Some("cell = \"student\"\n"), true);
        let monday = date("2026-10-05");
        for i in 0..3 {
            let d = monday + Days::new(i);
            day(dir.as_path(), "mara", d, d).unwrap();
        }
        let mut world = World::open(&dir.join("mara")).unwrap();
        // One day with a row this binary cannot name; one whose rows are gone.
        world
            .conn
            .execute_batch(
                "UPDATE days SET activity = 'juggling' WHERE date = '2026-10-06' AND seq = 0;
                 DELETE FROM days WHERE date = '2026-10-07';",
            )
            .unwrap();
        assert_eq!(world.prune(monday + Days::new(30)).unwrap(), 3);
        let week = world.week(monday).unwrap().unwrap();
        assert_eq!(week.days, 3);
        assert_eq!(week.unreadable, 2 * sampler::MINUTES_PER_DAY);
        let named: u32 = week.minutes.iter().map(|(_, m)| m).sum();
        assert_eq!(named + week.unreadable, 3 * sampler::MINUTES_PER_DAY);
    }

    #[test]
    fn every_input_moves_the_digest_and_no_two_splits_digest_alike() {
        let text = "cell = \"student\"\n";
        assert_eq!(
            inputs(text),
            inputs_of(SAMPLER_VERSION, atus::TEXT_SHA256, text)
        );
        let base = inputs_of("1", "abc", text);
        assert_ne!(inputs_of("2", "abc", text), base);
        assert_ne!(inputs_of("1", "abd", text), base);
        assert_ne!(inputs_of("1", "abc", "cell = \"retired\"\n"), base);
        assert_ne!(inputs_of("1a", "bc", text), inputs_of("1", "abc", text));
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
