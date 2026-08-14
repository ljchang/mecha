//! `mecha vet` — does the evidence a queued claim cites actually say it?
//!
//! The prior question to corroboration. `mecha corroborate` asks whether a
//! claim holds beyond its origin; this asks whether the origin ever said it
//! — extraction fidelity, judged with the origin episode handed over rather
//! than hunted for. One model call per candidate, no tools, no search: a
//! judge that could search the graph would find the claim there (extraction
//! put it there) and call that support.
//!
//! Three failure shapes come back, and they want different repairs:
//! UNSUPPORTED (the extractor invented it — reject), MISATTRIBUTED (the
//! wearable's diarization credited someone else's words to the subject —
//! rebind, the WHO line names the repair), OVERREACH (the evidence shows a
//! weaker thing — edit). Verdicts are opinions filed beside the candidate;
//! nothing is decided here.

use anyhow::{Context, Result};
use clap::Args;
use mecha_core::config::Config;
use mecha_core::gossip;
use mecha_core::tool::ToolCtx;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct VetArgs {
    /// Proposer of the class to work, e.g. `llm`.
    #[arg(long, default_value = "llm")]
    pub proposer: String,
    /// Predicate of the class to work, e.g. `has`.
    #[arg(long, default_value = "has")]
    pub predicate: String,
    /// Candidates to judge, oldest first.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
    #[arg(long, default_value = "pkg")]
    pub server: String,
    /// File the verdicts beside their candidates (mechanism `verification`).
    #[arg(long)]
    pub record: bool,
    /// Append one JSON line per judged candidate.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

pub async fn run(global: &crate::GlobalOpts, args: &VetArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let Some(server_cfg) = cfg.mcp.iter().find(|c| c.name == args.server) else {
        anyhow::bail!("no [[mcp]] server named '{}' in config", args.server);
    };
    let sandbox = mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone());
    let client = Arc::new(
        mecha_core::mcp::McpClient::connect(server_cfg, &sandbox, &cwd)
            .await
            .with_context(|| format!("connecting to MCP server '{}'", args.server))?,
    );

    let candidates = gossip::pending(
        &client,
        &args.proposer,
        &args.predicate,
        args.limit,
        Some("verification"),
        true,
    )
    .await?;
    if candidates.is_empty() {
        println!(
            "nothing unvetted pending in {}·{}",
            args.proposer, args.predicate
        );
        return Ok(());
    }

    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    let tool_ctx = ToolCtx {
        workspace: cwd.clone(),
        security: cfg.security.clone(),
        ..ToolCtx::default()
    };
    let approver = Arc::new(mecha_core::tool::ModeApprover {
        mode: mecha_core::config::PermissionMode::ReadOnly,
    });
    let cx = mecha_core::agent::RunContext::new(tool_ctx.clone(), approver);
    let judge = gossip::vet_judge(
        mecha_core::provider::build(provider_cfg)?,
        tool_ctx.clone(),
        cfg.agent.clone(),
        model.clone(),
    )?;

    println!(
        "vetting {} candidate(s) from {}·{} against their origin evidence · {} ({provider_name})\n",
        candidates.len(),
        args.proposer,
        args.predicate,
        model.as_deref().unwrap_or("default model"),
    );

    let mut tally: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut out_file = match &args.out {
        Some(p) => Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .with_context(|| format!("opening --out file {}", p.display()))?,
        ),
        None => None,
    };

    for cand in &candidates {
        if cand.evidence.is_none() {
            // bee:suggested stages with no episode at all — those candidates
            // can only be corroborated, never vetted.
            println!(
                "  [no-evidence] {} — cites no episode; corroboration is its check",
                cand.candidate_id
            );
            *tally.entry("no_evidence").or_default() += 1;
            continue;
        }
        let v = match gossip::vet(&judge, &cx, cand).await {
            Ok(v) => v,
            Err(e) => {
                println!("  [error] {} — {e:#}\n", cand.candidate_id);
                *tally.entry("error").or_default() += 1;
                continue;
            }
        };
        *tally.entry(v.verdict.as_str()).or_default() += 1;

        println!("  [{}] {}", v.verdict.as_str(), v.statement);
        if let Some(who) = &v.who {
            println!("      the evidence shows: {who}");
        }
        if let Some(p) = &v.predicate {
            println!("      better relation: {p}");
        }
        if !v.quote.is_empty() {
            println!("      {}", v.quote);
        }

        if args.record {
            let basis = match (&v.who, &v.predicate) {
                (Some(w), _) => format!("who:{w} · {}", v.quote),
                (None, Some(p)) => format!("predicate:{p} · {}", v.quote),
                (None, None) => v.quote.clone(),
            };
            if let Err(e) = gossip::file_verdict(
                &client,
                v.candidate_id,
                "verification",
                v.verdict.as_str(),
                &basis,
                model.as_deref(),
            )
            .await
            {
                println!("      (verdict not filed: {e:#})");
                *tally.entry("file_error").or_default() += 1;
            }
        }
        if let Some(f) = out_file.as_mut() {
            use std::io::Write;
            let line = serde_json::json!({
                "at": chrono::Local::now().to_rfc3339(),
                "proposer": args.proposer,
                "predicate": args.predicate,
                "subject": cand.subject,
                "origin_source": cand.origin_source,
                "model": model,
                "recorded": args.record,
                "result": v,
            });
            writeln!(f, "{line}").context("writing --out line")?;
        }
        println!();
    }

    let mut counts: Vec<(&&str, &usize)> = tally.iter().collect();
    counts.sort_by_key(|(k, _)| **k);
    println!(
        "— {}",
        counts
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if args.record {
        println!("Verdicts filed. Every candidate is still pending: a verdict is an opinion, and the decision is yours.");
    } else {
        println!(
            "Nothing was written. Re-run with --record to file these beside their candidates."
        );
    }
    Ok(())
}
