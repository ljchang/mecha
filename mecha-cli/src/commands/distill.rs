//! `mecha distill` — summarise closed sessions into episodes and stage them
//! to the personal knowledge graph.
//!
//! The counterpart to `mecha reflect`: reflect mines *how mecha should work*
//! from the moments the user stepped in; distill records *what happened* —
//! what the user would ask a personal assistant later — as an episode pushed
//! through the graph server's `kg_upsert`. Evidence, not belief: the facts
//! the graph extracts from the episode wait in its review queue.
//!
//! Idempotent like reflect: distilled session ids are ledgered (and the graph's
//! `(source, source_id)` key makes a duplicate push an update anyway), so a
//! nightly run or a `session_end` hook only ever pays for the new sessions.

use crate::logs::strip_ansi_and_controls;
use crate::GlobalOpts;
use anyhow::{bail, Context, Result};
use mecha_core::appraisal_store::{AppraisalStore, Recorded, SessionEvidence};
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
    // A test or stray experiment session must not become a graph episode —
    // the graph is the owner's memory (`session::split_admitted`).
    // Skipped sessions are never marked distilled, so this is every test or
    // experiment session in the store, reprinted each pass — worded so.
    let candidates: Vec<_> = sessions
        .into_iter()
        .filter(|(meta, _)| !done.contains(&meta.id))
        .collect();
    let (mut todo, skipped) = mecha_core::session::split_admitted(candidates);
    if skipped > 0 {
        println!("passing over {skipped} test or experiment session(s) in the store");
    }
    if let Some(limit) = args.limit {
        todo.truncate(limit);
    }
    // Oldest first, once the limit has chosen the newest: a session's
    // appraisal reads the clean appraisals of earlier sessions in the same
    // situation (row 2a-2), so an earlier session must be on record before
    // a later one is appraised — in one night's batch as across nights.
    todo.reverse();
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
    // The appraisal reads the whole transcript, so it runs on the local
    // model only (R29: sending transcripts to a cloud model for
    // interpretation is the owner's privacy decision, not proposed). A
    // distill run on another provider still distills; it appraises nothing,
    // and says so.
    let local = provider_cfg.kind == "local";
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

    // The charter, for the episode tag's sensored-line attribution
    // (§11.1) — one small file per distill run, and the one store this
    // command does read beside the outbox. It is also what lets a charter
    // id cross to the graph whole (`meta.serves_charter`, `charter:<id>`):
    // `of_session` keeps a charter reference only if the loaded charter
    // contains the line, so the pointer is resolved before it leaves.
    let (charter, charter_unreadable) = mecha_core::appraisal::load_charter();
    // The board's pointers, once per run, so a task or project id a run
    // named is resolved before it rides on an episode's `meta` — a token
    // is not a pointer until the board says so. Unreadable is said, and
    // then admits nothing: every such reference crosses as its kind word.
    let charter_lines: Vec<String> = charter
        .as_ref()
        .map(|c| c.lines().iter().map(|l| l.id.clone()).collect())
        .unwrap_or_default();
    let known = match distill::known_pointers(&client).await {
        Ok(k) => {
            if k.unreadable {
                eprintln!(
                    "mecha: the board's answer carried no task list this build can read — task \
                     and project ids cross as kind words this run"
                );
            }
            if k.truncated {
                eprintln!(
                    "mecha: the board's answer was truncated — a task or project id it did not \
                     list crosses as its kind word this run"
                );
            }
            k
        }
        Err(e) => {
            eprintln!(
                "mecha: could not read the board for goal pointers — task and project ids \
                 cross as kind words this run: {e:#}"
            );
            distill::KnownPointers::none()
        }
    }
    .with_charter_lines(charter_lines)
    // The stores behind the two structural anchor kinds, read in-process: a
    // `trigger:` or `request:` pointer crosses whole only if its own store
    // still holds it. A store that cannot be read admits nothing of its kind
    // — the pointer drops to its kind word, never a guess.
    // An absent store holds nothing to resolve — a true zero, said nowhere;
    // one that cannot be read, or a trigger file that did not load, is a
    // finding and is said, on the board's own wording above, because the
    // pointers it would have admitted now cross as bare kind words.
    .with_triggers(
        match mecha_core::trigger::TriggerStore::open_existing_default() {
            None => Vec::new(),
            Some(store) => match store.list() {
                Ok((triggers, problems)) => {
                    if !problems.is_empty() {
                        eprintln!(
                            "mecha: {} trigger file(s) did not load — runs anchored to them \
                         cross as the kind word `trigger` this run: {}",
                            problems.len(),
                            problems.join("; ")
                        );
                    }
                    triggers.into_iter().map(|t| t.name).collect()
                }
                Err(e) => {
                    eprintln!(
                        "mecha: could not read the trigger store for goal pointers — trigger \
                     ids cross as kind words this run: {e:#}"
                    );
                    Vec::new()
                }
            },
        },
    )
    .with_requests(
        match mecha_core::frontdoor::Frontdoor::open_existing_default() {
            None => Vec::new(),
            Some(frontdoor) => match frontdoor.records_counting() {
                Ok((records, skipped)) => {
                    if skipped > 0 {
                        eprintln!(
                            "mecha: {skipped} front-door record(s) could not be read — runs \
                             anchored to them cross as the kind word `request` this run"
                        );
                    }
                    records.into_iter().map(|r| r.seq).collect()
                }
                Err(e) => {
                    eprintln!(
                        "mecha: could not read the front-door store for goal pointers — \
                     request ids cross as kind words this run: {e:#}"
                    );
                    Vec::new()
                }
            },
        },
    );

    // The appraisal leg's stores, read once per run (row 2a-2): every store
    // the signed errors read — closures and workflows too, so the owner's
    // acts on a task reach the appraiser, where the episode's tag above
    // reads drafts only and stays as it was — and the comparisons drawn
    // from each session. Best-effort, and an unreadable one is said to the
    // appraiser rather than read as empty.
    let appraiser = if local {
        Some(Appraiser::open())
    } else {
        eprintln!(
            "mecha: {provider_name} is not a local provider — sessions are distilled but not \
             appraised (R29: an appraisal reads the whole transcript, and transcripts stay on \
             the local model)"
        );
        None
    };
    let mut tally = AppraisalTally::default();

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
        //
        // The appraisal's evidence comes off the same read (row 2a-2): the
        // transcript the appraiser is shown and the provenance its record
        // is stamped with must be one snapshot of a file that may still be
        // growing.
        let (transcript, evidence) = match SessionEvidence::read_with_transcript(path) {
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
                charter: charter.as_ref(),
                charter_unreadable,
                ..Default::default()
            },
            None,
        )
        .map(|built| built.appraisal);

        let rendered = distill::render_for_distill(&convo.messages, 6000, 18000);
        // One background seat for the pair of calls (`permit.rs`): the
        // episode and its appraisal run back to back on one conversation,
        // so the second lands on the slot holding the first's prefix. Only
        // on the local model: the seats are llama-server's, and a provider
        // elsewhere holds none of them.
        let seat = if local {
            take_seat(&meta.id).await
        } else {
            None
        };
        let turn = distiller.distill_turn(&rendered).await;
        if let (Some(appraiser), Ok(turn)) = (&appraiser, &turn) {
            // Shadow: whatever happens on this leg, the episode leg below is
            // unchanged — an appraisal that fails costs only itself.
            appraiser
                .appraise(
                    &distiller,
                    turn,
                    &transcript,
                    &evidence,
                    AppraisalContext {
                        session_id: &meta.id,
                        created_at: meta.created_at.to_rfc3339(),
                        charter: charter.as_ref(),
                        charter_unreadable,
                        known: &known,
                    },
                    &mut tally,
                )
                .await;
        }
        drop(seat);
        match turn.map(|t| t.distilled) {
            Ok(Some(out)) => {
                // Decide what may leave BEFORE writing the body: a carrier
                // describing a withheld correction would launder the claim
                // into episode prose, which the graph's extractor mines into
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
                // only in acting), where the graph is a *second automated
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
                    appraisal.as_ref().map(|a| (a, &known)),
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
                        // like the graph failing to pin one down, and the session
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
                            // theatre: anything the graph neither repaired nor
                            // queued went nowhere, and would otherwise
                            // leave no trace at all.
                            let accounted =
                                outcome.corrections_applied + outcome.corrections_unresolved;
                            let sent = sendable.len() as i64;
                            if accounted != sent || outcome.corrections_processed != sent {
                                eprintln!(
                                    "  WARNING: {sent} sent but the graph reports {} processed and \
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
    if appraiser.is_some() {
        println!("{}", tally.line());
    }
    Ok(())
}

/// Take a background seat, waiting for one if all are held — a nightly or
/// a detached `session_end` hook can wait; nothing interactive runs this.
/// The pool is a latency control, not a guard (`permit.rs`), so a pool that
/// cannot be read is said and the calls go ahead unseated.
async fn take_seat(session: &str) -> Option<mecha_core::permit::Held> {
    let pool = match super::tasks::permits() {
        Ok(pool) => pool,
        Err(e) => {
            eprintln!("mecha: the seat pool could not be opened ({e:#}); distilling unseated");
            return None;
        }
    };
    let what = format!("distill {session}");
    let mut said: Option<std::time::Instant> = None;
    loop {
        match pool.take(&what) {
            Ok(Ok(held)) => return Some(held),
            Ok(Err(holders)) => {
                if said.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(300)) {
                    said = Some(std::time::Instant::now());
                    eprintln!(
                        "mecha: all {} background model seat(s) are held ({}); waiting",
                        pool.capacity(),
                        holders
                            .iter()
                            .map(|p| p.what.as_deref().unwrap_or("unnamed"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            Err(e) => {
                eprintln!("mecha: a seat could not be taken ({e:#}); distilling unseated");
                return None;
            }
        }
    }
}

/// What the appraisal leg did this run (row 2a-2), for the closing line.
#[derive(Default)]
struct AppraisalTally {
    written_clean: usize,
    written_not_clean: usize,
    already: usize,
    /// Replies that stored nothing, by why.
    malformed: std::collections::BTreeMap<String, usize>,
    /// The provider failed on the follow-up, or the store could not be
    /// read or written.
    failed: usize,
    /// Wall-clock seconds of a seat the follow-up calls added, and the
    /// prompt tokens they sent and read from the server's cache.
    seconds: f64,
    /// Follow-up calls the provider answered — every one paid for, whatever
    /// became of its reply.
    calls: usize,
    prompt_tokens: u64,
    cached_tokens: u64,
}

impl AppraisalTally {
    fn line(&self) -> String {
        let malformed: usize = self.malformed.values().sum();
        let calls = self.calls;
        let why: Vec<String> = self
            .malformed
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        format!(
            "appraised {} session(s) ({} clean, {} not clean — the owner's alone) · {} already on \
             record · {} malformed, nothing stored{} · {} failed · {:.1}s of a seat over {} \
             follow-up call(s), {} of {} prompt token(s) from the server's cache",
            self.written_clean + self.written_not_clean,
            self.written_clean,
            self.written_not_clean,
            self.already,
            malformed,
            if why.is_empty() {
                String::new()
            } else {
                format!(" ({})", why.join(", "))
            },
            self.failed,
            self.seconds,
            calls,
            self.cached_tokens,
            self.prompt_tokens,
        )
    }
}

/// The per-session facts the appraisal leg needs from the run's loop.
struct AppraisalContext<'a> {
    session_id: &'a str,
    created_at: String,
    charter: Option<&'a mecha_core::charter::Charter>,
    charter_unreadable: bool,
    known: &'a distill::KnownPointers,
}

/// The appraisal leg's stores, opened once per run.
struct Appraiser {
    /// `Err` when the store cannot be opened: every session this run is
    /// counted as failed, and the episode leg is untouched.
    store: std::result::Result<AppraisalStore, String>,
    stores: mecha_core::appraisal::Stores,
    comparisons: std::result::Result<Vec<mecha_core::comparison::Comparison>, String>,
}

impl Appraiser {
    fn open() -> Appraiser {
        let store = AppraisalStore::open_default().map_err(|e| format!("{e:#}"));
        if let Err(e) = &store {
            eprintln!("mecha: the appraisal store could not be opened ({e}); appraising nothing");
        }
        let comparisons = match mecha_core::comparison::ComparisonStore::open_existing_default() {
            None => Ok(Vec::new()),
            Some(s) => match s.comparisons_counting() {
                Ok((rows, 0)) => Ok(rows),
                Ok((_, skipped)) => Err(format!("{skipped} line(s) could not be read")),
                Err(e) => Err(format!("{e:#}")),
            },
        };
        Appraiser {
            store,
            stores: mecha_core::appraisal::Stores::load(),
            comparisons,
        }
    }

    /// Appraise one session on the episode call's conversation and record
    /// it through the store's one door. Every outcome is counted; nothing
    /// here can stop the episode leg.
    #[allow(clippy::too_many_arguments)]
    async fn appraise(
        &self,
        distiller: &Distiller,
        turn: &distill::EpisodeTurn,
        transcript: &mecha_core::session::Transcript,
        evidence: &SessionEvidence,
        cx: AppraisalContext<'_>,
        tally: &mut AppraisalTally,
    ) {
        let id = cx.session_id;
        let store = match &self.store {
            Ok(store) => store,
            Err(_) => {
                tally.failed += 1;
                return;
            }
        };
        // One appraisal per session: asked before the model call the door
        // would refuse. A store that cannot answer is a failure, not "none".
        match store.on_record(evidence.session_id()) {
            Ok(Some(_)) => {
                tally.already += 1;
                return;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("· {id} — appraisal: the store could not be read ({e:#})");
                tally.failed += 1;
                return;
            }
        }
        let drafts = self.stores.drafts_of(id);
        let signed = mecha_core::appraisal::for_transcript(
            transcript,
            id,
            cx.created_at,
            self.stores.records(&drafts),
            None,
        )
        .map(|built| built.appraisal);
        let (brief, homeostat) = distill::AppraisalInputs::situation_of(transcript);
        let comparisons: Vec<&mecha_core::comparison::Comparison> = self
            .comparisons
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter(|c| c.pointers.session_id == id)
                    .collect()
            })
            .unwrap_or_default();
        // Past appraisals through the clean door only: a tainted one has no
        // way into another session's input.
        let clean = store.clean();
        let past = clean
            .as_ref()
            .map(|read| {
                read.same_situation_and_goal(evidence, mecha_core::appraisal_store::PAST_SHOWN)
            })
            .unwrap_or_default();
        let inputs = distill::render_appraisal_inputs(&distill::AppraisalInputs {
            evidence,
            charter: cx.charter,
            charter_unreadable: cx.charter_unreadable,
            brief,
            homeostat,
            drafts: &drafts,
            outbox_unreadable: self.stores.outbox_unreadable,
            signed: signed.as_ref(),
            comparisons: &comparisons,
            comparisons_unreadable: self.comparisons.is_err(),
            past: &past,
            past_unreadable: clean.is_err(),
            known: cx.known,
        });
        let answered = match distiller.appraise(turn, &inputs).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("· {id} — appraisal failed: {e:#}");
                tally.failed += 1;
                return;
            }
        };
        tally.calls += 1;
        tally.seconds += answered.elapsed.as_secs_f64();
        tally.prompt_tokens += answered.usage.total_input();
        tally.cached_tokens += answered.usage.cache_read_input_tokens;
        let draft = match answered.draft {
            Ok(d) => d,
            Err(why) => {
                eprintln!(
                    "· {id} — appraisal reply unusable ({}); nothing stored",
                    why.wire()
                );
                *tally.malformed.entry(why.wire()).or_default() += 1;
                return;
            }
        };
        match store.record(evidence, draft, distiller.model(), cx.known) {
            Ok(Recorded::Written {
                clean, grounding, ..
            }) => {
                if clean {
                    tally.written_clean += 1;
                } else {
                    tally.written_not_clean += 1;
                }
                println!(
                    "· {id} — appraised ({}; {} of {} claim(s) grounded) in {:.1}s of a seat, {} \
                     of {} prompt token(s) from the server's cache (the episode call: {:.1}s, \
                     {} prompt token(s))",
                    if clean {
                        "clean"
                    } else {
                        "not clean: the owner's alone"
                    },
                    grounding.offered - grounding.dropped,
                    grounding.offered,
                    answered.elapsed.as_secs_f64(),
                    answered.usage.cache_read_input_tokens,
                    answered.usage.total_input(),
                    turn.elapsed.as_secs_f64(),
                    turn.usage.total_input(),
                );
            }
            Ok(Recorded::AlreadyOnRecord { .. }) => tally.already += 1,
            Ok(Recorded::Empty) => {
                *tally
                    .malformed
                    .entry("no_interpretation".into())
                    .or_default() += 1
            }
            Err(e) => {
                eprintln!("· {id} — the appraisal could not be recorded: {e:#}");
                tally.failed += 1;
            }
        }
    }
}
