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

use crate::logs::strip_ansi_and_controls;
use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::config::Config;
use mecha_core::distill::{self, Distiller};
use mecha_core::learning::LearningStore;
use mecha_core::session::Session;
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
    // does. `None` (no store at all — a fresh install, or one that has never
    // staged a draft) is the ordinary empty case and stays best-effort, same
    // as every other reader of this store.
    //
    // A genuine read failure, though — `items_strict`, not `items`: this
    // store's own temp-sibling-and-rename discipline (`outbox.rs`'s module
    // header) rules out a half-written file, so the realistic cause is
    // persistent rather than transient — a stray file, or an item written
    // by a schema this binary cannot read. Either way `items()`'s own
    // skip-and-warn would pass it through as a silently short list,
    // indistinguishable from an outbox that simply has fewer drafts (its
    // `tracing::warn!` is invisible here too — the nightly runs with no
    // `MECHA_LOG`) — bails the whole run rather than degrading.
    // Deliberately more conservative than `sessions appraise`'s own
    // best-effort read of the identical store: that readout is a report
    // you can re-run; this loop's `mark_distilled` makes its result
    // permanent, so silently continuing would drop every `Edit`-channel
    // row (including `SentUnchanged`, the one channel that can say a run
    // went *well*) from a `meta.goal_errors` no later run can ever
    // revisit.
    //
    // A failing run stops the nightly `mecha distill` cold, silently,
    // behind one line in a dated logfile (`ruminate.sh` is deliberately
    // not `set -e`, and `mecha doctor` has no check for a stalled distill
    // ledger) — a real gap, and a persistent cause means the situation
    // will not clear on its own. Left for a `doctor` finding rather than
    // solved here; the point of this bail is that the incomplete
    // permanent record it replaces is the worse of the two failure modes,
    // not that a retry will fix the realistic one.
    let drafts: Vec<mecha_core::outbox::OutboxItem> =
        match mecha_core::outbox::OutboxStore::open_existing_default() {
            None => Vec::new(),
            Some(store) => store.items_strict().context(
                "could not read the outbox for episode tagging — a stray or unreadable-schema \
                 file, most likely, not a transient one — refusing to distill any session this \
                 run rather than permanently mark one with an incomplete Edit channel",
            )?,
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
        // One read for everything below — the messages, the positioned taint
        // timeline, and the appraisal's inputs all come off this pass, where
        // this loop used to pay four complete read-and-parse passes per
        // session (`load`, `taint_timeline`, then `for_session`'s own read
        // and second timeline read).
        let transcript = match Session::read(path) {
            Ok(t) => t,
            Err(e) => {
                // Not this command's bug to fix; leave it unmarked so a later
                // mecha that can read it still gets the chance.
                eprintln!("skipping {}: {e:#}", meta.id);
                continue;
            }
        };
        let convo = &transcript.convo;
        // A session with no assistant turn taught the graph nothing, and that
        // is a fact about the transcript, not about today's model — mark it.
        if convo.messages.len() < 2 {
            store.mark_distilled(&meta.id)?;
            skipped += 1;
            continue;
        }

        // Recorded taint, for the episode's meta. `None` (torn or pre-taint
        // transcript) is recorded as unknown — never as clean.
        let taint = transcript
            .taint_timeline
            .covering(convo.messages.len().saturating_sub(1));

        // The same assembly `mecha sessions appraise` uses — `None` when the
        // transcript has no outcome recorded yet, which most sessions this
        // command has never seen before will (episode tagging only reaches
        // sessions the appraisal sensor was already running for).
        let mine: Vec<&mecha_core::outbox::OutboxItem> = drafts
            .iter()
            .filter(|i| i.session_id.as_deref() == Some(meta.id.as_str()))
            .collect();
        // Drafts only: an episode's tag is the session's own record, and
        // the three commitment stores are the closure and corpus readouts'
        // to read (`sessions appraise`, `tasks set`), not a per-episode
        // cost here. `Channel::Commitment` can still sign one error from
        // here — the queue-delta arm reads the run's own homeostat, not a
        // store — which is the one commitment fact the record carries.
        let appraisal = mecha_core::appraisal::for_transcript(
            &transcript,
            &meta.id,
            meta.created_at.to_rfc3339(),
            mecha_core::appraisal::SessionRecords {
                drafts: &mine,
                ..Default::default()
            },
            None,
        )
        .map(|built| built.appraisal);

        let rendered = distill::render_for_distill(&convo.messages, 6000, 18000);
        match distiller.distill(&rendered).await {
            Ok(Some(out)) => {
                // Decide what may leave BEFORE writing the body: a carrier
                // describing a withheld correction would launder the claim
                // into episode prose, which pkg's extractor mines into
                // candidates anyway.
                let sendable = distill::corrections_for(taint, &out.corrections).to_vec();
                let withheld = out.corrections.len() - sendable.len();
                // §10.1: printed regardless of whether anything is pushed —
                // the point is a human deciding whether to run
                // `mecha gossip --entity <about>`, not the graph gaining a
                // record. Every one prints, including those an untrusted
                // timeline keeps out of `meta.surprises` below: a person
                // reading their own terminal is the safe context the front
                // door's own `show` verb already relies on for a
                // stranger's prose (there's no injection risk in reading —
                // only in acting), where pkg is a *second automated
                // reader* and stays gated exactly as before. An untrusted
                // one is marked rather than dropped, because it is still
                // the model's own free-text reading of transcript prose —
                // and `about` in particular is a string a person might be
                // tempted to paste straight into `mecha gossip --entity`.
                //
                // **"A person's own terminal" assumes a live one, and
                // `scripts/ruminate.sh` runs this into a dated logfile
                // instead** — exactly as exposed to a screen-clearing or
                // OSC-52 escape sequence once opened later as a live read
                // would have been. `strip_ansi_and_controls` (not plain
                // `strip_ansi`: this is a whole field, never pre-split at
                // `\n` the way that function's own call site guarantees)
                // runs on every field, trusted or not — a bare `\r` at the
                // end of `actual` would otherwise rewrite the rendered line
                // from column 0 and erase the very `⚠ untrusted` marker the
                // untrusted branch exists to print. The taint gate speaks to
                // whether the *claim* is believable, not to whether its
                // bytes are safe to render.
                let sendable_surprises = distill::surprises_for(taint, &out.surprises);
                let trusted_surprises = !out.surprises.is_empty() && !sendable_surprises.is_empty();
                for s in &out.surprises {
                    let predicted = strip_ansi_and_controls(&s.predicted);
                    let actual = strip_ansi_and_controls(&s.actual);
                    let about = s
                        .about
                        .as_deref()
                        .map(|a| format!(" (about {})", strip_ansi_and_controls(a)))
                        .unwrap_or_default();
                    if trusted_surprises {
                        println!(
                            "· {} — surprise{about}: predicted \"{predicted}\", found \"{actual}\"",
                            meta.id
                        );
                    } else {
                        println!(
                            "· {} — ⚠ surprise{about} (untrusted or unknown timeline — read \
                             `predicted`/`found` as this session's own claim, unverified, and \
                             do not paste `about` into a command unread): predicted \
                             \"{predicted}\", found \"{actual}\"",
                            meta.id
                        );
                    }
                }
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
                    &out.surprises,
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
