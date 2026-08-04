//! The counterfactual replay probe, shared by `mecha validate` and the
//! proposal gate in `mecha learn --propose`.
//!
//! One reflection, two arms: rebuild the recorded run up to the intervention
//! (recorded tool results, no steering text), drive it once per system
//! prompt, and grade each trace with [`mecha_core::counterfactual`]. The
//! caller chooses what the arms mean — validate compares no-rules against the
//! live rules; the gate compares the live rules against a candidate set that
//! exists nowhere but in memory yet.

use crate::setup::Prepared;
use anyhow::Result;
use mecha_core::agent::{Agent, RunContext};
use mecha_core::config::{PermissionMode, ProviderConfig};
use mecha_core::counterfactual::{denial_verdict, locate_denial, locate_steer, steer_verdict, truncate_after_run, ProbeVerdict};
use mecha_core::learning::{strip_rules_block, Reflexion, Trigger};
use mecha_core::replay::extract;
use mecha_core::replay_run::{drive, replay_registry, OnDivergence};
use mecha_core::session::Session;
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
    let path = match Session::find(sessions_dir, &r.session_id) {
        Ok(p) => p,
        Err(_) => return Ok(ProbeResult::Skipped(format!("session {} not found", r.session_id))),
    };
    let (_, convo) = match Session::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => return Ok(ProbeResult::Skipped(format!("session unreadable: {e:#}"))),
    };
    let point = if r.trigger == Trigger::Steer.as_str() {
        locate_steer(&convo.messages, &r.intervention)
    } else if r.trigger == Trigger::Denial.as_str() {
        locate_denial(&convo.messages, &r.intervention)
    } else {
        // An `edit` reflection's intervention lives in an outbox item, not in
        // any transcript — there is no prefix to replay. Explicit, so a new
        // trigger kind cannot silently be probed as if it were a denial.
        return Ok(ProbeResult::Skipped(format!(
            "`{}` reflections have no replayable intervention point",
            r.trigger
        )));
    };
    let Some(point) = point else {
        return Ok(ProbeResult::Skipped("could not locate the intervention".into()));
    };
    let slice = truncate_after_run(&convo.messages, point.message_index);
    let trajectory = extract(slice);
    if trajectory.turns.is_empty() {
        return Ok(ProbeResult::Skipped("no user turns before the intervention".into()));
    }
    let Some(recorded) = Session::run_configs(&path)?.first().cloned() else {
        return Ok(ProbeResult::Skipped("no RunConfig recorded".into()));
    };

    // The recorded system prompt with any rules block of its era removed: an
    // arm must carry exactly the block it was given, not a mixture of
    // generations.
    let base_system =
        recorded.system_prompt.as_deref().map(strip_rules_block).unwrap_or_default();
    let arm_system = |block: Option<&str>| match block {
        None => base_system.clone(),
        Some(b) if base_system.is_empty() => b.to_string(),
        Some(b) => format!("{base_system}\n\n{b}"),
    };

    let mut verdicts = Vec::new();
    for block in [baseline_block, treatment_block] {
        let system = arm_system(block);
        let cancel = CancellationToken::new();
        let registry = match replay_registry(
            &recorded.tools,
            prepared.agent.registry(),
            trajectory.calls.clone(),
            OnDivergence::Stop,
            cancel.clone(),
        ) {
            Ok(reg) => reg,
            Err(e) => return Ok(ProbeResult::Skipped(format!("{e:#}"))),
        };
        // Nothing executes under Stop mode, so nothing needs approving.
        let approver: Arc<dyn mecha_core::tool::Approver> =
            Arc::new(ModeApprover { mode: PermissionMode::Allow });
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
        match drive(&agent, &cx, &trajectory).await {
            Ok(report) => verdicts.push(if r.trigger == Trigger::Steer.as_str() {
                steer_verdict(&report, &point)
            } else {
                denial_verdict(&report, &point)
            }),
            Err(e) => return Ok(ProbeResult::Skipped(format!("replay failed: {e:#}"))),
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
