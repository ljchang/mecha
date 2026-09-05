//! `mecha rules` — the lifecycle a rule enters *after* acceptance.
//!
//! Additions have a gate (`mecha learn --propose` → `mecha proposals`);
//! this is tenure. `list` folds the validation ledger into per-rule tallies
//! and surfaces the pressure — attributed regressions, never-validated
//! rules, age. `retire` and `restore` are the human acting directly, the
//! apply-with-git-undo path, same standing as a direct `mecha learn`.
//! `propose-retirements` is the unattended path: a deterministic scan of the
//! ledger — no model anywhere — that stages an `enabled = false` +
//! `retired_*` diff through the same proposal gate every other rule change
//! passes. Retirement is a flag, never a deletion: the rule stays in the
//! file as evidence, the learner is told it was measured harmful, and
//! `restore` can undo what erasure could not.
//!
//! The threshold is deliberately conservative (default 3 attributed
//! regressions) and counts only *attributed* regressions — bisection
//! verdicts, not block-level context — because the library-drift result
//! cuts both ways: unpruned stores go negative, and over-eager retirement
//! measurably hurt too. No decay, no TTL, no usage-based eviction: low
//! usage is a review signal, only measured harm argues for retirement, and
//! a human accepts the argument.

use anyhow::{bail, Result};
use mecha_core::learning::{
    judge_convicted, retire_threshold_for, rule_tallies, LeapRun, LearningStore, Proposal, Rule,
    RuleTally, ValidationRecord, Verdict,
};
use mecha_core::session::Session;
use std::collections::BTreeMap;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Every rule with its ledger tallies and staleness (default).
    List {
        /// Machine-readable, for `/learning`.
        #[arg(long)]
        json: bool,
    },
    /// Retire a rule by id (or unique prefix): kept in the file as evidence,
    /// never rendered into a prompt again.
    Retire {
        id: String,
        /// Why — recorded on the rule and shown to the learner so the same
        /// lesson does not come back under new wording.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Un-retire a rule by id (or unique prefix).
    Restore { id: String },
    /// One rule in full: its text, domain, state and ledger tally. What the
    /// TUI's `Enter` on the Rules pane runs — a rule is the one record here
    /// that rides in every future prompt, so it is the one most worth
    /// reading in full.
    Show { id: String },
    /// Scan the ledger and stage retirement proposals for rules the
    /// bisection keeps convicting. Deterministic; review with `mecha
    /// proposals`.
    ProposeRetirements {
        /// Apply the retirements directly instead of staging a proposal.
        ///
        /// Safe to automate in a way that promotion is not: this scan is a
        /// deterministic fold over the validation ledger with no model in it,
        /// it only ever *disables* rules, and a retired rule stays in the file
        /// as evidence. It is also the precondition for ungated learning —
        /// promotion without a working NoGo path is a ratchet.
        #[arg(long)]
        apply: bool,
        /// Attributed regressions required before a rule is proposed for
        /// retirement.
        #[arg(long, default_value_t = mecha_core::learning::DEFAULT_RETIRE_AT)]
        min_attributed: u32,
    },
}

pub async fn execute(args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    match args.cmd.unwrap_or(Cmd::List { json: false }) {
        Cmd::List { json } => list(&store, json),
        Cmd::Retire { id, reason } => retire(&store, &id, reason),
        Cmd::Restore { id } => restore(&store, &id),
        Cmd::Show { id } => show(&store, &id),
        Cmd::ProposeRetirements {
            min_attributed,
            apply,
        } => propose(&store, min_attributed, apply),
    }
}

fn list(store: &LearningStore, as_json: bool) -> Result<()> {
    let tallies = rule_tallies(&store.validations()?);
    if as_json {
        let mut out = Vec::new();
        for domain in store.domains() {
            // User rules ride in the same prompt and are **not on trial** —
            // they are the owner's and are never tallied or retired. Listed
            // anyway, and flagged, because a surface that shows only the
            // learned half misdescribes what a run actually carries.
            for (r, mine) in store
                .user_rules(&domain)?
                .iter()
                .map(|r| (r, true))
                .chain(store.learned_rules(&domain)?.iter().map(|r| (r, false)))
            {
                let tally = r.id.as_deref().and_then(|id| tallies.get(id));
                out.push(serde_json::json!({
                    "id": r.id,
                    "domain": domain,
                    "title": r.text,
                    "user": mine,
                    "active": r.active(),
                    "retired": r.retired_at.is_some(),
                    "retired_reason": r.retired_reason,
                    // Applied without the gate being able to grade it. The
                    // web roster shows it for the same reason the terminal
                    // does: "measured clean" and "not measured" are different
                    // states and only one of them earned its place.
                    "probation": r.probation,
                    // Where it loads: `null` is a rule from before scoping
                    // (everywhere), a string is `Situation::describe`.
                    "scope": r.scope.as_ref().map(|s| s.describe()),
                    // Where the evidence was seen to hold, and whether a
                    // scan ever narrowed it — see `Rule::support`.
                    "support": r.support.iter().map(|s| s.describe()).collect::<Vec<_>>(),
                    "narrowed_at": r.narrowed_at,
                    "narrowed_reason": r.narrowed_reason,
                    "observations": tally.map(|t| t.observations),
                    // Beside observations, because they answer different
                    // questions and the gap between them is the roster's
                    // only way to explain a covered rule still on probation.
                    "graded": tally.map(|t| t.graded),
                    "attributed_regressions": tally.map(|t| t.attributed_regressions),
                    // Per exercised sub-region; rows with no region are
                    // under `unknown`. See `ValidationRecord::region`.
                    "regions": tally.map(|t| {
                        t.regions
                            .values()
                            .map(|(region, c)| {
                                serde_json::json!({
                                    "region": region.describe(),
                                    "graded": c.graded,
                                    "improved": c.improved,
                                    "regressed": c.regressed,
                                    "attributed_regressions": c.attributed_regressions,
                                })
                            })
                            .collect::<Vec<_>>()
                    }),
                    "unknown_region_graded": tally.map(|t| t.unknown_region.graded),
                    "created_at": r.created_at,
                }));
            }
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let mut any = false;
    for domain in store.domains() {
        let user = store.user_rules(&domain)?;
        let learned = store.learned_rules(&domain)?;
        if user.is_empty() && learned.is_empty() {
            continue;
        }
        any = true;
        println!("## {domain}");
        if !user.is_empty() {
            println!("  {} user rule(s) — immutable, never tallied", user.len());
        }
        for r in &learned {
            println!("  {}", describe(r, &tallies));
        }
    }
    if !any {
        println!("no rules yet — `mecha learn` creates them");
    }
    Ok(())
}

fn describe(r: &Rule, tallies: &BTreeMap<String, RuleTally>) -> String {
    let id =
        r.id.as_deref()
            .unwrap_or("(no id — predates identity; next learn pass mints one)");
    let state = if r.retired_at.is_some() {
        format!(
            "RETIRED {}{}",
            r.retired_at.as_deref().unwrap_or_default(),
            r.retired_reason
                .as_deref()
                .map(|w| format!(" — {w}"))
                .unwrap_or_default()
        )
    } else if !r.enabled {
        "disabled".into()
    } else if r.probation {
        // Visible, because a rule that went live ungraded is a different
        // thing to read than one that was measured clean, and the roster is
        // where the owner would notice the difference.
        format!(
            "active (probation — applied ungraded, retires at {})",
            mecha_core::learning::PROBATION_RETIRE_AT
        )
    } else {
        "active".into()
    };
    // Where the rule loads. Unscoped and standing both load everywhere, and
    // are printed apart because they are different facts about the evidence
    // — see `Rule::scope`.
    let scope = match &r.scope {
        None => "unscoped (predates scoping; loads everywhere)".to_string(),
        Some(s) if s.is_standing() => "standing (loads everywhere)".to_string(),
        Some(s) => format!("loads with {}", s.describe()),
    };
    // Where it was seen to hold, when that is more than where it loads —
    // a widened rule names each sub-region it widened over; a narrowed one
    // says what it shed.
    let support = if r.support.len() > 1
        || r.support
            .first()
            .is_some_and(|s| Some(s) != r.scope.as_ref())
    {
        format!(
            " · seen in {}",
            r.support
                .iter()
                .map(|s| s.describe())
                .collect::<Vec<_>>()
                .join("; ")
        )
    } else {
        String::new()
    };
    let narrowed = match (&r.narrowed_at, &r.narrowed_reason) {
        (Some(at), Some(why)) => format!(" · narrowed {at} — {why}"),
        (Some(at), None) => format!(" · narrowed {at}"),
        _ => String::new(),
    };
    // Three states, no two rendering alike: graded, ran-but-graded-nothing,
    // and never probed. Collapsing the middle into the first printed
    // "0 improved, 0 regressed" for inconclusive-only coverage — a clean
    // bill of health from rows that graded nothing, and the reason a
    // covered rule can still be on probation.
    let measured = match r.id.as_deref().and_then(|id| tallies.get(id)) {
        Some(t) if t.graded > 0 => format!(
            "{} probe(s), {} graded: {} improved, {} regressed, {} attributed to this rule; last {}{}",
            t.observations,
            t.graded,
            t.improved,
            t.regressed,
            t.attributed_regressions,
            t.last_validated.as_deref().unwrap_or("never"),
            regions_line(t)
        ),
        Some(t) if t.observations > 0 => format!(
            "{} probe(s) ran, none graded — inconclusive coverage measures nothing; last {}",
            t.observations,
            t.last_validated.as_deref().unwrap_or("never")
        ),
        _ => "never validated".into(),
    };
    format!(
        "[{state}] {}\n      id {id} · created {} · {scope}{support}{narrowed} · {measured}",
        r.text,
        r.created_at.as_deref().unwrap_or("unknown"),
    )
}

/// The graded rows split by the sub-region they exercised, after the
/// totals: `shell 4 graded (1 attributed) · fs_read 2 graded`, with rows
/// that named no region counted as such — unknown is not a region and is
/// never folded into one.
fn regions_line(t: &RuleTally) -> String {
    let mut parts: Vec<String> = t
        .regions
        .values()
        .filter(|(_, c)| c.graded > 0)
        .map(|(region, c)| {
            format!(
                "{} {} graded{}",
                region.describe(),
                c.graded,
                match c.attributed_regressions {
                    0 => String::new(),
                    n => format!(" ({n} attributed)"),
                }
            )
        })
        .collect();
    if t.unknown_region.graded > 0 {
        parts.push(format!(
            "no region recorded {} graded{}",
            t.unknown_region.graded,
            match t.unknown_region.attributed_regressions {
                0 => String::new(),
                n => format!(" ({n} attributed)"),
            }
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\n      by region: {}", parts.join(" · "))
    }
}

/// Find one learned rule by id or unique prefix, returning its domain.
/// Ambiguity is an error rather than a guess, same as proposal lookup.
fn find_rule(store: &LearningStore, id: &str) -> Result<(String, Vec<Rule>, usize)> {
    // `rid.starts_with("")` is true for every rule that has an id, so an
    // empty needle — a TUI row whose `Rule::id` was `None`, serialised to
    // `null` and read back as `""` — would match every learned rule in
    // every domain instead of none. `mecha rules retire ""` is reachable
    // from the command line too.
    anyhow::ensure!(!id.is_empty(), "no rule id given");
    let mut hits: Vec<(String, Vec<Rule>, usize)> = Vec::new();
    for domain in store.domains() {
        let rules = store.learned_rules(&domain)?;
        for (i, r) in rules.iter().enumerate() {
            if r.id.as_deref().is_some_and(|rid| rid.starts_with(id)) {
                hits.push((domain.clone(), rules.clone(), i));
            }
        }
    }
    match hits.len() {
        0 => bail!("no learned rule matching `{id}` — `mecha rules` lists ids"),
        1 => Ok(hits.remove(0)),
        n => bail!("`{id}` matches {n} rules; give more of the id"),
    }
}

fn retire(store: &LearningStore, id: &str, reason: Option<String>) -> Result<()> {
    let _lock = store.lock()?;
    let (domain, mut rules, i) = find_rule(store, id)?;
    if rules[i].retired_at.is_some() {
        bail!(
            "rule {} is already retired",
            rules[i].id.as_deref().unwrap_or(id)
        );
    }
    rules[i].enabled = false;
    rules[i].retired_at = Some(chrono::Utc::now().to_rfc3339());
    rules[i].retired_reason = Some(reason.unwrap_or_else(|| "retired by hand".into()));
    store.write_learned_rules(&domain, &rules)?;
    store.commit(&format!(
        "retire[{domain}]: {}",
        rules[i].id.as_deref().unwrap_or(id)
    ));
    println!("retired from `{domain}`: {}", rules[i].text);
    Ok(())
}

fn restore(store: &LearningStore, id: &str) -> Result<()> {
    let _lock = store.lock()?;
    let (domain, mut rules, i) = find_rule(store, id)?;
    if rules[i].retired_at.is_none() {
        bail!(
            "rule {} is not retired",
            rules[i].id.as_deref().unwrap_or(id)
        );
    }
    rules[i].enabled = true;
    rules[i].retired_at = None;
    rules[i].retired_reason = None;
    store.write_learned_rules(&domain, &rules)?;
    store.commit(&format!(
        "restore[{domain}]: {}",
        rules[i].id.as_deref().unwrap_or(id)
    ));
    println!("restored to `{domain}`: {}", rules[i].text);
    Ok(())
}

fn show(store: &LearningStore, id: &str) -> Result<()> {
    let tallies = rule_tallies(&store.validations()?);
    let (domain, rules, i) = find_rule(store, id)?;
    println!("## {domain}\n{}", describe(&rules[i], &tallies));
    Ok(())
}

fn propose(store: &LearningStore, min_attributed: u32, apply: bool) -> Result<()> {
    let _lock = store.lock()?;
    let records = store.validations()?;
    let tallies = rule_tallies(&records);
    let proposals = store.proposals()?;
    let mut staged = 0u32;

    for domain in store.domains() {
        let mut before = store.learned_rules(&domain)?;
        // Probation says "born ungraded", which stops being true once the
        // ledger grades the rule beyond its convictions. Released before the
        // threshold is chosen, or a rule with a real clean record would still
        // answer to the short leash — the stale-stamp failure, one store
        // over. The release deliberately does not key on bare coverage: an
        // attributed regression always arrives inside an observation, so
        // that release stripped the leash on the very rows that convict and
        // made PROBATION_RETIRE_AT unreachable from this scan.
        mecha_core::learning::release_probation_when_measured_clean(&mut before, &tallies);
        let before = before;
        // Per rule: stands, narrows, or retires. Narrowing is retirement's
        // gentler sibling (`judge_convicted`): a rule convicted in one of
        // the sub-regions it was seen in and clean in the others sheds the
        // failing one and keeps loading where it held.
        let verdicts: Vec<(&Rule, Verdict)> = before
            .iter()
            .filter(|r| r.active())
            .filter_map(|r| {
                // Per-rule, not per-pass: a probationary rule went live
                // ungraded and answers to a shorter leash.
                let threshold = retire_threshold_for(r, min_attributed);
                let tally = tallies.get(r.id.as_deref()?)?;
                match judge_convicted(r, tally, threshold) {
                    Verdict::Stands => None,
                    v => Some((r, v)),
                }
            })
            .collect();
        if verdicts.is_empty() {
            continue;
        }
        let convicted: Vec<&Rule> = verdicts.iter().map(|(r, _)| *r).collect();
        // A pending proposal already retiring or narrowing these exact
        // rules is not re-staged — the nightly must not spam the queue
        // while a human hasn't looked yet. Collected rather than tested,
        // because the apply path below owes each of these a resolution.
        let convicted_ids: Vec<&str> = convicted.iter().filter_map(|r| r.id.as_deref()).collect();
        let pending_twins: Vec<mecha_core::learning::Proposal> = proposals
            .iter()
            .filter(|p| {
                p.status == "pending"
                    && p.domain == domain
                    && convicted_ids.iter().all(|id| {
                        p.rules.iter().any(|r| {
                            r.id.as_deref() == Some(*id)
                                && (r.retired_at.is_some() || r.narrowed_at.is_some())
                        })
                    })
            })
            .cloned()
            .collect();
        if !pending_twins.is_empty() && !apply {
            println!("{domain}: retirement already pending — review with `mecha proposals`");
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut evidence_lines = Vec::new();
        let mut narrowed_count = 0u32;
        let rules: Vec<Rule> = before
            .iter()
            .map(|r| {
                let Some((_, verdict)) = verdicts
                    .iter()
                    .find(|(c, _)| c.id.is_some() && c.id == r.id)
                else {
                    return r.clone();
                };
                let t = &tallies[r.id.as_deref().unwrap()];
                let scope = r.scope.clone().unwrap_or_default().scope();
                let against = t.attributed_against(&scope);
                evidence_lines.push(format!(
                    "{}: {} attributed regression(s) against its scope ({} in all) across {} \
                     probe(s) ({} improved, {} regressed at block level); last validated {}\n  rule: {}",
                    r.id.as_deref().unwrap(),
                    against,
                    t.attributed_regressions,
                    t.observations,
                    t.improved,
                    t.regressed,
                    t.last_validated.as_deref().unwrap_or("never"),
                    r.text,
                ));
                let leash = match r.probation {
                    true => format!(
                        " (probation: retires at {})",
                        retire_threshold_for(r, min_attributed)
                    ),
                    false => String::new(),
                };
                match verdict {
                    Verdict::Narrow {
                        scope,
                        support,
                        shed,
                    } => {
                        narrowed_count += 1;
                        let why = format!(
                            "{against} attributed regression(s) in {}{leash}; kept where it \
                             held ({})",
                            shed.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "),
                            support.iter().map(|s| s.describe()).collect::<Vec<_>>().join("; "),
                        );
                        evidence_lines.push(format!(
                            "  narrowed: loads with {} — {why}",
                            scope.describe()
                        ));
                        let mut narrowed = r.clone();
                        narrowed.scope = Some(scope.clone());
                        narrowed.support = support.clone();
                        narrowed.narrowed_at = Some(now.clone());
                        narrowed.narrowed_reason = Some(why);
                        narrowed
                    }
                    Verdict::Retire { why } => {
                        evidence_lines.push(format!("  retired: {why}"));
                        let mut retired = r.clone();
                        retired.enabled = false;
                        retired.retired_at = Some(now.clone());
                        // Name the shorter leash when it is what convicted,
                        // or the record reads as though the ordinary
                        // threshold was met.
                        retired.retired_reason =
                            Some(format!("{why} in the validation ledger{leash}"));
                        retired
                    }
                    Verdict::Stands => unreachable!("filtered above"),
                }
            })
            .collect();
        let retired_count = convicted.len() as u32 - narrowed_count;
        evidence_lines.push(format!(
            "deterministic ledger scan over {} record(s); threshold {min_attributed} \
             attributed regression(s); no model involved",
            records
                .iter()
                .filter(|rec: &&ValidationRecord| rec.domain == domain)
                .count(),
        ));

        // ── the direct path: write the retirement, no queue, no human ──
        //
        // A retired rule is disabled in place and keeps `retired_at` /
        // `retired_reason`, so this removes it from every future prompt
        // without removing it from the record — the learner is still told it
        // was tried and measured harmful, which is what stops it being
        // re-derived. That is the mechanism `git revert` was standing in for,
        // and unlike a revert it is per-rule and leaves the rest of the store
        // alone.
        if apply {
            store.write_learned_rules(&domain, &rules)?;
            store.append_run(&LeapRun {
                id: Session::new_id(),
                domain: domain.clone(),
                reflexions_processed: 0,
                // **Whole file, not the active subset** — the count every
                // other `LeapRun` writer uses (`learn` writes
                // `learned_before.len()` / `rules.len()`; `accept` the same).
                // A retirement never removes a row, so these are equal and
                // the pass shows as a flat step; counting `active()` here
                // instead put two different measures on one series in the
                // "Rule set over time" chart, where a retirement would read
                // as a drop and a consolidation as a total.
                rules_before: before.len() as u32,
                rules_after: rules.len() as u32,
                created_at: now.clone(),
            })?;
            store.commit(&format!(
                "retire[{domain}]: {retired_count} retired, {narrowed_count} narrowed at \
                 {min_attributed}+ attributed regression(s)"
            ));
            println!(
                "{domain}: retired {retired_count} rule(s), narrowed {narrowed_count} — {}",
                convicted_ids.join(", ")
            );
            // Two lines per convicted rule: the tally and the verdict.
            for line in evidence_lines.iter().take(convicted.len() * 2) {
                println!("  {}", line.replace('\n', "\n  "));
            }
            // The pending twin resolves, not lingers: an applied retirement
            // leaves nothing for a human to decide, and a proposal still
            // `pending` after its content landed reads as awaiting review —
            // to `mecha proposals` and to doctor — forever. Superseded, not
            // accepted: nobody ruled on the paper, the direct path overtook
            // it. Retirement proposals hold no reflections, so there is
            // nothing to release.
            for mut p in pending_twins {
                p.status = "superseded".into();
                p.resolved_at = Some(now.clone());
                p.reason =
                    Some("superseded: the same retirement was applied directly (--apply)".into());
                store.write_proposal(&p)?;
            }
            staged += 1;
            continue;
        }

        let proposal = Proposal {
            id: Session::new_id(),
            domain: domain.clone(),
            status: "pending".into(),
            // No reflections are consumed: retirement argues from the
            // ledger, and the rules' own sources stay marked as they were.
            reflexion_ids: Vec::new(),
            rules_before: before.clone(),
            rules,
            evidence: evidence_lines.join("\n"),
            created_at: now,
            resolved_at: None,
            reason: None,
            // A retirement argues about the whole domain, not one region.
            scope: None,
        };
        store.write_proposal(&proposal)?;
        store.commit(&format!(
            "propose-retirement[{domain}]: {retired_count} to retire, {narrowed_count} to narrow \
             — {}",
            proposal.id
        ));
        println!(
            "{domain}: proposal {} retires {retired_count} and narrows {narrowed_count} rule(s) \
             — review with `mecha proposals show {}`",
            proposal.id, proposal.id
        );
        staged += 1;
    }
    if staged == 0 {
        println!(
            "no rule has {min_attributed}+ attributed regressions — nothing to retire \
             (`mecha rules` shows the tallies)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::learning::rules_hash;

    fn temp_store() -> LearningStore {
        // A process-unique counter, not a timestamp. `as_nanos()` is only as
        // fine-grained as the platform's clock: on macOS two of these called
        // from parallel test threads can land on the same value, and then two
        // tests share one directory — the first to finish `remove_dir_all`s
        // the other's store out from under it, which surfaces as a bare
        // `No such file or directory` in whichever test lost. Found on the
        // macOS CI arm, where it is a race rather than a certainty; it passed
        // twice before it failed.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("mecha-rules-test")
            .join(format!("{}-{seq}", std::process::id()));
        // **Cleared, not just uniquely named.** The counter guarantees no two
        // stores in *this* process collide; it says nothing about a directory
        // a previous run left behind, and `{pid}-{seq}` is deterministic, so a
        // later run drawing the same pid reopens it. These stores append, so
        // the leftover records would be counted alongside the new ones — a
        // confusing count mismatch rather than a clean failure. Removing first
        // makes the fixture fresh regardless of what any earlier run did.
        let _ = std::fs::remove_dir_all(&dir);
        LearningStore::open(dir).unwrap()
    }

    fn rule(text: &str, id: &str) -> Rule {
        Rule {
            text: text.into(),
            id: Some(id.into()),
            ..Default::default()
        }
    }

    fn regression(rule_id: &str, at: &str) -> ValidationRecord {
        ValidationRecord {
            reflexion_id: "refl".into(),
            trigger: "steer".into(),
            domain: "behavior".into(),
            rules_hash: rules_hash("block"),
            rule_ids: vec![rule_id.into()],
            outcome: "regressed".into(),
            attributed_rule_id: Some(rule_id.into()),
            model: "qwen".into(),
            created_at: at.into(),
            region: None,
        }
    }

    #[test]
    fn retirement_is_proposed_at_the_threshold_and_through_the_gate() {
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[rule("Bad rule.", "r-bad"), rule("Fine rule.", "r-ok")],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Below threshold: nothing staged.
        propose(&store, 4, false).unwrap();
        assert!(store.proposals().unwrap().is_empty());

        // At threshold: one pending proposal that retires r-bad, keeps r-ok,
        // and consumes no reflections. The live rules must be untouched —
        // only acceptance deploys.
        propose(&store, 3, false).unwrap();
        let all = store.proposals().unwrap();
        assert_eq!(all.len(), 1);
        let p = &all[0];
        assert_eq!(p.status, "pending");
        assert!(p.reflexion_ids.is_empty());
        let bad = p
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bad"))
            .unwrap();
        assert!(bad.retired_at.is_some() && !bad.enabled);
        assert!(bad
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("3 attributed"));
        assert!(p
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-ok"))
            .unwrap()
            .active());
        assert!(p.evidence.contains("no model involved"));
        let live = store.learned_rules("behavior").unwrap();
        assert!(
            live.iter().all(|r| r.active()),
            "staging must not touch the live rules"
        );

        // Re-running while the proposal is pending must not stage a twin.
        propose(&store, 3, false).unwrap();
        assert_eq!(store.proposals().unwrap().len(), 1);

        std::fs::remove_dir_all(store.root()).ok();
    }

    /// A pending retirement proposal overtaken by `--apply` resolves as
    /// superseded rather than lingering. Fails on the old behaviour: the
    /// direct path retired the rule and left the paper `pending`, so
    /// `mecha proposals` and doctor read an already-applied retirement as
    /// awaiting review forever.
    #[test]
    fn apply_resolves_the_pending_twin_it_overtakes() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Bad rule.", "r-bad")])
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Staged first, as the nightly would have before --apply existed.
        propose(&store, 3, false).unwrap();
        assert_eq!(store.proposals().unwrap()[0].status, "pending");

        // The direct path retires the rule and resolves the paper.
        propose(&store, 3, true).unwrap();
        let all = store.proposals().unwrap();
        assert_eq!(all.len(), 1, "no twin staged");
        assert_eq!(all[0].status, "superseded");
        assert!(all[0].resolved_at.is_some());
        assert!(all[0].reason.as_deref().unwrap().contains("--apply"));
        assert!(
            !store.learned_rules("behavior").unwrap()[0].active(),
            "the retirement itself landed"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }

    /// **Retirement removes a rule from the live set with no queue and no
    /// human.** Until this existed, a rule measured harmful reached
    /// `retired_at` only inside a *proposal*, and the only thing that ever
    /// took one out of a prompt was `git revert` over the whole store — a
    /// whole-store undo standing in for a per-rule mechanism.
    ///
    /// Fails on the old behaviour: `--apply` did not exist, so the live rules
    /// were untouched by any scan and `r-bad` stayed active forever.
    #[test]
    fn retirement_applied_directly_disables_the_rule_and_leaves_the_rest_alone() {
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[rule("Bad rule.", "r-bad"), rule("Fine rule.", "r-ok")],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&regression(
                    "r-bad",
                    &format!("2026-08-0{}T00:00:00Z", i + 1),
                ))
                .unwrap();
        }

        // Below threshold, --apply must be as inert as staging is.
        propose(&store, 4, true).unwrap();
        assert!(
            store
                .learned_rules("behavior")
                .unwrap()
                .iter()
                .all(|r| r.active()),
            "an unconvicted rule must survive an --apply scan"
        );

        propose(&store, 3, true).unwrap();

        // The live file moved, and nothing was queued for anyone to accept.
        assert!(
            store.proposals().unwrap().is_empty(),
            "--apply must not also stage a proposal"
        );
        let live = store.learned_rules("behavior").unwrap();
        let bad = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bad"))
            .unwrap();
        assert!(!bad.active(), "the convicted rule must leave the prompt");
        assert!(bad.retired_at.is_some());
        assert!(bad
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("3 attributed"));

        // Retired, not deleted: it stays as evidence so the learner is told it
        // was tried and measured harmful, which is what stops re-derivation.
        assert_eq!(live.len(), 2, "a retired rule stays in the file");
        assert!(
            live.iter()
                .find(|r| r.id.as_deref() == Some("r-ok"))
                .unwrap()
                .active(),
            "retirement must be per-rule, not a whole-store revert"
        );

        // And it is recorded as a pass, so `git log` in the store reads as the
        // system's learning history rather than an unexplained file change.
        // Counted over the whole file, like every other `LeapRun` writer — a
        // retirement disables a rule without removing its row, so both are 2
        // and the pass reads as a flat step. Counting the *active* subset here
        // would put two different measures on one chart series.
        let runs = std::fs::read_to_string(store.root().join("runs.jsonl")).unwrap();
        assert!(
            runs.contains("\"rules_before\":2") && runs.contains("\"rules_after\":2"),
            "a retirement pass records the file's rule count, not the active one: {runs}"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }
    /// **§17.4 end to end: the nightly scan narrows a widened rule to where
    /// it held and retires one it cannot narrow, in one pass, and the next
    /// pass leaves the narrowed rule alone.** Fails on the pre-narrowing
    /// scan, which retired both.
    #[test]
    fn the_scan_narrows_where_it_can_and_retires_where_it_cannot() {
        use mecha_core::situation::Situation;
        let sit = |tools: &[&str]| {
            Situation::of_run(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                None,
            )
        };
        let store = temp_store();
        let widened = Rule {
            scope: Some(Situation::default()),
            support: vec![sit(&["shell"]), sit(&["http_fetch"])],
            ..rule("Widened rule.", "r-wide")
        };
        store
            .write_learned_rules("behavior", &[widened, rule("Bare rule.", "r-bare")])
            .unwrap();
        let placed = |rule_id: &str, tools: &[&str], outcome: &str, attributed: bool, at: &str| {
            ValidationRecord {
                outcome: outcome.into(),
                attributed_rule_id: attributed.then(|| rule_id.into()),
                region: Some(sit(tools)),
                ..regression(rule_id, at)
            }
        };
        for i in 0..3 {
            let at = format!("2026-09-0{}T00:00:00Z", i + 1);
            store
                .append_validation(&placed(
                    "r-wide",
                    &["shell", "fs_read"],
                    "regressed",
                    true,
                    &at,
                ))
                .unwrap();
            store.append_validation(&regression("r-bare", &at)).unwrap();
        }
        store
            .append_validation(&placed(
                "r-wide",
                &["http_fetch"],
                "unchanged_pass",
                false,
                "2026-09-04T00:00:00Z",
            ))
            .unwrap();

        propose(&store, 3, true).unwrap();

        let live = store.learned_rules("behavior").unwrap();
        let wide = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(wide.active(), "narrowed, not retired");
        assert_eq!(wide.scope, Some(sit(&["http_fetch"])));
        assert_eq!(wide.support, vec![sit(&["http_fetch"])]);
        assert!(wide.narrowed_at.is_some());
        let why = wide.narrowed_reason.as_deref().unwrap();
        assert!(why.contains("shell") && why.contains("http_fetch"), "{why}");
        let bare = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-bare"))
            .unwrap();
        assert!(!bare.active());
        assert!(bare
            .retired_reason
            .as_deref()
            .unwrap()
            .contains("no recorded support"));
        let runs = std::fs::read_to_string(store.root().join("runs.jsonl")).unwrap();
        assert!(runs.contains("\"rules_before\":2"));

        // The convictions that narrowed it lie outside where it now loads:
        // a second scan finds nothing against it.
        propose(&store, 3, true).unwrap();
        let again = store.learned_rules("behavior").unwrap();
        let wide = again
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(wide.active());
        assert_eq!(wide.scope, Some(sit(&["http_fetch"])));

        // And the roster says so, in prose.
        let tallies = rule_tallies(&store.validations().unwrap());
        let line = describe(wide, &tallies);
        assert!(line.contains("loads with http_fetch"), "{line}");
        assert!(line.contains("narrowed "), "{line}");
        assert!(line.contains("by region:"), "{line}");
        assert!(
            line.contains("fs_read, shell 3 graded (3 attributed)"),
            "{line}"
        );
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// Staged rather than applied, a narrowing is a proposal whose rule
    /// carries the new scope, and a pending twin is not re-staged.
    #[test]
    fn a_narrowing_stages_as_a_proposal_and_is_not_staged_twice() {
        use mecha_core::situation::Situation;
        let sit = |tools: &[&str]| {
            Situation::of_run(
                &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                None,
            )
        };
        let store = temp_store();
        store
            .write_learned_rules(
                "behavior",
                &[Rule {
                    scope: Some(Situation::default()),
                    support: vec![sit(&["shell"]), sit(&["http_fetch"])],
                    ..rule("Widened rule.", "r-wide")
                }],
            )
            .unwrap();
        for i in 0..3 {
            store
                .append_validation(&ValidationRecord {
                    region: Some(sit(&["shell"])),
                    ..regression("r-wide", &format!("2026-09-0{}T00:00:00Z", i + 1))
                })
                .unwrap();
        }
        propose(&store, 3, false).unwrap();
        propose(&store, 3, false).unwrap();
        let proposals = store.proposals().unwrap();
        assert_eq!(proposals.len(), 1, "a pending twin is not re-staged");
        let staged = proposals[0]
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some("r-wide"))
            .unwrap();
        assert!(staged.active());
        assert_eq!(staged.scope, Some(sit(&["http_fetch"])));
        assert!(staged.narrowed_at.is_some());
        assert!(proposals[0]
            .evidence
            .contains("narrowed: loads with http_fetch"));
        // The live rule is untouched until someone accepts.
        let live = &store.learned_rules("behavior").unwrap()[0];
        assert_eq!(live.scope, Some(Situation::default()));
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// **The D1 leash is reachable: a probationary rule convicts at 2, and
    /// an ordinary rule with the same evidence survives.** Fails on the old
    /// release predicate (`observations > 0`): the two conviction rows were
    /// themselves observations, so probation was stripped in the same scan
    /// that read them and the rule answered to the ordinary threshold of 3 —
    /// `PROBATION_RETIRE_AT` could never be the operative threshold, and the
    /// only brake in front of an ungraded rule was two-thirds longer than
    /// every document said.
    #[test]
    fn a_probationary_rule_convicts_at_the_shorter_leash() {
        let store = temp_store();
        let mut bad = rule("Bad ungraded rule.", "r-probation");
        bad.probation = true;
        store
            .write_learned_rules("behavior", &[bad, rule("Plain rule.", "r-plain")])
            .unwrap();
        // Two attributed regressions each — conviction evidence and nothing
        // else, so the probationary rule's graded history is exactly its
        // convictions and the leash must hold.
        for i in 0..2 {
            store
                .append_validation(&regression(
                    "r-probation",
                    &format!("2026-08-3{}T00:00:00Z", i),
                ))
                .unwrap();
            store
                .append_validation(&regression("r-plain", &format!("2026-08-3{}T12:00:00Z", i)))
                .unwrap();
        }

        propose(&store, mecha_core::learning::DEFAULT_RETIRE_AT, true).unwrap();

        let live = store.learned_rules("behavior").unwrap();
        let bad = live
            .iter()
            .find(|r| r.id.as_deref() == Some("r-probation"))
            .unwrap();
        assert!(
            !bad.active(),
            "two attributed regressions retire a probationary rule"
        );
        assert!(
            bad.retired_reason.as_deref().unwrap().contains("probation"),
            "the record names the shorter leash, not the ordinary threshold: {:?}",
            bad.retired_reason
        );
        assert!(
            live.iter()
                .find(|r| r.id.as_deref() == Some("r-plain"))
                .unwrap()
                .active(),
            "the same evidence leaves an ordinary rule below its threshold"
        );

        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn retire_and_restore_round_trip_by_id_prefix() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Rule one.", "r-20260805-aaaa")])
            .unwrap();
        retire(&store, "r-20260805", Some("measured harmful".into())).unwrap();
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(!r.active());
        assert_eq!(r.retired_reason.as_deref(), Some("measured harmful"));

        // Already-retired is an error, not a silent double-stamp.
        assert!(retire(&store, "r-20260805", None).is_err());

        restore(&store, "r-20260805").unwrap();
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(r.active() && r.retired_at.is_none() && r.retired_reason.is_none());
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn an_ambiguous_or_unknown_rule_id_is_an_error() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("A.", "r-1a"), rule("B.", "r-1b")])
            .unwrap();
        assert!(find_rule(&store, "r-1").is_err(), "prefix matches two");
        assert!(find_rule(&store, "r-9").is_err(), "matches none");
        assert!(find_rule(&store, "r-1a").is_ok());
        std::fs::remove_dir_all(store.root()).ok();
    }

    /// `rid.starts_with("")` is true for every id, so an empty needle used to
    /// resolve to whichever learned rule happened to be alone in its domain
    /// — a TUI row with no id (a user rule, or a pre-identity learned one,
    /// both of which serialise `"id": null` and read back as `""`) would
    /// silently retire an unrelated rule instead of failing. This is the
    /// case with exactly one learned rule on disk, where the old code found
    /// exactly one hit and acted on it.
    #[test]
    fn an_empty_id_never_matches_a_rule_by_accident() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Only rule.", "r-only")])
            .unwrap();
        assert!(
            find_rule(&store, "").is_err(),
            "an empty needle matches nothing, not everything"
        );
        assert!(retire(&store, "", None).is_err());
        // The rule is untouched.
        let r = &store.learned_rules("behavior").unwrap()[0];
        assert!(r.active());
        std::fs::remove_dir_all(store.root()).ok();
    }

    #[test]
    fn show_prints_the_rule_and_its_tally() {
        let store = temp_store();
        store
            .write_learned_rules("behavior", &[rule("Ship the thing.", "r-show")])
            .unwrap();
        assert!(show(&store, "r-show").is_ok());
        assert!(show(&store, "").is_err(), "no id given");
        assert!(show(&store, "nope").is_err(), "no such rule");
        std::fs::remove_dir_all(store.root()).ok();
    }
}
