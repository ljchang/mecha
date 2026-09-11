//! The counterfactual replay probe, shared by `mecha validate` and the
//! proposal gate in `mecha learn --propose`.
//!
//! One reflection, two arms: resubmit the recorded run up to the intervention
//! verbatim (recorded messages and tool results, no steering text), let the
//! model continue from the branch point once per system prompt, and grade
//! each trace with [`mecha_core::counterfactual`]. The caller chooses what
//! the arms mean — validate compares no-rules against the live rules; the
//! gate compares the live rules against a candidate set that exists nowhere
//! but in memory yet.
//!
//! **Branched, never regenerated.** An earlier version drove the recorded
//! user turns from scratch and required the model to reproduce every call
//! before the intervention exactly; on the live store that lost 11 of 12
//! steer probes to pre-point divergence (typically at call #1, against
//! points at #10–#28), because every open choice before the point is a
//! chance to fork. [`mecha_core::counterfactual::branch_at`] and
//! [`mecha_core::replay_run::drive_branch`] make pre-point divergence
//! structurally impossible instead.
//!
//! Split into [`prepare_probe`] (load and slice the recording, once) and
//! [`drive_arm`] (one system prompt, one drive, one verdict) so a caller can
//! drive *more* than two arms against the same prefix — which is what the
//! bisection in `mecha validate` does when it hunts the rule behind a
//! regression.

use crate::setup::Prepared;
use anyhow::Result;
use mecha_core::agent::{Agent, RunContext};
use mecha_core::config::{PermissionMode, ProviderConfig};
use mecha_core::counterfactual::{
    branch_at, locate_denial, locate_steer, truncate_after_run, verdict, Branch, ProbePoint,
    ProbeVerdict,
};
use mecha_core::learning::{strip_rules_block, Reflexion, Trigger};
use mecha_core::replay::{extract, Trajectory};
use mecha_core::replay_run::{drive_branch, replay_registry, OnDivergence};
use mecha_core::session::{RunConfig, Session};
use mecha_core::situation::Situation;
use mecha_core::tool::ModeApprover;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// What probing one reflection produced.
pub enum ProbeResult {
    /// Baseline arm first, treatment arm second.
    Verdicts(ProbeVerdict, ProbeVerdict),
    /// The probe could not be run; the reason is for the human reading the
    /// report. A skip is never evidence for either arm.
    Skipped(String),
}

/// A reflection's recorded prefix, loaded and sliced once, ready to drive
/// under any number of system prompts.
pub struct ProbePrep {
    method: ProbeMethod,
    recorded: RunConfig,
    /// The exact specs the recording was sent, when the surface store still
    /// holds the blob its `tools_hash` cites. Empty for recordings from
    /// before the store. Loaded once here because bisection drives many arms
    /// off one prep.
    recorded_specs: Vec<mecha_core::message::ToolSpec>,
    base_system: String,
    recorded_system: String,
}

enum ProbeMethod {
    Followup {
        trajectory: Trajectory,
        branch: Branch,
    },
    Trace {
        trajectory: Trajectory,
        point: ProbePoint,
        branch: Branch,
    },
    Artifact {
        case: mecha_core::mismatch::ArtifactCase,
        session_id: String,
        reflection_id: String,
    },
}

impl ProbePrep {
    /// Identity of the recorded inputs; changing a transcript or config makes
    /// a fresh measurement even when the reflection and rules are unchanged.
    pub fn input_hash(&self) -> Result<String> {
        let inputs = match &self.method {
            ProbeMethod::Trace {
                trajectory, branch, ..
            }
            | ProbeMethod::Followup { trajectory, branch } => serde_json::json!({
                "recorded": self.recorded, "specs": self.recorded_specs,
                "seed": branch.seed, "base": branch.call_base, "trajectory": trajectory,
            }),
            ProbeMethod::Artifact { case, .. } => {
                serde_json::json!({"recorded": self.recorded, "case": case})
            }
        };
        Ok(mecha_core::learning::rules_hash(&serde_json::to_string(
            &inputs,
        )?))
    }

    /// The situation the recorded run was in: the registry its `RunConfig`
    /// names and the workspace its rules block was matched against — not
    /// the jail, which on `serve` and Slack is a different path. What a
    /// rules block for this probe is rendered against.
    pub fn situation(&self) -> Situation {
        Situation::of_run(
            &self.recorded.tools,
            self.recorded.rules_workspace.as_deref(),
        )
        .on(self.recorded.rules_surface)
    }

    /// The tool names the recording carried — what a fidelity check must
    /// narrow a live registry down to before fingerprinting it, never the
    /// live registry's own full name list.
    pub fn recorded_tools(&self) -> &[String] {
        &self.recorded.tools
    }

    /// The recorded system prompt **verbatim**, rules block and all.
    ///
    /// The arm an appraisal probe wants, and the difference is not cosmetic.
    /// `validate` asks *which rule set does better*, so its arms must carry
    /// exactly one generation's block and [`Self::system_with`] strips the
    /// recorded one to guarantee it. An appraisal asks a different question —
    /// *would this run, as it actually was, have got there unprompted* — and
    /// answering it with the rules removed replays a **weaker agent than the
    /// one that ran**, which diverges more readily and would bias every
    /// verdict toward `Mattered`. That inflates regret out of an artifact of
    /// the harness, in the field a label is derived from.
    pub fn system_as_recorded(&self) -> String {
        self.recorded_system.clone()
    }

    /// The recorded prompt stripped of its own rules block, plus `block`
    /// (`None` = rules-free). An arm must carry exactly the block it was
    /// given, not a mixture of generations.
    pub fn system_with(&self, block: Option<&str>) -> String {
        match block {
            None => self.base_system.clone(),
            Some(b) if self.base_system.is_empty() => b.to_string(),
            Some(b) => format!("{}\n\n{b}", self.base_system),
        }
    }

    /// The tool-surface hash the recording carried, if any — feeds
    /// [`mecha_core::surface::Fidelity::of`] against a live registry. `None`
    /// is a recording from before the field existed, not a match.
    pub fn tools_hash(&self) -> Option<&str> {
        self.recorded.tools_hash.as_deref()
    }

    /// The recorded specs themselves, when the surface store still holds
    /// them — what a fidelity check must hand [`replay_surface_specs`] so it
    /// fingerprints the surface a replay would actually send.
    pub fn recorded_specs(&self) -> &[mecha_core::message::ToolSpec] {
        &self.recorded_specs
    }
}

/// Load the recording behind a steer/denial reflection. `Err(reason)` in the
/// inner result is a skip — never evidence for either arm.
pub fn prepare_probe(sessions_dir: &Path, r: &Reflexion) -> Result<Result<ProbePrep, String>> {
    if r.trigger == Trigger::Mismatch.as_str() {
        return prepare_mismatch(sessions_dir, r);
    }
    prepare_probe_at(sessions_dir, &r.session_id, &r.trigger, &r.intervention)
}

/// The same, addressed by what a probe point actually needs rather than by the
/// record that happens to carry it.
///
/// A [`Reflexion`] is one of two things that names an intervention; a
/// [`mecha_core::learning::Intervention`] read straight off a transcript is the
/// other, and the appraisal probe has only the second. Nothing in the slicing
/// below ever wanted the reflection — it wanted a session, a trigger and the
/// text to match — so the narrower signature is what the function was already
/// doing.
pub fn prepare_probe_at(
    sessions_dir: &Path,
    session_id: &str,
    trigger: &str,
    intervention: &str,
) -> Result<Result<ProbePrep, String>> {
    let path = match Session::find(sessions_dir, session_id) {
        Ok(p) => p,
        Err(_) => return Ok(Err(format!("session {session_id} not found"))),
    };
    prepare_probe_in(&path, trigger, intervention)
}

/// The same, for a caller that already holds the transcript's path — the
/// appraisal probe walks `Session::list` itself, so re-resolving the id here
/// paid a full directory scan per intervention for an answer the caller had.
///
/// One `Session::read` per call, deliberately: the messages and every run
/// config come out of the same pass, where the first cut paid `Session::load`
/// plus a *second* full parse for `run_configs` — the three-reads mistake
/// `Session::read`'s own doc names — and, worse, propagated a failure of that
/// second read as `Err`, aborting a whole corpus walk that skips per-item
/// everywhere else. One read means one failure mode, and it is a skip.
pub fn prepare_probe_in(
    path: &Path,
    trigger: &str,
    intervention: &str,
) -> Result<Result<ProbePrep, String>> {
    let transcript = match Session::read(path) {
        Ok(t) => t,
        Err(e) => return Ok(Err(format!("session unreadable: {e:#}"))),
    };
    let messages = &transcript.convo.messages;
    let (method, message_index) = if trigger == Trigger::Followup.as_str() {
        let Some((index, branch)) =
            mecha_core::counterfactual::followup_branch(messages, intervention)
        else {
            return Ok(Err("could not locate the corrective turn".into()));
        };
        let trajectory = extract(truncate_after_run(messages, index));
        (ProbeMethod::Followup { trajectory, branch }, index)
    } else {
        let point = if trigger == Trigger::Steer.as_str() {
            locate_steer(messages, intervention)
        } else if trigger == Trigger::Denial.as_str() {
            locate_denial(messages, intervention)
        } else {
            // An `edit` reflection's intervention lives in an outbox item, not
            // in any transcript — there is no prefix to replay. Explicit, so a
            // new trigger kind cannot silently be probed as if it were a denial.
            return Ok(Err(format!(
                "`{trigger}` interventions have no replayable intervention point"
            )));
        };
        let Some(point) = point else {
            return Ok(Err("could not locate the intervention".into()));
        };
        let slice = truncate_after_run(messages, point.message_index);
        let trajectory = extract(slice);
        // A prefix that cannot be replayed, or one with no user turn before
        // the intervention, is unmeasurable rather than a failing arm: a
        // probe that cannot be set up is not evidence that the rules failed.
        if let Err(error) = trajectory.ensure_replayable() {
            return Ok(Err(error.to_string()));
        }
        if trajectory.turns.is_empty() {
            return Ok(Err("no user turns before the intervention".into()));
        }
        let Some(branch) = branch_at(slice, &point) else {
            return Ok(Err("could not rebuild the branch prefix".into()));
        };
        let index = point.message_index;
        (
            ProbeMethod::Trace {
                trajectory,
                point,
                branch,
            },
            index,
        )
    };
    // The config in effect *at the intervention*, not `first()`: a resumed
    // session's later attach ran under its own system prompt and tool list,
    // and replaying its turns under the first attach's diverges for reasons
    // that say nothing about the steer — which a counterfactual verdict then
    // reads as `Mattered`, inflating regret out of an artifact of the replay.
    let Some(recorded) = transcript.config_covering(message_index).cloned() else {
        return Ok(Err("no RunConfig recorded".into()));
    };
    if recorded.appraisal_evidence.is_some() {
        return Ok(Err(
            "owner-bound anticipatory evidence is not yet reproduced by counterfactual probes"
                .into(),
        ));
    }

    // The recorded system prompt with any rules block of its era removed: an
    // arm must carry exactly the block it was given, not a mixture of
    // generations.
    let recorded_system = recorded.system_prompt.clone().unwrap_or_default();
    let base_system = strip_rules_block(&recorded_system);
    // The recorded surface, recovered from the store the recording cites. An
    // unreadable store or a missing blob degrades to "no specs" — the replay
    // then offers today's surface exactly as it always did, and the fidelity
    // check labels the gap rather than this silently pretending to a match.
    let recorded_specs = recorded
        .tools_hash
        .as_deref()
        .and_then(|h| mecha_core::surface::SurfaceStore::open_default()?.load(h))
        .unwrap_or_default();
    Ok(Ok(ProbePrep {
        method,
        recorded,
        recorded_specs,
        base_system,
        recorded_system,
    }))
}

fn prepare_mismatch(sessions_dir: &Path, r: &Reflexion) -> Result<Result<ProbePrep, String>> {
    let attempt = || -> Result<ProbePrep> {
        use mecha_core::{
            learning::{classify_origin, Origin},
            planning::StepFeedback,
        };
        anyhow::ensure!(
            r.provenance() == Origin::Clean && r.learnable(),
            "mismatch reflection is not clean eligible evidence"
        );
        let path = Session::find(sessions_dir, &r.session_id)?;
        let transcript = Session::read(&path)?;
        let expected: StepFeedback = serde_json::from_str(&r.context)?;
        anyhow::ensure!(expected.mismatch(), "reflection names no mismatch");
        let found: Vec<_> = transcript
            .convo
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.planning
                    .as_ref()
                    .is_some_and(|f| f.steps.contains(&expected))
            })
            .collect();
        anyhow::ensure!(
            found.len() == 1,
            "mismatch evidence is absent or ambiguous in transcript"
        );
        let at = found[0].0;
        anyhow::ensure!(
            classify_origin(transcript.taint_timeline.covering(at)) == Origin::Clean,
            "recorded mismatch provenance is not clean"
        );
        let recorded = transcript
            .config_covering(at)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("mismatch has no recorded config"))?;
        let case = recorded.mismatch_case.clone().ok_or_else(|| {
            anyhow::anyhow!("no owner-supplied artifact fixture was bound before this run")
        })?;
        case.validate()?;
        case.validate_feedback(&expected)?;
        anyhow::ensure!(
            expected.goal.as_ref().map(ToString::to_string).as_deref() == Some(case.goal.as_str()),
            "mismatch goal differs from artifact case"
        );
        anyhow::ensure!(
            transcript.configs.len() == 1 && transcript.config_positions == [0],
            "artifact validation supports fresh single-run recordings only"
        );
        let first = transcript
            .convo
            .messages
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing task prompt"))?;
        anyhow::ensure!(
            first == &mecha_core::message::Message::user(&case.prompt),
            "recorded task differs from fixture"
        );
        mecha_core::mismatch::validate_recording(&recorded)?;
        let registry = mecha_core::mismatch::registry(&recorded.tools)?;
        let recorded_system = recorded.system_prompt.clone().unwrap_or_default();
        Ok(ProbePrep {
            base_system: strip_rules_block(&recorded_system),
            recorded_system,
            recorded_specs: registry.specs(),
            recorded,
            method: ProbeMethod::Artifact {
                case,
                session_id: r.session_id.clone(),
                reflection_id: r.id.clone(),
            },
        })
    };
    Ok(attempt().map_err(|e| format!("artifact mismatch probe unavailable: {e:#}")))
}

/// Drive the prepared prefix once under `system` and grade the trace.
///
/// The prompt arrives **resolved** rather than as a rules block to append,
/// because there is more than one right answer now: [`ProbePrep::system_with`]
/// is validate's, and [`ProbePrep::system_as_recorded`] is the appraisal
/// probe's. Deciding it here would mean this function knowing which caller it
/// had, which is the thing it was split apart to avoid.
pub async fn drive_arm(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    prep: &ProbePrep,
    system: String,
) -> Result<Result<ProbeVerdict, String>> {
    let recorded = &prep.recorded;
    if let ProbeMethod::Artifact {
        case,
        session_id,
        reflection_id,
    } = &prep.method
    {
        for lever in [
            mecha_core::harness::Lever::Hooks,
            mecha_core::harness::Lever::Outbox,
            mecha_core::harness::Lever::Messages,
        ] {
            if !prepared.levers_off.contains(&lever) {
                return Ok(Err(format!(
                    "artifact probes require {lever:?} disabled, as in the recording"
                )));
            }
        }
        let mut cfg = prepared.config.agent.clone();
        let system_hash = mecha_core::learning::rules_hash(&system);
        cfg.system_prompt = Some(system);
        cfg.system_prompt_file = None;
        cfg.effort = recorded.effort;
        cfg.thinking = recorded.thinking;
        cfg.cache_prompt = recorded.cache_prompt;
        cfg.max_tokens = recorded.max_tokens;
        cfg.max_turns = recorded.max_turns;
        cfg.max_output_tokens = recorded.max_output_tokens;
        cfg.max_cost_usd = recorded.max_cost_usd;
        cfg.compact_at_tokens = recorded.compact_at_tokens;
        cfg.compact_keep_recent = recorded.compact_keep_recent;
        cfg.goal_guidance = false;
        cfg.step_escalation = false;
        cfg.step_checks = !recorded
            .levers_off
            .as_ref()
            .is_some_and(|off| off.contains(&mecha_core::harness::Lever::StepChecks));
        let mut provider_cfg = provider_cfg.clone();
        provider_cfg.seed = recorded.seed;
        provider_cfg.temperature = recorded.temperature;
        let (verdict, stats) = match mecha_core::mismatch::drive(
            case,
            recorded,
            mecha_core::provider::build(&provider_cfg)?,
            cfg,
            model,
            prepared.agent.context(),
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return Ok(Err(format!("artifact task repeat failed: {e:#}"))),
        };
        let store = mecha_core::learning::LearningStore::open(
            mecha_core::learning::LearningStore::default_root()?,
        )?;
        let dir = store.root().join("artifact-probes");
        std::fs::create_dir_all(&dir)?;
        let receipt = serde_json::json!({"method":"artifact_task_repeat","session_id":session_id,"reflection_id":reflection_id,"model":model,"case_hash":mecha_core::learning::rules_hash(&serde_json::to_string(case)?),"system_hash":system_hash,"verdict":format!("{verdict:?}"),"stats":stats,"created_at":chrono::Utc::now().to_rfc3339()});
        std::fs::write(
            dir.join(format!("{}.json", Session::new_id())),
            serde_json::to_vec_pretty(&receipt)?,
        )?;
        return Ok(Ok(verdict));
    }
    let ProbeMethod::Trace { point, .. } = &prep.method else {
        return Ok(Err(
            "followups require the judge-graded continuation path".into()
        ));
    };
    Ok(
        drive_continuation(prepared, provider_cfg, model, prep, system)
            .await?
            .map(|report| {
                let result = verdict(&report, point);
                tracing::debug!(?result, calls = ?report.replayed_calls,
                    diffs = ?report.divergences, text = %report.final_text, "probe arm");
                result
            }),
    )
}

/// Non-executing continuation used by both trace and followup probes.
/// Followups retain their user correction and expose the recorded tools.
pub async fn drive_continuation(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    prep: &ProbePrep,
    system: String,
) -> Result<Result<mecha_core::replay_run::ReplayReport, String>> {
    let recorded = &prep.recorded;
    let (trajectory, branch) = match &prep.method {
        ProbeMethod::Trace {
            trajectory, branch, ..
        }
        | ProbeMethod::Followup { trajectory, branch } => (trajectory, branch),
        ProbeMethod::Artifact { .. } => return Ok(Err("artifact probes do not replay".into())),
    };
    let cancel = CancellationToken::new();
    // The recording the registry answers from starts at the branch base: the
    // calls before it were already resolved inside the forced prefix, and
    // handing them to the cursor again would answer the first regenerated
    // call with a result the model has already read.
    let tail_calls = trajectory
        .calls
        .get(branch.call_base..)
        .unwrap_or_default()
        .to_vec();
    let registry = match replay_registry(
        &recorded.tools,
        prepared.agent.registry(),
        // `ask_user` is registered only by a front-end that owns a human, so
        // no CLI registry has ever held it — and it is on the recorded surface
        // of every interactive session, which is every session that contains a
        // steer. Without this, every steer and denial probe skips.
        Some(&crate::setup::surface_only_registry()),
        &prep.recorded_specs,
        tail_calls,
        OnDivergence::Stop,
        cancel.clone(),
    ) {
        Ok(reg) => reg,
        Err(e) => return Ok(Err(format!("{e:#}"))),
    };
    // Nothing executes under Stop mode, so nothing needs approving.
    let approver: Arc<dyn mecha_core::tool::Approver> = Arc::new(ModeApprover {
        mode: PermissionMode::Allow,
    });
    let mut agent_cfg = prepared.config.agent.clone();
    agent_cfg.system_prompt = (!system.is_empty()).then_some(system);
    agent_cfg.system_prompt_file = None;
    agent_cfg.effort = recorded.effort;
    agent_cfg.thinking = recorded.thinking;
    agent_cfg.cache_prompt = recorded.cache_prompt;
    agent_cfg.max_tokens = recorded.max_tokens;
    agent_cfg.max_turns = recorded.max_turns;
    agent_cfg.max_output_tokens = recorded.max_output_tokens;
    agent_cfg.max_cost_usd = recorded.max_cost_usd;
    agent_cfg.compact_at_tokens = recorded.compact_at_tokens;
    agent_cfg.compact_keep_recent = recorded.compact_keep_recent;

    let mut tool_ctx = mecha_core::tool::ToolCtx {
        workspace: recorded.workspace.clone(),
        ..Default::default()
    };
    if !recorded.workspace.exists() {
        // Fine for a pure replay: nothing touches the filesystem.
        tool_ctx.workspace = std::env::temp_dir();
    }

    let agent = Agent::new(
        mecha_core::provider::build(provider_cfg)?,
        registry,
        Arc::clone(&approver),
        tool_ctx.clone(),
        agent_cfg,
        Some(model.to_string()),
    )?;
    let cx = RunContext::new(tool_ctx, approver)
        .with_cancel(cancel)
        .with_compact_at(recorded.compact_at_tokens);
    match drive_branch(
        &agent,
        &cx,
        branch.seed.clone(),
        trajectory,
        branch.call_base,
    )
    .await
    {
        Ok(report) => Ok(Ok(report)),
        Err(e) => Ok(Err(format!("replay failed: {e:#}"))),
    }
}

/// The (baseline, treatment) rules blocks a probe's two arms carry — each
/// appended to the recorded system prompt (stripped of any rules block of
/// its own era); `None` means that arm runs rules-free.
pub type Arms = (Option<String>, Option<String>);

/// Probe one steer/denial reflection by counterfactual replay: drive both
/// arms over `r`'s recorded session.
///
/// `arms` renders the (baseline, treatment) rules blocks **for the situation
/// the recorded run was in** — the block a run carries is a function of its
/// registry once rules are scoped, so a block rendered from the whole store
/// would measure a set no run has. The probe knows the situation only after
/// it has read the transcript, which is why this takes a renderer and not
/// two strings.
pub async fn probe_reflection(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    sessions_dir: &Path,
    r: &Reflexion,
    arms: &dyn Fn(&Situation) -> Result<Arms>,
) -> Result<ProbeResult> {
    let prep = match prepare_probe(sessions_dir, r)? {
        Ok(prep) => prep,
        Err(why) => return Ok(ProbeResult::Skipped(why)),
    };
    let (baseline_block, treatment_block) = arms(&prep.situation())?;
    // Two identical arms measure nothing and would grade as "unchanged" —
    // a verdict the caller counts. Reachable once rules are scoped: a
    // candidate whose new rules match no tool the recorded run carried
    // renders the same block as the current set.
    if baseline_block == treatment_block {
        return Ok(ProbeResult::Skipped(
            "the candidate changes nothing in the recorded run's situation".into(),
        ));
    }
    let mut verdicts = Vec::new();
    for block in [baseline_block, treatment_block] {
        match drive_arm(
            prepared,
            provider_cfg,
            model,
            &prep,
            prep.system_with(block.as_deref()),
        )
        .await?
        {
            Ok(v) => verdicts.push(v),
            Err(why) => return Ok(ProbeResult::Skipped(why)),
        }
    }
    let treatment = verdicts.pop().expect("two arms drove");
    let baseline = verdicts.pop().expect("two arms drove");
    Ok(ProbeResult::Verdicts(baseline, treatment))
}

/// Fold a pair of arm verdicts into the label the reports print, updating the
/// caller's counters. Returns `None` for inconclusive probes, which carry
/// their own explanation.
pub fn compare(
    baseline: &ProbeVerdict,
    treatment: &ProbeVerdict,
    improved: &mut u32,
    regressed: &mut u32,
    unchanged: &mut u32,
    inconclusive: &mut u32,
) -> Option<&'static str> {
    match (baseline, treatment) {
        (ProbeVerdict::Inconclusive(_), _) | (_, ProbeVerdict::Inconclusive(_)) => {
            *inconclusive += 1;
            None
        }
        (ProbeVerdict::Fail, ProbeVerdict::Pass) => {
            *improved += 1;
            Some("IMPROVED")
        }
        (ProbeVerdict::Pass, ProbeVerdict::Fail) => {
            *regressed += 1;
            Some("REGRESSED")
        }
        (ProbeVerdict::Pass, _) => {
            *unchanged += 1;
            Some("unchanged (both pass)")
        }
        (ProbeVerdict::Fail, _) => {
            *unchanged += 1;
            Some("unchanged (both fail)")
        }
    }
}

#[cfg(test)]
mod mismatch_tests {
    use super::*;
    use mecha_core::{
        agent::Taint,
        harness::Lever,
        message::Message,
        planning::{Feedback, StepFeedback, Verification},
        session::{Record, SessionMeta},
    };
    fn fixture(
        clean: Option<bool>,
        artifact: bool,
        criterion: bool,
    ) -> (mecha_core::mismatch::Workspace, Reflexion) {
        let root = mecha_core::mismatch::Workspace::new().unwrap();
        let mut case:mecha_core::mismatch::ArtifactCase=serde_json::from_value(serde_json::json!({"prompt":"write answer.json","goal":"task:x","files":{},"artifacts":{"answer.json":{"ok":true}}})).unwrap();
        if criterion {
            case.criteria.insert(
                "result".into(),
                mecha_core::mismatch::Criterion {
                    artifact: "answer.json".into(),
                    pointer: "/ok".into(),
                    context: None,
                },
            );
        }
        let criterion_step = case
            .criterion_feedback(root.path())
            .unwrap()
            .into_iter()
            .next();
        let tools = vec!["fs_write".into(), "todo".into()];
        let specs = mecha_core::mismatch::registry(&tools).unwrap().specs();
        let mut cfg = RunConfig {
            tools,
            tools_hash: Some(mecha_core::surface::fingerprint(&specs)),
            levers_off: Some(Lever::ALL.into_iter().collect()),
            ..Default::default()
        };
        if artifact {
            cfg.mismatch_case = Some(case);
        }
        let session = Session::create(
            root.path(),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: root.path().into(),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        session.append(&Record::Config(cfg)).unwrap();
        session
            .append(&Record::Message(Message::user("write answer.json")))
            .unwrap();
        let step = criterion_step.unwrap_or(StepFeedback {
            criterion: None,
            completion_batch: None,
            call_id: Some("todo-1".into()),
            step: "produce the artifact".into(),
            goal: Some("task:x".parse().unwrap()),
            expected: None,
            expected_calls: Some(1),
            actual_calls: Some(8),
            verification: Verification::NotDeclared,
            check_tampered: false,
        });
        let mut feedback = Message::user("plan feedback");
        feedback.planning = Some(Feedback {
            steps: vec![step.clone()],
            ..Default::default()
        });
        session.append(&Record::Message(feedback)).unwrap();
        if let Some(clean) = clean {
            session
                .append(&Record::Taint(Taint {
                    untrusted: !clean,
                    private: true,
                }))
                .unwrap();
        }
        let r=serde_json::from_value(serde_json::json!({"id":"reflection","session_id":session.meta.id,"domain":"behavior","trigger":"mismatch","context":serde_json::to_string(&step).unwrap(),"intervention":"observed mismatch","reflexion_text":"verify output","created_at":"now","origin":"clean","evidence":"full"})).unwrap();
        (root, r)
    }
    #[test]
    fn trusted_mismatch_prepares_but_missing_or_untrusted_evidence_does_not() {
        let (root, mut r) = fixture(Some(true), true, false);
        assert!(
            prepare_probe_at(root.path(), &r.session_id, &r.trigger, &r.intervention)
                .unwrap()
                .is_err(),
            "old trace-only dispatch cannot grade this mismatch"
        );
        assert!(
            prepare_probe(root.path(), &r).unwrap().is_ok(),
            "new artifact dispatch accepts the same evidence"
        );
        let mut step: StepFeedback = serde_json::from_str(&r.context).unwrap();
        step.goal = Some("task:other".parse().unwrap());
        r.context = serde_json::to_string(&step).unwrap();
        assert!(prepare_probe(root.path(), &r).unwrap().is_err());
        for (clean, artifact) in [(None, true), (Some(false), true), (Some(true), false)] {
            let (root, r) = fixture(clean, artifact, false);
            assert!(prepare_probe(root.path(), &r).unwrap().is_err());
        }
    }
    #[test]
    fn owner_criterion_failure_requires_its_recorded_clean_contract() {
        for clean in [Some(true), Some(false), None] {
            let (root, r) = fixture(clean, true, true);
            assert_eq!(
                prepare_probe(root.path(), &r).unwrap().is_ok(),
                clean == Some(true)
            );
        }
    }
}
