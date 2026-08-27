//! The counterfactual replay probe, shared by `mecha validate` and the
//! proposal gate in `mecha learn --propose`.
//!
//! One reflection, two arms: rebuild the recorded run up to the intervention
//! (recorded tool results, no steering text), drive it once per system
//! prompt, and grade each trace with [`mecha_core::counterfactual`]. The
//! caller chooses what the arms mean — validate compares no-rules against the
//! live rules; the gate compares the live rules against a candidate set that
//! exists nowhere but in memory yet.
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
    locate_denial, locate_steer, truncate_after_run, verdict, ProbePoint, ProbeVerdict,
};
use mecha_core::learning::{strip_rules_block, Reflexion, Trigger};
use mecha_core::replay::{extract, Trajectory};
use mecha_core::replay_run::{drive, replay_registry, OnDivergence};
use mecha_core::session::{RunConfig, Session};
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
    trajectory: Trajectory,
    point: ProbePoint,
    recorded: RunConfig,
    base_system: String,
    recorded_system: String,
}

impl ProbePrep {
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
}

/// Load the recording behind a steer/denial reflection. `Err(reason)` in the
/// inner result is a skip — never evidence for either arm.
pub fn prepare_probe(sessions_dir: &Path, r: &Reflexion) -> Result<Result<ProbePrep, String>> {
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
    let (_, convo) = match Session::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => return Ok(Err(format!("session unreadable: {e:#}"))),
    };
    let point = if trigger == Trigger::Steer.as_str() {
        locate_steer(&convo.messages, intervention)
    } else if trigger == Trigger::Denial.as_str() {
        locate_denial(&convo.messages, intervention)
    } else {
        // An `edit` reflection's intervention lives in an outbox item, not in
        // any transcript — there is no prefix to replay. Explicit, so a new
        // trigger kind cannot silently be probed as if it were a denial.
        return Ok(Err(format!(
            "`{trigger}` interventions have no replayable intervention point"
        )));
    };
    let Some(point) = point else {
        return Ok(Err("could not locate the intervention".into()));
    };
    let slice = truncate_after_run(&convo.messages, point.message_index);
    let trajectory = extract(slice);
    if trajectory.turns.is_empty() {
        return Ok(Err("no user turns before the intervention".into()));
    }
    let Some(recorded) = Session::run_configs(&path)?.first().cloned() else {
        return Ok(Err("no RunConfig recorded".into()));
    };

    // The recorded system prompt with any rules block of its era removed: an
    // arm must carry exactly the block it was given, not a mixture of
    // generations.
    let recorded_system = recorded.system_prompt.clone().unwrap_or_default();
    let base_system = strip_rules_block(&recorded_system);
    Ok(Ok(ProbePrep {
        trajectory,
        point,
        recorded,
        base_system,
        recorded_system,
    }))
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
    let cancel = CancellationToken::new();
    let registry = match replay_registry(
        &recorded.tools,
        prepared.agent.registry(),
        // `ask_user` is registered only by a front-end that owns a human, so
        // no CLI registry has ever held it — and it is on the recorded surface
        // of every interactive session, which is every session that contains a
        // steer. Without this, every steer and denial probe skips.
        Some(&crate::setup::surface_only_registry()),
        prep.trajectory.calls.clone(),
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
    match drive(&agent, &cx, &prep.trajectory).await {
        // Which rule grades this is carried by the point itself, so a kind
        // the caller has not heard of cannot be graded as a denial by default.
        Ok(report) => Ok(Ok(verdict(&report, &prep.point))),
        Err(e) => Ok(Err(format!("replay failed: {e:#}"))),
    }
}

/// Probe one steer/denial reflection by counterfactual replay.
///
/// `baseline_block` / `treatment_block` are rules blocks appended to the
/// recorded system prompt (stripped of any rules block of its own era) —
/// `None` means that arm runs rules-free.
pub async fn probe_reflection(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    sessions_dir: &Path,
    r: &Reflexion,
    baseline_block: Option<&str>,
    treatment_block: Option<&str>,
) -> Result<ProbeResult> {
    let prep = match prepare_probe(sessions_dir, r)? {
        Ok(prep) => prep,
        Err(why) => return Ok(ProbeResult::Skipped(why)),
    };
    let mut verdicts = Vec::new();
    for block in [baseline_block, treatment_block] {
        match drive_arm(
            prepared,
            provider_cfg,
            model,
            &prep,
            prep.system_with(block),
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
