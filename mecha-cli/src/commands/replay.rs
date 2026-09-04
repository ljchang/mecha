//! `mecha replay` — re-run a recorded session and report what changed.
//!
//! The recorded tool results are replayed verbatim; the only live component is
//! the model. That isolates the variable: same turns, same results, and a
//! divergence is a change in what the model chose, not in what the world said.
//!
//! The run is rebuilt from the session's `RunConfig` — its system prompt, tool
//! surface, budgets — not from today's flags, because a replay under different
//! conditions answers a different question. The provider and model default to
//! the recorded ones and *can* be overridden (`-p`, `-m`): replaying one
//! model's session on another is how you compare them on real work.

use crate::{setup, GlobalOpts};
use anyhow::{bail, Context, Result};
use mecha_core::agent::{Agent, RunContext};
use mecha_core::config::PermissionMode;
use mecha_core::replay::{extract, Divergence};
use mecha_core::replay_run::{drive, replay_registry, OnDivergence};
use mecha_core::session::Session;
use mecha_core::tool::ModeApprover;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Session id, unique prefix, or a path to a transcript file.
    pub session: String,

    /// What to do when the replay departs from the recording.
    ///
    /// `stop` ends the run there — after a divergence, every later recorded
    /// result answers a question nobody asked. `error` does the same and exits
    /// nonzero on *any* divergence, argument spellings included; use it in CI.
    /// `live` abandons the recording and keeps going against the real tools.
    #[arg(long, value_name = "stop|error|live", default_value = "stop")]
    pub on_divergence: String,

    /// Emit the report as JSON instead of prose.
    #[arg(long)]
    pub json: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let mode = match args.on_divergence.as_str() {
        "stop" => OnDivergence::Stop,
        "error" => OnDivergence::Error,
        "live" => OnDivergence::Live,
        other => bail!("--on-divergence must be stop, error, or live, not `{other}`"),
    };

    // --- load the recording ---
    let path = resolve_session(&args.session)?;
    let (meta, convo) = Session::load(&path)?;
    let configs = Session::run_configs(&path)?;
    let Some(recorded) = configs.first().cloned() else {
        bail!(
            "{} has no RunConfig record, so the replay cannot rebuild the run \
             (system prompt, tool list, budgets). Sessions recorded before that \
             record existed cannot be replayed",
            path.display()
        );
    };
    if configs.len() > 1 {
        eprintln!(
            "note: this session was attached {} times; replaying under its first config",
            configs.len()
        );
    }
    // Which learned rules the run carried is part of the arm being replayed,
    // and once loading is scoped the store cannot answer for a past run.
    let delivered = Session::read(&path)
        .ok()
        .and_then(|t| t.episode)
        .and_then(|e| e.delivered);
    eprintln!("note: {}", recorded.rules_arm_note(delivered.as_deref()));

    let trajectory = extract(&convo.messages);
    if trajectory.turns.is_empty() {
        bail!("the transcript contains no user turns; nothing to replay");
    }
    if trajectory.steered {
        eprintln!(
            "note: the recording was steered mid-run; steering cannot be re-injected, \
             so the comparison is approximate"
        );
    }

    // --- rebuild the run ---
    // Today's full setup supplies the live tool surface — builtins, MCP
    // servers, subagents — because the recorded registry may name any of them.
    // The parent agent it builds is discarded; only its registry is borrowed.
    let mut opts = global.clone();
    if opts.provider.is_none() {
        opts.provider = Some(recorded.provider.clone());
    }
    let prepared = setup::prepare(&opts, false).await?;

    let provider_name = opts.provider.clone().unwrap();
    let provider_cfg = prepared
        .config
        .providers
        .get(&provider_name)
        .with_context(|| {
            format!("provider `{provider_name}` is not in the config; add it or pass -p")
        })?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    // The recorded model only makes sense on the recorded provider. Replaying
    // on another provider (`-p`) means that provider's own model unless `-m`
    // says otherwise — sending the recorded name would name a model the other
    // server does not serve.
    let model = opts.model.clone().unwrap_or_else(|| {
        if global.provider.is_some() {
            provider_cfg
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_string())
        } else {
            recorded.model.clone()
        }
    });

    let mut agent_cfg = prepared.config.agent.clone();
    agent_cfg.system_prompt = recorded.system_prompt.clone();
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

    let cancel = CancellationToken::new();
    // The exact specs the recording was sent, when the surface store still
    // holds them — under stop/error they override today's descriptions and
    // stand in for tools nothing today can construct; under live they are
    // ignored, because live tools genuinely run and deserve their own words.
    let recorded_specs = recorded
        .tools_hash
        .as_deref()
        .and_then(|h| mecha_core::surface::SurfaceStore::open_default()?.load(h))
        .unwrap_or_default();
    let registry = replay_registry(
        &recorded.tools,
        prepared.agent.registry(),
        // Ignored under `Live`, where a missing tool must stay fatal; consulted
        // under both non-executing modes, `Stop` and `Error` alike — see
        // `replay_registry`.
        Some(&crate::setup::surface_only_registry()),
        &recorded_specs,
        trajectory.calls.clone(),
        mode,
        cancel.clone(),
    )?;

    // Nothing executes in stop/error mode, so nothing needs approving. Live
    // mode falls back to the configured permission mode — real tools run after
    // the divergence, and they deserve exactly the scrutiny they always get.
    let approver: Arc<dyn mecha_core::tool::Approver> = match mode {
        OnDivergence::Live => Arc::new(ModeApprover {
            mode: prepared.config.tools.permission_mode,
        }),
        _ => Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        }),
    };

    let mut tool_ctx = mecha_core::tool::ToolCtx {
        workspace: recorded.workspace.clone(),
        // **The `compact` channel, or a replayed compaction reads as divergence.**
        // The run gets a threshold from `with_compact_at` below, and a recorded
        // session whose tool list named `compact` gets the tool back through the
        // rebuilt registry — so without this the tool answers "compaction is not
        // enabled for this run", which is false of the run it is replaying. Under
        // `--on-divergence=live` that is an executed call returning the wrong
        // answer and counting as a divergence; in a harness probe it is worse,
        // because both arms then replay a trajectory missing the compactions the
        // recording had, and a `compact_at_tokens` candidate is measured on runs
        // that never compacted.
        //
        // Wired unconditionally: the flag costs nothing when no tool reads it,
        // and making it conditional would be a second place that has to agree
        // with `setup`'s about whether this run compacts at all — which is the
        // split `PreparedTools::compact_requested` exists to prevent.
        compact_requested: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        ))),
        ..Default::default()
    };
    if !recorded.workspace.exists() {
        eprintln!(
            "note: the recorded workspace {} no longer exists; fine for a pure \
             replay, fatal for --on-divergence=live",
            recorded.workspace.display()
        );
        tool_ctx.workspace = std::env::temp_dir();
    }

    let agent = Agent::new(
        provider,
        registry,
        Arc::clone(&approver),
        tool_ctx.clone(),
        agent_cfg,
        Some(model.clone()),
    )?
    .with_pricing(provider_cfg.pricing());

    let cx = RunContext::new(tool_ctx, approver)
        .with_cancel(cancel)
        .with_compact_at(recorded.compact_at_tokens);

    eprintln!(
        "replaying {} · {} turns, {} recorded calls · {} ({})",
        meta.id,
        trajectory.turns.len(),
        trajectory.calls.len(),
        model,
        provider_name
    );

    // --- run and judge ---
    let report = drive(&agent, &cx, &trajectory).await?;

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": meta.id,
                "provider": provider_name,
                "model": model,
                "recorded_calls": report.recorded_calls,
                "replayed_calls": report.replayed_calls.len(),
                "turns": report.turns,
                "stopped_early": report.stopped_early,
                "divergences": report.divergences,
                "final_text": report.final_text,
            })
        );
    } else {
        render_report(&report, &trajectory.final_text);
    }

    if mode == OnDivergence::Error && !report.divergences.is_empty() {
        bail!(
            "replay diverged: {} difference(s)",
            report.divergences.len()
        );
    }
    Ok(())
}

fn render_report(report: &mecha_core::replay_run::ReplayReport, recorded_final: &str) {
    if report.divergences.is_empty() {
        println!(
            "replay matched: {} calls over {} turn(s), no divergence",
            report.recorded_calls, report.turns
        );
    } else {
        println!(
            "replayed {} of {} recorded calls over {} turn(s){}",
            report.replayed_calls.len(),
            report.recorded_calls,
            report.turns,
            if report.stopped_early {
                ", stopped at divergence"
            } else {
                ""
            }
        );
        for d in &report.divergences {
            match d {
                Divergence::Tool {
                    index,
                    expected,
                    actual,
                } => {
                    println!("  call #{index}: recorded `{expected}`, replayed `{actual}`")
                }
                Divergence::Arguments {
                    index,
                    tool,
                    expected,
                    actual,
                } => {
                    println!("  call #{index} ({tool}): same tool, different arguments");
                    println!("    recorded: {expected}");
                    println!("    replayed: {actual}");
                }
                Divergence::Extra { index, actual } => {
                    println!("  call #{index}: replay kept going with `{actual}` after the recording ended")
                }
                Divergence::Missing { index, expected } => {
                    println!("  call #{index}: recording continued with `{expected}`; the replay stopped")
                }
            }
        }
    }
    let structural = report.structural().count();
    let cosmetic = report.divergences.len() - structural;
    if !report.divergences.is_empty() {
        println!("  {structural} structural, {cosmetic} argument-only");
    }
    if !report.final_text.is_empty() && report.final_text != recorded_final {
        println!("final answer differs:");
        println!("  recorded: {}", first_line(recorded_final));
        println!("  replayed: {}", first_line(&report.final_text));
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.len() > 120 {
        format!("{}…", &line[..120])
    } else {
        line.to_string()
    }
}

fn resolve_session(arg: &str) -> Result<PathBuf> {
    let as_path = PathBuf::from(arg);
    if as_path.is_file() {
        return Ok(as_path);
    }
    Session::find(&Session::default_dir()?, arg)
}
