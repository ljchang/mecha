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
use mecha_core::comparison::{Arm, Comparison, ComparisonStore, Kind, Outcome, Role};
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
/// **Five ways to produce no finding, counted apart, and the split is the
/// measurement.** Folding them into one `skipped` was the first cut and it hid
/// the thing worth knowing: an intervention with no replayable point is a
/// permanent ceiling on what this mechanism can ever reach, where a budget
/// that ran out is a number somebody chose and a replay that could not be
/// built is a machine that can be fixed. A reader who cannot tell them apart
/// cannot tell a probe worth extending from one worth abandoning — which is
/// the same mistake as a queue reporting its own unreadability as zero, one
/// layer down.
///
/// `Serialize` is what the `--json` readout renders, so a channel added here
/// cannot fall out of it: the first cut of `surface_lost` was incremented and
/// printed nowhere, and a corpus whose surfaces were all gone read as zero on
/// every line — "nothing went wrong" where the truth was "nothing could be
/// measured".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
    /// The recorded surface is gone and no route can rebuild it — a retired
    /// MCP server, in a recording made before the surface store. Kept apart
    /// from `unavailable` because that channel means *fixable*, and filing a
    /// permanent loss there reads as a backlog somebody could clear.
    pub surface_lost: usize,
    /// Never looked at, because the budget ran out first. Says nothing about
    /// the intervention at all.
    pub over_budget: usize,
    /// What the comparison store did with every driven probe's comparison
    /// (row 1g): kept, or refused for provenance — never silently neither.
    pub stored: crate::probe::StoredTally,
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
        self.surface_lost += other.surface_lost;
        self.over_budget += other.over_budget;
        self.stored.add(other.stored);
    }
}

/// A steer probe as a stored comparison: two arms from one recorded prefix.
///
/// The recording, with the owner's intervention, passes by construction —
/// the intervention is the target the structural validator grades against —
/// and the unsteered replay carries the verdict the probe drove. So the
/// derived verdict reads `Separated` (the recording preferred) exactly when
/// the steer was load-bearing, `Tied` when the run got there anyway, and
/// `Inconclusive` when the replay posed no question: [`finding`]'s three
/// answers, in the store's K-arm vocabulary. Both arms ran the recorded
/// prompt, rules block and all, so both carry the recorded rules hash.
pub(crate) fn steer_comparison(
    prep: &crate::probe::ProbePrep,
    i: &Intervention,
    unsteered: &ProbeVerdict,
    model: &str,
) -> Result<Comparison> {
    let policy = prep.recorded_rules_hash();
    prep.comparison(
        Kind::SteerProbe,
        Some(prep.situation_at(&i.tools_before, i.trigger.as_str())),
        vec![
            Arm::new(Role::Recorded, policy.clone(), Outcome::Pass),
            Arm::new(Role::WithoutIntervention, policy, Outcome::from(unsteered)),
        ],
        model,
    )
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
///
/// Every driven probe's comparison is offered to `store` — the recording
/// against its unsteered replay — and the store's provenance door decides
/// whether it is kept (a tainted session, or one whose recorded surface is
/// gone, leaves nothing). A write that fails is an error: the verdict was
/// paid for.
#[allow(clippy::too_many_arguments)]
pub async fn probe_appraisal(
    prepared: &Prepared,
    store: &ComparisonStore,
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
        // Asked before the budget, not after: a recorded tool no route can
        // rebuild fails inside `drive_continuation` at `replay_registry`,
        // before `Agent::new` and so before any provider call, so charging it
        // would spend an allowance this function's contract says is "consumed
        // by drives, never by skips" and count a run that did not happen in
        // `driven`. Asking *after* the budget guard would be worse than not
        // asking: once the allowance is spent every remaining lost surface
        // files as `over_budget` — "never looked at" — and the readout then
        // tells the owner to raise a budget that cannot buy a single one of
        // them. The cost of asking first is one session read for an
        // intervention past the budget, over a file this loop is already
        // walking.
        let lost = prep.lost_recorded_tools(prepared.agent.registry());
        if !lost.is_empty() {
            skipped(
                &appraisal.session_id,
                i.at,
                &format!(
                    "{} recorded tool(s) are gone from this machine's surface ({}{}); \
                     no later night can rebuild them",
                    lost.len(),
                    lost.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
                    if lost.len() > 3 {
                        format!(", and {} more", lost.len() - 3)
                    } else {
                        String::new()
                    }
                ),
            );
            tally.surface_lost += 1;
            continue;
        }
        // Everything past here spends the allowance, so the guard sits below
        // the two questions whose answers do not depend on it.
        if *budget == 0 {
            tally.over_budget += 1;
            continue;
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
        let comparison = steer_comparison(&prep, i, &verdict, model)?;
        crate::probe::store_comparison(store, prep.provenance(), &comparison, &mut tally.stored)?;
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

    /// The five no-finding outcomes answer five different questions, and the
    /// first cut of this file folded them into one counter — which reported a
    /// corpus that is 78% unprobeable as a probe that had merely run out.
    #[test]
    fn every_way_of_producing_no_finding_is_counted_apart() {
        let mut t = Tally::default();
        t.record(Probe::Inconclusive);
        t.unprobeable += 1;
        t.unavailable += 1;
        t.surface_lost += 1;
        t.over_budget += 1;
        assert_eq!(
            (
                t.inconclusive,
                t.unprobeable,
                t.unavailable,
                t.surface_lost,
                t.over_budget
            ),
            (1, 1, 1, 1, 1)
        );
        // Only the inconclusive one cost a model run; `unprobeable` and
        // `surface_lost` are the permanent pair and `unavailable` the fixable
        // one beside them; only the over-budget one is a number somebody chose.
        assert_eq!(t.driven, 0);
    }

    /// `add` folds every channel, including the newest. A merge that drops one
    /// reports a corpus-wide total lower than the sessions that made it, which
    /// reads as a smaller problem rather than a missing summand.
    #[test]
    fn merging_tallies_carries_every_channel() {
        let mut a = Tally {
            surface_lost: 2,
            unavailable: 1,
            ..Default::default()
        };
        a.add(Tally {
            surface_lost: 3,
            unavailable: 4,
            ..Default::default()
        });
        assert_eq!((a.surface_lost, a.unavailable), (5, 5));
    }

    /// A steered session recorded the way `chat` writes one: a run config
    /// naming its surface, the messages, a goal anchor, and a taint
    /// checkpoint after the last message. `surface_kept` writes the recorded
    /// surface's blob into the (moved) home's surface store.
    fn steered_session(
        home: &Path,
        untrusted: bool,
        surface_kept: bool,
    ) -> (std::path::PathBuf, Intervention) {
        use mecha_core::agent::Taint;
        use mecha_core::message::{Block, Message};
        use mecha_core::session::{Record, RunConfig, Session, SessionKind, SessionMeta};
        use serde_json::json;
        let specs = vec![
            mecha_core::message::ToolSpec {
                name: "fs_list".into(),
                description: "List files.".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            },
            mecha_core::message::ToolSpec {
                name: "fs_read".into(),
                description: "Read a file.".into(),
                input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            },
        ];
        if surface_kept {
            mecha_core::surface::SurfaceStore::open_default()
                .unwrap()
                .record(&specs)
                .unwrap();
        }
        let dir = home.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let session = Session::create(
            &dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: home.to_path_buf(),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        session
            .append(&Record::Config(RunConfig {
                tools: specs.iter().map(|s| s.name.clone()).collect(),
                tools_hash: Some(mecha_core::surface::fingerprint(&specs)),
                rules_hash: Some("rules-then".into()),
                rules_surface: Some(SessionKind::Tui),
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some("task:t-audit".parse().unwrap()),
            })
            .unwrap();
        let tool_use = |id: &str, name: &str, input: serde_json::Value| Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        };
        let result = |id: &str, content: &str| Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: false,
        };
        let mut steer = Message::tool_results(vec![result("t1", "a.md b.md"), result("t2", "x")]);
        steer
            .content
            .push(Block::text("change of plan: only summarize b.md"));
        let messages = vec![
            Message::user("audit the reports"),
            Message::assistant(vec![
                tool_use("t1", "fs_list", json!({})),
                tool_use("t2", "fs_read", json!({"path": "/private/a.md"})),
            ]),
            steer,
            Message::assistant(vec![tool_use("t3", "fs_read", json!({"path": "b.md"}))]),
            Message::tool_results(vec![result("t3", "b")]),
            Message::assistant(vec![Block::text("b.md says b")]),
        ];
        for m in &messages {
            session.append(&Record::Message(m.clone())).unwrap();
        }
        session
            .append(&Record::Taint(Taint {
                untrusted,
                private: true,
            }))
            .unwrap();
        let steer = mecha_core::learning::extract_interventions(&messages)
            .into_iter()
            .find(|i| i.trigger == mecha_core::learning::Trigger::Steer)
            .expect("the fixture holds a steer");
        (session.path.clone(), steer)
    }

    /// Row 1g's acceptance, at the probe: a driven steer probe over a clean
    /// session with a readable surface leaves a record a second read — a
    /// fresh handle on the store — returns, keyed on the situation, the
    /// goal kind and the call class, with the arms and the verdict the
    /// probe found; a tainted session leaves none, and neither does one
    /// whose recorded surface is gone. The verdict is supplied rather than
    /// driven: what is under test is the recording's path to the store,
    /// and driving needs a model.
    #[test]
    fn a_driven_steer_probe_leaves_a_record_and_a_tainted_one_leaves_none() {
        use mecha_core::comparison::{ComparisonStore, Kind, Outcome, Role, Validator, Verdict};
        let guard = crate::testenv::HomeGuard::new("comparisons-1g");
        let home = guard.dir.clone();
        let store = ComparisonStore::open_default().unwrap();

        let (clean, i) = steered_session(&home, false, true);
        let prep = crate::probe::prepare_probe_in(&clean, i.trigger.as_str(), &i.text)
            .unwrap()
            .expect("the clean steer prepares");
        let mut stored = crate::probe::StoredTally::default();
        let c = steer_comparison(&prep, &i, &ProbeVerdict::Fail, "scripted").unwrap();
        crate::probe::store_comparison(&store, prep.provenance(), &c, &mut stored).unwrap();

        let (tainted, i2) = steered_session(&home, true, true);
        let prep2 = crate::probe::prepare_probe_in(&tainted, i2.trigger.as_str(), &i2.text)
            .unwrap()
            .expect("a tainted steer still prepares — the store is what refuses it");
        let c2 = steer_comparison(&prep2, &i2, &ProbeVerdict::Fail, "scripted").unwrap();
        crate::probe::store_comparison(&store, prep2.provenance(), &c2, &mut stored).unwrap();

        // Written last, so the blob the clean fixtures cite is still there:
        // this one cites the same surface, so remove the store to lose it.
        std::fs::remove_dir_all(home.join("surfaces")).unwrap();
        let (lost, i3) = steered_session(&home, false, false);
        let prep3 = crate::probe::prepare_probe_in(&lost, i3.trigger.as_str(), &i3.text)
            .unwrap()
            .expect("prepares with no blob");
        let c3 = steer_comparison(&prep3, &i3, &ProbeVerdict::Pass, "scripted").unwrap();
        crate::probe::store_comparison(&store, prep3.provenance(), &c3, &mut stored).unwrap();

        assert_eq!(
            (
                stored.written,
                stored.refused_not_clean,
                stored.refused_surface
            ),
            (1, 1, 1)
        );
        let rows = ComparisonStore::open_default()
            .unwrap()
            .comparisons()
            .unwrap();
        assert_eq!(rows.len(), 1, "only the clean, readable session: {rows:?}");
        let row = &rows[0];
        assert_eq!(row, &c);
        assert_eq!(row.kind, Kind::SteerProbe);
        assert_eq!(row.validator, Validator::StructuralSteer);
        assert_eq!(row.verdict, Verdict::Separated, "Fail = load-bearing");
        assert_eq!(row.preferred, vec![0]);
        assert_eq!(
            row.arms
                .iter()
                .map(|a| (a.role, a.outcome))
                .collect::<Vec<_>>(),
            vec![
                (Role::Recorded, Outcome::Pass),
                (Role::WithoutIntervention, Outcome::Fail)
            ]
        );
        assert!(row
            .arms
            .iter()
            .all(|a| a.policy.as_deref() == Some("rules-then")));
        assert_eq!(row.goal_kind, Some(mecha_core::goal::GoalKind::Task));
        let situation = row.situation.as_ref().expect("the steer's window is known");
        assert_eq!(
            situation.scope().tools,
            vec!["fs_list".to_string(), "fs_read".to_string()]
        );
        assert_eq!(
            situation.surface,
            Some(mecha_core::session::SessionKind::Tui)
        );
        let call = row.call.as_ref().expect("the call the steer rode beside");
        assert_eq!(
            (call.tool.as_str(), call.args.as_slice()),
            ("fs_read", &["path".to_string()][..])
        );
        assert_eq!(row.pointers.message_index, Some(2));
        assert_eq!(row.pointers.call_index, Some(2));
        let wire = std::fs::read_to_string(store.root().join("comparisons.jsonl")).unwrap();
        for leaked in [
            "/private/a.md",
            "change of plan",
            "audit the reports",
            "t-audit",
        ] {
            assert!(!wire.contains(leaked), "{leaked} reached the store");
        }
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
