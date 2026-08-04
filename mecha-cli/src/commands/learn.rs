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

    /// Hold out this fraction of unprocessed reflections from the pass, so
    /// `mecha validate --unprocessed-only` has data the rules never saw.
    /// Deterministic (every k-th by id), because a measurement set that
    /// changes between runs measures nothing.
    #[arg(long, default_value_t = 0.0)]
    pub holdout: f64,

    /// Show what would run without calling a model or writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// Which reflection ids this pass leaves alone, given a holdout fraction.
///
/// Deterministic by construction: sort by id, then take every k-th. A random
/// sample would give `mecha validate` a different measurement set on every
/// pass, and a measurement set that moves measures nothing.
///
/// The stride floor of 2 is load-bearing. A fraction near 1 rounds to a stride
/// of 1, which would hold out *everything* and leave the pass with nothing to
/// learn from — a silently empty run that looks like it worked.
fn hold_out(ids: &[String], fraction: f64) -> std::collections::BTreeSet<String> {
    if fraction <= 0.0 {
        return Default::default();
    }
    let stride = (1.0 / fraction).round().max(2.0) as usize;
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort();
    sorted
        .into_iter()
        .enumerate()
        .filter(|(i, _)| (i + 1) % stride == 0)
        .map(|(_, id)| id.clone())
        .collect()
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    // Writer lock before reading the reflections this pass will consume —
    // a detached reflect landing mid-pass must wait, not interleave. Held
    // across the model call on purpose: the pass is a read-modify-write of
    // the rule set, and there is no smaller region that keeps it one.
    let _lock = if args.dry_run { None } else { Some(store.lock()?) };

    anyhow::ensure!(
        (0.0..1.0).contains(&args.holdout),
        "--holdout must be in [0, 1), not {}",
        args.holdout
    );

    // Group unprocessed reflections by domain.
    let mut by_domain: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for r in store.reflexions()? {
        if !r.is_processed {
            by_domain.entry(r.domain.clone()).or_default().push(r);
        }
    }

    if args.holdout > 0.0 {
        for (domain, rs) in by_domain.iter_mut() {
            let before = rs.len();
            let held = hold_out(&rs.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), args.holdout);
            rs.retain(|r| !held.contains(&r.id));
            if before != rs.len() {
                println!("{domain}: holding out {} of {before} reflection(s)", before - rs.len());
            }
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

#[cfg(test)]
mod tests {
    use super::hold_out;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("r{i:02}")).collect()
    }

    #[test]
    fn a_zero_fraction_holds_out_nothing() {
        assert!(hold_out(&ids(10), 0.0).is_empty());
    }

    #[test]
    fn a_fraction_takes_every_kth_by_id() {
        let held = hold_out(&ids(10), 0.5);
        assert_eq!(held.len(), 5);
        assert!(held.contains("r01") && held.contains("r09"));
        assert!(!held.contains("r00"));

        let held = hold_out(&ids(12), 0.25);
        assert_eq!(held.len(), 3);
        assert!(held.contains("r03") && held.contains("r07") && held.contains("r11"));
    }

    #[test]
    fn the_order_reflections_arrive_in_does_not_change_the_holdout() {
        // The store returns append order; validate must see the same set
        // whatever order a later pass happens to read them in.
        let mut shuffled = ids(9);
        shuffled.reverse();
        assert_eq!(hold_out(&ids(9), 0.5), hold_out(&shuffled, 0.5));
    }

    #[test]
    fn a_large_fraction_still_leaves_something_to_learn_from() {
        // Rounds to a stride of 1 without the floor, holding out every
        // reflection and turning the pass into a no-op that looks like a run.
        let held = hold_out(&ids(8), 0.9);
        assert!(held.len() < 8, "held out everything: {held:?}");
        assert_eq!(held.len(), 4);
    }
}
