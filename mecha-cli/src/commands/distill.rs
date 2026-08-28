//! `mecha distill` — summarise closed sessions into episodes and stage them
//! to the personal knowledge graph.
//!
//! The counterpart to `mecha reflect`: reflect mines *how mecha should work*
//! from the moments the user stepped in; distill records *what happened* —
//! what the user would ask a personal assistant later — as an episode pushed
//! through the graph server's `kg_upsert`. Evidence, not belief: the facts
//! pkg extracts from the episode wait in its review queue.
//!
//! Idempotent like reflect: distilled session ids are ledgered (and pkg's
//! `(source, source_id)` key makes a duplicate push an update anyway), so a
//! nightly run or a `session_end` hook only ever pays for the new sessions.

use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::config::Config;
use mecha_core::distill::{self, Distiller};
use mecha_core::learning::LearningStore;
use mecha_core::session::{Session, TaintTimeline};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Directory of session transcripts. Defaults to the standard location.
    #[arg(long)]
    pub sessions_dir: Option<PathBuf>,

    /// List what would be distilled without calling a model or writing.
    #[arg(long)]
    pub dry_run: bool,

    /// Distill at most this many sessions this run.
    #[arg(long)]
    pub limit: Option<usize>,

    /// The `[[mcp]]` server holding the knowledge graph.
    #[arg(long, default_value = "graph")]
    pub server: String,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let sessions_dir = match &args.sessions_dir {
        Some(dir) => dir.clone(),
        None => Session::default_dir()?,
    };
    let store = LearningStore::open(LearningStore::default_root()?)?;
    // Same lock discipline as reflect: taken before reading the ledger, so
    // two detached session_end hooks cannot both see a session as new.
    let _lock = if args.dry_run {
        None
    } else {
        Some(store.lock()?)
    };
    let done = store.distilled_sessions()?;

    let sessions = Session::list(&sessions_dir)?;
    let mut todo: Vec<_> = sessions
        .into_iter()
        .filter(|(meta, _)| !done.contains(&meta.id))
        .collect();
    if let Some(limit) = args.limit {
        todo.truncate(limit);
    }
    if todo.is_empty() {
        println!("nothing to distill: every session is already in the graph's ledger");
        return Ok(());
    }

    if args.dry_run {
        for (meta, path) in &todo {
            let n = Session::load(path)
                .map(|(_, c)| c.messages.len())
                .unwrap_or(0);
            println!("{} ({n} message(s), {})", meta.id, meta.created_at);
        }
        println!(
            "dry run: {} session(s) would be distilled; nothing written",
            todo.len()
        );
        return Ok(());
    }

    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let Some(server_cfg) = cfg.mcp.iter().find(|c| c.name == args.server) else {
        bail!(
            "no [[mcp]] server named '{}' in config — distillation stages episodes \
             through the knowledge graph server and cannot run without it",
            args.server
        );
    };

    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    let distiller = Distiller::new(provider, model);
    eprintln!(
        "distilling with {} ({provider_name}) → {}",
        distiller.model(),
        args.server
    );

    let sandbox = mecha_core::sandbox::Sandbox::new(cfg.sandbox.clone());
    let client = mecha_core::mcp::McpClient::connect(server_cfg, &sandbox, &cwd)
        .await
        .with_context(|| format!("connecting to MCP server '{}'", args.server))?;

    // For episode tagging (§10 of GOAL-SYSTEM-DESIGN.md): the affect label
    // and goal errors ride on the episode's `meta`, and a `GoalError` cites
    // an outbox draft (`Cite::Draft`) the same way `mecha sessions appraise`
    // does. Best-effort like every reader of this store — a failure here
    // costs the `Edit` channel of the tagging and nothing else, so unlike
    // that command's own readout there is no separate "could not read" vs
    // "empty" report to keep honest; the difference is invisible in what
    // gets pushed either way.
    let drafts: Vec<mecha_core::outbox::OutboxItem> =
        match mecha_core::outbox::OutboxStore::open_existing_default() {
            None => Vec::new(),
            Some(store) => store.items().unwrap_or_default(),
        };

    let mut distilled = 0usize;
    let mut skipped = 0usize;
    // Counted apart from `distilled`: a carrier is an episode the
    // distiller judged NOT worth remembering, pushed only so its
    // corrections have something to ride. Folding it into the episode
    // count would tell an operator the graph gained five memories on a
    // night it gained five repairs.
    let mut carriers = 0usize;
    for (meta, path) in &todo {
        let (_, convo) = match Session::load(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                // Not this command's bug to fix; leave it unmarked so a later
                // mecha that can read it still gets the chance.
                eprintln!("skipping {}: {e:#}", meta.id);
                continue;
            }
        };
        // A session with no assistant turn taught the graph nothing, and that
        // is a fact about the transcript, not about today's model — mark it.
        if convo.messages.len() < 2 {
            store.mark_distilled(&meta.id)?;
            skipped += 1;
            continue;
        }

        // Recorded taint, for the episode's meta. `None` (torn or pre-taint
        // transcript) is recorded as unknown — never as clean.
        let taint = Session::taint_timeline(path)
            .unwrap_or_else(|_| TaintTimeline::default())
            .covering(convo.messages.len().saturating_sub(1));

        // The same assembly `mecha sessions appraise` uses — `None` when the
        // transcript has no outcome recorded yet, which most sessions this
        // command has never seen before will (episode tagging only reaches
        // sessions the appraisal sensor was already running for).
        let mine: Vec<&mecha_core::outbox::OutboxItem> = drafts
            .iter()
            .filter(|i| i.session_id.as_deref() == Some(meta.id.as_str()))
            .collect();
        let appraisal = mecha_core::appraisal::for_session(
            path,
            &meta.id,
            meta.created_at.to_rfc3339(),
            &mine,
            None,
        )
        .map(|built| built.appraisal);

        let transcript = distill::render_for_distill(&convo.messages, 6000, 18000);
        match distiller.distill(&transcript).await {
            Ok(Some(out)) => {
                // Decide what may leave BEFORE writing the body: a carrier
                // describing a withheld correction would launder the claim
                // into episode prose, which pkg's extractor mines into
                // candidates anyway.
                let sendable = distill::corrections_for(taint, &out.corrections).to_vec();
                let withheld = out.corrections.len() - sendable.len();
                // `None` means nothing may leave this session: no episode
                // text, and any corrections withheld by taint. Pushing a
                // carrier here would be an episode *about* corrections
                // that were not sent.
                let Some(body) = out.body(taint) else {
                    store.mark_distilled(&meta.id)?;
                    skipped += 1;
                    if withheld > 0 {
                        println!(
                            "· {} — {withheld} correction(s) withheld (untrusted or unknown \
                             timeline); nothing to push",
                            meta.id
                        );
                    }
                    continue;
                };
                let push_args = distill::upsert_args(
                    &meta.id,
                    &path.display().to_string(),
                    &meta.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                    &body,
                    taint,
                    distiller.model(),
                    &sendable,
                    appraisal.as_ref(),
                );
                match distill::push_episode(&client, push_args).await {
                    Ok(outcome) => {
                        store.mark_distilled(&meta.id)?;
                        // A carrier is not a memory the graph gained.
                        let carrier = out.is_corrections_only(taint);
                        if carrier {
                            carriers += 1;
                        } else {
                            distilled += 1;
                        }
                        println!(
                            "· {} → {} ({}{}, {} entit{} linked)",
                            meta.id,
                            outcome.uid,
                            outcome.status,
                            if carrier { ", corrections only" } else { "" },
                            outcome.entities_linked,
                            if outcome.entities_linked == 1 {
                                "y"
                            } else {
                                "ies"
                            }
                        );
                        // A correction that resolved to nothing is a repair
                        // that silently did not happen — say so. Report
                        // SENT and WITHHELD separately: a zeroed tally for
                        // a correction we never transmitted reads exactly
                        // like pkg failing to pin one down, and the session
                        // is marked distilled either way.
                        if !sendable.is_empty() {
                            println!(
                                "  {} correction{} sent · {} repaired · {} sent to review",
                                sendable.len(),
                                if sendable.len() == 1 { "" } else { "s" },
                                outcome.corrections_applied,
                                outcome.corrections_unresolved
                            );
                            // The tally must add up, or the print is
                            // theatre: anything pkg neither repaired nor
                            // queued went nowhere, and would otherwise
                            // leave no trace at all.
                            let accounted =
                                outcome.corrections_applied + outcome.corrections_unresolved;
                            let sent = sendable.len() as i64;
                            if accounted != sent || outcome.corrections_processed != sent {
                                eprintln!(
                                    "  WARNING: {sent} sent but pkg reports {} processed and \
                                     {accounted} accounted for — {} unaccounted",
                                    outcome.corrections_processed,
                                    sent - accounted
                                );
                            }
                        }
                        if withheld > 0 {
                            println!(
                                "  {withheld} correction{} withheld — the session's timeline \
                                 is untrusted or unknown",
                                if withheld == 1 { "" } else { "s" }
                            );
                        }
                    }
                    Err(e) => {
                        // The push not landing is the one failure that must
                        // leave the session unmarked: the summary was worth
                        // keeping and a later run should retry it.
                        eprintln!(
                            "· push failed: {e:#}\n  leaving {} unledgered so a later run retries",
                            meta.id
                        );
                    }
                }
            }
            Ok(None) => {
                // A deliberate skip is a decision about the transcript;
                // re-arguing it nightly will not change it.
                store.mark_distilled(&meta.id)?;
                skipped += 1;
            }
            Err(e) => {
                eprintln!(
                    "· distillation failed: {e:#}\n  leaving {} unledgered so a later run retries",
                    meta.id
                );
            }
        }
    }

    store.commit(&format!(
        "distill: {distilled} episode(s), {carriers} carrier(s), {skipped} skip(s)"
    ));
    let carried = if carriers > 0 {
        format!(", {carriers} carried corrections only")
    } else {
        String::new()
    };
    println!(
        "distilled {distilled} session(s) into the graph{carried}, skipped {skipped} \
         (nothing durable); ledger: {}",
        store.root().join("distilled.jsonl").display()
    );
    Ok(())
}
