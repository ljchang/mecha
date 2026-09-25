//! `mecha sessions compare` — point-wise comparison at informative decision
//! points (`docs/APPRAISAL-WIRING-DESIGN.md` O1, row 2d-1).
//!
//! The points are found from records by `mecha_core::pointwise`, drawn
//! uniformly with a printed seed, and each is compared under up to
//! [`pointwise::ARMS_MAX`] policies — the prompt the run carried, the rules
//! deployed today for its situation, and none — driven through
//! [`probe::drive_arm_within`] a short horizon ([`pointwise::HORIZON_TURNS`])
//! from the point, and graded by the owner's recorded verdict through a
//! structural validator. Every comparison goes through
//! [`ComparisonStore::record`], the store's one provenance door.
//!
//! **What it costs, and who waits.** Nothing is driven for a point no
//! structural validator can pose; it is stored inconclusive and costs no
//! seat. A posed point holds one background seat (`permit.rs`) for its arms,
//! driven one after another, and gives it back before the next point — so
//! the owner's reserved seat is never touched and a delegation can start
//! between two points. A full pool is waited on, never pushed through; if
//! it stays full the rest of the pass is deferred and counted, which on a
//! nightly schedule means tomorrow catches up.
//!
//! **Only a model on this machine** (R29): every arm resubmits a recorded
//! transcript, so a provider whose endpoint is not loopback refuses the pass
//! before anything is read.

use crate::probe::{self, ProbePrep, StoredTally};
use crate::setup::Prepared;
use anyhow::{Context, Result};
use mecha_core::comparison::{
    Arm, CallClass, Comparison, ComparisonStore, Outcome, Pointers, Provenance, Refusal, Role,
    Validator, Verdict,
};
use mecha_core::config::ProviderConfig;
use mecha_core::outbox::OutboxItem;
use mecha_core::pointwise::{self, Locator, Point, PointKind, Policy};
use mecha_core::session::{Session, Transcript};
use mecha_core::situation::Situation;
use mecha_core::surface::SurfaceStore;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How long a point waits for a background seat before the pass defers
/// the rest, and how often it asks. A nightly pass that waits longer holds
/// the night's other stages behind it; one that does not wait at all loses
/// its points to a delegation that was about to finish.
const SEAT_WAIT: std::time::Duration = std::time::Duration::from_secs(300);
const SEAT_POLL: std::time::Duration = std::time::Duration::from_secs(10);

/// What a pass found, drove, skipped and stored — each way of producing no
/// comparison counted apart, because they call for different fixes.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Tally {
    pub sessions_read: usize,
    pub sessions_unreadable: usize,
    pub tests_hidden: usize,
    /// Every point found, by kind — before provenance, so a readout can say
    /// how much of the corpus the gate keeps out.
    pub found: BTreeMap<String, usize>,
    /// Points in a session whose recorded taint is not clean (or unknown):
    /// never drawn, since nothing from them may be stored.
    pub not_clean: usize,
    /// Points whose recorded tool surface is not readable: drawn and
    /// prepared (only the prepared point knows which surface applies), then
    /// refused before any seat or budget is spent.
    pub surface_unreadable: usize,
    /// The pool the draw was over.
    pub drawable: usize,
    /// Points whose arms were driven — each a seat and a budget unit paid —
    /// and the arms (the model runs paid). `driven` minus `drive_failed` is
    /// the comparisons offered to the store.
    pub driven: usize,
    /// Arms started — the runs paid for, including one that then failed.
    pub arms_driven: usize,
    /// Of `driven`, the points lost to an arm that could not be driven: paid
    /// for, and no comparison — a comparison with a missing arm is a failed
    /// attempt. Kept apart from `unavailable`, which is never paid for.
    pub drive_failed: usize,
    /// Points no structural validator can pose, stored inconclusive with
    /// nothing driven.
    pub unposed: usize,
    /// Already on record under the same policies and model.
    pub already_compared: usize,
    /// Every candidate policy ran the same prompt: nothing to compare.
    pub single_policy: usize,
    /// The point could not be prepared or cannot be driven here — counted
    /// before any seat or budget is spent, never evidence for any arm.
    pub unavailable: usize,
    /// Drawn after the budget ran out.
    pub over_budget: usize,
    /// Every background seat stayed held past the wait.
    pub no_seat: usize,
    pub stored: StoredTally,
    /// The derived verdicts of the comparisons this pass wrote.
    pub separated: usize,
    pub tied: usize,
    pub inconclusive: usize,
}

impl Tally {
    fn verdict(&mut self, v: Verdict) {
        match v {
            Verdict::Separated => self.separated += 1,
            Verdict::Tied => self.tied += 1,
            Verdict::Inconclusive | Verdict::Unknown => self.inconclusive += 1,
        }
    }

    fn refused(&mut self, r: Refusal) {
        match r {
            Refusal::NotClean => self.not_clean += 1,
            Refusal::SurfaceUnreadable => self.surface_unreadable += 1,
        }
    }
}

/// A point in the pool, with the transcript it lives in.
pub struct Drawable {
    pub path: PathBuf,
    pub point: Point,
}

/// The recorded surface covering message `at`: its config's tools hash and
/// the specs the surface store still holds for it.
fn surface_at(
    transcript: &Transcript,
    at: usize,
    surfaces: Option<&SurfaceStore>,
) -> (Option<String>, Vec<mecha_core::message::ToolSpec>) {
    let hash = transcript
        .config_covering(at)
        .and_then(|c| c.tools_hash.clone());
    let specs = hash
        .as_deref()
        .and_then(|h| surfaces?.load(h))
        .unwrap_or_default();
    (hash, specs)
}

/// Whether a comparison at message `at` of this transcript may be stored —
/// the store's own rule ([`Provenance::of_transcript`]), asked before
/// anything is driven so a pass never pays for a verdict it cannot keep.
fn provenance_at(
    transcript: &Transcript,
    at: usize,
    surfaces: Option<&SurfaceStore>,
) -> Provenance {
    let (hash, specs) = surface_at(transcript, at, surfaces);
    Provenance::of_transcript(transcript, hash.as_deref(), &specs)
}

/// Walk the sessions `scan` admits and gather every point in a clean
/// session with a readable surface; the rest are counted, not drawn.
///
/// `model`, when given, keeps only sessions recorded under it — a harness
/// candidate's point pool is scoped as its whole-session draw is
/// (`harness_probe::draw_episodes`: model-matched, workspace-scoped,
/// recency-bounded), or the two halves of one measurement read different
/// corpora (found on review).
pub fn collect(
    dir: &Path,
    scan: &mecha_core::runlog::Scan,
    limit: Option<usize>,
    model: Option<&str>,
    drafts: &[OutboxItem],
    surfaces: Option<&SurfaceStore>,
    tally: &mut Tally,
) -> Result<Vec<Drawable>> {
    let (listed, unreadable) = Session::list_counting(dir)?;
    tally.sessions_unreadable += unreadable;
    let mut pool = Vec::new();
    for (meta, path) in listed {
        if limit.is_some_and(|n| tally.sessions_read >= n) {
            break;
        }
        if model.is_some_and(|m| meta.model != m) {
            continue;
        }
        if !scan.admits(&meta) {
            if scan.hides_test(&meta) {
                tally.tests_hidden += 1;
            }
            continue;
        }
        let transcript = match Session::read(&path) {
            Ok(t) => t,
            Err(_) => {
                tally.sessions_unreadable += 1;
                continue;
            }
        };
        tally.sessions_read += 1;
        let mine: Vec<&OutboxItem> = drafts
            .iter()
            .filter(|i| i.session_id.as_deref() == Some(meta.id.as_str()))
            .collect();
        for point in pointwise::points_in(&meta.id, &transcript.convo.messages, &mine) {
            *tally
                .found
                .entry(point.kind.as_str().to_string())
                .or_default() += 1;
            // The taint half of the store's rule, asked here so a session
            // nothing may be kept from never takes a draw. The surface half
            // is asked of the prepared point instead (`Plan::provenance`):
            // an artifact probe rebuilds its surface from the owner's
            // fixture rather than the surface store, and only its
            // preparation knows which applies.
            if provenance_at(&transcript, point.message_index, surfaces).origin
                != mecha_core::learning::Origin::Clean
            {
                tally.not_clean += 1;
                continue;
            }
            pool.push(Drawable {
                path: path.clone(),
                point,
            });
        }
    }
    tally.drawable = pool.len();
    Ok(pool)
}

/// What one drawn point needs next.
pub enum Plan {
    /// A structural validator can pose it: drive these arms.
    Posed(Box<ProbePrep>),
    /// None can: store this inconclusive comparison, drive nothing.
    Unposed(Box<(Comparison, Provenance)>),
    /// It could not be prepared; the reason is for the person reading.
    Unavailable(String),
}

/// Whether a check point can be posed: an **owner-bound** criterion failed
/// and the recording carries the owner's artifact case, so the artifact
/// repeat against pinned gold decides. A declared check is the agent's own
/// (R11: one-sided until the owner confirms it), and a branch that executes
/// nothing cannot re-run a check against a workspace it never wrote.
fn check_is_posed(transcript: &Transcript, point: &Point) -> bool {
    let Locator::Step { step } = point.locator else {
        return false;
    };
    let criterion = step_at(transcript, point.message_index, step)
        .is_some_and(|s| s.criterion.is_some() && s.learnable_failure());
    criterion
        && transcript
            .config_covering(point.message_index)
            .is_some_and(|c| c.mismatch_case.is_some())
}

fn step_at(
    transcript: &Transcript,
    at: usize,
    step: usize,
) -> Option<&mecha_core::planning::StepFeedback> {
    transcript
        .convo
        .messages
        .get(at)?
        .planning
        .as_ref()?
        .steps
        .get(step)
}

/// Prepare one drawn point: load its recording, and either branch it for
/// driving or build the inconclusive comparison it gets instead.
pub fn plan(
    point: &Point,
    path: &Path,
    drafts: &BTreeMap<String, &OutboxItem>,
    surfaces: Option<&SurfaceStore>,
) -> Result<Plan> {
    let prepared = match (&point.kind, &point.locator) {
        (PointKind::Steer | PointKind::Denial, Locator::Intervention { text }) => {
            probe::prepare_intervention_at(path, point.kind.trigger(), text, point.message_index)?
        }
        (PointKind::EditedDraft | PointKind::RejectedDraft, Locator::Draft { item_id }) => {
            let Some(item) = drafts.get(item_id) else {
                return Ok(Plan::Unavailable(format!("outbox item {item_id} is gone")));
            };
            probe::prepare_draft_in(path, item)?
        }
        (PointKind::FailedCheck | PointKind::Surprise, Locator::Step { step }) => {
            let transcript = match Session::read(path) {
                Ok(t) => t,
                Err(e) => return Ok(Plan::Unavailable(format!("session unreadable: {e:#}"))),
            };
            if point.kind == PointKind::FailedCheck && check_is_posed(&transcript, point) {
                let Some(expected) = step_at(&transcript, point.message_index, *step) else {
                    return Ok(Plan::Unavailable("the step feedback is gone".into()));
                };
                let id = format!("point:{}:{}:{step}", point.session_id, point.message_index);
                probe::prepare_check_in(path, expected, &id)
            } else {
                return Ok(Plan::Unposed(Box::new(unposed(
                    &transcript,
                    point,
                    *step,
                    surfaces,
                ))));
            }
        }
        _ => return Ok(Plan::Unavailable("the point does not fit its kind".into())),
    };
    Ok(match prepared {
        Ok(prep) => Plan::Posed(Box::new(prep)),
        Err(why) => Plan::Unavailable(why),
    })
}

/// The inconclusive comparison a point no structural validator can pose is
/// stored as: its keys and pointers, **no arms** (none was driven, and an
/// arm outcome nobody measured is not recorded), `Validator::Unposed`, and
/// so the verdict `Verdict::of` derives from no arms — inconclusive. No
/// model drove anything, so `model` is empty.
fn unposed(
    transcript: &Transcript,
    point: &Point,
    step: usize,
    surfaces: Option<&SurfaceStore>,
) -> (Comparison, Provenance) {
    let at = point.message_index;
    let (hash, specs) = surface_at(transcript, at, surfaces);
    let config = transcript.config_covering(at);
    let messages = &transcript.convo.messages;
    let call_id = step_at(transcript, at, step).and_then(|s| s.call_id.clone());
    let call = call_id.as_deref().and_then(|id| {
        messages
            .iter()
            .flat_map(|m| m.tool_uses())
            .find(|(cid, _, _)| *cid == id)
            .and_then(|(_, name, input)| CallClass::of(name, input, &specs))
    });
    let comparison = Comparison::new(
        point.kind.comparison_kind(),
        // `ProbePrep::situation_at`'s construction, from the same covering
        // record: the surface, workspace and goal the run was matched on.
        config.map(|c| {
            Situation::recorded(
                &point.tools_before,
                point.kind.trigger(),
                c.rules_surface,
                c.rules_workspace.as_deref(),
            )
            .toward(c.rules_goal.clone())
        }),
        transcript.convo.goal_anchor.as_ref().map(|g| g.goal_kind()),
        call,
        Vec::new(),
        Validator::Unposed,
        Pointers {
            session_id: transcript.meta.id.clone(),
            message_index: Some(at),
            call_index: call_id
                .as_deref()
                .and_then(|id| pointwise::call_index_of(messages, id)),
            tools_hash: hash.clone(),
            ..Pointers::default()
        },
        "",
    );
    (
        comparison,
        Provenance::of_transcript(transcript, hash.as_deref(), &specs),
    )
}

/// The policies a posed point compares: the prompt the run carried, the
/// rules deployed today for the run's situation (when any ride there), and
/// none — distinct by prompt, at most `ARMS_MAX`.
pub fn policies(prep: &ProbePrep, deployed: Option<String>) -> Vec<Policy> {
    let mut candidates = vec![Policy {
        role: Role::WithoutIntervention,
        policy: prep.recorded_rules_hash(),
        system: prep.system_as_recorded(),
    }];
    if let Some(block) = deployed {
        candidates.push(Policy {
            role: Role::Rules,
            policy: Some(mecha_core::learning::rules_hash(&block)),
            system: prep.system_with(Some(&block)),
        });
    }
    candidates.push(Policy {
        role: Role::RulesFree,
        policy: Arm::no_block(),
        system: prep.system_with(None),
    });
    pointwise::distinct_policies(candidates)
}

/// The comparison a posed point writes, over `arms` — keyed on the point's
/// recorded situation, its verdict derived from the arms.
pub fn point_comparison(
    prep: &ProbePrep,
    point: &Point,
    arms: Vec<Arm>,
    model: &str,
) -> Result<Comparison> {
    prep.comparison(
        point.kind.comparison_kind(),
        Some(prep.situation_at(&point.tools_before, point.kind.trigger())),
        arms,
        model,
    )
}

/// Take a background seat, waiting up to [`SEAT_WAIT`]. `None` when the
/// pool stayed full.
async fn take_seat(
    pool: &mecha_core::permit::Permits,
    what: &str,
) -> Result<Option<mecha_core::permit::Held>> {
    let deadline = std::time::Instant::now() + SEAT_WAIT;
    let mut said = false;
    loop {
        match pool.take(what)? {
            Ok(held) => return Ok(Some(held)),
            Err(holders) => {
                if std::time::Instant::now() >= deadline {
                    return Ok(None);
                }
                if !said {
                    said = true;
                    eprintln!(
                        "all {} background model seat(s) are held ({}); waiting",
                        pool.capacity(),
                        holders
                            .iter()
                            .map(|p| p.what.as_deref().unwrap_or("unnamed"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                tokio::time::sleep(SEAT_POLL).await;
            }
        }
    }
}

/// The options `mecha sessions compare` takes.
pub struct Options {
    pub points: usize,
    pub seed: Option<u64>,
    pub days: Option<i64>,
    pub limit: Option<usize>,
    pub kind: Option<mecha_core::session::SessionKind>,
    pub include_tests: bool,
    pub json: bool,
}

/// Today's day number: the default seed, so each night draws afresh and
/// any night can be redrawn by passing the printed seed.
fn default_seed() -> u64 {
    chrono::Utc::now().timestamp().div_euclid(86_400) as u64
}

pub async fn run(global: &crate::GlobalOpts, opts: Options) -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine the working directory")?;
    let cfg = mecha_core::config::Config::load(&cwd)?;
    let (provider_name, provider_cfg) = cfg.provider(global.provider.as_deref())?;
    if !pointwise::on_this_machine(provider_cfg.base_url.as_deref()) {
        anyhow::bail!(
            "provider `{provider_name}` is not on this machine ({}); a point-wise comparison \
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
    // fails the pass before it pays for a verdict (1g's rule); read once for
    // the already-compared check, and grown as this pass writes.
    let store = ComparisonStore::open_default()?;
    let mut on_record = store.comparisons()?;

    // Best-effort, like every reader over the outbox: unreadable costs the
    // draft points and says so.
    let (drafts, outbox_read) = match mecha_core::outbox::OutboxStore::open_existing_default() {
        None => (Vec::new(), true),
        Some(o) => match o.items_counting() {
            Ok((items, skipped)) => (items, skipped == 0),
            Err(_) => (Vec::new(), false),
        },
    };
    let by_id: BTreeMap<String, &OutboxItem> = drafts.iter().map(|i| (i.id.clone(), i)).collect();
    let surfaces = SurfaceStore::open_default();

    let scan = mecha_core::runlog::Scan {
        max_sessions: None,
        since: opts
            .days
            .map(|d| chrono::Utc::now() - chrono::Duration::days(d)),
        workspace: None,
        kind: opts.kind,
        include_tests: opts.include_tests,
        include_experiments: mecha_core::experiment::in_experiment_home(),
    };
    let mut tally = Tally::default();
    let pool = collect(
        &Session::default_dir()?,
        &scan,
        opts.limit,
        None,
        &drafts,
        surfaces.as_ref(),
        &mut tally,
    )?;
    let drawn = pointwise::draw(pool, seed, |d| &d.point);
    if !opts.json {
        eprintln!(
            "comparing up to {} of {} drawable point(s) at seed {seed} with {model} ({provider_name})",
            opts.points.min(drawn.len()),
            drawn.len()
        );
    }

    let mut budget = opts.points;
    let mut prepared: Option<Prepared> = None;
    let mut rules: Option<Option<crate::commands::validate::RuleSurface>> = None;
    let seats = crate::commands::tasks::permits()?;
    let mut seats_gone = false;
    for Drawable { path, point } in &drawn {
        // A steer, a denial and a draft are always posed, so once nothing
        // more can be driven they are counted without reading their
        // transcripts again; a check or a surprise may be unposed and free,
        // so it is still prepared.
        let always_posed = matches!(
            point.kind,
            PointKind::Steer
                | PointKind::Denial
                | PointKind::EditedDraft
                | PointKind::RejectedDraft
        );
        if always_posed && budget == 0 {
            tally.over_budget += 1;
            continue;
        }
        if always_posed && seats_gone {
            tally.no_seat += 1;
            continue;
        }
        let planned = match plan(point, path, &by_id, surfaces.as_ref())? {
            Plan::Unavailable(why) => {
                eprintln!(
                    "· {} {} at message {}: {why}",
                    point.session_id,
                    point.kind.as_str(),
                    point.message_index
                );
                tally.unavailable += 1;
                continue;
            }
            Plan::Unposed(boxed) => {
                let (comparison, provenance) = *boxed;
                if let Err(r) = provenance.admit() {
                    tally.refused(r);
                    continue;
                }
                if pointwise::already_compared(&on_record, &comparison) {
                    tally.already_compared += 1;
                    continue;
                }
                tally.unposed += 1;
                store_one(&store, provenance, comparison, &mut on_record, &mut tally)?;
                continue;
            }
            Plan::Posed(prep) => prep,
        };
        // The store's rule, asked before a seat is taken or an arm driven:
        // a verdict it would refuse is not worth paying for.
        if let Err(r) = planned.provenance().admit() {
            tally.refused(r);
            continue;
        }
        if budget == 0 {
            tally.over_budget += 1;
            continue;
        }
        if seats_gone {
            tally.no_seat += 1;
            continue;
        }
        if prepared.is_none() {
            prepared = Some(crate::setup::prepare(global, false).await?);
        }
        let live = prepared.as_ref().expect("prepared above");
        // Both asked before the budget or a seat is spent: neither answer
        // can change by driving.
        if let Some(why) = planned.unrunnable_under(live) {
            eprintln!("· {} {}: {why}", point.session_id, point.kind.as_str());
            tally.unavailable += 1;
            continue;
        }
        let lost = planned.lost_recorded_tools(live.agent.registry());
        if !lost.is_empty() {
            eprintln!(
                "· {} {}: {} recorded tool(s) are gone from this machine's surface",
                point.session_id,
                point.kind.as_str(),
                lost.len()
            );
            tally.unavailable += 1;
            continue;
        }
        if rules.is_none() {
            rules = Some(
                match mecha_core::learning::LearningStore::open_existing_default() {
                    Some(s) => Some(crate::commands::validate::RuleSurface::load(&s)?),
                    None => None,
                },
            );
        }
        let deployed = rules.as_ref().and_then(|r| r.as_ref()).and_then(|r| {
            let run = planned.situation();
            r.block_with(&r.carried(&run), &run)
        });
        let arms_planned = policies(&planned, deployed);
        if arms_planned.len() < 2 {
            tally.single_policy += 1;
            continue;
        }
        let skeleton = point_comparison(
            &planned,
            point,
            arms_planned
                .iter()
                .map(|p| Arm::new(p.role, p.policy.clone(), Outcome::Unknown))
                .collect(),
            &model,
        )?;
        if pointwise::already_compared(&on_record, &skeleton) {
            tally.already_compared += 1;
            continue;
        }
        let what = format!("compare {} {}", point.kind.as_str(), point.session_id);
        let Some(_seat) = take_seat(&seats, &what).await? else {
            eprintln!("the background seats stayed held; deferring the rest of this pass");
            seats_gone = true;
            tally.no_seat += 1;
            continue;
        };
        budget -= 1;
        tally.driven += 1;
        match drive_point(
            live,
            provider_cfg,
            &model,
            &planned,
            &arms_planned,
            &mut tally,
        )
        .await?
        {
            Ok(arms) => {
                let comparison = point_comparison(&planned, point, arms, &model)?;
                store_one(
                    &store,
                    planned.provenance(),
                    comparison,
                    &mut on_record,
                    &mut tally,
                )?;
            }
            Err(why) => {
                eprintln!("· {} {}: {why}", point.session_id, point.kind.as_str());
                tally.drive_failed += 1;
            }
        }
    }

    let on_record_summary = crate::commands::sessions::comparisons_on_record();
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "seed": seed,
                "points": opts.points,
                "horizon_turns": pointwise::HORIZON_TURNS,
                "arms_max": pointwise::ARMS_MAX,
                "model": model,
                "outbox_read": outbox_read,
                "pass": tally,
                "comparisons": crate::commands::sessions::comparisons_json(&on_record_summary),
            }))?
        );
    } else {
        print_text(&tally, seed, outbox_read);
        println!(
            "{}",
            crate::commands::sessions::comparisons_line(&on_record_summary)
        );
    }
    Ok(())
}

/// What a harness candidate's point-wise comparison found (row 2d-2), for
/// its measurement.
#[derive(Debug, Default)]
pub struct CandidateEvidence {
    pub tally: mecha_core::candidate::PointwiseTally,
    /// Why nothing was compared, when nothing was.
    pub not_run: Option<String>,
    /// The comparison-store rows counted, driven now or reused.
    pub comparisons: Vec<String>,
    /// The outbox could not be fully read, so draft points were missing
    /// from the pool — a finding, not an empty queue.
    pub outbox_unreadable: bool,
}

/// What a candidate's point pool is scoped to: the same workspace and
/// recency bound as its whole-session draw, and the model measured.
pub struct CandidateScope<'a> {
    pub workspace: Option<&'a Path>,
    /// Newest sessions read, at most — the whole-session draw's pool size.
    pub sessions: usize,
}

/// The two arms a candidate is compared under at a point: the run's
/// recorded config, and the same with the change applied. Both carry the
/// recorded prompt, so both carry the recorded rules hash — the arms differ
/// by role, and the stored comparison names the candidate in
/// `Pointers::proposal_id`.
fn candidate_arms(prep: &ProbePrep) -> [(Role, Option<String>); 2] {
    let policy = prep.recorded_rules_hash();
    [
        (Role::WithoutIntervention, policy.clone()),
        (Role::Candidate, policy),
    ]
}

/// The change an arm runs under, by its role: the candidate arm alone
/// carries it. Asked by role, never by position, so reordering the arms
/// cannot invert every verdict with every test still green.
fn change_for(
    role: Role,
    change: &mecha_core::harness::ConfigChange,
) -> Option<&mecha_core::harness::ConfigChange> {
    (role == Role::Candidate).then_some(change)
}

/// The (baseline, candidate) outcomes of a candidate's comparison, by role.
fn outcomes_of(c: &Comparison) -> Option<(Outcome, Outcome)> {
    let of = |role| c.arms.iter().find(|a| a.role == role).map(|a| a.outcome);
    Some((of(Role::WithoutIntervention)?, of(Role::Candidate)?))
}

/// A candidate's comparison at one point, keyed and pointed like any other,
/// with the candidate named.
fn candidate_comparison(
    prep: &ProbePrep,
    point: &Point,
    arms: Vec<Arm>,
    model: &str,
    candidate: &str,
) -> Result<Comparison> {
    let mut c = point_comparison(prep, point, arms, model)?;
    c.pointers.proposal_id = Some(candidate.to_string());
    Ok(c)
}

/// Count a candidate's comparison already on record at this point, if
/// there is one — a re-measurement pays for no point twice. A reused point
/// is a point compared, so it spends the same budget a driven one does:
/// `POINTS_PER_CANDIDATE` is a ceiling on the evidence as much as on the
/// cost, and a re-measurement that reused eight and drove eight more would
/// decide on sixteen (found on review).
fn reuse(
    on_record: &[Comparison],
    skeleton: &Comparison,
    out: &mut CandidateEvidence,
    budget: &mut usize,
) -> bool {
    let Some(row) = pointwise::on_record(on_record, skeleton) else {
        return false;
    };
    if let Some((baseline, candidate)) = outcomes_of(row) {
        out.tally.count(baseline, candidate);
        out.comparisons.push(row.id.clone());
    }
    *budget = budget.saturating_sub(1);
    true
}

/// Why a candidate's point-wise pass compared nothing, when it did not —
/// each cause in its own words.
fn empty_reason(out: &CandidateEvidence, seats_held: bool, lost_to_arms: usize) -> Option<String> {
    if out.tally != mecha_core::candidate::PointwiseTally::default() {
        return None;
    }
    Some(if seats_held {
        "every background seat stayed held, so no point was driven".into()
    } else if lost_to_arms > 0 {
        format!("{lost_to_arms} point(s) were driven and each lost an arm that could not be driven")
    } else {
        "no posed point this machine could drive".into()
    })
}

/// R26's point-wise half for a harness candidate, as R36 sizes it: up to
/// [`mecha_core::candidate::POINTS_PER_CANDIDATE`] posed points drawn
/// uniformly with `seed` (the measurement's own), each driven twice — the
/// recorded config and the same with `change` applied — a short horizon
/// from the point, on one background seat per point, and graded by the
/// owner's recorded verdict. Every comparison is stored through the
/// store's door; one already on record for this candidate at this point is
/// reused, not re-driven.
///
/// **Never evidence by default.** A provider off this machine (R29), no
/// drawable point, points that cannot run here (an owner-bound check point
/// needs hooks, the outbox and messages off — the nightly line has none of
/// them, so those points are skipped), a seat that never frees: each leaves
/// the tally short, and a short tally is `Undecided`, which hands the
/// verdict back to today's numeric gate.
pub async fn compare_candidate(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    change: &mecha_core::harness::ConfigChange,
    candidate: &str,
    seed: u64,
    scope: CandidateScope<'_>,
) -> Result<CandidateEvidence> {
    let mut out = CandidateEvidence::default();
    if !pointwise::on_this_machine(provider_cfg.base_url.as_deref()) {
        out.not_run = Some(
            "the measurement provider is not on this machine, and a point-wise arm resubmits \
             a recorded transcript (R29)"
                .into(),
        );
        return Ok(out);
    }
    let store = ComparisonStore::open_default()?;
    let mut on_record = store.comparisons()?;
    let drafts = match mecha_core::outbox::OutboxStore::open_existing_default() {
        None => Vec::new(),
        Some(o) => match o.items_counting() {
            Ok((items, skipped)) => {
                out.outbox_unreadable = skipped > 0;
                items
            }
            Err(_) => {
                out.outbox_unreadable = true;
                Vec::new()
            }
        },
    };
    let by_id: BTreeMap<String, &OutboxItem> = drafts.iter().map(|i| (i.id.clone(), i)).collect();
    let surfaces = SurfaceStore::open_default();
    let scan = mecha_core::runlog::Scan {
        workspace: scope.workspace.map(Path::to_path_buf),
        include_experiments: mecha_core::experiment::in_experiment_home(),
        ..Default::default()
    };
    let mut tally = Tally::default();
    let pool = collect(
        &Session::default_dir()?,
        &scan,
        Some(
            scope
                .sessions
                .saturating_mul(crate::harness_probe::POOL_MULTIPLE)
                .max(scope.sessions),
        ),
        Some(model),
        &drafts,
        surfaces.as_ref(),
        &mut tally,
    )?;
    let seats = crate::commands::tasks::permits()?;
    let mut budget = mecha_core::candidate::POINTS_PER_CANDIDATE;
    // Why a pass may come back empty, kept apart: "the seats never freed"
    // and "every arm failed" are machine facts, "no point to compare" is a
    // fact about the corpus, and a reader months later must not take one
    // for another (found on review).
    let mut seats_held = false;
    let mut lost_to_arms = 0usize;
    for Drawable { path, point } in pointwise::draw(pool, seed, |d| &d.point) {
        if budget == 0 {
            break;
        }
        // Only a point a structural validator poses can decide anything.
        let Plan::Posed(prep) = plan(&point, &path, &by_id, surfaces.as_ref())? else {
            continue;
        };
        if prep.provenance().admit().is_err()
            || prep.unrunnable_under(prepared).is_some()
            || !prep
                .lost_recorded_tools(prepared.agent.registry())
                .is_empty()
        {
            continue;
        }
        let arms = candidate_arms(&prep);
        let skeleton = candidate_comparison(
            &prep,
            &point,
            arms.iter()
                .map(|(r, p)| Arm::new(*r, p.clone(), Outcome::Unknown))
                .collect(),
            model,
            candidate,
        )?;
        if reuse(&on_record, &skeleton, &mut out, &mut budget) {
            continue;
        }
        let what = format!("compare candidate {candidate} {}", point.kind.as_str());
        let Some(_seat) = take_seat(&seats, &what).await? else {
            eprintln!("the background seats stayed held; the point-wise comparison stops short");
            seats_held = true;
            break;
        };
        budget -= 1;
        let mut outcomes = Vec::new();
        for (role, policy) in arms.iter() {
            let driven = probe::drive_arm_under(
                prepared,
                provider_cfg,
                model,
                &prep,
                prep.system_as_recorded(),
                Some(pointwise::HORIZON_TURNS),
                change_for(*role, change),
            )
            .await?;
            match driven {
                Ok(v) => outcomes.push(Arm::new(*role, policy.clone(), Outcome::from(&v))),
                Err(why) => {
                    eprintln!("· {} {}: {why}", point.session_id, point.kind.as_str());
                    break;
                }
            }
        }
        if outcomes.len() != arms.len() {
            lost_to_arms += 1;
            continue;
        }
        let comparison = candidate_comparison(&prep, &point, outcomes, model, candidate)?;
        let written = tally.stored.written;
        store_one(
            &store,
            prep.provenance(),
            comparison.clone(),
            &mut on_record,
            &mut tally,
        )?;
        if tally.stored.written > written {
            if let Some((baseline, candidate)) = outcomes_of(&comparison) {
                out.tally.count(baseline, candidate);
                out.comparisons.push(comparison.id);
            }
        }
    }
    out.not_run = empty_reason(&out, seats_held, lost_to_arms);
    Ok(out)
}

/// Drive every arm of one point, in order, on the one seat the caller
/// holds. An arm that could not be driven loses the whole point: a
/// comparison with a missing arm is a failed attempt, not a comparison.
async fn drive_point(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    prep: &ProbePrep,
    arms: &[Policy],
    tally: &mut Tally,
) -> Result<Result<Vec<Arm>, String>> {
    let mut out = Vec::new();
    for policy in arms {
        tally.arms_driven += 1;
        match probe::drive_arm_within(
            prepared,
            provider_cfg,
            model,
            prep,
            policy.system.clone(),
            Some(pointwise::HORIZON_TURNS),
        )
        .await?
        {
            Ok(v) => out.push(Arm::new(
                policy.role,
                policy.policy.clone(),
                Outcome::from(&v),
            )),
            Err(why) => return Ok(Err(why)),
        }
    }
    Ok(Ok(out))
}

/// Offer one comparison to the store's door, and count what it did.
fn store_one(
    store: &ComparisonStore,
    provenance: Provenance,
    comparison: Comparison,
    on_record: &mut Vec<Comparison>,
    tally: &mut Tally,
) -> Result<()> {
    let before = tally.stored.written;
    probe::store_comparison(store, provenance, &comparison, &mut tally.stored)?;
    if tally.stored.written > before {
        tally.verdict(comparison.verdict);
        on_record.push(comparison);
    }
    Ok(())
}

fn print_text(t: &Tally, seed: u64, outbox_read: bool) {
    let found: Vec<String> = t.found.iter().map(|(k, n)| format!("{k} {n}")).collect();
    println!(
        "point-wise comparison (seed {seed}): {} session(s) read, {} unreadable, {} test session(s) hidden",
        t.sessions_read, t.sessions_unreadable, t.tests_hidden
    );
    println!(
        "  points found: {}",
        if found.is_empty() {
            "none".to_string()
        } else {
            found.join(" · ")
        }
    );
    if !outbox_read {
        println!("  the outbox could not be fully read, so draft points are floors");
    }
    println!(
        "  not compared: {} in a session that was not clean (never drawn) · {} with no readable \
         tool surface (refused before driving)",
        t.not_clean, t.surface_unreadable
    );
    println!(
        "  drawn from {}: {} driven ({} arm(s); {} lost to an arm that could not be driven) · \
         {} unposed, stored inconclusive with nothing driven",
        t.drawable, t.driven, t.arms_driven, t.drive_failed, t.unposed
    );
    println!(
        "  skipped: {} already compared · {} with one distinct policy · {} unavailable · \
         {} over budget · {} with no free seat",
        t.already_compared, t.single_policy, t.unavailable, t.over_budget, t.no_seat
    );
    if let Some(line) = t.stored.line() {
        println!("  {line}");
    }
    println!(
        "  verdicts written: {} separated · {} tied · {} inconclusive",
        t.separated, t.tied, t.inconclusive
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;
    use mecha_core::comparison::Kind;
    use mecha_core::message::{Block, Message, ToolSpec};
    use mecha_core::planning::{Feedback, StepFeedback, Verification};
    use mecha_core::session::{Record, RunConfig, SessionKind, SessionMeta};
    use serde_json::{json, Value};

    const RULES: &str = "## Learned rules\n\nRules distilled from how this user has corrected \
                         you before.\n\n### behavior\n- Ask before mailing anyone outside the team.";

    fn specs() -> Vec<ToolSpec> {
        let spec = |name: &str, properties: Value| ToolSpec {
            name: name.into(),
            description: format!("{name} things."),
            input_schema: json!({"type": "object", "properties": properties}),
        };
        vec![
            spec("fs_list", json!({})),
            spec("fs_read", json!({"path": {"type": "string"}})),
            spec("fs_write", json!({"path": {"type": "string"}})),
            spec(
                "mail_send",
                json!({"to": {"type": "string"}, "body": {"type": "string"},
                       "urgent": {"type": "boolean", "default": false}}),
            ),
            spec("todo", json!({"op": {"type": "string"}})),
        ]
    }

    fn scan() -> mecha_core::runlog::Scan {
        mecha_core::runlog::Scan {
            include_tests: true,
            ..Default::default()
        }
    }

    fn step(
        verification: Verification,
        expected: Option<u32>,
        actual: Option<u32>,
    ) -> StepFeedback {
        StepFeedback {
            criterion: None,
            completion_batch: None,
            call_id: Some("t5".into()),
            step: "total the quarter".into(),
            goal: None,
            expected: None,
            expected_calls: expected,
            actual_calls: actual,
            verification,
            check_tampered: false,
        }
    }

    /// A session holding a point of every kind, recorded as `chat` writes
    /// one — a steer, a denial, two staged drafts, a declared check that
    /// failed and a forecast that missed — with the recorded surface kept.
    fn session(home: &Path, untrusted: bool) -> String {
        SurfaceStore::open_default()
            .unwrap()
            .record(&specs())
            .unwrap();
        let session = Session::create(
            &home.join("sessions"),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: home.to_path_buf(),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        session
            .append(&Record::Config(RunConfig {
                tools: specs().iter().map(|s| s.name.clone()).collect(),
                tools_hash: Some(mecha_core::surface::fingerprint(&specs())),
                system_prompt: Some(format!("You are mecha.\n\n{RULES}")),
                rules_hash: Some(mecha_core::learning::rules_hash(RULES)),
                rules_surface: Some(SessionKind::Tui),
                rules_goal: Some(mecha_core::situation::GoalKey::Named(
                    "task:t-ledger".parse().unwrap(),
                )),
                max_turns: 8,
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some("task:t-quarter".parse().unwrap()),
            })
            .unwrap();
        let tool_use = |id: &str, name: &str, input: Value| Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        };
        let result = |id: &str, content: &str, is_error: bool| Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error,
        };
        let mut steer = Message::tool_results(vec![result("t1", "q3.csv q4.csv", false)]);
        steer
            .content
            .push(Block::text("only the third quarter, please"));
        let mut feedback = Message::tool_results(vec![result("t5", "done", false)]);
        feedback.planning = Some(Feedback {
            steps: vec![
                step(Verification::Failed, None, None),
                step(Verification::Passed, Some(2), Some(9)),
            ],
            ..Default::default()
        });
        for m in [
            Message::user("total the quarters and mail Dirk"),
            Message::assistant(vec![tool_use("t1", "fs_list", json!({}))]),
            steer,
            Message::assistant(vec![tool_use("t2", "fs_write", json!({"path": "q3.md"}))]),
            Message::tool_results(vec![result(
                "t2",
                "Denied by the user: not that file",
                true,
            )]),
            Message::assistant(vec![tool_use(
                "t3",
                "mail_send",
                json!({"to": "dirk@example.invalid", "body": "Totals attached."}),
            )]),
            Message::tool_results(vec![result("t3", "staged for review", false)]),
            Message::assistant(vec![tool_use(
                "t4",
                "mail_send",
                json!({"to": "cleo@example.invalid", "body": "Q3 is done."}),
            )]),
            Message::tool_results(vec![result("t4", "staged for review", false)]),
            Message::assistant(vec![tool_use("t5", "todo", json!({"op": "complete"}))]),
            feedback,
            Message::assistant(vec![Block::text("Done, and I think it went well.")]),
        ] {
            session.append(&Record::Message(m)).unwrap();
        }
        session
            .append(&Record::Taint(Taint {
                untrusted,
                private: true,
            }))
            .unwrap();
        session.meta.id.clone()
    }

    /// The drafts the session staged, and what the owner did with them: the
    /// one to Dirk rewritten and sent, the one to Cleo rejected.
    fn owner_acts(session_id: &str) -> Vec<OutboxItem> {
        let outbox = mecha_core::outbox::OutboxStore::open(
            mecha_core::outbox::OutboxStore::default_root().unwrap(),
        )
        .unwrap();
        let stage = |call: &str, args: Value| {
            outbox
                .stage(
                    "mail_send",
                    mecha_core::outbox::OutboxKind::Message,
                    args,
                    Taint::default(),
                    mecha_core::outbox::Provenance {
                        session_id: Some(session_id.into()),
                        call_id: Some(call.into()),
                        filled_defaults: vec!["urgent".into()],
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        let edited = stage(
            "t3",
            json!({"to": "dirk@example.invalid", "body": "Totals attached.", "urgent": false}),
        );
        outbox
            .update_args(
                &edited.id,
                json!({"to": "dirk@example.invalid", "body": "Totals attached; the Q3 sheet follows.", "urgent": false}),
            )
            .unwrap();
        outbox.resolve(&edited.id, "sent", None).unwrap();
        let rejected = stage(
            "t4",
            json!({"to": "cleo@example.invalid", "body": "Q3 is done.", "urgent": false}),
        );
        outbox
            .resolve(&rejected.id, "rejected", Some("not yet".into()))
            .unwrap();
        outbox.items().unwrap()
    }

    /// Plan every drawable point and store it: a posed point's arms carry
    /// the outcomes `outcomes` supplies, in policy order. The verdict is
    /// supplied rather than driven: what is under test is the recording's
    /// path to the store, and driving needs a model — the binary test in
    /// `tests/pointwise_comparison.rs` drives one.
    fn plan_and_store(
        home: &Path,
        drafts: &[OutboxItem],
        outcomes: &[Outcome],
    ) -> (Tally, Vec<Comparison>) {
        let store = ComparisonStore::open_default().unwrap();
        let surfaces = SurfaceStore::open_default();
        let mut tally = Tally::default();
        let pool = collect(
            &home.join("sessions"),
            &scan(),
            None,
            None,
            drafts,
            surfaces.as_ref(),
            &mut tally,
        )
        .unwrap();
        let by_id: BTreeMap<String, &OutboxItem> =
            drafts.iter().map(|i| (i.id.clone(), i)).collect();
        let mut on_record = store.comparisons().unwrap();
        let mut offered = Vec::new();
        for d in pointwise::draw(pool, 7, |d| &d.point) {
            let (comparison, provenance) =
                match plan(&d.point, &d.path, &by_id, surfaces.as_ref()).unwrap() {
                    Plan::Posed(prep) => {
                        let arms: Vec<Arm> = policies(&prep, None)
                            .into_iter()
                            .zip(outcomes)
                            .map(|(p, o)| Arm::new(p.role, p.policy, *o))
                            .collect();
                        (
                            point_comparison(&prep, &d.point, arms, "scripted").unwrap(),
                            prep.provenance(),
                        )
                    }
                    Plan::Unposed(b) => *b,
                    Plan::Unavailable(why) => panic!("{:?}: {why}", d.point.kind),
                };
            if pointwise::already_compared(&on_record, &comparison) {
                tally.already_compared += 1;
                continue;
            }
            offered.push(comparison.clone());
            store_one(&store, provenance, comparison, &mut on_record, &mut tally).unwrap();
        }
        (tally, offered)
    }

    fn by_kind(rows: &[Comparison]) -> BTreeMap<String, (Validator, Verdict, usize)> {
        rows.iter()
            .map(|c| {
                (
                    mecha_core::appraisal::enum_name(&c.kind),
                    (c.validator, c.verdict, c.arms.len()),
                )
            })
            .collect()
    }

    /// Row 2d-1's acceptance, at the pass: a fixture point of each kind
    /// leaves a comparison that a fresh store handle returns — the posed
    /// ones decided by their structural validator over the arms, the two no
    /// validator can pose (a declared check, a surprise) stored inconclusive
    /// with no arm driven — and nothing the run or the owner wrote reaches
    /// the store.
    #[test]
    fn a_point_of_each_kind_leaves_a_comparison_a_second_read_returns() {
        let guard = crate::testenv::HomeGuard::new("pointwise-each-kind");
        let home = guard.dir.clone();
        let id = session(&home, false);
        let drafts = owner_acts(&id);
        let (tally, offered) = plan_and_store(&home, &drafts, &[Outcome::Pass, Outcome::Fail]);
        assert_eq!(tally.drawable, 6, "{tally:?}");
        assert_eq!(tally.stored.written, 6, "{tally:?}");
        assert_eq!((tally.separated, tally.inconclusive), (4, 2));

        let rows = ComparisonStore::open_default()
            .unwrap()
            .comparisons()
            .unwrap();
        assert_eq!(rows.len(), 6);
        for c in &offered {
            assert!(rows.contains(c), "{c:?} did not read back as written");
        }
        use Validator::*;
        use Verdict::*;
        let expect: BTreeMap<String, (Validator, Verdict, usize)> = [
            ("point-steer", (StructuralSteer, Separated, 2)),
            ("point-denial", (StructuralDenial, Separated, 2)),
            ("point-edited-draft", (ReleasedDraft, Separated, 2)),
            ("point-rejected-draft", (RejectedDraft, Separated, 2)),
            ("point-check", (Unposed, Inconclusive, 0)),
            ("point-surprise", (Unposed, Inconclusive, 0)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        assert_eq!(by_kind(&rows), expect);
        for c in &rows {
            assert_eq!(c.pointers.session_id, id);
            assert_eq!(c.goal_kind, Some(mecha_core::goal::GoalKind::Task));
            // The goal key the run was matched on (`rules_goal`, #311), on
            // posed and unposed points alike — never the anchor rebuilt.
            assert_eq!(
                c.situation.as_ref().and_then(|s| s.goal.clone()),
                Some(mecha_core::situation::GoalKey::Named(
                    "task:t-ledger".parse().unwrap()
                )),
                "{:?}",
                c.kind
            );
            if c.validator == Validator::Unposed {
                assert_eq!(c.model, "", "no model drove an unposed point");
                continue;
            }
            // The recorded prompt and rules-free: this home deploys no
            // rules, so there is no third arm.
            assert_eq!(
                c.arms.iter().map(|a| a.role).collect::<Vec<_>>(),
                vec![Role::WithoutIntervention, Role::RulesFree]
            );
            assert_eq!(c.preferred, vec![0]);
        }
        let draft = rows
            .iter()
            .find(|c| c.kind == Kind::PointEditedDraft)
            .unwrap();
        let call = draft.call.as_ref().unwrap();
        assert_eq!(
            (call.tool.as_str(), call.args.clone()),
            ("mail_send", vec!["body".to_string(), "to".to_string()])
        );
        assert_eq!(
            draft.situation.as_ref().and_then(|s| s.focus()),
            Some("mail_send")
        );
        let check = rows.iter().find(|c| c.kind == Kind::PointCheck).unwrap();
        assert_eq!(
            check.pointers.call_index,
            Some(4),
            "the todo call the step names"
        );
        let wire = std::fs::read_to_string(
            ComparisonStore::default_root()
                .unwrap()
                .join("comparisons.jsonl"),
        )
        .unwrap();
        for leaked in [
            "Totals attached",
            "the Q3 sheet",
            "Q3 is done",
            "dirk@",
            "cleo@",
            "q3.md",
            "only the third quarter",
            "not that file",
            "total the quarters",
            "went well",
            "t-quarter",
        ] {
            assert!(!wire.contains(leaked), "{leaked} reached the store");
        }
    }

    /// An arm the validator could not decide makes its point inconclusive,
    /// an unposed point stays inconclusive, and a point already on record
    /// under the same policies is not compared again.
    #[test]
    fn an_undecided_point_stays_inconclusive_and_is_not_compared_twice() {
        let guard = crate::testenv::HomeGuard::new("pointwise-inconclusive");
        let home = guard.dir.clone();
        let id = session(&home, false);
        let drafts = owner_acts(&id);
        let undecided = [Outcome::Pass, Outcome::Inconclusive];
        let (tally, _) = plan_and_store(&home, &drafts, &undecided);
        assert_eq!(tally.stored.written, 6);
        assert_eq!((tally.separated, tally.tied, tally.inconclusive), (0, 0, 6));
        let rows = ComparisonStore::open_default()
            .unwrap()
            .comparisons()
            .unwrap();
        assert!(rows
            .iter()
            .all(|c| c.verdict == Verdict::Inconclusive && c.preferred.is_empty()));
        let summary = mecha_core::comparison::Summary::of(&rows);
        assert_eq!((summary.inconclusive, summary.unposed), (6, 2));

        let (again, _) = plan_and_store(&home, &drafts, &undecided);
        assert_eq!((again.already_compared, again.stored.written), (6, 0));
        assert_eq!(
            ComparisonStore::open_default()
                .unwrap()
                .comparisons()
                .unwrap()
                .len(),
            6
        );
    }

    /// A tainted session's points are never drawn, and the store's door
    /// refuses any comparison offered from it anyway: nothing is written.
    #[test]
    fn a_tainted_session_leaves_nothing() {
        let guard = crate::testenv::HomeGuard::new("pointwise-tainted");
        let home = guard.dir.clone();
        let id = session(&home, true);
        let drafts = owner_acts(&id);
        let (tally, offered) = plan_and_store(&home, &drafts, &[Outcome::Pass, Outcome::Fail]);
        assert_eq!((tally.drawable, tally.not_clean), (0, 6), "{tally:?}");
        assert!(offered.is_empty());
        assert!(!ComparisonStore::default_root()
            .unwrap()
            .join("comparisons.jsonl")
            .exists());

        // Offered past the draw, the door still refuses every kind.
        let path = Session::find(&home.join("sessions"), &id).unwrap();
        let transcript = Session::read(&path).unwrap();
        let mine: Vec<&OutboxItem> = drafts.iter().collect();
        let by_id: BTreeMap<String, &OutboxItem> =
            drafts.iter().map(|i| (i.id.clone(), i)).collect();
        let surfaces = SurfaceStore::open_default();
        let store = ComparisonStore::open_default().unwrap();
        let mut t = Tally::default();
        let mut on_record = Vec::new();
        for point in pointwise::points_in(&id, &transcript.convo.messages, &mine) {
            let (c, p) = match plan(&point, &path, &by_id, surfaces.as_ref()).unwrap() {
                Plan::Posed(prep) => (
                    point_comparison(&prep, &point, Vec::new(), "scripted").unwrap(),
                    prep.provenance(),
                ),
                Plan::Unposed(b) => *b,
                Plan::Unavailable(why) => panic!("{why}"),
            };
            store_one(&store, p, c, &mut on_record, &mut t).unwrap();
        }
        assert_eq!((t.stored.written, t.stored.refused_not_clean), (0, 6));
        assert!(store.comparisons().unwrap().is_empty());
    }

    /// Two identical steers in one session are two points, and each is
    /// prepared at its own message — so each stores under its own pointers,
    /// and neither reads as "already compared" for the other (found on
    /// review: relocated by text, both prepared the first).
    #[test]
    fn two_identical_steers_are_two_comparisons() {
        let guard = crate::testenv::HomeGuard::new("pointwise-repeated-steer");
        let home = guard.dir.clone();
        SurfaceStore::open_default()
            .unwrap()
            .record(&specs())
            .unwrap();
        let session = Session::create(
            &home.join("sessions"),
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: home.clone(),
                title: None,
                kind: Some(SessionKind::Tui),
            },
        )
        .unwrap();
        session
            .append(&Record::Config(RunConfig {
                tools: specs().iter().map(|s| s.name.clone()).collect(),
                tools_hash: Some(mecha_core::surface::fingerprint(&specs())),
                system_prompt: Some(format!("You are mecha.\n\n{RULES}")),
                rules_hash: Some(mecha_core::learning::rules_hash(RULES)),
                max_turns: 8,
                ..Default::default()
            }))
            .unwrap();
        let steer = |id: &str| {
            let mut m = Message::tool_results(vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: "q3.csv".into(),
                is_error: false,
            }]);
            m.content.push(Block::text("only the third quarter"));
            m
        };
        let list = |id: &str| {
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: "fs_list".into(),
                input: json!({}),
            }])
        };
        for m in [
            Message::user("total the quarters"),
            list("t1"),
            steer("t1"),
            list("t2"),
            steer("t2"),
            Message::assistant(vec![Block::text("Done.")]),
        ] {
            session.append(&Record::Message(m)).unwrap();
        }
        session
            .append(&Record::Taint(Taint {
                untrusted: false,
                private: true,
            }))
            .unwrap();
        let (tally, offered) = plan_and_store(&home, &[], &[Outcome::Pass, Outcome::Fail]);
        assert_eq!(tally.drawable, 2);
        assert_eq!(
            (tally.stored.written, tally.already_compared),
            (2, 0),
            "{tally:?}"
        );
        let mut at: Vec<Option<usize>> = offered.iter().map(|c| c.pointers.message_index).collect();
        at.sort();
        assert_eq!(at, vec![Some(2), Some(4)]);
    }

    /// A harness candidate's comparison at a point names the candidate and
    /// carries the recorded config against the change; stored, it is found
    /// again by the same point, arms and candidate — and counted rather
    /// than re-driven — but never for a different candidate.
    #[test]
    fn a_candidates_comparison_is_stored_named_and_reused() {
        let guard = crate::testenv::HomeGuard::new("pointwise-candidate");
        let home = guard.dir.clone();
        let id = session(&home, false);
        let path = Session::find(&home.join("sessions"), &id).unwrap();
        let transcript = Session::read(&path).unwrap();
        let steer = pointwise::points_in(&id, &transcript.convo.messages, &[])
            .into_iter()
            .find(|p| p.kind == PointKind::Steer)
            .unwrap();
        let Plan::Posed(prep) = plan(&steer, &path, &BTreeMap::new(), None).unwrap() else {
            panic!("a steer is posed");
        };
        let arms: Vec<Arm> = candidate_arms(&prep)
            .into_iter()
            .zip([Outcome::Fail, Outcome::Pass])
            .map(|((r, p), o)| Arm::new(r, p, o))
            .collect();
        assert_eq!(
            arms.iter().map(|a| a.role).collect::<Vec<_>>(),
            vec![Role::WithoutIntervention, Role::Candidate]
        );
        assert_eq!(arms[0].policy, arms[1].policy, "one prompt, two configs");
        let c = candidate_comparison(&prep, &steer, arms, "scripted", "hc-ledger").unwrap();
        assert_eq!(c.pointers.proposal_id.as_deref(), Some("hc-ledger"));
        let store = ComparisonStore::open_default().unwrap();
        let mut written = Vec::new();
        store_one(
            &store,
            prep.provenance(),
            c.clone(),
            &mut written,
            &mut Tally::default(),
        )
        .unwrap();
        let rows = ComparisonStore::open_default()
            .unwrap()
            .comparisons()
            .unwrap();
        assert_eq!(rows, vec![c.clone()]);

        let skeleton = |candidate: &str| {
            candidate_comparison(
                &prep,
                &steer,
                candidate_arms(&prep)
                    .into_iter()
                    .map(|(r, p)| Arm::new(r, p, Outcome::Unknown))
                    .collect(),
                "scripted",
                candidate,
            )
            .unwrap()
        };
        let mut out = CandidateEvidence::default();
        let mut budget = mecha_core::candidate::POINTS_PER_CANDIDATE;
        assert!(reuse(&rows, &skeleton("hc-ledger"), &mut out, &mut budget));
        assert_eq!((out.tally.decided, out.tally.candidate_only), (1, 1));
        assert_eq!(out.comparisons, vec![c.id]);
        assert_eq!(
            budget,
            mecha_core::candidate::POINTS_PER_CANDIDATE - 1,
            "a reused point spends the budget a driven one does"
        );
        let mut other = CandidateEvidence::default();
        assert!(
            !reuse(&rows, &skeleton("hc-quarter"), &mut other, &mut budget),
            "another candidate's verdict at the same point is not this one's"
        );
        assert_eq!(budget, mecha_core::candidate::POINTS_PER_CANDIDATE - 1);
    }

    /// A re-measurement that finds every point on record reuses exactly the
    /// budget's worth and stops — never eight reused and eight more driven.
    #[test]
    fn a_re_measurement_reuses_no_more_than_the_budget() {
        let guard = crate::testenv::HomeGuard::new("pointwise-reuse-budget");
        let home = guard.dir.clone();
        let id = session(&home, false);
        let path = Session::find(&home.join("sessions"), &id).unwrap();
        let transcript = Session::read(&path).unwrap();
        let steer = pointwise::points_in(&id, &transcript.convo.messages, &[])
            .into_iter()
            .find(|p| p.kind == PointKind::Steer)
            .unwrap();
        let Plan::Posed(prep) = plan(&steer, &path, &BTreeMap::new(), None).unwrap() else {
            panic!("a steer is posed");
        };
        let stored = candidate_comparison(
            &prep,
            &steer,
            candidate_arms(&prep)
                .into_iter()
                .map(|(r, p)| Arm::new(r, p, Outcome::Pass))
                .collect(),
            "scripted",
            "hc-ledger",
        )
        .unwrap();
        let rows = vec![stored.clone()];
        let mut out = CandidateEvidence::default();
        let mut budget = mecha_core::candidate::POINTS_PER_CANDIDATE;
        let mut reused = 0;
        // The loop's own guard, over more on-record points than the budget.
        for _ in 0..(2 * mecha_core::candidate::POINTS_PER_CANDIDATE) {
            if budget == 0 {
                break;
            }
            if reuse(&rows, &stored, &mut out, &mut budget) {
                reused += 1;
            }
        }
        assert_eq!(reused, mecha_core::candidate::POINTS_PER_CANDIDATE);
        assert_eq!(
            out.tally.decided,
            mecha_core::candidate::POINTS_PER_CANDIDATE
        );
    }

    /// A candidate's point pool is scoped as its whole-session draw is: a
    /// session recorded under another model, or outside the workspace the
    /// night was scoped to, contributes no point (found on review).
    #[test]
    fn a_candidates_pool_is_scoped_to_the_model_and_workspace_measured() {
        let guard = crate::testenv::HomeGuard::new("pointwise-scope");
        let home = guard.dir.clone();
        session(&home, false);
        let pool = |model: Option<&str>, workspace: Option<std::path::PathBuf>| {
            let scan = mecha_core::runlog::Scan {
                include_tests: true,
                workspace,
                ..Default::default()
            };
            collect(
                &home.join("sessions"),
                &scan,
                None,
                model,
                &[],
                None,
                &mut Tally::default(),
            )
            .unwrap()
            .len()
        };
        assert_eq!(pool(Some("scripted"), None), 4, "its own model's points");
        assert_eq!(pool(Some("another-model"), None), 0);
        assert_eq!(pool(None, Some(home.clone())), 4, "inside the workspace");
        assert_eq!(pool(None, Some(home.join("elsewhere"))), 0);
    }

    /// The change goes to the candidate arm, and outcomes are read by role:
    /// a comparison stored with its arms in the other order still counts
    /// the baseline as the baseline.
    #[test]
    fn the_change_and_the_outcomes_follow_the_role_not_the_position() {
        let change = mecha_core::harness::parse_change("effort=low").unwrap();
        assert_eq!(change_for(Role::Candidate, &change), Some(&change));
        assert_eq!(change_for(Role::WithoutIntervention, &change), None);
        let swapped = Comparison::new(
            mecha_core::comparison::Kind::PointSteer,
            None,
            None,
            None,
            vec![
                Arm::new(Role::Candidate, None, Outcome::Pass),
                Arm::new(Role::WithoutIntervention, None, Outcome::Fail),
            ],
            Validator::StructuralSteer,
            Default::default(),
            "scripted",
        );
        assert_eq!(outcomes_of(&swapped), Some((Outcome::Fail, Outcome::Pass)));
        let mut t = mecha_core::candidate::PointwiseTally::default();
        let (b, c) = outcomes_of(&swapped).unwrap();
        t.count(b, c);
        assert_eq!(
            t.candidate_only, 1,
            "the candidate passed where the baseline failed"
        );
    }

    /// An empty pass says why: a held seat, arms that could not be driven,
    /// and an empty corpus are three findings, never one.
    #[test]
    fn an_empty_candidate_pass_names_its_cause() {
        let empty = CandidateEvidence::default();
        let why = |seats, lost| empty_reason(&empty, seats, lost).unwrap();
        assert!(why(true, 0).contains("seat"));
        assert!(why(false, 3).contains("3 point(s) were driven"));
        assert!(why(false, 0).contains("no posed point"));
        let mut some = CandidateEvidence::default();
        some.tally.count(Outcome::Pass, Outcome::Pass);
        assert_eq!(
            empty_reason(&some, true, 2),
            None,
            "evidence is not an empty pass"
        );
    }

    /// A check an owner-bound criterion graded, in a recording that carries
    /// the owner's artifact case, is posed — graded by the artifact repeat
    /// against pinned gold — where a declared check (above) is not.
    #[test]
    fn an_owner_bound_check_is_posed_through_its_artifact() {
        let _guard = crate::testenv::HomeGuard::new("pointwise-artifact");
        let (root, r) = crate::probe::mismatch_tests::fixture(Some(true), true, true);
        let path = Session::find(root.path(), &r.session_id).unwrap();
        let transcript = Session::read(&path).unwrap();
        let points = pointwise::points_in(&r.session_id, &transcript.convo.messages, &[]);
        let check = points
            .iter()
            .find(|p| p.kind == PointKind::FailedCheck)
            .expect("the failed criterion is a check point");
        let Plan::Posed(prep) = plan(check, &path, &BTreeMap::new(), None).unwrap() else {
            panic!("an owner-bound check with its case bound is posed");
        };
        assert_eq!(prep.validator(), Validator::ArtifactGold);
        let arms = policies(&prep, None)
            .into_iter()
            .zip([Outcome::Fail, Outcome::Pass])
            .map(|(p, o)| Arm::new(p.role, p.policy, o))
            .collect();
        let c = point_comparison(&prep, check, arms, "scripted").unwrap();
        let store = ComparisonStore::open_default().unwrap();
        let mut t = Tally::default();
        store_one(
            &store,
            prep.provenance(),
            c.clone(),
            &mut Vec::new(),
            &mut t,
        )
        .unwrap();
        assert_eq!(t.stored.written, 1, "{:?}", prep.provenance());
        let back = ComparisonStore::open_default()
            .unwrap()
            .comparisons()
            .unwrap();
        assert_eq!(back, vec![c]);
        assert_eq!(back[0].kind, Kind::PointCheck);
    }
}
