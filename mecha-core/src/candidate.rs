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
            Metric::CutShort => f64::from(u8::from(
                s.stop_cause
                    .is_some_and(|c| c != crate::agent::StopCause::Completed),
            )),
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
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    episode.hash(&mut h);
    h.finish().is_multiple_of(holdout_in)
}

/// Grade a candidate against its own prediction.
pub fn judge(
    class: ChangeClass,
    prediction: &Prediction,
    pairs: &[Pair],
    holdout_in: u64,
) -> Judgement {
    let (holdout, selection): (Vec<&Pair>, Vec<&Pair>) = pairs
        .iter()
        .partition(|p| is_holdout(&p.episode, holdout_in));

    let tally = |slice: &[&Pair]| {
        let mut t = Tally::default();
        for p in slice {
            let before = prediction.metric.of(&p.baseline);
            let after = prediction.metric.of(&p.candidate);
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

    let work = |slice: &[&Pair], pick: fn(&Pair) -> &RunStats| -> u64 {
        slice.iter().map(|p| u64::from(pick(p).tool_calls)).sum()
    };
    let work_baseline = work(&selection, |p| &p.baseline) + work(&holdout, |p| &p.baseline);
    let work_candidate = work(&selection, |p| &p.candidate) + work(&holdout, |p| &p.candidate);

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
