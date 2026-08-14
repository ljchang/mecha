//! `mecha corroborate` — does a generalisation hold beyond the one
//! transcript it came from?
//!
//! The review queue is not one problem. Clustered by (proposer, predicate)
//! it splits into classes that fail in structurally different ways, and the
//! design variable between them is not the prompt — it is what makes the
//! two readers independent.
//!
//! Here the axis is SOURCE. `bee:suggested·related_to` holds 300 behavioural
//! generalisations drawn from single conversations — "Luke prefers DIY
//! approaches over formal design consultation" — every one at confidence
//! 0.50 with no verdict history at all. The question they need is the one
//! gossip can actually ask: does anything else in the graph show this, or is
//! it one transcript over-generalised?
//!
//! That is a bounded accept-or-reject judgement over an enumerated
//! candidate, which is a far safer operation than assertion. Nothing is
//! generated; nothing new can be hallucinated into the graph. The two
//! readers only report what their own evidence shows, and the verdict is
//! computed from their two answers in code.

use anyhow::{bail, Context, Result};
use clap::Args;
use mecha_core::config::Config;
use mecha_core::gossip::{self, ReaderSetup, Vantage};
use mecha_core::tool::ToolCtx;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct CorroborateArgs {
    /// Proposer of the class to work, e.g. `bee:suggested`.
    #[arg(long, default_value = "bee:suggested")]
    pub proposer: String,
    /// Predicate of the class to work, e.g. `related_to`.
    #[arg(long, default_value = "related_to")]
    pub predicate: String,
    /// Candidates to judge, oldest first.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,
    /// Evidence on/after this date. Both readers get the same window: here
    /// a difference between them must be the SOURCES disagreeing, not the
    /// world having moved. (The persistence pattern inverts this — there,
    /// the world having moved is the signal.)
    #[arg(long, default_value = "2024-01-01")]
    pub since: String,
    #[arg(long, default_value = "pkg")]
    pub server: String,
    /// Minimum episodes a source needs before it can be a vantage.
    #[arg(long, default_value_t = 3)]
    pub min_coverage: i64,
    /// File the verdicts beside their candidates. Off by default: a verdict
    /// decides nothing, but a store filling up with an unmeasured
    /// mechanism's opinions is still worth opting into rather than
    /// inheriting.
    #[arg(long)]
    pub record: bool,
}

pub async fn run(global: &crate::GlobalOpts, args: &CorroborateArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let Some(server_cfg) = cfg.mcp.iter().find(|c| c.name == args.server) else {
        bail!("no [[mcp]] server named '{}' in config", args.server);
    };
    let sandbox = mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone());
    let client = Arc::new(
        mecha_core::mcp::McpClient::connect(server_cfg, &sandbox, &cwd)
            .await
            .with_context(|| format!("connecting to MCP server '{}'", args.server))?,
    );

    let candidates = gossip::pending(&client, &args.proposer, &args.predicate, args.limit).await?;
    if candidates.is_empty() {
        println!("nothing pending in {}·{}", args.proposer, args.predicate);
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
    let until = chrono::Local::now().format("%Y-%m-%d").to_string();

    println!(
        "corroborating {} candidate(s) from {}·{} · {}..{} · {} ({provider_name})\n",
        candidates.len(),
        args.proposer,
        args.predicate,
        args.since,
        until,
        model.as_deref().unwrap_or("default model"),
    );

    // Coverage is per SUBJECT, and most of a class shares one — cache it
    // rather than asking the graph the same question 300 times.
    let mut coverage_cache: HashMap<String, Vec<gossip::SourceCoverage>> = HashMap::new();
    let mut tally: HashMap<&str, usize> = HashMap::new();

    for cand in &candidates {
        let subject = cand.subject.clone().unwrap_or_default();
        if subject.is_empty() {
            println!(
                "  [skipped] {} — no subject to measure coverage for",
                cand.candidate_id
            );
            *tally.entry("skipped").or_default() += 1;
            continue;
        }
        let cov = match coverage_cache.get(&subject) {
            Some(c) => c.clone(),
            None => {
                let c = match gossip::coverage(&client, &subject).await {
                    Ok((name, sources, ambiguous)) if ambiguous.is_empty() && !name.is_empty() => {
                        gossip::windowed_coverage(&client, &name, &sources, &args.since, &until)
                            .await
                            .unwrap_or_default()
                    }
                    // An ambiguous or unresolvable subject is not a
                    // corroboration failure; say so and move on rather than
                    // guessing which person was meant.
                    _ => vec![],
                };
                coverage_cache.insert(subject.clone(), c.clone());
                c
            }
        };

        let Some((va, vb)) =
            // Fall back to the proposer when there is no originating
            // episode: `bee:suggested` still says the claim came from Bee.
            gossip::vantages_excluding(
                &cov,
                cand.origin_source.as_deref().or(Some(&args.proposer)),
                args.min_coverage,
            )
        else {
            println!(
                "  [no-witness] {} — nothing but {} covers '{}'; a claim cannot corroborate itself\n      {}",
                cand.candidate_id,
                cand.origin_source.as_deref().unwrap_or("its origin"),
                subject,
                cand.statement,
            );
            *tally.entry("no_witness").or_default() += 1;
            continue;
        };

        let build = |v: &Vantage| -> Result<mecha_core::agent::Agent> {
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
                    system_prompt: gossip::SIGHT_SYS.to_string(),
                },
            )
        };
        let readers = vec![(va.clone(), build(&va)?), (vb.clone(), build(&vb)?)];

        let result = gossip::corroborate(&readers, &cx, cand).await?;
        *tally.entry(result.verdict).or_default() += 1;

        println!("  [{}] {}", result.verdict, result.statement);
        if cand.subject_ambiguous {
            println!("      (subject '{subject}' guessed from an ambiguous name — a duplicate identity upstream)");
        }
        for (who, sighting, cite) in &result.sightings {
            println!("      {who}: {sighting:?} — {cite}");
        }
        if result.rechecked {
            println!("      (the dissenter looked again after seeing the other's citation)");
        }

        if args.record {
            let basis = result
                .sightings
                .iter()
                .map(|(w, s, _)| format!("{w}:{s:?}"))
                .collect::<Vec<_>>()
                .join(" ");
            gossip::file_verdict(
                &client,
                result.candidate_id,
                "corroboration",
                result.verdict,
                &basis,
                model.as_deref(),
            )
            .await?;
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
