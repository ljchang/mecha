//! `mecha harness` — the self-improvement loop over the harness itself, and
//! the review surface for what it did.
//!
//! `ruminate` is the nightly verb: diagnose one change from the run corpus,
//! record it as a candidate, measure it by counterfactual replay of recent
//! sessions where the closed set allows, and dispose of it through the
//! candidate gate. A config change that **wins on selection, is confirmed on
//! the holdout, and holds the work guardrail** auto-accepts into the
//! override layer (§13.3 of the self-improvement research, the owner
//! ruling); everything else waits for a person. Prose and architecture stage
//! unmeasured — prose needs the content-sensitive arm (`mecha eval
//! --ab-config`), which is a human's spend — and a security-class proposal
//! stages with the standing warning and is never measured at all: a loop
//! that can argue for widening its own confinement will eventually argue
//! well.
//!
//! Everything is recorded, acceptances and rejections alike, so "is this
//! loop actually helping" is answerable from the store rather than from
//! impression — the §2 failure mode the whole design is arranged against.

use crate::commands::diagnose::{
    corpus_slice, evidence_for, harness_history, run_diagnostician, DiagnosisOutcome,
};
use crate::harness_probe;
use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::candidate::{judge_drawn, ChangeClass, Disposition, Pair, Prediction};
use mecha_core::harness::{
    parse_change, AcceptedOverride, HarnessCandidate, HarnessStore, Measurement, STATUS_ACCEPTED,
    STATUS_REJECTED, STATUS_REVERTED, STATUS_STAGED,
};
use mecha_core::session::Session;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// The nightly pass: diagnose → record → measure by replay → dispose.
    ///
    /// A measured win auto-applies to the override layer, reversibly; every
    /// other outcome stages or rejects with the evidence on the record.
    /// Exits 0 on "nothing to do" — a skipped night is not a failed night.
    Ruminate {
        /// Replay this many recent sessions per arm.
        #[arg(long, default_value_t = 16)]
        sessions: usize,
        /// Only diagnose from sessions started in the last N days.
        #[arg(long)]
        days: Option<i64>,
        /// Scan at most this many sessions for the corpus.
        #[arg(long, short = 'n', default_value_t = 200)]
        limit: usize,
        /// One episode in this many is held out of selection.
        #[arg(long, default_value_t = 3)]
        holdout_in: u64,
    },
    /// Candidates waiting on you (--all for the whole record).
    List {
        #[arg(long)]
        all: bool,
        /// Machine-readable, for /queues
        #[arg(long)]
        json: bool,
    },
    /// One candidate, whole: prediction, measurement, evidence.
    Show { id: String },
    /// Accept a staged candidate. A config change inside the closed set is
    /// applied to the override layer; anything else is only marked — the
    /// change itself is yours to make.
    Accept { id: String },
    /// Reject a staged candidate, with a reason the record keeps.
    Reject {
        id: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Take an accepted override back out. The candidate record survives as
    /// evidence, and the key returns to whatever your config says.
    Revert {
        /// Candidate id (or unique prefix), or a bare override key.
        id: String,
    },
    /// The active override layer, with where each entry came from.
    Overrides,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd {
        Cmd::Ruminate {
            sessions,
            days,
            limit,
            holdout_in,
        } => {
            anyhow::ensure!(holdout_in >= 2, "--holdout-in must be at least 2");
            ruminate(global, sessions, days, limit, holdout_in).await
        }
        Cmd::List { all, json } => list(all, json),
        Cmd::Show { id } => show(&id),
        Cmd::Accept { id } => accept(&id),
        Cmd::Reject { id, reason } => reject(&id, reason),
        Cmd::Revert { id } => revert(&id),
        Cmd::Overrides => overrides(),
    }
}

async fn ruminate(
    global: &GlobalOpts,
    sessions: usize,
    days: Option<i64>,
    limit: usize,
    holdout_in: u64,
) -> Result<()> {
    let store = HarnessStore::open_default()?;

    let Some((model, slice, _)) = corpus_slice(None, days, limit)? else {
        println!("no recorded run outcomes yet — nothing to diagnose from; deferring");
        return Ok(());
    };
    let evidence = evidence_for(&model, &slice, harness_history().unwrap_or_default());

    let diagnosis = run_diagnostician(global, &evidence).await?;
    let proposal = match diagnosis.outcome {
        DiagnosisOutcome::NoProposal => {
            println!(
                "no proposal tonight — the diagnostician found nothing worth changing, \
                 which is a legitimate answer and is never coerced into one"
            );
            return Ok(());
        }
        DiagnosisOutcome::Quoted { run } => {
            // Not recorded: a proposal that reproduces its sources is the
            // source's, and a candidate store is a place changes wait to be
            // applied — the one place lifted text must not sit.
            println!("proposal refused — it reproduces what it read: \"{run}\"");
            return Ok(());
        }
        DiagnosisOutcome::Proposal(p) => p,
    };

    // The same change, re-derived: the measurement is already paid for, and
    // the brief told it not to. Comparison is on the canonical spec when the
    // change parses, so spacing differences cannot re-open a rejection.
    let canonical = |change: &str| {
        parse_change(change)
            .map(|c| c.spec())
            .unwrap_or_else(|_| change.trim().to_string())
    };
    let new_spec = canonical(&proposal.change);
    if let Some(prior) = store
        .all()?
        .into_iter()
        .find(|c| canonical(&c.change) == new_spec)
    {
        println!(
            "the diagnostician re-derived `{}`, already on file as {} ({}) — nothing new tonight",
            proposal.change, prior.id, prior.status
        );
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut cand = HarnessCandidate {
        id: HarnessStore::mint_id(),
        created_at: now.clone(),
        class: proposal.class,
        change: proposal.change.clone(),
        metric: proposal.metric,
        rationale: proposal.rationale.clone(),
        evidence: evidence.brief(),
        model: Some(model.clone()),
        status: STATUS_STAGED.into(),
        measurement: None,
        resolved_at: None,
        reason: None,
    };

    println!("── candidate {} ──", cand.id);
    println!("class:     {:?}", cand.class);
    println!("change:    {}", cand.change);
    println!("predicts:  lower {:?}", cand.metric);
    println!("because:   {}", cand.rationale);
    // Printed, not only stored: the nightly's log is where a pattern of
    // mislabels would first be visible, and a candidate nobody opens is one
    // nobody sees the note on.
    if let Some(note) = &proposal.reclassified {
        println!("note:      {note}");
    }

    match proposal.class {
        ChangeClass::Security => {
            let mut reason = String::new();
            // The mislabel leads, because it is the part a reviewer most needs
            // and the part the standing warning cannot imply: it says the
            // proposer's own account of its change did not match the change.
            if let Some(note) = &proposal.reclassified {
                reason.push_str(note);
                reason.push_str(" — ");
            }
            reason.push_str(
                "security-class: never measured and never auto-applied — a loop that can \
                 argue for widening its own confinement will eventually argue well",
            );
            cand.reason = Some(reason);
            store.write(&cand)?;
            println!(
                "\nstaged for review (security-class; `mecha harness show {}`)",
                cand.id
            );
        }
        ChangeClass::Prose | ChangeClass::Architecture => {
            cand.reason = Some(format!(
                "{:?}-class changes wait for a person; prose needs the content-sensitive arm \
                 (`mecha eval --ab-config`), and architecture is always a human's call",
                proposal.class
            ));
            store.write(&cand)?;
            println!("\nstaged for review (`mecha harness show {}`)", cand.id);
        }
        ChangeClass::Config => match parse_change(&proposal.change) {
            Err(e) => {
                cand.reason = Some(format!("outside the closed override set: {e:#}"));
                store.write(&cand)?;
                println!(
                    "\nstaged for review — {}",
                    cand.reason.as_deref().unwrap_or("")
                );
            }
            Ok(change) => {
                measure(global, &store, cand, change, &model, sessions, holdout_in).await?;
            }
        },
    }
    Ok(())
}

/// Replay the corpus under both arms, judge, and dispose.
async fn measure(
    global: &GlobalOpts,
    store: &HarnessStore,
    mut cand: HarnessCandidate,
    change: mecha_core::harness::ConfigChange,
    model: &str,
    sessions: usize,
    holdout_in: u64,
) -> Result<()> {
    // The live registry, for tool specs the replay registry mirrors. Built
    // plain — the diagnostician's narrowed opts are its own, not the arms'.
    let prepared = setup::prepare(&global.clone(), false).await?;
    let (_, provider_cfg) = prepared.config.provider(global.provider.as_deref())?;

    let sessions_dir = Session::default_dir()?;
    // Seeded off the candidate id rather than the clock: re-measuring the same
    // candidate must draw the same holdout, or "confirmed on unseen work"
    // means something different every night. Printed, because a sample nobody
    // can redraw is a sample nobody can check.
    let seed = {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in cand.id.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    };
    let draw = harness_probe::draw_episodes(
        &sessions_dir,
        model,
        cand.metric,
        sessions,
        holdout_in,
        seed,
    )?;
    let unusable = draw.skipped;
    if draw.selection.is_empty() && draw.holdout.is_empty() {
        cand.reason = Some(format!(
            "no replayable sessions recorded for `{model}` — staged unmeasured"
        ));
        store.write(&cand)?;
        println!("\n{}", cand.reason.as_deref().unwrap_or(""));
        return Ok(());
    }

    eprintln!(
        "\nmeasuring `{}` over {} selected + {} held-out episode(s) × 2 arms \
         (seed {}, {} unusable session(s) passed over)",
        change.spec(),
        draw.selection.len(),
        draw.holdout.len(),
        draw.seed,
        unusable
    );

    let mut selection_pairs: Vec<Pair> = Vec::new();
    let mut holdout_pairs: Vec<Pair> = Vec::new();
    let mut diverged: Vec<String> = Vec::new();
    let mut skipped = unusable;
    let total = draw.selection.len() + draw.holdout.len();
    let slices = [("select", &draw.selection), ("hold  ", &draw.holdout)];
    let mut n = 0usize;
    for (label, preps) in slices {
        for prep in preps.iter() {
            n += 1;
            eprint!("· [{label}] {} ({}/{}) baseline…", prep.id, n, total);
            let baseline =
                match harness_probe::drive_episode(&prepared, provider_cfg, model, prep, None)
                    .await?
                {
                    Ok(arm) => arm,
                    Err(why) => {
                        eprintln!(" skipped: {why}");
                        skipped += 1;
                        continue;
                    }
                };
            eprint!(" candidate…");
            let candidate = match harness_probe::drive_episode(
                &prepared,
                provider_cfg,
                model,
                prep,
                Some(&change),
            )
            .await?
            {
                Ok(arm) => arm,
                Err(why) => {
                    eprintln!(" skipped: {why}");
                    skipped += 1;
                    continue;
                }
            };
            if baseline.diverged || candidate.diverged {
                // The recording has nothing truthful to say past a divergence;
                // stats over the tracked prefix would grade a behaviour-visible
                // change on the fraction it happened to track.
                eprintln!(" diverged — dropped");
                diverged.push(prep.id.clone());
                continue;
            }
            eprintln!(" paired");
            let pair = Pair {
                episode: prep.id.clone(),
                baseline: baseline.stats,
                candidate: candidate.stats,
            };
            if label == "select" {
                selection_pairs.push(pair);
            } else {
                holdout_pairs.push(pair);
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    if selection_pairs.is_empty() && holdout_pairs.is_empty() {
        cand.reason = Some(format!(
            "nothing measurable: {} diverged, {} skipped — a change the replay cannot hold on \
             the recording needs the eval arm instead",
            diverged.len(),
            skipped
        ));
        store.write(&cand)?;
        println!(
            "\nstaged for review — {}",
            cand.reason.as_deref().unwrap_or("")
        );
        return Ok(());
    }

    let prediction = Prediction {
        metric: cand.metric,
        rationale: cand.rationale.clone(),
    };
    let judgement = judge_drawn(cand.class, &prediction, &selection_pairs, &holdout_pairs);
    let episodes: Vec<String> = selection_pairs
        .iter()
        .chain(holdout_pairs.iter())
        .map(|p| p.episode.clone())
        .collect();
    cand.measurement = Some(Measurement::record(
        &judgement,
        model,
        now.clone(),
        episodes,
        diverged,
        skipped,
    ));

    println!(
        "\nselection  {}+ {}- {}=    holdout  {}+ {}- {}=    work {} → {}",
        judgement.selection.wins,
        judgement.selection.losses,
        judgement.selection.ties,
        judgement.holdout.wins,
        judgement.holdout.losses,
        judgement.holdout.ties,
        judgement.work_baseline,
        judgement.work_candidate,
    );

    match &judgement.disposition {
        Disposition::Accept => {
            let replaced = store.set_override(AcceptedOverride {
                key: change.key.as_str().into(),
                value: change.value.clone(),
                candidate: cand.id.clone(),
                accepted_at: now.clone(),
            })?;
            cand.status = STATUS_ACCEPTED.into();
            cand.resolved_at = Some(now);
            store.write(&cand)?;
            println!(
                "ACCEPTED — `{}` is live in the override layer (revert with \
                 `mecha harness revert {}`)",
                change.spec(),
                cand.id
            );
            if let Some(old) = replaced {
                println!(
                    "  replaced `{}={}` from candidate {} — that record still holds its evidence",
                    old.key, old.value, old.candidate
                );
            }
        }
        Disposition::Propose(reason) => {
            cand.reason = Some(reason.clone());
            store.write(&cand)?;
            println!(
                "staged for review — {reason}\n(`mecha harness show {}`)",
                cand.id
            );
        }
        Disposition::Reject(reason) => {
            cand.status = STATUS_REJECTED.into();
            cand.reason = Some(reason.clone());
            cand.resolved_at = Some(now);
            store.write(&cand)?;
            println!("rejected — {reason}");
        }
    }
    Ok(())
}

fn list(all: bool, as_json: bool) -> Result<()> {
    let store = HarnessStore::open_default()?;
    let candidates = store.all()?;
    let shown: Vec<&HarnessCandidate> = if all {
        candidates.iter().collect()
    } else {
        candidates.iter().filter(|c| c.pending()).collect()
    };
    // One shape across every reviewable store — see proposals.rs.
    if as_json {
        let rows: Vec<serde_json::Value> = shown
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "kind": format!("{:?}", c.class).to_lowercase(),
                    "title": c.change,
                    "detail": c.measurement.as_ref()
                        .map(|m| format!("{} {} | {}", m.selection, m.holdout, m.disposition))
                        .unwrap_or_else(|| c.status.clone()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if shown.is_empty() {
        if all || candidates.is_empty() {
            println!("no harness candidates on file — `mecha harness ruminate` creates them");
        } else {
            println!(
                "nothing waiting on you ({} resolved; --all shows them)",
                candidates.len()
            );
        }
        return Ok(());
    }
    for c in &shown {
        let verdict = c
            .measurement
            .as_ref()
            .map(|m| format!("  [{} {} | {}]", m.selection, m.holdout, m.disposition))
            .unwrap_or_default();
        println!(
            "{}  {:11} {:12} {}{}",
            c.id,
            format!("{:?}", c.class).to_lowercase(),
            c.status,
            c.change,
            verdict
        );
        if let Some(reason) = &c.reason {
            println!("    {reason}");
        }
    }
    if !all {
        let resolved = candidates.len() - shown.len();
        if resolved > 0 {
            println!("({resolved} resolved hidden; --all shows them)");
        }
    }
    Ok(())
}

fn show(id: &str) -> Result<()> {
    let store = HarnessStore::open_default()?;
    let c = store.find(id)?;
    println!("id:         {}", c.id);
    println!("created:    {}", c.created_at);
    println!("status:     {}", c.status);
    println!("class:      {:?}", c.class);
    println!("change:     {}", c.change);
    println!("predicts:   lower {:?}", c.metric);
    println!("because:    {}", c.rationale);
    if let Some(model) = &c.model {
        println!("model:      {model}");
    }
    if let Some(reason) = &c.reason {
        println!("reason:     {reason}");
    }
    if let Some(m) = &c.measurement {
        println!("\n── measurement ({}, {}) ──", m.model, m.measured_at);
        println!("gate:       {} {}", m.disposition, m.reason);
        println!("selection:  {}", m.selection);
        println!("holdout:    {}", m.holdout);
        println!("work:       {} → {}", m.work_baseline, m.work_candidate);
        println!("episodes:   {}", m.episodes.join(", "));
        if !m.diverged.is_empty() {
            println!(
                "diverged:   {} (dropped, not scored)",
                m.diverged.join(", ")
            );
        }
        if m.skipped > 0 {
            println!("skipped:    {}", m.skipped);
        }
    }
    println!("\n── evidence the diagnostician saw ──\n{}", c.evidence);
    Ok(())
}

fn accept(id: &str) -> Result<()> {
    let store = HarnessStore::open_default()?;
    let mut c = store.find(id)?;
    anyhow::ensure!(
        c.pending(),
        "{} is `{}`, not staged — only a staged candidate can be accepted",
        c.id,
        c.status
    );
    let now = chrono::Utc::now().to_rfc3339();
    match (c.class, parse_change(&c.change)) {
        (ChangeClass::Config, Ok(change)) => {
            store.set_override(AcceptedOverride {
                key: change.key.as_str().into(),
                value: change.value.clone(),
                candidate: c.id.clone(),
                accepted_at: now.clone(),
            })?;
            println!(
                "`{}` is live in the override layer (revert with `mecha harness revert {}`)",
                change.spec(),
                c.id
            );
        }
        _ => {
            println!(
                "marked accepted — nothing applied mechanically: a {:?}-class change is yours \
                 to make, and this record is the decision, not the deed",
                c.class
            );
        }
    }
    c.status = STATUS_ACCEPTED.into();
    c.resolved_at = Some(now);
    store.write(&c)?;
    Ok(())
}

fn reject(id: &str, reason: Option<String>) -> Result<()> {
    let store = HarnessStore::open_default()?;
    let mut c = store.find(id)?;
    anyhow::ensure!(
        c.pending(),
        "{} is `{}`, not staged — only a staged candidate can be rejected",
        c.id,
        c.status
    );
    c.status = STATUS_REJECTED.into();
    c.resolved_at = Some(chrono::Utc::now().to_rfc3339());
    c.reason = reason.or(c.reason).or(Some("rejected by hand".into()));
    store.write(&c)?;
    println!("{} rejected — the record stays as evidence", c.id);
    Ok(())
}

fn revert(id: &str) -> Result<()> {
    let store = HarnessStore::open_default()?;
    // A bare override key reverts whatever holds that key; an id reverts one
    // candidate's acceptance.
    if mecha_core::harness::OverrideKey::parse(id).is_some() {
        let Some(removed) = store.remove_override(id)? else {
            println!("no active override on `{id}`");
            return Ok(());
        };
        mark_reverted(&store, &removed.candidate)?;
        println!(
            "removed `{}={}` — config returns to your own layers",
            removed.key, removed.value
        );
        return Ok(());
    }
    let c = store.find(id)?;
    anyhow::ensure!(
        c.status == STATUS_ACCEPTED,
        "{} is `{}` — only an accepted candidate has an override to revert",
        c.id,
        c.status
    );
    let change = parse_change(&c.change)
        .with_context(|| format!("{} does not carry an applicable config change", c.id))?;
    match store.remove_override(change.key.as_str())? {
        Some(removed) if removed.candidate == c.id => {
            println!(
                "removed `{}={}` — config returns to your own layers",
                removed.key, removed.value
            );
        }
        Some(removed) => {
            // A later candidate replaced this key: put its override back and
            // leave it alone — reverting B must not silently remove A.
            store.set_override(removed.clone())?;
            anyhow::bail!(
                "the `{}` override now belongs to candidate {} — revert that one instead",
                removed.key,
                removed.candidate
            );
        }
        None => println!(
            "no active override on `{}` — marking the record anyway",
            change.key.as_str()
        ),
    }
    mark_reverted(&store, &c.id)?;
    Ok(())
}

fn mark_reverted(store: &HarnessStore, id: &str) -> Result<()> {
    if let Ok(mut c) = store.find(id) {
        c.status = STATUS_REVERTED.into();
        c.resolved_at = Some(chrono::Utc::now().to_rfc3339());
        store.write(&c)?;
    }
    Ok(())
}

fn overrides() -> Result<()> {
    let store = HarnessStore::open_default()?;
    let all = store.overrides()?;
    if all.is_empty() {
        println!("no active harness overrides — config is exactly what your files say");
        return Ok(());
    }
    println!("active overrides (your config files still win where they name a key):");
    for ov in all {
        println!(
            "  {} = {}    from {} at {}",
            ov.key, ov.value, ov.candidate, ov.accepted_at
        );
    }
    Ok(())
}
