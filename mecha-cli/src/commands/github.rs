//! `mecha github` — the GitHub board's collector
//! (`docs/GITHUB-DASHBOARD-DESIGN.md`). No model runs anywhere in this
//! command, and mecha never holds the GitHub token: every call is `gh api`,
//! through `gh`'s own login.
//!
//! `collect` is what `scripts/mecha-github-collect.timer` runs. How often is
//! the timer's setting — once a day, the owner's ruling G10 — and never a
//! constant here.

use anyhow::Result;
use mecha_core::feature::Feature;
use mecha_core::github;

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
    /// refusal and failure, which `mecha doctor` reads. Exits non-zero when
    /// anything was refused or failed — after writing what it found.
    Collect {
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(_global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Collect { json } => collect(json),
    }
}

fn collect(json: bool) -> Result<()> {
    crate::commands::features::require(Feature::Github)?;
    let home = mecha_core::work::mecha_home()?;
    let gh = github::Gh::from_env();
    let found = github::collect(&gh, &home, chrono::Utc::now());
    let db = github::db_path(&home);
    github::record(&db, &found)?;

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
