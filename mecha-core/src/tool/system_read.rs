//! `system_read`: the machine's state, for the owner's questions about it
//! (`docs/SYSTEM-STATE-DESIGN.md` §5, rulings R3, R3b and R7; step S5).
//!
//! "Why is chat slow right now?", "how full is the disk?", "which model is
//! loaded?" become factual answers from the series mecha's own sampler
//! writes. The tool exists to answer the owner; it is not a way for a run to
//! reason about load (§4, rule 8).
//!
//! ## A closed schema
//!
//! A measurement by name or group (`memory.available`, `gpu`), `models` for
//! the model servers, or nothing for an overview; a window from a closed set;
//! a breakdown by kind of work or none. No SQL, no path. Readings reach a
//! model only as this tool's result, never the system prompt (§4, rule 4).
//!
//! ## Voice is readable now, never over a window (R7)
//!
//! A window over anything that moves with a call — `voice.live`, the voice
//! category's load, the speech models' rows — is, in effect, a log of when
//! calls happened, and a result lands in a transcript. The owner ruled
//! (2026-10-12) that the tool may say whether a call is live *now* and never
//! give that history, so call timing (an incognito call's included) never
//! reaches a chat log through it. Each is refused by name rather than
//! dropped silently.
//!
//! ## Capabilities (§5, R3b)
//!
//! `private_data`: a category's series is a record of when that kind of
//! work happened. Not `untrusted_input`, and never `from_outside`: our own
//! sampler wrote every value and every label is a closed enum. No egress.

use std::fmt::Write as _;
use std::path::PathBuf;

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::system::measure::{How, Measurement, Unit};
use crate::system::models::ModelKind;
use crate::system::window::{self, Bucket, ModelNow, ModelWindow, Summary, Window, Windowed};
use crate::system::{self, Probed, Reader, Reading};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Value};

/// What a no-argument call shows: the questions the owner asks most.
const OVERVIEW: &[Measurement] = &[
    Measurement::MemoryAvailable,
    Measurement::MemoryTotal,
    Measurement::DiskRootUsed,
    Measurement::DiskRootTotal,
    Measurement::CpuBusy,
    Measurement::GpuUtilization,
    Measurement::GpuTemperature,
    Measurement::GpuPower,
    Measurement::PressureMemoryAvg10,
];

/// A model row older than this is said to be stale.
const STALE_MINUTES: i64 = 5;

/// What one `now` read may spend on the machine's commands, together.
const READ_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// The machine's state, read from the series and the machine.
pub struct SystemRead {
    db: PathBuf,
    /// The owner's zone, for the times a window shows (`[agent] timezone`).
    tz: Option<chrono_tz::Tz>,
    /// What a `Now` read needs from config (`Reader::configured`).
    config: Option<crate::config::Config>,
}

impl SystemRead {
    pub fn new(db: PathBuf, config: Option<crate::config::Config>) -> Self {
        SystemRead {
            tz: config.as_ref().and_then(|c| c.agent.timezone()),
            db,
            config,
        }
    }
}

#[async_trait]
impl Tool for SystemRead {
    fn name(&self) -> &str {
        "system_read"
    }

    fn description(&self) -> &str {
        "This machine's state: memory, disk, CPU, GPU, memory pressure, network, the model \
         servers (which model is loaded, its memory and speed) and mecha's own work (runs, \
         jobs, triggers). With no arguments, an overview right now. `measurement` takes a \
         name (`disk.root.used`), a group (`memory`, `gpu`, `pressure`, `network`, \
         `category`) or `models`; `window` is now, 1h, 6h, 24h or 7d; `by` = `category` \
         splits load by kind of work. Use it to answer questions about the machine, never \
         guess them."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "measurement": {
                    "type": "string",
                    "description": "A measurement name or group, `models`, or leave out for an overview."
                },
                "window": {"type": "string", "enum": ["now", "1h", "6h", "24h", "7d"]},
                "by": {"type": "string", "enum": ["none", "category"]}
            }
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    /// R3b: private, and nothing else (module doc).
    fn capabilities(&self) -> Capabilities {
        Capabilities::default().private()
    }

    fn effect(&self) -> Option<crate::tool::effect::Effect> {
        Some(crate::tool::effect::Effect::Read)
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let window = match input.get("window").and_then(Value::as_str) {
            None => Window::Now,
            Some(w) => match Window::parse(w) {
                Some(w) => w,
                None => return Ok(ToolOutput::err("`window` is one of now, 1h, 6h, 24h, 7d.")),
            },
        };
        let by_category = match input.get("by").and_then(Value::as_str) {
            None | Some("none") => false,
            Some("category") => true,
            Some(_) => return Ok(ToolOutput::err("`by` is none or category.")),
        };
        let query = input
            .get("measurement")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let (db, tz, config) = (self.db.clone(), self.tz, self.config.clone());
        let out = tokio::task::spawn_blocking(move || {
            answer(
                &db,
                tz,
                config.as_ref(),
                query.as_deref(),
                window,
                by_category,
                Utc::now(),
            )
        })
        .await??;
        Ok(out)
    }
}

/// The whole answer, as text. `Err` only for I/O the model cannot route
/// around; a wrong name is an `is_error` result listing the right ones.
pub fn answer(
    db: &std::path::Path,
    tz: Option<chrono_tz::Tz>,
    config: Option<&crate::config::Config>,
    query: Option<&str>,
    window: Window,
    by_category: bool,
    now: DateTime<Utc>,
) -> Result<ToolOutput> {
    let mut out = String::new();
    // Said under the heading it qualifies, not above it (review of #660).
    let mut note: Option<String> = None;
    let wanted: Vec<Measurement> = match query {
        Some("models") => return models(db, tz, window, now, by_category),
        None => {
            let mut list = OVERVIEW.to_vec();
            if by_category {
                list.push(Measurement::CategoryMemory);
            }
            list
        }
        Some(q) => {
            let found = Measurement::matching(q);
            if found.is_empty() {
                return Ok(ToolOutput::err(format!(
                    "No measurement is named or grouped {q:?}. Groups: memory, cpu, pressure, \
                     disk, gpu, network, system, services, runs, tasks, holds, permits, jobs, \
                     voice, triggers, messages, category; or `models`. Names: {}.",
                    Measurement::ALL
                        .iter()
                        .map(|m| m.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            found
        }
    };
    // `by: category` asks for the by-category measurements, and a single
    // level measurement has no breakdown: say so instead of ignoring it.
    let wanted: Vec<Measurement> = if by_category && query.is_some() {
        let split: Vec<Measurement> = wanted
            .iter()
            .copied()
            .filter(|m| m.how() == How::ByCategory)
            .collect();
        if split.is_empty() {
            note = Some(format!(
                "(`by: category` applies to the category.* measurements; showing {} as a whole)",
                query.unwrap_or_default()
            ));
            wanted
        } else {
            split
        }
    } else {
        wanted
    };

    if window == Window::Now {
        // One budget for the whole read, as the sampler has: a fork-heavy
        // group must not cost a bound per command (review of #660).
        let reader = Reader::until(std::time::Instant::now() + READ_BUDGET).configured(config);
        let _ = writeln!(out, "Now ({}):", clock(now, tz));
        if let Some(n) = &note {
            let _ = writeln!(out, "{n}");
        }
        for m in wanted {
            match system::probe(&reader, db, m, now) {
                Probed::One(r) => {
                    let _ = writeln!(out, "- {}: {}", m.name(), shown(m.unit(), &r));
                }
                Probed::ByCategory(cats) => {
                    let _ = writeln!(out, "- {}, by kind of work:", m.name());
                    for (label, r) in cats {
                        let _ = writeln!(out, "    {label}: {}", shown(m.unit(), &r));
                    }
                }
            }
        }
        if query.is_none() {
            out.push('\n');
            out.push_str(&models(db, tz, Window::Now, now, false)?.content);
        }
        return Ok(ToolOutput::ok(out));
    }

    let _ = writeln!(
        out,
        "Over the last {} (times {}):",
        window.name(),
        zone_name(tz)
    );
    if let Some(n) = &note {
        let _ = writeln!(out, "{n}");
    }
    let mut refused = Vec::new();
    for m in wanted {
        // A series locked by the minute's sample, or otherwise unreadable,
        // is said for this measurement, as a `now` read says `Unread` —
        // never a failed tool call (review of #660).
        let read = match window::read(db, m, window, now) {
            Ok(read) => read,
            Err(e) => {
                let _ = writeln!(
                    out,
                    "- {}: unknown — the series could not be read: {e:#}",
                    m.name()
                );
                continue;
            }
        };
        match read {
            None => {
                out.push_str(
                    "The series does not exist yet: is mecha-system-sample.timer running?\n",
                );
                break;
            }
            Some(Windowed::One(s)) => {
                let _ = writeln!(out, "- {}: {}", m.name(), summary(m, &s, tz));
            }
            Some(Windowed::OlderSampler) => {
                let _ = writeln!(
                    out,
                    "- {}: the series was written by an older sampler; it is recorded from the \
                     next sample",
                    m.name()
                );
            }
            Some(Windowed::NotHere) => {
                let _ = writeln!(out, "- {}: not on this machine", m.name());
            }
            // R7 is the read's (`window::withheld_over_a_window`); this only
            // says it.
            Some(Windowed::Withheld) => refused.push(m.name().to_string()),
            Some(Windowed::ByCategory { rows, withheld }) => {
                let _ = writeln!(out, "- {}, by kind of work:", m.name());
                for (label, s) in rows {
                    let _ = writeln!(out, "    {label}: {}", summary(m, &s, tz));
                }
                for label in withheld {
                    refused.push(format!("{} for {label}", m.name()));
                }
            }
        }
    }
    if !refused.is_empty() {
        let _ = writeln!(
            out,
            "Not shown: {} — voice is readable now only, never over a window, so when calls \
             happened never reaches a transcript (the owner's ruling).",
            refused.join(", ")
        );
    }
    Ok(ToolOutput::ok(out))
}

/// The model servers: now, or over a window without the speech kinds.
fn models(
    db: &std::path::Path,
    tz: Option<chrono_tz::Tz>,
    window: Window,
    now: DateTime<Utc>,
    by_category: bool,
) -> Result<ToolOutput> {
    let mut out = String::new();
    if by_category {
        out.push_str("(`by: category` does not apply to `models`: each server is its own row)\n");
    }
    if window == Window::Now {
        // A speech kind's "now" speed is the last three minutes' and carries
        // no time: `models_now` enforces it (R7, review of #660).
        let Some(rows) = window::models_now(db, now)? else {
            return Ok(ToolOutput::ok(
                "Model servers: the series does not exist yet.",
            ));
        };
        if rows.is_empty() {
            return Ok(ToolOutput::ok(
                "Model servers: no reading yet (the sampler records them from #658 on).",
            ));
        }
        let _ = writeln!(out, "Model servers (as of {}):", stamp(&rows[0].at, tz));
        // A dead sampler leaves a week-old row reading like a fresh one.
        if let Ok(t) = NaiveDateTime::parse_from_str(&rows[0].at, "%Y-%m-%d %H:%M:%S") {
            let age = (now - t.and_utc()).num_minutes();
            if age > STALE_MINUTES {
                let _ = writeln!(
                    out,
                    "(the newest reading is {age} minutes old: is mecha-system-sample.timer running?)"
                );
            }
        }
        for r in &rows {
            let _ = writeln!(out, "- {}", model_now(r, tz));
        }
        return Ok(ToolOutput::ok(out));
    }
    let rows = match window::models_window(db, window, now)? {
        None => {
            return Ok(ToolOutput::ok(
                "Model servers: the series does not exist yet.",
            ))
        }
        // Kept apart from a quiet window (review of #660).
        Some(window::ModelRows::OlderSampler) => {
            return Ok(ToolOutput::ok(
                "Model servers: the series was written by a sampler older than the model \
                 rows; they are recorded from the next sample.",
            ))
        }
        Some(window::ModelRows::Rows { servers }) => servers,
    };
    let _ = writeln!(
        out,
        "Model servers over the last {} (times {}):",
        window.name(),
        zone_name(tz)
    );
    if rows.is_empty() {
        out.push_str("No model server recorded a minute in this window.\n");
    }
    for r in &rows {
        let _ = writeln!(out, "- {}", model_window(r, tz));
    }
    out.push_str(
        "Not shown: speech to text and text to speech — their history moves with calls, and \
         voice is readable now only (the owner's ruling); ask for `models` now for their state.\n",
    );
    Ok(ToolOutput::ok(out))
}

const GIB: f64 = 1073741824.0;

fn model_now(r: &ModelNow, tz: Option<chrono_tz::Tz>) -> String {
    let state = if r.running { "running" } else { "stopped" };
    let model = match (&r.model, r.model_known, r.running) {
        (Some(m), _, _) => m.clone(),
        (None, true, true) => "no model loaded".into(),
        (None, _, false) => "—".into(),
        (None, false, true) => "model unknown".into(),
    };
    let mem = r.mem_bytes.map_or("memory unknown".to_string(), |b| {
        format!("{:.1} GiB", b / GIB)
    });
    let speed = match (&r.speed, window::is_speech(r.kind_id)) {
        // Now only, and no time: a time would say when the call was (R7).
        (Some((_, v)), true) => format!("; speaking now, {}", speed_text(r.kind_id, *v)),
        (None, true) => "; not speaking now".into(),
        (Some((at, v)), false) => format!(
            "; latest speed {} at {}",
            speed_text(r.kind_id, *v),
            stamp(at, tz)
        ),
        // A kind with no speed column (embeddings, or one this build does
        // not know) was never read for one: say nothing (review of #660).
        (None, false) if window::speed_column(r.kind_id).is_none() => String::new(),
        (None, false) => "; no measured speed in the last day".into(),
    };
    format!("{}: {state}, {model}, {mem}{speed}", r.kind)
}

fn model_window(r: &ModelWindow, tz: Option<chrono_tz::Tz>) -> String {
    let models = if r.models.is_empty() {
        "no model loaded".to_string()
    } else {
        r.models
            .iter()
            .map(|(m, n)| format!("{m} ({n} min)"))
            .collect::<Vec<_>>()
            .join(" → ")
    };
    let mem = match (r.mem.mean, r.mem.max) {
        (Some(mean), Some(max)) => {
            format!("memory mean {:.1}, max {:.1} GiB", mean / GIB, max / GIB)
        }
        _ => "memory unknown".into(),
    };
    // A total over only the minutes whose count was read says so: an
    // unreadable minute is not a quiet one (review of #660).
    let requests = match r.requests {
        None => "requests unknown".to_string(),
        Some(n) if r.requests_minutes < r.minutes => format!(
            "{n:.0} requests in the {} minutes read ({} unknown)",
            r.requests_minutes,
            r.minutes - r.requests_minutes
        ),
        Some(n) => format!("{n:.0} requests"),
    };
    let speed = match (r.speed.mean, &r.speed.latest) {
        (Some(mean), Some((at, v))) => format!(
            "; speed mean {}, latest {} at {}",
            speed_text(r.kind_id, mean),
            speed_text(r.kind_id, *v),
            stamp(at, tz)
        ),
        _ => String::new(),
    };
    format!(
        "{}: ran {} of the window's {} minutes (recorded for {}); {models}; {mem}; {requests}{speed}",
        r.kind, r.running, r.window_minutes, r.minutes
    )
}

fn speed_text(kind: u8, v: f64) -> String {
    match ModelKind::from_u8(kind) {
        Some(ModelKind::Chat | ModelKind::Ocr) => format!("{v:.0} tokens/s"),
        Some(ModelKind::SpeechToText | ModelKind::TextToSpeech) => format!("RTF {v:.2}"),
        Some(ModelKind::Image) => format!("{v:.0} s per picture"),
        Some(ModelKind::Embeddings) | None => format!("{v}"),
    }
}

/// A window's summary on one line, with its buckets after.
fn summary(m: Measurement, s: &Summary, tz: Option<chrono_tz::Tz>) -> String {
    let unit = m.unit();
    if s.minutes == 0 {
        return "no minutes recorded in this window".into();
    }
    let known = s.minutes - s.unknown;
    if known == 0 {
        return format!("unknown in all {} minutes", s.minutes);
    }
    let mut text = match unit {
        Unit::Flag => format!(
            "on in {:.0}% of minutes",
            s.mean.unwrap_or_default() * 100.0
        ),
        Unit::Label(labels) => match &s.latest {
            Some((at, v)) => format!(
                "latest {} at {}",
                labels.get(*v as usize).copied().unwrap_or("unknown"),
                stamp(at, tz)
            ),
            None => "unknown".into(),
        },
        _ if counts_per_minute(m) => {
            format!("{} in all", value_text(unit, s.sum.unwrap_or_default()))
        }
        _ => {
            let (lat_at, lat) = s.latest.clone().unwrap_or_default();
            format!(
                "latest {} at {}; min {}, mean {}, max {}",
                value_text(unit, lat),
                stamp(&lat_at, tz),
                value_text(unit, s.min.unwrap_or_default()),
                value_text(unit, s.mean.unwrap_or_default()),
                value_text(unit, s.max.unwrap_or_default()),
            )
        }
    };
    // Against the window's own length: a sampler that was down leaves
    // minutes with no row, and they are said (review of #660).
    let _ = write!(
        text,
        " ({known} of the window's {} minutes known",
        s.window_minutes
    );
    // Three counts that add up to the window: known, recorded but unknown
    // (the source would not answer — `Reading::Unread`), and not recorded
    // (the sampler was not running). Unknown is never folded into either
    // neighbour (review of #660).
    if s.unknown > 0 {
        let _ = write!(text, "; {} unknown", s.unknown);
    }
    let missing = s.window_minutes.saturating_sub(s.minutes);
    if missing > 0 {
        let _ = write!(text, "; {missing} not recorded");
    }
    text.push(')');
    if !matches!(unit, Unit::Label(_)) {
        let _ = write!(text, "\n      by stretch: {}", buckets(m, &s.buckets, tz));
    }
    text
}

/// Measurements recorded as a count per minute, which a window totals.
fn counts_per_minute(m: Measurement) -> bool {
    matches!(
        m,
        Measurement::MemoryOomKills
            | Measurement::MemoryOomKillsGlobal
            | Measurement::MemoryOomKillsCapped
            | Measurement::CategoryOomKills
    )
}

fn buckets(m: Measurement, b: &[Bucket], tz: Option<chrono_tz::Tz>) -> String {
    b.iter()
        .map(|b| {
            // A per-minute count's stretch is its total, as the headline is;
            // a mean would dilute a burst to nothing (review of #660).
            let v = match (b.mean, b.sum) {
                (None, _) => "—".to_string(),
                (_, Some(sum)) if counts_per_minute(m) => value_text(m.unit(), sum),
                (Some(v), _) if m.unit() == Unit::Flag => format!("{:.0}%", v * 100.0),
                (Some(v), _) => value_text(m.unit(), v),
            };
            format!("{} {v}", stamp(&b.start, tz))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A reading for a model, in its unit.
fn shown(unit: Unit, r: &Reading) -> String {
    match r {
        Reading::Observed { value } => value_text(unit, *value),
        Reading::Unread { why } => format!("unknown — {why}"),
        Reading::NotHere => "not on this machine".into(),
    }
}

/// One value in its unit — what `mecha system probe` prints, too.
pub fn value_text(unit: Unit, v: f64) -> String {
    match unit {
        Unit::Bytes => format!("{:.2} GiB", v / GIB),
        Unit::BytesPerSec if v >= 1048576.0 => format!("{:.1} MiB/s", v / 1048576.0),
        Unit::BytesPerSec => format!("{:.1} KiB/s", v / 1024.0),
        Unit::Percent => format!("{v:.1}%"),
        Unit::Count | Unit::Load => {
            if v.fract() == 0.0 {
                format!("{v:.0}")
            } else {
                format!("{v:.1}")
            }
        }
        Unit::Celsius => format!("{v:.1} °C"),
        Unit::Watts => format!("{v:.1} W"),
        Unit::Seconds => format!("{:.1} days", v / 86400.0),
        Unit::Dbm => format!("{v:.0} dBm"),
        Unit::Mbits => format!("{v:.1} Mbit/s"),
        Unit::Flag => (if v != 0.0 { "yes" } else { "no" }).to_string(),
        Unit::Label(labels) => labels
            .get(v as usize)
            .map_or_else(|| format!("unknown label {v}"), |l| l.to_string()),
    }
}

/// A series time (`YYYY-MM-DD HH:MM:SS`, UTC) as the owner's clock reads it.
fn stamp(at: &str, tz: Option<chrono_tz::Tz>) -> String {
    match NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S") {
        Ok(t) => clock(t.and_utc(), tz),
        Err(_) => at.to_string(),
    }
}

fn clock(t: DateTime<Utc>, tz: Option<chrono_tz::Tz>) -> String {
    match tz {
        Some(tz) => t.with_timezone(&tz).format("%a %H:%M").to_string(),
        None => t.format("%a %H:%M UTC").to_string(),
    }
}

fn zone_name(tz: Option<chrono_tz::Tz>) -> String {
    tz.map_or("UTC".to_string(), |t| t.name().to_string())
}

#[cfg(test)]
#[path = "system_read_tests.rs"]
mod tests;
