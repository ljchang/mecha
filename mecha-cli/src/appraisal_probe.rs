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
use mecha_core::replay_run::replay_surface_specs;
use mecha_core::surface::Fidelity;
use std::path::Path;

/// Say why a probe produced nothing, on stderr, always.
///
/// **A skip with its reason discarded is the finding this whole rung is about,
/// one layer down.** `appraise` exists because a label that says nothing is
/// indistinguishable from a label nobody could compute; a probe that reports
/// `skipped` without saying whether the session was unreadable, the point
/// unlocatable or the replay refused reproduces exactly that. `validate`
/// prints its skips for the same reason and it is the precedent.
fn skipped(session_id: &str, at: usize, why: &str) {
    eprintln!("· {session_id} turn {at}: {why}");
}

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
/// **Four ways to produce no finding, counted apart, and the split is the
/// measurement.** Folding them into one `skipped` was the first cut and it hid
/// the thing worth knowing: an intervention with no replayable point is a
/// permanent ceiling on what this mechanism can ever reach, where a budget
/// that ran out is a number somebody chose and a replay that could not be
/// built is a machine that can be fixed. A reader who cannot tell them apart
/// cannot tell a probe worth extending from one worth abandoning — which is
/// the same mistake as a queue reporting its own unreadability as zero, one
/// layer down.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Arms actually driven — the model runs this cost.
    pub driven: usize,
    pub mattered: usize,
    pub redundant: usize,
    /// Driven, but the replay departed before the probe point.
    pub inconclusive: usize,
    /// No replayable intervention point *by construction* — a `followup` is a
    /// later user turn rather than text riding with tool results, and an
    /// `edit` lives in an outbox item and not in any transcript. Free, and a
    /// structural ceiling on the probe's reach rather than a failure.
    pub unprobeable: usize,
    /// The replay could not be built or driven: an unreadable session, or a
    /// recorded tool the current registry cannot offer. Fixable, and worth
    /// separating for exactly that reason.
    pub unavailable: usize,
    /// Never looked at, because the budget ran out first. Says nothing about
    /// the intervention at all.
    pub over_budget: usize,

    /// How faithfully each driven probe reproduced the recorded request, split
    /// three ways because the middle state is the one that was missing.
    ///
    /// A replay rebuilds the system prompt from the recording and the **tool
    /// specs from today's registry**, and render order puts tools first — so a
    /// description edited since the recording changes the bytes ahead of
    /// everything else. Measured before this existed: the replay tracked the
    /// recording for a median of **one** tool call, deterministically, with
    /// one session's six probes all giving up at the same index. A per-session
    /// constant, which is what a surface is.
    ///
    /// `differs` and `unknown` are kept apart deliberately. Drift that is
    /// *provable* and drift that is merely *unmeasured* call for opposite
    /// responses — one invalidates the probe, the other says nothing — and
    /// folding them together is the same conflation that made twelve
    /// inconclusive probes look like one unexplained number.
    pub matches: usize,
    pub differs: usize,
    pub unknown: usize,
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
        self.unprobeable += other.unprobeable;
        self.unavailable += other.unavailable;
        self.over_budget += other.over_budget;
        self.matches += other.matches;
        self.differs += other.differs;
        self.unknown += other.unknown;
    }
}

/// Is this intervention one a structural replay can pose a question about?
///
/// `Steer` and `Denial` only. A `Followup` is a later user turn, so removing
/// it does not leave a run that "would have got there anyway" — there is no
/// counterfactual to drive, which is why `validate` reaches followups with a
/// judge instead. An `Edit` never appears in a transcript at all.
///
/// Asked *before* the session is loaded, so the commonest skip costs no I/O
/// and no budget.
fn replayable(t: mecha_core::learning::Trigger) -> bool {
    use mecha_core::learning::Trigger;
    matches!(t, Trigger::Steer | Trigger::Denial)
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
        // Asked first: a followup or an edit can never be probed, so counting
        // it against a budget it did not spend would make a corpus of them
        // look like a probe that ran out.
        if !replayable(i.trigger) {
            tally.unprobeable += 1;
            continue;
        }
        if *budget == 0 {
            tally.over_budget += 1;
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
            Err(why) => {
                skipped(&appraisal.session_id, i.at, &why);
                tally.unavailable += 1;
                continue;
            }
        };
        // How faithfully this probe can reproduce the recorded request, read
        // *before* it is driven — a verdict is only worth what the request
        // behind it was. Fingerprints the surface `drive_arm` will actually
        // send — the recorded names, narrowed from the live registry and
        // filled from the surface-only stand-in — never the bare CLI
        // registry, which can never hold `ask_user` and so would report
        // `Differs` on nearly every probe regardless of whether anything
        // actually changed.
        let surface_specs = match replay_surface_specs(
            prep.recorded_tools(),
            prepared.agent.registry(),
            Some(&crate::setup::surface_only_registry()),
        ) {
            Ok(specs) => specs,
            // The same failure `drive_arm` would hit below, on the identical
            // inputs — surfaced here instead of spending a model call to
            // rediscover it.
            Err(why) => {
                skipped(&appraisal.session_id, i.at, &format!("{why:#}"));
                tally.unavailable += 1;
                continue;
            }
        };
        let fidelity = Fidelity::of(prep.recorded_tools_hash(), &surface_specs);
        match fidelity {
            Fidelity::Matches => tally.matches += 1,
            Fidelity::Differs => tally.differs += 1,
            Fidelity::Unknown => tally.unknown += 1,
        }

        // The run exactly as it was, rules block and all — see
        // `ProbePrep::system_as_recorded` for why a rules-free arm would bias
        // every verdict toward `Mattered`.
        let system = prep.system_as_recorded();
        *budget -= 1;
        tally.driven += 1;
        let verdict = match drive_arm(prepared, provider_cfg, model, &prep, system).await? {
            Ok(v) => v,
            Err(why) => {
                // Driven and lost. Charged to the budget, because the model
                // run happened, and recorded as a skip, because it produced
                // no finding.
                skipped(&appraisal.session_id, i.at, &why);
                tally.unavailable += 1;
                continue;
            }
        };
        let found = finding(&verdict);
        // Per probe, not just per run. A tally answers "how many" and cannot
        // answer "the same ones?" — which is the question that separates a
        // defect from sampling noise, and the only cheap way to ask it is to
        // diff two runs. `steer_verdict`'s own inconclusive arm carries the
        // call index it gave up at, so print the reason it wrote rather than
        // a label of our own.
        // The caveat rides on the line rather than being summarised away: a
        // reader looking at one inconclusive probe needs to know whether the
        // request behind it was the recorded one, and the tally cannot say
        // which of its rows this line belongs to.
        let caveat = fidelity
            .caveat()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();
        match &verdict {
            ProbeVerdict::Inconclusive(why) => {
                eprintln!(
                    "· {} turn {}: inconclusive — {why}{caveat}",
                    appraisal.session_id, i.at
                )
            }
            v => eprintln!(
                "· {} turn {}: {v:?} → {found:?}{caveat}",
                appraisal.session_id, i.at
            ),
        }
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

    /// The four no-finding outcomes answer four different questions, and the
    /// first cut of this file folded them into one counter — which reported a
    /// corpus that is 78% unprobeable as a probe that had merely run out.
    #[test]
    fn every_way_of_producing_no_finding_is_counted_apart() {
        let mut t = Tally::default();
        t.record(Probe::Inconclusive);
        t.unprobeable += 1;
        t.unavailable += 1;
        t.over_budget += 1;
        assert_eq!(
            (t.inconclusive, t.unprobeable, t.unavailable, t.over_budget),
            (1, 1, 1, 1)
        );
        // Only the inconclusive one cost a model run; only the unprobeable one
        // is permanent; only the over-budget one is a number somebody chose.
        assert_eq!(t.driven, 0);
    }

    /// A followup has no counterfactual to drive — removing a later user turn
    /// does not leave a run that would have got there anyway — and an edit is
    /// not in the transcript at all. Both are structural, not failures.
    #[test]
    fn only_steers_and_denials_are_replayable() {
        use mecha_core::learning::Trigger;
        assert!(replayable(Trigger::Steer));
        assert!(replayable(Trigger::Denial));
        assert!(!replayable(Trigger::Followup));
        assert!(!replayable(Trigger::Edit));
    }
}
