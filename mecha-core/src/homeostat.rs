//! The conditions a run happened under.
//!
//! Every counter on [`RunStats`](crate::session::RunStats) says what a run
//! *did*. Nothing said what it was working *against* — whether the context was
//! nearly full, whether the machine was loaded, how much was already waiting on
//! the owner. `docs/GOAL-SYSTEM-DESIGN.md` §4 is the argument; the short form
//! is that an outcome is not interpretable without the state it happened in.
//! A run that failed under a saturated machine and one that failed on an idle
//! one are the same row today, and they are not the same event: appraisal
//! separates *regret* from *disappointment* on exactly whether an alternative
//! existed.
//!
//! **Read-only.** Nothing here changes what a run does. It is recorded so that
//! later rungs — predictive compaction, the diagnostician's brief, the
//! appraiser — have a series to reason from, and so that the corpus exists
//! before anything is built on it.
//!
//! ## Three rules it inherits
//!
//! - **Opt-in, never automatic.** It rides on [`RunContext`] the way
//!   cancellation does. Sampling costs directory scans, and more importantly
//!   `mecha eval` and the replay probes must not read *live* machine state:
//!   a scorecard that varies with how busy the box was is not a scorecard, and
//!   an arm that samples today's backlog is measuring the afternoon rather than
//!   the change. Anything reconstructing a run reads the recorded snapshot.
//! - **Absent is not zero.** Every field is `Option` and `None` means the
//!   sensor could not be read, never that its value was zero — the rule
//!   [`crate::backlog`] states at length and for the same reason.
//! - **It never reaches the system prompt.** Render order is tools → system →
//!   messages with the cache breakpoint on the last system block, so a
//!   per-turn value there would re-pay the whole prefix, tools included, every
//!   request. Whatever eventually shows this to a model puts it in the turn
//!   tail or in a tool result.
//!
//! **Deliberately not sampled yet: llama-server's `/slots`.** It is the best
//! load signal available — occupancy directly rather than by proxy — but it is
//! an HTTP call, and nothing reads it yet. A sensor with no consumer should not
//! put a request in the path of every run's start; it goes in beside whatever
//! first needs it.
//!
//! [`RunContext`]: crate::agent::RunContext

use crate::backlog::{Backlog, BacklogDelta};
use crate::pressure::ContextTracker;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// A run's conditions: sampled at start, completed at end.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Homeostat {
    /// One-minute load average.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_avg_1m: Option<f32>,
    /// `MemAvailable`, in kB. On unified-memory hardware this is the *only*
    /// memory sensor — `nvidia-smi` reports `[N/A]` for GPU memory on GB10,
    /// because there is one pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_available_kb: Option<u64>,
    /// What was waiting on the owner when the run began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog: Option<Backlog>,
    /// What this run added to that, or took off it. See [`BacklogDelta`] —
    /// the level alone cannot separate a run's own output from what it
    /// inherited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog_delta: Option<BacklogDelta>,
    /// The largest prompt this run actually sent.
    ///
    /// The **peak**, not the last and not the total. The total is what
    /// `RunOutcome::usage` already reports and it is the wrong number for this
    /// — it sums every prompt the run ever sent, so a long conversation shows
    /// a figure several times the window and reads as impossible pressure. The
    /// last is the wrong number too: a run that spent twenty turns at the
    /// threshold and then compacted would record the small number, which is
    /// the one moment it was not under pressure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_prompt_tokens: Option<u64>,
    /// That peak as a share of the context window, when the window is known.
    ///
    /// `None` where it is not — the fraction is unknowable rather than zero,
    /// and a provider with no declared `context_window` is common enough that
    /// recording 0.0 there would put a floor of healthy-looking rows under
    /// every later reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_context_pressure: Option<f32>,
    /// A harness-computed proxy for anticipated guilt
    /// (`docs/GOAL-SYSTEM-DESIGN.md` §7.4) — predicted error against another
    /// party's expectation: the standing **level**, folded from how long the
    /// oldest recorded commitment in [`backlog`](Self::backlog)'s stores
    /// has waited and how much room this run had to act on it. What this
    /// run did about it is the next field, deliberately not folded in here:
    /// `Corpus::mean_anticipated_guilt` averages this across every row the
    /// store holds, and `anticipated_guilt`'s own doc chose `None` over a
    /// differently-computed number precisely so that mean stays one
    /// quantity — writing the relief-scaled reading into the same field
    /// blended two formulas with nothing marking which (found on review).
    /// See [`crate::guilt`] for the formula and, importantly, for what this
    /// is *not* used for yet: only the diagnostician's brief reads it. It is
    /// recorded so the corpus exists before anything is built on it, on
    /// `runlog`'s own rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticipated_guilt: Option<f32>,
    /// The level above, scaled down by the owner-facing share of the
    /// inherited backlog this run cleared (`guilt::with_backlogs`) — the
    /// run's *act* on the situation, in its own field so the level stays
    /// comparable across every row. `None` wherever the level is, or where
    /// the delta could not be read. Nothing consumes this yet either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guilt_after_relief: Option<f32>,
    /// Each sensored charter line, read against its store as the run began
    /// (`docs/GOAL-SYSTEM-DESIGN.md` §11.1's readings; [`crate::reading`]):
    /// the line-specific form of the guilt above, one reading per line
    /// instead of one number over three stores. Taken off the inherited
    /// backlog for the same reason `anticipated_guilt` is — what waited on
    /// the owner as the run began, not what the run left. The corpus kind
    /// (`intervention_rate`) reads `Deferred` here; the surfaces read it.
    ///
    /// `None` when no reading was taken — a row from before the field, or a
    /// charter that did not load — and `Some([])` when the charter carries
    /// no sensor: absent and empty are different facts, on
    /// `RunStats::delivered`'s shape. A row whose readings this binary
    /// cannot parse (a kind or state a later one wrote) loads as `None`
    /// rather than failing the record: a closed enum written to an
    /// append-only store is a wire format.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::reading::lenient"
    )]
    pub charter: Option<Vec<crate::reading::LineReading>>,
}

impl Homeostat {
    /// Sample the conditions at the start of a run.
    pub fn at_start() -> Homeostat {
        let backlog = Backlog::read();
        // The charter is loaded here rather than handed in: it is global
        // and read-only by construction (`charter.rs`), exactly as the
        // backlog's stores are, and the reading is a fact about the machine
        // — what waits on the owner against the owner's own numbers — that
        // holds whether or not this run's prompt carried the charter. A
        // charter that does not load records `None`: unknown, not
        // sensorless.
        let charter = crate::charter::Charter::default_path()
            .ok()
            .and_then(|p| crate::charter::Charter::load(&p).ok())
            .map(|c| {
                crate::reading::read_lines(
                    &c,
                    &backlog,
                    &crate::reading::CorpusRate::NotScanned,
                    Utc::now(),
                )
            });
        Homeostat {
            load_avg_1m: load_avg_1m(),
            mem_available_kb: mem_available_kb(),
            backlog: Some(backlog),
            charter,
            ..Homeostat::default()
        }
    }

    /// Complete the snapshot at the end of a run: difference the backlog, and
    /// take the context pressure off the run's own size series.
    ///
    /// The tracker is the source rather than `RunOutcome::usage` for the
    /// reason `peak_prompt_tokens` gives — the usage total is a sum across
    /// turns, and the quantity wanted is a maximum over them. It is passed in
    /// rather than read from anywhere, because the series exists only in the
    /// loop's memory and is deliberately never stored: what is worth keeping
    /// is the one number below, not a per-turn trace of every run.
    pub fn finish(mut self, pressure: &ContextTracker, window: Option<u64>) -> Homeostat {
        // Zero means the run never got a response — no request was ever
        // priced — which is an absence and not a measurement of nought.
        self.peak_prompt_tokens = (pressure.peak_tokens() > 0).then(|| pressure.peak_tokens());
        self.peak_context_pressure = pressure.peak_pressure(window);
        if let Some(before) = &self.backlog {
            // Guilt is computed from what this run *inherited*, not what it
            // leaves behind — the same distinction `backlog_delta` already
            // makes ("the level alone cannot separate a run's own output
            // from what it inherited"). Reading it off `after` instead would
            // score a trigger that staged three replies overnight as
            // maximally guilty for doing exactly its job: those drafts are
            // seconds old and this run's own output, not neglected debt.
            let after = Backlog::read();
            let now = Utc::now();
            // The level is what the run inherited, and it keeps its own
            // field so the corpus mean over it stays one quantity. The
            // delta is what the run did about it, and only relief moves
            // the second reading — the comment above is still the rule,
            // and `guilt::with_backlogs` keeps it: a run that added to the
            // queue reads the level it inherited, not a maximum for doing
            // its job, and relief is the owner-facing share of what was
            // waiting (one seam derives both numbers from this same pair of
            // reads — found on review, when the numerator spanned five
            // stores and the denominator three).
            let fold = crate::guilt::with_backlogs(before, &after, self.peak_context_pressure, now);
            self.anticipated_guilt = fold.level;
            self.guilt_after_relief = fold.after_relief;
            self.backlog_delta = Some(fold.delta);
        }
        self
    }
}

fn load_avg_1m() -> Option<f32> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn mem_available_kb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded snapshot has to survive a round trip through a session
    /// file, because replay reads it back rather than re-sampling — a run
    /// reconstructed against today's machine state is measuring the afternoon.
    #[test]
    fn a_snapshot_round_trips_and_an_older_record_without_one_still_loads() {
        let h = Homeostat {
            load_avg_1m: Some(0.56),
            mem_available_kb: Some(21_000_000),
            backlog: Some(Backlog::default()),
            backlog_delta: Some(BacklogDelta::default()),
            peak_prompt_tokens: Some(18_008),
            peak_context_pressure: Some(0.0687),
            anticipated_guilt: Some(0.0),
            guilt_after_relief: Some(0.0),
            charter: Some(vec![crate::reading::LineReading {
                line: "waits".into(),
                kind: crate::charter::SensorKind::OutboxAge,
                setpoint: "24h".into(),
                reading: crate::reading::Reading::Nothing,
            }]),
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Homeostat>(&json).unwrap(), h);

        // A reading this binary cannot parse — a kind a later one added —
        // costs the readings, never the row: the rest of the snapshot loads
        // and `charter` reads as unknown.
        let later = json.replace("\"outbox_age\"", "\"board_overdue\"");
        assert_ne!(later, json);
        let loaded: Homeostat = serde_json::from_str(&later).unwrap();
        assert_eq!(loaded.charter, None);
        assert_eq!(loaded.anticipated_guilt, Some(0.0));

        // Every field defaults, so a record written before this existed loads
        // as "nothing was sampled" rather than failing.
        let empty: Homeostat = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Homeostat::default());
    }

    /// The peak, and the two ways it can be absent.
    #[test]
    fn the_recorded_pressure_is_the_runs_high_water_mark() {
        let mut pressure = ContextTracker::new();
        pressure.observe(4_000, 4_000);
        pressure.observe(30_000, 30_000);
        // A compaction brought it back down. The row must still say the run
        // reached 30,000 — that is the pressure it ran under, and the last
        // reading is the one moment it was not under it.
        pressure.observe(6_000, 6_000);

        let h = Homeostat::default().finish(&pressure, Some(60_000));
        assert_eq!(h.peak_prompt_tokens, Some(30_000));
        assert_eq!(h.peak_context_pressure, Some(0.5));

        // No declared window: the count is still a fact, the fraction is not
        // knowable, and neither is reported as zero.
        let no_window = Homeostat::default().finish(&pressure, None);
        assert_eq!(no_window.peak_prompt_tokens, Some(30_000));
        assert_eq!(no_window.peak_context_pressure, None);

        // A run that never got a response priced nothing at all.
        let never = Homeostat::default().finish(&ContextTracker::new(), Some(60_000));
        assert_eq!(never.peak_prompt_tokens, None, "absent, not zero");
        assert_eq!(never.peak_context_pressure, None);
    }

    /// The sensors degrade rather than panic where /proc is absent or shaped
    /// differently. On this machine they read; the assertion that matters is
    /// that a miss is `None`.
    #[test]
    fn a_sensor_that_cannot_be_read_is_absent_rather_than_zero() {
        if let Some(load) = load_avg_1m() {
            assert!(load >= 0.0);
        }
        if let Some(kb) = mem_available_kb() {
            assert!(kb > 0, "MemAvailable parsed as zero would be a parse bug");
        }
    }
}
