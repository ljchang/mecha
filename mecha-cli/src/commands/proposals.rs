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
    List,
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
}

pub async fn execute(args: Args) -> Result<()> {
    let store = LearningStore::open(LearningStore::default_root()?)?;
    match args.cmd.unwrap_or(Cmd::List) {
        Cmd::List => list(&store),
        Cmd::Show { id } => show(&store, &id),
        Cmd::Accept { id, force } => accept(&store, &id, force),
        Cmd::Reject { id, reason } => reject(&store, &id, reason),
    }
}

fn list(store: &LearningStore) -> Result<()> {
    let proposals = store.proposals()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> Rule {
        Rule {
            text: text.into(),
            ..Default::default()
        }
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
