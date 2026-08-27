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
use mecha_core::replay_run::{drive, replay_registry, OnDivergence};
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
}

/// Load one session as a replayable episode. `Err(reason)` in the inner
/// result is a skip — never evidence for either arm.
pub fn prepare_episode(path: &Path, id: &str) -> Result<Result<EpisodePrep, String>> {
    // One read. This runs over the whole pool — four times the wanted episode
    // count — every nightly, and `load` + `run_configs` + `episode_stats` were
    // three full reads and parses of the same file to answer questions one
    // walk answers together.
    let read = match Session::read(path) {
        Ok(t) => t,
        Err(e) => return Ok(Err(format!("session unreadable: {e:#}"))),
    };
    let trajectory = extract(&read.convo.messages);
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
    Ok(Ok(EpisodePrep {
        id: id.to_string(),
        trajectory,
        recorded,
        episode: read.episode,
    }))
}

/// Newest-first replayable episodes for one model, up to `want`, from the
/// default session store. Returns the preps and how many candidates were
/// looked at and skipped.
/// The two slices a candidate is judged on, drawn separately on purpose.
pub struct Draw {
    /// Drawn by [`Metric::headroom`]: the episodes that can discriminate.
    pub selection: Vec<EpisodePrep>,
    /// Drawn uniformly from the same eligible pool, and drawn **first**.
    pub holdout: Vec<EpisodePrep>,
    /// Printed, because a sample nobody can redraw is one nobody can check.
    pub seed: u64,
    pub skipped: usize,
}

/// How much wider than the draw the eligible pool has to be.
///
/// **If the pool equals the draw there is no draw**: "prioritised" and
/// "uniform" both degenerate to "all of them", and the holdout stops being
/// independent of the selection because there was nothing to choose between.
/// Four is enough for the two draws to differ and small enough that the walk
/// stays bounded, which is `runlog::Scan`'s constraint.
const POOL_MULTIPLE: usize = 4;

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

/// Draw a selection and a holdout for one candidate.
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
pub fn draw_episodes(
    sessions_dir: &Path,
    model: &str,
    metric: Metric,
    want: usize,
    holdout_in: u64,
    seed: u64,
) -> Result<Draw> {
    let mut listed: Vec<(SessionMeta, PathBuf)> = Session::list(sessions_dir)?;
    listed.sort_by_key(|entry| std::cmp::Reverse(entry.0.created_at));

    // Build the eligible pool: model-matched, replayable, recency-bounded.
    let pool_size = want.saturating_mul(POOL_MULTIPLE).max(want);
    let mut pool: Vec<(EpisodePrep, f64)> = Vec::new();
    let mut skipped = 0usize;
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
        match prepare_episode(&path, &meta.id)? {
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

    // Then the selection, by what can discriminate.
    rest.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    rest.truncate(selection_n);

    Ok(Draw {
        selection: rest.into_iter().map(|(p, _)| p).collect(),
        holdout: std::mem::take(&mut holdout)
            .into_iter()
            .map(|(p, _)| p)
            .collect(),
        seed,
        skipped,
    })
}
/// What one arm of one episode produced.
pub struct ArmOutcome {
    pub stats: RunStats,
    /// The replay left the recording and was stopped. The stats above cover
    /// only the prefix it tracked; the caller drops the episode.
    pub diverged: bool,
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
    let registry = match replay_registry(
        &recorded.tools,
        prepared.agent.registry(),
        Some(&crate::setup::surface_only_registry()),
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
    )?;
    let cx = RunContext::new(tool_ctx, approver)
        .with_cancel(cancel)
        .with_compact_at(agent_cfg.compact_at_tokens);
    match drive(&agent, &cx, &prep.trajectory).await {
        Ok(report) => Ok(Ok(ArmOutcome {
            diverged: report.stopped_early,
            stats: report.stats,
        })),
        Err(e) => Ok(Err(format!("replay failed: {e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
