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
//! A replay can tell them apart, and that is the whole of this module:
//! resubmit the recorded run up to the intervention verbatim, let the model
//! continue **without** the steering text, and see whether it gets there
//! anyway ([`mecha_core::replay_run::drive_branch`], via `drive_arm` — so a
//! pre-point divergence cannot eat the probe; it used to, on nearly every
//! mid-run point). `mecha_core::appraisal::Probe` is what the answer is
//! called and `apply_probe` is what it means; nothing here decides a label.
//!
//! **Why this is worth its cost, when the charter is not yet.** The corpus
//! says a counterfactual verdict labels 102 intervention errors where the
//! charter (§14 rung 10) buys 11 positive ones. That is a change to the design
//! doc's build order argued from a measurement rather than from the design,
//! which is what rung 7 existed to produce.

use crate::probe::{drive_arm, prepare_probe_in};
use crate::setup::Prepared;
use anyhow::Result;
use mecha_core::appraisal::{apply_probe, relabel, Appraisal, Cite, Probe};
use mecha_core::config::ProviderConfig;
use mecha_core::counterfactual::ProbeVerdict;
use mecha_core::learning::Intervention;
use mecha_core::replay_run::replay_surface_specs;
use std::path::Path;

/// Say why a probe produced nothing, on stderr, always.
///
/// **A skip with its reason discarded is the finding this whole rung is about,
/// one layer down.** `appraise` exists because a label that says nothing is
/// indistinguishable from a label nobody could compute; a probe that reports
/// `skipped` without saying whether the session was unreadable, the point
/// unlocatable or the replay refused reproduces exactly that. `validate`
/// prints its skips for the same reason and it is the precedent. An
/// `Inconclusive` verdict is the same case one step later — driven, and
/// still no directional finding — so it is reported through here too.
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

/// Legibility, wired in rather than left to the module doc's claim: a
/// fidelity mismatch is exactly the kind of "diverged for a reason that says
/// nothing about the intervention" case this rung exists to name rather than
/// let read as a mystery. `Fidelity::of` needs no blob read — comparing
/// hashes is the whole check for `Differs` and `Unknown` alike — so this
/// costs nothing beyond building the narrowed registry the caller already
/// has to build to know what `live` should even be (see the call site: it
/// must be [`replay_surface_specs`]'s output, the surface a replay actually
/// sends, never the bare CLI registry's own specs — no CLI process ever
/// holds `ask_user`, so comparing against it would report `Differs` on
/// nearly every inconclusive probe regardless of whether anything real had
/// changed).
fn annotate_with_fidelity(
    reason: &str,
    recorded_tools_hash: Option<&str>,
    live: &[mecha_core::message::ToolSpec],
) -> String {
    match mecha_core::surface::Fidelity::of(recorded_tools_hash, live).caveat() {
        Some(caveat) => format!("{reason} ({caveat})"),
        None => reason.to_string(),
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
pub(crate) fn replayable(t: mecha_core::learning::Trigger) -> bool {
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
    session_path: &Path,
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
        // By path, not by id: the caller walked `Session::list` to get here,
        // so re-resolving the id would pay a directory scan per intervention
        // for an answer it already holds.
        let prep = match prepare_probe_in(session_path, i.trigger.as_str(), &i.text)? {
            Ok(prep) => prep,
            // A skip is never evidence for either arm, so the error keeps the
            // `Owner`/`None` it was assembled with and stays neutral.
            Err(why) => {
                skipped(&appraisal.session_id, i.at, &why);
                tally.unavailable += 1;
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
            Err(why) => {
                // Driven and lost. Charged to the budget, because the model
                // run happened, and recorded as a skip, because it produced
                // no finding.
                skipped(&appraisal.session_id, i.at, &why);
                tally.unavailable += 1;
                continue;
            }
        };
        if let ProbeVerdict::Inconclusive(reason) = &verdict {
            // Fingerprints the surface `drive_arm` actually sent — the
            // recorded names, narrowed from the live registry and filled
            // from the surface-only stand-in — never the bare CLI registry,
            // which can never hold `ask_user` and so would report `Differs`
            // on nearly every inconclusive probe regardless of whether
            // anything real had changed. A failure here would mean
            // `drive_arm` failed to build this same registry moments ago,
            // which is already handled above as `unavailable`; the only sane
            // fallback for that unreachable case is the reason uncaveated,
            // never a fabricated caveat from an empty surface.
            let why = match replay_surface_specs(
                prep.recorded_tools(),
                prepared.agent.registry(),
                Some(&crate::setup::surface_only_registry()),
                prep.recorded_specs(),
            ) {
                Ok(live) => annotate_with_fidelity(reason, prep.tools_hash(), &live),
                Err(_) => reason.clone(),
            };
            skipped(&appraisal.session_id, i.at, &why);
        }
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

    fn spec(name: &str, description: &str) -> mecha_core::message::ToolSpec {
        mecha_core::message::ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    /// The case that motivates this at all: a probe diverged, and the
    /// surface it replayed against is provably not the one the recording
    /// saw. The reason must say so rather than leaving the mismatch mute.
    #[test]
    fn an_inconclusive_reason_names_a_surface_that_has_since_changed() {
        let recorded_hash = mecha_core::surface::fingerprint(&[spec("build", "Build it.")]);
        let live = [spec("build", "Build it, now with retries.")];
        let why = annotate_with_fidelity("diverged early", Some(&recorded_hash), &live);
        assert!(
            why.contains("diverged early") && why.contains("changed since this was recorded"),
            "{why}"
        );
    }

    /// A recording from before the field existed is `Unknown`, not a match —
    /// and the reason says that too, not silence.
    #[test]
    fn an_inconclusive_reason_names_a_recording_with_no_surface_hash_at_all() {
        let live = [spec("build", "Build it.")];
        let why = annotate_with_fidelity("diverged early", None, &live);
        assert!(
            why.contains("diverged early") && why.contains("before the tool surface was kept"),
            "{why}"
        );
    }

    /// The common case: the surface is unchanged, so the reason stays
    /// exactly what the probe said — no manufactured caveat on a faithful
    /// replay.
    #[test]
    fn an_inconclusive_reason_is_untouched_when_the_surface_still_matches() {
        let recorded_hash = mecha_core::surface::fingerprint(&[spec("build", "Build it.")]);
        let live = [spec("build", "Build it.")];
        let why = annotate_with_fidelity("diverged early", Some(&recorded_hash), &live);
        assert_eq!(why, "diverged early");
    }

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
