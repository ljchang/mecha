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
    appraise: Option<(
        &mecha_core::appraisal::Stores,
        chrono::DateTime<chrono::Utc>,
    )>,
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
    let config_caveat = (read.configs.len() > 1).then(|| {
        format!(
            "attached {} times; replayed under the first config",
            read.configs.len()
        )
    });
    // The rank rides on the appraisal the sessions readout would build for
    // this transcript, from the stores the caller loaded once: the plan's
    // `serves:` and the sensored-line attribution both land a charter line
    // on the errors, and `charter_rank` reads the highest. Only a charter
    // with lines can rank anything, so an empty one skips the appraisal.
    let charter_rank = appraise.and_then(|(stores, created_at)| {
        let charter = stores.charter.as_ref().filter(|c| !c.is_empty())?;
        let drafts = stores.drafts_of(id);
        let built = mecha_core::appraisal::for_transcript(
            &read,
            id,
            created_at.to_rfc3339(),
            stores.records(&drafts),
            None,
        )?;
        mecha_core::appraisal::charter_rank(&built.appraisal, charter)
    });
    Ok(Ok(EpisodePrep {
        id: id.to_string(),
        trajectory,
        recorded,
        episode: read.episode,
        config_caveat,
        charter_rank,
    }))
}

/// The selection's order, over (headroom, charter rank, id): what can
/// discriminate first; among equals, the episode whose signed error names
/// the higher-ranked charter line (`GOAL-SYSTEM-DESIGN.md` §11.1 — a signed
/// error against the top line replays before one against the fifth), with
/// an unranked episode after every ranked one; then the id, so the order is
/// total and the seed is not a lie. Pure, because the old order — headroom
/// then id — and this one agree on every episode but the tied ones, which
/// is exactly the case a test has to construct.
pub fn selection_order(
    a: (f64, Option<usize>, &str),
    b: (f64, Option<usize>, &str),
) -> std::cmp::Ordering {
    b.0.partial_cmp(&a.0)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| match (a.1, b.1) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| a.2.cmp(b.2))
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
    /// How many of the selection carry a charter rank — recorded on the
    /// measurement beside the seed (`Measurement::ranked`), because the
    /// rank is read off inputs the seed and the corpus do not pin: the
    /// charter, whose order is the rank, and the stores a session's errors
    /// are signed from as they stand at the draw. A resolved draft or a
    /// re-ranked line redraws the ties.
    pub ranked: usize,
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
    let stores = mecha_core::appraisal::Stores::load_if_chartered();
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
        match prepare_episode(
            &path,
            &meta.id,
            stores.as_ref().map(|s| (s, meta.created_at)),
        )? {
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
    rest.sort_by(|a, b| {
        selection_order(
            (a.1, a.0.charter_rank, &a.0.id),
            (b.1, b.0.charter_rank, &b.0.id),
        )
    });
    rest.truncate(selection_n);
    let ranked = rest
        .iter()
        .filter(|(p, _)| p.charter_rank.is_some())
        .count();

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
/// What one arm of one episode produced.
pub struct ArmOutcome {
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
    )?;
    let cx = RunContext::new(tool_ctx, approver)
        .with_cancel(cancel)
        .with_compact_at(agent_cfg.compact_at_tokens);
    match drive(&agent, &cx, &prep.trajectory).await {
        Ok(report) => Ok(Ok(ArmOutcome {
            // `stopped_early` is the authority on *whether* — it is what the
            // cancel token did — and the probe supplies the *why*. A run
            // stopped early with no recorded reason still counts as
            // diverged, with the reason named as unrecorded rather than
            // silently dropped: unknown is not clean.
            divergence: report.stopped_early.then(|| {
                divergence
                    .reason()
                    .unwrap_or_else(|| "left the recording; no reason recorded".into())
            }),
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
        use mecha_core::message::{Block, Message};
        use mecha_core::session::{Record, RunConfig, Session, SessionMeta};

        let dir = std::env::temp_dir().join(format!("mecha-probe-ws-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
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

    /// The tiebreak alone, on tuples: equal headroom orders by rank, an
    /// unranked episode after every ranked one, and the id last — and
    /// headroom still outranks rank, so the tiebreak never promotes an
    /// uninformative episode over an informative one.
    #[test]
    fn ties_in_headroom_break_on_the_charter_rank_then_the_id() {
        use std::cmp::Ordering::*;
        assert_eq!(selection_order((1.0, None, "z"), (0.5, Some(0), "a")), Less);
        assert_eq!(
            selection_order((0.5, Some(0), "z"), (0.5, Some(1), "a")),
            Less
        );
        assert_eq!(selection_order((0.5, Some(3), "z"), (0.5, None, "a")), Less);
        assert_eq!(selection_order((0.5, None, "a"), (0.5, None, "b")), Less);
        assert_eq!(
            selection_order((0.5, Some(2), "a"), (0.5, Some(2), "a")),
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
        let make = |id: &str, serves: Option<&str>| {
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
                    role: mecha_core::message::Role::User,
                    content: results,
                },
                Message::assistant(vec![Block::text("done")]),
            ])
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
        make("20260101T000002-top", Some("charter:top"));
        make("20260101T000001-fifth", Some("charter:fifth"));
        make("20260101T000000-none", None);

        let d = draw_episodes(&dir, "m", Metric::Turns, 3, 3, 11, None).unwrap();
        assert_eq!(d.selection.len() + d.holdout.len(), 3);
        let order: Vec<&str> = d.selection.iter().map(|p| p.id.as_str()).collect();
        // The charter's order, whichever one the seed held out.
        let expected = [
            "20260101T000002-top",
            "20260101T000001-fifth",
            "20260101T000000-none",
        ];
        let mut cursor = 0;
        for id in &order {
            let at = expected[cursor..]
                .iter()
                .position(|e| e == id)
                .unwrap_or_else(|| panic!("selection out of charter order: {order:?}"));
            cursor += at + 1;
        }
        assert_eq!(order.len(), 2, "{order:?}");
        assert_eq!(
            d.ranked,
            order.iter().filter(|id| !id.ends_with("none")).count(),
            "ranked counts the selected episodes whose error named a line"
        );

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
}
