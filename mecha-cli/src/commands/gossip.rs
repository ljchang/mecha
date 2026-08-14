//! `mecha gossip` — two readers, different sources, generative follow-ups.
//!
//! Run by hand on a person you know well. That is deliberate for now: a
//! claim the exchange produces can then be checked against your own memory
//! rather than only against the graph, which is a far stronger test of "did
//! it find something real" than any automated target selection gives.
//!
//! Nothing is written. The run prints the exchange and stops, because the
//! first question is whether dialogue produces anything worth staging, and
//! a run that stages before that is answered is just a faster way to fill
//! the review queue.

use anyhow::{bail, Context, Result};
use clap::Args;
use mecha_core::config::Config;
use mecha_core::gossip::{self, ReaderSetup, Vantage};
use mecha_core::tool::ToolCtx;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct GossipArgs {
    /// The person (or project) to gossip about — a name, alias or id.
    #[arg(long)]
    pub entity: String,
    /// Rounds of question-and-answer. Bounded because convergence is not
    /// the goal: a preserved disagreement is a finding, not a failure.
    #[arg(long, default_value_t = 3)]
    pub rounds: u32,
    /// Only evidence on/after this date. BOTH readers get the same window,
    /// or a difference between them is the world having changed.
    #[arg(long, default_value = "2024-01-01")]
    pub since: String,
    /// The `[[mcp]]` server that serves the knowledge graph.
    #[arg(long, default_value = "pkg")]
    pub server: String,
    /// Minimum episodes a source needs before it can be a vantage.
    #[arg(long, default_value_t = 3)]
    pub min_coverage: i64,
    /// Claims to audit after the exchange; 0 skips the audit.
    ///
    /// Bounded because it costs one model call each, and because an audit
    /// nobody reads is worse than none — it lends a transcript the look of
    /// having been checked.
    #[arg(long, default_value_t = 8)]
    pub verify: usize,
    /// Append the whole run as one JSON line — exchange, graph findings,
    /// audit verdicts. Exploration surfaces gaps and inconsistencies; a
    /// surfacing that lives only in scrollback surfaces nothing twice.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

pub async fn run(global: &crate::GlobalOpts, args: &GossipArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let Some(server_cfg) = cfg.mcp.iter().find(|c| c.name == args.server) else {
        bail!(
            "no [[mcp]] server named '{}' in config — gossip reads the graph and \
             cannot run without it",
            args.server
        );
    };

    let sandbox = mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone());
    let client = Arc::new(
        mecha_core::mcp::McpClient::connect(server_cfg, &sandbox, &cwd)
            .await
            .with_context(|| format!("connecting to MCP server '{}'", args.server))?,
    );

    // Which sources actually cover this entity. Asking a source that holds
    // nothing wastes a reader and produces a confident silence.
    let (name, coverage, ambiguous) = gossip::coverage(&client, &args.entity).await?;
    if !ambiguous.is_empty() {
        eprintln!("'{}' is ambiguous — name one:", args.entity);
        for c in &ambiguous {
            eprintln!("  {c}");
        }
        return Ok(());
    }

    // A plan is not a fact: the calendar is full of meetings that have not
    // happened, and probing them invites reporting intentions as history.
    let until = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Re-measure inside the window the run will actually read. All-time
    // counts chose a pair that was nearly empty in-window on the first live
    // run — 493 Slack episodes since 2015, two since 2024 — and both readers
    // dutifully reported knowing nothing, a null manufactured by the
    // selection rather than found in the graph.
    let coverage = gossip::windowed_coverage(&client, &name, &coverage, &args.since, &until)
        .await
        .context("measuring in-window coverage")?;

    let Some((va, vb)) = gossip::choose_vantages(&coverage, args.min_coverage) else {
        println!(
            "{name}: fewer than two sources hold {} episode(s) between {} and {until} — \
             there is only one witness here, and one witness cannot gossip.",
            args.min_coverage, args.since
        );
        for c in &coverage {
            println!("  {} {} episode(s) in window", c.source, c.episodes);
        }
        return Ok(());
    };

    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    // Workspace only; a reader has one read-only tool and touches no files,
    // but the security config must still come from the user's own settings
    // rather than a default this command invented.
    let tool_ctx = ToolCtx {
        workspace: cwd.clone(),
        security: cfg.security.clone(),
        ..ToolCtx::default()
    };

    let build = |v: &Vantage, prompt: &str| -> Result<mecha_core::agent::Agent> {
        gossip::reader(
            mecha_core::provider::build(provider_cfg)?,
            ReaderSetup {
                client: Arc::clone(&client),
                vantage: v.clone(),
                since: args.since.clone(),
                until: until.clone(),
                tool_ctx: tool_ctx.clone(),
                agent_cfg: cfg.agent.clone(),
                model: model.clone(),
                system_prompt: prompt.to_string(),
            },
        )
    };

    println!(
        "gossiping about {name} · {} vs {} · {}..{} · {} round(s) · {} ({provider_name})",
        va.label,
        vb.label,
        args.since,
        until,
        args.rounds,
        model.as_deref().unwrap_or("default model"),
    );

    let answerers = vec![
        (va.clone(), build(&va, gossip::ANSWER_SYS)?),
        (vb.clone(), build(&vb, gossip::ANSWER_SYS)?),
    ];
    // Askers get NO tools. Handed kg_search they research and answer
    // instead of asking — the first live run's round-2 "question" was the
    // other reader's answer pasted back.
    let ask = || -> Result<mecha_core::agent::Agent> {
        gossip::asker(
            mecha_core::provider::build(provider_cfg)?,
            tool_ctx.clone(),
            cfg.agent.clone(),
            model.clone(),
        )
    };
    let askers = vec![(va.clone(), ask()?), (vb.clone(), ask()?)];

    let approver = Arc::new(mecha_core::tool::ModeApprover {
        mode: mecha_core::config::PermissionMode::ReadOnly,
    });
    let cx = mecha_core::agent::RunContext::new(tool_ctx.clone(), approver);

    let exchange = gossip::exchange(
        &answerers,
        &askers,
        &cx,
        &name,
        &format!("What should I know about {name}?"),
        args.rounds,
    )
    .await?;

    println!("\n{}", gossip::render(&exchange));

    let mut graph_findings_text: Option<String> = None;
    let mut audit_verdicts = Vec::new();
    if args.verify > 0 {
        // pkg's deterministic tier first, and printed whether or not the
        // model tier agrees with it. `kg_verify` dereferences stored claims
        // to their cited evidence with no model in the loop, so it is the
        // one part of this audit that cannot hallucinate — it belongs above
        // the part that can.
        match gossip::graph_findings(&client, &name).await {
            Ok(text) => {
                println!("\nWhat the graph says about its own claims:\n{text}");
                graph_findings_text = Some(text);
            }
            Err(e) => eprintln!("kg_verify failed: {e}"),
        }

        let verdicts = gossip::audit(
            &gossip::extractor(
                mecha_core::provider::build(provider_cfg)?,
                tool_ctx.clone(),
                cfg.agent.clone(),
                model.clone(),
            )?,
            &gossip::verifier(
                mecha_core::provider::build(provider_cfg)?,
                Arc::clone(&client),
                tool_ctx.clone(),
                cfg.agent.clone(),
                model.clone(),
            )?,
            &cx,
            &exchange,
            args.verify,
        )
        .await?;
        println!("{}", gossip::render_audit(&verdicts));
        audit_verdicts = verdicts;
    }

    if let Some(p) = &args.out {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .with_context(|| format!("opening --out file {}", p.display()))?;
        let line = serde_json::json!({
            "at": chrono::Local::now().to_rfc3339(),
            "entity": name,
            "since": args.since,
            "rounds": args.rounds,
            "model": model,
            "exchange": exchange,
            "graph_findings": graph_findings_text,
            "audit": audit_verdicts,
        });
        writeln!(f, "{line}").context("writing --out line")?;
    }

    println!(
        "Nothing was written to the graph. Read it and decide whether the \
         follow-ups found anything the templates would not have."
    );
    Ok(())
}
