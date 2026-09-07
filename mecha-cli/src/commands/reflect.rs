//! `mecha reflect` — mine recorded sessions for the user stepping in, and turn
//! each intervention into a reflection.
//!
//! The behavior half of the self-learning system needs nothing the harness
//! does not already record: a mid-run steer, an approval denial, and a
//! corrective follow-up turn are all in the session JSONL. This command reads
//! sessions that have not been mined yet, extracts those moments (pure,
//! unit-tested in core), asks a model for the reusable lesson behind each, and
//! appends the results to `~/.mecha/learning/reflections.jsonl`.
//!
//! Idempotent by design: mined session ids are recorded, so running it nightly
//! (or after every session) only ever pays for the new ones.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::learning::{
    evidence_for, extract_interventions, Evidence, Intervention, LearningStore, Origin, Reflector,
    Trigger,
};
use mecha_core::session::{Session, TaintTimeline};
use std::path::{Path, PathBuf};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Directory of session transcripts. Defaults to the standard location.
    #[arg(long)]
    pub sessions_dir: Option<PathBuf>,

    /// List what would be mined without calling a model or writing anything.
    #[arg(long)]
    pub dry_run: bool,

    /// Mine at most this many sessions this run.
    #[arg(long)]
    pub limit: Option<usize>,

    /// One-shot backfill: re-mine the sessions whose reflections the
    /// provenance gate excluded, through the clean-evidence path — the
    /// user's own words and tool names, with the tainted excerpts withheld.
    /// Idempotent: an intervention already carrying a user-turns reflection
    /// is skipped, and clean-covered interventions are never re-mined.
    #[arg(long)]
    pub remine_untrusted: bool,

    /// One-shot backfill: give the reflections mined before the situation
    /// field a situation, recomputed from their transcripts — the tool
    /// window, surface and workspace — with no model call. A reflection
    /// whose intervention cannot be found once in its transcript stays
    /// without one. Idempotent: a reflection that has a situation is never
    /// touched. `--dry-run` reports what would be written.
    #[arg(long, conflicts_with_all = ["remine_untrusted", "limit"])]
    pub backfill_situations: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let sessions_dir = match &args.sessions_dir {
        Some(dir) => dir.clone(),
        None => Session::default_dir()?,
    };
    let store = LearningStore::open(LearningStore::default_root()?)?;
    if args.backfill_situations {
        return backfill_situations(&store, &sessions_dir, args.dry_run);
    }
    // The writer lock, taken *before* reading what has been mined — that read
    // is where the race lives now that a session_end hook fires a detached
    // reflect at every close: two closes in quick succession must not both
    // see the same session as unmined. Blocking is right: the second pass
    // waits, re-reads, finds nothing left, and exits. A dry run only reads.
    let _lock = if args.dry_run {
        None
    } else {
        Some(store.lock()?)
    };
    let mined = store.mined_sessions()?;

    // The backfill re-visits exactly the sessions whose lessons the gate
    // excluded; its dedup key is (session, intervention text) against the
    // user-turns reflections already on file, so running it twice is free.
    let excluded_sessions: std::collections::HashSet<String> = if args.remine_untrusted {
        store
            .reflexions()?
            .iter()
            .filter(|r| r.origin != Origin::Clean && !r.session_id.is_empty())
            .map(|r| r.session_id.clone())
            .collect()
    } else {
        Default::default()
    };
    let already_user_turns: std::collections::HashSet<(String, String)> = if args.remine_untrusted {
        store
            .reflexions()?
            .iter()
            .filter(|r| r.evidence == Evidence::UserTurns)
            .map(|r| (r.session_id.clone(), r.intervention.clone()))
            .collect()
    } else {
        Default::default()
    };

    // `list_counting`, so the reconcile below can say when a cited
    // transcript is one the store could not read rather than one deleted.
    let (sessions, unreadable_sessions) = Session::list_counting(&sessions_dir)?;
    let paths: std::collections::HashMap<String, PathBuf> = sessions
        .iter()
        .map(|(meta, path)| (meta.id.clone(), path.clone()))
        .collect();
    let mut todo: Vec<_> = sessions
        .into_iter()
        .filter(|(meta, _)| {
            if args.remine_untrusted {
                excluded_sessions.contains(&meta.id)
            } else {
                !mined.contains(&meta.id)
            }
        })
        .collect();
    if let Some(limit) = args.limit {
        todo.truncate(limit);
    }

    // The outbox pass mines a different kind of correction: a `sent` item
    // whose released arguments differ from the drafted ones is the user
    // editing mecha's writing, recorded structurally by `mecha outbox edit`.
    // Open non-creating: reflect must not conjure an outbox as a side effect.
    //
    // **Messages only, and this is a security filter rather than a tidiness
    // one.** A publish's arguments are a path and a visibility flag, so its
    // diff is a changed directory name — and a `writing`-domain reflection
    // rides in every future run's system prompt inside the cached prefix. That
    // is the longest-half-life path anything in this project has, and teaching
    // it voice rules from filesystem noise is the same mistake as learning from
    // `"Blocked by a hook:"`: machine bookkeeping read as a human correction.
    // The filter is structural, before any prompt is built.
    let outbox = mecha_core::outbox::OutboxStore::open_existing_default();
    let outbox_mined = store.mined_outbox()?;
    let outbox_todo: Vec<_> = match &outbox {
        Some(ob) => ob
            .items()?
            .into_iter()
            .filter(|i| i.mineable_as_writing() && !outbox_mined.contains(&i.id))
            .collect(),
        None => Vec::new(),
    };

    // Every pass, before anything is mined or batched: the workspace on
    // each recorded situation against the run record. Not a flag a human
    // runs once after installing — the nightly's `learn --auto` follows
    // this pass, and a row stamped with a jail before the key existed
    // would have scoped a rule to nowhere while the flag waited to be run
    // (found on review). Free when there is nothing to apply.
    reconcile_recorded_keys(&store, &paths, unreadable_sessions, args.dry_run)?;

    if todo.is_empty() && outbox_todo.is_empty() {
        println!("nothing to mine: every session and sent draft is already reflected on");
        return Ok(());
    }

    // The reflector is only built when something needs it, so --dry-run and
    // the nothing-new case work with no provider configured at all.
    let reflector = if args.dry_run {
        None
    } else {
        let cwd = std::env::current_dir().context("cannot determine the working directory")?;
        let cfg = Config::load(&cwd)?;
        let (name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
        let provider = mecha_core::provider::build(provider_cfg)?;
        let model = global.model.clone().or_else(|| provider_cfg.model.clone());
        let reflector = Reflector::new(provider, model);
        eprintln!("reflecting with {} ({name})", reflector.model());
        Some(reflector)
    };

    let mut sessions_mined = 0usize;
    let mut interventions_found = 0usize;
    let mut reflections_written = 0usize;

    for (meta, path) in &todo {
        // The workspace the session's rules block was matched against, off
        // its run record — the key a match presents. Never the session's
        // jail: on `serve` the block is rendered against the producer root
        // and each session is jailed a level below, so a lesson stamped
        // with the jail scoped its rule to a workspace no run presents
        // (found on review). A record from before the field gives none.
        // One read of the transcript for the conversation and its run
        // records both — `load` and `run_configs` each parsed the whole
        // file, twice per session on the nightly's hot path (found on
        // review).
        let t = match Session::read(path) {
            Ok(t) => t,
            Err(e) => {
                // A transcript that does not load is not this command's bug to
                // fix; skip it *without* marking it mined, so a later mecha
                // that can read it still gets the chance.
                eprintln!("skipping {}: {e:#}", meta.id);
                continue;
            }
        };

        let convo = &t.convo;
        let interventions = extract_interventions(&convo.messages);
        interventions_found += interventions.len();

        // Provenance, read from the transcript's recorded taint — not from
        // anything a model says. A reflection from a conversation that held
        // third-party content becomes a rule in every future run's prompt,
        // so the classification must fail closed: a timeline that cannot be
        // read covers nothing, and uncovered means Untrusted.
        let timeline = Session::taint_timeline(path).unwrap_or_else(|e| {
            eprintln!(
                "· cannot read taint from {}: {e:#}; treating as untrusted",
                meta.id
            );
            TaintTimeline::default()
        });

        if args.dry_run {
            for i in &interventions {
                let (_, origin, evidence) = evidence_for(timeline.covering(i.at), i);
                println!(
                    "{} [{}] ({}) {}",
                    meta.id,
                    i.trigger.as_str(),
                    match (origin, evidence) {
                        (Origin::Clean, Evidence::Full) => "clean",
                        (Origin::Clean, Evidence::UserTurns) => "clean, user-turns only",
                        (Origin::Untrusted, _) => "untrusted",
                        (Origin::Derived, _) => "derived",
                    },
                    i.text.lines().next().unwrap_or("")
                );
            }
            sessions_mined += 1;
            continue;
        }

        // All-or-nothing per session. An error here is usually the provider
        // being down — and reflect now runs unattended, where "print and mark
        // mined anyway" turns an outage into silent permanent loss. Nothing
        // is appended until every intervention reflected, so a retry after a
        // partial failure cannot duplicate the ones that succeeded; the
        // session stays unmined and the next run pays again, which local
        // inference makes free.
        let reflector = reflector.as_ref().expect("built unless dry-run");
        let mut pending = Vec::new();
        let mut failed = false;
        for intervention in &interventions {
            // What the reflector may see, decided per intervention: full
            // excerpts under provably clean coverage, the user's own words
            // and tool names otherwise. See `learning::evidence_for`.
            let (input, origin, evidence) =
                evidence_for(timeline.covering(intervention.at), intervention);
            if args.remine_untrusted {
                // Backfill visits only what the gate excluded: an
                // intervention that was clean is already on file in full,
                // and one already re-mined must not double.
                if evidence != Evidence::UserTurns
                    || already_user_turns.contains(&(meta.id.clone(), intervention.text.clone()))
                {
                    continue;
                }
            }
            match reflector.reflect(&input).await {
                Ok(Some(mut r)) => {
                    r.session_id = meta.id.clone();
                    r.origin = origin;
                    r.evidence = evidence;
                    // Where it happened, from what the miner already held:
                    // the tool window is registry names (it survives the
                    // user-evidence-only view for the same reason), the
                    // surface and workspace are the run record's matched
                    // ones — never `meta.kind`, never the jail. Set here
                    // and not by the reflector, which saw prose.
                    // The keys of the run record covering *this*
                    // intervention, not the session's first: a session may
                    // hold runs matched on different keys (a resumed
                    // question, a `/model` switch), and `config_covering`
                    // is the exact answer per message (found on review).
                    let (matched_workspace, matched_surface) = keys_covering(&t, intervention.at);
                    r.situation = Some(mecha_core::situation::Situation::recorded(
                        &intervention.tools_before,
                        intervention.trigger.as_str(),
                        matched_surface,
                        matched_workspace.as_deref(),
                    ));
                    pending.push(r);
                }
                Ok(None) => {
                    if intervention.trigger != Trigger::Followup {
                        // Steers and denials are unambiguous interventions; a
                        // skip there is worth seeing. Followup skips are the
                        // common case and would only be noise.
                        eprintln!("· [{}] no lesson drawn", intervention.trigger.as_str());
                    }
                }
                Err(e) => {
                    eprintln!(
                        "· reflection failed: {e:#}\n  leaving {} unmined so a later run retries",
                        meta.id
                    );
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        for r in &pending {
            store.append_reflexion(r)?;
            reflections_written += 1;
            println!("· [{}] {}", r.trigger, r.reflexion_text);
        }

        if !args.remine_untrusted {
            store.mark_mined(&meta.id)?;
        }
        sessions_mined += 1;
    }

    // The outbox pass. Per-item rather than all-or-nothing: items are
    // independent corrections, so one reflection failure leaves only that
    // item unmined for the next run.
    let mut edits_mined = 0usize;
    for item in &outbox_todo {
        let intervention = outbox_intervention(item);
        if args.dry_run {
            println!(
                "{} [edit] {} draft edited before sending",
                item.id, item.tool
            );
            continue;
        }
        let reflector = reflector.as_ref().expect("built unless dry-run");
        match reflector.reflect(&intervention).await {
            Ok(reflected) => {
                if let Some(mut r) = reflected {
                    // The drafting session, when the front-end knew it — the
                    // same lineage a behavior reflection carries.
                    r.session_id = item.session_id.clone().unwrap_or_default();
                    // The item snapshots the conversation's taint at staging,
                    // which is exactly the provenance question: was there
                    // third-party text in context when this draft was written.
                    r.origin = if item.taint.untrusted {
                        Origin::Untrusted
                    } else {
                        Origin::Clean
                    };
                    // The drafting tool is the focus: a lesson from editing
                    // a mail draft scopes to `mail_send` and loads only
                    // where that tool is registered. The item records no
                    // surface or workspace.
                    r.situation = Some(mecha_core::situation::Situation::recorded(
                        std::slice::from_ref(&item.tool),
                        Trigger::Edit.as_str(),
                        None,
                        None,
                    ));
                    store.append_reflexion(&r)?;
                    reflections_written += 1;
                    println!("· [edit] {}", r.reflexion_text);
                }
                // Mined either way: a skip means the edit taught nothing
                // (a typo fix), and re-arguing it nightly will not change
                // that.
                store.mark_outbox_mined(&item.id)?;
                edits_mined += 1;
            }
            Err(e) => {
                eprintln!(
                    "· reflection failed: {e:#}\n  leaving outbox item {} unmined so a \
                     later run retries",
                    item.id
                );
            }
        }
    }

    if args.dry_run {
        println!(
            "dry run: {sessions_mined} session(s) with {interventions_found} intervention(s), \
             {} edited draft(s); nothing written",
            outbox_todo.len()
        );
    } else {
        store.commit(&format!(
            "reflect: {sessions_mined} session(s), {edits_mined} draft edit(s), \
             {reflections_written} reflection(s)"
        ));
        println!(
            "mined {sessions_mined} session(s) and {edits_mined} draft edit(s): \
             {interventions_found} intervention(s), {reflections_written} reflection(s) → {}",
            store.root().join("reflections.jsonl").display()
        );
    }
    Ok(())
}

/// Frame one edited-then-sent outbox item as an intervention for the
/// writing-domain reflector: the draft is the context, the diff is what the
/// user did, the sent version is the aftermath.
/// The keys a session's rules block was matched against, off its first
/// run record (`RunConfig::rules_workspace`, `rules_surface`) — what a
/// match presents, never the session's jail and never `SessionMeta::kind`.
/// `None` for a record from before the field, or a run that declared
/// none; `Err` when the transcript cannot be read, which confirms nothing
/// either way.
fn matched_keys_of(
    path: &Path,
) -> std::result::Result<(Option<PathBuf>, Option<mecha_core::session::SessionKind>), String> {
    // Through `Session::read`, the same reader the miner holds, so the
    // reconcile and the miner cannot disagree about one record by parsing
    // it two ways (found on review).
    Session::read(path)
        .map(|t| matched_keys_in(&t))
        .map_err(|e| format!("session unreadable: {e:#}"))
}

/// The keys of the run record covering message `at` of a transcript
/// already read (`Transcript::config_covering`): what the miner and the
/// backfill stamp on an intervention, since a session may hold runs
/// matched on different keys. `(None, None)` for a transcript recorded
/// before configs were kept.
fn keys_covering(
    t: &mecha_core::session::Transcript,
    at: usize,
) -> (Option<PathBuf>, Option<mecha_core::session::SessionKind>) {
    t.config_covering(at)
        .map(|rc| (rc.rules_workspace.clone(), rc.rules_surface))
        .unwrap_or((None, None))
}

/// The first run record's keys off a transcript already read — what the
/// reconcile compares an already-stamped row against, since a stored
/// reflection carries no message index; a session with runs matched on
/// different keys is reconciled to its first.
fn matched_keys_in(
    t: &mecha_core::session::Transcript,
) -> (Option<PathBuf>, Option<mecha_core::session::SessionKind>) {
    t.configs
        .first()
        .map(|rc| (rc.rules_workspace.clone(), rc.rules_surface))
        .unwrap_or((None, None))
}

/// Reconcile the keys on every recorded situation — the workspace and the
/// surface — with what its session's rules block was matched against,
/// decided key by key by `learning::reconcile_key` before anything is
/// written: a key the row does not record is never added (an outbox
/// edit's lesson scopes by tools on purpose, and a key added here would
/// narrow it on no conviction); a session that cannot be found or read
/// confirms nothing and the row stays, with the reason; a record that was
/// read and disagrees sets what it names, `None` included. Runs on every
/// `reflect` pass — the nightly's `learn --auto` follows it — so
/// correctness does not depend on a human running a flag first (found on
/// review). Commits its own writes; a pass with nothing to apply does not
/// touch the file. Returns how many rows were (or would be) rewritten.
fn reconcile_recorded_keys(
    store: &LearningStore,
    paths: &std::collections::HashMap<String, PathBuf>,
    unreadable_sessions: usize,
    dry_run: bool,
) -> Result<usize> {
    use mecha_core::learning::{reconcile_key, KeyReconcile, KeyUpdate};
    let present: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| {
            r.situation
                .as_ref()
                .is_some_and(|s| s.workspace.is_some() || s.surface.is_some())
                && !r.session_id.is_empty()
        })
        .collect();
    if present.is_empty() {
        return Ok(0);
    }
    type Matched = (Option<PathBuf>, Option<mecha_core::session::SessionKind>);
    let mut record_by_session: std::collections::HashMap<
        String,
        std::result::Result<Matched, String>,
    > = Default::default();
    let mut reconcile: Vec<(String, KeyUpdate)> = Vec::new();
    let mut to_none = 0usize;
    let mut left = 0usize;
    let show_w = |w: Option<&PathBuf>| {
        w.map(|w| w.display().to_string())
            .unwrap_or_else(|| "none".into())
    };
    let show_k =
        |k: Option<mecha_core::session::SessionKind>| k.map(|k| k.as_str()).unwrap_or("none");
    for r in &present {
        let Some(s) = &r.situation else { continue };
        let record = record_by_session
            .entry(r.session_id.clone())
            .or_insert_with(|| match paths.get(&r.session_id) {
                Some(path) => matched_keys_of(path),
                None if unreadable_sessions > 0 => Err(format!(
                    "no readable session matching \"{}\" — {unreadable_sessions} \
                     transcript(s) in the store could not be read, and it may be one",
                    r.session_id
                )),
                None => Err(format!("no session matching \"{}\"", r.session_id)),
            })
            .clone();
        let mut update = KeyUpdate::default();
        match reconcile_key(
            s.workspace.as_ref(),
            record
                .as_ref()
                .map(|(w, _)| w.as_ref())
                .map_err(Clone::clone),
        ) {
            KeyReconcile::Keep => {}
            KeyReconcile::Unreadable(why) => {
                left += 1;
                println!(
                    "· {} keeps workspace {} — {why}",
                    r.id,
                    show_w(s.workspace.as_ref())
                );
            }
            KeyReconcile::Set(matched) => {
                if matched.is_none() {
                    to_none += 1;
                }
                println!(
                    "· {} workspace {} → {}",
                    r.id,
                    show_w(s.workspace.as_ref()),
                    if matched.is_none() {
                        "none (the run record carries none)".to_string()
                    } else {
                        show_w(matched.as_ref())
                    }
                );
                update.workspace = Some(matched);
            }
        }
        match reconcile_key(
            s.surface.as_ref(),
            record
                .as_ref()
                .map(|(_, k)| k.as_ref())
                .map_err(Clone::clone),
        ) {
            KeyReconcile::Keep => {}
            KeyReconcile::Unreadable(why) => {
                // Counted once per row, above, when the workspace was also
                // recorded; a surface-only row is counted here.
                if s.workspace.is_none() {
                    left += 1;
                    println!("· {} keeps surface {} — {why}", r.id, show_k(s.surface));
                }
            }
            KeyReconcile::Set(matched) => {
                if matched.is_none() {
                    to_none += 1;
                }
                println!(
                    "· {} surface {} → {}",
                    r.id,
                    show_k(s.surface),
                    if matched.is_none() {
                        "none (the run record carries none)".to_string()
                    } else {
                        show_k(matched).to_string()
                    }
                );
                update.surface = Some(matched);
            }
        }
        if !update.is_empty() {
            reconcile.push((r.id.clone(), update));
        }
    }
    let written = if dry_run {
        reconcile.len()
    } else {
        let written = store.reconcile_keys(&reconcile, &chrono::Utc::now().to_rfc3339())?;
        if written > 0 {
            store.commit(&format!(
                "reflect: {written} situation(s)' keys reconciled with the run record"
            ));
        }
        written
    };
    if written + left > 0 {
        println!(
            "{} {written} situation(s) of {} recorded against the run record ({to_none} key(s) \
             to none: the record carries none; {left} left as recorded: session not found or \
             unreadable)",
            if dry_run {
                "would reconcile"
            } else {
                "reconciled"
            },
            present.len()
        );
    }
    Ok(written)
}

fn outbox_intervention(item: &mecha_core::outbox::OutboxItem) -> Intervention {
    let pretty =
        |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    Intervention {
        trigger: Trigger::Edit,
        context: format!(
            "mecha drafted this `{}` call (outbox item {}):\n{}",
            item.tool,
            item.id,
            pretty(&item.args_before)
        ),
        text: format!(
            "the user edited the draft before releasing it:\n{}",
            mecha_core::outbox::diff_args(&item.args_before, &item.args)
        ),
        aftermath: format!("the user sent the edited version:\n{}", pretty(&item.args)),
        // Not a transcript position: an edit lives in the outbox item, and its
        // provenance comes from the item's taint snapshot, not a timeline.
        at: 0,
        tools_before: Vec::new(),
        tools_after: Vec::new(),
    }
}

/// `--backfill-situations`: `docs/GOAL-SYSTEM-DESIGN.md` §17.7 item 6.
///
/// Deterministic end to end — `extract_interventions` over each transcript,
/// the match on (session, trigger, intervention text), `Situation::recorded`
/// off the window and the session header — so it costs no model and can be
/// re-run. Each transcript is read once for all the reflections that cite
/// it. A reflection is left without a situation, and said so, when its
/// session cannot be found or read, when no intervention in the transcript
/// carries its trigger and text (a compaction since mining, or an outbox
/// edit with no transcript), or when several do with different windows;
/// absent is the honest reading and the pass never picks one. The goal is
/// never backfilled.
fn backfill_situations(store: &LearningStore, sessions_dir: &Path, dry_run: bool) -> Result<()> {
    use mecha_core::learning::{backfill_situation, extract_interventions, Backfilled};
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    // The store listed once — `Session::find` is a full scan of the
    // directory per call (found on review) — then one read per cited
    // session, shared by every reflection that cites it.
    // `list_counting`, not `list`: a transcript whose header cannot be read
    // is absent from the map, and a reflection citing it must not read as
    // citing a session that was deleted (found on review) — the count is
    // carried into that reason and the summary line.
    let (listed, unreadable_sessions) = Session::list_counting(sessions_dir)?;
    let paths: std::collections::HashMap<String, std::path::PathBuf> = listed
        .into_iter()
        .map(|(meta, path)| (meta.id, path))
        .collect();
    // The workspace reconcile first — the ordinary pass runs it too, and
    // this flag is the place to run it by hand and read every line.
    reconcile_recorded_keys(store, &paths, unreadable_sessions, dry_run)?;
    let todo: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| r.situation.is_none())
        .collect();
    if todo.is_empty() {
        println!("every reflection carries a situation — nothing to backfill");
        return Ok(());
    }
    // One session read: its record, its interventions, and the workspace
    // its rules block was matched against; or why it could not be read.
    type SessionRead = Result<
        (
            Vec<mecha_core::learning::Intervention>,
            mecha_core::session::Transcript,
        ),
        String,
    >;
    let mut by_session: std::collections::HashMap<String, SessionRead> = Default::default();
    let mut updates: Vec<(String, mecha_core::situation::Situation)> = Vec::new();
    let mut unmatched: Vec<(String, String)> = Vec::new();
    for r in &todo {
        if r.session_id.is_empty() {
            unmatched.push((r.id.clone(), "no session recorded (an outbox edit)".into()));
            continue;
        }
        let read = by_session.entry(r.session_id.clone()).or_insert_with(|| {
            let path = paths.get(&r.session_id).ok_or_else(|| {
                if unreadable_sessions > 0 {
                    format!(
                        "no readable session matching \"{}\" — {unreadable_sessions} \
                             transcript(s) in the store could not be read, and it may be one",
                        r.session_id
                    )
                } else {
                    format!("no session matching \"{}\"", r.session_id)
                }
            })?;
            let t = Session::read(path).map_err(|e| format!("session unreadable: {e:#}"))?;
            Ok((extract_interventions(&t.convo.messages), t))
        });
        match read {
            Err(why) => unmatched.push((r.id.clone(), why.clone())),
            Ok((interventions, t)) => {
                match backfill_situation(r, interventions, &|at| keys_covering(t, at)) {
                    Backfilled::Matched(s) => updates.push((r.id.clone(), s)),
                    Backfilled::NoMatch => unmatched.push((
                        r.id.clone(),
                        "no intervention with this trigger and text in the transcript".into(),
                    )),
                    Backfilled::Ambiguous(n) => {
                        unmatched.push((r.id.clone(), format!("fits {n} different tool windows")))
                    }
                }
            }
        }
    }
    for (id, s) in &updates {
        println!("· {id} ← {}", s.describe());
    }
    for (id, why) in &unmatched {
        println!("· {id} stays without a situation — {why}");
    }
    let verb = if dry_run {
        "would recompute"
    } else {
        "recomputed"
    };
    let written = if dry_run {
        updates.len()
    } else {
        let written = store.set_situations(&updates, &chrono::Utc::now().to_rfc3339())?;
        // Committed on its own, like every batch pass over this store: the
        // rewrite changes which region batches the next `learn --auto`
        // argues, and left uncommitted it would ride into the next
        // nightly's `reflect: 0 session(s)` commit (found on review).
        if written > 0 {
            store.commit(&format!(
                "reflect --backfill-situations: {written} situation(s) recomputed, {} left absent",
                unmatched.len()
            ));
        }
        written
    };
    println!(
        "{verb} {written} of {} situation(s); {} left absent, {} session(s) read{}",
        todo.len(),
        unmatched.len(),
        by_session.values().filter(|r| r.is_ok()).count(),
        if unreadable_sessions > 0 {
            format!(", {unreadable_sessions} transcript(s) in the store unreadable")
        } else {
            String::new()
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::session::{Record, RunConfig, SessionKind, SessionMeta};

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn session_in(dir: &Path, jail: &str, matched: Option<&str>) -> Session {
        session_on(
            dir,
            jail,
            matched,
            Some(SessionKind::Web),
            Some(SessionKind::Web),
        )
    }

    fn session_on(
        dir: &Path,
        jail: &str,
        matched: Option<&str>,
        kind: Option<SessionKind>,
        matched_surface: Option<SessionKind>,
    ) -> Session {
        let s = Session::create(
            dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                workspace: PathBuf::from(jail),
                title: None,
                kind,
            },
        )
        .unwrap();
        s.append(&Record::Config(RunConfig {
            workspace: PathBuf::from(jail),
            rules_workspace: matched.map(PathBuf::from),
            rules_surface: matched_surface,
            ..Default::default()
        }))
        .unwrap();
        s
    }

    /// The miner stamps the workspace and the surface the block was
    /// matched against, never the session's jail and never its kind — the
    /// bug that recurred on `serve`, Slack and task sessions, each of which
    /// jails a run somewhere the block was not rendered, and the board's
    /// task door, which records a task while the block was matched as web.
    /// Fails on the old reads of `meta.workspace` and `meta.kind`.
    #[test]
    fn the_miner_reads_the_matched_keys_and_never_the_jail_or_the_kind() {
        let dir = scratch("mecha-reflect-test");
        let s = session_in(
            &dir,
            "/home/x/.mecha/work/web/main",
            Some("/home/x/.mecha/work/web"),
        );
        assert_eq!(
            s.meta.workspace,
            PathBuf::from("/home/x/.mecha/work/web/main")
        );
        let (matched, surface) = matched_keys_of(&s.path).unwrap();
        assert_eq!(
            matched.as_deref(),
            Some(Path::new("/home/x/.mecha/work/web"))
        );
        let recorded = mecha_core::situation::Situation::recorded(
            &["shell".into()],
            "denial",
            surface,
            matched.as_deref(),
        );
        assert_eq!(
            recorded.workspace.as_deref(),
            Some(Path::new("/home/x/.mecha/work/web")),
            "the key a match presents"
        );
        assert_ne!(
            recorded.workspace,
            Some(s.meta.workspace.clone()),
            "not the jail"
        );
        // A record from before the fields: no key, never the jail or kind.
        let old = session_on(&dir, "/jail", None, Some(SessionKind::Tui), None);
        assert_eq!(matched_keys_of(&old.path).unwrap(), (None, None));
        // The board's task door on serve: recorded as a task, matched as
        // web — the surface stamped is the matched one, never the kind.
        let door = session_on(
            &dir,
            "/jail",
            Some("/root"),
            Some(SessionKind::Task),
            Some(SessionKind::Web),
        );
        assert_eq!(
            matched_keys_of(&door.path).unwrap().1,
            Some(SessionKind::Web)
        );
        assert_eq!(door.meta.kind, Some(SessionKind::Task));
        // No transcript at all: unreadable, not "none".
        assert!(matched_keys_of(&dir.join("missing.jsonl")).is_err());
    }

    /// The pass over a store: a jailed row goes to the matched keys, a
    /// row whose session is gone stays as it is, a row with no key is
    /// never given one, and the second pass writes nothing.
    #[test]
    fn the_reconcile_pass_sets_from_a_read_record_and_leaves_the_rest() {
        use mecha_core::learning::Reflexion;
        let dir = scratch("mecha-reconcile-pass");
        let sessions = dir.join("sessions");
        let store = LearningStore::open(dir.join("learning")).unwrap();
        // The door: recorded as a task, the block matched as web.
        let s = session_on(
            &sessions,
            "/jail",
            Some("/root"),
            Some(SessionKind::Task),
            Some(SessionKind::Web),
        );
        let situated = |id: &str, session_id: &str, ws: Option<&str>| {
            let r = Reflexion {
                id: id.into(),
                domain: "behavior".into(),
                session_id: session_id.into(),
                trigger: "denial".into(),
                context: "c".into(),
                intervention: "Denied by the user: no".into(),
                reflexion_text: "t".into(),
                error_type: None,
                confidence: None,
                is_processed: false,
                leap_run_id: None,
                created_at: "2026-09-07T00:00:00Z".into(),
                origin: mecha_core::learning::Origin::Clean,
                evidence: mecha_core::learning::Evidence::Full,
                edited_at: None,
                dropped_at: None,
                dropped_reason: None,
                // Stamped the old way: the jail, and the session's kind.
                situation: Some(mecha_core::situation::Situation::recorded(
                    &["shell".into()],
                    "denial",
                    ws.map(|_| SessionKind::Task),
                    ws.map(Path::new),
                )),
                situation_recomputed_at: None,
            };
            store.append_reflexion(&r).unwrap();
        };
        situated("jailed", &s.meta.id, Some("/jail"));
        situated("agrees", &s.meta.id, Some("/root"));
        situated("gone", "20260101T000000-deadbeef", Some("/jail"));
        situated("edit", &s.meta.id, None);
        let (listed, unreadable) = Session::list_counting(&sessions).unwrap();
        let paths: std::collections::HashMap<String, PathBuf> =
            listed.into_iter().map(|(m, p)| (m.id, p)).collect();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            2,
            "jailed (both keys) and agrees (the surface alone)"
        );
        let sit = |id: &str| store.reflexion(id).unwrap().situation.unwrap();
        assert_eq!(sit("jailed").workspace.as_deref(), Some(Path::new("/root")));
        assert_eq!(
            sit("jailed").surface,
            Some(SessionKind::Web),
            "the door: matched as web, whatever the session is recorded as"
        );
        assert_eq!(sit("agrees").workspace.as_deref(), Some(Path::new("/root")));
        assert_eq!(sit("agrees").surface, Some(SessionKind::Web));
        assert_eq!(
            sit("gone").workspace.as_deref(),
            Some(Path::new("/jail")),
            "unreadable: left"
        );
        assert_eq!(sit("gone").surface, Some(SessionKind::Task));
        assert_eq!(sit("edit").workspace, None, "never given a key");
        assert_eq!(sit("edit").surface, None);
        let file = dir.join("learning").join("reflections.jsonl");
        let before = std::fs::read(&file).unwrap();
        assert_eq!(
            reconcile_recorded_keys(&store, &paths, unreadable, false).unwrap(),
            0
        );
        assert_eq!(
            std::fs::read(&file).unwrap(),
            before,
            "free the second time"
        );
    }
}
