//! `mecha learn` — the abstraction/consolidation pass.
//!
//! Unprocessed reflections per domain go in; a rewritten learned rule set
//! comes out, within the budget. The user's own rules are immutable context.
//! Every pass appends a `LeapRun` audit record and commits the store, so
//! `git log` in `~/.mecha/learning` reads as the system's learning history
//! and `git revert` undoes a pass that made things worse.
//!
//! `--propose` is the hyperagent gate: instead of writing `learned.toml`,
//! the pass measures its candidate by counterfactual replay (candidate vs
//! the currently deployed rules, on the very interventions it learned from)
//! and stages the result as a proposal for `mecha proposals` to review. A
//! candidate that *regresses* a probe is rejected by the gate before any
//! human sees it. Unattended learning — the nightly timer — should always
//! propose; direct `mecha learn` at a terminal remains apply-with-git-undo.

use crate::{probe, setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::config::Config;
use mecha_core::learning::{
    budget_refuses, domain_rules_section, wrap_rules_block, Learner, LearningStore, LeapRun,
    Proposal, Trigger, MAX_ACTIVE_RULES_PER_DOMAIN, RULES_CHAR_BUDGET,
};
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

    /// Stage the result as a proposal instead of writing the live rules.
    /// The candidate is gated by counterfactual replay first; review with
    /// `mecha proposals`.
    #[arg(long)]
    pub propose: bool,

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

    // Reflections claimed by a pending proposal are spoken for: consuming
    // them again would either duplicate the proposal nightly until someone
    // reviews it, or double-count them into the live rules on a direct pass.
    let proposals = store.proposals()?;
    let claimed: std::collections::BTreeSet<String> = proposals
        .iter()
        .filter(|p| p.status == "pending")
        .flat_map(|p| p.reflexion_ids.iter().cloned())
        .collect();

    // Group unprocessed reflections by domain.
    let mut by_domain: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut awaiting_review = 0usize;
    let mut excluded_by_origin = 0usize;
    for r in store.reflexions()? {
        if r.is_processed {
            continue;
        }
        if claimed.contains(&r.id) {
            awaiting_review += 1;
            continue;
        }
        // Structural, before any prompt is built: a lesson drawn while
        // third-party content sat in context must never become a rule that
        // rides in every future run's system prompt. Excluded here rather
        // than scored inside the consolidation — no amount of confidence
        // promotes untrusted evidence.
        if !r.learnable() {
            excluded_by_origin += 1;
            continue;
        }
        by_domain.entry(r.domain.clone()).or_default().push(r);
    }
    if awaiting_review > 0 {
        println!(
            "{awaiting_review} reflection(s) are claimed by pending proposal(s) — \
             review with `mecha proposals`"
        );
    }
    if excluded_by_origin > 0 {
        println!(
            "{excluded_by_origin} reflection(s) excluded by origin — evidence from \
             untrusted or non-interactive sessions stays in the archive, never in rules"
        );
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

    // A batch identical to one some proposal already argued — most likely a
    // gate rejection whose reflections rightly returned to the pool — is not
    // argued again until the pool changes. Without this, an unchanged pool
    // means a fresh near-identical proposal (and its probe cost) every night.
    if args.propose {
        by_domain.retain(|domain, rs| {
            let batch: std::collections::BTreeSet<&str> =
                rs.iter().map(|r| r.id.as_str()).collect();
            let argued = proposals.iter().any(|p| {
                p.domain == *domain
                    && p.reflexion_ids.len() == batch.len()
                    && p.reflexion_ids.iter().all(|id| batch.contains(id.as_str()))
            });
            if argued {
                println!(
                    "{domain}: this exact batch of {} reflection(s) was already argued \
                     (see `mecha proposals`); waiting for new reflections",
                    batch.len()
                );
            }
            !argued
        });
    }

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

    // The gate replays against the recorded tool surface, which needs the
    // live registry for specs — same borrow `mecha validate` makes.
    let prepared =
        if args.propose { Some(setup::prepare(&global.clone(), false).await?) } else { None };
    let sessions_dir = Session::default_dir()?;

    for (domain, reflexions) in &by_domain {
        let user_rules = store.user_rules(domain)?;
        let learned_before = store.learned_rules(domain)?;

        let Some(rules) = learner.learn(domain, &user_rules, &learned_before, reflexions).await?
        else {
            eprintln!("{domain}: the learner produced no usable rule set; nothing changed");
            continue;
        };

        // Identity before anything persists or is measured: surviving rules
        // keep their id and lineage, new ones are minted with this batch as
        // provenance, retired rules are carried through untouched. The gate
        // below measures exactly what acceptance would deploy.
        let ids: Vec<String> = reflexions.iter().map(|r| r.id.clone()).collect();
        let rules = mecha_core::learning::finalize_rules(
            rules,
            &learned_before,
            &ids,
            &chrono::Utc::now().to_rfc3339(),
        );

        // Retired rules stay in the file but never render, so they cost the
        // budget nothing.
        let rendered: usize =
            rules.iter().filter(|r| r.active()).map(|r| r.text.len() + 2).sum();
        if rendered > RULES_CHAR_BUDGET {
            eprintln!(
                "{domain}: warning — the new rule set renders to {rendered} chars, over the \
                 {RULES_CHAR_BUDGET} budget; kept, but the next pass should consolidate harder"
            );
        }

        // The count cap is a refusal, not a warning: the always-loaded block
        // may never grow past it, however the learner argued. The frames
        // already say fifteen; this is the check that does not depend on the
        // model listening. The batch stays unprocessed, so the reflections
        // return to the next pass — which must merge or retire first.
        let active_before = learned_before.iter().filter(|r| r.active()).count();
        let active_after = rules.iter().filter(|r| r.active()).count();
        if budget_refuses(active_before, active_after) {
            eprintln!(
                "{domain}: refused — {active_after} active rules is over the cap of \
                 {MAX_ACTIVE_RULES_PER_DOMAIN} and no smaller than the current \
                 {active_before}. Nothing changed; consolidate or retire before adding."
            );
            continue;
        }

        // ── the gate: measure the candidate, stage it, never apply it ──
        if args.propose {
            let prepared = prepared.as_ref().expect("built under --propose");
            let candidate_block = wrap_rules_block(
                domain_rules_section(domain, &user_rules, &rules).into_iter().collect(),
            );
            let current_block = store.rules_prompt_block()?;

            let mut lines = Vec::new();
            let (mut improved, mut regressed, mut unchanged, mut inconclusive) =
                (0u32, 0u32, 0u32, 0u32);
            let mut measured = 0u32;
            // An allowlist, not an exclusion: only steers and denials have a
            // replayable intervention point. Followups keep the judge path in
            // `mecha validate`; edits (outbox) have no transcript at all.
            for r in reflexions.iter().filter(|r| {
                r.trigger == Trigger::Steer.as_str() || r.trigger == Trigger::Denial.as_str()
            }) {
                match probe::probe_reflection(
                    prepared,
                    provider_cfg,
                    learner.model(),
                    &sessions_dir,
                    r,
                    current_block.as_deref(),
                    candidate_block.as_deref(),
                )
                .await?
                {
                    probe::ProbeResult::Skipped(why) => {
                        lines.push(format!("{} [{}]: skipped — {why}", r.id, r.trigger));
                    }
                    probe::ProbeResult::Verdicts(b, t) => {
                        measured += 1;
                        let label = probe::compare(
                            &b,
                            &t,
                            &mut improved,
                            &mut regressed,
                            &mut unchanged,
                            &mut inconclusive,
                        )
                        .unwrap_or("inconclusive");
                        lines.push(format!("{} [{}]: {label}", r.id, r.trigger));
                    }
                }
            }
            lines.push(if measured == 0 {
                "no trace-gradeable reflections in this batch; review by reading".into()
            } else {
                format!(
                    "candidate vs current rules, replayed on the batch's own interventions: \
                     {improved} improved, {regressed} regressed, {unchanged} unchanged, \
                     {inconclusive} inconclusive"
                )
            });
            let evidence = lines.join("\n");

            // A candidate that makes any probe worse than what is deployed
            // never reaches a human — recorded with its evidence, though,
            // because a gate that leaves no trace teaches nobody anything.
            let status = if regressed > 0 { "rejected_by_gate" } else { "pending" };
            let proposal = Proposal {
                id: Session::new_id(),
                domain: domain.clone(),
                status: status.into(),
                reflexion_ids: ids,
                rules_before: learned_before.clone(),
                rules: rules.clone(),
                evidence: evidence.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                resolved_at: None,
                reason: None,
            };
            store.write_proposal(&proposal)?;
            println!(
                "{domain}: proposal {} [{status}] — {} rule(s) from {} reflection(s)",
                proposal.id,
                proposal.rules.len(),
                proposal.reflexion_ids.len()
            );
            println!("{evidence}");
            if status == "pending" {
                println!("review with `mecha proposals show {}`", proposal.id);
            }
            store.commit(&format!(
                "propose[{domain}]: {} rule(s) from {} reflection(s), {status}",
                proposal.rules.len(),
                proposal.reflexion_ids.len()
            ));
            continue;
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
