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
use mecha_core::candidate::{
    judge_drawn, measurable, ChangeClass, Disposition, Pair, Prediction, MIN_MEASURABLE_RUNS,
};
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
        /// Only diagnose from sessions rooted at this path or beneath it.
        #[arg(long, value_name = "PATH")]
        from_workspace: Option<std::path::PathBuf>,
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
            from_workspace,
        } => {
            anyhow::ensure!(holdout_in >= 2, "--holdout-in must be at least 2");
            ruminate(global, sessions, days, limit, holdout_in, from_workspace).await
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
    from_workspace: Option<std::path::PathBuf>,
) -> Result<()> {
    let store = HarnessStore::open_default()?;

    // Resolved once, at the boundary, so the brief and the draw compare the
    // same path against the same recorded ones. See `resolve_workspace_filter`.
    let from_workspace = crate::commands::diagnose::resolve_workspace_filter(from_workspace)?;
    let Some((model, slice, _)) = corpus_slice(None, days, limit, from_workspace.clone())? else {
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

    // Whether the corpus can refute this prediction at all. Every metric is a
    // cost, so one that no run has any of can only tie or worsen under any
    // change — and finding that out costs a real model run per episode per
    // arm. The brief now reports the table this reads, so a diagnostician
    // choosing such a metric has been told; on 2026-08-28 it had not been, and
    // predicted a lower `cut_short` over 170 runs in which `cut_short` was
    // zero. The 37 "interrupted" it was reading are cancellations, which
    // `StopCause::cut_short` excludes on purpose.
    let no_headroom = evidence
        .metrics
        .iter()
        .find(|(m, _, _)| *m == cand.metric)
        .is_some_and(|(_, _, with)| *with == 0);
    let unrefutable = format!(
        "predicts a lower `{}`, which no run in this corpus has any of — \
         the measurement could only tie",
        cand.metric.as_str()
    );
    if no_headroom {
        println!("note:      {unrefutable}");
    }

    match proposal.class {
        ChangeClass::Security => {
            // The mislabel leads, because it is the part a reviewer most needs
            // and the part the standing warning cannot imply: it says the
            // proposer's own account of its change did not match the change.
            // Through the shared composer so this arm and the one below cannot
            // disagree about whether the note survives.
            cand.reason = Some(staged_reason(
                proposal.reclassified.as_deref(),
                "security-class: never measured and never auto-applied — a loop that can \
                 argue for widening its own confinement will eventually argue well",
            ));
            store.write(&cand)?;
            println!(
                "\nstaged for review (security-class; `mecha harness show {}`)",
                cand.id
            );
        }
        ChangeClass::Prose | ChangeClass::Architecture => {
            cand.reason = Some(staged_reason(
                proposal.reclassified.as_deref(),
                &format!(
                    "{:?}-class changes wait for a person; prose needs the content-sensitive \
                     arm (`mecha eval --ab-config`), and architecture is always a human's call",
                    proposal.class
                ),
            ));
            store.write(&cand)?;
            println!("\nstaged for review (`mecha harness show {}`)", cand.id);
        }
        ChangeClass::Config => match parse_change(&proposal.change) {
            // **Not "outside the closed override set" any more**, though it
            // said so until review caught it. `parse_proposal` reclassifies an
            // unknown key to `Architecture` before this arm is reached, so the
            // only thing that can still land here is a key this harness *does*
            // own carrying a value `parse_change` refused —
            // `a_real_knob_with_a_refused_value_is_still_a_config_change` is
            // what makes that the sole survivor. The old wording told a
            // reviewer to go looking for a key that is right there in the set.
            Err(e) => {
                cand.reason = Some(format!(
                    "a known key with a value that does not parse: {e:#}"
                ));
                store.write(&cand)?;
                println!(
                    "\nstaged for review — {}",
                    cand.reason.as_deref().unwrap_or("")
                );
            }
            Ok(change) => match measurement_verdict(slice.len(), no_headroom) {
                // Staged rather than rejected and recorded rather than
                // dropped: the change may be perfectly good and it is the
                // evidence that is missing, which is a person's call and
                // belongs in the history that stops it being re-derived.
                Verdict::CorpusTooSmall => {
                    cand.reason = Some(format!(
                        "{} recorded run(s) for `{model}`, below the {} a selection slice and \
                         a holdout need between them — staged unmeasured. A necessary \
                         condition only: the eligible pool is the replayable subset of those \
                         runs.",
                        slice.len(),
                        MIN_MEASURABLE_RUNS
                    ));
                    store.write(&cand)?;
                    println!(
                        "\nstaged unmeasured — {}",
                        cand.reason.as_deref().unwrap_or("")
                    );
                }
                // Rejected so the brief's history carries it: dropped, it
                // would be free to come back tomorrow.
                Verdict::NoHeadroom => {
                    cand.status = STATUS_REJECTED.into();
                    cand.resolved_at = Some(now.clone());
                    cand.reason = Some(format!("{unrefutable}, so it was not worth a replay"));
                    store.write(&cand)?;
                    println!("\nrejected unmeasured — {unrefutable}");
                }
                Verdict::Measure => {
                    measure(
                        global,
                        &store,
                        cand,
                        change,
                        &model,
                        DrawSpec {
                            sessions,
                            holdout_in,
                            workspace: from_workspace.as_deref(),
                        },
                    )
                    .await?;
                }
            },
        },
    }
    Ok(())
}

/// Compose the `reason` a staged candidate carries, note first.
///
/// **One composer, because forgetting the note is invisible.** The `Security`
/// arm prepended it deliberately — "the mislabel leads, because it is the part
/// a reviewer most needs" — and the `Prose | Architecture` arm overwrote
/// `reason` with the generic class text and dropped it. `HarnessCandidate` has
/// no field for the note, so `reason` is its only durable home; on stdout it
/// scrolls past, and the store is what a reviewer opens.
///
/// That silently un-did this branch's headline case, one class over. A
/// `context.auto_compact=true` proposal now reclassifies to `Architecture` and
/// reached the store reading:
///
/// > class: Architecture · reason: Architecture-class changes wait for a person…
///
/// with nothing recording that the model called it `config` and named a key
/// that has never existed anywhere in this codebase — an ordinary staged
/// change, which is exactly the appearance the reclassification exists to
/// remove.
fn staged_reason(note: Option<&str>, detail: &str) -> String {
    match note {
        Some(note) => format!("{note} — {detail}"),
        None => detail.to_string(),
    }
}

/// What happens to a `Config` proposal whose change parses, before any of it
/// is written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The corpus cannot fill both slices, so `measure` would draw nothing.
    CorpusTooSmall,
    /// The prediction names a metric no run in the corpus has any of.
    NoHeadroom,
    Measure,
}

/// Decide, as a function of the two facts and nothing else.
///
/// Split out from the arm that writes the candidate because **the ordering is
/// the policy**, and the ordering was the only part of this with no test —
/// review named these the two guarantees on this branch least measured, and it
/// was right: the writing needs a provider and a store, the decision needs
/// neither.
///
/// **Corpus size is asked first, and that is the substantive choice.** Both
/// conditions can hold at once, and they disagree about who decides:
/// `NoHeadroom` rejects, `CorpusTooSmall` stages for a person. On a corpus too
/// small to measure, "no run has any of this metric" is a statement about a
/// handful of runs and not about the harness — so rejecting on it would be
/// refusing a proposal for want of evidence, which this design calls an
/// absence of a verdict rather than a verdict. Thin evidence stages.
fn measurement_verdict(runs: usize, no_headroom: bool) -> Verdict {
    if !measurable(runs) {
        return Verdict::CorpusTooSmall;
    }
    if no_headroom {
        return Verdict::NoHeadroom;
    }
    Verdict::Measure
}

/// How a measurement's episodes are drawn.
///
/// Three fields rather than three parameters because they travel together and
/// only together: `workspace` joined them when the review found the draw
/// re-scanning every session while the diagnosis was scoped, and the argument
/// list crossed clippy's threshold in the same edit. Bundling the ones that
/// must agree is the fix; an `allow` would have been the other one.
struct DrawSpec<'a> {
    sessions: usize,
    holdout_in: u64,
    /// Scoped identically to the diagnosis, or the two halves of one night
    /// disagree about what they are talking about.
    workspace: Option<&'a std::path::Path>,
}

/// Replay the corpus under both arms, judge, and dispose.
async fn measure(
    global: &GlobalOpts,
    store: &HarnessStore,
    mut cand: HarnessCandidate,
    change: mecha_core::harness::ConfigChange,
    model: &str,
    draw_spec: DrawSpec<'_>,
) -> Result<()> {
    let DrawSpec {
        sessions,
        holdout_in,
        workspace,
    } = draw_spec;
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
        workspace,
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
         (seed {}, {} of the selection ranked by a charter line, {} unusable session(s) \
         passed over)",
        change.spec(),
        draw.selection.len(),
        draw.holdout.len(),
        draw.seed,
        draw.ranked,
        unusable
    );

    let mut selection_pairs: Vec<Pair> = Vec::new();
    let mut holdout_pairs: Vec<Pair> = Vec::new();
    let mut diverged: Vec<String> = Vec::new();
    let mut replay_caveats: Vec<String> = Vec::new();
    // See `Measurement::divergence_detail`.
    let mut divergence_detail: Vec<mecha_core::harness::Divergence> = Vec::new();
    // Recorded as each arm finishes, never after both. Review finding: the
    // push used to sit past the candidate's `Err` arm, so an episode whose
    // BASELINE left the recording and whose candidate then failed to replay
    // at all lost the baseline's reason entirely and landed in `skipped`
    // with no account of itself — the same loss this branch exists to fix,
    // one case narrower.
    macro_rules! note_divergence {
        ($arm:expr, $outcome:expr, $id:expr) => {
            if let Some(why) = &$outcome.divergence {
                divergence_detail.push(mecha_core::harness::Divergence {
                    episode: $id.clone(),
                    arm: $arm,
                    reason: why.clone(),
                });
            }
        };
    }
    let mut skipped = unusable;
    let total = draw.selection.len() + draw.holdout.len();
    // The flag decides the slice; the label only decides the column width.
    // These were one string: `if label == "select"` routed the pair, so
    // renaming the display text — `"sel"` for a narrower line — would have
    // sent every selection pair into the holdout, and the gate would have read
    // a candidate that won on nothing and confirmed on everything.
    let slices = [
        (true, "select", &draw.selection),
        (false, "hold  ", &draw.holdout),
    ];
    let mut n = 0usize;
    for (is_selection, label, preps) in slices {
        for prep in preps.iter() {
            n += 1;
            // The caveat rides the episode's own line: a later "diverged —
            // dropped" on a multi-config session is then legible as the
            // recording being unreplayable-as-one-run, not the candidate or
            // the replay machinery failing.
            let caveat = prep
                .config_caveat
                .as_deref()
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            // Into the stored record for *every* multi-config episode, not
            // only the dropped ones — found on review: an episode that pairs
            // cleanly contributes to the tally that gates acceptance, and
            // that is the more consequential place for the decider reading
            // `mecha harness show` to know the replay was compromising.
            if let Some(c) = &prep.config_caveat {
                replay_caveats.push(format!("{} — {c}", prep.id));
            }
            eprint!(
                "· [{label}] {}{caveat} ({}/{}) baseline…",
                prep.id, n, total
            );
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
            note_divergence!(mecha_core::harness::Arm::Baseline, baseline, prep.id);
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
            // Which arm left the recording, and why — beside the id, never
            // folded into it (`Measurement::diverged`'s ids stay joinable,
            // the same contract `replay_caveats` respects).
            //
            // **The distinction the bare bool destroyed.** A baseline
            // divergence says the replay is unreliable on this episode; a
            // candidate-only divergence says the change altered behaviour,
            // which is the measurement working, not failing. On 2026-09-01
            // twelve of sixteen episodes were dropped as one undifferentiated
            // pile, and the four that paired were all far below the
            // threshold the candidate moved — so the tally scored only the
            // episodes where the change provably did nothing, and reported
            // it as a thin sample rather than a censored one.
            note_divergence!(mecha_core::harness::Arm::Candidate, candidate, prep.id);
            if baseline.diverged() || candidate.diverged() {
                // The recording has nothing truthful to say past a divergence;
                // stats over the tracked prefix would grade a behaviour-visible
                // change on the fraction it happened to track.
                //
                // The caveat rides into the stored record too, not only the
                // nightly's stderr — beside the id, never folded into it:
                // `diverged` is a joinable id list by contract, and the
                // reader the caveat was written for is `mecha harness
                // show`'s (whoever decides on a staged candidate) — such a
                // divergence says something about the replay's compromise,
                // not necessarily about the change.
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
            if is_selection {
                selection_pairs.push(pair);
            } else {
                holdout_pairs.push(pair);
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    if selection_pairs.is_empty() && holdout_pairs.is_empty() {
        // The caveats ride in the reason here — found on review: this early
        // return stores no Measurement, which dropped them in exactly the
        // all-diverged case they exist to explain (a pool of multi-config
        // episodes that all diverged says "replay compromise", not "the
        // change cannot hold").
        let caveats = if replay_caveats.is_empty() {
            String::new()
        } else {
            format!("; replay caveats: {}", replay_caveats.join(", "))
        };
        // Which arm, in the same breath. "Everything diverged" is two very
        // different findings depending on whether the BASELINE did: if it
        // did, the recording is the problem; if only the candidate did, the
        // change moves behaviour on every episode drawn, which is a result
        // about the change and not an absence of one.
        let arms = mecha_core::harness::Divergence::arms_summary(&divergence_detail, skipped);
        cand.reason = Some(format!(
            "nothing measurable: {} diverged, {} skipped — a change the replay cannot hold on \
             the recording needs the eval arm instead{arms}{caveats}",
            diverged.len(),
            skipped
        ));
        // **Store the Measurement even here.** Review finding: this early
        // return wrote none, so the diverged ids, the per-arm reasons, the
        // seed and the caveats were all discarded in exactly the case they
        // exist to explain — a single sentence survived where the evidence
        // should have. The tallies are empty because nothing paired; that is
        // the honest record, and it is not the same as having no record.
        let empty = judge_drawn(
            cand.class,
            &Prediction {
                metric: cand.metric,
                rationale: cand.rationale.clone(),
            },
            &[],
            &[],
        );
        let mut record = Measurement::record(
            &empty,
            model,
            now.clone(),
            mecha_core::harness::Drawn {
                episodes: Vec::new(),
                holdout_episodes: Vec::new(),
                seed: draw.seed,
                ranked: draw.ranked,
                diverged,
                replay_caveats,
                divergence_detail,
                skipped,
            },
        );
        // **The stored reason must not be the floor text.** `judge_drawn`
        // over two empty slices yields "only 0 paired episode(s) … below the
        // floor of 8" — the sample-size framing this whole branch argues
        // misdirects, and `show` would print it directly above the arm
        // split that contradicts it. Nothing paired because every arm left
        // the recording, which is a different finding from too small a draw.
        record.reason = cand.reason.clone().unwrap_or_default();
        cand.measurement = Some(record);
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
    let holdout_episodes: Vec<String> = holdout_pairs.iter().map(|p| p.episode.clone()).collect();
    cand.measurement = Some(Measurement::record(
        &judgement,
        model,
        now.clone(),
        mecha_core::harness::Drawn {
            episodes,
            holdout_episodes,
            seed: draw.seed,
            ranked: draw.ranked,
            diverged,
            replay_caveats,
            divergence_detail,
            skipped,
        },
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
        // Named by slice, because "which episodes confirmed this" is the
        // question a reader of a measurement actually has.
        let held: std::collections::HashSet<&str> =
            m.holdout_episodes.iter().map(String::as_str).collect();
        let selected: Vec<&str> = m
            .episodes
            .iter()
            .map(String::as_str)
            .filter(|e| !held.contains(e))
            .collect();
        println!("selected:   {}", selected.join(", "));
        println!(
            "held out:   {} (seed {})",
            m.holdout_episodes.join(", "),
            m.seed
        );
        // Absent on a record from before the tiebreak: unknown, not none.
        match m.ranked {
            Some(n) => println!("ranked:     {n} of the selection by a charter line"),
            None => println!("ranked:     not recorded"),
        }
        if !m.diverged.is_empty() {
            println!(
                "diverged:   {} (dropped, not scored)",
                m.diverged.join(", ")
            );
        }
        // Its own block, like `replay caveats` below and for the same
        // reason: the ids above stay a bare joinable list, and the reader
        // deciding on this candidate gets the arm and the cause without
        // parsing them back out.
        if !m.divergence_detail.is_empty() {
            let baseline_arms =
                mecha_core::harness::Divergence::baseline_count(&m.divergence_detail);
            let candidate_arms =
                mecha_core::harness::Divergence::candidate_count(&m.divergence_detail);
            println!(
                "divergence detail ({baseline_arms} baseline-arm, {candidate_arms} \
                 candidate-arm):"
            );
            for d in &m.divergence_detail {
                println!("  {} — {} arm: {}", d.episode, d.arm.as_str(), d.reason);
            }
            // The reading, stated once rather than left to be re-derived.
            // Scoped on `skipped` as well as on the arm split: episodes lost
            // before either arm ran are not evidence that the change moved
            // anything.
            if baseline_arms == 0 && candidate_arms > 0 && m.skipped == 0 {
                println!(
                    "  → every divergence was the candidate arm: the change moved behaviour \
                     on these episodes, so they are dropped for being unscoreable, NOT for \
                     showing nothing. Check whether the episodes that DID pair are ones the \
                     change can affect at all."
                );
            }
        }
        // Its own block with its own label, not indented under `diverged:`
        // — found on review: these cover *every* episode the replay
        // compromised on, the cleanly paired ones included, and rendering
        // them as an appendix to the drops read a scored episode as a
        // dropped one (or, with nothing diverged, hung them under whatever
        // line came before).
        if !m.replay_caveats.is_empty() {
            println!("replay caveats:");
            for caveat in &m.replay_caveats {
                println!("  {caveat}");
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::candidate::MIN_MEASURABLE_RUNS as FLOOR;

    #[test]
    fn a_mislabel_survives_into_the_record_a_reviewer_actually_opens() {
        // The note was on the nightly's stdout and not in the store, so the
        // branch's own headline case — a model calling `context.auto_compact`
        // a config change when that key has never existed — reached
        // `mecha harness show` looking like an ordinary staged Architecture
        // change. `HarnessCandidate` has no field for the note; `reason` is
        // the only durable place it can live.
        let note = "proposed as `Config`, reclassified: `context.auto_compact` is not one \
                    of the 4 keys this harness can override";
        let composed = staged_reason(Some(note), "Architecture-class changes wait for a person");
        assert!(composed.starts_with(note), "the mislabel leads: {composed}");
        assert!(composed.contains("wait for a person"), "{composed}");

        // An honestly-labelled proposal carries no note and gains no
        // separator — a dangling em dash on every ordinary candidate would
        // train a reviewer to skip the position the mislabel appears in.
        let plain = staged_reason(None, "Architecture-class changes wait for a person");
        assert_eq!(plain, "Architecture-class changes wait for a person");
    }

    #[test]
    fn a_corpus_too_small_to_measure_stages_rather_than_rejecting() {
        // Both conditions hold, and they disagree about who decides. On a
        // corpus below the floor, "no run has any of this metric" describes a
        // handful of runs rather than the harness, so it must not be grounds
        // for refusing the proposal — thin evidence is an absence of a verdict,
        // not a verdict. Swapping the two checks makes this return NoHeadroom
        // and quietly converts a staging into a rejection.
        assert_eq!(
            measurement_verdict(FLOOR - 1, true),
            Verdict::CorpusTooSmall
        );
        assert_eq!(
            measurement_verdict(FLOOR - 1, false),
            Verdict::CorpusTooSmall
        );
        // The live case: the morning-briefing job, 11 runs against a floor
        // of 12.
        assert_eq!(measurement_verdict(11, false), Verdict::CorpusTooSmall);
    }

    #[test]
    fn a_prediction_the_corpus_cannot_refute_is_refused_before_it_costs_replays() {
        assert_eq!(measurement_verdict(FLOOR, true), Verdict::NoHeadroom);
        assert_eq!(measurement_verdict(236, true), Verdict::NoHeadroom);
    }

    #[test]
    fn enough_corpus_and_a_metric_with_room_is_measured() {
        assert_eq!(measurement_verdict(FLOOR, false), Verdict::Measure);
        assert_eq!(measurement_verdict(236, false), Verdict::Measure);
    }
}
