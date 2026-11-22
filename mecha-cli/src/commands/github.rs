//! `mecha github` — the GitHub board's collector
//! (`docs/GITHUB-DASHBOARD-DESIGN.md`). mecha never holds the GitHub token:
//! every call is `gh api`, through `gh`'s own login.
//!
//! The one model call is the quick summary (§5, step 4): after the
//! collection is written, each new or edited body goes to a quarantined
//! one-shot (`github::summary`) on this machine's model, a background seat
//! per call. Nothing the pass does can cost the board its tables, and
//! `--no-summaries` runs the collector without it.
//!
//! `collect` is what `scripts/mecha-github-collect.timer` runs. How often is
//! the timer's setting — once a day, the owner's ruling G10 — and never a
//! constant here.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use mecha_core::feature::Feature;
use mecha_core::github::{self, summary};

use crate::GlobalOpts;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Ask GitHub, through `gh api`, about the repositories
    /// ~/.mecha/github/repos.toml selects, and write
    /// ~/.mecha/github/github.db: the inventory, default-branch CI runs, open
    /// issues and outside pull requests, today's counts, and a log of every
    /// refusal and failure, which `mecha doctor` reads. Then summarise new
    /// and edited issue bodies on this machine's model, a few dozen a run.
    /// Exits non-zero when the collection was refused or failed anywhere —
    /// after writing what it found. A summary pass that was skipped or failed
    /// is logged for `mecha doctor` and leaves the exit alone: the collection
    /// it follows was written whole.
    Collect {
        #[arg(long)]
        json: bool,
        /// Collect without the summary pass: no model is asked, and the run's
        /// log says the pass did not run (never that it summarised nothing).
        #[arg(long)]
        no_summaries: bool,
    },
}

impl Cmd {
    /// Whether this invocation asks a model (`main`'s router snapshot).
    pub fn runs_a_model(&self) -> bool {
        match self {
            Cmd::Collect { no_summaries, .. } => !no_summaries,
        }
    }
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Collect { json, no_summaries } => collect(global, json, no_summaries).await,
    }
}

async fn collect(global: &GlobalOpts, json: bool, no_summaries: bool) -> Result<()> {
    crate::commands::features::require(Feature::Github)?;
    let home = mecha_core::work::mecha_home()?;
    let gh = github::Gh::from_env();
    let found = github::collect(&gh, &home, chrono::Utc::now());
    let db = github::db_path(&home);
    // Written before any model is asked: the pass cannot cost the board the
    // collection, however it ends.
    github::record(&db, &found)?;

    let pass = if no_summaries {
        None
    } else {
        let pass = summaries(global, &db, &found).await;
        // A pass that cannot be written is said and the run goes on: the
        // collection is already in, and the doctor reads the missing counts.
        match summary::record_pass(&db, found.started, &pass) {
            Ok(()) => Some(pass),
            Err(e) => {
                eprintln!("mecha: the summaries could not be written: {e:#}");
                None
            }
        }
    };

    if json {
        let out = serde_json::json!({
            "store": db,
            "started": found.started,
            "finished": found.finished,
            "selected": found.selected,
            "collected": found.collected(),
            "inventory": found.inventory.as_ref().map(Vec::len),
            "items": found.repos.iter().map(|r| r.items.len()).sum::<usize>(),
            "ci_runs": found.ci.len(),
            "problems": found.problems,
            "rate_before": found.rate_before,
            "rate_after": found.rate_after,
            // Counts and the reason it stopped, never a summary: the store is
            // where model prose about a stranger's text is kept, and only there.
            "summaries": pass.as_ref().map(|p| serde_json::json!({
                "tally": p.tally(),
                "skipped": p.skipped,
                "failed": p.failed.len(),
            })),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        match &found.inventory {
            Some(inv) => println!("inventory: {} repositories the login can see", inv.len()),
            None => println!("inventory: not listed this run; the last one is kept"),
        }
        println!(
            "watched: {} selected, {} read in full; {} open item(s), {} default-branch run(s)",
            found
                .selected
                .map_or_else(|| "unknown".to_string(), |n| n.to_string()),
            found.collected(),
            found.repos.iter().map(|r| r.items.len()).sum::<usize>(),
            found.ci.len()
        );
        if let (Some(b), Some(a)) = (found.rate_before, found.rate_after) {
            println!(
                "quota: REST {} → {} left, GraphQL {} → {} left",
                b.core_remaining, a.core_remaining, b.graphql_remaining, a.graphql_remaining
            );
        }
        for p in &found.problems {
            eprintln!(
                "{}: {}{}: {}",
                p.kind.as_str(),
                p.step.as_str(),
                p.repo
                    .as_deref()
                    .map(|r| format!(" {r}"))
                    .unwrap_or_default(),
                p.detail
            );
        }
        match &pass {
            None if no_summaries => println!("summaries: not made (--no-summaries)"),
            None => {}
            Some(p) => {
                let t = p.tally();
                println!(
                    "summaries: {} made, {} refused, {} failed, {} left for the next run",
                    t.summarised, t.refused, t.failed, t.deferred
                );
                if let Some(why) = &p.skipped {
                    eprintln!("summary_skipped: {why}");
                }
            }
        }
        println!("written to {}", db.display());
    }

    if found.problems.is_empty() {
        Ok(())
    } else {
        // Refused or failed: non-zero, so the unit reads failed too. What was
        // found is already written, and the log says which kind.
        crate::exit_with(1);
    }
}

/// The summary pass, from what this run's listing found to what it wrote.
///
/// **In this order, and each step can end it:** anything to ask; a hold on
/// the router, refused rather than waited for when a switch is pending (the
/// run is the side that yields, and a daily pass can wait a day); the
/// router observed *after* the hold, so the model resolved is the one the
/// hold protects (`follow::Follower::enter`'s order); a provider that is this
/// machine's; then the calls, a seat each. The hold is taken here and not
/// for the command's life, so a switch never waits behind eight minutes of
/// `gh`.
///
/// **The global config only.** The unit runs in the home directory, and a
/// project's `mecha.toml` must not choose which model reads strangers' text
/// from the owner's private repositories — the selection file's argument
/// (§2), one table over.
async fn summaries(global: &GlobalOpts, db: &Path, found: &github::Collected) -> summary::Pass {
    let (todo, empty) = match summary::pending(db, found) {
        Ok(p) => p,
        Err(e) => {
            return summary::Pass::skipped(
                format!("the store's summaries could not be read: {e:#}"),
                &[],
                Vec::new(),
            )
        }
    };
    if todo.is_empty() {
        return summary::Pass::idle(empty);
    }
    // What waits whatever model answers — a body another model refused is
    // not waiting on this one, and does not make a skip worth logging.
    let fresh: Vec<summary::Input> = todo
        .iter()
        .filter(|i| i.refused_by.is_none())
        .cloned()
        .collect();
    // Only refused bodies waiting, and no model to ask: nothing was missed.
    let skip = |why: String| {
        if fresh.is_empty() {
            summary::Pass::idle(empty.clone())
        } else {
            summary::Pass::skipped(why, &fresh, empty.clone())
        }
    };

    let cfg = match mecha_core::config::Config::load_global() {
        Ok(c) => c,
        Err(e) => return skip(format!("the global config could not be read: {e:#}")),
    };
    // Not "local": nothing to hold, and the check below refuses it.
    let held = match router_of(&cfg, global.provider.as_deref()) {
        None => None,
        Some(base) => {
            let holds = match mecha_core::hold::Holds::open_default() {
                Ok(h) => h,
                Err(e) => return skip(format!("the router's holds could not be opened: {e:#}")),
            };
            match holds.try_hold(&base, "mecha github collect") {
                Ok(Ok(h)) => Some(h),
                Ok(Err(switch)) => {
                    return skip(format!(
                        "the model was switching to {} (since {})",
                        switch.to,
                        switch.started_at.format("%H:%M:%SZ")
                    ))
                }
                Err(e) => return skip(format!("the router could not be held: {e:#}")),
            }
        }
    };
    // "Switch now" stops the pass at its next item, as Ctrl-C would.
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(h) = &held {
        let stop = Arc::clone(&stop);
        h.on_cancel(move || stop.store(true, Ordering::SeqCst));
    }
    let follows = global.model.is_none() && global.provider.is_none();
    for warning in mecha_core::provider::router::observe(&cfg, follows).await {
        tracing::warn!("{warning}");
    }
    let (provider_name, provider_cfg) = match cfg.provider(global.provider.as_deref()) {
        Ok(p) => p,
        Err(e) => return skip(format!("no model is configured: {e:#}")),
    };
    let provider = match mecha_core::provider::build(provider_cfg) {
        Ok(p) => p,
        Err(e) => {
            return skip(format!(
                "the provider {provider_name:?} could not be built: {e:#}"
            ))
        }
    };
    // This machine's model only: the bodies of private repositories are the
    // owner's, and the seats are llama-server's (`permit.rs`).
    if !provider.local() {
        return skip(format!(
            "the provider {provider_name:?} is not a model on this machine; summaries are made \
             only on one"
        ));
    }
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| provider.default_model().to_string());
    let permits = match crate::commands::tasks::permits() {
        Ok(p) => p,
        Err(e) => return skip(format!("the seat pool could not be opened: {e:#}")),
    };
    let asked = summary::for_model(&todo, &model);
    eprintln!(
        "summarising {} of {} new or edited item(s) with {model} ({provider_name})",
        asked.len().min(summary::PER_RUN),
        asked.len()
    );
    let pass = summary::run(
        provider.as_ref(),
        &model,
        &asked,
        empty,
        summary::Limits::standard(),
        &permits,
        &stop,
    )
    .await;
    drop(held);
    pass
}

/// The router `provider` (the default when `None`) is served from, when it
/// is a local server on this machine — `follow::hold_router`'s rule.
fn router_of(cfg: &mecha_core::config::Config, provider: Option<&str>) -> Option<String> {
    let name = provider.unwrap_or(&cfg.default_provider);
    cfg.providers
        .get(name)
        .filter(|p| p.kind == "local")
        .and_then(|p| p.base_url.as_deref())
        .filter(|u| mecha_core::provider::router::is_loopback(u))
        .map(mecha_core::provider::router::base)
}
