//! `mecha learn --compare-sources` — the reflector's lessons against the
//! appraisal's, on the same interventions (`docs/APPRAISAL-WIRING-DESIGN.md`
//! row 2e-1, R25's gate for 2a-4). The pairing, the arms and the report are
//! `mecha_core::lesson_source`'s; this file drives and stores.
//!
//! **Shadow, measurement only.** A lesson from either source reaches one
//! probe arm's system prompt and nothing else. The pass reads the learning
//! store and never writes it — no rule, no proposal, no validation-ledger
//! row (a ledger row charges rule ids, and neither lesson is a rule). Its
//! one write is a 1g comparison per intervention, through the store's own
//! provenance door.
//!
//! **What it costs, and who waits** — `sessions compare`'s rules, which
//! this reuses: one background seat per intervention (`permit.rs`), held for
//! its three arms and given back before the next, waited on for up to five
//! minutes before the rest of the pass is deferred and counted; the
//! owner's reserved seat is never touched. Each arm is the validation
//! probe as `mecha validate` drives it (`probe::drive_arm`: branched at the
//! intervention, replayed under `Stop`, the recording's own turn ceiling),
//! so the three arms run under one provider config — one seed — and a
//! verdict here means what a validation verdict means. Interventions are
//! drawn uniformly with a printed seed; one whose lessons as they stand
//! are already on record under this model is not driven again.
//!
//! **Only a model on this machine** (R29): every arm resubmits a recorded
//! transcript, so a provider whose endpoint is not loopback refuses the pass
//! before anything is read.

use crate::probe::{self, StoredTally};
use crate::setup::Prepared;
use anyhow::{Context, Result};
use mecha_core::comparison::{Arm, ComparisonStore, Kind, Outcome, Refusal};
use mecha_core::learning::{LearningStore, Reflexion};
use mecha_core::lesson_source::{self, Exclusion, Pair, Report, Sources};
use mecha_core::session::Session;
use std::collections::BTreeMap;

/// What a pass found, drove, skipped and stored — each way of producing no
/// comparison counted apart.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Tally {
    pub reflections_read: usize,
    /// Torn lines in the reflection store: a floor on every count below.
    pub reflections_skipped: usize,
    /// Appraisals withheld by the clean door, and torn appraisal lines.
    pub appraisals_withheld: usize,
    pub appraisals_skipped: usize,
    /// Interventions both sources may speak to.
    pub eligible: usize,
    /// Every reflection that makes no pair, by why.
    pub excluded: BTreeMap<Exclusion, usize>,
    /// Eligible, and their lessons as they stand are on record already.
    pub already_measured: usize,
    /// The pool the draw was over.
    pub drawable: usize,
    /// Interventions whose arms were driven — each a seat and a budget unit
    /// paid — and the arms (the model runs paid).
    pub driven: usize,
    pub arms_driven: usize,
    /// Arms not driven because an earlier arm at the same intervention
    /// carried the same prompt; its outcome is recorded for both.
    pub arms_shared: usize,
    /// Of `driven`, the ones lost to an arm that could not be driven.
    pub drive_failed: usize,
    /// Drawn and refused before any seat or budget was spent.
    pub unavailable: usize,
    pub not_clean: usize,
    pub surface_unreadable: usize,
    pub over_budget: usize,
    pub no_seat: usize,
    pub stored: StoredTally,
}

/// The options the pass takes.
pub struct Options {
    pub interventions: usize,
    pub seed: Option<u64>,
    pub json: bool,
}

/// Today's day number, as `sessions compare` seeds its draw.
fn default_seed() -> u64 {
    chrono::Utc::now().timestamp().div_euclid(86_400) as u64
}

/// The two stores the pairing reads, and what reading them skipped.
struct Read {
    reflections: Vec<Reflexion>,
    reflections_skipped: usize,
    clean: mecha_core::appraisal_store::CleanRead,
    on_record: std::collections::BTreeSet<String>,
    appraisals_skipped: usize,
}

/// Read the learning store's reflections and the appraisal store — the
/// clean door for lessons, ids only for the rest. A store that does not
/// exist is empty; one that cannot be read is an `Err`.
fn read_sources() -> Result<Read> {
    let (reflections, reflections_skipped) = match LearningStore::open_existing_default() {
        Some(store) => store
            .reflexions_counting()
            .context("reading the reflections")?,
        None => (Vec::new(), 0),
    };
    let (clean, on_record, appraisals_skipped) =
        match mecha_core::appraisal_store::AppraisalStore::open_existing_default() {
            Some(store) => {
                let clean = store.clean().context("reading the text appraisals")?;
                let (ids, skipped) = store
                    .sessions_on_record()
                    .context("reading the text appraisals")?;
                (clean, ids, skipped)
            }
            None => Default::default(),
        };
    Ok(Read {
        reflections,
        reflections_skipped,
        clean,
        on_record,
        appraisals_skipped,
    })
}

pub async fn run(global: &crate::GlobalOpts, opts: Options) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    if !mecha_core::pointwise::on_this_machine(provider_cfg.base_url.as_deref()) {
        anyhow::bail!(
            "provider `{provider_name}` is not on this machine ({}); comparing lesson sources \
             resubmits recorded transcripts, so it runs on the local model only (R29)",
            provider_cfg
                .base_url
                .as_deref()
                .unwrap_or("its default endpoint")
        );
    }
    let model = global
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| {
            mecha_core::provider::build(provider_cfg)
                .map(|p| p.default_model().to_string())
                .unwrap_or_default()
        });
    let seed = opts.seed.unwrap_or_else(default_seed);

    // Opened before anything is driven, so a store that cannot be created
    // fails the pass before it pays for a verdict (1g's rule).
    let store = ComparisonStore::open_default()?;
    let on_record = store.comparisons()?;
    let read = read_sources()?;
    let sources = Sources::new(&read.clean, read.on_record.clone());

    let mut tally = Tally {
        reflections_read: read.reflections.len(),
        reflections_skipped: read.reflections_skipped,
        appraisals_withheld: read.clean.withheld,
        appraisals_skipped: read.appraisals_skipped,
        ..Tally::default()
    };
    let mut pool: Vec<Pair<'_>> = Vec::new();
    for r in &read.reflections {
        match sources.pair(r) {
            Err(why) => *tally.excluded.entry(why).or_default() += 1,
            Ok(pair) => {
                tally.eligible += 1;
                if lesson_source::on_record(&on_record, &pair, &model).is_some() {
                    tally.already_measured += 1;
                } else {
                    pool.push(pair);
                }
            }
        }
    }
    tally.drawable = pool.len();
    pool.sort_by(|a, b| a.reflection.id.cmp(&b.reflection.id));
    let drawn = mecha_core::sample::shuffled(pool, seed);
    if !opts.json {
        eprintln!(
            "comparing lesson sources at up to {} of {} intervention(s) at seed {seed} with \
             {model} ({provider_name})",
            opts.interventions.min(drawn.len()),
            drawn.len()
        );
    }

    let sessions_dir = Session::default_dir()?;
    let mut unavailable: BTreeMap<Option<String>, usize> = BTreeMap::new();
    let mut budget = opts.interventions;
    let mut prepared: Option<Prepared> = None;
    let seats = crate::commands::tasks::permits()?;
    let mut seats_gone = false;
    for pair in &drawn {
        let r = pair.reflection;
        let region = lesson_source::region_of(r.situation.as_ref());
        let mut refuse = |why: &str, tally: &mut Tally| {
            eprintln!("· {} {} ({}): {why}", r.session_id, r.trigger, r.id);
            tally.unavailable += 1;
            *unavailable.entry(region.clone()).or_default() += 1;
        };
        if budget == 0 {
            tally.over_budget += 1;
            continue;
        }
        if seats_gone {
            tally.no_seat += 1;
            continue;
        }
        let prep = match probe::prepare_probe(&sessions_dir, r)? {
            Ok(prep) => prep,
            Err(why) => {
                refuse(&why, &mut tally);
                continue;
            }
        };
        // The store's rule, asked before a seat is taken or an arm driven.
        if let Err(refusal) = prep.provenance().admit() {
            match refusal {
                Refusal::NotClean => tally.not_clean += 1,
                Refusal::SurfaceUnreadable => tally.surface_unreadable += 1,
            }
            refuse(
                match refusal {
                    Refusal::NotClean => "the session is not clean at its end",
                    Refusal::SurfaceUnreadable => "the recorded tool surface is not readable",
                },
                &mut tally,
            );
            continue;
        }
        if prepared.is_none() {
            prepared = Some(crate::setup::prepare(global, false).await?);
        }
        let live = prepared.as_ref().expect("prepared above");
        let lost = prep.lost_recorded_tools(live.agent.registry());
        if !lost.is_empty() {
            refuse(
                &format!(
                    "{} recorded tool(s) are gone from this machine's surface",
                    lost.len()
                ),
                &mut tally,
            );
            continue;
        }
        let Some(arms) = pair.arms() else {
            // `Sources::pair` admits only pairs with a lesson on each side.
            refuse("a source has no lesson to carry", &mut tally);
            continue;
        };
        let what = format!("compare lesson sources {}", r.session_id);
        let Some(_seat) = crate::pointwise_pass::take_seat(&seats, &what).await? else {
            eprintln!("the background seats stayed held; deferring the rest of this pass");
            seats_gone = true;
            tally.no_seat += 1;
            continue;
        };
        budget -= 1;
        tally.driven += 1;
        let mut driven: Vec<(Option<String>, Outcome)> = Vec::new();
        let mut out = Vec::new();
        let mut failed = None;
        for lesson_source::LessonArm {
            role,
            policy,
            block,
        } in arms
        {
            // Two arms under one prompt are one arm: drive it once and
            // record its outcome for both, rather than read noise between
            // two samples of the same prompt as a difference in lessons.
            if let Some((_, o)) = driven.iter().find(|(b, _)| *b == block) {
                tally.arms_shared += 1;
                out.push(Arm::new(role, policy, *o));
                continue;
            }
            tally.arms_driven += 1;
            match probe::drive_arm(
                live,
                provider_cfg,
                &model,
                &prep,
                prep.system_with(block.as_deref()),
            )
            .await?
            {
                Ok(v) => {
                    let o = Outcome::from(&v);
                    driven.push((block, o));
                    out.push(Arm::new(role, policy, o));
                }
                Err(why) => {
                    failed = Some(why);
                    break;
                }
            }
        }
        if let Some(why) = failed {
            // Paid for, so `drive_failed` rather than `unavailable` in the
            // pass's tally; still unavailable in the region's report, which
            // counts every drawn intervention that left no comparison.
            eprintln!(
                "· {} {} ({}): an arm could not be driven: {why}",
                r.session_id, r.trigger, r.id
            );
            tally.drive_failed += 1;
            *unavailable.entry(region.clone()).or_default() += 1;
            continue;
        }
        let mut comparison =
            prep.comparison(Kind::LessonSource, r.situation.clone(), out, &model)?;
        comparison.pointers.reflection_id = Some(r.id.clone());
        comparison.pointers.appraisal_id = Some(pair.appraisal.id.clone());
        probe::store_comparison(&store, prep.provenance(), &comparison, &mut tally.stored)?;
    }

    // Re-read from the stores: the report is what they hold, never this
    // pass's memory.
    let report = lesson_source::report(
        &read.reflections,
        &sources,
        &store.comparisons()?,
        Some(&model),
        Some(&unavailable),
    );
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "seed": seed,
                "interventions": opts.interventions,
                "model": model,
                "pass": tally,
                "report": report_json(&report),
            }))?
        );
    } else {
        print_tally(&tally, seed);
        for line in report_lines(&report) {
            println!("{line}");
        }
    }
    Ok(())
}

fn print_tally(t: &Tally, seed: u64) {
    println!(
        "lesson sources compared (seed {seed}): {} reflection(s) read{} · {} eligible · {} \
         already measured as they stand",
        t.reflections_read,
        if t.reflections_skipped > 0 {
            format!(
                " ({} torn line(s) skipped, so these are floors)",
                t.reflections_skipped
            )
        } else {
            String::new()
        },
        t.eligible,
        t.already_measured
    );
    println!(
        "  drawn from {}: {} driven ({} arm(s) run, {} shared an identical prompt; {} lost to an \
         arm that could not be driven)",
        t.drawable, t.driven, t.arms_driven, t.arms_shared, t.drive_failed
    );
    println!(
        "  skipped: {} unavailable ({} not clean at the session's end, {} with no readable tool \
         surface) · {} over budget · {} with no free seat",
        t.unavailable, t.not_clean, t.surface_unreadable, t.over_budget, t.no_seat
    );
    if let Some(line) = t.stored.line() {
        println!("  {line}");
    }
}

/// An exclusion in words, for the readout.
fn exclusion_words(e: Exclusion) -> &'static str {
    match e {
        Exclusion::NotTraceGraded => "not trace-graded",
        Exclusion::NotARunDomain => "not a run domain",
        Exclusion::DroppedByOwner => "dropped by you",
        Exclusion::EditedByOwner => "rewritten by you",
        Exclusion::NoAppraisal => "no appraisal",
        Exclusion::CleanForReflectorOnly => "clean for the reflector only",
        Exclusion::CleanForAppraisalOnly => "clean for the appraisal only",
        Exclusion::CleanForNeither => "clean for neither",
        Exclusion::ReflectorWithoutLesson => "no reflector lesson",
        Exclusion::AppraisalWithoutLessons => "no appraisal lesson",
    }
}

fn rate_words(c: &lesson_source::SourceCounts) -> String {
    match c.rate() {
        Some(r) => format!(
            "{:.0}% ({} of {} decided)",
            r * 100.0,
            c.pass,
            c.pass + c.fail
        ),
        None => "— (nothing decided)".into(),
    }
}

/// The report as lines: a header, then per region each source's rate with
/// its counts beneath it, and the region's other counts.
pub(crate) fn report_lines(report: &Report) -> Vec<String> {
    let total = report.total();
    let mut out = vec![format!(
        "lessons by source (row 2e-1, shadow{}): {} eligible intervention(s) in {} region(s), \
         {} compared{}",
        report
            .model
            .as_deref()
            .map(|m| format!("; model {m}"))
            .unwrap_or_default(),
        total.eligible,
        report.regions.len(),
        total.compared,
        if report.other_models > 0 {
            format!(
                " · {} comparison(s) under other models not counted",
                report.other_models
            )
        } else {
            String::new()
        }
    )];
    for r in &report.regions {
        out.push(format!(
            "  {}{}",
            r.describe,
            match &r.region {
                None => String::new(),
                Some(k) if k.is_empty() => " [standing]".into(),
                Some(k) => format!(" [{k}]"),
            }
        ));
        for (name, c) in [
            ("reflector ", &r.reflector),
            ("appraisal ", &r.appraisal),
            ("rules-free", &r.rules_free),
        ] {
            out.push(format!("    {name}  {}", rate_words(c)));
            if name != "rules-free" {
                out.push(format!(
                    "                {} pass · {} fail · {} improved · {} regressed vs rules-free · \
                     {} inconclusive arm(s)",
                    c.pass, c.fail, c.improved, c.regressed, c.inconclusive
                ));
            }
        }
        let excluded: Vec<String> = r
            .excluded
            .iter()
            .map(|(why, n)| format!("{n} {}", exclusion_words(*why)))
            .collect();
        out.push(format!(
            "    {} eligible · {} compared ({} inconclusive) · {} unmeasured · {} unavailable · \
             excluded: {}",
            r.eligible,
            r.compared,
            r.inconclusive,
            r.unmeasured,
            r.unavailable
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            if excluded.is_empty() {
                "none".into()
            } else {
                excluded.join(" · ")
            }
        ));
    }
    out
}

/// The report as JSON: the struct, with each source's rate beside its
/// counts — `null` over nothing decided.
pub(crate) fn report_json(report: &Report) -> serde_json::Value {
    let mut v = serde_json::to_value(report).unwrap_or_default();
    let with_rates = |value: &mut serde_json::Value, r: &lesson_source::RegionReport| {
        for (key, counts) in [
            ("reflector", &r.reflector),
            ("appraisal", &r.appraisal),
            ("rules_free", &r.rules_free),
        ] {
            if let Some(o) = value.get_mut(key).and_then(|x| x.as_object_mut()) {
                o.insert("rate".into(), serde_json::json!(counts.rate()));
            }
        }
    };
    if let Some(regions) = v.get_mut("regions").and_then(|x| x.as_array_mut()) {
        for (value, r) in regions.iter_mut().zip(&report.regions) {
            with_rates(value, r);
        }
    }
    let total = report.total();
    let mut t = serde_json::to_value(&total).unwrap_or_default();
    with_rates(&mut t, &total);
    if let Some(o) = v.as_object_mut() {
        o.insert("total".into(), t);
    }
    v
}

/// What the stores say, read for `sessions appraise` — free: no model, no
/// seat. `Ok(None)` when no reflection and no lesson comparison exists;
/// `Err` when a store could not be read — a finding, not an empty report.
pub(crate) fn on_record() -> std::result::Result<Option<Report>, String> {
    let read = read_sources().map_err(|e| format!("{e:#}"))?;
    let rows = match ComparisonStore::open_existing_default() {
        Some(store) => store.comparisons().map_err(|e| format!("{e:#}"))?,
        None => Vec::new(),
    };
    if read.reflections.is_empty() && !rows.iter().any(|c| c.kind == Kind::LessonSource) {
        return Ok(None);
    }
    let sources = Sources::new(&read.clean, read.on_record.clone());
    Ok(Some(lesson_source::report(
        &read.reflections,
        &sources,
        &rows,
        None,
        None,
    )))
}
