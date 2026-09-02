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
    /// party's expectation: the standing level, folded from how long the
    /// oldest recorded commitment in [`backlog`](Self::backlog)'s stores
    /// has waited and how much room this run had to act on it, scaled down
    /// by what this run itself cleared (`guilt::with_delta`) and never up
    /// by what it added. See [`crate::guilt`] for the
    /// formula and, importantly, for what this is *not* used for yet:
    /// **nothing consumes this today.** It is recorded so the corpus exists
    /// before anything is built on it, on `runlog`'s own rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticipated_guilt: Option<f32>,
}

impl Homeostat {
    /// Sample the conditions at the start of a run.
    pub fn at_start() -> Homeostat {
        Homeostat {
            load_avg_1m: load_avg_1m(),
            mem_available_kb: mem_available_kb(),
            backlog: Some(Backlog::read()),
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
            let level =
                crate::guilt::anticipated_guilt(before, self.peak_context_pressure, Utc::now());
            let delta = Backlog::delta(before, &Backlog::read());
            // The level is what the run inherited; the delta is what it did
            // about it, and only relief moves the reading — the comment above
            // is still the rule, and `guilt::with_delta` keeps it: a run that
            // added to the queue reads the level it inherited, not a
            // maximum for doing its job.
            self.anticipated_guilt =
                crate::guilt::with_delta(level, delta.net(), crate::guilt::waiting(before));
            self.backlog_delta = Some(delta);
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
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Homeostat>(&json).unwrap(), h);

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
