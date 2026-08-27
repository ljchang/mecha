//! The appraisal probe: what a counterfactual replay says about an
//! intervention, and the one thing that lifts an intervention's agency.
//!
//! Rung 7's observation half came back degenerate — 119 signed goal errors
//! across 120 appraised sessions and **100% neutral labels** — and the reason
//! was one field. `of_session` records an intervention as `Agency::Owner` with
//! `controllable: None`, because from the transcript alone it cannot tell a
//! correction of a wrong trajectory from a change of the owner's mind. That
//! lands in the one branch of `label_of` with no word for it.
//!
//! A replay can tell them apart, and that is the whole of this module: rebuild
//! the recorded run up to the intervention, drive it **without** the steering
//! text, and see whether it gets there anyway. `mecha_core::appraisal::Probe`
//! is what the answer is called and `apply_probe` is what it means; nothing
//! here decides a label.
//!
//! **Why this is worth its cost, when the charter is not yet.** The corpus
//! says a counterfactual verdict labels 102 intervention errors where the
//! charter (§14 rung 10) buys 11 positive ones. That is a change to the design
//! doc's build order argued from a measurement rather than from the design,
//! which is what rung 7 existed to produce.

use crate::probe::{drive_arm, prepare_probe_at};
use crate::setup::Prepared;
use anyhow::Result;
use mecha_core::appraisal::{apply_probe, relabel, Appraisal, Cite, Probe};
use mecha_core::config::ProviderConfig;
use mecha_core::counterfactual::ProbeVerdict;
use mecha_core::learning::Intervention;
use std::path::Path;

/// Read a graded arm as a finding about the intervention.
///
/// **Note the polarity, because it inverts between callers.** `steer_verdict`
/// is written from `validate`'s point of view, where the recording after the
/// steer is the target and reaching it unprompted is a `Pass`. So a **`Fail`
/// is the informative result here**: the replay went somewhere else, meaning
/// the steer was load-bearing and an alternative demonstrably existed — the
/// agent simply did not take it. Reading `Fail` as "the probe failed, so
/// nothing was learned" is exactly backwards and would attribute regret to the
/// runs that needed no help.
fn finding(v: &ProbeVerdict) -> Probe {
    match v {
        ProbeVerdict::Fail => Probe::Mattered,
        ProbeVerdict::Pass => Probe::Redundant,
        ProbeVerdict::Inconclusive(_) => Probe::Inconclusive,
    }
}

/// What probing one session's interventions cost and found.
///
/// Skips and inconclusives are counted **apart**, on this store's oldest rule:
/// "could not look" and "looked and learned nothing" are opposite findings,
/// and a reader that folds them together cannot tell a probe budget that ran
/// out from a corpus with nothing in it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Arms actually driven — the model runs this cost.
    pub driven: usize,
    pub mattered: usize,
    pub redundant: usize,
    /// Driven, but the replay departed before the probe point.
    pub inconclusive: usize,
    /// Never driven: no replayable point, an unreadable session, or the
    /// budget. Costs nothing and is evidence of nothing.
    pub skipped: usize,
}

impl Tally {
    fn record(&mut self, p: Probe) {
        match p {
            Probe::Mattered => self.mattered += 1,
            Probe::Redundant => self.redundant += 1,
            Probe::Inconclusive => self.inconclusive += 1,
        }
    }

    pub fn add(&mut self, other: Tally) {
        self.driven += other.driven;
        self.mattered += other.mattered;
        self.redundant += other.redundant;
        self.inconclusive += other.inconclusive;
        self.skipped += other.skipped;
    }
}

/// Probe every intervention behind one appraisal and fold the findings in.
///
/// `budget` is the number of arms this call may still drive, decremented as it
/// goes — **consumed by drives, never by skips**. A session whose
/// interventions are all `edit` (no replayable point, by construction) must
/// not silently exhaust a corpus-wide allowance without having probed
/// anything, which is what charging per intervention would do.
///
/// The join back to the record is [`Cite::Turn`], not position. `of_session`
/// pushes counter, intervention and edit errors into one vector, so an index
/// into `errors` is not an index into interventions — it happens to line up
/// today only because the counter channel is usually empty, which is the kind
/// of coincidence that holds until the first run that stopped on a loop.
pub async fn probe_appraisal(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    sessions_dir: &Path,
    interventions: &[Intervention],
    appraisal: &mut Appraisal,
    budget: &mut usize,
) -> Result<Tally> {
    let mut tally = Tally::default();
    for i in interventions {
        if *budget == 0 {
            tally.skipped += 1;
            continue;
        }
        let prep = match prepare_probe_at(
            sessions_dir,
            &appraisal.session_id,
            i.trigger.as_str(),
            &i.text,
        )? {
            Ok(prep) => prep,
            // A skip is never evidence for either arm, so the error keeps the
            // `Owner`/`None` it was assembled with and stays neutral.
            Err(_) => {
                tally.skipped += 1;
                continue;
            }
        };
        // The run exactly as it was, rules block and all — see
        // `ProbePrep::system_as_recorded` for why a rules-free arm would bias
        // every verdict toward `Mattered`.
        let system = prep.system_as_recorded();
        *budget -= 1;
        tally.driven += 1;
        let verdict = match drive_arm(prepared, provider_cfg, model, &prep, system).await? {
            Ok(v) => v,
            Err(_) => {
                // Driven and lost. Charged to the budget, because the model
                // run happened, and recorded as a skip, because it produced
                // no finding.
                tally.skipped += 1;
                continue;
            }
        };
        let found = finding(&verdict);
        tally.record(found);
        for e in appraisal
            .errors
            .iter_mut()
            .filter(|e| e.cite == Cite::Turn(i.at))
        {
            apply_probe(e, found);
        }
    }
    // Once, at the end: frustration is a fact about the whole record, so a
    // label cannot be recomputed one error at a time.
    relabel(appraisal);
    Ok(tally)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The polarity, pinned. This is the assertion that fails if anyone
    /// "fixes" the mapping to read `Pass` as the good outcome.
    #[test]
    fn a_failed_arm_is_the_steer_that_mattered() {
        assert_eq!(finding(&ProbeVerdict::Fail), Probe::Mattered);
        assert_eq!(finding(&ProbeVerdict::Pass), Probe::Redundant);
        assert_eq!(
            finding(&ProbeVerdict::Inconclusive("diverged early".into())),
            Probe::Inconclusive
        );
    }

    #[test]
    fn a_tally_keeps_skipped_and_inconclusive_apart() {
        let mut t = Tally::default();
        t.record(Probe::Inconclusive);
        t.skipped += 1;
        assert_eq!(t.inconclusive, 1);
        assert_eq!(t.skipped, 1);
        // Folding them together is the bug this asserts against: the two
        // answer different questions and only one of them cost a model run.
        assert_ne!(t.inconclusive + t.skipped, t.inconclusive);
    }
}
