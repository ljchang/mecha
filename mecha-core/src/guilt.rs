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
//! expectation, so this is a deliberately simple first cut over three
//! directly-sensed values — nothing here is extrapolated or a growth rate,
//! the same discipline §4.4 states for predictive compaction.

use crate::backlog::{Backlog, Depth};
use chrono::{DateTime, Utc};

/// How many recorded commitments make the count term treat "more" as no
/// worse than this. A single item is not yet "piling up" — the term is zero
/// at one and ramps linearly from there — so a lone fresh commitment cannot
/// be read as several stacking up; it saturates at three, past which a
/// fourth waiting item does not make a run four times as concerning.
const SATURATES_AT_COUNT: usize = 3;

/// How long the oldest recorded commitment can wait before the age term
/// treats it as maximally so.
const SATURATES_AT_HOURS: f64 = 24.0;

/// A magnitude in `[0, 1]`, combining three signals as a logical OR —
/// `1 - (1-a)(1-b)(1-c)` — rather than an average or a product:
///
/// - **age** — how long the oldest recorded commitment has sat unresolved,
///   saturating at [`SATURATES_AT_HOURS`].
/// - **count** — how many are recorded as waiting at all, zero at a single
///   item and saturating at [`SATURATES_AT_COUNT`].
/// - **pressure** — the run's own
///   [`crate::homeostat::Homeostat::peak_context_pressure`], a proxy for how
///   much room the run actually had to act on any of it.
///
/// **This is deliberately not an estimate of a true probability.** It is
/// three independent alarms, any one of which is treated as sufficient to
/// raise concern on its own — a run that ran out of room at 100% pressure
/// reads as maximally concerning even against a commitment recorded an hour
/// ago, and that is intentional rather than a slip: no term may be argued
/// back down by the others being low, which is the "may only narrow, never
/// loosen" shape §7.3 gives affect generally, applied here to three inputs
/// instead of one. A future consumer that wants "old *and* under pressure is
/// worse than either alone" is a different, stricter function than this one,
/// and should replace it deliberately rather than by way of this comment.
///
/// **Returns `None` unless all three stores were read.** A partial reading —
/// two stores readable and one not — must not collapse into a number that
/// looks exactly like "nothing is owed"; that is the same reasoning
/// [`crate::backlog::Waiting`] states for why a backlog total is reported
/// beside how much of it could not be read rather than silently as a lower
/// bound. The same applies to a commitment counted but whose timestamp could
/// not be parsed: `waiting > 0` with no age reachable is unknown, not zero.
pub fn anticipated_guilt(
    backlog: &Backlog,
    peak_context_pressure: Option<f32>,
    now: DateTime<Utc>,
) -> Option<f32> {
    let depths: [&Depth; 3] = [
        backlog.outbox.as_ref()?,
        backlog.questions.as_ref()?,
        backlog.frontdoor.as_ref()?,
    ];

    let mut waiting = 0usize;
    let mut oldest_hours: Option<f64> = None;
    for depth in depths {
        waiting += depth.waiting;
        if let Some(stamp) = &depth.oldest {
            if let Some(hours) = hours_since(stamp, now) {
                oldest_hours = Some(oldest_hours.map_or(hours, |h: f64| h.max(hours)));
            }
        }
    }
    if waiting == 0 {
        // Genuinely nothing recorded as owed — a real zero, not an absence.
        return Some(0.0);
    }
    // Something is recorded as waiting, but nothing readable said how long —
    // an age-blind reading would silently score it as fresh, which is a
    // guess dressed as a measurement.
    let oldest_hours = oldest_hours?;

    let age = (oldest_hours / SATURATES_AT_HOURS).clamp(0.0, 1.0) as f32;
    // Zero at one item — a single fresh commitment is not "several piling
    // up" — ramping linearly from two toward the saturation count.
    let count =
        ((waiting.saturating_sub(1)) as f32 / (SATURATES_AT_COUNT - 1) as f32).clamp(0.0, 1.0);
    let pressure = peak_context_pressure.unwrap_or(0.0).clamp(0.0, 1.0);
    let combined = 1.0 - (1.0 - age) * (1.0 - count) * (1.0 - pressure);
    Some(combined.clamp(0.0, 1.0))
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

    /// A fully readable backlog with nothing else set: all three depths
    /// present and empty. This is the "genuinely nothing waiting" fixture
    /// every positive test below starts from and overrides one field of,
    /// rather than the bare `Backlog::default()` — whose fields default to
    /// `None`, i.e. *unreadable*, which is a different fact entirely.
    fn readable_and_empty() -> Backlog {
        Backlog {
            outbox: Some(Depth::default()),
            questions: Some(Depth::default()),
            frontdoor: Some(Depth::default()),
            ..Backlog::default()
        }
    }

    #[test]
    fn every_store_unreadable_is_unknown_rather_than_zero() {
        let backlog = Backlog::default();
        assert_eq!(anticipated_guilt(&backlog, Some(0.5), Utc::now()), None);
    }

    #[test]
    fn one_unreadable_store_beside_two_empty_ones_is_still_unknown() {
        // The partial-read case: outbox and questions came back readable and
        // empty, front-door did not come back at all. Reporting `Some(0.0)`
        // here would say "nothing is owed" about a store this never actually
        // saw.
        let backlog = Backlog {
            outbox: Some(Depth::default()),
            questions: Some(Depth::default()),
            frontdoor: None,
            ..Backlog::default()
        };
        assert_eq!(anticipated_guilt(&backlog, Some(0.9), Utc::now()), None);
    }

    #[test]
    fn nothing_waiting_is_a_real_zero() {
        let backlog = readable_and_empty();
        assert_eq!(
            anticipated_guilt(&backlog, Some(0.9), Utc::now()),
            Some(0.0)
        );
    }

    #[test]
    fn a_fresh_lone_commitment_under_no_pressure_reads_near_zero() {
        let now = Utc::now();
        let backlog = Backlog {
            outbox: Some(depth(1, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        assert!(g < 0.1, "{g}");
    }

    #[test]
    fn a_day_old_commitment_saturates_the_age_term() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(48);
        let backlog = Backlog {
            questions: Some(depth(1, Some(&old.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g = anticipated_guilt(&backlog, None, now).unwrap();
        assert!((g - 1.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn pressure_alone_can_saturate_the_reading_by_design() {
        let now = Utc::now();
        let recent = now - chrono::Duration::hours(1);
        let backlog = Backlog {
            frontdoor: Some(depth(1, Some(&recent.to_rfc3339()))),
            ..readable_and_empty()
        };
        let low_pressure = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        let high_pressure = anticipated_guilt(&backlog, Some(1.0), now).unwrap();
        assert!(
            high_pressure > low_pressure,
            "{low_pressure} vs {high_pressure}"
        );
        // Full pressure alone still cannot be argued down by having barely
        // any age at all — that is the OR shape working as documented, not
        // an accident of the arithmetic.
        assert!((high_pressure - 1.0).abs() < 1e-6, "{high_pressure}");
    }

    #[test]
    fn several_waiting_items_saturate_the_count_term_even_when_fresh() {
        let now = Utc::now();
        let one = Backlog {
            outbox: Some(depth(1, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        let several = Backlog {
            outbox: Some(depth(SATURATES_AT_COUNT, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g_one = anticipated_guilt(&one, Some(0.0), now).unwrap();
        let g_several = anticipated_guilt(&several, Some(0.0), now).unwrap();
        assert!(g_several > g_one, "{g_one} vs {g_several}");
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
            ..readable_and_empty()
        };
        let g = anticipated_guilt(&backlog, None, now).unwrap();
        // 30h saturates the 24h age term regardless of the 1h row beside it.
        assert!((g - 1.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn a_count_with_no_parseable_age_is_unknown_rather_than_fresh() {
        let backlog = Backlog {
            outbox: Some(depth(1, Some("not-a-timestamp"))),
            ..readable_and_empty()
        };
        // Scoring this as age-zero would understate a real commitment this
        // sensor simply failed to date.
        assert_eq!(anticipated_guilt(&backlog, None, Utc::now()), None);
    }
}
