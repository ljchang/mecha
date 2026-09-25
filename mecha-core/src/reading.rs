//! A sensored charter line, read against the store it watches.
//!
//! `docs/GOAL-SYSTEM-DESIGN.md` §11.1 is the design; this is its *readings*
//! phase. A [`crate::charter::Sensor`] names an observable a store already
//! holds and a setpoint the owner wrote; a [`LineReading`] is what that
//! observable read at one moment, against that setpoint. Every reading is a
//! pure function of the stores and the charter — no model, no clock inside
//! (`now` is injected) — so it replays over the whole corpus, which is the
//! property the no-store rule (§15) protects.
//!
//! ## What a reading may and may not do
//!
//! - **It never reaches a prompt** (§11.1, containment 2). The line's text
//!   already rides in the cached prefix; the sensor's value is harness-only,
//!   because every exposed number invites the model to reason about it. This
//!   module has no dependency on the agent, the prompt builders or
//!   `crate::message`, and the absence is the enforcement — the same shape
//!   `charter.rs` uses for taint.
//! - **It never feeds a [`crate::candidate::Metric`]** (containment 1). A
//!   line with a number is a metric, and harness rumination auto-accepts
//!   config changes measured against metrics; a reading feeds appraisal,
//!   the doctor and the owner's own surfaces, and the `Metric` enum stays
//!   closed at the type.
//! - **Unknown is never zero.** A store that could not be read is
//!   [`Reading::Unread`]; a store this reader does not scan is
//!   [`Reading::Deferred`]; nothing waiting is [`Reading::Nothing`]; too few
//!   runs for a share is [`Reading::Sparse`]; and only [`Reading::Observed`]
//!   carries a value. Five facts, kept apart, because a finding that fires
//!   on an absence is the failure `backlog.rs` and `guilt.rs` both state at
//!   length.
//!
//! ## Line-specific guilt, and why the term is asymptotic
//!
//! §11.1's promise is that "harmed another" becomes "a recorded commitment
//! aged past *this* line's setpoint", one reading per sensored line instead
//! of the single saturated number `guilt.rs` once folded (retired to a
//! readout in 1f; per-commitment guilt there applies [`excess`] to each
//! item's age against its patience). The magnitude here —
//! [`Reading::Observed`]'s `excess` — is how far past the setpoint the
//! observable sits, in `[0, 1)`: zero within the setpoint, half of maximal
//! at twice it, approaching but never reaching one. Asymptotic because a
//! term that reaches `1.0` stops varying, and a corpus of a constant carries
//! nothing — the lesson the retired scalar's age term learned twice.
//! Containment 5 is the other half of that argument — a setpoint of one hour
//! where the owner meant one day would sit past its setpoint on every run —
//! so a zero setpoint is refused at the parser (nothing could ever be within
//! it), and the doctor reports a line whose reading has been past its
//! setpoint on every one of the last [`SATURATED_AFTER_RUNS`] runs, rather
//! than the harness quietly living with a constant again.
//!
//! ## Where a reading is taken
//!
//! - **On the homeostat, at the start of every run** — from the backlog the
//!   run *inherited*, the same survey per-commitment guilt is read from
//!   (`Homeostat::commitments`), so the record says what waited on the
//!   owner as the run began. The corpus kind (`intervention_rate`) is
//!   [`Reading::Deferred`] there: reading it is a scan of the session store,
//!   and `guilt.rs`'s rule for the graph applies — fine once a night, too
//!   expensive in the path of every run.
//! - **On the owner's surfaces** (`mecha charter`, the TUI's `/charter`, the
//!   web settings page) through [`read_charter`], which does scan the corpus
//!   when a line asks for it — containment 5's first guard is the editor
//!   showing each sensor's current reading beside its line.
//! - **In the doctor**, which reads its own walkers against the owner's
//!   setpoints rather than fixed constants, and reads the recorded readings
//!   back for saturation.
//!
//! The formula is argued, not measured: there is no corpus yet linking
//! `excess` to a real missed expectation, the same discipline every sensor
//! in this arc shipped under.
//!
//! ## Per item, per run, and withdrawn when saturated (S5)
//!
//! A level is one number, and one stale item pins it: an age kind's level
//! is the oldest item's age, so the live store's one sensored line read
//! past its setpoint on 126 of 126 runs whatever else the queue did
//! (`docs/APPRAISAL-WIRING-DESIGN.md` S5, inventory §1). Three additions
//! keep the reading informative under that, none replacing the level:
//!
//! - **[`Items`]**, beside the level on every reading taken from a store:
//!   how many items wait, how many are past the setpoint, the oldest one's
//!   age, and the stale ones' ids. Read from the same survey the level is
//!   (`backlog::Backlog::survey`), so the two never describe different
//!   states of one queue.
//! - **[`LineReading::delta`]**, set when a run finishes: the ids the run's
//!   window added to and cleared from the line's store. A level difference
//!   reads a run that staged one draft while another was sent as nothing.
//! - **Withdrawal.** A line that reads past its setpoint now and did on
//!   each of the last [`SATURATED_AFTER_RUNS`] informative recorded runs
//!   ([`saturated`], the one definition the doctor's finding uses too) is
//!   marked [`LineReading::withdrawn`] and left out of what in-run
//!   consumers read (`Homeostat::in_run_readings`) — a consumer that fires
//!   identically on every run is a prompt suffix with extra steps. It stays
//!   on the record in full, so the streak and the one doctor finding go on
//!   until the line reads within its setpoint.
//!
//! The run reads its history by streaming the session store newest first
//! ([`recorded_readings`]) and stops as soon as each over-setpoint line is
//! decided — nothing at all when no line is over — so the corpus kind's
//! full scan stays the surfaces' cost, not every run's.

use crate::backlog::{Backlog, Depth, Flow, Inventory, Waiter};
use crate::charter::{Charter, CharterLine, SensorKind, Setpoint, Unit};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How many consecutive recorded runs a line may read past its setpoint
/// before the doctor calls the sensor saturated (§11.1, containment 5) and
/// a run withholds it from its in-run consumers (S5).
///
/// Argued, not measured: ten runs is a day or two of ordinary use, long
/// enough that a genuinely overdue draft has been nagged about by the
/// doctor's own stuck-draft finding, and short enough that a setpoint set
/// in the wrong unit is caught in the first session that notices the
/// charter page. Only informative readings count toward the streak — an
/// [`Reading::Unread`] or [`Reading::Deferred`] row says nothing either way.
pub const SATURATED_AFTER_RUNS: usize = 10;

/// The most recorded runs [`saturated`] reads back, newest first: room for
/// [`SATURATED_AFTER_RUNS`] informative readings among twice as many rows
/// that said nothing (an unreadable store, a row with no reading of the
/// line). Argued, not measured. Past it an undecided line is not
/// saturated — the direction that leaves it in front of the run, as before
/// withdrawal existed — and a run never pays for more than this many rows.
pub const SATURATION_ROWS_MAX: usize = 3 * SATURATED_AFTER_RUNS;

/// The observable's value, in the kind's unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observed {
    /// How long the oldest waiting item has waited.
    Seconds(u64),
    /// How many items wait.
    Count(u64),
    /// A share in `0.0..=1.0`.
    Rate(f64),
}

impl Observed {
    /// The value as a number in the kind's unit, for the comparison.
    fn as_f64(self) -> f64 {
        match self {
            Observed::Seconds(s) => s as f64,
            Observed::Count(n) => n as f64,
            Observed::Rate(r) => r,
        }
    }
}

/// What a sensored line read.
///
/// **A closed enum written to an append-only store is a wire format**: this
/// lands on [`crate::homeostat::Homeostat`] and rides in every session file
/// from now on, so a variant added later must degrade on load rather than
/// fail the record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Reading {
    /// The store the kind reads from could not be read. Unknown, never zero.
    Unread,
    /// This reader does not scan the store the kind reads from — today the
    /// run corpus, in the path of a run. The surfaces and the doctor do.
    Deferred,
    /// Nothing is waiting: the observable has no item, and the line is met
    /// by construction. Distinct from a count of zero, which is
    /// [`Reading::Observed`] — "no draft is old" and "zero drafts wait" are
    /// the same store state read by two different kinds.
    Nothing,
    /// The corpus kind over fewer runs than a share can be read from
    /// (`doctor::RUNS_MIN`): a share of three runs is noise, and the doctor
    /// refuses one, so the surface the owner judges a setpoint on must not
    /// print one either (found on review). Says nothing either way.
    Sparse { runs: usize },
    /// A value, against the setpoint.
    Observed {
        value: Observed,
        /// Past the setpoint.
        over: bool,
        /// How far past, in `[0, 1)`: zero within the setpoint, half of
        /// maximal at twice it, asymptotic above — see the module doc.
        excess: f32,
    },
}

impl Reading {
    /// Past the setpoint, where that is known. `None` for a reading that
    /// says nothing either way — the doctor's saturation streak skips these
    /// rather than counting them on either side.
    pub fn over(&self) -> Option<bool> {
        match self {
            Reading::Unread | Reading::Deferred | Reading::Sparse { .. } => None,
            Reading::Nothing => Some(false),
            Reading::Observed { over, .. } => Some(*over),
        }
    }

    /// The line-specific guilt term: `excess` where a value was observed,
    /// zero where nothing waits, `None` where nothing is known.
    pub fn excess(&self) -> Option<f32> {
        match self {
            Reading::Unread | Reading::Deferred | Reading::Sparse { .. } => None,
            Reading::Nothing => Some(0.0),
            Reading::Observed { excess, .. } => Some(*excess),
        }
    }
}

/// How many of a line's items [`Items::stale`] names — enough for a doctor
/// finding to point at what to act on, few enough that a queue of hundreds
/// does not grow every run record with it.
pub const ITEMS_NAMED: usize = 5;

/// A line's reading **per item**, beside the level (`docs/APPRAISAL-WIRING-
/// DESIGN.md` S5): how many items the line's store holds, how many of them
/// are past the setpoint, and the oldest one's age.
///
/// **Why the level alone is not enough.** An age kind's level is the oldest
/// item's age, so one stale draft pins the line past its setpoint on every
/// run whatever else the queue does — the constant the live store showed
/// (126 of 126 runs over, inventory §1). The per-item form keeps varying
/// under that: fresh drafts added and sent move `waiting`, and only a
/// second item going stale moves `over`. A consumer reads this, never the
/// level (here §1, decision 3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Items {
    /// Items the line's store holds.
    pub waiting: u64,
    /// Of those, how many are past the setpoint: for an age kind, the items
    /// whose own age exceeds it; for a count kind, how many the queue holds
    /// beyond the count.
    pub over: u64,
    /// Items whose stamp would not parse, so whether they are past the
    /// setpoint is unknown — counted apart, never as within it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown: u64,
    /// The oldest item's age, in seconds. `None` when nothing waits or no
    /// stamp parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_secs: Option<u64>,
    /// The items past the setpoint, oldest first, at most [`ITEMS_NAMED`],
    /// by the id their store knows them by — so the doctor's saturation
    /// finding can name what to act on. For a count kind these are the
    /// oldest items: the ones the queue would shed first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<String>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One sensored line, read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineReading {
    /// The charter line's id — the same id a [`crate::goal::GoalRef::Charter`]
    /// names, so a record joins back to the line without the charter.
    pub line: String,
    pub kind: SensorKind,
    /// The setpoint as the owner spelled it, kept beside the reading so a
    /// record from a charter since edited still says what it was read
    /// against.
    pub setpoint: String,
    /// The level: one value against the setpoint. Kept, because the doctor's
    /// saturation streak and the owner's surfaces read it; never what a
    /// consumer decides on.
    pub reading: Reading,
    /// The same moment read per item. `None` where the kind has no items
    /// (`intervention_rate`), the store could not be read, the reader was
    /// handed no items, or the row predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Items>,
    /// What the run did to the line's store, id by id — set when the run
    /// finishes, from the same two reads as `Homeostat::backlog_delta`.
    /// `None` on a surface's reading, on a kind with no items, where either
    /// read failed, and on a row from before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<crate::backlog::Flow>,
    /// Withheld from the run's in-run consumers (`Decision::assess`)
    /// because the line had read past its setpoint on each of the last
    /// [`SATURATED_AFTER_RUNS`] informative recorded runs and still did —
    /// a line that fires identically on every run is a prompt suffix with
    /// extra steps. The reading is still recorded in full, so the streak
    /// the doctor reports goes on counting.
    #[serde(default, skip_serializing_if = "is_false")]
    pub withdrawn: bool,
}

impl LineReading {
    /// One line of prose for a surface: `3d 4h, past the 24h setpoint`,
    /// `nothing waiting`, `store unreadable`, `not read here`. Never a
    /// prompt — every caller is a surface the owner reads.
    pub fn summary(&self) -> String {
        let mut out = self.level_summary();
        if let Some(items) = &self.items {
            if items.waiting > 0 {
                out.push_str(&format!(
                    "; {} of {} item{} past it",
                    items.over,
                    items.waiting,
                    if items.waiting == 1 { "" } else { "s" }
                ));
                if items.unknown > 0 {
                    out.push_str(&format!(", {} undated", items.unknown));
                }
            }
        }
        if let Some(delta) = &self.delta {
            out.push_str(&format!(
                "; this run added {} and cleared {}",
                delta.added, delta.cleared
            ));
        }
        if self.withdrawn {
            out.push_str(&format!(
                "; withdrawn from runs — past its setpoint on each of the last {SATURATED_AFTER_RUNS}"
            ));
        }
        out
    }

    fn level_summary(&self) -> String {
        match &self.reading {
            Reading::Unread => "store unreadable".to_string(),
            Reading::Deferred => "not read in a run; `mecha charter` reads it".to_string(),
            // The same state reads differently by store: the corpus kind's
            // "nothing" is no run in the window, not an empty queue.
            Reading::Nothing => match self.kind {
                SensorKind::InterventionRate => "no runs recorded yet".to_string(),
                _ => "nothing waiting".to_string(),
            },
            Reading::Sparse { runs } => format!(
                "only {runs} run{} recorded, under the {}-run floor a share needs",
                if *runs == 1 { "" } else { "s" },
                crate::doctor::RUNS_MIN
            ),
            Reading::Observed { value, over, .. } => {
                let value = match value {
                    Observed::Seconds(s) => render_secs(*s),
                    Observed::Count(n) => format!("{n} waiting"),
                    Observed::Rate(r) => format!("{:.0}% of recent runs", r * 100.0),
                };
                if *over {
                    format!("{value}, past the {} setpoint", self.setpoint)
                } else {
                    format!("{value}, within the {} setpoint", self.setpoint)
                }
            }
        }
    }

    /// The reading plus its prose, for a JSON surface — `{state, value?,
    /// over?, excess?, summary}`, without the line, kind and setpoint the
    /// surface already shows beside it. One shape for `mecha charter
    /// --json` and the web settings endpoint, so the two cannot drift.
    pub fn json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(&self.reading).unwrap_or(serde_json::Value::Null);
        v["summary"] = serde_json::Value::String(self.summary());
        // The per-item form, the run's delta and the withdrawal beside the
        // level, each only where it is known — the same keys the record
        // carries, so a script reads one shape from both.
        if let Some(items) = &self.items {
            v["items"] = serde_json::to_value(items).unwrap_or(serde_json::Value::Null);
        }
        if let Some(delta) = &self.delta {
            v["delta"] = serde_json::to_value(delta).unwrap_or(serde_json::Value::Null);
        }
        if self.withdrawn {
            v["withdrawn"] = serde_json::Value::Bool(true);
        }
        v
    }

    /// Is this the same sensor as `other` — same line, same kind, same
    /// setpoint spelling? An edited setpoint is a different sensor, which
    /// is what starts a fresh saturation streak.
    fn same_sensor(&self, line: &str, kind: SensorKind, setpoint: &str) -> bool {
        self.line == line && self.kind == kind && self.setpoint == setpoint
    }
}

/// The homeostat's `charter` field, loaded leniently: readings this binary
/// cannot parse — a kind or a state a later one wrote — read as `None`,
/// unknown, rather than failing the whole run record they sit on. The
/// field-level rule for a closed enum in an append-only store.
pub fn lenient<'de, D>(d: D) -> Result<Option<Vec<LineReading>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<serde_json::Value> = Option::deserialize(d)?;
    Ok(raw.and_then(|v| serde_json::from_value(v).ok()))
}

/// Each charter line as the JSON surfaces serve it — `{id, text}` with
/// `sensor: {kind, setpoint}` beside a sensored line and `reading` beside
/// that where one was taken. One shape for `mecha charter --json` and the
/// web settings endpoint, which used to build it twice; the web editor's
/// serialiser reads `sensor` back on a save, so that key's shape is a wire
/// format and `reading` is deliberately a sibling rather than a field of it.
pub fn lines_json(charter: &Charter, readings: &[LineReading]) -> Vec<serde_json::Value> {
    charter
        .lines()
        .iter()
        .map(|l| {
            let mut line = serde_json::json!({
                "id": l.id,
                "text": l.text,
            });
            if let Some(s) = &l.sensor {
                line["sensor"] = serde_json::json!({
                    "kind": s.kind.wire(),
                    "setpoint": s.setpoint_text,
                });
                if let Some(r) = readings.iter().find(|r| r.line == l.id) {
                    line["reading"] = r.json();
                }
            }
            line
        })
        .collect()
}

/// `3d 4h`, `2h 10m`, `45m`, `12s` — the largest two units that are
/// non-zero, so an age reads at the precision a person compares it at.
pub fn render_secs(secs: u64) -> String {
    let (d, rem) = (secs / 86_400, secs % 86_400);
    let (h, rem) = (rem / 3600, rem % 3600);
    let (m, s) = (rem / 60, rem % 60);
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "m"), (s, "s")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .take(2)
        .map(|(n, u)| format!("{n}{u}"))
        .collect();
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

/// The run corpus, as the `intervention_rate` kind reads it.
///
/// Kept apart from the backlog because it is read on a different budget: a
/// scan of the session store, which a per-run reader does not pay
/// ([`CorpusRate::NotScanned`] reads as [`Reading::Deferred`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CorpusRate {
    /// The reader did not scan the corpus.
    NotScanned,
    /// It tried and the store could not be read.
    Unreadable,
    /// It read the store and found no runs in the window — nothing to have
    /// stepped into.
    Empty,
    /// It read the store and found runs, but fewer than `doctor::RUNS_MIN`.
    Sparse(usize),
    /// The share of runs in the window the owner stepped into.
    Share(f64),
}

/// What the readers draw on, read once by the caller.
///
/// The front door is read twice over, on purpose: [`Backlog::frontdoor`]
/// counts every open request, which is what every recorded row has always
/// held, and `requests_on_owner` is the subset a person owes an answer to
/// (`frontdoor::waiting_on_owner`) — the set the doctor's stale-request
/// finding measures, and so the set the `request_closure` sensor must
/// measure, or a `needs_info` parked on the stranger for a week reads as a
/// saturated line no finding names (found on review).
pub struct Sources<'a> {
    pub backlog: &'a Backlog,
    /// `None` when the front door could not be read.
    pub requests_on_owner: Option<Depth>,
    pub corpus: CorpusRate,
    /// The items the depths above were counted from, for the per-item
    /// reading ([`Items`]). `None` for a caller that holds only depths — a
    /// reading then carries the level alone, never a per-item form guessed
    /// from it.
    pub items: Option<&'a Inventory>,
}

/// How far past `setpoint` an observed value sits, in `[0, 1)`.
///
/// Zero at or within the setpoint; `e / (e + setpoint)` above it, where `e`
/// is the overshoot in the kind's unit — half of maximal at twice the
/// setpoint, asymptotic from there. A zero setpoint is refused by the
/// charter parser, so the denominator is never zero here; the `e <= 0`
/// arm is also what keeps `0 / 0` out.
pub fn excess(observed: f64, setpoint: f64) -> f32 {
    let e = observed - setpoint;
    if e <= 0.0 {
        0.0
    } else {
        (e / (e + setpoint)) as f32
    }
}

fn setpoint_f64(setpoint: Setpoint) -> f64 {
    match setpoint {
        Setpoint::Duration(d) => d.as_secs_f64(),
        Setpoint::Count(n) => n as f64,
        Setpoint::Rate(r) => r,
    }
}

fn observed(value: Observed, setpoint: Setpoint) -> Reading {
    let sp = setpoint_f64(setpoint);
    let v = value.as_f64();
    Reading::Observed {
        value,
        over: v > sp,
        excess: excess(v, sp),
    }
}

/// Seconds since `stamp`, or `None` when the stamp does not parse. A future
/// stamp reads as zero — clock skew is not a negative age.
pub(crate) fn age_secs(stamp: &str, now: DateTime<Utc>) -> Option<u64> {
    let then = DateTime::parse_from_rfc3339(stamp)
        .ok()?
        .with_timezone(&Utc);
    Some(now.signed_duration_since(then).num_seconds().max(0) as u64)
}

/// The oldest waiting item's age against a duration setpoint, from one
/// store's depth: unread store → `Unread`; nothing waiting → `Nothing`; a
/// stamp that will not parse → `Unread`, because an age-blind reading would
/// score the item as fresh, which is a guess dressed as a measurement.
fn age_reading(depth: Option<&Depth>, setpoint: Setpoint, now: DateTime<Utc>) -> Reading {
    let Some(depth) = depth else {
        return Reading::Unread;
    };
    if depth.waiting == 0 {
        return Reading::Nothing;
    }
    match depth.oldest.as_deref().and_then(|s| age_secs(s, now)) {
        Some(secs) => observed(Observed::Seconds(secs), setpoint),
        None => Reading::Unread,
    }
}

/// Read one line. `None` for a line with no sensor — an unsensored line is
/// not the lesser kind (§11.1, containment 4), it just has nothing to read.
pub fn read_line(line: &CharterLine, sources: &Sources, now: DateTime<Utc>) -> Option<LineReading> {
    let sensor = line.sensor.as_ref()?;
    let backlog = sources.backlog;
    let reading = match sensor.kind {
        SensorKind::OutboxWaiting => match backlog.outbox.as_ref() {
            None => Reading::Unread,
            Some(depth) => observed(Observed::Count(depth.waiting as u64), sensor.setpoint),
        },
        SensorKind::OutboxAge => age_reading(backlog.outbox.as_ref(), sensor.setpoint, now),
        SensorKind::QuestionLatency => {
            age_reading(backlog.questions.as_ref(), sensor.setpoint, now)
        }
        SensorKind::RequestClosure => {
            age_reading(sources.requests_on_owner.as_ref(), sensor.setpoint, now)
        }
        SensorKind::InterventionRate => match &sources.corpus {
            CorpusRate::NotScanned => Reading::Deferred,
            CorpusRate::Unreadable => Reading::Unread,
            CorpusRate::Empty => Reading::Nothing,
            CorpusRate::Sparse(runs) => Reading::Sparse { runs: *runs },
            CorpusRate::Share(r) => observed(Observed::Rate(*r), sensor.setpoint),
        },
    };
    // Every kind's unit is fixed by the kind; the setpoint was typed by it
    // at load, so a mismatch here is a bug in `charter.rs`, not a reading.
    debug_assert!(matches!(
        (sensor.kind.unit(), &reading),
        (
            Unit::Duration,
            Reading::Observed {
                value: Observed::Seconds(_),
                ..
            }
        ) | (
            Unit::Count,
            Reading::Observed {
                value: Observed::Count(_),
                ..
            }
        ) | (
            Unit::Rate,
            Reading::Observed {
                value: Observed::Rate(_),
                ..
            }
        ) | (
            _,
            Reading::Unread | Reading::Deferred | Reading::Nothing | Reading::Sparse { .. }
        )
    ));
    // The per-item form only where the level was read from the same store:
    // an unread store has no items to count, and one read beside a level
    // that says `Unread` would be a second answer to one question.
    let items = match &reading {
        Reading::Observed { .. } | Reading::Nothing => sources
            .items
            .and_then(|inv| waiters_for(sensor.kind, inv))
            .map(|w| per_item(w, sensor.setpoint, now)),
        _ => None,
    };
    Some(LineReading {
        line: line.id.clone(),
        kind: sensor.kind,
        setpoint: sensor.setpoint_text.clone(),
        reading,
        items,
        delta: None,
        withdrawn: false,
    })
}

/// The items a kind reads, out of an inventory: `None` for the corpus kind,
/// which has no items, and for a store the inventory could not read.
fn waiters_for(kind: SensorKind, inv: &Inventory) -> Option<&[Waiter]> {
    match kind {
        SensorKind::OutboxWaiting | SensorKind::OutboxAge => inv.outbox.as_deref(),
        SensorKind::QuestionLatency => inv.questions.as_deref(),
        SensorKind::RequestClosure => inv.requests_on_owner.as_deref(),
        SensorKind::InterventionRate => None,
    }
}

/// One store's items read against a setpoint (`Items`). Pure: `now` is
/// injected, like every reading here.
fn per_item(waiters: &[Waiter], setpoint: Setpoint, now: DateTime<Utc>) -> Items {
    // Oldest first, the order `stale` names them in; an undated item sorts
    // last, since its age is the one thing not known about it.
    let mut aged: Vec<(Option<u64>, &Waiter)> = waiters
        .iter()
        .map(|w| (age_secs(&w.since, now), w))
        .collect();
    aged.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.1.id.cmp(&b.1.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.id.cmp(&b.1.id),
    });
    let waiting = aged.len() as u64;
    let unknown = aged.iter().filter(|(age, _)| age.is_none()).count() as u64;
    let oldest_secs = aged.iter().find_map(|(age, _)| *age);
    let stale: Vec<&Waiter> = match setpoint {
        Setpoint::Duration(d) => {
            let limit = d.as_secs();
            aged.iter()
                .filter(|(age, _)| age.is_some_and(|a| a > limit))
                .map(|(_, w)| *w)
                .collect()
        }
        // A count: the items beyond it, named oldest first — the ones the
        // queue would shed first. Undated items are still items, so the
        // count kind counts them.
        Setpoint::Count(n) => aged
            .iter()
            .take(waiting.saturating_sub(n) as usize)
            .map(|(_, w)| *w)
            .collect(),
        Setpoint::Rate(_) => Vec::new(),
    };
    Items {
        waiting,
        over: stale.len() as u64,
        // A count kind knows every item's standing without a stamp.
        unknown: match setpoint {
            Setpoint::Duration(_) => unknown,
            _ => 0,
        },
        oldest_secs,
        stale: stale
            .iter()
            .take(ITEMS_NAMED)
            .map(|w| w.id.clone())
            .collect(),
    }
}

/// Each line's per-run delta ([`LineReading::delta`]), from the inventory
/// the run started with and the one it finished with. A line whose kind has
/// no items, whose store either read could not see, or whose start reading
/// carried no per-item form (the level read `Unread`) keeps `None` — a
/// delta beside "store unreadable" would be two answers to one question.
pub fn with_deltas(readings: &mut [LineReading], before: &Inventory, after: &Inventory) {
    for r in readings {
        r.delta = match r.items {
            Some(_) => Flow::between(waiters_for(r.kind, before), waiters_for(r.kind, after)),
            None => None,
        };
    }
}

/// A line that has saturated: past its setpoint on each of the last
/// [`SATURATED_AFTER_RUNS`] informative recorded runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Saturated {
    pub line: String,
    pub kind: SensorKind,
    pub setpoint: String,
    /// The newest of those runs' per-item reading, so a finding can name
    /// the items. `None` where that run recorded none.
    pub items: Option<Items>,
}

/// The lines of `charter` saturated over `rows` — each recorded run's
/// charter readings, **newest first**. The one definition the doctor's
/// finding and a run's withdrawal both use, so the two cannot disagree
/// about which line is saturated.
///
/// Only readings of the *same sensor* count (line, kind, setpoint
/// spelling), and only informative ones: a row whose reading says nothing
/// either way (`Unread`, `Deferred`, `Sparse`, or no reading of the line)
/// is skipped, never counted on either side. Lazy: it stops pulling rows
/// once every line in `only` is decided, so a caller streaming a session
/// store pays for the rows it needed and no more.
///
/// **Bounded, because a streak may never decide** (found on review). Two
/// cases: the corpus kind, which a run records as `Deferred` on every row
/// and so can never saturate — it is left out here; and a setpoint the
/// owner just edited, whose new spelling no recorded row has read yet.
/// The second is what [`SATURATION_ROWS_MAX`] caps: without it, every run
/// for the next ten would parse the whole doctor window at its start.
pub fn saturated<I, R>(charter: &Charter, only: Option<&[&str]>, rows: I) -> Vec<Saturated>
where
    I: IntoIterator<Item = R>,
    R: AsRef<[LineReading]>,
{
    struct Streak<'a> {
        line: &'a CharterLine,
        over: usize,
        newest: Option<Option<Items>>,
        decided: bool,
    }
    let mut streaks: Vec<Streak> = charter
        .lines()
        .iter()
        .filter(|l| {
            l.sensor
                .as_ref()
                .is_some_and(|s| s.kind != SensorKind::InterventionRate)
        })
        .filter(|l| only.is_none_or(|o| o.contains(&l.id.as_str())))
        .map(|line| Streak {
            line,
            over: 0,
            newest: None,
            decided: false,
        })
        .collect();
    let mut out = Vec::new();
    if streaks.is_empty() {
        return out;
    }
    for row in rows.into_iter().take(SATURATION_ROWS_MAX) {
        let row = row.as_ref();
        for s in streaks.iter_mut().filter(|s| !s.decided) {
            let sensor = s.line.sensor.as_ref().expect("filtered to sensored lines");
            let Some(r) = row
                .iter()
                .find(|r| r.same_sensor(&s.line.id, sensor.kind, &sensor.setpoint_text))
            else {
                continue;
            };
            match r.reading.over() {
                None => continue,
                Some(false) => s.decided = true,
                Some(true) => {
                    if s.newest.is_none() {
                        s.newest = Some(r.items.clone());
                    }
                    s.over += 1;
                    if s.over == SATURATED_AFTER_RUNS {
                        s.decided = true;
                        out.push(Saturated {
                            line: s.line.id.clone(),
                            kind: sensor.kind,
                            setpoint: sensor.setpoint_text.clone(),
                            items: s.newest.clone().flatten(),
                        });
                    }
                }
            }
        }
        if streaks.iter().all(|s| s.decided) {
            break;
        }
    }
    // Charter order, whatever order the streaks completed in.
    out.sort_by_key(|s| charter.rank_of(&s.line));
    out
}

/// Each recorded run's charter readings in a session store, newest first,
/// read lazily — one transcript at a time, and only as far as the consumer
/// pulls. The same admission and the same window as the doctor's run
/// check (`Scan::default`, [`crate::doctor::RUNS_WINDOW`] sessions), so a
/// streak read here and one read there are over the same rows.
///
/// Never fails: an unreadable store or transcript contributes nothing,
/// which can only *shorten* a streak — the direction that leaves a line in
/// front of the run, as it was before withdrawal existed.
pub fn recorded_readings(dir: &std::path::Path) -> impl Iterator<Item = Vec<LineReading>> {
    let dir = dir.to_path_buf();
    // Lazy from the first step: even the listing — a header read per
    // transcript — waits for the first row to be pulled, so a caller that
    // pulls none (no line over its setpoint) touches nothing.
    std::iter::once(()).flat_map(move |()| {
        let scan = crate::runlog::Scan::default();
        let listed = crate::session::Session::list_counting(&dir)
            .map(|(listed, _)| listed)
            .unwrap_or_default();
        listed
            .into_iter()
            .filter(move |(meta, _)| scan.admits(meta))
            .take(crate::doctor::RUNS_WINDOW)
            .flat_map(|(_, path)| {
                let mut rows: Vec<Vec<LineReading>> =
                    crate::session::Session::outcomes_attributed(&path)
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|(_, _, stats)| stats.homeostat?.charter)
                        .collect();
                // A session's runs are recorded oldest first.
                rows.reverse();
                rows
            })
    })
}

/// Withdraw the saturated lines from a run's readings: mark
/// [`LineReading::withdrawn`] on each line that reads past its setpoint now
/// *and* did on each of the last [`SATURATED_AFTER_RUNS`] informative
/// recorded runs (`rows`, newest first — [`recorded_readings`] in a run).
///
/// A line reading within its setpoint now is never withdrawn, whatever its
/// history: the streak is broken, and a consumer should see a met line. And
/// only lines reading over now are looked up, so a run on a healthy charter
/// reads no transcript at all.
pub fn withdraw_saturated<I, R>(readings: &mut [LineReading], charter: &Charter, rows: I)
where
    I: IntoIterator<Item = R>,
    R: AsRef<[LineReading]>,
{
    let over_now: Vec<&str> = readings
        .iter()
        .filter(|r| r.reading.over() == Some(true))
        .map(|r| r.line.as_str())
        .collect();
    if over_now.is_empty() {
        return;
    }
    let saturated = saturated(charter, Some(&over_now), rows);
    for r in readings.iter_mut() {
        if saturated
            .iter()
            .any(|s| r.same_sensor(&s.line, s.kind, &s.setpoint))
        {
            r.withdrawn = true;
        }
    }
}

/// The readings a run's in-run consumers may read: every line except the
/// withdrawn ones. `None` stays `None` — no reading taken is not the same
/// fact as every line withdrawn.
pub fn in_run(readings: Option<&[LineReading]>) -> Option<Vec<LineReading>> {
    readings.map(|rs| rs.iter().filter(|r| !r.withdrawn).cloned().collect())
}

/// Every sensored line, in charter order. Empty for a charter with no
/// sensor, which a caller reports as *having none* rather than as reading
/// nothing (`Charter::has_sensors`).
pub fn read_lines(charter: &Charter, sources: &Sources, now: DateTime<Utc>) -> Vec<LineReading> {
    charter
        .lines()
        .iter()
        .filter_map(|l| read_line(l, sources, now))
        .collect()
}

/// Does any line read from the corpus? Decides whether [`read_charter`]
/// pays for the scan.
fn wants_corpus(charter: &Charter) -> bool {
    charter.lines().iter().any(|l| {
        l.sensor
            .as_ref()
            .is_some_and(|s| s.kind == SensorKind::InterventionRate)
    })
}

/// The corpus kind's source, read from the session store under the mecha
/// home over the doctor's window — the one reader here that pays for a scan,
/// which is why it is a separate call rather than part of [`read_lines`].
pub fn corpus_rate() -> CorpusRate {
    let Ok(home) = crate::work::mecha_home() else {
        return CorpusRate::Unreadable;
    };
    corpus_rate_in(&home.join("sessions"))
}

/// [`corpus_rate`] over a named session store, so a test can hand it a
/// directory. Three ways to read as not-a-share, kept apart: a store that
/// has never existed or holds no run is `Empty`; a store whose every
/// transcript is torn is `Unreadable` — `Corpus::scan` returns `Ok` with
/// `unreadable` counted and no rows for that, which is exactly the rot the
/// doctor's own finding exists for, and reading it as "no runs" reported
/// unknown as met (found on review); and fewer runs than the doctor's own
/// floor is `Sparse`, because a share of three runs is noise there and is
/// noise here.
pub fn corpus_rate_in(dir: &std::path::Path) -> CorpusRate {
    use crate::runlog::{Corpus, Scan};
    if !dir.is_dir() {
        // A machine that has never recorded a run has nothing to have
        // stepped into — empty, not unreadable, on `Backlog::read`'s rule
        // for a store that has never existed.
        return CorpusRate::Empty;
    }
    let corpus = match Corpus::scan(
        dir,
        &Scan {
            max_sessions: Some(crate::doctor::RUNS_WINDOW),
            since: None,
            workspace: None,
            kind: None,
            include_tests: false,
            include_experiments: crate::experiment::in_experiment_home(),
        },
    ) {
        Ok(corpus) => corpus,
        Err(_) => return CorpusRate::Unreadable,
    };
    if corpus.is_empty() {
        return if corpus.unreadable > 0 {
            CorpusRate::Unreadable
        } else {
            CorpusRate::Empty
        };
    }
    if corpus.len() < crate::doctor::RUNS_MIN {
        return CorpusRate::Sparse(corpus.len());
    }
    match corpus.intervention_rate() {
        Some(rate) => CorpusRate::Share(rate),
        None => CorpusRate::Empty,
    }
}

/// Read the whole charter against the live stores, for a surface the owner
/// reads: the backlog's three stores, and the corpus only when a line asks
/// for it. Not for a run — [`crate::homeostat::Homeostat::at_start`] reads
/// the backlog it already holds and defers the corpus kind.
pub fn read_charter(charter: &Charter, now: DateTime<Utc>) -> Vec<LineReading> {
    let survey = Backlog::survey();
    let corpus = if wants_corpus(charter) {
        corpus_rate()
    } else {
        CorpusRate::NotScanned
    };
    let mut readings = read_lines(
        charter,
        &Sources {
            backlog: &survey.backlog,
            requests_on_owner: survey.requests_on_owner,
            corpus,
            items: Some(&survey.items),
        },
        now,
    );
    // The surface says what a run would see: a line withdrawn from runs is
    // marked here too, from the same rows, so the owner reads it where the
    // setpoint is edited — not only in the doctor.
    if let Ok(sessions) = crate::session::Session::default_dir() {
        withdraw_saturated(&mut readings, charter, recorded_readings(&sessions));
    }
    readings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charter::{RawLine, RawSensor, RawSetpoint};

    fn charter(lines: Vec<RawLine>) -> Charter {
        Charter::from_raw_lines(lines).unwrap()
    }

    fn sensored(id: &str, kind: SensorKind, setpoint: &str) -> RawLine {
        RawLine {
            id: id.to_string(),
            text: format!("{id} text"),
            sensor: Some(RawSensor {
                kind,
                setpoint: RawSetpoint::Text(setpoint.to_string()),
            }),
        }
    }

    fn depth(waiting: usize, oldest: Option<&str>) -> Depth {
        Depth {
            waiting,
            oldest: oldest.map(str::to_string),
            given_up: 0,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-09-04T12:00:00Z".parse().unwrap()
    }

    /// Sources whose owner-facing requests are the backlog's front door
    /// unchanged — for the tests that are not about that distinction.
    fn src(backlog: &Backlog, corpus: CorpusRate) -> Sources<'_> {
        Sources {
            backlog,
            requests_on_owner: backlog.frontdoor.clone(),
            corpus,
            items: None,
        }
    }

    /// `request_closure` reads the requests waiting on the owner, not every
    /// open request: a week-old `needs_info` sits in the backlog's front
    /// door, and the line reads nothing waiting.
    #[test]
    fn a_request_parked_on_the_stranger_does_not_age_the_request_closure_line() {
        let c = charter(vec![sensored("close", SensorKind::RequestClosure, "72h")]);
        let backlog = Backlog {
            frontdoor: Some(depth(1, Some("2026-08-28T12:00:00Z"))),
            ..Default::default()
        };
        let widened = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        assert_eq!(
            widened[0].reading.over(),
            Some(true),
            "the wide reader would saturate"
        );
        let narrowed = read_lines(
            &c,
            &Sources {
                backlog: &backlog,
                requests_on_owner: Some(depth(0, None)),
                corpus: CorpusRate::NotScanned,
                items: None,
            },
            now(),
        );
        assert_eq!(narrowed[0].reading, Reading::Nothing);
        // And an unreadable front door is unread on the line, whatever the
        // wide depth says.
        let unread = read_lines(
            &c,
            &Sources {
                backlog: &backlog,
                requests_on_owner: None,
                corpus: CorpusRate::NotScanned,
                items: None,
            },
            now(),
        );
        assert_eq!(unread[0].reading, Reading::Unread);
    }

    #[test]
    fn excess_is_zero_within_the_setpoint_half_at_twice_it_and_never_one() {
        assert_eq!(excess(10.0, 24.0), 0.0);
        assert_eq!(excess(24.0, 24.0), 0.0);
        assert_eq!(excess(48.0, 24.0), 0.5);
        let far = excess(24.0 * 1_000_000.0, 24.0);
        assert!(far < 1.0 && far > 0.99, "{far}");
        // No NaN from a degenerate pair, whatever the parser lets through.
        assert_eq!(excess(0.0, 0.0), 0.0);
    }

    #[test]
    fn an_age_kind_reads_the_oldest_item_against_a_duration_setpoint() {
        let c = charter(vec![sensored("waits", SensorKind::OutboxAge, "24h")]);
        let backlog = Backlog {
            outbox: Some(depth(2, Some("2026-09-02T12:00:00Z"))),
            ..Default::default()
        };
        let r = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].line, "waits");
        assert_eq!(r[0].setpoint, "24h");
        // Two days against one: over, and half of maximal.
        assert_eq!(
            r[0].reading,
            Reading::Observed {
                value: Observed::Seconds(2 * 86_400),
                over: true,
                excess: 0.5
            }
        );
        assert_eq!(r[0].summary(), "2d, past the 24h setpoint");
    }

    #[test]
    fn nothing_waiting_is_nothing_for_an_age_and_a_zero_for_a_count() {
        let c = charter(vec![
            sensored("age", SensorKind::OutboxAge, "24h"),
            sensored("count", SensorKind::OutboxWaiting, "3"),
        ]);
        let backlog = Backlog {
            outbox: Some(depth(0, None)),
            ..Default::default()
        };
        let r = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        assert_eq!(r[0].reading, Reading::Nothing);
        assert_eq!(r[0].summary(), "nothing waiting");
        assert_eq!(
            r[1].reading,
            Reading::Observed {
                value: Observed::Count(0),
                over: false,
                excess: 0.0
            }
        );
        assert_eq!(r[1].summary(), "0 waiting, within the 3 setpoint");
        assert_eq!(r[0].reading.over(), Some(false));
        assert_eq!(r[0].reading.excess(), Some(0.0));
    }

    #[test]
    fn an_unreadable_store_and_an_unparseable_stamp_are_both_unread_never_fresh() {
        let c = charter(vec![
            sensored("q", SensorKind::QuestionLatency, "12h"),
            sensored("r", SensorKind::RequestClosure, "72h"),
        ]);
        let backlog = Backlog {
            questions: None,
            frontdoor: Some(depth(1, Some("not a stamp"))),
            ..Default::default()
        };
        let r = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        assert_eq!(r[0].reading, Reading::Unread);
        assert_eq!(r[1].reading, Reading::Unread);
        assert_eq!(r[0].reading.over(), None);
        assert_eq!(r[0].reading.excess(), None);
        assert_eq!(r[0].summary(), "store unreadable");
    }

    #[test]
    fn the_corpus_kind_is_deferred_in_a_run_and_read_on_a_surface() {
        let c = charter(vec![sensored("hands", SensorKind::InterventionRate, "20%")]);
        let backlog = Backlog::default();
        let deferred = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        assert_eq!(deferred[0].reading, Reading::Deferred);
        assert_eq!(deferred[0].reading.over(), None);

        let read = read_lines(&c, &src(&backlog, CorpusRate::Share(0.3)), now());
        assert_eq!(
            read[0].reading,
            Reading::Observed {
                value: Observed::Rate(0.3),
                over: true,
                // 0.1 over a 0.2 setpoint: 0.1 / 0.3.
                excess: (0.1f64 / 0.30000000000000004f64) as f32
            }
        );
        assert_eq!(
            read[0].summary(),
            "30% of recent runs, past the 20% setpoint"
        );

        let empty = read_lines(&c, &src(&backlog, CorpusRate::Empty), now());
        assert_eq!(empty[0].reading, Reading::Nothing);
        assert_eq!(empty[0].summary(), "no runs recorded yet");
        let sparse = read_lines(&c, &src(&backlog, CorpusRate::Sparse(2)), now());
        assert_eq!(sparse[0].reading, Reading::Sparse { runs: 2 });
        assert_eq!(sparse[0].reading.over(), None);
        assert_eq!(sparse[0].reading.excess(), None);
        assert_eq!(
            sparse[0].summary(),
            format!(
                "only 2 runs recorded, under the {}-run floor a share needs",
                crate::doctor::RUNS_MIN
            )
        );
        let unreadable = read_lines(&c, &src(&backlog, CorpusRate::Unreadable), now());
        assert_eq!(unreadable[0].reading, Reading::Unread);
    }

    #[test]
    fn an_unsensored_line_reads_nothing_and_a_sensorless_charter_reads_empty() {
        let c = charter(vec![
            RawLine {
                id: "plain".into(),
                text: "no sensor".into(),
                sensor: None,
            },
            sensored("age", SensorKind::OutboxAge, "24h"),
        ]);
        let r = read_lines(&c, &src(&Backlog::default(), CorpusRate::NotScanned), now());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].line, "age");
        let none = charter(vec![RawLine {
            id: "plain".into(),
            text: "no sensor".into(),
            sensor: None,
        }]);
        assert!(read_lines(
            &none,
            &src(&Backlog::default(), CorpusRate::NotScanned),
            now()
        )
        .is_empty());
    }

    /// The record rides in every session file from now on: it must round
    /// trip, an old row without it must load, and a variant this binary
    /// does not know must not fail the record.
    #[test]
    fn a_reading_round_trips_and_an_unknown_state_degrades_on_load() {
        let r = LineReading {
            line: "waits".into(),
            kind: SensorKind::OutboxAge,
            setpoint: "24h".into(),
            reading: Reading::Observed {
                value: Observed::Seconds(90_000),
                over: true,
                excess: 0.25,
            },
            items: None,
            delta: None,
            withdrawn: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"state\":\"observed\""), "{json}");
        assert!(json.contains("\"seconds\":90000"), "{json}");
        assert_eq!(serde_json::from_str::<LineReading>(&json).unwrap(), r);

        let deferred = serde_json::to_string(&Reading::Deferred).unwrap();
        assert_eq!(deferred, r#"{"state":"deferred"}"#);

        let j = r.json();
        assert_eq!(j["summary"], "1d 1h, past the 24h setpoint");
        assert_eq!(j["state"], "observed");
        assert_eq!(j["over"], true);
        assert_eq!(j["value"]["seconds"], 90_000);
    }

    #[test]
    fn the_json_surface_puts_the_reading_beside_the_sensor_never_inside_it() {
        let c = charter(vec![
            RawLine {
                id: "plain".into(),
                text: "no sensor".into(),
                sensor: None,
            },
            sensored("waits", SensorKind::OutboxAge, "24h"),
        ]);
        let backlog = Backlog {
            outbox: Some(depth(0, None)),
            ..Default::default()
        };
        let readings = read_lines(&c, &src(&backlog, CorpusRate::NotScanned), now());
        let json = lines_json(&c, &readings);
        assert_eq!(
            json[0],
            serde_json::json!({"id": "plain", "text": "no sensor"})
        );
        // `sensor` is what the web editor writes back on a save: exactly
        // the two keys the serialiser knows, with the reading a sibling.
        assert_eq!(
            json[1]["sensor"],
            serde_json::json!({"kind": "outbox_age", "setpoint": "24h"})
        );
        assert_eq!(json[1]["reading"]["state"], "nothing");
        assert_eq!(json[1]["reading"]["summary"], "nothing waiting");
    }

    /// The three not-a-share readings of a store, told apart: never
    /// existed, every transcript torn, and fewer runs than the floor. The
    /// torn case is the one that read as "no runs" before — `Corpus::scan`
    /// is `Ok` with the rot counted, not `Err`.
    #[test]
    fn a_torn_corpus_is_unreadable_and_a_thin_one_is_sparse_never_met() {
        use crate::session::{Record, RunStats, Session, SessionMeta};
        let root = std::env::temp_dir().join(format!(
            "mecha-reading-corpus-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(corpus_rate_in(&root.join("never")), CorpusRate::Empty);

        let torn = root.join("torn");
        std::fs::create_dir_all(&torn).unwrap();
        std::fs::write(torn.join("20260801T000000-x.jsonl"), "{not json\n").unwrap();
        assert_eq!(corpus_rate_in(&torn), CorpusRate::Unreadable);

        let thin = root.join("thin");
        std::fs::create_dir_all(&thin).unwrap();
        for i in 0..3 {
            let s = Session::create(
                &thin,
                SessionMeta {
                    id: format!("20260801T00000{i}-r"),
                    created_at: now(),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            s.append(&Record::Outcome(RunStats::default())).unwrap();
        }
        assert_eq!(corpus_rate_in(&thin), CorpusRate::Sparse(3));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ages_render_at_two_units() {
        assert_eq!(render_secs(0), "0s");
        assert_eq!(render_secs(12), "12s");
        assert_eq!(render_secs(45 * 60), "45m");
        assert_eq!(render_secs(2 * 3600 + 10 * 60 + 5), "2h 10m");
        assert_eq!(render_secs(3 * 86_400 + 4 * 3600 + 59), "3d 4h");
        assert_eq!(render_secs(86_400), "1d");
    }

    #[test]
    fn a_future_stamp_is_a_zero_age_not_a_negative_one() {
        assert_eq!(age_secs("2026-09-05T12:00:00Z", now()), Some(0));
        assert_eq!(age_secs("2026-09-04T11:59:00Z", now()), Some(60));
        assert_eq!(age_secs("yesterday", now()), None);
    }

    // --- S5: per item, per run, and saturation ---------------------------

    /// `n` fresh drafts an hour old and, when `stale`, one five days old.
    fn outbox_items(stale: bool, fresh: &[&str]) -> Inventory {
        let mut items: Vec<Waiter> = fresh
            .iter()
            .map(|id| Waiter::new(*id, "2026-09-04T11:00:00Z"))
            .collect();
        if stale {
            items.push(Waiter::new("old-draft", "2026-08-30T12:00:00Z"));
        }
        Inventory {
            outbox: Some(items),
            ..Default::default()
        }
    }

    fn depth_of(inv: &Inventory) -> Backlog {
        let items = inv.outbox.as_deref().unwrap();
        Backlog {
            outbox: Some(Depth {
                waiting: items.len(),
                oldest: items.iter().map(|w| w.since.clone()).min(),
                given_up: 0,
            }),
            ..Default::default()
        }
    }

    fn read_with(c: &Charter, inv: &Inventory) -> Vec<LineReading> {
        let backlog = depth_of(inv);
        read_lines(
            c,
            &Sources {
                backlog: &backlog,
                requests_on_owner: None,
                corpus: CorpusRate::NotScanned,
                items: Some(inv),
            },
            now(),
        )
    }

    /// The acceptance's fixture, in the pure reader: one stale draft and
    /// several fresh ones. The level is pinned past the setpoint by the
    /// stale one however the queue moves; the per-item reading moves with
    /// every draft added and cleared, and names the one that is stale.
    #[test]
    fn one_stale_item_pins_the_level_while_the_per_item_reading_moves() {
        let c = charter(vec![sensored("replies", SensorKind::OutboxAge, "24h")]);
        let three = read_with(&c, &outbox_items(true, &["a", "b", "c"]));
        let five = read_with(&c, &outbox_items(true, &["a", "b", "c", "d", "e"]));
        let one = read_with(&c, &outbox_items(true, &["e"]));
        // The level: identical on all three.
        for r in [&three[0], &five[0], &one[0]] {
            assert_eq!(
                r.reading,
                Reading::Observed {
                    value: Observed::Seconds(5 * 86_400),
                    over: true,
                    excess: 0.8
                }
            );
        }
        // Per item: the count moves, one item past the setpoint, named.
        let items = |r: &LineReading| r.items.clone().unwrap();
        assert_eq!(
            items(&three[0]),
            Items {
                waiting: 4,
                over: 1,
                unknown: 0,
                oldest_secs: Some(5 * 86_400),
                stale: vec!["old-draft".into()],
            }
        );
        assert_eq!(items(&five[0]).waiting, 6);
        assert_eq!(items(&one[0]).waiting, 2);
        assert_eq!(items(&five[0]).over, 1);
        assert_eq!(
            three[0].summary(),
            "5d, past the 24h setpoint; 1 of 4 items past it"
        );
        // Clearing the stale one clears the line; the fresh ones stay.
        let cleared = read_with(&c, &outbox_items(false, &["a", "b"]));
        assert_eq!(cleared[0].reading.over(), Some(false));
        assert_eq!(items(&cleared[0]).over, 0);
        assert_eq!(items(&cleared[0]).waiting, 2);
    }

    /// A count kind's items past it are the ones beyond the count, named
    /// oldest first; an age kind's undated item is unknown, never within.
    #[test]
    fn a_count_kind_counts_beyond_the_count_and_an_undated_item_is_unknown() {
        let c = charter(vec![
            sensored("queue", SensorKind::OutboxWaiting, "2"),
            sensored("age", SensorKind::OutboxAge, "24h"),
        ]);
        let mut inv = outbox_items(true, &["a", "b", "c"]);
        inv.outbox
            .as_mut()
            .unwrap()
            .push(Waiter::new("torn", "not a stamp"));
        let r = read_with(&c, &inv);
        let queue = r[0].items.clone().unwrap();
        assert_eq!((queue.waiting, queue.over, queue.unknown), (5, 3, 0));
        assert_eq!(queue.stale, vec!["old-draft", "a", "b"]);
        let age = r[1].items.clone().unwrap();
        assert_eq!((age.waiting, age.over, age.unknown), (5, 1, 1));
        assert!(r[1].summary().ends_with("1 of 5 items past it, 1 undated"));
    }

    /// No items handed in, an unread store, or the corpus kind: no per-item
    /// form, never one guessed from the level.
    #[test]
    fn a_per_item_reading_is_absent_where_nothing_was_counted() {
        let c = charter(vec![
            sensored("age", SensorKind::OutboxAge, "24h"),
            sensored("hands", SensorKind::InterventionRate, "20%"),
            sensored("q", SensorKind::QuestionLatency, "12h"),
        ]);
        let inv = outbox_items(true, &["a"]);
        let backlog = depth_of(&inv);
        let without = read_lines(&c, &src(&backlog, CorpusRate::Share(0.5)), now());
        assert!(without.iter().all(|r| r.items.is_none()));
        let with = read_with(&c, &inv);
        assert!(with[0].items.is_some());
        assert!(with[1].items.is_none(), "the corpus kind has no items");
        assert_eq!(with[2].reading, Reading::Unread);
        assert!(with[2].items.is_none(), "an unread store counts nothing");
    }

    /// The per-run delta is id by id: a run that added one and cleared one
    /// moved the queue, which the net count reads as nothing.
    #[test]
    fn the_per_run_delta_counts_ids_added_and_cleared() {
        let c = charter(vec![
            sensored("replies", SensorKind::OutboxAge, "24h"),
            sensored("q", SensorKind::QuestionLatency, "12h"),
        ]);
        let before = outbox_items(true, &["a", "b", "c"]);
        let after = outbox_items(true, &["b", "c", "d"]);
        let mut r = read_with(&c, &before);
        with_deltas(&mut r, &before, &after);
        assert_eq!(
            r[0].delta,
            Some(Flow {
                added: 1,
                cleared: 1
            })
        );
        assert!(r[0].delta.unwrap().moved());
        assert_eq!(r[1].delta, None, "an unread store has no delta");
        assert!(r[0].summary().ends_with("; this run added 1 and cleared 1"));
        // Both readings still pinned: the level cannot tell these runs apart.
        let after_reading = read_with(&c, &after);
        assert_eq!(r[0].reading, after_reading[0].reading);
    }

    /// A recorded run's readings, as the doctor and a run read them back.
    fn row(reading: Reading, setpoint: &str, items: Option<Items>) -> Vec<LineReading> {
        vec![LineReading {
            line: "replies".into(),
            kind: SensorKind::OutboxAge,
            setpoint: setpoint.into(),
            reading,
            items,
            delta: None,
            withdrawn: false,
        }]
    }

    fn over_row() -> Vec<LineReading> {
        row(
            Reading::Observed {
                value: Observed::Seconds(200_000),
                over: true,
                excess: 0.5,
            },
            "24h",
            None,
        )
    }

    /// The one definition of saturation: ten informative over-readings of
    /// the same sensor, newest first; an uninformative row is skipped, a
    /// met one breaks the streak, another setpoint is another sensor, and
    /// the newest run's items are what a finding names.
    #[test]
    fn saturation_is_ten_informative_readings_past_the_setpoint_of_one_sensor() {
        let c = charter(vec![sensored("replies", SensorKind::OutboxAge, "24h")]);
        let named = Items {
            waiting: 4,
            over: 1,
            stale: vec!["old-draft".into()],
            ..Default::default()
        };
        let mut rows = vec![row(
            Reading::Observed {
                value: Observed::Seconds(200_000),
                over: true,
                excess: 0.5,
            },
            "24h",
            Some(named.clone()),
        )];
        rows.extend(std::iter::repeat_n(over_row(), SATURATED_AFTER_RUNS - 1));
        // Skipped, not counted and not breaking: nothing either way.
        rows.insert(3, row(Reading::Unread, "24h", None));
        rows.insert(4, Vec::new());
        let s = saturated(&c, None, &rows);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].line, "replies");
        assert_eq!(s[0].items, Some(named));

        // Nine is not ten.
        let nine: Vec<_> = std::iter::repeat_n(over_row(), SATURATED_AFTER_RUNS - 1).collect();
        assert!(saturated(&c, None, &nine).is_empty());
        // A met reading anywhere inside the ten breaks it.
        let mut broken: Vec<_> = std::iter::repeat_n(over_row(), SATURATED_AFTER_RUNS).collect();
        broken.insert(5, row(Reading::Nothing, "24h", None));
        assert!(saturated(&c, None, &broken).is_empty());
        // Another setpoint spelling is another sensor.
        let other: Vec<_> = std::iter::repeat_n(
            row(
                Reading::Observed {
                    value: Observed::Seconds(200_000),
                    over: true,
                    excess: 0.5,
                },
                "48h",
                None,
            ),
            SATURATED_AFTER_RUNS,
        )
        .collect();
        assert!(saturated(&c, None, &other).is_empty());
    }

    /// Lazy: a caller streaming transcripts pays for the rows it needed.
    /// A met newest row decides the line at once; a saturated line stops
    /// at its tenth.
    #[test]
    fn saturation_stops_pulling_rows_once_every_line_is_decided() {
        let c = charter(vec![sensored("replies", SensorKind::OutboxAge, "24h")]);
        let pulled = std::cell::Cell::new(0usize);
        let endless = std::iter::repeat_with(|| {
            pulled.set(pulled.get() + 1);
            over_row()
        });
        assert_eq!(saturated(&c, None, endless.take(1_000)).len(), 1);
        assert_eq!(pulled.get(), SATURATED_AFTER_RUNS);

        pulled.set(0);
        let met_first = std::iter::once(row(Reading::Nothing, "24h", None)).chain(
            std::iter::repeat_with(|| {
                pulled.set(pulled.get() + 1);
                over_row()
            })
            .take(1_000),
        );
        assert!(saturated(&c, None, met_first).is_empty());
        assert_eq!(pulled.get(), 0);
    }

    /// Bounded (found on review): a streak that can never decide — a
    /// setpoint just edited, whose new spelling no row has read — stops at
    /// `SATURATION_ROWS_MAX` rows rather than reading the whole window; and
    /// the corpus kind, which a run records as `Deferred` on every row, is
    /// never a candidate and pulls nothing.
    #[test]
    fn a_streak_that_cannot_decide_stops_at_the_row_cap() {
        let edited = charter(vec![sensored("replies", SensorKind::OutboxAge, "36h")]);
        let pulled = std::cell::Cell::new(0usize);
        let rows = std::iter::repeat_with(|| {
            pulled.set(pulled.get() + 1);
            over_row() // read against "24h": another sensor
        })
        .take(1_000);
        assert!(saturated(&edited, None, rows).is_empty());
        assert_eq!(pulled.get(), SATURATION_ROWS_MAX);

        let corpus = charter(vec![sensored("hands", SensorKind::InterventionRate, "20%")]);
        pulled.set(0);
        let rows = std::iter::repeat_with(|| {
            pulled.set(pulled.get() + 1);
            over_row()
        })
        .take(1_000);
        assert!(saturated(&corpus, None, rows).is_empty());
        assert_eq!(pulled.get(), 0, "the corpus kind is never a candidate");
    }

    /// Withdrawal: a line past its setpoint now and saturated over the
    /// record is withheld from in-run consumers and still recorded; a line
    /// within its setpoint now never is, whatever its history; a healthy
    /// charter reads no row at all.
    #[test]
    fn a_saturated_line_is_withdrawn_from_the_run_and_kept_on_the_record() {
        let c = charter(vec![
            sensored("replies", SensorKind::OutboxAge, "24h"),
            sensored("queue", SensorKind::OutboxWaiting, "10"),
        ]);
        let history: Vec<_> = std::iter::repeat_n(over_row(), SATURATED_AFTER_RUNS).collect();
        let mut now_readings = read_with(&c, &outbox_items(true, &["a", "b"]));
        assert_eq!(now_readings[0].reading.over(), Some(true));
        assert_eq!(now_readings[1].reading.over(), Some(false));
        withdraw_saturated(&mut now_readings, &c, &history);
        assert!(now_readings[0].withdrawn);
        assert!(!now_readings[1].withdrawn);
        assert!(now_readings[0]
            .summary()
            .ends_with("withdrawn from runs — past its setpoint on each of the last 10"));
        let seen = in_run(Some(&now_readings)).unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].line, "queue");
        assert_eq!(in_run(None), None, "no reading is not every line withdrawn");

        // The stale draft sent: within now, so never withdrawn.
        let mut met = read_with(&c, &outbox_items(false, &["a"]));
        withdraw_saturated(&mut met, &c, &history);
        assert!(met.iter().all(|r| !r.withdrawn));

        // Nothing over now: the rows are never pulled.
        let pulled = std::cell::Cell::new(0usize);
        let rows = std::iter::repeat_with(|| {
            pulled.set(pulled.get() + 1);
            over_row()
        })
        .take(100);
        withdraw_saturated(&mut met, &c, rows);
        assert_eq!(pulled.get(), 0);
    }

    /// The new fields are a wire format: they round-trip, an old row
    /// without them loads with none, and absent ones are not written.
    #[test]
    fn the_per_item_fields_round_trip_and_an_old_row_loads_without_them() {
        let mut r = read_with(
            &charter(vec![sensored("replies", SensorKind::OutboxAge, "24h")]),
            &outbox_items(true, &["a"]),
        )
        .remove(0);
        r.delta = Some(Flow {
            added: 2,
            cleared: 1,
        });
        r.withdrawn = true;
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<LineReading>(&json).unwrap(), r);
        let j = r.json();
        assert_eq!(j["items"]["over"], 1);
        assert_eq!(j["items"]["stale"][0], "old-draft");
        assert_eq!(j["delta"]["added"], 2);
        assert_eq!(j["withdrawn"], true);

        let old = r#"{"line":"replies","kind":"outbox_age","setpoint":"24h","reading":{"state":"nothing"}}"#;
        let loaded: LineReading = serde_json::from_str(old).unwrap();
        assert_eq!(loaded.items, None);
        assert_eq!(loaded.delta, None);
        assert!(!loaded.withdrawn);
        let rewritten = serde_json::to_string(&loaded).unwrap();
        assert_eq!(rewritten, old, "absent fields are not written");
    }
}
