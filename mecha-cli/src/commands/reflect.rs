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

    let sessions = Session::list(&sessions_dir)?;
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
        let (_, convo) = match Session::load(path) {
            Ok(loaded) => loaded,
            Err(e) => {
                // A transcript that does not load is not this command's bug to
                // fix; skip it *without* marking it mined, so a later mecha
                // that can read it still gets the chance.
                eprintln!("skipping {}: {e:#}", meta.id);
                continue;
            }
        };

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
                    // surface and workspace are the session record's. Set
                    // here and not by the reflector, which saw prose.
                    r.situation = Some(mecha_core::situation::Situation::recorded(
                        &intervention.tools_before,
                        intervention.trigger.as_str(),
                        meta.kind,
                        Some(&meta.workspace),
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
    let todo: Vec<_> = store
        .reflexions()?
        .into_iter()
        .filter(|r| r.situation.is_none())
        .collect();
    if todo.is_empty() {
        println!("every reflection carries a situation — nothing to backfill");
        return Ok(());
    }
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
    let mut by_session: std::collections::HashMap<
        String,
        Result<
            (
                mecha_core::session::SessionMeta,
                Vec<mecha_core::learning::Intervention>,
            ),
            String,
        >,
    > = Default::default();
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
            let (meta, convo) =
                Session::load(path).map_err(|e| format!("session unreadable: {e:#}"))?;
            Ok((meta, extract_interventions(&convo.messages)))
        });
        match read {
            Err(why) => unmatched.push((r.id.clone(), why.clone())),
            Ok((meta, interventions)) => match backfill_situation(r, interventions, meta) {
                Backfilled::Matched(s) => updates.push((r.id.clone(), s)),
                Backfilled::NoMatch => unmatched.push((
                    r.id.clone(),
                    "no intervention with this trigger and text in the transcript".into(),
                )),
                Backfilled::Ambiguous(n) => {
                    unmatched.push((r.id.clone(), format!("fits {n} different tool windows")))
                }
            },
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
