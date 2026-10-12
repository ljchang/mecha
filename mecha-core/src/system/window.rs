//! A measurement over a window of the series (`docs/SYSTEM-STATE-DESIGN.md`
//! §3.5's `read`, S3; and the model tool's windows, §5, S5).
//!
//! A window is summarised, never dumped: a day of minutes is 1,440 rows, and
//! the readers here are a person at a terminal and a model's context. So a
//! read returns the latest value, min, mean, max and sum, how many minutes
//! held a value and how many were unknown, and a dozen-or-so bucket means.
//! Unknown is never zero here either: a minute whose value is NULL counts as
//! unknown and stays out of every statistic, and a bucket with no value is
//! `None`, not 0.
//!
//! Read-only, like every reader of the series.

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

use super::measure::{How, Measurement};
use super::Category;

/// The windows a read may ask for — closed, as the model tool's schema is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Window {
    Now,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
}

impl Window {
    pub const ALL: [Window; 5] = [Window::Now, Window::H1, Window::H6, Window::H24, Window::D7];

    pub fn name(self) -> &'static str {
        match self {
            Window::Now => "now",
            Window::H1 => "1h",
            Window::H6 => "6h",
            Window::H24 => "24h",
            Window::D7 => "7d",
        }
    }

    pub fn parse(s: &str) -> Option<Window> {
        Window::ALL.into_iter().find(|w| w.name() == s)
    }

    /// How far back, and how wide each bucket is. `None` for `now`, which is
    /// a probe, not a window.
    pub fn span(self) -> Option<(Duration, Duration)> {
        match self {
            Window::Now => None,
            Window::H1 => Some((Duration::hours(1), Duration::minutes(5))),
            Window::H6 => Some((Duration::hours(6), Duration::minutes(30))),
            Window::H24 => Some((Duration::hours(24), Duration::hours(2))),
            Window::D7 => Some((Duration::days(7), Duration::hours(12))),
        }
    }
}

/// One stretch of a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bucket {
    /// Its first minute, `YYYY-MM-DD HH:MM:SS` UTC as the series stores it.
    pub start: String,
    /// The mean of the minutes in it that held a value; `None` when none did.
    pub mean: Option<f64>,
    /// Their sum — what a stretch of a per-minute count shows.
    pub sum: Option<f64>,
}

/// One measurement (or one category of one) over a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The window's length in minutes — the denominator: a sampler that was
    /// down records fewer rows, and that gap is said, never folded away
    /// (review of #660).
    pub window_minutes: usize,
    /// Minutes the series has a row for in the window.
    pub minutes: usize,
    /// Of those, the minutes whose value was unknown.
    pub unknown: usize,
    /// The newest minute that held a value, and that value.
    pub latest: Option<(String, f64)>,
    pub min: Option<f64>,
    pub mean: Option<f64>,
    pub max: Option<f64>,
    /// The sum — meaningful for a per-minute count (OOM kills), not a level.
    pub sum: Option<f64>,
    pub buckets: Vec<Bucket>,
}

/// What a window read found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Windowed {
    One(Summary),
    /// One summary per kind of work, and the kinds withheld (R7).
    ByCategory {
        rows: Vec<(&'static str, Summary)>,
        withheld: Vec<&'static str>,
    },
    /// The series predates the table this measurement is kept in: said, not
    /// an error (as `system::latest` says it).
    OlderSampler,
    /// The sampler ran and recorded no row for it: this machine does not
    /// have it (a GPU, Wi-Fi). Kept apart from a sampler that did not run,
    /// as a `Now` read keeps `NotHere` apart (review of #660).
    NotHere,
    /// Readable now only, never over a window (R7): its history is a record
    /// of when calls happened.
    Withheld,
}

/// R7, in the type rather than a formatter: what moves with a call is never
/// read over a window — `voice.live`, the voice category, the speech
/// models' rows. Every caller of this module gets it, so a later `--json`
/// or a second reader cannot leak call timing by forgetting (review of #660).
pub fn withheld_over_a_window(m: Measurement) -> bool {
    m == Measurement::VoiceLive
}

/// A model kind whose rows move with a call (R7).
pub fn is_speech(kind: u8) -> bool {
    use super::models::ModelKind;
    matches!(
        ModelKind::from_u8(kind),
        Some(ModelKind::SpeechToText | ModelKind::TextToSpeech)
    )
}

/// How far back a speech kind's "now" speed may be found: `voice.live`'s
/// three minutes, so it can only ever say "speaking now".
const SPEAKING_NOW: Duration = Duration::minutes(3);

fn ts(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Fold `(at, value)` rows, oldest first, into a summary over the window
/// starting at `start` with buckets `width` wide.
fn summarise(
    rows: &[(String, Option<f64>)],
    start: DateTime<Utc>,
    span: Duration,
    width: Duration,
) -> Summary {
    let n = (span.num_seconds() / width.num_seconds().max(1)).max(1) as usize;
    let mut sums = vec![(0.0_f64, 0usize); n];
    let mut s = Summary {
        window_minutes: span.num_minutes().max(0) as usize,
        minutes: rows.len(),
        unknown: 0,
        latest: None,
        min: None,
        mean: None,
        max: None,
        sum: None,
        buckets: Vec::new(),
    };
    let (mut total, mut count) = (0.0, 0usize);
    for (at, v) in rows {
        let Some(v) = *v else {
            s.unknown += 1;
            continue;
        };
        total += v;
        count += 1;
        s.min = Some(s.min.map_or(v, |m: f64| m.min(v)));
        s.max = Some(s.max.map_or(v, |m: f64| m.max(v)));
        s.latest = Some((at.clone(), v));
        if let Ok(t) = NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S") {
            let i = ((t.and_utc() - start).num_seconds() / width.num_seconds().max(1)) as usize;
            if let Some(b) = sums.get_mut(i.min(n - 1)) {
                b.0 += v;
                b.1 += 1;
            }
        }
    }
    if count > 0 {
        s.mean = Some(total / count as f64);
        s.sum = Some(total);
    }
    s.buckets = sums
        .into_iter()
        .enumerate()
        .map(|(i, (sum, c))| Bucket {
            start: ts(start + width * i as i32),
            mean: (c > 0).then(|| sum / c as f64),
            sum: (c > 0).then_some(sum),
        })
        .collect();
    s
}

fn open(db: &Path) -> Result<Option<Connection>> {
    if !db.exists() {
        return Ok(None);
    }
    Ok(Some(Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?))
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |r| r.get(0),
    )?)
}

/// `m` over `w` (not `Now`; that is [`super::probe`]'s). `Ok(None)` when the
/// series does not exist yet, so the caller says so rather than showing an
/// empty window as a quiet one.
pub fn read(db: &Path, m: Measurement, w: Window, now: DateTime<Utc>) -> Result<Option<Windowed>> {
    let Some((span, width)) = w.span() else {
        anyhow::bail!("`now` is a probe, not a window");
    };
    let Some(conn) = open(db)? else {
        return Ok(None);
    };
    let start = now - span;
    let from = ts(start);
    if withheld_over_a_window(m) {
        return Ok(Some(Windowed::Withheld));
    }
    if m.how() != How::ByCategory {
        // A series from before S2a has no `reading_minute` (review of #660).
        if !has_table(&conn, "reading_minute")? {
            return Ok(Some(Windowed::OlderSampler));
        }
        // Every measurement, `Now` ones included, is recorded each minute
        // under its number (§7.3).
        let mut stmt = conn.prepare(
            "SELECT at, value FROM reading_minute
             WHERE measurement = ?1 AND at >= ?2 ORDER BY at",
        )?;
        let rows: Vec<(String, Option<f64>)> = stmt
            .query_map(params![m as u16, from], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        // No row at all while the sampler was writing minutes: the machine
        // does not have it, which `record_readings` writes as no row — but
        // only if that sampler knew the measurement; one it did not know is
        // an older sampler's, not this machine's lack (review of #660).
        if rows.is_empty() && sampled_in(&conn, &from)? {
            return Ok(Some(if known_to_sampler(&conn, m)? {
                Windowed::NotHere
            } else {
                Windowed::OlderSampler
            }));
        }
        return Ok(Some(Windowed::One(summarise(&rows, start, span, width))));
    }
    let table = if m == Measurement::CategoryOomKills {
        "category_reading_minute"
    } else {
        "category_minute"
    };
    if !has_table(&conn, table)? {
        return Ok(Some(Windowed::OlderSampler));
    }
    let mut out = Vec::new();
    let mut withheld = Vec::new();
    for cat in Category::ALL {
        if cat == Category::Voice {
            withheld.push(cat.label());
            continue;
        }
        let rows: Vec<(String, Option<f64>)> = if m == Measurement::CategoryOomKills {
            if !has_table(&conn, "category_reading_minute")? {
                Vec::new()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT at, value FROM category_reading_minute
                     WHERE measurement = ?1 AND category = ?2 AND at >= ?3 ORDER BY at",
                )?;
                let rows = stmt
                    .query_map(params![m as u16, cat as u8, from], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                rows
            }
        } else {
            // Named, never defaulted, as `latest` names them: a new
            // by-category measurement without a column here is refused.
            let column = match m {
                Measurement::CategoryMemory => "CAST(mem_bytes AS REAL)",
                Measurement::CategoryCpu => "cpu_pct",
                Measurement::CategoryTasks => "CAST(tasks AS REAL)",
                Measurement::CategoryGpuMemory => "gpu_mib * 1048576.0",
                _ => anyhow::bail!("{}: {}", m.name(), super::measure::NOT_WIRED),
            };
            let mut stmt = conn.prepare(&format!(
                "SELECT at, {column} FROM category_minute
                 WHERE category = ?1 AND at >= ?2 ORDER BY at"
            ))?;
            let rows = stmt
                .query_map(params![cat as u8, from], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        out.push((cat.label(), summarise(&rows, start, span, width)));
    }
    Ok(Some(Windowed::ByCategory {
        rows: out,
        withheld,
    }))
}

/// Whether the sampler that wrote this series listed `m` in its
/// `measurements` table: every id it knew, whether or not it read one.
fn known_to_sampler(conn: &Connection, m: Measurement) -> Result<bool> {
    if !has_table(conn, "measurements")? {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT count(*) > 0 FROM measurements WHERE id = ?1",
        params![m as u16],
        |r| r.get(0),
    )?)
}

/// Whether the sampler wrote any minute in the window — what tells "this
/// machine has no such thing" from "nothing was recorded".
fn sampled_in(conn: &Connection, from: &str) -> Result<bool> {
    if !has_table(conn, "system_minute")? {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT count(*) > 0 FROM system_minute WHERE at >= ?1",
        params![from],
        |r| r.get(0),
    )?)
}

/// One model server's row as the series holds it (`model_minute`, R6).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelNow {
    pub at: String,
    pub kind: &'static str,
    pub kind_id: u8,
    pub running: bool,
    /// `None` with `model_known` false is "could not tell"; with it true,
    /// "none loaded".
    pub model: Option<String>,
    pub model_known: bool,
    pub mem_bytes: Option<f64>,
    /// The last minute in the day that measured a speed, and it: tokens/s
    /// for chat and OCR, RTF for speech, seconds per picture. `None` for a
    /// kind with no speed column (embeddings). For a speech kind (R7) it is
    /// only the last three minutes', and its time is empty: it can say
    /// "speaking now", never when.
    pub speed: Option<(String, f64)>,
}

/// One model server over a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelWindow {
    pub kind: &'static str,
    pub kind_id: u8,
    /// The window's length in minutes.
    pub window_minutes: usize,
    /// Minutes the series has a row for, and of those, minutes it ran.
    pub minutes: usize,
    pub running: usize,
    /// Each model seen loaded in the window, with its minutes, oldest first —
    /// a switch shows as two entries.
    pub models: Vec<(String, usize)>,
    pub mem: Summary,
    /// Requests finished in the window; `None` when no minute could be read.
    pub requests: Option<f64>,
    /// The kind's speed (as [`ModelNow::speed`]) over its minutes with requests.
    pub speed: Summary,
}

/// Each kind's speed column, named and never defaulted: embeddings has
/// none, and a kind this build does not know reads none rather than a
/// neighbour's (review of #660).
pub(crate) fn speed_column(kind: u8) -> Option<&'static str> {
    use super::models::ModelKind;
    match ModelKind::from_u8(kind)? {
        ModelKind::Chat | ModelKind::Ocr => Some("gen_tps"),
        ModelKind::SpeechToText | ModelKind::TextToSpeech => Some("rtf"),
        ModelKind::Image => Some("job_secs"),
        ModelKind::Embeddings => None,
    }
}

/// A `model_minute` row as `models_now` reads it: kind, label, running,
/// model, whether the model is known, memory.
type NowRow = (u8, String, bool, Option<String>, bool, Option<f64>);

/// Each model server's newest row. Empty when the series has none (an older
/// sampler, or none yet).
pub fn models_now(db: &Path, now: DateTime<Utc>) -> Result<Option<Vec<ModelNow>>> {
    let Some(conn) = open(db)? else {
        return Ok(None);
    };
    if !has_table(&conn, "model_minute")? {
        return Ok(Some(Vec::new()));
    }
    let newest: Option<String> =
        conn.query_row("SELECT max(at) FROM model_minute", [], |r| r.get(0))?;
    let Some(newest) = newest else {
        return Ok(Some(Vec::new()));
    };
    let mut stmt = conn.prepare(
        // No join on `model_kinds`: the label is the closed enum's
        // (`kind_label`), and an inner join would only drop a row whose kind
        // the lookup lacks (review of #660).
        "SELECT m.kind, '', m.running, m.model, m.model_known, CAST(m.mem_bytes AS REAL)
         FROM model_minute m WHERE m.at = ?1 ORDER BY m.kind",
    )?;
    let rows: Vec<NowRow> = stmt
        .query_map(params![newest], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for (kind_id, label, running, model, model_known, mem_bytes) in rows {
        let speech = is_speech(kind_id);
        let since = ts(now
            - if speech {
                SPEAKING_NOW
            } else {
                Duration::hours(24)
            });
        let speed: Option<(String, f64)> = speed_column(kind_id).and_then(|col| {
            conn.query_row(
                &format!(
                    "SELECT at, {col} FROM model_minute
                     WHERE kind = ?1 AND at >= ?2 AND {col} IS NOT NULL
                     ORDER BY at DESC LIMIT 1"
                ),
                params![kind_id, since],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        });
        // R7: a speech kind's speed carries no time.
        let speed = speed.map(|(at, v)| (if speech { String::new() } else { at }, v));
        out.push(ModelNow {
            at: newest.clone(),
            kind: kind_label(kind_id, &label),
            kind_id,
            running,
            model,
            model_known,
            mem_bytes,
            speed,
        });
    }
    Ok(Some(out))
}

/// The closed kind labels, so a label is never text read from a file.
fn kind_label(id: u8, _stored: &str) -> &'static str {
    super::models::ModelKind::from_u8(id).map_or("unknown kind", |k| k.label())
}

/// Each model server over `w`, but the speech kinds, which are readable now
/// only (R7).
pub fn models_window(db: &Path, w: Window, now: DateTime<Utc>) -> Result<Option<Vec<ModelWindow>>> {
    let Some((span, width)) = w.span() else {
        anyhow::bail!("`now` is a probe, not a window");
    };
    let Some(conn) = open(db)? else {
        return Ok(None);
    };
    if !has_table(&conn, "model_minute")? {
        return Ok(Some(Vec::new()));
    }
    let start = now - span;
    let from = ts(start);
    let mut out = Vec::new();
    for kind in super::models::ModelKind::ALL {
        let id = kind as u8;
        if is_speech(id) {
            continue;
        }
        let col = speed_column(id).unwrap_or("NULL");
        let mut stmt = conn.prepare(&format!(
            "SELECT at, running, model, CAST(mem_bytes AS REAL), requests, {col}
             FROM model_minute WHERE kind = ?1 AND at >= ?2 ORDER BY at"
        ))?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            bool,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        )> = stmt
            .query_map(params![id, from], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            continue;
        }
        let mut models: Vec<(String, usize)> = Vec::new();
        for (_, _, m, ..) in &rows {
            if let Some(m) = m {
                match models.last_mut() {
                    Some((last, n)) if last == m => *n += 1,
                    _ => models.push((m.clone(), 1)),
                }
            }
        }
        let mem: Vec<(String, Option<f64>)> = rows.iter().map(|r| (r.0.clone(), r.3)).collect();
        let speed: Vec<(String, Option<f64>)> = rows.iter().map(|r| (r.0.clone(), r.5)).collect();
        let read: Vec<f64> = rows.iter().filter_map(|r| r.4).collect();
        out.push(ModelWindow {
            kind: kind.label(),
            kind_id: id,
            window_minutes: span.num_minutes().max(0) as usize,
            minutes: rows.len(),
            running: rows.iter().filter(|r| r.1).count(),
            models,
            mem: summarise(&mem, start, span, width),
            requests: (!read.is_empty()).then(|| read.iter().sum()),
            speed: summarise(&speed, start, span, width),
        });
    }
    Ok(Some(out))
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
