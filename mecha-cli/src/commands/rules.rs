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
    rule_tallies, LearningStore, Proposal, Rule, RuleTally, ValidationRecord,
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
        /// Attributed regressions required before a rule is proposed for
        /// retirement.
        #[arg(long, default_value_t = 3)]
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
        Cmd::ProposeRetirements { min_attributed } => propose(&store, min_attributed),
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
                    "observations": tally.map(|t| t.observations),
                    "attributed_regressions": tally.map(|t| t.attributed_regressions),
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
    } else {
        "active".into()
    };
    let measured = match r.id.as_deref().and_then(|id| tallies.get(id)) {
        Some(t) => format!(
            "{} probe(s): {} improved, {} regressed, {} attributed to this rule; last {}",
            t.observations,
            t.improved,
            t.regressed,
            t.attributed_regressions,
            t.last_validated.as_deref().unwrap_or("never")
        ),
        None => "never validated".into(),
    };
    format!(
        "[{state}] {}\n      id {id} · created {} · {measured}",
        r.text,
        r.created_at.as_deref().unwrap_or("unknown"),
    )
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

fn propose(store: &LearningStore, min_attributed: u32) -> Result<()> {
    let _lock = store.lock()?;
    let records = store.validations()?;
    let tallies = rule_tallies(&records);
    let proposals = store.proposals()?;
    let mut staged = 0u32;

    for domain in store.domains() {
        let before = store.learned_rules(&domain)?;
        let convicted: Vec<&Rule> = before
            .iter()
            .filter(|r| r.active())
            .filter(|r| {
                r.id.as_deref()
                    .and_then(|id| tallies.get(id))
                    .is_some_and(|t| t.attributed_regressions >= min_attributed)
            })
            .collect();
        if convicted.is_empty() {
            continue;
        }
        // A pending proposal already retiring these exact rules is not
        // re-staged — the nightly must not spam the queue while a human
        // hasn't looked yet.
        let convicted_ids: Vec<&str> = convicted.iter().filter_map(|r| r.id.as_deref()).collect();
        let already = proposals.iter().any(|p| {
            p.status == "pending"
                && p.domain == domain
                && convicted_ids.iter().all(|id| {
                    p.rules
                        .iter()
                        .any(|r| r.id.as_deref() == Some(*id) && r.retired_at.is_some())
                })
        });
        if already {
            println!("{domain}: retirement already pending — review with `mecha proposals`");
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut evidence_lines = Vec::new();
        let rules: Vec<Rule> = before
            .iter()
            .map(|r| {
                let convicted =
                    r.id.as_deref()
                        .is_some_and(|id| convicted_ids.contains(&id));
                if !convicted {
                    return r.clone();
                }
                let t = &tallies[r.id.as_deref().unwrap()];
                evidence_lines.push(format!(
                    "{}: {} attributed regression(s) across {} probe(s) ({} improved, {} \
                     regressed at block level); last validated {}\n  rule: {}",
                    r.id.as_deref().unwrap(),
                    t.attributed_regressions,
                    t.observations,
                    t.improved,
                    t.regressed,
                    t.last_validated.as_deref().unwrap_or("never"),
                    r.text,
                ));
                let mut retired = r.clone();
                retired.enabled = false;
                retired.retired_at = Some(now.clone());
                retired.retired_reason = Some(format!(
                    "{} attributed regression(s) in the validation ledger",
                    t.attributed_regressions
                ));
                retired
            })
            .collect();
        evidence_lines.push(format!(
            "deterministic ledger scan over {} record(s); threshold {min_attributed} \
             attributed regression(s); no model involved",
            records
                .iter()
                .filter(|rec: &&ValidationRecord| rec.domain == domain)
                .count(),
        ));

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
        };
        store.write_proposal(&proposal)?;
        store.commit(&format!(
            "propose-retirement[{domain}]: {} rule(s) — {}",
            convicted.len(),
            proposal.id
        ));
        println!(
            "{domain}: proposal {} retires {} rule(s) — review with `mecha proposals show {}`",
            proposal.id,
            convicted.len(),
            proposal.id
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
        propose(&store, 4).unwrap();
        assert!(store.proposals().unwrap().is_empty());

        // At threshold: one pending proposal that retires r-bad, keeps r-ok,
        // and consumes no reflections. The live rules must be untouched —
        // only acceptance deploys.
        propose(&store, 3).unwrap();
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
        propose(&store, 3).unwrap();
        assert_eq!(store.proposals().unwrap().len(), 1);

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
