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
//! **Known imprecision, not fixed here**: `Backlog`'s frontdoor count
//! includes a request parked in `needs_info` — waiting on the *requester* to
//! answer, not on mecha — which this module has no way to exclude without
//! reading `Frontdoor` records directly instead of through `Backlog`'s
//! already-aggregated `Depth`, the one seam this module deliberately mirrors
//! rather than bypasses. So a request nobody but the requester owes anything
//! on today still counts toward `waiting`. Narrowing that needs a `Backlog`
//! (or a sibling) that can tell the two frontdoor states apart, which is a
//! `backlog.rs` change, not a `guilt.rs` one.
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

/// How many recorded commitments read as half of maximal on the count term.
/// A single item is not yet "piling up" — the term is zero at one — and it
/// climbs asymptotically from there: `0.5` at three, approaching but never
/// reaching `1.0`, so a fourth waiting item still reads worse than a third
/// without any count ever pinning the combined reading outright (see the
/// midpoint-not-ceiling note on [`AGE_HALF_AT_HOURS`]).
const COUNT_HALF_AT: usize = 3;

/// How long the oldest recorded commitment waits before the age term reads
/// half of maximal.
///
/// **A day was the wrong number, found by reading what the three stores this
/// reads actually hold rather than by guessing.** `questions.rs` exists
/// precisely because "the honest case is that nobody answers until
/// morning" — an overnight-parked question is the mechanism working, not
/// neglect. `read_frontdoor` (`backlog.rs`) counts every request with
/// `state != CLOSED`, `backlog.rs`'s own canonical fixture ages one 8–9
/// days, and a `needs_info` request (parked waiting on the *requester*, not
/// the owner) ages without bound while nothing is actually owed. At a
/// one-day horizon, a single week-old parked question saturates `age` to
/// `1.0` — and because the combination is an OR, that alone saturates the
/// *whole* reading regardless of count or pressure (`1 - (1-1)(1-c)(1-p) ==
/// 1`), which is a machine reading a constant `anticipated_guilt: 1.0`
/// forever. A corpus of a constant carries nothing, the same degenerate-label
/// shape rung 7's own measurement found the hard way.
///
/// **A midpoint, not a ceiling — and that arrived the same way, one week
/// later.** Widening the horizon to a week only moved the cliff: on
/// 2026-08-28 the first run recorded under this sensor read exactly `1.0`,
/// because the live outbox held drafts eight days old — past any horizon a
/// clamp-to-1.0 could reasonably use, since the owner sitting on a draft for
/// a week is this store's *normal* state, not an anomaly (doctor already
/// nags about it separately). A term that reaches exactly `1.0` multiplies
/// every other term's variance away in the OR, so the standing-debt terms
/// (age, count) now approach the maximum asymptotically — `h / (h +
/// AGE_HALF_AT_HOURS)`, `0.5` at one week, `0.67` at two — instead of
/// clamping. Ordering is preserved (older always reads worse), no term is
/// ever argued down by the others (still an OR), and the corpus keeps its
/// variance under exactly the backlog it actually has. Pressure keeps its
/// hard top: it is a fact about *this run*, not standing debt, so it cannot
/// pin the corpus across runs.
///
/// Rows recorded under the old clamped formula and rows recorded under this
/// one share the `anticipated_guilt` field with nothing marking which
/// produced them — the numeric cousin of "a closed enum written to an
/// append-only store is a wire format". Accepted deliberately: the sensor
/// was one day and one row old at the change, so the mixed span is a
/// handful of rows, and a version marker would outlive the problem it
/// dated. Worth remembering only if this formula moves again after the
/// corpus has real depth.
const AGE_HALF_AT_HOURS: f64 = 24.0 * 7.0;

/// A magnitude in `[0, 1]`, combining three signals as a logical OR —
/// `1 - (1-a)(1-b)(1-c)` — rather than an average or a product:
///
/// - **age** — how long the oldest recorded commitment has sat unresolved,
///   half of maximal at [`AGE_HALF_AT_HOURS`] and asymptotic above it.
/// - **count** — how many are recorded as waiting at all, zero at a single
///   item, half of maximal at [`COUNT_HALF_AT`], asymptotic above it.
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
/// instead of one. The standing-debt alarms are asymptotic rather than
/// clamped for the reason [`AGE_HALF_AT_HOURS`]'s doc carries: an alarm that
/// reaches exactly `1.0` does not merely stay raised, it erases the other
/// two from the reading entirely, which on the live store's normal backlog
/// made the whole sensor a constant. A future consumer that wants "old *and*
/// under pressure is worse than either alone" is a different, stricter
/// function than this one, and should replace it deliberately rather than by
/// way of this comment.
///
/// **Returns `None` unless all three stores were read.** A partial reading —
/// two stores readable and one not — must not collapse into a number that
/// looks exactly like "nothing is owed"; that is the same reasoning
/// [`crate::backlog::Waiting`] states for why a backlog total is reported
/// beside how much of it could not be read rather than silently as a lower
/// bound. The same applies to a commitment counted but whose timestamp could
/// not be parsed (`waiting > 0` with no age reachable is unknown, not zero),
/// and to pressure itself: `peak_context_pressure` is `None` on any provider
/// with no declared `context_window`, which [`crate::homeostat::Homeostat`]'s
/// own doc says must never be read as a measured `0.0` — silently treating
/// unknown pressure as none would put exactly that floor under this reading
/// instead, and would let a two-term computation average into
/// `Corpus::mean_anticipated_guilt` beside three-term ones with no mark
/// telling them apart. So this returns `None` whenever `waiting > 0` and
/// pressure is unsensed, even though age and count are both known — the same
/// "unknown beats a confident-looking guess" rule this function makes for
/// every other input, applied to the one it was tempted to treat as a
/// default instead.
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
    // Whether *any* non-empty depth's age could not be read — not whether
    // *every* depth's could. A store with three waiting items and a corrupt
    // stamp must not have its unknown age silently overridden by a second,
    // readable store's fresher one: the true oldest could easily be the
    // unreadable row, and reporting the readable row's age as the answer
    // understates it exactly as much as reading it as zero would.
    let mut age_unknown = false;
    for depth in depths {
        waiting += depth.waiting;
        if depth.waiting == 0 {
            continue;
        }
        match depth.oldest.as_deref().and_then(|s| hours_since(s, now)) {
            Some(hours) => oldest_hours = Some(oldest_hours.map_or(hours, |h: f64| h.max(hours))),
            None => age_unknown = true,
        }
    }
    if waiting == 0 {
        // Genuinely nothing recorded as owed — a real zero, not an absence.
        return Some(0.0);
    }
    // Something is recorded as waiting, but at least one non-empty depth's
    // age could not be read — an age-blind reading would silently score it
    // as fresh, or worse, let a sibling depth's real age stand in for it,
    // which is a guess dressed as a measurement either way.
    if age_unknown {
        return None;
    }
    let oldest_hours = oldest_hours?;

    // Asymptotic, never clamped — see AGE_HALF_AT_HOURS: a standing-debt
    // term that reaches exactly 1.0 erases the other terms from the OR.
    let age = (oldest_hours / (oldest_hours + AGE_HALF_AT_HOURS)) as f32;
    // Zero at one item — a single fresh commitment is not "several piling
    // up" — climbing toward (never to) 1.0, half of maximal at the midpoint.
    let above_one = waiting.saturating_sub(1) as f32;
    let count = above_one / (above_one + (COUNT_HALF_AT - 1) as f32);
    // Unknown, not a measured zero — see the doc comment above.
    let pressure = peak_context_pressure?.clamp(0.0, 1.0);
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
    fn a_week_old_commitment_reads_half_of_maximal_on_the_age_term() {
        let now = Utc::now();
        let old = now - chrono::Duration::hours(AGE_HALF_AT_HOURS.round() as i64);
        let backlog = Backlog {
            questions: Some(depth(1, Some(&old.to_rfc3339()))),
            ..readable_and_empty()
        };
        // Pressure known and zero, so age alone is what is being measured.
        let g = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        assert!((g - 0.5).abs() < 1e-3, "{g}");
    }

    /// The regression the midpoint exists for, taken from the live store on
    /// 2026-08-28: four drafts eight days old plus three questions read
    /// exactly `1.0` on the first run recorded under this sensor — the age
    /// term clamped, the OR erased count and pressure, and the corpus was a
    /// constant from its first row. Standing debt must *order* readings, not
    /// pin them: past-the-midpoint debt reads high but below `1.0`, still
    /// worsens as it ages, and still lets pressure move the reading.
    #[test]
    fn a_standing_week_old_backlog_does_not_pin_the_reading_at_a_constant() {
        let now = Utc::now();
        let eight_days = now - chrono::Duration::hours(24 * 8);
        let two_days = now - chrono::Duration::hours(48);
        let live_shape = Backlog {
            outbox: Some(depth(4, Some(&eight_days.to_rfc3339()))),
            questions: Some(depth(3, Some(&two_days.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g = anticipated_guilt(&live_shape, Some(0.06), now).unwrap();
        assert!(g > 0.5, "eight-day-old debt should still read high: {g}");
        assert!(g < 1.0 - 1e-3, "…but must not pin the reading: {g}");

        // Variance survives in both remaining inputs.
        let under_pressure = anticipated_guilt(&live_shape, Some(0.6), now).unwrap();
        assert!(under_pressure > g, "{g} vs {under_pressure}");
        let older = Backlog {
            outbox: Some(depth(
                4,
                Some(&(now - chrono::Duration::hours(24 * 16)).to_rfc3339()),
            )),
            questions: Some(depth(3, Some(&two_days.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g_older = anticipated_guilt(&older, Some(0.06), now).unwrap();
        assert!(
            g_older > g,
            "older debt must still read worse: {g} vs {g_older}"
        );
    }

    #[test]
    fn a_two_day_old_commitment_does_not_saturate_the_age_term() {
        // The gap a one-day horizon left open: `questions.rs` parks answers
        // overnight by design, and a two-day-old one is not neglect. This
        // must read as partial concern, not the maximum.
        let now = Utc::now();
        let two_days = now - chrono::Duration::hours(48);
        let backlog = Backlog {
            questions: Some(depth(1, Some(&two_days.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g = anticipated_guilt(&backlog, Some(0.0), now).unwrap();
        assert!(g > 0.0 && g < 0.5, "{g}");
    }

    #[test]
    fn unknown_pressure_is_unknown_not_a_measured_zero() {
        // A provider with no declared context_window reports `None` here —
        // not "definitely idle" — and this function must not quietly treat
        // it as the latter just because age and count are both known.
        let now = Utc::now();
        let old = now - chrono::Duration::hours(48);
        let backlog = Backlog {
            questions: Some(depth(1, Some(&old.to_rfc3339()))),
            ..readable_and_empty()
        };
        assert_eq!(anticipated_guilt(&backlog, None, now), None);
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
    fn several_waiting_items_raise_the_count_term_even_when_fresh() {
        let now = Utc::now();
        let one = Backlog {
            outbox: Some(depth(1, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        let several = Backlog {
            outbox: Some(depth(COUNT_HALF_AT, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        let g_one = anticipated_guilt(&one, Some(0.0), now).unwrap();
        let g_several = anticipated_guilt(&several, Some(0.0), now).unwrap();
        assert!(g_several > g_one, "{g_one} vs {g_several}");
    }

    #[test]
    fn the_oldest_across_stores_wins_not_the_first() {
        let now = Utc::now();
        let fresh_first = Backlog {
            outbox: Some(depth(
                1,
                Some(&(now - chrono::Duration::hours(1)).to_rfc3339()),
            )),
            questions: Some(depth(
                1,
                Some(
                    &(now - chrono::Duration::hours(AGE_HALF_AT_HOURS.round() as i64 * 2))
                        .to_rfc3339(),
                ),
            )),
            ..readable_and_empty()
        };
        let both_fresh = Backlog {
            outbox: Some(depth(
                1,
                Some(&(now - chrono::Duration::hours(1)).to_rfc3339()),
            )),
            questions: Some(depth(
                1,
                Some(&(now - chrono::Duration::hours(1)).to_rfc3339()),
            )),
            ..readable_and_empty()
        };
        // The 1h row in the first store must not stand in for the two-week
        // row behind it: the reading is driven by the oldest anywhere.
        let g_old_behind = anticipated_guilt(&fresh_first, Some(0.0), now).unwrap();
        let g_fresh = anticipated_guilt(&both_fresh, Some(0.0), now).unwrap();
        assert!(g_old_behind > g_fresh, "{g_fresh} vs {g_old_behind}");
        // Two midpoints old (age 2/3) OR two waiting items (count 1/3):
        // 1 - (1/3)(2/3) = 7/9. Pinned so the arithmetic stays honest.
        assert!((g_old_behind - 7.0 / 9.0).abs() < 1e-2, "{g_old_behind}");
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

    #[test]
    fn one_undated_store_is_unknown_even_when_a_sibling_store_is_dated() {
        // The gap the single-store version of this test above didn't cover:
        // a second, readable store's real age must not stand in for the
        // first store's unreadable one. The true oldest commitment could
        // easily be the one this can't date at all.
        let now = Utc::now();
        let backlog = Backlog {
            outbox: Some(depth(1, Some("not-a-timestamp"))),
            questions: Some(depth(1, Some(&now.to_rfc3339()))),
            ..readable_and_empty()
        };
        assert_eq!(anticipated_guilt(&backlog, Some(0.0), now), None);
    }
}
