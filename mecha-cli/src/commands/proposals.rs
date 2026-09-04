//! `mecha proposals` — review what unattended learning wants to change.
//!
//! The human half of the hyperagent gate. `mecha learn --propose` stages a
//! candidate rule set with the counterfactual evidence that argues for it;
//! nothing touches the live rules until `accept` here. The two resolutions
//! differ in what happens to the evidence trail, not just the rules:
//!
//! - **accept** writes the rules, records the `LeapRun`, and marks the
//!   reflections processed with the proposal's id — the same lineage a
//!   direct `mecha learn` leaves, plus the proposal file with its evidence.
//! - **reject** also marks the reflections processed. They were real
//!   corrections, but re-arguing them nightly against a human's explicit no
//!   is how a proposal queue becomes spam. The refusal is recorded with its
//!   reason; the reflections stay in the archive as evidence.
//! - **supersede** is the one resolution that does **not** consume them. A
//!   proposal nobody answered is not a refusal, and burning its evidence to
//!   clear the queue would lose real corrections the owner never ruled on.
//!   Status leaves `pending`, so `learn`'s claim (`status == "pending"`)
//!   releases the reflections back to the pool and the next pass argues them
//!   against the rules that are live *now*.
//!
//! **Why a queue needs this at all.** Every proposal is a full rewrite of its
//! domain measured against `rules_before`, and `accept` refuses to apply one
//! whose baseline moved. So a second pending proposal is not a second
//! decision — accepting either one makes the rest unappliable. Four
//! accumulated over six days on 2026-08-29 holding 27 of 43 reflections
//! hostage, and `learn` skipped every night for want of three free ones: the
//! queue had stalled itself, and nothing said so.
//!
//! Accepting checks that the live rules still match what the candidate was
//! diffed (and measured!) against — if a direct learn pass or a hand edit
//! moved them in the meantime, the proposal's evidence no longer describes
//! this deployment, and applying it anyway needs `--force`.

use anyhow::{bail, Result};
use mecha_core::learning::{LeapRun, LearningStore, Rule};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// List proposals (default).
    List {
        /// Machine-readable, for /queues
        #[arg(long)]
        json: bool,
    },
    /// Show one proposal: rules diff and the gate's evidence.
    Show { id: String },
    /// Apply a pending proposal to the live rules.
    Accept {
        id: String,
        /// Apply even though the live rules changed since the proposal was
        /// measured.
        #[arg(long)]
        force: bool,
    },
    /// Refuse a pending proposal, consuming its reflections.
    Reject {
        id: String,
        /// Why — recorded on the proposal for the next reader.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Retire a stale pending proposal, releasing its reflections unconsumed.
    Supersede {
        /// The proposal to retire. Omit with --stale to sweep every pending
        /// proposal whose baseline no longer matches the live rules.
        id: Option<String>,
        /// Sweep every pending proposal measured against a baseline that has
        /// since moved.
        #[arg(long)]
        stale: bool,
        /// Why — recorded on the proposal for the next reader.
        #[arg(long)]
        reason: Option<String>,
    },
}

pub async fn execute(args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    match args.cmd.unwrap_or(Cmd::List { json: false }) {
        Cmd::List { json } => list(&store, json),
        Cmd::Show { id } => show(&store, &id),
        Cmd::Accept { id, force } => accept(&store, &id, force),
        Cmd::Reject { id, reason } => reject(&store, &id, reason),
        Cmd::Supersede { id, stale, reason } => supersede_cmd(&store, id, stale, reason),
    }
}

fn list(store: &LearningStore, as_json: bool) -> Result<()> {
    let proposals = store.proposals()?;
    // The shape every reviewable store answers in, so /queues can hold one
    // review surface rather than one per store: what it is, what it would
    // do, and what state it is in.
    if as_json {
        let rows: Vec<serde_json::Value> = proposals
            .iter()
            .filter(|p| p.status == "pending")
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "kind": p.domain,
                    "title": format!("{} rule(s) from {} reflection(s)",
                                     p.rules.len(), p.reflexion_ids.len()),
                    "detail": p.status,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if proposals.is_empty() {
        println!("no proposals — `mecha learn --propose` creates them");
        return Ok(());
    }
    for p in &proposals {
        println!(
            "{}  {:<16} {:<10} {} rule(s) from {} reflection(s)",
            p.id,
            p.status,
            p.domain,
            p.rules.len(),
            p.reflexion_ids.len()
        );
    }
    Ok(())
}

fn show(store: &LearningStore, id: &str) -> Result<()> {
    let p = store.proposal(id)?;
    println!("proposal {} · {} · {}", p.id, p.domain, p.status);
    println!("created {}", p.created_at);
    if let Some(scope) = &p.scope {
        println!("situation: {}", scope.describe());
    }
    if let Some(resolved) = &p.resolved_at {
        println!(
            "resolved {resolved}{}",
            p.reason
                .as_deref()
                .map(|r| format!(" — {r}"))
                .unwrap_or_default()
        );
    }
    println!("\n{}", render_diff(&p.rules_before, &p.rules));
    println!("evidence:\n{}", indent(&p.evidence));
    if p.status == "pending" {
        println!("\naccept with `mecha proposals accept {}`", p.id);
    }
    Ok(())
}

fn accept(store: &LearningStore, id: &str, force: bool) -> Result<()> {
    let _lock = store.lock()?;
    let mut p = store.proposal(id)?;
    if p.status != "pending" {
        bail!("proposal {} is {}, not pending", p.id, p.status);
    }
    // The evidence measured the candidate against these exact rules. If the
    // live set moved, the diff on screen is not the change being applied.
    let live = store.learned_rules(&p.domain)?;
    if !same_rules(&live, &p.rules_before) && !force {
        bail!(
            "the live rules for `{}` changed after this proposal was measured; \
             re-run `mecha learn --propose`, or apply anyway with --force",
            p.domain
        );
    }

    store.write_learned_rules(&p.domain, &p.rules)?;
    store.append_run(&LeapRun {
        id: p.id.clone(),
        domain: p.domain.clone(),
        reflexions_processed: p.reflexion_ids.len() as u32,
        rules_before: p.rules_before.len() as u32,
        rules_after: p.rules.len() as u32,
        created_at: chrono::Utc::now().to_rfc3339(),
    })?;
    store.mark_reflexions_processed(&p.reflexion_ids, &p.id)?;
    p.status = "accepted".into();
    p.resolved_at = Some(chrono::Utc::now().to_rfc3339());
    store.write_proposal(&p)?;
    store.commit(&format!(
        "accept[{}]: proposal {} — {} rule(s)",
        p.domain,
        p.id,
        p.rules.len()
    ));
    println!(
        "accepted: {} rule(s) now live for `{}`",
        p.rules.len(),
        p.domain
    );
    Ok(())
}

fn reject(store: &LearningStore, id: &str, reason: Option<String>) -> Result<()> {
    let _lock = store.lock()?;
    let mut p = store.proposal(id)?;
    if p.status != "pending" {
        bail!("proposal {} is {}, not pending", p.id, p.status);
    }
    store.mark_reflexions_processed(&p.reflexion_ids, &p.id)?;
    p.status = "rejected".into();
    p.resolved_at = Some(chrono::Utc::now().to_rfc3339());
    p.reason = reason;
    store.write_proposal(&p)?;
    store.commit(&format!("reject[{}]: proposal {}", p.domain, p.id));
    println!("rejected; its reflections will not be re-argued");
    Ok(())
}

/// A text diff of two rule sets, matched by rule text — order is not meaning
/// in a rules file, and a reordering that changes nothing should show as
/// nothing. A rule whose text survives but whose *liveness* changed is a real
/// change too: retirement proposals are exactly that shape, and a diff that
/// answered them with "(no textual change)" would show a reviewer nothing.
fn render_diff(before: &[Rule], after: &[Rule]) -> String {
    let mut out = String::new();
    for r in before {
        if !after.iter().any(|a| a.text == r.text) {
            out.push_str(&format!("  - {}\n", r.text));
        }
    }
    for r in after {
        match before.iter().find(|b| b.text == r.text) {
            None => out.push_str(&format!("  + {}\n", r.text)),
            Some(b) if b.active() && !r.active() => out.push_str(&format!(
                "  ~ retired: {}{}\n",
                r.text,
                r.retired_reason
                    .as_deref()
                    .map(|w| format!(" ({w})"))
                    .unwrap_or_default()
            )),
            Some(b) if !b.active() && r.active() => {
                out.push_str(&format!("  ~ restored: {}\n", r.text))
            }
            Some(_) => {}
        }
    }
    if out.is_empty() {
        out.push_str("  (no textual change)\n");
    }
    out
}

fn same_rules(a: &[Rule], b: &[Rule]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.text == y.text && x.enabled == y.enabled)
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Mark one proposal superseded, releasing its reflections **unconsumed**.
///
/// The distinction from [`reject`] is the whole point and is the reason this
/// is a separate verb rather than a flag: reject is the owner saying no, and
/// marks the reflections processed so the same argument is not made nightly
/// forever. Supersede is nobody having said anything, and must leave the
/// evidence exactly as it found it — the reflections were real corrections
/// the owner never ruled on, and consuming them to tidy the queue would
/// destroy the only record of them.
///
/// Releasing is implicit rather than an edit to the reflections: `learn`
/// claims on `status == "pending"`, so moving the status is the release.
fn supersede_one(p: &mut mecha_core::learning::Proposal, reason: &str) {
    p.status = "superseded".into();
    p.resolved_at = Some(chrono::Utc::now().to_rfc3339());
    p.reason = Some(reason.to_string());
}

/// Whether a pending proposal could still be applied at all.
///
/// `accept` refuses a proposal whose measured baseline no longer matches the
/// live rules, so one that fails this is not a decision awaiting an owner —
/// it is unappliable paper, and the reflections behind it are being held for
/// nothing.
fn is_stale(store: &LearningStore, p: &mecha_core::learning::Proposal) -> Result<bool> {
    let live = store.learned_rules(&p.domain)?;
    Ok(!same_rules(&live, &p.rules_before))
}

fn supersede_cmd(
    store: &LearningStore,
    id: Option<String>,
    stale: bool,
    reason: Option<String>,
) -> Result<()> {
    let _lock = store.lock()?;
    let mut proposals = store.proposals()?;

    // Which ones this call is about. `--stale` is deliberately not "every
    // pending proposal": a proposal whose baseline still matches is a real
    // decision waiting for the owner, and sweeping it away silently would be
    // this command committing the failure it exists to fix.
    let targets: Vec<String> = match (&id, stale) {
        // Resolved through `store.proposal`, which matches on **prefix** and
        // bails on zero or ambiguous — the same resolution `accept` and
        // `reject` use. Comparing full ids here instead made the abbreviation
        // that works for every other verb silently match nothing and exit 0,
        // which reads as "done" when nothing happened.
        (Some(id), false) => vec![store.proposal(id)?.id],
        (None, true) => {
            let mut out = Vec::new();
            for p in proposals.iter().filter(|p| p.status == "pending") {
                if is_stale(store, p)? {
                    out.push(p.id.clone());
                }
            }
            out
        }
        (Some(_), true) => bail!("give an id or --stale, not both"),
        (None, false) => bail!("give a proposal id, or --stale to sweep unappliable ones"),
    };

    if targets.is_empty() {
        // Only reachable under `--stale`: an explicit id has already been
        // resolved by `store.proposal`, which errors rather than returning
        // nothing.
        println!("no stale proposals — nothing to supersede");
        return Ok(());
    }

    let why = reason.unwrap_or_else(|| {
        "superseded: measured against a baseline the live rules have moved past".into()
    });
    let mut released: std::collections::BTreeSet<String> = Default::default();
    let mut done = 0usize;
    for p in proposals.iter_mut() {
        if !targets.contains(&p.id) {
            continue;
        }
        if p.status != "pending" {
            bail!("proposal {} is {}, not pending", p.id, p.status);
        }
        released.extend(p.reflexion_ids.iter().cloned());
        supersede_one(p, &why);
        store.write_proposal(p)?;
        done += 1;
    }
    store.commit(&format!(
        "supersede: {done} proposal(s), {} reflection(s) released",
        released.len()
    ));
    println!(
        "superseded {done} proposal(s); {} reflection(s) released back to the pool \
         (not consumed — `mecha learn` will argue them against the current rules)",
        released.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::learning::{Evidence, Origin, Proposal, Reflexion};

    fn rule(text: &str) -> Rule {
        Rule {
            text: text.into(),
            ..Default::default()
        }
    }

    fn temp_store() -> LearningStore {
        // Same fixture discipline as `rules.rs`: a process-unique counter
        // rather than a clock, and cleared rather than merely named, because
        // these stores append and a directory a previous run left behind
        // would be counted alongside the new records.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("mecha-proposals-test")
            .join(format!("{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        LearningStore::open(dir).unwrap()
    }

    fn reflexion(id: &str) -> Reflexion {
        Reflexion {
            id: id.into(),
            domain: "behavior".into(),
            session_id: "s-1".into(),
            trigger: "steer".into(),
            context: "c".into(),
            intervention: "i".into(),
            reflexion_text: "a lesson".into(),
            error_type: None,
            confidence: None,
            is_processed: false,
            leap_run_id: None,
            created_at: "2026-08-23T00:00:00Z".into(),
            origin: Origin::Clean,
            evidence: Evidence::Full,
            edited_at: None,
            dropped_at: None,
            dropped_reason: None,
            situation: None,
        }
    }

    fn staged(store: &LearningStore, id: &str, reflexions: &[&str]) -> Proposal {
        let p = Proposal {
            id: id.into(),
            domain: "behavior".into(),
            status: "pending".into(),
            reflexion_ids: reflexions.iter().map(|s| (*s).to_string()).collect(),
            rules_before: Vec::new(),
            rules: vec![rule("learned something")],
            evidence: "no trace-gradeable reflections in this batch".into(),
            created_at: "2026-08-23T00:00:00Z".into(),
            resolved_at: None,
            reason: None,
            scope: None,
        };
        store.write_proposal(&p).unwrap();
        p
    }

    /// **Supersede releases a proposal's reflections; reject consumes them.**
    ///
    /// This is the whole reason supersede is a separate verb rather than a
    /// flag on reject. A proposal nobody answered is not a refusal: its
    /// reflections were real corrections the owner never ruled on, and
    /// clearing the queue with `reject` would mark them processed and destroy
    /// the only record. Four accumulated on 2026-08-29 holding 27 reflections,
    /// and reject was the only verb that would move them.
    ///
    /// Fails on the old behaviour: there was no verb that released a claim,
    /// so the only ways out of the queue both consumed the evidence.
    #[test]
    fn supersede_releases_the_claim_where_reject_consumes_it() {
        let store = temp_store();
        for id in ["r-1", "r-2", "r-3"] {
            store.append_reflexion(&reflexion(id)).unwrap();
        }
        staged(&store, "p-super", &["r-1", "r-2"]);
        staged(&store, "p-reject", &["r-3"]);

        supersede_cmd(&store, Some("p-super".into()), false, None).unwrap();
        reject(&store, "p-reject", Some("no".into())).unwrap();

        let by_id = |id: &str| {
            store
                .proposals()
                .unwrap()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
        };
        assert_eq!(by_id("p-super").status, "superseded");
        assert_eq!(by_id("p-reject").status, "rejected");

        // The distinction that matters is what happened to the evidence.
        let processed: Vec<String> = store
            .reflexions()
            .unwrap()
            .into_iter()
            .filter(|r| r.is_processed)
            .map(|r| r.id)
            .collect();
        assert_eq!(
            processed,
            vec!["r-3".to_string()],
            "reject consumes its reflections; supersede must not"
        );

        // And `learn` claims on `status == "pending"`, so both are free of a
        // claim — but only the superseded pair will be argued again.
        let claimed: Vec<String> = store
            .proposals()
            .unwrap()
            .into_iter()
            .filter(|p| p.status == "pending")
            .flat_map(|p| p.reflexion_ids)
            .collect();
        assert!(claimed.is_empty(), "neither holds a claim: {claimed:?}");

        let free: Vec<String> = store
            .reflexions()
            .unwrap()
            .into_iter()
            .filter(|r| !r.is_processed)
            .map(|r| r.id)
            .collect();
        assert_eq!(
            free,
            vec!["r-1".to_string(), "r-2".to_string()],
            "the superseded proposal's evidence returns to the pool"
        );
    }

    /// `--stale` sweeps only proposals that could no longer be applied, never
    /// every pending one.
    ///
    /// A proposal whose baseline still matches is a real decision waiting for
    /// the owner. Sweeping it away silently would be this command committing
    /// the failure it exists to fix.
    #[test]
    fn stale_sweeps_only_what_accept_would_already_refuse() {
        let store = temp_store();
        staged(&store, "p-appliable", &["r-1"]);

        // Live rules still match `rules_before` (both empty), so nothing is
        // stale and the sweep must decline.
        supersede_cmd(&store, None, true, None).unwrap();
        assert_eq!(
            store.proposals().unwrap()[0].status,
            "pending",
            "an appliable proposal is a decision, not paper"
        );

        // Move the live rules out from under it; now `accept` would refuse it
        // and the sweep must take it.
        store
            .write_learned_rules("behavior", &[rule("something else")])
            .unwrap();
        supersede_cmd(&store, None, true, None).unwrap();
        assert_eq!(store.proposals().unwrap()[0].status, "superseded");
    }

    /// Resolving twice is an error rather than a silent no-op: a second call
    /// means the caller believed something was still pending.
    #[test]
    fn a_resolved_proposal_cannot_be_superseded_again() {
        let store = temp_store();
        staged(&store, "p-1", &["r-1"]);
        supersede_cmd(&store, Some("p-1".into()), false, None).unwrap();
        assert!(supersede_cmd(&store, Some("p-1".into()), false, None).is_err());
    }

    /// An id and `--stale` are different requests and giving both is a
    /// mistake worth naming rather than silently preferring one.
    #[test]
    fn an_id_and_stale_together_are_refused() {
        let store = temp_store();
        assert!(supersede_cmd(&store, Some("p-1".into()), true, None).is_err());
        assert!(supersede_cmd(&store, None, false, None).is_err());
    }

    #[test]
    fn the_diff_names_what_changed_and_ignores_reordering() {
        let before = vec![rule("a"), rule("b")];
        let after = vec![rule("b"), rule("c")];
        let d = render_diff(&before, &after);
        assert!(d.contains("- a") && d.contains("+ c"), "{d}");
        assert!(!d.contains("- b") && !d.contains("+ b"), "{d}");

        let reordered = render_diff(&[rule("a"), rule("b")], &[rule("b"), rule("a")]);
        assert!(reordered.contains("no textual change"), "{reordered}");
    }

    #[test]
    fn a_retirement_shows_in_the_diff_even_though_the_text_survives() {
        let before = vec![rule("keep"), rule("bad")];
        let mut after = before.clone();
        after[1].enabled = false;
        after[1].retired_at = Some("2026-08-05T00:00:00Z".into());
        after[1].retired_reason = Some("3 attributed regressions".into());
        let d = render_diff(&before, &after);
        assert!(
            d.contains("~ retired: bad (3 attributed regressions)"),
            "{d}"
        );
        assert!(!d.contains("no textual change"), "{d}");

        let back = render_diff(&after, &before);
        assert!(back.contains("~ restored: bad"), "{back}");
    }

    #[test]
    fn rule_sets_match_on_text_and_enablement_not_metadata() {
        let a = vec![rule("x")];
        let mut b = vec![rule("x")];
        b[0].confidence = Some(0.5);
        assert!(same_rules(&a, &b), "confidence drift is not a conflict");
        b[0].enabled = false;
        assert!(!same_rules(&a, &b), "disabling a rule is a real change");
    }
}
