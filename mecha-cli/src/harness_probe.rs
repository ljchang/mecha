//! The session-corpus arm of harness rumination: one recorded session,
//! replayed whole, under the config that recorded it or under a candidate
//! change — the assembly `probe.rs` does for rule validation, re-shaped for
//! grading the harness itself.
//!
//! The differences from a rule probe are the point:
//!
//! - **The whole recording replays.** A rule probe truncates at the
//!   intervention because the intervention is the label; here the label is
//!   [`mecha_core::session::RunStats`] over the full episode — stop cause,
//!   tool errors, compactions — which §8 of the self-improvement research
//!   established is deterministic, objective, and computable with no judge.
//! - **The arms differ in config, not in system prompt.** Both arms carry
//!   the recorded system prompt verbatim, rules block included: the change
//!   under test is a knob, and anything else that differs between arms is
//!   noise dressed as signal.
//! - **A divergent episode is dropped, not scored.** Replay answers from the
//!   recording, so once an arm structurally departs, its remaining stats
//!   describe a run against tool results nobody asked for. Scoring it would
//!   grade a behaviour-visible change on the fraction it happened to track;
//!   dropping it fails safe, because thin evidence can only ever *propose*.

use crate::setup::Prepared;
use anyhow::Result;
use mecha_core::agent::{Agent, RunContext};
use mecha_core::candidate::Metric;
use mecha_core::config::{PermissionMode, ProviderConfig};
use mecha_core::harness::ConfigChange;
use mecha_core::replay::{extract, Trajectory};
use mecha_core::replay_priority::{Priority, Ranker};
use mecha_core::replay_run::{drive, replay_registry_reporting, OnDivergence};
use mecha_core::session::{RunConfig, RunStats, Session, SessionMeta};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// One recorded session, loaded and extracted once, ready to drive under any
/// number of candidate changes.
pub struct EpisodePrep {
    pub id: String,
    trajectory: Trajectory,
    recorded: RunConfig,
    /// Every outcome the session recorded, folded — what the episode's replay
    /// will be compared against, and what `Metric::headroom` sorts on. Carried
    /// on the prep because the read that produced the trajectory had it in
    /// hand; asking for it separately is another walk of the same file.
    episode: Option<mecha_core::session::RunStats>,
    /// The highest-ranked charter line a signed goal error of this session
    /// names (`appraisal::charter_rank`), zero the top line — §11.1's replay
    /// tiebreak, and the first thing the appraisal record has ever decided
    /// about what gets replayed. `None` where nothing was appraised (the
    /// caller passed no stores) or no signed error names a line the charter
    /// holds. Computed off the same read as the trajectory, for the same
    /// reason `episode` is.
    charter_rank: Option<usize>,
    /// The session's replay priority (row 2e-6, `mecha_core::replay_priority`)
    /// — what orders the selection among the episodes that can
    /// discriminate. `None` for a caller that is not drawing.
    priority: Option<Priority>,
    /// The compromise this replay is making, when it is making one — today,
    /// a session attached several times (a resume, or a mid-session
    /// `/provider`/`/mode` switch) replayed under its first config.
    /// A whole-session replay has no better single choice (`first()` is
    /// exactly right for the opening turns, and the driver cannot rebuild
    /// the agent at a config boundary mid-drive), but the compromise raises
    /// the odds of a divergence that says nothing about the candidate, and a
    /// dropped pair with no stated reason reads as the replay failing rather
    /// than the recording being unreplayable-as-one-run. `mecha replay`
    /// already caveats the same choice; this carries it to the probe's
    /// per-episode lines.
    pub config_caveat: Option<String>,
}

/// Load one session as a replayable episode. `Err(reason)` in the inner
/// result is a skip — never evidence for either arm.
///
/// `appraise` — the loaded stores and the session's start — asks for the
/// charter rank as well, off the same read; `None` leaves it unranked, for
/// a caller that is not drawing.
pub fn prepare_episode(
    path: &Path,
    id: &str,
    ranker: Option<&Ranker>,
) -> Result<Result<EpisodePrep, String>> {
    // One read. This runs over the whole pool — four times the wanted episode
    // count — every nightly, and `load` + `run_configs` + `episode_stats` were
    // three full reads and parses of the same file to answer questions one
    // walk answers together.
    let read = match Session::read(path) {
        Ok(t) => t,
        Err(e) => return Ok(Err(format!("session unreadable: {e:#}"))),
    };
    let trajectory = extract(&read.convo.messages);
    if let Err(error) = trajectory.ensure_replayable() {
        return Ok(Err(error.to_string()));
    }
    if trajectory.turns.is_empty() {
        return Ok(Err("no user turns".into()));
    }
    if trajectory.calls.is_empty() {
        // A session with no tool calls replays, but a harness change has
        // almost nothing to reach in it, and the budget is real model runs.
        return Ok(Err("no recorded tool calls".into()));
    }
    let Some(recorded) = read.configs.first().cloned() else {
        return Ok(Err("no RunConfig recorded".into()));
    };
    // `first()`, deliberately, unlike the intervention probe's point-scoped
    // `config_covering`: this replay runs the whole session from turn one as
    // a single drive, so no one config is right for all of it, and first is
    // exactly right for the opening turns where a divergence would land
    // first. The compromise is carried as a caveat rather than silently —
    // see `EpisodePrep::config_caveat`. "Attached", matching `mecha
    // replay`'s own line, not "resumed" — found on review: a `/provider`
    // or `/mode` switch mid-session also appends a `Config`, and that case
    // (the *worse* compromise: the session's later half ran under another
    // model) would have read as a resume it never was.
    if read.configs.iter().any(|c| c.appraisal_evidence.is_some()) {
        return Ok(Err(
            "owner-bound anticipatory evidence is not yet reproduced by harness probes".into(),
        ));
    }
    let config_caveat = (read.configs.len() > 1).then(|| {
        format!(
            "attached {} times; replayed under the first config",
            read.configs.len()
        )
    });
    // The priority and the rank both ride on the appraisal the sessions
    // readout would build for this transcript, from the stores the ranker
    // loaded once for the pass — built once here, off the same read: the
    // plan's `serves:` and the sensored-line attribution both land a
    // charter line on the errors, and `charter_rank` reads the highest.
    let (priority, charter_rank) = match ranker {
        None => (None, None),
        Some(ranker) => {
            let (priority, built) = ranker.of_transcript(&read);
            let rank = ranker
                .stores()
                .charter
                .as_ref()
                .filter(|c| !c.is_empty())
                .zip(built.as_ref())
                .and_then(|(charter, a)| mecha_core::appraisal::charter_rank(a, charter));
            (Some(priority), rank)
        }
    };
    Ok(Ok(EpisodePrep {
        id: id.to_string(),
        trajectory,
        recorded,
        episode: read.episode,
        config_caveat,
        charter_rank,
        priority,
    }))
}

/// The selection's order (row 2e-6): **what can discriminate first** — an
/// episode with headroom on the predicted metric ahead of one with none,
/// which can only tie or worsen and so cannot show a candidate winning —
/// then **the replay priority** (`replay_priority::order_by_priority`: gain
/// × need × decay, an unknown factor after every known positive priority,
/// a known zero after that, the hopeless last), then the highest charter
/// line a signed error names (§11.1's tiebreak, an unranked episode after
/// every ranked one), then the id, so the order is total and the seed is
/// not a lie. Headroom's *size* no longer orders: among episodes that can
/// discriminate, the priority decides (L1: "|goal error| as a priority in
/// its own right").
pub fn selection_order(
    a: (f64, &Priority, Option<usize>, &str),
    b: (f64, &Priority, Option<usize>, &str),
) -> std::cmp::Ordering {
    (b.0 > 0.0)
        .cmp(&(a.0 > 0.0))
        .then_with(|| mecha_core::replay_priority::order_by_priority(a.1, b.1))
        .then_with(|| match (a.2, b.2) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| a.3.cmp(b.3))
}

/// Newest-first replayable episodes for one model, up to `want`, from the
/// default session store. Returns the preps and how many candidates were
/// looked at and skipped.
/// The two slices a candidate is judged on, drawn separately on purpose.
pub struct Draw {
    /// Drawn by [`selection_order`]: the episodes that can discriminate on
    /// the metric ([`Metric::headroom`] above zero), in replay-priority order.
    pub selection: Vec<EpisodePrep>,
    /// Drawn uniformly from the same eligible pool, and drawn **first**.
    pub holdout: Vec<EpisodePrep>,
    /// Printed, because a sample nobody can redraw is one nobody can check.
    pub seed: u64,
    pub skipped: usize,
    /// How many of the selection, as drawn, carry a charter rank — recorded
    /// on the measurement beside the seed (`Measurement::ranked`), because
    /// the rank is read off inputs the seed and the corpus do not pin: the
    /// charter, whose order is the rank, and the stores a session's errors
    /// are signed from as they stand at the draw. A resolved draft or a
    /// re-ranked line redraws the ties. `None` when the tiebreak could not
    /// run — the charter did not load — which is not the same finding as a
    /// charter with nothing to rank against, a true zero (found on review).
    pub ranked: Option<usize>,
}

/// How much wider than the draw the eligible pool has to be.
///
/// **If the pool equals the draw there is no draw**: "prioritised" and
/// "uniform" both degenerate to "all of them", and the holdout stops being
/// independent of the selection because there was nothing to choose between.
/// Four is enough for the two draws to differ and small enough that the walk
/// stays bounded, which is `runlog::Scan`'s constraint.
pub(crate) const POOL_MULTIPLE: usize = 4;

/// How to split what the pool can supply between holdout and selection.
///
/// Pure, and tested, on `compact.rs`'s reasoning: getting it wrong is silent.
/// Nothing errors, nothing looks wrong in the log — the run just spends its
/// real-model budget on the slice that cannot decide anything and reports
/// thin evidence, which is indistinguishable from a corpus that was genuinely
/// too small.
///
/// **Both counts come off `min(want, pool)`.** Clamping only the holdout while
/// computing the selection from the unclamped `want` hands the holdout the
/// entire pool the moment the corpus is smaller than asked for: at the
/// defaults (`--sessions 16 --holdout-in 3`) over six eligible episodes that
/// drew 5 held and 1 selected — five sixths of the budget on the confirming
/// slice, and `MIN_SELECTION_PAIRS` tripped every time. The hash partition
/// this replaced would have given 2/4 there and could accept.
fn slice_sizes(want: usize, holdout_in: u64, pool: usize) -> (usize, usize) {
    let drawable = want.min(pool);
    // At least one held whenever there is anything to draw: a measurement with
    // no holdout is the multiple-comparisons trap the split exists to close,
    // and reporting it as confirmed would be worse than reporting thin
    // evidence. At `drawable == 1` that leaves no selection, which `judge`
    // correctly reads as nothing to decide from.
    let holdout_n = (drawable / holdout_in.max(1) as usize)
        .max(usize::from(drawable > 0))
        .min(drawable);
    (holdout_n, drawable - holdout_n)
}

/// Both phases of a draw in one call, for a test with the metric in hand.
/// `ruminate` calls them apart — [`draw_pool`] before the diagnosis,
/// [`Pool::select`] after it (row 2f).
#[cfg(test)]
pub fn draw_episodes(
    sessions_dir: &Path,
    model: &str,
    metric: Metric,
    want: usize,
    holdout_in: u64,
    seed: u64,
    workspace: Option<&Path>,
) -> Result<Draw> {
    Ok(draw_pool(sessions_dir, model, want, holdout_in, seed, workspace)?.select(metric))
}

/// The first phase of a [`Draw`]: the eligible pool and its uniform holdout
/// — everything the draw decides before a metric exists.
///
/// **Split so the diagnostician can read the episodes a candidate will be
/// measured on before there is a candidate** (row 2f; the owner's ruling
/// R38). Nightly, `ruminate` mints the candidate id — the seed — draws this
/// phase, hands the diagnostician the clean appraisals of [`Pool::remainder`],
/// and only after the proposal names its metric calls [`Pool::select`]. The
/// holdout is fixed before the diagnosis and is **never** in the remainder,
/// so the slice that confirms a change is one its author never read about.
///
/// The split moved nothing about what is measured: the pool and the holdout
/// are exactly what the single-phase draw produced for the same seed and
/// store, since the holdout reads neither the metric nor any priority. The
/// selection is drawn from the same remainder at the same size; since row
/// 2e-6 its order is [`selection_order`]'s — the replay priority among the
/// episodes that can discriminate — where it was headroom's
/// (`the_split_draw_holds_the_single_phase_holdout_under_the_ranking` holds
/// the holdout against the pre-priority single-phase draw, element for
/// element).
pub struct Pool {
    /// Drawn uniformly, in pool order — as the single-phase draw left it.
    holdout: Vec<EpisodePrep>,
    /// The rest of the pool, in pool order, not yet ranked.
    rest: Vec<EpisodePrep>,
    selection_n: usize,
    seed: u64,
    skipped: usize,
    /// The charter did not load, so the tiebreak cannot run —
    /// [`Draw::ranked`]'s `None`.
    charter_unreadable: bool,
    /// What the ranker could not read — each an unknown factor on every
    /// priority — for the caller to print beside the draw.
    pub caveats: Vec<String>,
}

impl Pool {
    /// The episodes the selection will be ranked from: the pool minus the
    /// holdout. What the diagnostician may read appraisals of, and nothing
    /// else.
    pub fn remainder(&self) -> Vec<String> {
        self.rest.iter().map(|p| p.id.clone()).collect()
    }

    /// The held-out episodes' ids.
    #[cfg(test)]
    pub fn holdout_ids(&self) -> Vec<String> {
        self.holdout.iter().map(|p| p.id.clone()).collect()
    }

    /// The second phase: rank the remainder by [`selection_order`] — what
    /// can discriminate for `metric` first, then the replay priority, then
    /// the charter line the record names (§11.1's tiebreak) — and keep the
    /// selection's share. The holdout was fixed by the first phase and is
    /// handed on untouched.
    pub fn select(self, metric: Metric) -> Draw {
        // Headroom off *every* outcome the session recorded, folded.
        // `last_outcome` describes how the session ended, and an episode
        // here is the whole session — `extract` pulls every recorded user
        // turn and `drive_episode` replays all of them, folding each run
        // with `absorb`. Sizing the priority signal from one run while the
        // arms it feeds are folded over all of them is a unit mismatch, and
        // it inverts: a resumed chat with nine error-heavy runs and a clean
        // tenth scores zero and sorts to the bottom, so the most
        // discriminating episode in the corpus is the one prioritised
        // sampling drops.
        //
        // A session with no recorded outcome scores zero rather than being
        // dropped: it was still drawable by the uniform half, which is the
        // half that must not be filtered by informativeness.
        let mut rest: Vec<(EpisodePrep, f64)> = self
            .rest
            .into_iter()
            .map(|p| {
                let headroom = p
                    .episode
                    .as_ref()
                    .map(|s| metric.headroom(s))
                    .unwrap_or(0.0);
                (p, headroom)
            })
            .collect();
        // An episode prepared without a ranker has every factor unknown —
        // never the case in a draw, whose pool is always ranked.
        let unread = Priority::unread();
        rest.sort_by(|a, b| {
            selection_order(
                (
                    a.1,
                    a.0.priority.as_ref().unwrap_or(&unread),
                    a.0.charter_rank,
                    &a.0.id,
                ),
                (
                    b.1,
                    b.0.priority.as_ref().unwrap_or(&unread),
                    b.0.charter_rank,
                    &b.0.id,
                ),
            )
        });
        rest.truncate(self.selection_n);
        // Unknown where the tiebreak could not run, a count — zero included —
        // where it did.
        let ranked = (!self.charter_unreadable).then(|| {
            rest.iter()
                .filter(|(p, _)| p.charter_rank.is_some())
                .count()
        });
        Draw {
            selection: rest.into_iter().map(|(p, _)| p).collect(),
            holdout: self.holdout,
            seed: self.seed,
            skipped: self.skipped,
            ranked,
        }
    }
}

/// The first phase of a [`Draw`] for one candidate — see [`Pool`]: the
/// eligible pool and its uniform holdout. Everything in it is what the
/// single-phase draw did before it read the metric, unchanged.
///
/// **The holdout comes off the pool first, uniformly.** Prioritised experience
/// replay samples by how much a transition can teach, which is right for
/// choosing what to spend a replay on and wrong for confirming the result:
/// prioritised sampling is biased sampling, and PER corrects it with
/// importance weights. Here the correction is that the confirming slice is
/// never prioritised. Taking it first also means the selection cannot quietly
/// steal the episodes that would have checked it.
///
/// Eligibility stays recency-bounded, because the harness being graded is the
/// one running now and the oldest sessions were recorded by versions of it
/// that no longer exist. So the draw is uniform *over the eligible corpus*,
/// which is the honest claim — not uniform over all history.
pub fn draw_pool(
    sessions_dir: &Path,
    model: &str,
    want: usize,
    holdout_in: u64,
    seed: u64,
    workspace: Option<&Path>,
) -> Result<Pool> {
    let mut listed: Vec<(SessionMeta, PathBuf)> = Session::list(sessions_dir)?;
    listed.sort_by_key(|entry| std::cmp::Reverse(entry.0.created_at));

    // Build the eligible pool: model-matched, replayable, recency-bounded.
    let pool_size = want.saturating_mul(POOL_MULTIPLE).max(want);
    let mut pool: Vec<EpisodePrep> = Vec::new();
    let mut skipped = 0usize;
    // The same admission the diagnosis applies (`Scan::admits`), built once:
    // see the comment at its use below.
    let admission = mecha_core::runlog::Scan {
        workspace: workspace.map(std::path::Path::to_path_buf),
        ..Default::default()
    };
    // Everything a priority is read from, once for the whole pool: the
    // stores an appraisal reads, the score ledger, the harness and
    // comparison stores, and one walk of the recent corpus for recurrence
    // (`replay_priority::Ranker`). Loaded before the holdout is drawn and
    // never read by it: the holdout below takes ids alone.
    let ranker = Ranker::load(sessions_dir, chrono::Utc::now());
    for (meta, path) in listed {
        if pool.len() >= pool_size {
            break;
        }
        // The header model, not per-run attribution: a whole-session replay
        // runs under one model, so the filter's job is only to keep the
        // corpus representative of the model being graded.
        if meta.model != model {
            continue;
        }
        // Scoped the same way the diagnosis was, or the two halves of one
        // night disagree about what they are talking about: `--from-workspace`
        // narrowed the brief while the draw kept re-scanning every session, so
        // a change reasoned about one job was accepted or rejected on the
        // average of four. Found in review, and it read as a working flag —
        // the diagnosis was visibly scoped, and only the arms were not.
        //
        // Prefix, matching `runlog::Scan`: a checkout's worktrees are the same
        // population as the checkout. Through `Scan::admits` itself rather
        // than a second spelling of the rule, so the draw and the diagnosis
        // cannot disagree again through a new filter — the second time this
        // paragraph's incident recurred, it was the `kind` filter: the
        // diagnosis excluded smoke-test sessions and the draw did not, so
        // real-model budget was spent replaying them (found on review).
        if !admission.admits(&meta) {
            continue;
        }
        match prepare_episode(&path, &meta.id, Some(&ranker))? {
            Ok(prep) => pool.push(prep),
            Err(_) => skipped += 1,
        }
    }

    let (holdout_n, selection_n) = slice_sizes(want, holdout_in, pool.len());
    let mut caveats = ranker.caveats().to_vec();
    caveats.extend(mecha_core::replay_priority::unknown_summary(
        pool.iter().filter_map(|p| p.priority.as_ref()),
    ));

    // Uniform first. Sorted by id before the shuffle, or the seed is a lie —
    // a deterministic shuffle of a nondeterministic order is nondeterministic.
    let mut ids: Vec<String> = pool.iter().map(|p| p.id.clone()).collect();
    ids.sort();
    let held: std::collections::HashSet<String> =
        mecha_core::sample::take_uniform(ids, seed, holdout_n)
            .into_iter()
            .collect();

    let (holdout, rest): (Vec<_>, Vec<_>) = pool.into_iter().partition(|p| held.contains(&p.id));

    Ok(Pool {
        holdout,
        rest,
        selection_n,
        seed,
        skipped,
        charter_unreadable: ranker.stores().charter_unreadable,
        caveats,
    })
}
/// What one arm of one episode produced.
pub struct ArmOutcome {
    pub receipt: serde_json::Value,
    pub stats: RunStats,
    /// Why the replay left the recording, if it did. The stats above then
    /// cover only the prefix it tracked, and the caller drops the episode.
    ///
    /// **A reason, not a bool.** The caller ORs the two arms together, so a
    /// bare flag lost the one distinction that decides what to do next: a
    /// baseline that diverged means the replay itself is unreliable on this
    /// episode, while a candidate-only divergence is the change altering
    /// behaviour — which is what the measurement is *for*. On 2026-09-01
    /// twelve of sixteen episodes were dropped and the record could not say
    /// which kind either of them was.
    pub divergence: Option<String>,
}

impl ArmOutcome {
    fn from_report(
        report: mecha_core::replay_run::ReplayReport,
        cursor_reason: Option<String>,
    ) -> Self {
        Self {
            receipt: serde_json::json!({"stats": report.stats, "calls": report.replayed_calls,
                "divergences": report.divergences, "recorded_calls": report.recorded_calls,
                "stopped_early": report.stopped_early, "final_text": report.final_text}),
            divergence: report
                .unmeasurable_reason()
                .map(|why| cursor_reason.unwrap_or(why)),
            stats: report.stats,
        }
    }

    pub fn diverged(&self) -> bool {
        self.divergence.is_some()
    }
}

/// Drive one episode once, under the recorded config plus an optional
/// candidate change. `None` is the baseline arm — the same protocol as the
/// candidate arm, because comparing a live recording against a replay would
/// measure replay artifacts, not the change.
pub async fn drive_episode(
    prepared: &Prepared,
    provider_cfg: &ProviderConfig,
    model: &str,
    prep: &EpisodePrep,
    change: Option<&ConfigChange>,
) -> Result<Result<ArmOutcome, String>> {
    let recorded = &prep.recorded;
    let cancel = CancellationToken::new();
    // The recorded specs, when the surface store still holds them. Loaded per
    // arm rather than on the prep: both arms of one episode read the same
    // small blob, and the prep's own doc says it exists to avoid re-walking
    // the *session* file — the store is a different, deduped read.
    let recorded_specs = recorded
        .tools_hash
        .as_deref()
        .and_then(|h| mecha_core::surface::SurfaceStore::open_default()?.load(h))
        .unwrap_or_default();
    let (registry, divergence) = match replay_registry_reporting(
        &recorded.tools,
        prepared.agent.registry(),
        Some(&crate::setup::surface_only_registry()),
        &recorded_specs,
        prep.trajectory.calls.clone(),
        OnDivergence::Stop,
        cancel.clone(),
    ) {
        Ok(reg) => reg,
        Err(e) => return Ok(Err(format!("{e:#}"))),
    };
    // Nothing executes under Stop mode, so nothing needs approving.
    let approver: Arc<dyn mecha_core::tool::Approver> = Arc::new(mecha_core::tool::ModeApprover {
        mode: PermissionMode::Allow,
    });

    // The recorded run's own settings, exactly as `probe::drive_arm` restores
    // them — then the candidate change on top, which is the only difference
    // the two arms are allowed to have.
    let mut agent_cfg = prepared.config.agent.clone();
    agent_cfg.system_prompt = recorded.system_prompt.clone();
    agent_cfg.system_prompt_file = None;
    agent_cfg.effort = recorded.effort;
    agent_cfg.thinking = recorded.thinking;
    agent_cfg.cache_prompt = recorded.cache_prompt;
    agent_cfg.max_tokens = recorded.max_tokens;
    agent_cfg.max_turns = recorded.max_turns;
    agent_cfg.max_output_tokens = recorded.max_output_tokens;
    agent_cfg.max_cost_usd = recorded.max_cost_usd;
    agent_cfg.compact_at_tokens = recorded.compact_at_tokens;
    agent_cfg.compact_keep_recent = recorded.compact_keep_recent;
    if let Some(change) = change {
        if let Err(e) = change.apply_to_agent(&mut agent_cfg) {
            return Ok(Err(format!("candidate change failed to apply: {e:#}")));
        }
    }

    let mut tool_ctx = mecha_core::tool::ToolCtx {
        workspace: recorded.workspace.clone(),
        // **The `compact` channel, or a replayed compaction reads as divergence.**
        // The run gets a threshold from `with_compact_at` below, and a recorded
        // session whose tool list named `compact` gets the tool back through the
        // rebuilt registry — so without this the tool answers "compaction is not
        // enabled for this run", which is false of the run it is replaying. Under
        // `--on-divergence=live` that is an executed call returning the wrong
        // answer and counting as a divergence; in a harness probe it is worse,
        // because both arms then replay a trajectory missing the compactions the
        // recording had, and a `compact_at_tokens` candidate is measured on runs
        // that never compacted.
        //
        // Wired unconditionally: the flag costs nothing when no tool reads it,
        // and making it conditional would be a second place that has to agree
        // with `setup`'s about whether this run compacts at all — which is the
        // split `PreparedTools::compact_requested` exists to prevent.
        compact_requested: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        ))),
        ..Default::default()
    };
    if !recorded.workspace.exists() {
        // Fine for a pure replay: nothing touches the filesystem.
        tool_ctx.workspace = std::env::temp_dir();
    }

    let agent = Agent::new(
        mecha_core::provider::build(provider_cfg)?,
        registry,
        Arc::clone(&approver),
        tool_ctx.clone(),
        agent_cfg.clone(),
        Some(model.to_string()),
    )?
    .with_clock(mecha_core::clock::for_replay(recorded.clock));
    let cx = RunContext::new(tool_ctx, approver)
        .with_cancel(cancel)
        .with_compact_at(agent_cfg.compact_at_tokens);
    match drive(&agent, &cx, &prep.trajectory).await {
        Ok(report) => Ok(Ok(ArmOutcome::from_report(report, divergence.reason()))),
        Err(e) => Ok(Err(format!("replay failed: {e:#}"))),
    }
}

/// A replayable recorded session for draw tests: one tool call, a clean
/// taint checkpoint, and an outcome that varies with `n` on every metric, so
/// headroom orders differ by metric. `serves` names a charter line on a
/// failed declared check, which is what gives an episode a rank.
#[cfg(test)]
pub(crate) fn fixture_session(dir: &Path, id: &str, n: u32, serves: Option<&str>) {
    use mecha_core::message::{Block, Message, Role};
    use mecha_core::session::Record;
    let s = Session::create(
        dir,
        SessionMeta {
            id: id.into(),
            created_at: chrono::Utc::now(),
            provider: "local".into(),
            model: "m".into(),
            workspace: PathBuf::from("/tmp"),
            title: None,
            kind: None,
        },
    )
    .unwrap();
    s.append(&Record::Config(RunConfig::default())).unwrap();
    let mut calls = vec![Block::ToolUse {
        id: "t1".into(),
        name: "shell".into(),
        input: serde_json::json!({}),
    }];
    let mut results = vec![Block::ToolResult {
        tool_use_id: "t1".into(),
        content: "ok".into(),
        is_error: false,
    }];
    if let Some(serves) = serves {
        calls.push(Block::ToolUse {
            id: "t2".into(),
            name: "todo".into(),
            input: serde_json::json!({
                "items": [{"content": "do it", "status": "completed"}],
                "serves": serves,
            }),
        });
        results.push(Block::ToolResult {
            tool_use_id: "t2".into(),
            content: "ok".into(),
            is_error: false,
        });
    }
    s.append_messages(&[
        Message::user("do the thing"),
        Message::assistant(calls),
        Message {
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
            role: Role::User,
            content: results,
        },
        Message::assistant(vec![Block::text("done")]),
    ])
    .unwrap();
    s.append(&Record::Taint(mecha_core::agent::Taint {
        private: false,
        untrusted: false,
    }))
    .unwrap();
    s.append(&Record::Outcome(RunStats {
        turns: 1 + n % 4,
        tool_calls: 3,
        tool_errors: n % 3,
        compactions: n % 2,
        ended_on_failed_call: n.is_multiple_of(2),
        checks_declared: serves.map(|_| 1),
        checks_passed: serves.map(|_| 0),
        ..Default::default()
    }))
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_calls_never_enter_a_harness_pair_even_without_cancellation() {
        let report = mecha_core::replay_run::ReplayReport {
            divergences: vec![mecha_core::replay::Divergence::Missing {
                index: 0,
                expected: "fs_write".into(),
            }],
            replayed_calls: vec![],
            recorded_calls: 1,
            turns: 1,
            stopped_early: false,
            final_text: "done".into(),
            stats: RunStats::default(),
            call_base: 0,
        };
        let arm = ArmOutcome::from_report(report, None);
        assert!(arm.diverged());
        assert_eq!(arm.receipt["recorded_calls"], 1);
        assert_eq!(arm.receipt["divergences"][0]["kind"], "missing");
    }

    #[test]
    fn neither_slice_is_sized_from_a_pool_that_does_not_exist() {
        // The regression: `holdout_n` was clamped to the pool and
        // `selection_n` was `want - holdout_n` off the *unclamped* want, so a
        // short pool went almost entirely to the holdout.
        assert_eq!(slice_sizes(16, 3, 6), (2, 4), "was (5, 1)");
        assert_eq!(
            slice_sizes(16, 3, 12),
            (4, 8),
            "was (5, 7) — under the floor"
        );
        // A full pool is unaffected, which is what makes this a fix rather
        // than a retuning.
        assert_eq!(slice_sizes(16, 3, 64), (5, 11));
        assert_eq!(slice_sizes(16, 3, 16), (5, 11));
    }

    /// The `draw_episodes` half of the workspace filter, which is the half
    /// that had no test.
    ///
    /// Its twin in `Corpus::scan` is pinned in `runlog.rs`, and that test's own
    /// comment says this is the one most worth pinning — because this is the
    /// half that was *missing* when the flag shipped, while the brief looked
    /// correctly scoped. A filter whose applied half is the visible half is
    /// indistinguishable from a working one, so the unseen half is where the
    /// test belongs.
    #[test]
    fn the_draw_is_scoped_to_the_same_workspace_the_diagnosis_was() {
        mecha_core::session::ignore_kind_env_for_tests();
        use mecha_core::message::{Block, Message};
        use mecha_core::session::{Record, RunConfig, Session, SessionMeta};

        // Under a scratch home: the draw reads the charter and, where it has
        // lines, the stores, so a test without one would read the
        // developer's real `~/.mecha` (found on review).
        let home = crate::testenv::HomeGuard::new("probe-ws");
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let replayable = |s: &Session| {
            s.append(&Record::Config(RunConfig::default())).unwrap();
            s.append_messages(&[
                Message::user("do the thing"),
                Message::assistant(vec![Block::ToolUse {
                    id: "t1".into(),
                    name: "shell".into(),
                    input: serde_json::json!({}),
                }]),
                Message {
                    harness: false,
                    planning: None,
                    tool_provenance: Default::default(),
                    role: mecha_core::message::Role::User,
                    content: vec![Block::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        };
        let make = |id: &str, workspace: &str| {
            let s = Session::create(
                &dir,
                SessionMeta {
                    id: id.into(),
                    created_at: chrono::Utc::now(),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from(workspace),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            replayable(&s);
        };
        make("20260101T000000-a", "/src/mecha");
        make("20260101T000001-b", "/src/mecha/.claude/worktrees/lane");
        make("20260101T000002-c", "/tmp");
        // Shares a textual prefix and is a different directory: `starts_with`
        // is component-wise, and this is what a naive string check lets in.
        make("20260101T000003-d", "/src/mecha-other");

        let drawn = |ws: Option<&Path>| {
            let d = draw_episodes(&dir, "m", Metric::Turns, 16, 3, 7, ws).unwrap();
            d.selection.len() + d.holdout.len()
        };
        assert_eq!(
            drawn(None),
            4,
            "unscoped, every replayable session is eligible"
        );
        assert_eq!(
            drawn(Some(Path::new("/src/mecha"))),
            2,
            "the checkout and its worktree, and neither /tmp nor /src/mecha-other"
        );
        assert_eq!(drawn(Some(Path::new("/tmp"))), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The order alone, on tuples (row 2e-6): an episode that can
    /// discriminate outranks one that cannot, whatever their priorities;
    /// among those that can, the priority decides and headroom's size does
    /// not; equal priority orders by charter rank, an unranked episode
    /// after every ranked one, and the id last.
    #[test]
    fn the_selection_orders_by_headroom_then_priority_then_rank_then_id() {
        use mecha_core::replay_priority::Inputs;
        use std::cmp::Ordering::*;
        let p = |owner_gain: f64| {
            Priority::of(&Inputs {
                owner_gain: Some(owner_gain),
                surprises: Some(0),
                recurrence: Some(1),
                age_days: 0.0,
                hopeless: Some(false),
            })
        };
        let (high, low) = (p(3.0), p(1.0));
        // Headroom gates: no priority promotes an episode that can only tie.
        assert_eq!(
            selection_order((0.1, &low, None, "z"), (0.0, &high, Some(0), "a")),
            Less
        );
        // Among the informative, the priority decides, not headroom's size —
        // the old order put the 9.0 first.
        assert_eq!(
            selection_order((0.5, &high, None, "z"), (9.0, &low, Some(0), "a")),
            Less
        );
        // Equal priority: the charter rank, then the id.
        assert_eq!(
            selection_order((0.5, &low, Some(0), "z"), (0.5, &low, Some(1), "a")),
            Less
        );
        assert_eq!(
            selection_order((0.5, &low, Some(3), "z"), (0.5, &low, None, "a")),
            Less
        );
        assert_eq!(
            selection_order((0.5, &low, None, "a"), (0.5, &low, None, "b")),
            Less
        );
        assert_eq!(
            selection_order((0.5, &low, Some(2), "a"), (0.7, &low, Some(2), "a")),
            Equal
        );
    }

    /// The draw end to end: three sessions with equal headroom whose ids
    /// sort one way and whose plans name charter lines the other way. The
    /// old order — headroom then id — selects them by id; this one by the
    /// line the plan served, with the planless one last. Whichever the seed
    /// holds out, the selection's order is the charter's.
    #[test]
    fn a_signed_error_against_the_top_line_replays_before_one_against_the_fifth() {
        mecha_core::session::ignore_kind_env_for_tests();
        use mecha_core::message::{Block, Message};
        use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionMeta};

        let home = crate::testenv::HomeGuard::new("probe-rank");
        std::fs::write(
            home.dir.join("charter.toml"),
            "[[line]]\nid = \"top\"\ntext = \"First.\"\n\n[[line]]\nid = \"fifth\"\ntext = \"Later.\"\n",
        )
        .unwrap();
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();

        // Ids reversed against rank — the stamp leads the id, so the
        // planless session sorts first by id and last by the charter.
        let make = |id: &str, serves: Option<&str>, untrusted: bool| {
            let s = Session::create(
                &dir,
                SessionMeta {
                    id: id.into(),
                    created_at: chrono::Utc::now(),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            s.append(&Record::Config(RunConfig::default())).unwrap();
            let mut calls = vec![Block::ToolUse {
                id: "t1".into(),
                name: "shell".into(),
                input: serde_json::json!({}),
            }];
            let mut results = vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            }];
            if let Some(serves) = serves {
                calls.push(Block::ToolUse {
                    id: "t2".into(),
                    name: "todo".into(),
                    input: serde_json::json!({
                        "items": [{"content": "do it", "status": "completed"}],
                        "serves": serves,
                    }),
                });
                results.push(Block::ToolResult {
                    tool_use_id: "t2".into(),
                    content: "ok".into(),
                    is_error: false,
                });
            }
            s.append_messages(&[
                Message::user("do the thing"),
                Message::assistant(calls),
                Message {
                    harness: false,
                    planning: None,
                    tool_provenance: Default::default(),
                    role: mecha_core::message::Role::User,
                    content: results,
                },
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
            // The taint checkpoint every front-end writes after a turn: the
            // rank reads only a clean-origin appraisal, and a session with
            // no checkpoint is unknown, which is never clean.
            s.append(&Record::Taint(mecha_core::agent::Taint {
                private: false,
                untrusted,
            }))
            .unwrap();
            // Equal headroom on every metric, and one signed error each: a
            // declared check that did not pass, which carries the plan's
            // goal onto the error.
            s.append(&Record::Outcome(RunStats {
                turns: 2,
                tool_calls: 1,
                checks_declared: Some(1),
                checks_passed: Some(0),
                ..Default::default()
            }))
            .unwrap();
        };
        make("20260101T000002-top", Some("charter:top"), false);
        make("20260101T000001-fifth", Some("charter:fifth"), false);
        make("20260101T000000-none", None, false);
        // Names the top line from a session that carried untrusted content:
        // its `serves:` is the model's own string under injection, so it
        // ranks nothing and sorts with the planless one, by id.
        make("20260101T000003-tainted", Some("charter:top"), true);

        let d = draw_episodes(&dir, "m", Metric::Turns, 4, 4, 11, None).unwrap();
        assert_eq!(d.selection.len() + d.holdout.len(), 4);
        let order: Vec<&str> = d.selection.iter().map(|p| p.id.as_str()).collect();
        // The charter's order, whichever one the seed held out.
        let expected = [
            "20260101T000002-top",
            "20260101T000001-fifth",
            "20260101T000000-none",
            "20260101T000003-tainted",
        ];
        let mut cursor = 0;
        for id in &order {
            let at = expected[cursor..]
                .iter()
                .position(|e| e == id)
                .unwrap_or_else(|| panic!("selection out of charter order: {order:?}"));
            cursor += at + 1;
        }
        assert_eq!(order.len(), 3, "{order:?}");
        assert_eq!(
            d.ranked,
            Some(
                order
                    .iter()
                    .filter(|id| id.ends_with("top") || id.ends_with("fifth"))
                    .count()
            ),
            "ranked counts the selected episodes whose clean error named a line"
        );

        // A charter that does not load: the draw still happens, by headroom
        // then id, and the record says the tiebreak could not run — never
        // that it ran and ranked nothing.
        std::fs::write(
            home.dir.join("charter.toml"),
            "[[line]]\nid = \"\"\ntext = \"x\"\n",
        )
        .unwrap();
        let d = draw_episodes(&dir, "m", Metric::Turns, 4, 4, 11, None).unwrap();
        assert_eq!(d.selection.len(), 3);
        assert_eq!(d.ranked, None);

        // And the sort is not by id: the id order is the reverse of the
        // charter's, so an id-ordered selection of any two would differ.
        let by_id: Vec<&str> = {
            let mut v = order.clone();
            v.sort();
            v
        };
        assert_ne!(
            order, by_id,
            "the old order (by id) would have selected the reverse"
        );
    }

    /// A resumed session replays under its first config — the only choice a
    /// single whole-session drive can make — and the compromise must be said
    /// on the prep rather than read later as the replay machinery failing.
    /// A single-config session carries no caveat: a caveat on the common
    /// case would train the reader to skip it.
    #[test]
    fn a_multi_config_session_carries_the_first_config_caveat() {
        use mecha_core::message::{Block, Message};
        use mecha_core::session::{Record, RunConfig, Session, SessionMeta};

        let dir = std::env::temp_dir().join(format!("mecha-probe-caveat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let meta = |id: &str| SessionMeta {
            id: id.into(),
            created_at: chrono::Utc::now(),
            provider: "local".into(),
            model: "m".into(),
            workspace: std::path::PathBuf::from("/tmp"),
            title: None,
            kind: None,
        };
        // One tool call, so the "no recorded tool calls" skip does not fire.
        let turns = |s: &Session| {
            s.append(&Record::Config(RunConfig::default())).unwrap();
            s.append_messages(&[
                Message::user("do the thing"),
                Message::assistant(vec![Block::ToolUse {
                    id: "t1".into(),
                    name: "shell".into(),
                    input: serde_json::json!({}),
                }]),
                Message {
                    harness: false,
                    planning: None,
                    tool_provenance: Default::default(),
                    role: mecha_core::message::Role::User,
                    content: vec![Block::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "ok".into(),
                        is_error: false,
                    }],
                },
                Message::assistant(vec![Block::text("done")]),
            ])
            .unwrap();
        };

        let resumed = Session::create(&dir, meta("20260101T000000-multi")).unwrap();
        turns(&resumed);
        resumed
            .append(&Record::Config(RunConfig::default()))
            .unwrap();
        let prep = prepare_episode(&resumed.path, "multi", None)
            .unwrap()
            .unwrap();
        assert_eq!(
            prep.config_caveat.as_deref(),
            // "Attached", not "resumed": a mid-session `/provider` or
            // `/mode` switch also writes a Config, and that case must not
            // read as a resume it never was.
            Some("attached 2 times; replayed under the first config")
        );

        let single = Session::create(&dir, meta("20260101T000001-single")).unwrap();
        turns(&single);
        let prep = prepare_episode(&single.path, "single", None)
            .unwrap()
            .unwrap();
        assert_eq!(prep.config_caveat, None);

        // The whole episode replays: evidence on a later attach also invalidates it.
        resumed
            .append(&Record::Config(RunConfig {
                appraisal_evidence: Some(serde_json::json!({"future_evidence":true})),
                ..Default::default()
            }))
            .unwrap();
        let skipped = prepare_episode(&resumed.path, "multi", None)
            .unwrap()
            .err()
            .expect("unsupported evidence must not grade either arm");
        assert!(skipped.contains("anticipatory evidence"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_split_never_promises_more_episodes_than_the_pool_holds() {
        for pool in 0..40usize {
            for want in [1usize, 4, 16, 33] {
                for holdout_in in [1u64, 2, 3, 7] {
                    let (h, sel) = slice_sizes(want, holdout_in, pool);
                    assert!(h + sel <= pool, "{want}/{holdout_in}/{pool} overdrew");
                    assert!(h + sel <= want, "{want}/{holdout_in}/{pool} over want");
                    assert_eq!(h == 0, pool == 0 || want == 0, "a draw must hold one back");
                }
            }
        }
    }

    // ── The split draw (row 2f, R38) ────────────────────────────────────

    /// The single-phase draw as it stood before row 2f split it, verbatim
    /// but for its name — the reference the split is held against — and
    /// before row 2e-6 ranked the selection by replay priority: its
    /// selection is ordered by headroom, then the charter rank, then the id,
    /// as it was (`pre_priority_order`, the old `selection_order` verbatim).
    /// Two edits, both forced: `prepare_episode` now takes the pass's
    /// `Ranker`, which reads the charter rank off the same appraisal the old
    /// `Stores` argument did; and that old order is inlined.
    #[allow(clippy::too_many_arguments)]
    fn single_phase_reference(
        sessions_dir: &Path,
        model: &str,
        metric: Metric,
        want: usize,
        holdout_in: u64,
        seed: u64,
        workspace: Option<&Path>,
    ) -> Result<Draw> {
        let mut listed: Vec<(SessionMeta, PathBuf)> = Session::list(sessions_dir)?;
        listed.sort_by_key(|entry| std::cmp::Reverse(entry.0.created_at));

        // Build the eligible pool: model-matched, replayable, recency-bounded.
        let pool_size = want.saturating_mul(POOL_MULTIPLE).max(want);
        let mut pool: Vec<(EpisodePrep, f64)> = Vec::new();
        let mut skipped = 0usize;
        // The same admission the diagnosis applies (`Scan::admits`), built once:
        // see the comment at its use below.
        let admission = mecha_core::runlog::Scan {
            workspace: workspace.map(std::path::Path::to_path_buf),
            ..Default::default()
        };
        // The stores an appraisal reads, once for the whole pool — four store
        // reads per draw rather than per episode, and none at all without a
        // charter, since only a charter with lines can rank anything. Only the
        // charter rank comes of it here; see `EpisodePrep::charter_rank`.
        let chartered = mecha_core::appraisal::Stores::load_if_chartered();
        let ranker = Ranker::load(sessions_dir, chrono::Utc::now());
        for (meta, path) in listed {
            if pool.len() >= pool_size {
                break;
            }
            // The header model, not per-run attribution: a whole-session replay
            // runs under one model, so the filter's job is only to keep the
            // corpus representative of the model being graded.
            if meta.model != model {
                continue;
            }
            // Scoped the same way the diagnosis was, or the two halves of one
            // night disagree about what they are talking about: `--from-workspace`
            // narrowed the brief while the draw kept re-scanning every session, so
            // a change reasoned about one job was accepted or rejected on the
            // average of four. Found in review, and it read as a working flag —
            // the diagnosis was visibly scoped, and only the arms were not.
            //
            // Prefix, matching `runlog::Scan`: a checkout's worktrees are the same
            // population as the checkout. Through `Scan::admits` itself rather
            // than a second spelling of the rule, so the draw and the diagnosis
            // cannot disagree again through a new filter — the second time this
            // paragraph's incident recurred, it was the `kind` filter: the
            // diagnosis excluded smoke-test sessions and the draw did not, so
            // real-model budget was spent replaying them (found on review).
            if !admission.admits(&meta) {
                continue;
            }
            match prepare_episode(&path, &meta.id, Some(&ranker))? {
                Ok(prep) => {
                    // Headroom off *every* outcome the session recorded, folded.
                    // `last_outcome` describes how the session ended, and an
                    // episode here is the whole session — `extract` pulls every
                    // recorded user turn and `drive_episode` replays all of them,
                    // folding each run with `absorb`. Sizing the priority signal
                    // from one run while the arms it feeds are folded over all of
                    // them is a unit mismatch, and it inverts: a resumed chat with
                    // nine error-heavy runs and a clean tenth scores zero and
                    // sorts to the bottom, so the most discriminating episode in
                    // the corpus is the one prioritised sampling drops.
                    //
                    // A session with no recorded outcome scores zero rather than
                    // being dropped: it is still drawable by the uniform half,
                    // which is the half that must not be filtered by
                    // informativeness.
                    let headroom = prep
                        .episode
                        .as_ref()
                        .map(|s| metric.headroom(s))
                        .unwrap_or(0.0);
                    pool.push((prep, headroom));
                }
                Err(_) => skipped += 1,
            }
        }

        let (holdout_n, selection_n) = slice_sizes(want, holdout_in, pool.len());

        // Uniform first. Sorted by id before the shuffle, or the seed is a lie —
        // a deterministic shuffle of a nondeterministic order is nondeterministic.
        let mut ids: Vec<String> = pool.iter().map(|(p, _)| p.id.clone()).collect();
        ids.sort();
        let held: std::collections::HashSet<String> =
            mecha_core::sample::take_uniform(ids, seed, holdout_n)
                .into_iter()
                .collect();

        let (mut holdout, mut rest): (Vec<_>, Vec<_>) =
            pool.into_iter().partition(|(p, _)| held.contains(&p.id));

        // Then the selection, by what can discriminate — and among equals, by
        // the charter line the record names (§11.1's tiebreak).
        let pre_priority_order = |a: (f64, Option<usize>, &str), b: (f64, Option<usize>, &str)| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| match (a.1, b.1) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.2.cmp(b.2))
        };
        rest.sort_by(|a, b| {
            pre_priority_order(
                (a.1, a.0.charter_rank, &a.0.id),
                (b.1, b.0.charter_rank, &b.0.id),
            )
        });
        rest.truncate(selection_n);
        // Unknown where the tiebreak could not run, a count — zero included —
        // where it did.
        let ranked = match chartered {
            mecha_core::appraisal::Chartered::Unreadable => None,
            _ => Some(
                rest.iter()
                    .filter(|(p, _)| p.charter_rank.is_some())
                    .count(),
            ),
        };

        Ok(Draw {
            selection: rest.into_iter().map(|(p, _)| p).collect(),
            holdout: std::mem::take(&mut holdout)
                .into_iter()
                .map(|(p, _)| p)
                .collect(),
            seed,
            skipped,
            ranked,
        })
    }

    /// Write clean surprises for `sessions` into the home's score ledger —
    /// the 2b-2 record the replay priority reads.
    fn surprise(home: &Path, sessions: &[&str]) {
        let dir = home.join("appraisals");
        std::fs::create_dir_all(&dir).unwrap();
        let lines: Vec<String> = sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": format!("scr-{s}"), "scored_at": "2026-01-02T00:00:00Z",
                    "appraisal_id": format!("apr-{s}"), "session_id": s,
                    "expected": "released_unchanged", "actual": "rejected",
                    "hit": false, "surprise": true, "clean": true,
                })
                .to_string()
            })
            .collect();
        std::fs::write(dir.join("scores.jsonl"), lines.join("\n") + "\n").unwrap();
    }

    /// The owner's condition on the split (R38), and 2e-6's on the ranking:
    /// neither changes the holdout. Over one store, for every metric, several
    /// seeds, sizes and holdout rates — the store holding a charter and a
    /// score ledger, so ranks and priorities are both in play — the
    /// two-phase draw's holdout is the single-phase, pre-priority draw's,
    /// element for element and in order, and so are the seed and the skips.
    /// The selection is drawn from exactly the pool minus that holdout, at
    /// the size it always was; its order is the priority's, which is the
    /// change — so on some draws it differs from the old one, or the
    /// ranking did nothing and this test proves less than it says.
    #[test]
    fn the_split_draw_holds_the_single_phase_holdout_under_the_ranking() {
        mecha_core::session::ignore_kind_env_for_tests();
        let home = crate::testenv::HomeGuard::new("probe-split");
        std::fs::write(
            home.dir.join("charter.toml"),
            "[[line]]\nid = \"top\"\ntext = \"First.\"\n\n[[line]]\nid = \"fifth\"\ntext = \"Later.\"\n",
        )
        .unwrap();
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..14u32 {
            let serves = match i % 3 {
                0 => Some("charter:top"),
                1 => Some("charter:fifth"),
                _ => None,
            };
            fixture_session(&dir, &format!("20260101T0000{i:02}-s{i:02}"), i, serves);
        }
        surprise(
            &home.dir,
            &[
                "20260101T000003-s03",
                "20260101T000008-s08",
                "20260101T000013-s13",
            ],
        );
        let fingerprint = |preps: &[EpisodePrep]| -> Vec<String> {
            preps
                .iter()
                .map(|p| {
                    format!(
                        "{}|{:?}|{:?}|{}",
                        p.id,
                        p.charter_rank,
                        p.config_caveat,
                        serde_json::to_string(&p.episode).unwrap()
                    )
                })
                .collect()
        };
        let ids =
            |preps: &[EpisodePrep]| -> Vec<String> { preps.iter().map(|p| p.id.clone()).collect() };
        let mut compared = 0;
        let mut reordered = 0;
        for metric in Metric::ALL {
            for seed in [0u64, 7, 11, 0xdead_beef] {
                for (want, holdout_in) in [(4usize, 3u64), (6, 2), (16, 3)] {
                    let old =
                        single_phase_reference(&dir, "m", metric, want, holdout_in, seed, None)
                            .unwrap();
                    let pool = draw_pool(&dir, "m", want, holdout_in, seed, None).unwrap();
                    let remainder = pool.remainder();
                    let held = pool.holdout_ids();
                    let new = pool.select(metric);
                    let at = format!("{metric:?} seed {seed} want {want}/{holdout_in}");
                    assert_eq!(fingerprint(&new.holdout), fingerprint(&old.holdout), "{at}");
                    assert_eq!((new.seed, new.skipped), (old.seed, old.skipped), "{at}");
                    assert_eq!(new.selection.len(), old.selection.len(), "{at}");
                    assert!(!old.holdout.is_empty() && !old.selection.is_empty(), "{at}");
                    // The remainder: the selection is drawn from it, and no
                    // held-out episode is in it — the same remainder the old
                    // selection was drawn from.
                    assert!(remainder.iter().all(|id| !held.contains(id)), "{at}");
                    assert!(
                        new.selection.iter().all(|p| remainder.contains(&p.id))
                            && old.selection.iter().all(|p| remainder.contains(&p.id)),
                        "{at}"
                    );
                    if ids(&new.selection) != ids(&old.selection) {
                        reordered += 1;
                    }
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, Metric::ALL.len() * 4 * 3);
        assert!(reordered > 0, "the ranking never changed a selection");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Row 2e-6's acceptance line: **the uniform holdout is unchanged by the
    /// ranking.** One store, one seed, drawn twice — before and after the
    /// owner's acts give three sessions a surprise. The holdout is the same,
    /// element for element; the selection is not, and the surprised sessions
    /// the remainder holds lead it. Fails on the old order, which read no
    /// surprise and drew the same selection both times.
    #[test]
    fn the_uniform_holdout_is_unchanged_by_the_ranking() {
        mecha_core::session::ignore_kind_env_for_tests();
        let home = crate::testenv::HomeGuard::new("probe-holdout-fixed");
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..16u32 {
            fixture_session(&dir, &format!("20260101T0000{i:02}-h{i:02}"), i, None);
        }
        let ids =
            |preps: &[EpisodePrep]| -> Vec<String> { preps.iter().map(|p| p.id.clone()).collect() };
        let before = draw_pool(&dir, "m", 8, 2, 42, None)
            .unwrap()
            .select(Metric::Turns);
        // Surprise the three latest ids of the remainder — last among the
        // known zeros, and outside a four-episode selection, so the new
        // order must move them in.
        let pool = draw_pool(&dir, "m", 8, 2, 42, None).unwrap();
        let mut remainder = pool.remainder();
        remainder.sort();
        let surprised: Vec<&str> = remainder
            .iter()
            .rev()
            .take(3)
            .rev()
            .map(String::as_str)
            .collect();
        surprise(&home.dir, &surprised);
        let after = draw_pool(&dir, "m", 8, 2, 42, None)
            .unwrap()
            .select(Metric::Turns);
        assert_eq!(ids(&after.holdout), ids(&before.holdout));
        assert_ne!(ids(&after.selection), ids(&before.selection));
        let lead: Vec<&str> = after
            .selection
            .iter()
            .take(3)
            .map(|p| p.id.as_str())
            .collect();
        let mut lead_sorted = lead.clone();
        lead_sorted.sort();
        assert_eq!(lead_sorted, surprised, "the surprised lead the selection");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A score ledger with a line that does not parse could have held any
    /// session's surprise, so the factor is unknown on every priority — not
    /// zero, which here would have been a known zero for every fixture (no
    /// owner verdict) — and the draw says so.
    #[test]
    fn an_unreadable_score_ledger_is_unknown_on_every_priority_not_zero() {
        use mecha_core::replay_priority::Factor;
        mecha_core::session::ignore_kind_env_for_tests();
        let home = crate::testenv::HomeGuard::new("probe-unknown");
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..4u32 {
            fixture_session(&dir, &format!("20260101T0000{i:02}-u{i:02}"), i, None);
        }
        let known = draw_pool(&dir, "m", 4, 2, 1, None)
            .unwrap()
            .select(Metric::Turns);
        assert!(known.selection.iter().chain(&known.holdout).all(|p| p
            .priority
            .as_ref()
            .unwrap()
            .known_zero));
        std::fs::create_dir_all(home.dir.join("appraisals")).unwrap();
        std::fs::write(home.dir.join("appraisals/scores.jsonl"), "{torn\n").unwrap();
        let pool = draw_pool(&dir, "m", 4, 2, 1, None).unwrap();
        assert!(
            pool.caveats.iter().any(|c| c.contains("surprises unknown")),
            "{:?}",
            pool.caveats
        );
        let d = pool.select(Metric::Turns);
        for p in d.selection.iter().chain(&d.holdout) {
            let priority = p.priority.as_ref().unwrap();
            assert!(
                priority.unknown.contains(&Factor::Surprises),
                "{priority:?}"
            );
            assert!(!priority.known_zero, "{priority:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The first phase reads no metric, so the remainder a diagnostician is
    /// shown is the same whichever metric the proposal later names.
    #[test]
    fn the_remainder_is_fixed_before_the_metric_is_known() {
        mecha_core::session::ignore_kind_env_for_tests();
        let home = crate::testenv::HomeGuard::new("probe-remainder");
        let dir = home.dir.join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..9u32 {
            fixture_session(&dir, &format!("20260101T0000{i:02}-r{i:02}"), i, None);
        }
        let pool = draw_pool(&dir, "m", 6, 3, 5, None).unwrap();
        let remainder = pool.remainder();
        let held = pool.holdout_ids();
        assert_eq!(held.len(), 2);
        for metric in Metric::ALL {
            let again = draw_pool(&dir, "m", 6, 3, 5, None).unwrap();
            assert_eq!(again.remainder(), remainder);
            let d = again.select(metric);
            let drawn: Vec<String> = d.holdout.iter().map(|p| p.id.clone()).collect();
            assert_eq!(drawn, held, "{metric:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
