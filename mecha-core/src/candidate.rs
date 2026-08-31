//! A proposed harness change, and the decision about it.
//!
//! This is the gate `docs/SELF-IMPROVEMENT-RESEARCH.md` §13.3 specifies, and
//! it is pure on purpose: the arms are run elsewhere, and what arrives here is
//! two sets of [`RunStats`] plus the prediction that was made before either
//! was measured. Getting this wrong is silent — a bad rule that scores well
//! ships and rides in every future prompt — so it is the part that gets unit
//! tests rather than a live trial.
//!
//! ## The shape
//!
//! A candidate carries a **falsifiable prediction** (AHE's decision
//! observability): the metric it claims to move and the direction. Without
//! one, a proposal cannot be refuted by the next measurement, and
//! "harness updating is not harness benefit" is what follows — agents
//! modifying themselves with no corresponding gain.
//!
//! ## Why paired, and why a holdout
//!
//! Episodes differ from each other far more than arms differ from each other,
//! so an unpaired comparison measures which episodes landed in which arm.
//! Pairing by episode removes that. And selecting among candidates on the same
//! episodes that justify the winner is a multiple-comparisons trap: the more
//! candidates, the better the winner looks and the less of it is real. So the
//! corpus is split deterministically, selection happens on one slice, and the
//! winner is confirmed on a slice never used for selection.
//!
//! ## Why counts rather than a significance test
//!
//! Deliberate. With a few dozen episodes the noise is the model's sampling,
//! not the measurement, and the answer to sampling noise is repetition
//! (`--runs k`, pass^k) rather than a p-value over one sample. A test here
//! would put a number on the wrong uncertainty and read as rigour. The raw
//! win/loss/tie counts are reported instead, so a human reading a proposal
//! sees what the decision was made from.

use crate::session::RunStats;
use std::collections::BTreeMap;

/// What a candidate claims it will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Runs that finished with their last tool call failed.
    EndedOnFailedCall,
    /// Share of attempted tool calls the environment refused.
    ToolErrorRate,
    /// Runs the harness cut short rather than the model finishing.
    CutShort,
    /// Summaries taken. Fewer is better only when the work is unchanged,
    /// which is what the work guardrail is for.
    Compactions,
    /// Turns spent.
    Turns,
    /// Arguments the model produced that did not parse.
    MalformedArgs,
}

impl Metric {
    /// Every metric a proposal may name.
    ///
    /// The list exists so the diagnostic brief and the diagnostic instruction
    /// cannot disagree about it. Until it did, `DIAGNOSE_INSTRUCTION` offered
    /// six metrics to predict while `Evidence::brief` reported values for
    /// three, and the nightly's two worst proposals were both on metrics whose
    /// value it had never been shown — one of them `cut_short`, on a corpus
    /// where `cut_short` was zero and the measurement could only tie.
    pub const ALL: [Metric; 6] = [
        Metric::EndedOnFailedCall,
        Metric::ToolErrorRate,
        Metric::CutShort,
        Metric::Compactions,
        Metric::Turns,
        Metric::MalformedArgs,
    ];

    /// The name the proposal block uses, which is the serde spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Metric::EndedOnFailedCall => "ended_on_failed_call",
            Metric::ToolErrorRate => "tool_error_rate",
            Metric::CutShort => "cut_short",
            Metric::Compactions => "compactions",
            Metric::Turns => "turns",
            Metric::MalformedArgs => "malformed_args",
        }
    }

    /// How much this episode can say about the metric, higher being more.
    ///
    /// **The priority for a prioritised replay draw, and it needs no new
    /// concept: it is the metric's own value on the recorded run.** Every
    /// metric here is a cost, so an episode already at zero has no room to
    /// improve — whatever the change does, that pair can only tie or worsen,
    /// and it costs a real model run per arm to learn that. An episode with a
    /// high recorded cost is the one that can discriminate.
    ///
    /// This is prioritised experience replay's shape with the sensor that
    /// exists today. PER samples by |TD error| because a surprising transition
    /// carries the most information; here the same argument is made with
    /// headroom, because the appraisal record that would supply a goal error
    /// is not built yet. When it is, |goal error| joins this rather than
    /// replacing it — a run can be uninformative about a metric and still be
    /// the most instructive thing that happened all week.
    ///
    /// **It is only ever a priority, never a score.** Drawing the *selection*
    /// slice this way is safe precisely because selection only picks; the
    /// holdout, drawn uniformly, is what confirms. See [`judge_drawn`].
    pub fn headroom(&self, recorded: &RunStats) -> f64 {
        self.of(recorded)
    }

    /// The metric's value for one run. Lower is better for every metric here,
    /// which is a deliberate constraint rather than a coincidence: a mixed
    /// polarity is the kind of thing that inverts a comparison silently, so
    /// anything worth predicting gets phrased as a cost.
    pub fn of(&self, s: &RunStats) -> f64 {
        match self {
            Metric::EndedOnFailedCall => f64::from(u8::from(s.ended_on_failed_call)),
            Metric::ToolErrorRate => {
                if s.tool_calls == 0 {
                    // No calls is no evidence, not a clean record. Neutral,
                    // so an episode that made no calls in either arm cannot
                    // be counted as a win by a change that suppressed work.
                    0.0
                } else {
                    f64::from(s.tool_errors) / f64::from(s.tool_calls)
                }
            }
            // The harness ending the run, not a person cancelling it — the
            // same predicate `doctor` reads. Counting `Interrupted` here made
            // a cancelled arm a loss on the metric it was predicting.
            Metric::CutShort => f64::from(u8::from(s.stop_cause.is_some_and(|c| c.cut_short()))),
            Metric::Compactions => f64::from(s.compactions),
            Metric::Turns => f64::from(s.turns),
            Metric::MalformedArgs => f64::from(s.malformed_tool_args),
        }
    }
}

/// The claim a candidate is judged against, made before the measurement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Prediction {
    pub metric: Metric,
    /// Free text from the diagnostician: what it thinks is wrong and why this
    /// change addresses it. Recorded for the human who reads the proposal —
    /// never parsed, and never consulted by the decision.
    pub rationale: String,
}

/// What kind of change this is, which decides how far it can get without a
/// person. See §13.2–13.3 of the research.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeClass {
    /// A reversible configuration value.
    Config,
    /// Text entering the system prompt.
    Prose,
    /// A new hook, subagent, trigger, eval case, tool surface, or source
    /// change. Always a human's call.
    Architecture,
    /// The interlock, the path jail, sandbox configuration, outbox routing.
    /// Human-gated, and the standing recommendation is that these are never
    /// proposed at all: a loop that can argue for widening its own
    /// confinement will eventually argue well, and the metric agrees with it
    /// — a run that can reach the network fails fewer calls.
    Security,
}

impl ChangeClass {
    /// Whether measurement alone can accept this class.
    fn auto_acceptable(&self) -> bool {
        matches!(self, ChangeClass::Config | ChangeClass::Prose)
    }
}

/// One episode measured in both arms. Paired by `episode`, which is a replay
/// corpus id — a session id, or an eval case id.
#[derive(Debug, Clone)]
pub struct Pair {
    pub episode: String,
    pub baseline: RunStats,
    pub candidate: RunStats,
}

/// How one slice of the corpus came out.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Tally {
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
}

impl Tally {
    pub fn total(&self) -> usize {
        self.wins + self.losses + self.ties
    }
    fn better(&self) -> bool {
        self.wins > self.losses
    }
    fn not_worse(&self) -> bool {
        self.wins >= self.losses
    }
}

/// What the gate decided, and why in words a human can check.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Disposition {
    /// Measurement carried it: nothing further needed.
    Accept,
    /// Measured well but the class requires a person, or the evidence is thin.
    Propose(String),
    /// Measured badly, or a guardrail moved.
    Reject(String),
}

/// The full result of grading a candidate, kept whole so a proposal records
/// what it was decided from rather than only the verdict.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Judgement {
    pub disposition: Disposition,
    pub selection: Tally,
    pub holdout: Tally,
    /// Tool calls attempted across each arm — the work guardrail. A change
    /// that improves its metric by attempting less has not improved anything.
    pub work_baseline: u64,
    pub work_candidate: u64,
}

/// Below this many paired episodes in a slice, a difference is not evidence.
///
/// Eight and four, which are small — the constraint is that a replay corpus
/// costs a real model run per episode per arm, so a floor set where the
/// statistics would like it is a floor that stops the loop running at all.
/// The holdout is doing the work that a larger sample would; these numbers
/// only stop a two-episode coincidence being called a result.
pub const MIN_SELECTION_PAIRS: usize = 8;
pub const MIN_HOLDOUT_PAIRS: usize = 4;

/// How many held-out episodes must have had **room to move** before the
/// holdout is allowed to confirm anything.
///
/// **An episode whose baseline cost is already zero cannot produce a win.** It
/// can still produce a loss, so an all-zero holdout is a real regression check
/// — but it is not confirmation, and `not_worse` cannot tell the two apart:
/// `0 wins, 0 losses, 4 ties` satisfies `wins >= losses` and reads as
/// "confirmed on unseen work".
///
/// That is not hypothetical here. The holdout is drawn *uniformly* on purpose,
/// and on 2026-08-31 the live corpus was 69% runs of two turns or fewer, with
/// four of the six metrics at zero across all 172 runs. A uniform draw of four
/// from that pool is usually four episodes that could not have moved whatever
/// was predicted. So the gate's strongest claim was the one its evidence was
/// least able to support.
///
/// Set equal to [`MIN_HOLDOUT_PAIRS`] rather than below it: a slice that
/// cannot confirm is not a smaller confirmation, and the disposition for
/// "nothing has confirmed this" already exists and is [`Disposition::Propose`]
/// — it reaches a person instead of being believed.
pub const MIN_INFORMATIVE_HOLDOUT: usize = MIN_HOLDOUT_PAIRS;

/// How far a metric the candidate did *not* predict may worsen before the
/// measured win is treated as bought rather than earned.
///
/// The work guardrail below counts tool calls, which catches a gain bought by
/// attempting less. It does not catch a gain bought by failing more: nothing
/// stopped a change that halved `turns` while doubling `tool_error_rate`, and
/// `turns` is currently the only metric in this corpus with real headroom, so
/// that is the trade the loop is most likely to be offered.
///
/// This is the regression-awareness that GRASP (arXiv:2605.29668) names as the
/// difference between a self-improvement loop that compounds and one that
/// accumulates: a candidate is accepted on one number and the others are never
/// looked at, so each accepted change silently pays for itself somewhere else.
/// A cliff rather than a ratchet, like [`WORK_FLOOR`] — some movement is noise.
pub const REGRESSION_CEILING: f64 = 1.25;

/// The smallest corpus a measurement can be drawn from.
///
/// Both slices come off the same pool, so a corpus below their sum cannot fill
/// them however it is split. **A necessary condition and not a sufficient
/// one** — these are recorded runs, and the eligible pool is the replayable
/// subset of them, which is smaller — and a caller must say which of the two
/// it is claiming when it reports the refusal.
///
/// It gates the *measurement*, never the diagnosis. A `Prose`, `Architecture`
/// or `Security` proposal is staged for a person and never touches a replay,
/// so a small corpus must not withhold those: found in review, where an early
/// return skipped the entire night and `--from-workspace` made a sub-floor
/// corpus easy to reach on purpose.
pub const MIN_MEASURABLE_RUNS: usize = MIN_SELECTION_PAIRS + MIN_HOLDOUT_PAIRS;

/// Whether a corpus of this many runs could fill both slices. See
/// [`MIN_MEASURABLE_RUNS`].
pub fn measurable(runs: usize) -> bool {
    runs >= MIN_MEASURABLE_RUNS
}

/// How far work may fall before a gain is treated as bought rather than
/// earned. Some drop is legitimate — a change that stops a redundant re-read
/// does less work and is better for it — so this is a cliff, not a ratchet.
pub const WORK_FLOOR: f64 = 0.75;

/// Split an episode into selection or holdout, deterministically.
///
/// By id hash rather than at random: the same corpus must split the same way
/// every time or a rerun silently grades a candidate against a different
/// holdout, and "confirmed on unseen episodes" stops meaning anything. Pure,
/// so the split is unit-testable.
pub fn is_holdout(episode: &str, holdout_in: u64) -> bool {
    // FNV-1a, written out rather than `DefaultHasher`. std explicitly does not
    // guarantee `DefaultHasher`'s algorithm across releases, so a toolchain
    // upgrade would re-partition selection and holdout with nothing visible
    // changing — and "confirmed on episodes it was never chosen on" would
    // quietly stop being true. The invariant this function exists for is
    // stability, so the hash has to be one this file owns.
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x100_0000_01b3;
    let mut h = OFFSET;
    for byte in episode.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(PRIME);
    }
    h.is_multiple_of(holdout_in)
}

/// Grade a candidate against its own prediction.
/// Reject an accepted candidate that paid for its win on a metric it never
/// predicted.
///
/// Applied only to an `Accept`, and only here rather than inside
/// [`judge_slices`], because it is the one thing that needs [`RunStats`]: the
/// generic gate sees one cost function and cannot ask about the metrics it was
/// not given, and `mecha eval --ab-config` grades cases rather than runs so it
/// has no `RunStats` to ask about.
///
/// **This is not a claim that the eval rig is untouched by this change as a
/// whole.** `MIN_INFORMATIVE_HOLDOUT` *is* inside [`judge_slices`], so eval's
/// A/B arm gained that check too — deliberately, since a held-out case with no
/// cost to lose confirms exactly as little there as here, but the two are
/// separate decisions and only this one was scoped to runs.
///
/// A cost appearing from *nothing* is treated differently from one that grew:
/// it **proposes** rather than rejects. `compactions`, `malformed_args` and
/// `ended_on_failed_call` are all zero across the live corpus, so "0 → 2" is
/// not noise on a large number — but neither is it always bad, and this
/// function cannot tell which. A `compact_at_tokens` low enough to compact is
/// a change whose whole purpose is to move `compactions` off zero; rejecting
/// it would make one of the four overridable knobs unusable by construction.
/// See the branch itself.
fn guard_regressions<'a>(
    j: Judgement,
    predicted: Metric,
    pairs: impl Iterator<Item = &'a Pair> + Clone,
) -> Judgement {
    if j.disposition != Disposition::Accept {
        return j;
    }
    // **Every metric is examined before anything is returned, because the two
    // outcomes are not equally serious and they were racing on array order.**
    // A `0 → nonzero` proposal returned immediately, so a genuine ratio breach
    // on a metric later in `Metric::ALL` was never reached: the reviewer got
    // the benign "a cost that was not there before, and only a person can tell
    // which" and never saw that something else had also doubled. `Compactions`
    // sits at index 3 and `MalformedArgs` at 5, so the pair that hides the
    // worse finding behind the milder one is reachable rather than theoretical.
    //
    // A `Reject` still returns as soon as it is found — it is the strongest
    // verdict available and nothing later can outrank it. Only the `Propose`
    // waits.
    let mut appeared: Option<String> = None;
    for metric in Metric::ALL {
        if metric == predicted {
            continue;
        }
        let total = |pick: fn(&Pair) -> &RunStats| -> f64 {
            pairs.clone().map(|p| metric.of(pick(p))).sum()
        };
        let (before, after) = (total(|p| &p.baseline), total(|p| &p.candidate));
        if before > 0.0 {
            if after > before * REGRESSION_CEILING {
                return Judgement {
                    disposition: Disposition::Reject(format!(
                        "predicted a lower {predicted:?} and got one, but {metric:?} rose from \
                         {before:.2} to {after:.2} across the same episodes: a win paid for on \
                         a metric nobody was watching is not a win"
                    )),
                    ..j
                };
            }
        } else if after > 0.0 && appeared.is_none() {
            // **A cost appearing from nothing reaches a person; it does not
            // auto-reject.** A first version rejected it outright, on the
            // reasoning that a failure mode that was not there before is not
            // noise on a large number. True, and it foreclosed the closed
            // override set's own purpose: `compactions` is zero across this
            // corpus, so *any* `compact_at_tokens` low enough to actually
            // compact scored a hard rejection on the metric that knob exists
            // to move. The guard would have made one of four knobs unusable
            // and said nothing about why.
            //
            // `Propose` is the honest disposition and the one this design
            // already has for it. Nothing here can tell "compaction started
            // happening, which is the point" from "malformed arguments
            // appeared, which is not" — and a reader can. So it never
            // auto-accepts, and it never silently refuses either.
            appeared = Some(format!(
                "predicted a lower {predicted:?} and got one, but {metric:?} rose from \
                 nothing to {after:.2} across the same episodes — a cost that was not there \
                 before. That is the intended effect for some changes and a regression for \
                 others, and only a person can tell which"
            ));
        }
    }
    match appeared {
        Some(why) => Judgement {
            disposition: Disposition::Propose(why),
            ..j
        },
        None => j,
    }
}

pub fn judge(
    class: ChangeClass,
    prediction: &Prediction,
    pairs: &[Pair],
    holdout_in: u64,
) -> Judgement {
    let metric = prediction.metric;
    let judged = judge_with(
        class,
        pairs,
        |p| {
            (
                p.episode.as_str(),
                metric.of(&p.baseline),
                metric.of(&p.candidate),
            )
        },
        |p| {
            (
                u64::from(p.baseline.tool_calls),
                u64::from(p.candidate.tool_calls),
            )
        },
        holdout_in,
    );
    guard_regressions(judged, metric, pairs.iter())
}

/// Judge two slices the caller drew: selection by priority, holdout uniformly.
///
/// The replay path's entry point. `judge` hash-partitions one pool and is
/// still right for `eval --ab-config`, where every case runs and the pool is
/// therefore already uniform. A replay corpus is *sampled*, and once it is
/// sampled by informativeness the partition inherits the bias — see
/// [`judge_slices`].
pub fn judge_drawn(
    class: ChangeClass,
    prediction: &Prediction,
    selection: &[Pair],
    holdout: &[Pair],
) -> Judgement {
    let metric = prediction.metric;
    let sel: Vec<&Pair> = selection.iter().collect();
    let hold: Vec<&Pair> = holdout.iter().collect();
    let judged = judge_slices(
        class,
        &sel,
        &hold,
        // Inline rather than bound: a named closure here cannot be inferred
        // as higher-ranked over the borrow, the same reason `judge` spells
        // these out at the call.
        |p| {
            (
                p.episode.as_str(),
                metric.of(&p.baseline),
                metric.of(&p.candidate),
            )
        },
        |p| {
            (
                u64::from(p.baseline.tool_calls),
                u64::from(p.candidate.tool_calls),
            )
        },
    );
    guard_regressions(judged, metric, selection.iter().chain(holdout.iter()))
}

/// The same gate over anything that can name an episode and produce a cost.
///
/// Two currencies grade a candidate here and they are not interchangeable.
/// Replayed sessions are scored on [`RunStats`] — did the *harness* go better
/// — while eval cases are scored on whether the case **passed**, which is the
/// content-sensitive arm a prose change needs, because replay holds tool
/// results fixed and cannot see a change in what the model actually said. One
/// gate, so the guardrails and the holdout cannot drift apart between them.
///
/// `cost` returns `(episode, baseline, candidate)` and lower must be better,
/// as in [`Metric`]. `work` returns the two arms' work volume for the Goodhart
/// guardrail.
pub fn judge_with<T>(
    class: ChangeClass,
    pairs: &[T],
    cost: impl for<'a> Fn(&'a T) -> (&'a str, f64, f64),
    work: impl Fn(&T) -> (u64, u64),
    holdout_in: u64,
) -> Judgement {
    let (holdout, selection): (Vec<&T>, Vec<&T>) = pairs
        .iter()
        .partition(|p| is_holdout(cost(p).0, holdout_in));
    judge_slices(class, &selection, &holdout, cost, work)
}

/// The gate over two slices the caller drew itself.
///
/// **Extracted because prioritising a corpus prioritises both halves of a
/// partition of it.** [`is_holdout`] splits one pool, which is right when the
/// pool was gathered uniformly — every eval case runs, so `--ab-config` still
/// uses it. It is wrong the moment the pool is drawn by informativeness:
/// hashing a biased pool yields two biased slices, and the holdout stops being
/// the thing that corrects the selection's bias. Prioritised experience replay
/// has the same problem and answers it with importance weights; here the
/// answer is that the two slices are **drawn separately** — the holdout
/// uniformly, the selection by [`Metric::headroom`] — and this function's job
/// is to score whatever it is handed rather than to decide what goes where.
pub fn judge_slices<T>(
    class: ChangeClass,
    selection: &[&T],
    holdout: &[&T],
    cost: impl for<'a> Fn(&'a T) -> (&'a str, f64, f64),
    work: impl Fn(&T) -> (u64, u64),
) -> Judgement {
    let (selection, holdout) = (selection.to_vec(), holdout.to_vec());
    let tally = |slice: &[&T]| {
        let mut t = Tally::default();
        for p in slice {
            let (_, before, after) = cost(p);
            // Every metric is a cost, so down is a win.
            match after.partial_cmp(&before) {
                Some(std::cmp::Ordering::Less) => t.wins += 1,
                Some(std::cmp::Ordering::Greater) => t.losses += 1,
                _ => t.ties += 1,
            }
        }
        t
    };
    let sel = tally(&selection);
    let hold = tally(&holdout);

    let sum = |slice: &[&T], pick: fn((u64, u64)) -> u64| -> u64 {
        slice.iter().map(|p| pick(work(p))).sum()
    };
    let work_baseline = sum(&selection, |(b, _)| b) + sum(&holdout, |(b, _)| b);
    let work_candidate = sum(&selection, |(_, c)| c) + sum(&holdout, |(_, c)| c);

    let judgement = |disposition| Judgement {
        disposition,
        selection: sel.clone(),
        holdout: hold.clone(),
        work_baseline,
        work_candidate,
    };

    // Order matters: a guardrail breach is a rejection whatever the score, and
    // thin evidence is not a rejection — it is an absence of one.
    if work_baseline > 0 && (work_candidate as f64) < work_baseline as f64 * WORK_FLOOR {
        return judgement(Disposition::Reject(format!(
            "work fell from {work_baseline} tool calls to {work_candidate}: a gain bought by \
             attempting less is not a gain"
        )));
    }
    if sel.total() < MIN_SELECTION_PAIRS {
        return judgement(Disposition::Propose(format!(
            "only {} paired episode(s) in the selection slice, below the floor of \
             {MIN_SELECTION_PAIRS} — read it rather than trusting it",
            sel.total()
        )));
    }
    if !sel.better() {
        return judgement(Disposition::Reject(format!(
            "did not beat the original: {} better, {} worse, {} unchanged",
            sel.wins, sel.losses, sel.ties
        )));
    }
    if hold.total() < MIN_HOLDOUT_PAIRS {
        return judgement(Disposition::Propose(format!(
            "won on the selection slice but the holdout has only {} episode(s), below \
             {MIN_HOLDOUT_PAIRS} — nothing has confirmed it on unseen work",
            hold.total()
        )));
    }
    if !hold.not_worse() {
        return judgement(Disposition::Reject(format!(
            "won on selection and lost on the holdout ({} better, {} worse): the gain did not \
             survive episodes it was not chosen on",
            hold.wins, hold.losses
        )));
    }
    // Only now, with the regression check already passed. A holdout that
    // *found* a loss has spoken, whatever its headroom — ordering this ahead
    // of `not_worse` turned a detected regression into "thin evidence", which
    // is the opposite finding, and the overfitting test caught it.
    //
    // What is left here is the all-ties case, where sample size and
    // discriminating power come apart. See `MIN_INFORMATIVE_HOLDOUT`.
    let informative = holdout
        .iter()
        .filter(|p| {
            let (_, before, _) = cost(p);
            before > 0.0
        })
        .count();
    if informative < MIN_INFORMATIVE_HOLDOUT {
        return judgement(Disposition::Propose(format!(
            "won on the selection slice, and nothing got worse on the holdout — but only \
             {informative} of {} held-out episode(s) had any of this metric to begin with, \
             so the holdout ruled out a regression without ever being able to confirm a gain",
            hold.total()
        )));
    }
    if !class.auto_acceptable() {
        return judgement(Disposition::Propose(format!(
            "measured better, but a {class:?} change is a person's decision however it scored"
        )));
    }
    judgement(Disposition::Accept)
}

/// Pair two arms by episode id, dropping anything that ran in only one.
///
/// An episode missing from an arm is not a tie and not a loss — it is missing,
/// and scoring it either way would let a candidate that *crashes* on hard
/// episodes look good on the ones it survived.
pub fn pair_arms(
    baseline: &BTreeMap<String, RunStats>,
    candidate: &BTreeMap<String, RunStats>,
) -> Vec<Pair> {
    baseline
        .iter()
        .filter_map(|(episode, b)| {
            candidate.get(episode).map(|c| Pair {
                episode: episode.clone(),
                baseline: b.clone(),
                candidate: c.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corpus_that_cannot_fill_both_slices_cannot_measure() {
        // The floor is the sum, not either half: both slices are drawn from
        // one pool, so a corpus between the two still cannot fill them.
        assert!(!measurable(0));
        assert!(!measurable(MIN_SELECTION_PAIRS));
        assert!(!measurable(MIN_MEASURABLE_RUNS - 1));
        assert!(measurable(MIN_MEASURABLE_RUNS));
        // The live case that motivated it: the morning-briefing job has 11
        // recorded runs against a floor of 12.
        assert!(!measurable(11));
        assert!(measurable(236));
    }

    #[test]
    fn every_metric_variant_reaches_all() {
        // `ALL` drives the brief, `guard_regressions`, and both consistency
        // tests below — which is why it cannot police itself: a seventh
        // variant that never joined this array would silently drop out of all
        // four, and every test that iterates `ALL` would keep passing while
        // covering less.
        //
        // The match is exhaustive, so adding a variant stops the build here
        // and the count names the fix. That is the whole guard: the compiler
        // points, the assertion says what to do.
        for m in Metric::ALL {
            match m {
                Metric::EndedOnFailedCall
                | Metric::ToolErrorRate
                | Metric::CutShort
                | Metric::Compactions
                | Metric::Turns
                | Metric::MalformedArgs => {}
            }
        }
        assert_eq!(
            Metric::ALL.len(),
            6,
            "a Metric variant was added or removed without updating ALL — the brief, \
             guard_regressions and the drift tests all read it"
        );
    }

    #[test]
    fn every_metric_name_is_its_serde_spelling() {
        // `as_str` is what the brief prints and what the proposal block is
        // parsed against, and serde is what the candidate store round-trips
        // through. A metric whose two spellings disagree would be reported
        // under one name and proposable only under the other.
        for m in Metric::ALL {
            let wire = serde_json::to_string(&m).unwrap();
            assert_eq!(wire.trim_matches('"'), m.as_str());
        }
    }

    #[test]
    fn a_metric_no_run_has_any_of_is_visible_as_zero_headroom() {
        // The 2026-08-28 nightly in shape: it predicted a lower `cut_short`
        // over a corpus where every run was `Completed` or `Interrupted`, and
        // `cut_short` excludes `Interrupted` on purpose. Every pair could only
        // tie, so the measurement it would have cost could not have informed
        // anything.
        let completed = RunStats {
            stop_cause: Some(crate::agent::StopCause::Completed),
            ..Default::default()
        };
        let interrupted = RunStats {
            stop_cause: Some(crate::agent::StopCause::Interrupted),
            ..Default::default()
        };
        assert_eq!(Metric::CutShort.of(&completed), 0.0);
        assert_eq!(Metric::CutShort.of(&interrupted), 0.0);
        let cut = RunStats {
            stop_cause: Some(crate::agent::StopCause::MaxTurns),
            ..Default::default()
        };
        assert_eq!(Metric::CutShort.of(&cut), 1.0);
    }

    use crate::agent::StopCause;

    fn run(calls: u32, errors: u32, ended_failed: bool) -> RunStats {
        RunStats {
            tool_calls: calls,
            tool_errors: errors,
            ended_on_failed_call: ended_failed,
            stop_cause: Some(StopCause::Completed),
            ..RunStats::default()
        }
    }

    fn prediction(metric: Metric) -> Prediction {
        Prediction {
            metric,
            rationale: "because".into(),
        }
    }

    /// Episodes named so the split is known: with `holdout_in = 3` the ids
    /// below land where the assertions expect. Built by asking `is_holdout`
    /// rather than by assuming, so the fixture cannot drift from the hash.
    fn corpus(n: usize, holdout_in: u64, f: impl Fn(usize) -> (RunStats, RunStats)) -> Vec<Pair> {
        let mut pairs = Vec::new();
        let mut i = 0;
        let (mut sel, mut hold) = (0, 0);
        while sel < n || hold < n.div_ceil(2) {
            let episode = format!("ep-{i}");
            i += 1;
            let is_h = is_holdout(&episode, holdout_in);
            if is_h && hold >= n.div_ceil(2) {
                continue;
            }
            if !is_h && sel >= n {
                continue;
            }
            if is_h {
                hold += 1
            } else {
                sel += 1
            }
            let (baseline, candidate) = f(pairs.len());
            pairs.push(Pair {
                episode,
                baseline,
                candidate,
            });
        }
        pairs
    }

    #[test]
    fn a_change_that_wins_on_both_slices_is_accepted_without_a_person() {
        let pairs = corpus(12, 3, |_| (run(10, 4, true), run(10, 1, false)));
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::EndedOnFailedCall),
            &pairs,
            3,
        );
        assert_eq!(j.disposition, Disposition::Accept, "{j:#?}");
        assert!(j.selection.wins >= MIN_SELECTION_PAIRS);
        assert_eq!(j.selection.losses, 0);
    }

    #[test]
    fn a_gain_bought_by_attempting_less_is_rejected_however_it_scored() {
        // The Goodhart case, and the one this gate exists for: every episode
        // improves on the metric, and the improvement is that the run stopped
        // doing anything. Measured elsewhere at 30.4% of RE-Bench runs.
        let pairs = corpus(12, 3, |_| (run(20, 6, true), run(1, 0, false)));
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::EndedOnFailedCall),
            &pairs,
            3,
        );
        match j.disposition {
            Disposition::Reject(ref why) => assert!(why.contains("attempting less"), "{why}"),
            other => panic!("a suppressed-work win was not rejected: {other:?}"),
        }
        assert!(j.work_candidate < j.work_baseline);
    }

    #[test]
    fn a_holdout_that_could_not_have_confirmed_anything_does_not_confirm() {
        // The hole this closes: `not_worse` is `wins >= losses`, so a holdout
        // of four all-tie episodes satisfies it with 0 and 0 and reads as
        // "confirmed on unseen work". The holdout is drawn uniformly on
        // purpose, and on 2026-08-31 the live corpus was 69% runs of two turns
        // or fewer — so a uniform draw of four was usually four episodes that
        // could not have moved whatever was predicted.
        //
        // Every holdout baseline here has `ended_on_failed_call` false, so no
        // held-out episode had anything to lose.
        let pairs: Vec<Pair> = corpus(12, 3, |_| (run(10, 5, true), run(10, 5, true)))
            .into_iter()
            .map(|mut p| {
                if is_holdout(&p.episode, 3) {
                    p.baseline = run(10, 5, false);
                    p.candidate = run(10, 5, false);
                } else {
                    p.baseline = run(10, 5, true);
                    p.candidate = run(10, 5, false);
                }
                p
            })
            .collect();
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::EndedOnFailedCall),
            &pairs,
            3,
        );
        match j.disposition {
            Disposition::Propose(ref why) => {
                assert!(why.contains("without ever being able to confirm"), "{why}")
            }
            other => panic!("a vacuous holdout was treated as confirmation: {other:?}"),
        }
    }

    #[test]
    fn a_win_paid_for_on_a_metric_nobody_predicted_is_not_a_win() {
        // The work guardrail counts tool calls, so it catches a gain bought by
        // attempting *less*. It never caught a gain bought by failing *more*:
        // this candidate does exactly the same amount of work and halves the
        // predicted metric, while every call it makes now fails.
        //
        // Live, this is the trade the loop is most likely to be offered —
        // `turns` is currently the only metric in the corpus with real
        // headroom, and spending accuracy to end sooner buys it.
        let pairs: Vec<Pair> = corpus(12, 3, |_| {
            let mut baseline = run(10, 1, false);
            baseline.turns = 10;
            let mut candidate = run(10, 9, false);
            candidate.turns = 5;
            (baseline, candidate)
        });
        let j = judge(ChangeClass::Config, &prediction(Metric::Turns), &pairs, 3);
        match j.disposition {
            Disposition::Reject(ref why) => {
                assert!(why.contains("ToolErrorRate"), "{why}");
                assert!(why.contains("nobody was watching"), "{why}");
            }
            other => panic!("a bought win was accepted: {other:?}"),
        }
    }

    #[test]
    fn an_unpredicted_metric_that_holds_steady_does_not_block_a_real_win() {
        // The guard above must be a cliff, not a ratchet — otherwise noise on
        // any of five other metrics vetoes every candidate and the loop stops
        // accepting anything at all, which is the failure it was added to
        // avoid wearing the opposite costume.
        let pairs: Vec<Pair> = corpus(12, 3, |_| {
            let mut baseline = run(10, 2, false);
            baseline.turns = 10;
            let mut candidate = run(10, 2, false);
            candidate.turns = 5;
            (baseline, candidate)
        });
        let j = judge(ChangeClass::Config, &prediction(Metric::Turns), &pairs, 3);
        assert_eq!(j.disposition, Disposition::Accept, "{:?}", j.disposition);
    }

    #[test]
    fn winning_selection_and_losing_the_holdout_is_a_rejection() {
        // Overfitting made visible: the candidate is better on exactly the
        // episodes it was chosen on, and worse on the ones it was not.
        let pairs: Vec<Pair> = corpus(12, 3, |_| (run(10, 5, true), run(10, 5, true)))
            .into_iter()
            .map(|mut p| {
                if is_holdout(&p.episode, 3) {
                    p.candidate = run(10, 5, true);
                    p.baseline = run(10, 5, false);
                } else {
                    p.baseline = run(10, 5, true);
                    p.candidate = run(10, 5, false);
                }
                p
            })
            .collect();
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::EndedOnFailedCall),
            &pairs,
            3,
        );
        match j.disposition {
            Disposition::Reject(ref why) => assert!(why.contains("holdout"), "{why}"),
            other => panic!("an overfit candidate was not rejected: {other:?}"),
        }
    }

    #[test]
    fn thin_evidence_proposes_rather_than_rejecting() {
        // An absence of evidence is not evidence of harm. Three episodes that
        // all improved is exactly the shape a person should read.
        let pairs = corpus(3, 3, |_| (run(10, 4, true), run(10, 1, false)));
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::EndedOnFailedCall),
            &pairs,
            3,
        );
        match j.disposition {
            Disposition::Propose(ref why) => assert!(why.contains("floor"), "{why}"),
            other => panic!("thin evidence should propose, not {other:?}"),
        }
    }

    #[test]
    fn architecture_and_security_reach_a_person_however_well_they_score() {
        let pairs = corpus(12, 3, |_| (run(10, 4, true), run(10, 0, false)));
        for class in [ChangeClass::Architecture, ChangeClass::Security] {
            let j = judge(class, &prediction(Metric::EndedOnFailedCall), &pairs, 3);
            match j.disposition {
                Disposition::Propose(ref why) => {
                    assert!(why.contains("person's decision"), "{why}")
                }
                other => panic!("{class:?} must not auto-accept: {other:?}"),
            }
        }
    }

    #[test]
    fn a_run_that_made_no_calls_is_neutral_on_the_error_rate() {
        // No calls is no evidence, so it must not be scored as a perfect
        // record — otherwise suppressing work wins on the rate metric too,
        // and the work guardrail would be the only thing standing.
        let none = run(0, 0, false);
        assert_eq!(Metric::ToolErrorRate.of(&none), 0.0);
        let clean = run(10, 0, false);
        assert_eq!(Metric::ToolErrorRate.of(&clean), 0.0);
        // Which is why they tie rather than one beating the other.
        let pairs = corpus(12, 3, |_| (run(10, 0, false), run(0, 0, false)));
        let j = judge(
            ChangeClass::Config,
            &prediction(Metric::ToolErrorRate),
            &pairs,
            3,
        );
        assert_eq!(
            j.selection.wins, 0,
            "doing nothing must not beat doing well"
        );
    }

    #[test]
    fn the_split_is_stable_across_runs_or_the_holdout_means_nothing() {
        let ids: Vec<String> = (0..200).map(|i| format!("ep-{i}")).collect();
        let first: Vec<bool> = ids.iter().map(|e| is_holdout(e, 4)).collect();
        let again: Vec<bool> = ids.iter().map(|e| is_holdout(e, 4)).collect();
        assert_eq!(first, again);
        // And it actually splits: a "holdout" that takes everything or
        // nothing would pass every test above while measuring nothing.
        let held = first.iter().filter(|h| **h).count();
        assert!((20..80).contains(&held), "{held} of 200 held out");
    }

    #[test]
    fn a_suite_the_baseline_already_passes_cannot_confirm_an_improvement() {
        // The eval side of `MIN_INFORMATIVE_HOLDOUT`, which review noted was
        // coupled and untested. In `mecha eval --ab-config` the cost is
        // `!passed`, so a held-out case the baseline already passed has cost 0
        // and no room to win — exactly the shape the run-side check exists
        // for, in a different currency.
        //
        // A change is not confirmed by cases that were already green. It
        // stages for a person instead, which is what the disposition is for.
        struct Case {
            id: String,
            was: bool,
            now: bool,
        }
        let cases: Vec<Case> = (0..24)
            .map(|i| Case {
                id: format!("case-{i}"),
                // Held-out cases all passed under the baseline; selection
                // cases were failing and now pass.
                was: is_holdout(&format!("case-{i}"), 3),
                now: true,
            })
            .collect();
        fn cost(c: &Case) -> (&str, f64, f64) {
            (
                c.id.as_str(),
                f64::from(u8::from(!c.was)),
                f64::from(u8::from(!c.now)),
            )
        }
        let refs: Vec<&Case> = cases.iter().collect();
        let (hold, sel): (Vec<&Case>, Vec<&Case>) =
            refs.into_iter().partition(|c| is_holdout(&c.id, 3));
        let j = judge_slices(ChangeClass::Config, &sel, &hold, cost, |_| (6, 6));
        match j.disposition {
            Disposition::Propose(ref why) => {
                assert!(why.contains("without ever being able to confirm"), "{why}")
            }
            other => panic!("an all-green holdout was read as confirmation: {other:?}"),
        }
    }

    #[test]
    fn a_real_regression_is_not_hidden_behind_a_milder_one_earlier_in_the_list() {
        // The short-circuit: `guard_regressions` returned on the first metric
        // it found something on, so a `0 → nonzero` Propose at `Compactions`
        // (index 3) suppressed a ratio breach at `MalformedArgs` (index 5) and
        // the reviewer saw only the benign explanation. Each of the other
        // tests moves exactly one unpredicted metric, so none of them could
        // see it.
        let pairs: Vec<Pair> = corpus(12, 3, |_| {
            let mut baseline = run(10, 1, false);
            baseline.turns = 10;
            baseline.compactions = 0;
            baseline.malformed_tool_args = 4;
            let mut candidate = run(10, 1, false);
            candidate.turns = 5;
            // Milder, and earlier in `Metric::ALL`.
            candidate.compactions = 2;
            // Worse, and later: 10 against a 1.25 ceiling on 4.
            candidate.malformed_tool_args = 10;
            (baseline, candidate)
        });
        let j = judge(ChangeClass::Config, &prediction(Metric::Turns), &pairs, 3);
        match j.disposition {
            Disposition::Reject(ref why) => assert!(why.contains("MalformedArgs"), "{why}"),
            other => panic!("the worse finding was hidden behind the milder one: {other:?}"),
        }
    }

    #[test]
    fn a_cost_appearing_from_nothing_reaches_a_person_rather_than_being_refused() {
        // `compactions` is zero across the live corpus, so a
        // `compact_at_tokens` low enough to actually compact takes the metric
        // from 0 to nonzero. A first version of `guard_regressions` rejected
        // that outright, which made one of the four overridable knobs unusable
        // for the metric it exists to move — found in review.
        //
        // Nothing here can tell "compaction started, which is the point" from
        // "malformed arguments appeared, which is not". A person can.
        let pairs: Vec<Pair> = corpus(12, 3, |_| {
            let mut baseline = run(10, 1, false);
            baseline.turns = 10;
            baseline.compactions = 0;
            let mut candidate = run(10, 1, false);
            candidate.turns = 5;
            candidate.compactions = 2;
            (baseline, candidate)
        });
        let j = judge(ChangeClass::Config, &prediction(Metric::Turns), &pairs, 3);
        match j.disposition {
            Disposition::Propose(ref why) => {
                assert!(why.contains("rose from"), "{why}");
                assert!(why.contains("only a person can tell which"), "{why}");
            }
            other => panic!("the knob's own effect was treated as a regression: {other:?}"),
        }
    }

    #[test]
    fn the_generic_gate_grades_case_outcomes_by_the_same_rules() {
        // The content-sensitive arm: eval cases scored on whether they passed,
        // which is what a prose change needs, since replay holds tool results
        // fixed and cannot see a change in what the model said. Same gate, so
        // the guardrails and the holdout cannot drift between currencies.
        struct Case {
            id: String,
            was: bool,
            now: bool,
            calls: u64,
        }
        let cases: Vec<Case> = (0..24)
            .map(|i| Case {
                id: format!("case-{i}"),
                was: false,
                now: true,
                calls: 6,
            })
            .collect();

        // A failure is the cost, so passing is a win. A `fn` rather than a
        // closure: the gate's `cost` is higher-ranked over the borrow, and an
        // un-annotated closure infers a single lifetime that will not unify.
        fn cost(c: &Case) -> (&str, f64, f64) {
            (
                c.id.as_str(),
                f64::from(u8::from(!c.was)),
                f64::from(u8::from(!c.now)),
            )
        }
        let j = judge_with(ChangeClass::Prose, &cases, cost, |c| (c.calls, c.calls), 3);
        assert_eq!(j.disposition, Disposition::Accept, "{j:#?}");

        // And the work guardrail applies in this currency too: a prose change
        // that passes more cases by attempting less is still buying its win.
        let lazy: Vec<Case> = cases
            .into_iter()
            .map(|mut c| {
                c.calls = 6;
                c
            })
            .collect();
        let j = judge_with(ChangeClass::Prose, &lazy, cost, |c| (c.calls, 1), 3);
        match j.disposition {
            Disposition::Reject(ref why) => assert!(why.contains("attempting less"), "{why}"),
            other => panic!("the work guardrail did not cross currencies: {other:?}"),
        }
    }

    #[test]
    fn an_episode_that_ran_in_only_one_arm_is_dropped_not_scored() {
        // A candidate that dies on the hard episodes must not look good on
        // the ones it survived.
        let mut baseline = BTreeMap::new();
        baseline.insert("a".to_string(), run(5, 0, false));
        baseline.insert("hard".to_string(), run(5, 3, true));
        let mut candidate = BTreeMap::new();
        candidate.insert("a".to_string(), run(5, 0, false));

        let pairs = pair_arms(&baseline, &candidate);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].episode, "a");
    }
}

#[cfg(test)]
mod prioritised_tests {
    use super::*;

    fn stats(tool_calls: u32, tool_errors: u32) -> RunStats {
        RunStats {
            tool_calls,
            tool_errors,
            ..RunStats::default()
        }
    }

    /// Headroom is the metric's own value, and an episode at the floor is the
    /// one worth *not* spending a replay on: whatever the change does, it can
    /// only tie or worsen.
    #[test]
    fn an_episode_with_no_room_to_improve_has_no_priority() {
        let m = Metric::ToolErrorRate;
        assert_eq!(m.headroom(&stats(10, 5)), 0.5);
        assert_eq!(m.headroom(&stats(10, 0)), 0.0, "clean run, nothing to fix");
        assert_eq!(
            m.headroom(&stats(0, 0)),
            0.0,
            "no calls is no evidence, which the metric already says"
        );
        assert!(m.headroom(&stats(10, 9)) > m.headroom(&stats(10, 1)));
    }

    /// **The reason the slices are drawn separately.** `is_holdout` partitions
    /// one pool, so if that pool was gathered by headroom, *both* halves carry
    /// only high-headroom episodes and the holdout stops being a check on the
    /// selection's bias. Drawing it uniformly from the whole corpus is what
    /// keeps "confirmed on unseen work" meaning what it says.
    #[test]
    fn hashing_a_prioritised_pool_yields_a_prioritised_holdout() {
        let corpus: Vec<(String, RunStats)> = (0..40)
            .map(|i| {
                // Half the corpus is clean and can say nothing about the
                // error rate; half has real headroom.
                let s = if i % 2 == 0 {
                    stats(10, 0)
                } else {
                    stats(10, 4)
                };
                (format!("ep-{i:02}"), s)
            })
            .collect();
        let m = Metric::ToolErrorRate;

        // Gather by priority, then hash-split it the old way.
        let mut by_priority = corpus.clone();
        by_priority.sort_by(|a, b| m.headroom(&b.1).partial_cmp(&m.headroom(&a.1)).unwrap());
        let pool: Vec<&(String, RunStats)> = by_priority.iter().take(20).collect();
        let hashed_holdout: Vec<_> = pool.iter().filter(|p| is_holdout(&p.0, 2)).collect();
        assert!(
            !hashed_holdout.is_empty(),
            "the split has to produce a holdout for this to be a real comparison"
        );
        assert!(
            hashed_holdout.iter().all(|p| m.headroom(&p.1) > 0.0),
            "every episode in it came from the prioritised pool, so it inherits the bias"
        );

        // Drawn uniformly from the *whole* corpus instead, it is representative.
        let drawn = crate::sample::take_uniform(corpus.clone(), 7, 20);
        let zero = drawn.iter().filter(|p| m.headroom(&p.1) == 0.0).count();
        assert!(
            zero > 0,
            "a uniform draw contains episodes the priority would have excluded"
        );
    }

    /// The gate still gates: `judge_drawn` scores the slices it is handed and
    /// applies the same guardrails in the same order.
    #[test]
    fn the_drawn_gate_applies_the_same_guardrails() {
        let pair = |id: &str, before: u32, after: u32| Pair {
            episode: id.into(),
            baseline: stats(10, before),
            candidate: stats(10, after),
        };
        let prediction = Prediction {
            metric: Metric::ToolErrorRate,
            rationale: String::new(),
        };
        let selection: Vec<Pair> = (0..MIN_SELECTION_PAIRS)
            .map(|i| pair(&format!("s{i}"), 5, 2))
            .collect();
        let holdout: Vec<Pair> = (0..MIN_HOLDOUT_PAIRS)
            .map(|i| pair(&format!("h{i}"), 5, 4))
            .collect();
        let j = judge_drawn(ChangeClass::Config, &prediction, &selection, &holdout);
        assert_eq!(j.disposition, Disposition::Accept);
        assert_eq!(j.selection.wins, MIN_SELECTION_PAIRS);

        // A thin holdout proposes rather than accepting — unchanged behaviour,
        // reached through the new entry point.
        let j = judge_drawn(ChangeClass::Config, &prediction, &selection, &holdout[..1]);
        assert!(matches!(j.disposition, Disposition::Propose(_)));
    }
}
