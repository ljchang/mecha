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
use mecha_core::learning::{extract_interventions, LearningStore, Reflector, Trigger};
use mecha_core::session::Session;
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
    let _lock = if args.dry_run { None } else { Some(store.lock()?) };
    let mined = store.mined_sessions()?;

    let sessions = Session::list(&sessions_dir)?;
    let mut todo: Vec<_> = sessions
        .into_iter()
        .filter(|(meta, _)| !mined.contains(&meta.id))
        .collect();
    if let Some(limit) = args.limit {
        todo.truncate(limit);
    }

    if todo.is_empty() {
        println!("nothing to mine: every session is already reflected on");
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

        if args.dry_run {
            for i in &interventions {
                println!(
                    "{} [{}] {}",
                    meta.id,
                    i.trigger.as_str(),
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
            match reflector.reflect(intervention).await {
                Ok(Some(mut r)) => {
                    r.session_id = meta.id.clone();
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

        store.mark_mined(&meta.id)?;
        sessions_mined += 1;
    }

    if args.dry_run {
        println!(
            "dry run: {sessions_mined} session(s) with {interventions_found} intervention(s); \
             nothing written"
        );
    } else {
        store.commit(&format!(
            "reflect: {sessions_mined} session(s), {reflections_written} reflection(s)"
        ));
        println!(
            "mined {sessions_mined} session(s): {interventions_found} intervention(s), \
             {reflections_written} reflection(s) → {}",
            store.root().join("reflections.jsonl").display()
        );
    }
    Ok(())
}
