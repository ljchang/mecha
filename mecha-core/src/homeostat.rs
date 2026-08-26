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

    /// Complete the snapshot at the end of a run, by differencing the backlog.
    ///
    /// **Context pressure is deliberately not here yet.** It wants the *last
    /// request's* prompt size, and `RunOutcome::usage` is the run's total —
    /// accumulated across turns, so its `total_input` is the sum of every
    /// prompt the run ever sent, a number that exceeds the window in any long
    /// conversation and would read as impossible pressure. The figure that is
    /// wanted lives in a loop local behind six exit points, and the thing that
    /// actually needs it is rung 5's in-run tracker, which keeps last turn's
    /// size in memory and records nothing. So it arrives with that, rather
    /// than as a field nothing sets.
    pub fn finish(mut self) -> Homeostat {
        if let Some(before) = &self.backlog {
            self.backlog_delta = Some(Backlog::delta(before, &Backlog::read()));
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
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Homeostat>(&json).unwrap(), h);

        // Every field defaults, so a record written before this existed loads
        // as "nothing was sampled" rather than failing.
        let empty: Homeostat = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Homeostat::default());
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
