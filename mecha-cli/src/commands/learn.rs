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
    batches_by_region, budget_refuses, LeapRun, Learner, LearningStore, Proposal, Trigger,
    MAX_ACTIVE_RULES_PER_DOMAIN, RULES_CHAR_BUDGET,
};
use mecha_core::session::Session;
use std::collections::BTreeMap;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Only run when a domain has at least this many unprocessed reflections.
    #[arg(long, default_value_t = mecha_core::learning::LEARN_MIN_REFLECTIONS)]
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

    /// Measure the candidate and apply it when the measurement carries it —
    /// the ungated path, with the gate still in front.
    ///
    /// The middle mode this command was missing. Bare `learn` applies with no
    /// measurement at all; `--propose` measures and then waits for a human,
    /// which is how 27 reflections sat behind four unreviewed proposals. This
    /// runs the same probes and *disposes* of the result:
    ///
    /// - any probe worse than the deployed rules → refused, recorded
    /// - probes ran and none regressed → applied
    /// - **nothing gradeable in the batch → applied on probation**, marked
    ///   and retired sooner. The D1 ruling: holding here reproduces the stall,
    ///   since unmeasurable batches are the common case, and refusing gives up
    ///   the `writing` and `followup` half of the corpus for good.
    #[arg(long, conflicts_with = "propose")]
    pub auto: bool,

    /// Show what would run without calling a model or writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// What the gate decided about a candidate, and whether it lands marked.
#[derive(Debug, PartialEq)]
pub struct Disposition {
    /// Recorded on the proposal; also what `git log` in the store reads as.
    pub status: &'static str,
    /// Applied without the gate being able to grade it.
    pub probation: bool,
}

/// Turn the probe tally into a decision. Pure, so the three-way split is a
/// unit test rather than a claim about a long function.
///
/// The `measured == 0` arm is the one that matters and the one an intuition
/// gets wrong: it is not a rare near-miss, it is the **common** case, because
/// only steers and denials have a replayable intervention point at all.
/// Refusing there would silently discard every batch of edits and followups —
/// the majority of the corpus — and passing it as though it were clean would
/// lose the only distinction probation exists to record. So it applies, and
/// it applies *marked*.
///
/// Under `--propose` nothing is ever applied, and `measured == 0` still
/// reaches a human, who can read the evidence the gate could not grade.
pub fn dispose(auto: bool, regressed: u32, measured: u32) -> Disposition {
    // A candidate that makes any probe worse than what is deployed is refused
    // in both modes, before anything else is considered. Checked first so no
    // combination of the other two can talk past it.
    if regressed > 0 {
        return Disposition {
            status: "rejected_by_gate",
            probation: false,
        };
    }
    if !auto {
        return Disposition {
            status: "pending",
            probation: false,
        };
    }
    match measured {
        0 => Disposition {
            status: "auto_applied_probation",
            probation: true,
        },
        _ => Disposition {
            status: "auto_applied",
            probation: false,
        },
    }
}

/// Mark a probation pass's rules as applied-ungraded — but only the ones the
/// ledger has never graded.
///
/// A consolidation is a full replacement, so `rules` carries forward
/// long-standing rules with measured records; stamping those "applied
/// ungraded" would contradict `Rule::probation`'s own doc ("the gate ran and
/// *could not grade it*") and put a rule with dozens of clean observations on
/// the stricter retirement leash for an accident of this batch's contents.
///
/// **Stamping and releasing answer different questions, so they use
/// different predicates.** Stamping asks "was this rule born ungraded?" —
/// any verdict-bearing row in the ledger answers no, whatever the verdicts
/// were, so a rule with graded history keeps whatever probation state
/// `finalize_rules` carried for it. Releasing asks "has a probationary rule
/// earned its way off the leash?" — which its own convictions must not
/// answer (`release_probation_when_measured_clean`). An earlier version
/// shared one predicate for both, and once the release stopped keying on
/// bare coverage that stamped a *born-graded* rule whose only verdicts were
/// its convictions: threshold silently 3 → 2 and a `retired_reason` naming
/// a probation it never had — the mirror image of the leash bug.
///
/// The trailing release also *persists* through this pass's write, where
/// retirement's own call lands only when a domain has a conviction to
/// record.
///
/// **Only the batch's own region is stamped.** `finalize_region_rules`
/// carries every other region's active rules through untouched, and a
/// stamp is a verdict about what *this* gate could not grade — a `shell`
/// batch landing on probation must not shorten an unrelated standing
/// rule's leash from 3 to 2 (found on review; it was safe only while a pass
/// rewrote the whole domain). The release below still runs over the whole
/// set, because it is a function of the ledger and not of this batch.
fn stamp_probation(
    rules: &mut [mecha_core::learning::Rule],
    tallies: &BTreeMap<String, mecha_core::learning::RuleTally>,
    region: &mecha_core::situation::Situation,
) {
    for r in rules
        .iter_mut()
        .filter(|r| r.retired_at.is_none() && mecha_core::learning::rewritable_in(r, region))
    {
        let ever_graded =
            r.id.as_deref()
                .and_then(|id| tallies.get(id))
                .is_some_and(|t| t.graded > 0);
        if !ever_graded {
            r.probation = true;
        }
    }
    mecha_core::learning::release_probation_when_measured_clean(rules, tallies);
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
    let _lock = if args.dry_run {
        None
    } else {
        Some(store.lock()?)
    };

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
    let mut dropped_by_owner = 0usize;
    for r in store.reflexions()? {
        if r.is_processed {
            continue;
        }
        if claimed.contains(&r.id) {
            awaiting_review += 1;
            continue;
        }
        // The owner's own refusal outranks a provenance argument the same
        // way `learnable()` orders them — checked first and counted
        // separately, or a person dropping lessons they disagree with would
        // read back as "excluded by origin", which is the gate's doing
        // reported as though it were theirs.
        if r.dropped_at.is_some() {
            dropped_by_owner += 1;
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
    if dropped_by_owner > 0 {
        println!(
            "{dropped_by_owner} reflection(s) dropped by you — kept as evidence, never a \
             candidate again"
        );
    }

    if args.holdout > 0.0 {
        for (domain, rs) in by_domain.iter_mut() {
            let before = rs.len();
            let held = hold_out(
                &rs.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
                args.holdout,
            );
            rs.retain(|r| !held.contains(&r.id));
            if before != rs.len() {
                println!(
                    "{domain}: holding out {} of {before} reflection(s)",
                    before - rs.len()
                );
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
            for (region, batch) in batches_by_region(rs.clone()) {
                println!("  [{}]", region.describe());
                for r in batch {
                    println!("  · {}", r.reflexion_text);
                }
            }
        }
        return Ok(());
    }

    // The ledger, folded per rule, so the consolidation can drop what has
    // been measured harmful instead of guessing from the rule text. Read once
    // for every domain: it is a scan of one append-only file.
    let tallies = mecha_core::learning::rule_tallies(&store.validations()?);

    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    let provider = mecha_core::provider::build(provider_cfg)?;
    let model = global.model.clone().or_else(|| provider_cfg.model.clone());
    let learner = Learner::new(provider, model);
    eprintln!("learning with {} ({provider_name})", learner.model());

    // The gate replays against the recorded tool surface, which needs the
    // live registry for specs — same borrow `mecha validate` makes.
    let prepared = if args.propose || args.auto {
        Some(setup::prepare(&global.clone(), false).await?)
    } else {
        None
    };
    let sessions_dir = Session::default_dir()?;

    for (domain, reflexions) in &by_domain {
        let user_rules = store.user_rules(domain)?;
        // One learner call per situation batch, not per domain: the rules
        // it writes are scoped to the region the batch shares, and it is
        // shown the domain's other rules as context it may not rewrite
        // (`learning::batches_by_region`, `Learner::learn`).
        for (region, reflexions) in batches_by_region(reflexions.clone()) {
            let reflexions = &reflexions;
            // The floor is per batch as well as per domain: the learner call
            // moved inside the batch, and a domain floor alone let three
            // reflections on three tools pass `--min 3` as three
            // single-incident learner calls, each minting a scoped rule (and
            // under `--auto` a probe pair) from one event — the permissive
            // failure, found on review. A small region waits, unprocessed,
            // until its own pool reaches the floor; the standing batch
            // usually gets there first.
            if reflexions.len() < args.min {
                println!(
                    "{domain} [{}]: {} reflection(s), below --min {}; waiting for more in \
                     this situation",
                    region.describe(),
                    reflexions.len(),
                    args.min
                );
                continue;
            }
            // One pending proposal per domain. Each batch proposes a
            // whole-domain set from the same base, so two pending rows for a
            // domain are alternatives: accepting one moves the rules under
            // the other, and `accept`'s only way past that is `--force`, the
            // lossy path. The later batches wait, unprocessed, behind the
            // review (`--auto` resolves at birth and never parks here).
            if args.propose && !args.auto {
                let pending = store
                    .proposals()?
                    .iter()
                    .any(|p| p.domain == *domain && p.status == "pending");
                if pending {
                    println!(
                        "{domain} [{}]: a proposal for this domain is pending review; this \
                         batch waits behind it (`mecha proposals`)",
                        region.describe()
                    );
                    continue;
                }
            }
            // A batch identical to one some proposal already argued — most
            // likely a gate rejection whose reflections rightly returned to the
            // pool — is not argued again until the pool changes. Without this,
            // an unchanged pool means a fresh near-identical proposal (and its
            // probe cost) every night. `--auto` needs the brake most: its
            // `rejected_by_gate` arm leaves the reflections unprocessed, and
            // `learn-live.sh` runs per *session*, so an unguarded identical
            // batch re-pays a learner call plus a probe pair per steer/denial on
            // every session close until the pool changes.
            if (args.propose || args.auto) && already_argued(&proposals, domain, reflexions) {
                println!(
                    "{domain} [{}]: this exact batch of {} reflection(s) was already argued \
                 (see `mecha proposals`); waiting for new reflections",
                    region.describe(),
                    reflexions.len()
                );
                continue;
            }
            // Read per batch, not per domain: the previous batch of this domain
            // may have just written the set this one carries forward.
            let learned_before = store.learned_rules(domain)?;
            println!(
                "{domain} [{}]: {} reflection(s)",
                region.describe(),
                reflexions.len()
            );

            let Some(rules) = learner
                .learn(
                    domain,
                    &region,
                    &user_rules,
                    &learned_before,
                    reflexions,
                    &tallies,
                )
                .await?
            else {
                eprintln!("{domain}: the learner produced no usable rule set; nothing changed");
                continue;
            };

            // Identity before anything persists or is measured: surviving rules
            // keep their id and lineage, new ones are minted with this batch as
            // provenance, retired rules are carried through untouched. The gate
            // below measures exactly what acceptance would deploy.
            let ids: Vec<String> = reflexions.iter().map(|r| r.id.clone()).collect();
            let mut rules = mecha_core::learning::finalize_region_rules(
                rules,
                &learned_before,
                &region,
                &ids,
                &chrono::Utc::now().to_rfc3339(),
            );

            // Retired rules stay in the file but never render, so they cost the
            // budget nothing. Summed over the whole domain's active set, which
            // is now more than any one run carries — the same seam as the
            // count cap, and the warning stays on the store-wide figure
            // because that is the ceiling every run is under.
            let rendered: usize = rules
                .iter()
                .filter(|r| r.active())
                .map(|r| r.text.len() + 2)
                .sum();
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

            // ── the gate: measure the candidate, then dispose of it ──
            if args.propose || args.auto {
                let prepared = prepared.as_ref().expect("built under --propose/--auto");
                // Both arms rendered for the situation of the session each probe
                // replays — the domains a run exercising this one would carry,
                // filtered to what that run's registry matches — so the two
                // arms differ in the candidate and nothing else, and neither is
                // a block no run has.
                let arms = |run: &mecha_core::situation::Situation| -> Result<probe::Arms> {
                    let domains = mecha_core::learning::run_domains_including(domain);
                    let current = store.rules_carried_for(&domains, run)?.block;
                    let candidate = store
                        .rules_carried_with(&domains, run, Some((domain, &rules)))?
                        .block;
                    Ok((current, candidate))
                };

                let mut lines = Vec::new();
                let (mut improved, mut regressed, mut unchanged, mut inconclusive) =
                    (0u32, 0u32, 0u32, 0u32);
                let mut measured = 0u32;
                let mut skipped = 0u32;
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
                        &arms,
                    )
                    .await?
                    {
                        probe::ProbeResult::Skipped(why) => {
                            skipped += 1;
                            lines.push(format!("{} [{}]: skipped — {why}", r.id, r.trigger));
                        }
                        probe::ProbeResult::Verdicts(b, t) => {
                            // Counted only when the pair *graded* — `compare`
                            // returns `None` on an inconclusive arm. A pair that
                            // ran and concluded nothing is not evidence, and
                            // counting it let an all-inconclusive batch reach
                            // `dispose` as measured-clean and land unmarked on
                            // zero actual verdicts — the exact conflation ("not
                            // measured" read as "measured clean") probation
                            // exists to prevent.
                            match probe::compare(
                                &b,
                                &t,
                                &mut improved,
                                &mut regressed,
                                &mut unchanged,
                                &mut inconclusive,
                            ) {
                                Some(label) => {
                                    measured += 1;
                                    lines.push(format!("{} [{}]: {label}", r.id, r.trigger));
                                }
                                None => {
                                    lines.push(format!("{} [{}]: inconclusive", r.id, r.trigger));
                                }
                            }
                        }
                    }
                }
                lines.push(if measured == 0 && inconclusive > 0 {
                    // Ran and graded nothing is a different fact from had nothing
                    // to run — both land as probation, but the evidence must say
                    // which happened.
                    format!(
                        "{inconclusive} probe pair(s) ran and none graded (inconclusive); \
                     review by reading"
                    )
                } else if measured == 0 && skipped > 0 {
                    // Skipped is a third fact: the probe was possible and
                    // declined — no rule scoped to that run, or a candidate
                    // arm identical to the current one — which is not "had
                    // nothing to run".
                    format!("{skipped} probe(s) skipped and none ran; review by reading")
                } else if measured == 0 {
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
                // is refused in both modes — recorded with its evidence, though,
                // because a gate that leaves no trace teaches nobody anything.
                //
                // The three-way split under `--auto` is the D1 ruling made
                // mechanical. `measured == 0` is not a near-miss to be argued
                // about: it is the *common* case, because only steers and denials
                // have a replayable intervention point at all. Treating it as a
                // refusal would silently discard every batch of edits and
                // followups; treating it as a pass would lose the distinction
                // between "measured clean" and "not measured", which is the one
                // thing probation exists to remember.
                let Disposition { status, probation } = dispose(args.auto, regressed, measured);
                if probation {
                    stamp_probation(&mut rules, &tallies, &region);
                }
                let applied = status == "auto_applied" || status == "auto_applied_probation";
                // The proposal is written whichever way this went. Under `--auto`
                // nobody is going to read it as a decision — it is the audit
                // trail, and it is the only record of *why* a rule set landed or
                // did not, since the gate's probes never reach the validation
                // ledger.
                let proposal = Proposal {
                    id: Session::new_id(),
                    domain: domain.clone(),
                    status: status.into(),
                    reflexion_ids: ids.clone(),
                    rules_before: learned_before.clone(),
                    rules: rules.clone(),
                    evidence: evidence.clone(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    // Resolved at birth when the gate decided: nothing is waiting
                    // on anyone, and leaving it `None` would make every auto pass
                    // read as pending review to `mecha proposals` and to doctor.
                    resolved_at: applied
                        .then(|| chrono::Utc::now().to_rfc3339())
                        .or_else(|| {
                            (status == "rejected_by_gate").then(|| chrono::Utc::now().to_rfc3339())
                        }),
                    reason: None,
                    scope: Some(region.clone()),
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

                if applied {
                    let run = LeapRun {
                        id: proposal.id.clone(),
                        domain: domain.clone(),
                        reflexions_processed: ids.len() as u32,
                        rules_before: learned_before.len() as u32,
                        rules_after: rules.len() as u32,
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };
                    store.write_learned_rules(domain, &rules)?;
                    store.mark_reflexions_processed(&ids, &run.id)?;
                    store.append_run(&run)?;
                    for r in &rules {
                        println!(
                            "  - {}{}",
                            r.text,
                            if r.probation { "  [probation]" } else { "" }
                        );
                    }
                    if probation {
                        println!(
                            "  applied on probation: no reflection in this batch had a replayable \
                         intervention point, so the gate could not grade it. Retires at \
                         {} attributed regression(s) rather than {}.",
                            mecha_core::learning::PROBATION_RETIRE_AT,
                            mecha_core::learning::DEFAULT_RETIRE_AT,
                        );
                    }
                }

                store.commit(&format!(
                    "{}[{domain}]: {} rule(s) from {} reflection(s), {status}",
                    if applied { "learn" } else { "propose" },
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
    }
    Ok(())
}

/// Whether a proposal already argued exactly this batch of reflections.
fn already_argued(
    proposals: &[mecha_core::learning::Proposal],
    domain: &str,
    batch: &[mecha_core::learning::Reflexion],
) -> bool {
    let ids: std::collections::BTreeSet<&str> = batch.iter().map(|r| r.id.as_str()).collect();
    proposals.iter().any(|p| {
        p.domain == domain
            && p.reflexion_ids.len() == ids.len()
            && p.reflexion_ids.iter().all(|id| ids.contains(id.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::{already_argued, dispose, hold_out, stamp_probation};
    use std::collections::BTreeMap;

    /// A `shell` batch on probation stamps its own rules and leaves the
    /// standing rule it carried through on the ordinary leash. Fails on the
    /// unscoped stamp, which marked every ungraded rule in the domain.
    #[test]
    fn probation_stamps_only_the_batch_region() {
        use mecha_core::learning::Rule;
        use mecha_core::situation::Situation;
        let shell = Situation::of_run(&["shell".into()], None);
        let mut rules = vec![
            Rule {
                text: "Standing, carried through.".into(),
                id: Some("r-standing".into()),
                ..Default::default()
            },
            Rule {
                text: "New shell rule.".into(),
                id: Some("r-shell".into()),
                scope: Some(shell.clone()),
                ..Default::default()
            },
        ];
        stamp_probation(&mut rules, &BTreeMap::new(), &shell);
        assert!(!rules[0].probation, "not this batch's to stamp");
        assert!(rules[1].probation);
    }

    /// The brake compares one region's batch against the proposals, so a
    /// shell batch already argued waits while a new standing batch in the
    /// same domain still learns.
    #[test]
    fn the_argued_brake_is_per_batch_not_per_domain() {
        use mecha_core::learning::{Proposal, Reflexion};
        let refl = |id: &str| Reflexion {
            id: id.into(),
            domain: "behavior".into(),
            session_id: "s".into(),
            trigger: "denial".into(),
            context: String::new(),
            intervention: String::new(),
            reflexion_text: String::new(),
            error_type: None,
            confidence: None,
            is_processed: false,
            leap_run_id: None,
            created_at: String::new(),
            origin: mecha_core::learning::Origin::Clean,
            evidence: mecha_core::learning::Evidence::Full,
            edited_at: None,
            dropped_at: None,
            dropped_reason: None,
            situation: None,
        };
        let argued = Proposal {
            id: "p".into(),
            domain: "behavior".into(),
            status: "rejected_by_gate".into(),
            reflexion_ids: vec!["a".into(), "b".into()],
            rules_before: Vec::new(),
            rules: Vec::new(),
            evidence: String::new(),
            created_at: String::new(),
            resolved_at: None,
            reason: None,
            scope: None,
        };
        assert!(already_argued(
            std::slice::from_ref(&argued),
            "behavior",
            &[refl("a"), refl("b")]
        ));
        assert!(!already_argued(
            std::slice::from_ref(&argued),
            "behavior",
            &[refl("a")]
        ));
        assert!(!already_argued(
            std::slice::from_ref(&argued),
            "behavior",
            &[refl("c")]
        ));
        assert!(!already_argued(
            &[argued],
            "writing",
            &[refl("a"), refl("b")]
        ));
    }

    /// Probation marks what this pass could not grade — never a rule the
    /// ledger has already measured. A consolidation is a full replacement, so
    /// the candidate set carries forward long-standing rules with records; on
    /// the old behaviour every one of them was stamped "applied ungraded" and
    /// `rules list` reported measured-clean rules as probationary
    /// indefinitely, since the retirement-side clear only persists when a
    /// domain has a conviction to write.
    #[test]
    fn probation_lands_only_on_rules_the_ledger_never_graded() {
        use mecha_core::learning::{Rule, RuleTally};
        let rule = |id: Option<&str>, probation: bool| Rule {
            text: "r".into(),
            id: id.map(Into::into),
            probation,
            ..Default::default()
        };
        let mut rules = vec![
            rule(Some("measured"), false),
            // Probationary from an earlier pass, since graded clean: the
            // release clears it, and this write persists the clear.
            rule(Some("measured-probationary"), true),
            rule(Some("unmeasured"), false),
            // Pre-identity: no id, so no tally can exist — unmeasured.
            rule(None, false),
            // Born graded, and its only verdicts are its convictions. The
            // stamp must not leash it: stamping asks "born ungraded?", and
            // any graded row answers no — while the *release* predicate,
            // which its convictions fail, must not be the one deciding.
            // Fails on the shared-predicate version, where this rule came
            // out probationary with threshold 3 silently dropped to 2.
            rule(Some("convicted-born-graded"), false),
            // Probationary and convicted: the leash it is on must survive
            // both the stamp and the trailing release.
            rule(Some("convicted-probationary"), true),
        ];
        let retired = Rule {
            retired_at: Some("2026-08-01T00:00:00Z".into()),
            ..rule(Some("retired"), false)
        };
        rules.push(retired);

        let mut tallies = std::collections::BTreeMap::new();
        for id in ["measured", "measured-probationary"] {
            tallies.insert(
                id.to_string(),
                // Graded, not merely covered: release keys on verdicts, and
                // observations alone would read as four probes that ran and
                // graded nothing — which releases no leash.
                RuleTally {
                    observations: 4,
                    graded: 4,
                    ..Default::default()
                },
            );
        }
        for id in ["convicted-born-graded", "convicted-probationary"] {
            tallies.insert(
                id.to_string(),
                RuleTally {
                    observations: 1,
                    graded: 1,
                    regressed: 1,
                    attributed_regressions: 1,
                    ..Default::default()
                },
            );
        }

        stamp_probation(
            &mut rules,
            &tallies,
            &mecha_core::situation::Situation::default(),
        );

        let by_id = |want: Option<&str>| {
            rules
                .iter()
                .find(|r| r.id.as_deref() == want)
                .unwrap()
                .probation
        };
        assert!(!by_id(Some("measured")), "a graded rule is not ungraded");
        assert!(
            !by_id(Some("measured-probationary")),
            "the ledger takes back what it has graded"
        );
        assert!(
            by_id(Some("unmeasured")),
            "never graded — the leash applies"
        );
        assert!(by_id(None), "no id means no tally can ever exist: ungraded");
        assert!(
            !by_id(Some("retired")),
            "a retired rule rides in no prompt and wears no leash"
        );
        assert!(
            !by_id(Some("convicted-born-graded")),
            "a born-graded rule must not be leashed by its own convictions"
        );
        assert!(
            by_id(Some("convicted-probationary")),
            "a convicted probationary rule stays on the leash its convictions argue to"
        );
    }

    /// **The three-way split, which is the whole of `--auto`.**
    ///
    /// Fails on the old behaviour: there were two outcomes, and the only
    /// non-refusal was `pending` — a human. That is how 27 reflections came
    /// to sit behind four proposals nobody read.
    #[test]
    fn a_regression_is_refused_in_both_modes_and_nothing_else_can_talk_past_it() {
        for auto in [true, false] {
            let d = dispose(auto, 1, 9);
            assert_eq!(d.status, "rejected_by_gate", "auto={auto}");
            assert!(!d.probation);
            // Even with nothing else measured, a regression still refuses.
            assert_eq!(dispose(auto, 2, 0).status, "rejected_by_gate");
        }
    }

    #[test]
    fn measured_clean_applies_unmarked_under_auto_and_still_waits_under_propose() {
        let auto = dispose(true, 0, 6);
        assert_eq!(auto.status, "auto_applied");
        assert!(!auto.probation, "measured clean is not probation");

        let propose = dispose(false, 0, 6);
        assert_eq!(propose.status, "pending");
        assert!(!propose.probation);
    }

    /// The common case, not a corner: only steers and denials are replayable,
    /// so every batch of edits and followups grades zero. Refusing here would
    /// discard the majority of the corpus.
    #[test]
    fn an_ungradeable_batch_applies_marked_under_auto_and_reaches_a_human_under_propose() {
        let auto = dispose(true, 0, 0);
        assert_eq!(auto.status, "auto_applied_probation");
        assert!(
            auto.probation,
            "ungraded rules must be marked or the leash means nothing"
        );

        // `--propose` never applies, so an ungradeable batch is still a
        // person's to read rather than something marked and shipped.
        let propose = dispose(false, 0, 0);
        assert_eq!(propose.status, "pending");
        assert!(!propose.probation);
    }

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
