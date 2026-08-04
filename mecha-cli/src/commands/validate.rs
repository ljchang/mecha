//! `mecha validate` — measure whether the learned rules actually help.
//!
//! Every trigger gets a counterfactual probe, and the two kinds are graded
//! differently on purpose:
//!
//! - **Followups** re-ask the user's corrective turn against the rebuilt
//!   conversation, with and without the rules, and a judge grades both
//!   answers. Judge-graded, so a single flip is a prompt to read the two
//!   answers, not a result.
//! - **Steers and denials** land mid-run, so their probes *replay* the
//!   recorded prefix — recorded tool results, seeded sampler, no steering
//!   text (extraction drops it, which makes the replay the no-steer
//!   counterfactual by construction) — once per arm, and the verdict is
//!   structural: did the model do the steered thing without the steer, did
//!   it repeat the exact call the user refused. Trace-graded, no judge. See
//!   [`mecha_core::counterfactual`] for the verdict semantics.
//!
//! A rule set that does not move these verdicts on reflections it did not
//! train on is prompt clutter, and `mecha learn --holdout` exists to keep
//! such reflections available.

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::agent::{Agent, RunContext};
use mecha_core::config::{Config, PermissionMode};
use mecha_core::counterfactual::{
    denial_verdict, locate_denial, locate_steer, steer_verdict, truncate_after_run, ProbeVerdict,
};
use mecha_core::eval::Judge;
use mecha_core::learning::{locate_followup, strip_rules_block, LearningStore, Trigger};
use mecha_core::message::{CompletionRequest, Message};
use mecha_core::replay::extract;
use mecha_core::replay_run::{drive, replay_registry, OnDivergence};
use mecha_core::session::Session;
use mecha_core::tool::ModeApprover;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Provider entry the judge runs on. Defaults to the model under test,
    /// which is worth avoiding for the usual reason.
    #[arg(long)]
    pub judge_provider: Option<String>,

    /// Judge model id.
    #[arg(long)]
    pub judge_model: Option<String>,

    /// Only validate reflections not yet consumed by a learn pass — the
    /// held-out set `mecha learn --holdout` leaves.
    #[arg(long)]
    pub unprocessed_only: bool,

    /// Probe only these triggers (comma-separated: steer, denial, followup).
    /// Default is all three.
    #[arg(long, value_delimiter = ',')]
    pub trigger: Vec<String>,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    let rules_block = store.rules_prompt_block()?;
    let Some(rules_block) = rules_block else {
        println!("no rules to validate — run `mecha learn` first");
        return Ok(());
    };

    let wanted_triggers: Vec<&str> = if args.trigger.is_empty() {
        vec![Trigger::Steer.as_str(), Trigger::Denial.as_str(), Trigger::Followup.as_str()]
    } else {
        args.trigger.iter().map(String::as_str).collect()
    };
    let reflexions: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| wanted_triggers.contains(&r.trigger.as_str()))
        .filter(|r| !args.unprocessed_only || !r.is_processed)
        .collect();
    if reflexions.is_empty() {
        println!("no reflections to probe");
        return Ok(());
    }

    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());

    let (judge_name, judge_cfg) = cfg.provider(args.judge_provider.as_deref())?;
    let judge = Judge::new(
        mecha_core::provider::build(judge_cfg)?,
        args.judge_model.clone().or_else(|| judge_cfg.model.clone()),
    )
    // These rubrics carry more context than eval's, and the judge reasons
    // about the whole exchange before the JSON appears. 4096 was measured
    // insufficient on the very first probe.
    .with_max_tokens(16384);
    eprintln!(
        "probing {} reflection(s) with {model} ({provider_name}), judged by {} ({judge_name})",
        reflexions.len(),
        judge.model()
    );

    // Steer and denial probes replay against the recorded tool surface, which
    // needs the live registry for specs — builtins, MCP servers, subagents.
    // Built once, only when something will use it; the parent agent it builds
    // is discarded and only its registry is borrowed, as in `mecha replay`.
    let needs_replay =
        reflexions.iter().any(|r| r.trigger != Trigger::Followup.as_str());
    let prepared = if needs_replay {
        Some(setup::prepare(&global.clone(), false).await?)
    } else {
        None
    };

    let sessions_dir = Session::default_dir()?;
    let mut improved = 0u32;
    let mut regressed = 0u32;
    let mut unchanged = 0u32;
    let mut inconclusive = 0u32;
    let mut skipped = 0u32;

    for r in &reflexions {
        let path = match Session::find(&sessions_dir, &r.session_id) {
            Ok(p) => p,
            Err(_) => {
                eprintln!("· {}: session {} not found; skipping", r.id, r.session_id);
                skipped += 1;
                continue;
            }
        };
        let (_, convo) = Session::load(&path)?;

        // The recorded system prompt, with any rules block of its era removed:
        // the baseline arm must be rules-free and the treatment arm must carry
        // exactly the current rules, not a mixture of generations.
        let base_system = Session::run_configs(&path)?
            .first()
            .and_then(|rc| rc.system_prompt.clone())
            .map(|s| strip_rules_block(&s))
            .unwrap_or_default();
        let with_rules = if base_system.is_empty() {
            rules_block.clone()
        } else {
            format!("{base_system}\n\n{rules_block}")
        };

        // ── steers and denials: replay the prefix, grade the trace ──
        if r.trigger != Trigger::Followup.as_str() {
            let point = if r.trigger == Trigger::Steer.as_str() {
                locate_steer(&convo.messages, &r.intervention)
            } else {
                locate_denial(&convo.messages, &r.intervention)
            };
            let Some(point) = point else {
                eprintln!("· {}: could not locate the intervention; skipping", r.id);
                skipped += 1;
                continue;
            };
            let slice = truncate_after_run(&convo.messages, point.message_index);
            let trajectory = extract(slice);
            if trajectory.turns.is_empty() {
                eprintln!("· {}: no user turns before the intervention; skipping", r.id);
                skipped += 1;
                continue;
            }
            let Some(recorded) =
                Session::run_configs(&path)?.first().cloned()
            else {
                eprintln!("· {}: no RunConfig recorded; skipping", r.id);
                skipped += 1;
                continue;
            };
            let prepared = prepared.as_ref().expect("built because needs_replay");

            let mut verdicts = Vec::new();
            for system in [&base_system, &with_rules] {
                let cancel = CancellationToken::new();
                let registry = match replay_registry(
                    &recorded.tools,
                    prepared.agent.registry(),
                    trajectory.calls.clone(),
                    OnDivergence::Stop,
                    cancel.clone(),
                ) {
                    Ok(reg) => reg,
                    Err(e) => {
                        eprintln!("· {}: {e:#}; skipping", r.id);
                        break;
                    }
                };
                // Nothing executes under Stop mode, so nothing needs approving.
                let approver: Arc<dyn mecha_core::tool::Approver> =
                    Arc::new(ModeApprover { mode: PermissionMode::Allow });
                let mut agent_cfg = prepared.config.agent.clone();
                agent_cfg.system_prompt = (!system.is_empty()).then(|| system.to_string());
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
                    Some(model.clone()),
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
                    Err(e) => {
                        eprintln!("· {}: replay failed ({e:#}); skipping", r.id);
                        break;
                    }
                }
            }
            let [baseline, with] = &verdicts[..] else {
                skipped += 1;
                continue;
            };

            let label = match (baseline, with) {
                (ProbeVerdict::Inconclusive(why), _) | (_, ProbeVerdict::Inconclusive(why)) => {
                    inconclusive += 1;
                    println!("· {} [{}] inconclusive: {why}", r.id, r.trigger);
                    continue;
                }
                (ProbeVerdict::Fail, ProbeVerdict::Pass) => {
                    improved += 1;
                    "IMPROVED"
                }
                (ProbeVerdict::Pass, ProbeVerdict::Fail) => {
                    regressed += 1;
                    "REGRESSED"
                }
                (both, _) => {
                    unchanged += 1;
                    if *both == ProbeVerdict::Pass {
                        "unchanged (both pass)"
                    } else {
                        "unchanged (both fail)"
                    }
                }
            };
            println!("· {} [{}, {label}] {}", r.id, r.trigger, r.reflexion_text);
            continue;
        }

        // ── followups: re-ask the corrective turn, judge both answers ──
        let Some(idx) = locate_followup(&convo.messages, &r.intervention) else {
            eprintln!("· {}: could not locate the intervention turn; skipping", r.id);
            skipped += 1;
            continue;
        };

        let mut messages: Vec<Message> = convo.messages[..idx].to_vec();
        messages.push(Message::user(r.intervention.clone()));

        let mut answers = Vec::new();
        for system in [&base_system, &with_rules] {
            let request = CompletionRequest {
                model: model.clone(),
                system: (!system.is_empty()).then(|| system.clone()),
                messages: messages.clone(),
                tools: Vec::new(),
                max_tokens: 4096,
                effort: None,
                thinking: false,
                cache_prompt: true,
            };
            let response = provider.complete(&request, None).await?;
            answers.push(response.message.text());
        }

        // The rubric is what the intervention itself established the user
        // wanted; the reflection's lesson names it directly.
        let rubric = format!(
            "the answer does what the user's message asks, in the light of this \
             known expectation: {}",
            r.reflexion_text
        );
        let mut verdicts = Vec::new();
        for answer in &answers {
            match judge.assess(&r.intervention, &rubric, answer).await {
                Ok(v) => verdicts.push(v.pass),
                Err(e) => {
                    eprintln!("· {}: judge failed ({e:#}); skipping", r.id);
                    verdicts.clear();
                    break;
                }
            }
        }
        let [baseline, with] = verdicts[..] else {
            skipped += 1;
            continue;
        };

        let label = match (baseline, with) {
            (false, true) => {
                improved += 1;
                "IMPROVED"
            }
            (true, false) => {
                regressed += 1;
                "REGRESSED"
            }
            _ => {
                unchanged += 1;
                if baseline { "unchanged (both pass)" } else { "unchanged (both fail)" }
            }
        };
        println!("· {} [{}] {}", r.id, label, r.reflexion_text);
        if baseline != with {
            println!("    baseline:   {}", first_line(&answers[0]));
            println!("    with rules: {}", first_line(&answers[1]));
        }
    }

    println!(
        "\n{improved} improved, {regressed} regressed, {unchanged} unchanged, \
         {inconclusive} inconclusive, {skipped} skipped (n={}; steers and denials are \
         trace-graded, followups judge-graded — read before believing a single flip)",
        reflexions.len()
    );
    Ok(())
}

fn first_line(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(140).collect::<String>())
    } else {
        line.to_string()
    }
}
