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
//!   [`Reading::Deferred`]; nothing waiting is [`Reading::Nothing`]; and only
//!   [`Reading::Observed`] carries a value. Four facts, kept apart, because
//!   a finding that fires on an absence is the failure `backlog.rs` and
//!   `guilt.rs` both state at length.
//!
//! ## Line-specific guilt, and why the term is asymptotic
//!
//! §11.1's promise is that "harmed another" becomes "a recorded commitment
//! aged past *this* line's setpoint", one reading per sensored line instead
//! of the single saturated number `guilt.rs` found on the live store. The
//! magnitude here — [`Reading::Observed`]'s `excess` — is how far past the
//! setpoint the observable sits, in `[0, 1)`: zero within the setpoint, half
//! of maximal at twice it, approaching but never reaching one. Asymptotic
//! for exactly the reason `AGE_HALF_AT_HOURS` in `guilt.rs` is: a term that
//! reaches `1.0` stops varying, and a corpus of a constant carries nothing.
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
//!   run *inherited*, the same level `anticipated_guilt` reads, so the record
//!   says what waited on the owner as the run began. The corpus kind
//!   (`intervention_rate`) is [`Reading::Deferred`] there: reading it is a
//!   scan of the session store, and `guilt.rs`'s rule for the graph applies
//!   — fine once a night, too expensive in the path of every run.
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

use crate::backlog::{Backlog, Depth};
use crate::charter::{Charter, CharterLine, SensorKind, Setpoint, Unit};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How many consecutive recorded runs a line may read past its setpoint
/// before the doctor calls the sensor saturated (§11.1, containment 5).
///
/// Argued, not measured: ten runs is a day or two of ordinary use, long
/// enough that a genuinely overdue draft has been nagged about by the
/// doctor's own stuck-draft finding, and short enough that a setpoint set
/// in the wrong unit is caught in the first session that notices the
/// charter page. Only informative readings count toward the streak — an
/// [`Reading::Unread`] or [`Reading::Deferred`] row says nothing either way.
pub const SATURATED_AFTER_RUNS: usize = 10;

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
            Reading::Unread | Reading::Deferred => None,
            Reading::Nothing => Some(false),
            Reading::Observed { over, .. } => Some(*over),
        }
    }

    /// The line-specific guilt term: `excess` where a value was observed,
    /// zero where nothing waits, `None` where nothing is known.
    pub fn excess(&self) -> Option<f32> {
        match self {
            Reading::Unread | Reading::Deferred => None,
            Reading::Nothing => Some(0.0),
            Reading::Observed { excess, .. } => Some(*excess),
        }
    }
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
    pub reading: Reading,
}

impl LineReading {
    /// One line of prose for a surface: `3d 4h, past the 24h setpoint`,
    /// `nothing waiting`, `store unreadable`, `not read here`. Never a
    /// prompt — every caller is a surface the owner reads.
    pub fn summary(&self) -> String {
        match &self.reading {
            Reading::Unread => "store unreadable".to_string(),
            Reading::Deferred => "not read in a run; `mecha charter` reads it".to_string(),
            // The same state reads differently by store: the corpus kind's
            // "nothing" is no run in the window, not an empty queue.
            Reading::Nothing => match self.kind {
                SensorKind::InterventionRate => "no runs recorded yet".to_string(),
                _ => "nothing waiting".to_string(),
            },
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
        v
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
    /// The share of runs in the window the owner stepped into.
    Share(f64),
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
fn age_secs(stamp: &str, now: DateTime<Utc>) -> Option<u64> {
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
pub fn read_line(
    line: &CharterLine,
    backlog: &Backlog,
    corpus: &CorpusRate,
    now: DateTime<Utc>,
) -> Option<LineReading> {
    let sensor = line.sensor.as_ref()?;
    let reading = match sensor.kind {
        SensorKind::OutboxWaiting => match backlog.outbox.as_ref() {
            None => Reading::Unread,
            Some(depth) => observed(Observed::Count(depth.waiting as u64), sensor.setpoint),
        },
        SensorKind::OutboxAge => age_reading(backlog.outbox.as_ref(), sensor.setpoint, now),
        SensorKind::QuestionLatency => {
            age_reading(backlog.questions.as_ref(), sensor.setpoint, now)
        }
        SensorKind::RequestClosure => age_reading(backlog.frontdoor.as_ref(), sensor.setpoint, now),
        SensorKind::InterventionRate => match corpus {
            CorpusRate::NotScanned => Reading::Deferred,
            CorpusRate::Unreadable => Reading::Unread,
            CorpusRate::Empty => Reading::Nothing,
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
        ) | (_, Reading::Unread | Reading::Deferred | Reading::Nothing)
    ));
    Some(LineReading {
        line: line.id.clone(),
        kind: sensor.kind,
        setpoint: sensor.setpoint_text.clone(),
        reading,
    })
}

/// Every sensored line, in charter order. Empty for a charter with no
/// sensor, which a caller reports as *having none* rather than as reading
/// nothing (`Charter::has_sensors`).
pub fn read_lines(
    charter: &Charter,
    backlog: &Backlog,
    corpus: &CorpusRate,
    now: DateTime<Utc>,
) -> Vec<LineReading> {
    charter
        .lines()
        .iter()
        .filter_map(|l| read_line(l, backlog, corpus, now))
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
    use crate::runlog::{Corpus, Scan};
    let Ok(home) = crate::work::mecha_home() else {
        return CorpusRate::Unreadable;
    };
    let dir = home.join("sessions");
    if !dir.is_dir() {
        // A machine that has never recorded a run has nothing to have
        // stepped into — empty, not unreadable, on `Backlog::read`'s rule
        // for a store that has never existed.
        return CorpusRate::Empty;
    }
    match Corpus::scan(
        &dir,
        &Scan {
            max_sessions: Some(crate::doctor::RUNS_WINDOW),
            since: None,
            workspace: None,
            kind: None,
            include_tests: false,
            include_experiments: crate::experiment::in_experiment_home(),
        },
    ) {
        Ok(corpus) => match corpus.intervention_rate() {
            Some(rate) => CorpusRate::Share(rate),
            None => CorpusRate::Empty,
        },
        Err(_) => CorpusRate::Unreadable,
    }
}

/// Read the whole charter against the live stores, for a surface the owner
/// reads: the backlog's three stores, and the corpus only when a line asks
/// for it. Not for a run — [`crate::homeostat::Homeostat::at_start`] reads
/// the backlog it already holds and defers the corpus kind.
pub fn read_charter(charter: &Charter, now: DateTime<Utc>) -> Vec<LineReading> {
    let backlog = Backlog::read();
    let corpus = if wants_corpus(charter) {
        corpus_rate()
    } else {
        CorpusRate::NotScanned
    };
    read_lines(charter, &backlog, &corpus, now)
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
        let r = read_lines(&c, &backlog, &CorpusRate::NotScanned, now());
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
        let r = read_lines(&c, &backlog, &CorpusRate::NotScanned, now());
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
        let r = read_lines(&c, &backlog, &CorpusRate::NotScanned, now());
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
        let deferred = read_lines(&c, &backlog, &CorpusRate::NotScanned, now());
        assert_eq!(deferred[0].reading, Reading::Deferred);
        assert_eq!(deferred[0].reading.over(), None);

        let read = read_lines(&c, &backlog, &CorpusRate::Share(0.3), now());
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

        let empty = read_lines(&c, &backlog, &CorpusRate::Empty, now());
        assert_eq!(empty[0].reading, Reading::Nothing);
        assert_eq!(empty[0].summary(), "no runs recorded yet");
        let unreadable = read_lines(&c, &backlog, &CorpusRate::Unreadable, now());
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
        let r = read_lines(&c, &Backlog::default(), &CorpusRate::NotScanned, now());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].line, "age");
        let none = charter(vec![RawLine {
            id: "plain".into(),
            text: "no sensor".into(),
            sensor: None,
        }]);
        assert!(read_lines(&none, &Backlog::default(), &CorpusRate::NotScanned, now()).is_empty());
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
        let readings = read_lines(&c, &backlog, &CorpusRate::NotScanned, now());
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
}
