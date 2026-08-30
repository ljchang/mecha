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
    budget_refuses, domain_rules_section, wrap_rules_block, LeapRun, Learner, LearningStore,
    Proposal, Trigger, MAX_ACTIVE_RULES_PER_DOMAIN, RULES_CHAR_BUDGET,
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
/// Stamp everything active, then let the ledger take back what it has
/// graded — one predicate for "measured", deliberately shared with retirement
/// (`clear_probation_when_measured`) rather than spelled a second time here.
/// The clear also *persists* through this pass's write, where retirement's
/// own call lands only when a domain has a conviction to record.
fn stamp_probation(
    rules: &mut [mecha_core::learning::Rule],
    tallies: &BTreeMap<String, mecha_core::learning::RuleTally>,
) {
    for r in rules.iter_mut().filter(|r| r.retired_at.is_none()) {
        r.probation = true;
    }
    mecha_core::learning::clear_probation_when_measured(rules, tallies);
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

    // A batch identical to one some proposal already argued — most likely a
    // gate rejection whose reflections rightly returned to the pool — is not
    // argued again until the pool changes. Without this, an unchanged pool
    // means a fresh near-identical proposal (and its probe cost) every night.
    // `--auto` needs the brake most: its `rejected_by_gate` arm leaves the
    // reflections unprocessed, and `learn-live.sh` runs per *session*, so an
    // unguarded identical batch re-pays a learner call plus a probe pair per
    // steer/denial on every session close until the pool changes.
    if args.propose || args.auto {
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
        let learned_before = store.learned_rules(domain)?;

        let Some(rules) = learner
            .learn(domain, &user_rules, &learned_before, reflexions, &tallies)
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
        let mut rules = mecha_core::learning::finalize_rules(
            rules,
            &learned_before,
            &ids,
            &chrono::Utc::now().to_rfc3339(),
        );

        // Retired rules stay in the file but never render, so they cost the
        // budget nothing.
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
            let candidate_block = wrap_rules_block(
                domain_rules_section(domain, &user_rules, &rules)
                    .into_iter()
                    .collect(),
            );
            // The before-arm of the counterfactual: the domains a probe
            // exercising this one would carry, so the two arms differ in the
            // candidate and nothing else.
            let current_block = store
                .rules_prompt_block_for(&mecha_core::learning::run_domains_including(domain))?;

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
                stamp_probation(&mut rules, &tallies);
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dispose, hold_out, stamp_probation};

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
            // Probationary from an earlier pass, since graded: the shared
            // predicate clears it, and this write persists the clear.
            rule(Some("measured-probationary"), true),
            rule(Some("unmeasured"), false),
            // Pre-identity: no id, so no tally can exist — unmeasured.
            rule(None, false),
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
                RuleTally {
                    observations: 4,
                    ..Default::default()
                },
            );
        }

        stamp_probation(&mut rules, &tallies);

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
