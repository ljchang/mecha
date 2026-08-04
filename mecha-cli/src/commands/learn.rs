//! `mecha learn` — the abstraction/consolidation pass.
//!
//! Unprocessed reflections per domain go in; a rewritten learned rule set
//! comes out, within the budget. The user's own rules are immutable context.
//! Every pass appends a `LeapRun` audit record and commits the store, so
//! `git log` in `~/.mecha/learning` reads as the system's learning history
//! and `git revert` undoes a pass that made things worse.

use crate::GlobalOpts;
use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::learning::{Learner, LearningStore, LeapRun, RULES_CHAR_BUDGET};
use mecha_core::session::Session;
use std::collections::BTreeMap;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Only run when a domain has at least this many unprocessed reflections.
    #[arg(long, default_value_t = 3)]
    pub min: usize,

    /// Show what would run without calling a model or writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;

    // Group unprocessed reflections by domain.
    let mut by_domain: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in store.reflexions()? {
        if !r.is_processed {
            by_domain.entry(r.domain.clone()).or_default().push(r);
        }
    }
    by_domain.retain(|domain, rs| {
        if rs.len() < args.min {
            println!(
                "{domain}: {} unprocessed reflection(s), below --min {}; skipping",
                rs.len(),
                args.min
            );
            false
        } else {
            true
        }
    });

    if by_domain.is_empty() {
        println!("nothing to learn from yet");
        return Ok(());
    }

    if args.dry_run {
        for (domain, rs) in &by_domain {
            let learned = store.learned_rules(domain)?;
            println!(
                "{domain}: would absorb {} reflection(s) into {} existing learned rule(s)",
                rs.len(),
                learned.len()
            );
            for r in rs {
                println!("  · {}", r.reflexion_text);
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    let learner = Learner::new(provider, model);
    eprintln!("learning with {} ({provider_name})", learner.model());

    for (domain, reflexions) in &by_domain {
        let user_rules = store.user_rules(domain)?;
        let learned_before = store.learned_rules(domain)?;

        let Some(rules) = learner.learn(domain, &user_rules, &learned_before, reflexions).await?
        else {
            eprintln!("{domain}: the learner produced no usable rule set; nothing changed");
            continue;
        };

        let rendered: usize = rules.iter().map(|r| r.text.len() + 2).sum();
        if rendered > RULES_CHAR_BUDGET {
            eprintln!(
                "{domain}: warning — the new rule set renders to {rendered} chars, over the \
                 {RULES_CHAR_BUDGET} budget; kept, but the next pass should consolidate harder"
            );
        }

        let run = LeapRun {
            id: Session::new_id(),
            domain: domain.clone(),
            reflexions_processed: reflexions.len() as u32,
            rules_before: learned_before.len() as u32,
            rules_after: rules.len() as u32,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        store.write_learned_rules(domain, &rules)?;
        let ids: Vec<String> = reflexions.iter().map(|r| r.id.clone()).collect();
        store.mark_reflexions_processed(&ids, &run.id)?;
        store.append_run(&run)?;

        println!(
            "{domain}: {} reflection(s) → {} rule(s) (was {})",
            reflexions.len(),
            run.rules_after,
            run.rules_before
        );
        for r in &rules {
            println!("  - {}", r.text);
        }

        store.commit(&format!(
            "learn[{domain}]: {} reflection(s), {} → {} rule(s)",
            reflexions.len(),
            run.rules_before,
            run.rules_after
        ));
    }
    Ok(())
}
