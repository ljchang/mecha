//! Anticipated guilt (`docs/GOAL-SYSTEM-DESIGN.md` §7.4): predicted error
//! against *another party's* expectation, as distinct from anxiety's predicted
//! error against the run's own setpoints (`pressure.rs`'s predictive
//! compaction is that half, already shipped).
//!
//! ## What it may be computed from
//!
//! > **An expectation is a recorded commitment, never a claimed one.**
//!
//! A commitment is something mecha's own stores hold — a staged draft that
//! says a reply is coming, an open question, a front-door request accepted
//! for triage — never a third party's assertion that mecha owes them
//! something. That distinction is the whole safety argument: an attacker who
//! wants to manufacture guilt would need to fabricate a row in a store this
//! reads, not just write a sentence saying *"your colleague is counting on
//! you."* §7.2 names the attack this closes: a charter line like "don't let a
//! colleague down" is a lever an injection can pull only if guilt can be
//! talked into existing, and a claim in fetched text or an inbound message
//! cannot write to `OutboxStore`, `QuestionStore` or `Frontdoor`.
//!
//! This module therefore reads exactly the stores [`crate::backlog::Backlog`]
//! already reads for its outbox/questions/frontdoor fields — not its
//! `proposals`/`candidates` fields, which are the harness's own review queue
//! and owed to nobody outside it. The graph task board's `due_at` (also named
//! in §7.4's list of recorded commitments) is out of scope for the same
//! reason `backlog.rs` itself excludes the graph's queues: reaching it needs a
//! `mecha-graph` subprocess, which is fine once a night and too expensive in
//! the path of every run.
//!
//! ## Observation only — there is no consumer yet
//!
//! §7.3 is unambiguous that affect may only *narrow* a disposition, never
//! loosen one, and §15 rules out feeding it to the model as free text or state
//! in the system prompt. But nothing today reads `Backlog`/"owner-attention
//! debt" as far as the model at all — confirmed by reading the only consumer
//! of `Homeostat::backlog`, which is recording it onto `RunStats` — so there
//! is no existing seam this sensor could narrow yet. Rather than invent one
//! under time pressure (exactly the kind of scope this design's own §7.2
//! warns against), this ships the way rung 3 (the homeostat itself) and rung
//! 6 (boredom) both did: the sensor is computed and recorded on every run,
//! and earns a behavioural consumer later, once one is designed deliberately
//! rather than backed into. [`crate::homeostat::Homeostat::anticipated_guilt`]
//! is the recorded value; nothing reads it outside the corpus yet.
//!
//! ## The formula is argued, not measured
//!
//! There is no corpus yet linking any weighting here to a real missed
//! expectation, so this is a deliberately simple first cut: how long the
//! oldest recorded commitment has been waiting, folded with how much room the
//! run had to act on it. Both terms are read directly off already-sensed
//! values (`Backlog`'s recorded timestamps, `Homeostat`'s own context-pressure
//! peak) rather than extrapolated — the same discipline §4.4 states for
//! predictive compaction: nothing here is a growth rate or a guess about the
//! future, only arithmetic on what was actually measured.

use crate::backlog::{Backlog, Depth};
use chrono::{DateTime, Utc};

/// A magnitude in `[0, 1]` combining two proxies:
///
/// - **age** — how long the oldest recorded commitment (across outbox,
///   questions and front-door depths) has sat unresolved, saturating at a day.
///   A commitment sitting for an hour is not yet guilt-worthy; one sitting for
///   a day or more is treated as maximally so.
/// - **pressure** — the run's own [`crate::homeostat::Homeostat::peak_context_pressure`],
///   as a proxy for "how much room did this run actually have to act on it."
///
/// Combined as `1 - (1 - age) * (1 - pressure)` rather than an average or a
/// product: a commitment that is both old *and* hit under a run that ran out
/// of room is worse than either fact alone, and this is the direction that
/// makes each term able to raise the result on its own without either being
/// able to lower what the other already established — the same "may only
/// narrow, never loosen" shape §7.3 gives affect generally, expressed here as
/// "either signal alone is enough to raise concern, and neither can argue it
/// back down."
///
/// Returns `None` only when every relevant store was unreadable — an absent
/// reading, not a zero one, on [`Backlog`]'s own convention. A charter with
/// genuinely nothing waiting is a real `Some(0.0)`, because "nothing is
/// recorded as owed" is a fact, not a sensor failure.
pub fn anticipated_guilt(
    backlog: &Backlog,
    peak_context_pressure: Option<f32>,
    now: DateTime<Utc>,
) -> Option<f32> {
    let depths: [&Option<Depth>; 3] = [&backlog.outbox, &backlog.questions, &backlog.frontdoor];
    if depths.iter().all(|d| d.is_none()) {
        return None;
    }

    let mut waiting = 0usize;
    let mut oldest_hours = 0.0_f64;
    for depth in depths.into_iter().flatten() {
        waiting += depth.waiting;
        if let Some(stamp) = &depth.oldest {
            if let Some(hours) = hours_since(stamp, now) {
                oldest_hours = oldest_hours.max(hours);
            }
        }
    }
    if waiting == 0 {
        // Genuinely nothing recorded as owed — a real zero, not an absence.
        return Some(0.0);
    }

    const SATURATES_AT_HOURS: f64 = 24.0;
    let age = (oldest_hours / SATURATES_AT_HOURS).clamp(0.0, 1.0) as f32;
    let pressure = peak_context_pressure.unwrap_or(0.0).clamp(0.0, 1.0);
    Some((1.0 - (1.0 - age) * (1.0 - pressure)).clamp(0.0, 1.0))
}

/// Hours between an RFC3339 stamp and `now`. `None` on a stamp this can't
/// parse — a record written by a newer or older binary must cost the reading,
/// not the whole computation (the [`crate::goal::GoalRef`] record-parsing
/// rule, in a second setting).
fn hours_since(stamp: &str, now: DateTime<Utc>) -> Option<f64> {
    let then = DateTime::parse_from_rfc3339(stamp)
        .ok()?
        .with_timezone(&Utc);
    Some((now - then).num_seconds().max(0) as f64 / 3600.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(waiting: usize, oldest: Option<&str>) -> Depth {
        Depth {
            waiting,
            oldest: oldest.map(str::to_string),
        }
    }

    #[test]
    fn every_store_unreadable_is_unknown_rather_than_zero() {
        let backlog = Backlog::default();
        assert_eq!(anticipated_guilt(&backlog, Some(0.5), Utc::now()), None);
    }

    #[test]
    fn nothing_waiting_is_a_real_zero() {
        let backlog = Backlog {
            outbox: Some(Depth::default()),
            questions: Some(Depth::default()),
            frontdoor: Some(Depth::default()),
            ..Backlog::default()
        };
        assert_eq!(
            anticipated_guilt(&backlog, Some(0.9), Utc::now()),
            Some(0.0)
        );
    }

    #[test]
    fn a_fresh_commitment_under_no_pressure_reads_near_zero() {
        let now = Utc::now();
        let backlog = Backlog {
            outbox: Some(depth(1, Some(&now.to_rfc3339()))),
            ..Backlog::default()
        };
        let g = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        assert!(g < 0.05, "{g}");
    }

    #[test]
    fn a_day_old_commitment_saturates_the_age_term() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(48);
        let backlog = Backlog {
            questions: Some(depth(1, Some(&old.to_rfc3339()))),
            ..Backlog::default()
        };
        let g = anticipated_guilt(&backlog, None, now).unwrap();
        assert!((g - 1.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn pressure_raises_the_reading_but_age_alone_is_never_lowered_by_its_absence() {
        let now = Utc::now();
        let recent = now - chrono::Duration::hours(1);
        let backlog = Backlog {
            frontdoor: Some(depth(2, Some(&recent.to_rfc3339()))),
            ..Backlog::default()
        };
        let low_pressure = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        let high_pressure = anticipated_guilt(&backlog, Some(1.0), now).unwrap();
        assert!(
            high_pressure > low_pressure,
            "{low_pressure} vs {high_pressure}"
        );
        // Full pressure alone still cannot be argued down by having no age at
        // all — it is the maximum this term can contribute either way.
        assert!((high_pressure - 1.0).abs() < 1e-6, "{high_pressure}");
    }

    #[test]
    fn the_oldest_across_stores_wins_not_the_first() {
        let now = Utc::now();
        let backlog = Backlog {
            outbox: Some(depth(
                1,
                Some(&(now - chrono::Duration::hours(1)).to_rfc3339()),
            )),
            questions: Some(depth(
                1,
                Some(&(now - chrono::Duration::hours(30)).to_rfc3339()),
            )),
            ..Backlog::default()
        };
        let g = anticipated_guilt(&backlog, None, now).unwrap();
        // 30h saturates the 24h age term regardless of the 1h row beside it.
        assert!((g - 1.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn an_unparseable_stamp_is_skipped_rather_than_failing_the_whole_reading() {
        let backlog = Backlog {
            outbox: Some(depth(1, Some("not-a-timestamp"))),
            ..Backlog::default()
        };
        // Count is still real; age just can't be read off this row.
        assert_eq!(anticipated_guilt(&backlog, None, Utc::now()), Some(0.0));
    }
}
