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
use std::path::PathBuf;

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
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let sessions_dir = match &args.sessions_dir {
        Some(dir) => dir.clone(),
        None => Session::default_dir()?,
    };
    let store = LearningStore::open(LearningStore::default_root()?)?;
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
