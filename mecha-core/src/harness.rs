//! Harness rumination: the candidate store behind `mecha harness`, and the
//! override layer an accepted change rides in.
//!
//! This is the persistence half of the self-improvement loop. `diagnose.rs`
//! proposes, `replay_run.rs` measures, `candidate.rs` judges — and until this
//! module existed, the judgement evaporated when the process exited, which is
//! why the loop had never run unattended: a nightly that proposes changes
//! nobody reads is worse than no nightly. Here a proposal becomes a record,
//! the record carries the measurement it was decided from, and an accepted
//! change becomes one entry in an overrides file that any run can read and
//! one command can revert.
//!
//! ## The override layer, and why the user always wins
//!
//! `overrides.toml` is applied to a [`Config`] **after defaults and before
//! any file layer** ([`apply_accepted_overrides`], called from
//! `Config::load` / `load_global`). Layering is assignment, so a key the
//! user names in `config.toml` overwrites the accepted value — an override
//! only ever fills space the user left empty. That is §13.3's "reversible"
//! made structural: reverting is deleting a line, and nothing the loop does
//! can pin a value against the user's own file.
//!
//! ## The closed set
//!
//! [`OverrideKey`] is the same closed set `mecha eval --ab-config` accepts,
//! defined once here so the measurement arm and the acceptance arm cannot
//! drift apart — a candidate measured under one applier and applied under
//! another would be accepted on evidence about a different change. An open
//! set would let a proposer reach settings whose effect replay cannot
//! measure, or worse, security settings, which are never a measurement's to
//! decide. An unknown key in the overrides file is **skipped loudly and
//! never applied** — the file is machine-written, but a boundary that trusts
//! its writer is not one.

use crate::candidate::{ChangeClass, Judgement, Metric};
use crate::config::Config;
use crate::message::Effort;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

// ─── The closed set ─────────────────────────────────────────────────────────

/// The configuration knobs an automated proposer may move. Closed on
/// purpose; see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideKey {
    CompactAtTokens,
    MaxTurns,
    MaxOutputTokens,
    Effort,
}

impl OverrideKey {
    pub const ALL: [OverrideKey; 4] = [
        OverrideKey::CompactAtTokens,
        OverrideKey::MaxTurns,
        OverrideKey::MaxOutputTokens,
        OverrideKey::Effort,
    ];

    pub fn parse(key: &str) -> Option<OverrideKey> {
        match key {
            "compact_at_tokens" => Some(OverrideKey::CompactAtTokens),
            "max_turns" => Some(OverrideKey::MaxTurns),
            "max_output_tokens" => Some(OverrideKey::MaxOutputTokens),
            "effort" => Some(OverrideKey::Effort),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OverrideKey::CompactAtTokens => "compact_at_tokens",
            OverrideKey::MaxTurns => "max_turns",
            OverrideKey::MaxOutputTokens => "max_output_tokens",
            OverrideKey::Effort => "effort",
        }
    }

    /// The keys, for an error message that names the whole set.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The subsystems an arm may switch **off** — the on/off half of the closed
/// set, beside [`OverrideKey`]'s value-carrying half.
///
/// An ablation here is a subsystem that is *structurally absent* from a run:
/// chosen by a switch, recorded on `RunConfig::levers_off`, never a sentence
/// in a prompt (`docs/EXPERIMENT-DESIGN.md` Part II (PR #156 until it lands), *The switch set* and its
/// decision that one closed set of levers sits beside the knobs). **The record is the
/// switch, not the effect**, for every variant alike: `levers_off` says
/// which switches the run was built with thrown, and the realised surface —
/// whether a charter block or a rules block actually reached the prompt,
/// whether `compact` or an MCP tool was actually registered — is on the same
/// `RunConfig` in `system_prompt` and `tools`. An experiment pairs arms on
/// the switch set and checks the realised surface from those two fields; a
/// field that mixed the two meanings (found on review, when one variant
/// briefly recorded its effect) could not be read either way. The set is
/// closed for
/// the same reason `OverrideKey` is — an arm that can vary something the
/// record does not name is a confound that nothing can read back — and it
/// is defined once, here, so `mecha eval`'s bare arm and an experiment's
/// cannot disagree about what "bare" means: eval forces every lever off
/// except the two it allows as opt-ins, through [`Lever::bare`] — plus the
/// one lever `bare` never throws on its own, see there.
///
/// **A variant here is a switch that exists**, not a wish. A lever with no
/// off position would make the record say "absent" of something that ran,
/// which is the one lie a confound record must not tell. Of the
/// dispositions *The switch set* lists, the appraiser's pass is still
/// absent here: it runs only under `mecha sessions appraise --appraise`,
/// never inside a run, so there is nothing for a per-run lever to remove;
/// and `sensors_in_brief` is a *stage* lever (a `ruminate` input), recorded
/// on a trial's manifest rather than on a run, and waits for that store.
///
/// Serialised by name — the same names [`Lever::as_str`] answers — because
/// the record is an append-only wire format: a reader that meets a name it
/// does not know reads the *whole* recorded set as unknown rather than a
/// shorter one, since a lever silently dropped from `levers_off` reads as
/// "on". That is `session::lenient_levers`' job, tested beside `RunConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lever {
    /// `--no-mcp`: configured MCP servers are not connected. (Whether any
    /// *were* configured is `tools`' to say.)
    Mcp,
    /// `--no-learned-rules`: the learned-rules block is not built into the
    /// prefix. (An empty store leaves this lever on; `system_prompt` shows
    /// the block either way.)
    LearnedRules,
    /// `--no-hooks`: configured `[[hook]]` commands do not run.
    Hooks,
    /// `--no-outbox`: `[outbox] tools` execute unstaged.
    Outbox,
    /// `--no-fallback`: a provider failure is not retried against a fallback.
    /// A provider whose `fallbacks` list is empty has none to retry against
    /// either, and that is not recorded here — a list's emptiness is not a
    /// switch; the same holds for empty `[[hook]]` and `[outbox] tools`
    /// lists. Known blind spots of this field, named so a reader pairing
    /// arms does not assume the record covers them.
    Fallback,
    /// `--no-messages`, or `[messages] enabled = false`: no mailbox is
    /// attached. The config half is folded in because it is a config
    /// *switch*, the same shape as the three `[agent]` ones — not because
    /// the surface cannot show it: `message_send` is registered in the same
    /// block as the route, so `tools` does say (found on review, both ways).
    Messages,
    /// `--no-skills`: the level-1 skill block is not built.
    Skills,
    /// `--no-charter`: the charter block is not built.
    Charter,
    /// `--no-compact-tool`: the `compact` tool is not registered by request.
    /// It may also be absent because it could not exist (no provider
    /// `context_window`, or a `[tools]` list that excludes it); that is
    /// `tools`' to say, and this lever stays on.
    CompactTool,
    /// `--no-step-escalation`, or `[agent] step_escalation = false`.
    StepEscalation,
    /// `no_rules`, which only `mecha eval` sets: the approval rules file is
    /// not loaded.
    ApprovalRules,
    /// `--no-boredom`, or `[agent] boredom = false`.
    Boredom,
    /// `--no-compact-validate`, or `[agent] compact_validate = false`.
    CompactValidate,
    /// `--no-predictive-compaction`, or `[agent] predictive_compaction =
    /// false`: the compaction *trigger* fires on the reported size only,
    /// never on the forecast of the next request. The threshold stays, and
    /// so do the predictor's other two uses — the per-turn tool-output
    /// budget and the headroom forecast the model is shown. "No compaction
    /// on the forecast", not "no forecast".
    PredictiveCompaction,
    /// `--no-carried-state`, or `[agent] carried_state = false`: a tool's
    /// state (the plan) does not ride across a compaction.
    CarriedState,
}

impl Lever {
    /// Every lever, in a stable order — the order `levers_off` is recorded
    /// in, so two records with the same set compare byte-for-byte.
    /// **Membership here is not compiler-enforced**: `as_str`'s exhaustive
    /// match forces a fourteenth variant to be *named*, and nothing forces
    /// it into this array, whose length is typed by hand — and a lever
    /// missing from `ALL` can never be recorded, is dropped by
    /// `RunConfig::of`, and silently leaves `mecha eval`'s bare arm (found
    /// on review). The test `all_names_every_variant_serde_knows` closes
    /// it from the derive: serde's unknown-variant error lists every
    /// variant, and the test asserts this array covers that list.
    pub const ALL: [Lever; 15] = [
        Lever::Mcp,
        Lever::LearnedRules,
        Lever::Hooks,
        Lever::Outbox,
        Lever::Fallback,
        Lever::Messages,
        Lever::Skills,
        Lever::Charter,
        Lever::CompactTool,
        Lever::StepEscalation,
        Lever::ApprovalRules,
        Lever::Boredom,
        Lever::CompactValidate,
        Lever::PredictiveCompaction,
        Lever::CarriedState,
    ];

    pub fn parse(name: &str) -> Option<Lever> {
        Self::ALL.into_iter().find(|l| l.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Lever::Mcp => "mcp",
            Lever::LearnedRules => "learned_rules",
            Lever::Hooks => "hooks",
            Lever::Outbox => "outbox",
            Lever::Fallback => "fallback",
            Lever::Messages => "messages",
            Lever::Skills => "skills",
            Lever::Charter => "charter",
            Lever::CompactTool => "compact_tool",
            Lever::StepEscalation => "step_escalation",
            Lever::ApprovalRules => "approval_rules",
            Lever::Boredom => "boredom",
            Lever::CompactValidate => "compact_validate",
            Lever::PredictiveCompaction => "predictive_compaction",
            Lever::CarriedState => "carried_state",
        }
    }

    /// The levers, for an error message that names the whole set.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The bare arm: every lever off except `allow`, in [`Lever::ALL`]'s
    /// order — **except [`Lever::ApprovalRules`], which no preset throws.**
    /// A `forbid` is the operator's standing word, and a preset that lifted
    /// it for one run is the silently-degrading-guard shape; the lever is in
    /// the set so the record can *name* the one caller that does lift it
    /// (`mecha eval`, whose fixture workspaces justify it), and that caller
    /// throws it explicitly and on its own line. A future `exp` arm over a
    /// real workspace gets "bare" and keeps the rules (found on review).
    /// Otherwise this is what `mecha eval` runs (`--mcp` and `--ab-rules` are
    /// its two opt-ins) and what an experiment's `bare` preset means, from
    /// one definition — a second spelling is how the measurement arm and
    /// the acceptance arm silently stop being comparable.
    pub fn bare(allow: &[Lever]) -> Vec<Lever> {
        Self::ALL
            .into_iter()
            .filter(|l| !allow.contains(l) && *l != Lever::ApprovalRules)
            .collect()
    }
}

/// A `KEY=VALUE` change, parsed and value-validated. The value is kept as the
/// canonical string it parsed from, so one shape serialises to the overrides
/// file and re-parses on load through the same validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigChange {
    pub key: OverrideKey,
    pub value: String,
}

/// The key half of a `KEY=VALUE`, if it names something this loop can override.
///
/// Separate from [`parse_change`] because the two questions have different
/// answers and only one of them is about the proposer. `max_turns=0` names a
/// real knob with a value that was refused; `context.auto_compact=true` names
/// nothing at all. The first is a config change a human can correct, the
/// second is a request that someone add a setting — and until this existed
/// both were stored `class: Config, status: staged`, which reads to a reviewer
/// as a config change waiting to be applied when it is a feature request for a
/// knob that has never existed.
pub fn names_override_key(spec: &str) -> Option<OverrideKey> {
    OverrideKey::parse(spec.split_once('=')?.0.trim())
}

/// Parse a proposal's `KEY=VALUE` into the closed set, validating the value.
///
/// Refusal is the common case and the safe one: a proposal whose change does
/// not parse here is not a config change this loop can measure or apply, so
/// it goes to a human instead.
pub fn parse_change(spec: &str) -> Result<ConfigChange> {
    let (key, value) = spec
        .split_once('=')
        .with_context(|| format!("expected KEY=VALUE, got `{spec}`"))?;
    let key = key.trim();
    let value = value.trim();
    let key = OverrideKey::parse(key).with_context(|| {
        format!(
            "`{key}` is not in the closed override set ({})",
            OverrideKey::names()
        )
    })?;
    let canonical = match key {
        OverrideKey::CompactAtTokens => {
            let n: u64 = value
                .parse()
                .with_context(|| format!("compact_at_tokens takes a number, got `{value}`"))?;
            anyhow::ensure!(
                n >= 1000,
                "compact_at_tokens below 1000 would compact on nearly every turn"
            );
            n.to_string()
        }
        OverrideKey::MaxTurns => {
            let n: u32 = value
                .parse()
                .with_context(|| format!("max_turns takes a number, got `{value}`"))?;
            anyhow::ensure!(n >= 1, "max_turns must be at least 1");
            n.to_string()
        }
        OverrideKey::MaxOutputTokens => {
            let n: u64 = value
                .parse()
                .with_context(|| format!("max_output_tokens takes a number, got `{value}`"))?;
            anyhow::ensure!(n >= 1, "max_output_tokens must be at least 1");
            n.to_string()
        }
        OverrideKey::Effort => value
            .parse::<Effort>()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .as_str()
            .to_string(),
    };
    Ok(ConfigChange {
        key,
        value: canonical,
    })
}

impl ConfigChange {
    /// Apply onto an [`crate::config::AgentConfig`]. The value was validated
    /// at parse time; a value that no longer parses (a hand-edited file) is
    /// an error the caller reports, never a silent skip-and-apply-half.
    pub fn apply_to_agent(&self, agent: &mut crate::config::AgentConfig) -> Result<()> {
        match self.key {
            OverrideKey::CompactAtTokens => agent.compact_at_tokens = Some(self.value.parse()?),
            OverrideKey::MaxTurns => agent.max_turns = self.value.parse()?,
            OverrideKey::MaxOutputTokens => agent.max_output_tokens = Some(self.value.parse()?),
            OverrideKey::Effort => {
                agent.effort = Some(self.value.parse().map_err(|e| anyhow::anyhow!("{e}"))?)
            }
        }
        Ok(())
    }

    pub fn spec(&self) -> String {
        format!("{}={}", self.key.as_str(), self.value)
    }
}

// ─── Candidates ─────────────────────────────────────────────────────────────

/// Waiting on a human — unmeasurable by replay, or measured without a verdict
/// strong enough to act alone.
pub const STATUS_STAGED: &str = "staged";
/// Cleared the full gate; the override is live.
pub const STATUS_ACCEPTED: &str = "accepted";
/// Measured worse, or a guardrail moved, or a human said no.
pub const STATUS_REJECTED: &str = "rejected";
/// Was accepted, then a human took the override back out.
pub const STATUS_REVERTED: &str = "reverted";

/// One proposed harness change, from diagnosis through disposal.
///
/// The record keeps everything the decision was made from — the evidence
/// brief, the prediction, the paired tallies — because "is this loop actually
/// helping" has to be answerable from the store rather than from impression.
/// Statuses are strings, not an enum, on the wire-format rule: a status this
/// version does not know must not make the record unreadable to `list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessCandidate {
    pub id: String,
    pub created_at: String,
    pub class: ChangeClass,
    /// The change as the diagnostician wrote it — `KEY=VALUE` for config.
    pub change: String,
    /// The metric it predicted this would reduce.
    pub metric: Metric,
    pub rationale: String,
    /// The counters brief the diagnostician reasoned from. Machine-authored.
    pub evidence: String,
    /// Model whose corpus was diagnosed, and whose sessions were replayed.
    #[serde(default)]
    pub model: Option<String>,
    /// `staged` | `accepted` | `rejected` | `reverted`.
    pub status: String,
    #[serde(default)]
    pub measurement: Option<Measurement>,
    #[serde(default)]
    pub resolved_at: Option<String>,
    /// Why it sits where it does, human-readable.
    #[serde(default)]
    pub reason: Option<String>,
}

impl HarnessCandidate {
    /// Whether this candidate still needs a person to look at it.
    pub fn pending(&self) -> bool {
        self.status == STATUS_STAGED
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TallyRecord {
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
}

impl std::fmt::Display for TallyRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}+ {}- {}=", self.wins, self.losses, self.ties)
    }
}

/// What the counterfactual replay measured, kept whole on the candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub measured_at: String,
    pub model: String,
    /// `accept` | `propose` | `reject` — the gate's own verdict, which is not
    /// the candidate's status: a `propose` verdict stages for a human.
    pub disposition: String,
    /// The gate's reason, empty for accept.
    pub reason: String,
    pub selection: TallyRecord,
    pub holdout: TallyRecord,
    pub work_baseline: u64,
    pub work_candidate: u64,
    /// Session ids paired and judged, selection slice first.
    pub episodes: Vec<String>,
    /// Which of those were the holdout, and the seed the uniform draw used.
    ///
    /// Recorded rather than recomputed, because it can no longer *be*
    /// recomputed. The split used to be `is_holdout(id, holdout_in)` — a pure
    /// function of the episode id, so any later reader could reconstruct which
    /// episodes confirmed a result. Drawing uniformly from a pool makes the
    /// split depend on the corpus as it stood at measurement time, which is
    /// gone the moment another session is written. Without these two fields
    /// "which episodes was this confirmed on" stops being answerable, which is
    /// the property the drawing was introduced to protect: a sample nobody can
    /// redraw is a sample nobody can check.
    #[serde(default)]
    pub holdout_episodes: Vec<String>,
    #[serde(default)]
    pub seed: u64,
    /// How many of the selection the charter tiebreak ranked
    /// (`GOAL-SYSTEM-DESIGN.md` §11.1; `harness_probe::selection_order`).
    /// The seed and the corpus pin the holdout; the selection's order among
    /// equal headroom is the rank, and the rank is read off the *present*
    /// stores — the charter, and the outbox, question, front-door and
    /// learning stores a session's errors are signed from — so the same
    /// seed over the same session corpus can tie differently once a draft
    /// is resolved or a line re-ranked. `episodes` keeps the resulting
    /// order; this says whether the tiebreak had anything to decide, and
    /// it used to live on a stderr line nobody keeps (found on review).
    /// `None` on a record from before the field: unknown, not zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranked: Option<usize>,
    /// Sessions dropped because an arm left the recording — a divergent
    /// replay's stats describe a truncated run, and scoring one would let a
    /// behaviour-visible change be graded on the fraction it tracked.
    /// **Bare ids, and they stay that way**: `episodes`/`holdout_episodes`'s
    /// own contract (a sample nobody can redraw is one nobody can check)
    /// holds here too, and anything resolving an entry back to a session
    /// path must not have to parse annotation out of it first.
    pub diverged: Vec<String>,
    /// What the replay was compromising on, per episode that carried a
    /// compromise — "id — attached N times; replayed under the first
    /// config" — **whatever became of the episode**, skipped ones included
    /// (the caveat is computed at prepare time, before any arm drives): a
    /// dropped one's divergence may say more about the compromise than
    /// about the change, and a cleanly paired one feeds the tally that
    /// gates acceptance,
    /// which is the more consequential place for the decider reading
    /// `mecha harness show` to know the replay was compromising. Beside
    /// `diverged` rather than folded into it, so the ids stay joinable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_caveats: Vec<String>,
    /// Why an arm left the recording, and which arm — one entry per arm
    /// that diverged.
    ///
    /// **Not joinable one-to-one against [`Self::diverged`], and that is
    /// deliberate.** Each arm is recorded as it finishes, so an episode
    /// whose baseline diverged and whose candidate then failed to replay at
    /// all contributes an entry here while landing in `skipped` rather than
    /// in `diverged`. Recording later would lose that reason entirely,
    /// which is the loss this field exists to prevent — so the entry stays
    /// and the contract is "an arm that diverged", not "a dropped episode".
    ///
    /// Beside [`Self::diverged`] rather than folded into it, the same shape
    /// and for the same reason as [`Self::replay_caveats`]: those ids are a
    /// joinable list by contract and must not need parsing.
    ///
    /// **The two arms want opposite responses, and the old bool could not
    /// tell them apart.** A baseline divergence is the replay failing on
    /// that episode; a candidate-only divergence is the change altering
    /// behaviour — evidence the change does something, on precisely the
    /// episodes where it bites. Dropping both as one pile is individually
    /// correct (a truncated arm cannot be scored) and collectively biased:
    /// on 2026-09-01 a `compact_at_tokens=16384` candidate paired only on
    /// episodes far below 16384 tokens, so the sample it scored was the
    /// sample where it provably did nothing, and the gate reported a thin
    /// sample rather than a censored one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub divergence_detail: Vec<Divergence>,
    /// Sessions that could not be replayed at all (unreadable, no recorded
    /// calls, tool surface moved). Never evidence for either arm.
    pub skipped: usize,
}

/// Which arm of a paired replay left the recording.
///
/// A closed enum written into an append-only store, so it is a wire format:
/// an unknown variant on load degrades rather than failing the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    Baseline,
    Candidate,
    /// A variant written by a newer build than this one. Never counted as
    /// either arm — unknown is not clean.
    #[serde(other)]
    Unrecognised,
}

impl Arm {
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Baseline => "baseline",
            Arm::Candidate => "candidate",
            Arm::Unrecognised => "unrecognised",
        }
    }
}

/// One arm of one episode leaving the recording, and why.
///
/// **Structured, not a formatted line.** The first version of this was
/// `Vec<String>` rendered as "id — baseline arm: reason" and read back with
/// `contains("— baseline arm:")` in two places. Losing the em dash, or
/// rewording the prefix, would have silently zeroed the baseline count and
/// made `mecha harness show` assert "every divergence was the candidate arm
/// — the change moved behaviour" for a run in which every divergence was the
/// baseline. A format string is not a data structure.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Divergence {
    pub episode: String,
    pub arm: Arm,
    pub reason: String,
}

impl Divergence {
    /// The one-line arm split for a measurement that produced no pairs.
    ///
    /// Extracted so it can be tested: it lands in the candidate's stored
    /// reason, which on the all-diverged path is the whole durable record
    /// of the run, and it was previously built inline where no test could
    /// reach it — including the case where the two `Arm` literals are
    /// swapped at the call site, which makes the reporter assert the
    /// opposite of what happened.
    pub fn arms_summary(all: &[Divergence], skipped: usize) -> String {
        let baseline = Self::baseline_count(all);
        let candidate = Self::candidate_count(all);
        if all.is_empty() {
            return String::new();
        }
        if baseline == 0 && skipped == 0 {
            return format!(
                "; all {candidate} divergence(s) were the CANDIDATE arm and nothing was \
                 skipped — the change moved behaviour on every episode that diverged, \
                 which is a finding about the change, not a missing measurement"
            );
        }
        format!(
            "; {baseline} baseline-arm and {candidate} candidate-arm divergence(s) — a \
             baseline divergence says the replay is unreliable here, a candidate one says \
             the change moved behaviour; read the split before concluding either"
        )
    }

    /// How many of these were the baseline arm — the count both the report
    /// and the renderer key on, derived rather than parsed.
    pub fn baseline_count(all: &[Divergence]) -> usize {
        all.iter().filter(|d| d.arm == Arm::Baseline).count()
    }
    pub fn candidate_count(all: &[Divergence]) -> usize {
        all.iter().filter(|d| d.arm == Arm::Candidate).count()
    }
}

/// Which episodes a measurement drew, where they went, and what it could not
/// use. One value rather than five arguments: they are produced together by
/// one draw and read together by one reader, and threading them separately is
/// how the seed came to be `eprintln!`'d and never stored.
pub struct Drawn {
    /// Every episode paired and judged, selection slice first.
    pub episodes: Vec<String>,
    pub holdout_episodes: Vec<String>,
    pub seed: u64,
    /// See [`Measurement::ranked`].
    pub ranked: usize,
    pub diverged: Vec<String>,
    /// See [`Measurement::replay_caveats`].
    pub replay_caveats: Vec<String>,
    /// See [`Measurement::divergence_detail`].
    pub divergence_detail: Vec<Divergence>,
    pub skipped: usize,
}

impl Measurement {
    pub fn record(
        judgement: &Judgement,
        model: &str,
        measured_at: String,
        drawn: Drawn,
    ) -> Measurement {
        let Drawn {
            episodes,
            holdout_episodes,
            seed,
            ranked,
            diverged,
            replay_caveats,
            divergence_detail,
            skipped,
        } = drawn;
        use crate::candidate::Disposition;
        let (disposition, reason) = match &judgement.disposition {
            Disposition::Accept => ("accept", String::new()),
            Disposition::Propose(r) => ("propose", r.clone()),
            Disposition::Reject(r) => ("reject", r.clone()),
        };
        let tally = |t: &crate::candidate::Tally| TallyRecord {
            wins: t.wins,
            losses: t.losses,
            ties: t.ties,
        };
        Measurement {
            measured_at,
            model: model.to_string(),
            disposition: disposition.to_string(),
            reason,
            selection: tally(&judgement.selection),
            holdout: tally(&judgement.holdout),
            work_baseline: judgement.work_baseline,
            work_candidate: judgement.work_candidate,
            episodes,
            holdout_episodes,
            seed,
            ranked: Some(ranked),
            diverged,
            replay_caveats,
            divergence_detail,
            skipped,
        }
    }
}

// ─── The store ──────────────────────────────────────────────────────────────

/// One accepted override: the line in `overrides.toml`, with its provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AcceptedOverride {
    /// Canonical [`OverrideKey`] string. Kept as a string on the wire-format
    /// rule; [`OverrideKey::parse`] is re-applied on every load.
    pub key: String,
    pub value: String,
    /// The candidate this acceptance came from.
    pub candidate: String,
    pub accepted_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OverridesFile {
    #[serde(default, rename = "override")]
    overrides: Vec<AcceptedOverride>,
}

/// `~/.mecha/learning/harness/` — candidates and the overrides file.
pub struct HarnessStore {
    root: PathBuf,
}

impl HarnessStore {
    /// Beside the learning store's other artifacts, so `MECHA_LEARNING_DIR`
    /// relocates the whole learning surface together.
    pub fn default_root() -> Result<PathBuf> {
        Ok(crate::learning::LearningStore::default_root()?.join("harness"))
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<HarnessStore> {
        let root = root.into();
        crate::create_private_dir(&root.join("candidates"))
            .with_context(|| format!("creating {}", root.display()))?;
        Ok(HarnessStore { root })
    }

    pub fn open_default() -> Result<HarnessStore> {
        Self::open(Self::default_root()?)
    }

    /// Open at the default location only if it already exists — for read
    /// paths that must not create state as a side effect, on
    /// `LearningStore::open_existing_default`'s rule. `Backlog::read` is
    /// the reader that needed it: two reads per run created
    /// `learning/harness/candidates` on a machine that had never
    /// ruminated (found on review).
    pub fn open_existing_default() -> Option<HarnessStore> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(HarnessStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write (or rewrite) one candidate, atomically — a nightly pass and a
    /// `list` in a terminal must never meet over a half-written file.
    pub fn write(&self, c: &HarnessCandidate) -> Result<()> {
        let path = self.root.join("candidates").join(format!("{}.json", c.id));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(c)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Every candidate, oldest first. Unreadable files are skipped with a
    /// warning — one bad record must not hide the store.
    pub fn all(&self) -> Result<Vec<HarnessCandidate>> {
        let dir = self.root.join("candidates");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match serde_json::from_str(&std::fs::read_to_string(&path)?) {
                Ok(c) => out.push(c),
                Err(e) => tracing::warn!("skipping unreadable candidate {}: {e}", path.display()),
            }
        }
        out.sort_by(|a: &HarnessCandidate, b: &HarnessCandidate| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Find one candidate by id or unique prefix. Ambiguity is an error
    /// rather than a guess, same as session lookup.
    pub fn find(&self, id: &str) -> Result<HarnessCandidate> {
        let all = self.all()?;
        let matches: Vec<&HarnessCandidate> = all.iter().filter(|c| c.id.starts_with(id)).collect();
        match matches.len() {
            0 => anyhow::bail!("no candidate matching `{id}`"),
            1 => Ok(matches[0].clone()),
            n => anyhow::bail!(
                "`{id}` matches {n} candidates: {}",
                matches
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn overrides_path(&self) -> PathBuf {
        self.root.join("overrides.toml")
    }

    /// The currently accepted overrides, in file order.
    pub fn overrides(&self) -> Result<Vec<AcceptedOverride>> {
        let path = self.overrides_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let file: OverridesFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(file.overrides)
    }

    /// Install an override, replacing any earlier one on the same key.
    /// Returns what it replaced, so the acceptance can record the reversal.
    pub fn set_override(&self, ov: AcceptedOverride) -> Result<Option<AcceptedOverride>> {
        let _lock = self.lock()?;
        let mut all = self.overrides()?;
        let replaced = all
            .iter()
            .position(|o| o.key == ov.key)
            .map(|i| all.remove(i));
        all.push(ov);
        self.write_overrides(&all)?;
        Ok(replaced)
    }

    /// Remove the override on `key`, returning it if there was one. Removal
    /// returns the key to whatever the user's own config layers say — the
    /// candidate files keep the history.
    pub fn remove_override(&self, key: &str) -> Result<Option<AcceptedOverride>> {
        let _lock = self.lock()?;
        let mut all = self.overrides()?;
        let removed = all.iter().position(|o| o.key == key).map(|i| all.remove(i));
        if removed.is_some() {
            self.write_overrides(&all)?;
        }
        Ok(removed)
    }

    fn write_overrides(&self, all: &[AcceptedOverride]) -> Result<()> {
        let path = self.overrides_path();
        let file = OverridesFile {
            overrides: all.to_vec(),
        };
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(&file)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Advisory flock over override mutations — a nightly acceptance and a
    /// human `revert` are both read-modify-write on one file.
    fn lock(&self) -> Result<std::fs::File> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the harness store");
        }
        Ok(file)
    }

    /// Mint a candidate id: sortable timestamp plus a sub-second tail, so
    /// two candidates minted close together cannot collide.
    pub fn mint_id() -> String {
        let now = chrono::Utc::now();
        format!(
            "hc-{}-{:04x}",
            now.format("%Y%m%dT%H%M%S"),
            (now.timestamp_subsec_nanos() ^ std::process::id()) & 0xffff
        )
    }
}

// ─── The config layer ───────────────────────────────────────────────────────

/// Apply the accepted overrides at the default location onto a
/// just-defaulted [`Config`]. Called from `Config::load` / `load_global`
/// before any file layer merges, so the user's own config always wins.
///
/// Best-effort by design: an unreadable or malformed overrides file warns
/// and applies nothing, because a performance knob must never be the reason
/// every run fails to start. Contrast the sandbox, where silent degradation
/// removes a boundary — an override that fails to apply leaves the config
/// exactly as the user wrote it.
pub fn apply_accepted_overrides(cfg: &mut Config) {
    let Ok(root) = HarnessStore::default_root() else {
        return;
    };
    apply_overrides_file(cfg, &root.join("overrides.toml"));
}

/// The testable half: apply one overrides file onto a config.
pub fn apply_overrides_file(cfg: &mut Config, path: &Path) {
    if !path.exists() {
        return;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("harness overrides unreadable ({}): {e}", path.display());
            return;
        }
    };
    let file: OverridesFile = match toml::from_str(&text) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(
                "harness overrides malformed ({}): {e} — applying none",
                path.display()
            );
            return;
        }
    };
    for ov in file.overrides {
        // Re-validated on every load: the closed set is enforced where the
        // value is *used*, not only where it was written.
        let change = match parse_change(&format!("{}={}", ov.key, ov.value)) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "harness override `{}={}` skipped: {e:#} (from candidate {})",
                    ov.key,
                    ov.value,
                    ov.candidate
                );
                continue;
            }
        };
        if let Err(e) = change.apply_to_agent(&mut cfg.agent) {
            tracing::warn!(
                "harness override `{}` failed to apply: {e:#}",
                change.spec()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    /// `ranked` is on the wire beside `seed` and `holdout_episodes`, and a
    /// record from before it reads unknown, never zero.
    #[test]
    fn a_measurement_records_how_many_of_the_selection_the_tiebreak_ranked() {
        use crate::candidate::{Disposition, Judgement, Tally};
        let judgement = Judgement {
            disposition: Disposition::Accept,
            selection: Tally::default(),
            holdout: Tally::default(),
            work_baseline: 3,
            work_candidate: 3,
        };
        let m = super::Measurement::record(
            &judgement,
            "m",
            "2026-09-04T00:00:00Z".into(),
            super::Drawn {
                episodes: vec!["a".into(), "b".into()],
                holdout_episodes: vec!["b".into()],
                seed: 7,
                ranked: 1,
                diverged: vec![],
                replay_caveats: vec![],
                divergence_detail: vec![],
                skipped: 0,
            },
        );
        assert_eq!(m.ranked, Some(1));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"ranked\":1"), "{json}");
        let back: super::Measurement = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ranked, Some(1));

        // Before the field: the key is absent, and the reading is unknown.
        let older = json.replace(",\"ranked\":1", "");
        assert_ne!(older, json);
        let back: super::Measurement = serde_json::from_str(&older).unwrap();
        assert_eq!(back.ranked, None);
        assert_eq!(back.seed, 7);
    }

    use super::*;

    /// The serialised name and the parsed name are the same string for every
    /// lever — the record is written through serde and read back through
    /// `parse` by the lenient loader, and a lever spelt two ways would be
    /// recorded off and read as on.
    #[test]
    fn every_lever_round_trips_by_the_same_name() {
        for lever in Lever::ALL {
            let wire = serde_json::to_string(&lever).unwrap();
            assert_eq!(wire, format!("\"{}\"", lever.as_str()), "{lever:?}");
            assert_eq!(Lever::parse(lever.as_str()), Some(lever));
            assert_eq!(serde_json::from_str::<Lever>(&wire).unwrap(), lever);
        }
        assert_eq!(
            Lever::parse("appraisal"),
            None,
            "the readout is not a lever (Part II, *The switch set*)"
        );
        assert_eq!(Lever::parse("MCP"), None, "names are exact");
    }

    /// `ALL` must list every variant, and the compiler does not check that:
    /// the exhaustive matches force a new variant to be *named*, not
    /// *listed*, and a lever absent from `ALL` can never be recorded or
    /// forced. Serde's unknown-variant error names every variant the
    /// derive knows, so the derive maintains the reference list for free.
    #[test]
    fn all_names_every_variant_serde_knows() {
        let err = serde_json::from_str::<Lever>("\"no_such_lever\"")
            .unwrap_err()
            .to_string();
        // `unknown variant `no_such_lever`, expected one of `mcp`, `learned_rules`, …`
        let listed: std::collections::BTreeSet<&str> = err
            .split("expected one of ")
            .nth(1)
            .expect("serde names the variants")
            // `… `compact_validate` at line 1 column 15`: the position is
            // serde_json's, not a variant.
            .split(" at line")
            .next()
            .unwrap()
            .split(',')
            .map(|s| s.trim().trim_matches(|c| c == '`' || c == ' ' || c == '.'))
            .filter(|s| !s.is_empty())
            .collect();
        let all: std::collections::BTreeSet<&str> = Lever::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(
            listed, all,
            "a variant serde knows is missing from ALL, or ALL names a ghost"
        );
    }

    /// `ALL` is the closed set: exhaustive over the enum, no duplicates, and
    /// the set the bare arm is built from — minus the one lever no preset
    /// may throw, so "bare" cannot quietly become "unguarded".
    #[test]
    fn the_lever_set_is_closed_and_the_bare_arm_keeps_the_operators_rules() {
        // Exhaustive: a new variant fails to compile here until it is listed.
        for lever in Lever::ALL {
            match lever {
                Lever::Mcp
                | Lever::LearnedRules
                | Lever::Hooks
                | Lever::Outbox
                | Lever::Fallback
                | Lever::Messages
                | Lever::Skills
                | Lever::Charter
                | Lever::CompactTool
                | Lever::StepEscalation
                | Lever::ApprovalRules
                | Lever::Boredom
                | Lever::CompactValidate
                | Lever::PredictiveCompaction
                | Lever::CarriedState => {}
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        assert!(Lever::ALL.iter().all(|l| seen.insert(*l)), "no duplicates");
        let bare = Lever::bare(&[]);
        assert!(
            !bare.contains(&Lever::ApprovalRules),
            "a forbid is never lifted by preset"
        );
        assert_eq!(
            bare.len(),
            Lever::ALL.len() - 1,
            "and everything else is off"
        );
        assert!(
            Lever::ALL.contains(&Lever::ApprovalRules),
            "but the record can still name it"
        );
        let with_opt_ins = Lever::bare(&[Lever::Mcp, Lever::LearnedRules]);
        assert_eq!(with_opt_ins.len(), Lever::ALL.len() - 3);
        assert!(!with_opt_ins.contains(&Lever::Mcp));
        assert!(!with_opt_ins.contains(&Lever::LearnedRules));
        assert!(
            with_opt_ins.contains(&Lever::Boredom),
            "an opt-in opts into nothing else"
        );
        // The two halves of the closed set do not overlap by name.
        for lever in Lever::ALL {
            assert!(
                OverrideKey::parse(lever.as_str()).is_none(),
                "{lever:?} is a knob too"
            );
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("mecha-harness-test")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_closed_set_refuses_everything_outside_it() {
        // The keys eval's --ab-config accepts, and nothing else.
        assert!(parse_change("compact_at_tokens=24000").is_ok());
        assert!(parse_change("max_turns=30").is_ok());
        assert!(parse_change("max_output_tokens=8000").is_ok());
        assert!(parse_change("effort=low").is_ok());

        // The settings a proposer must never reach.
        for hostile in [
            "sandbox=none",
            "trifecta=allow",
            "outbox.tools=",
            "context_window=999999",
            "temperature=2.0",
        ] {
            assert!(parse_change(hostile).is_err(), "{hostile} must be refused");
        }
        // And a shape that is not KEY=VALUE at all.
        assert!(parse_change("just prose").is_err());
    }

    #[test]
    fn values_are_validated_not_just_typed() {
        assert!(parse_change("compact_at_tokens=1").is_err());
        assert!(parse_change("max_turns=0").is_err());
        assert!(parse_change("max_turns=notanumber").is_err());
        assert!(parse_change("effort=extreme").is_err());
        // Whitespace tolerated, value canonicalised.
        let c = parse_change(" effort = LOW ").unwrap();
        assert_eq!(c.value, "low");
    }

    #[test]
    fn overrides_apply_beneath_the_user_and_unknown_keys_are_skipped() {
        let dir = temp_dir();
        let path = dir.join("overrides.toml");
        std::fs::write(
            &path,
            r#"
[[override]]
key = "compact_at_tokens"
value = "24000"
candidate = "hc-test"
accepted_at = "2026-08-22T00:00:00Z"

[[override]]
key = "sandbox"
value = "none"
candidate = "hc-evil"
accepted_at = "2026-08-22T00:00:00Z"

[[override]]
key = "max_turns"
value = "notanumber"
candidate = "hc-corrupt"
accepted_at = "2026-08-22T00:00:00Z"
"#,
        )
        .unwrap();

        let mut cfg = Config::default();
        apply_overrides_file(&mut cfg, &path);
        // The known, valid key applied.
        assert_eq!(cfg.agent.compact_at_tokens, Some(24000));
        // The unknown key was skipped, not smuggled anywhere.
        // The corrupt value was skipped, and the default survived.
        assert_eq!(cfg.agent.max_turns, 40);
    }

    #[test]
    fn a_malformed_overrides_file_applies_nothing() {
        let dir = temp_dir();
        let path = dir.join("overrides.toml");
        std::fs::write(&path, "this is not toml [[[").unwrap();
        let mut cfg = Config::default();
        let before = cfg.agent.max_turns;
        apply_overrides_file(&mut cfg, &path);
        assert_eq!(cfg.agent.max_turns, before);
    }

    #[test]
    fn set_and_remove_override_round_trip_and_replacement_is_returned() {
        let dir = temp_dir();
        let store = HarnessStore::open(&dir).unwrap();

        let first = AcceptedOverride {
            key: "compact_at_tokens".into(),
            value: "24000".into(),
            candidate: "hc-a".into(),
            accepted_at: "2026-08-22T00:00:00Z".into(),
        };
        assert!(store.set_override(first.clone()).unwrap().is_none());

        // Same key again: replaced, and the replacement is reported so the
        // acceptance can record it.
        let second = AcceptedOverride {
            key: "compact_at_tokens".into(),
            value: "20000".into(),
            candidate: "hc-b".into(),
            ..first.clone()
        };
        let replaced = store.set_override(second).unwrap();
        assert_eq!(replaced.unwrap().candidate, "hc-a");

        let all = store.overrides().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].value, "20000");

        // And the file it wrote applies.
        let mut cfg = Config::default();
        apply_overrides_file(&mut cfg, &store.overrides_path());
        assert_eq!(cfg.agent.compact_at_tokens, Some(20000));

        // Revert: removed, returned, and the file no longer applies it.
        let removed = store.remove_override("compact_at_tokens").unwrap();
        assert_eq!(removed.unwrap().value, "20000");
        let mut cfg = Config::default();
        apply_overrides_file(&mut cfg, &store.overrides_path());
        assert_eq!(cfg.agent.compact_at_tokens, None);
        // Removing what is not there is a no-op, not an error.
        assert!(store
            .remove_override("compact_at_tokens")
            .unwrap()
            .is_none());
    }

    #[test]
    fn candidates_round_trip_and_an_unknown_status_still_loads() {
        let dir = temp_dir();
        let store = HarnessStore::open(&dir).unwrap();
        let c = HarnessCandidate {
            id: "hc-20260822T000000-0001".into(),
            created_at: "2026-08-22T00:00:00Z".into(),
            class: ChangeClass::Config,
            change: "compact_at_tokens=24000".into(),
            metric: Metric::CutShort,
            rationale: "runs are dying at the ceiling".into(),
            evidence: "runs: 64".into(),
            model: Some("qwen3.6-35b-a3b".into()),
            status: STATUS_STAGED.into(),
            measurement: None,
            resolved_at: None,
            reason: None,
        };
        store.write(&c).unwrap();
        let read = store.find("hc-2026").unwrap();
        assert_eq!(read.change, c.change);
        assert!(read.pending());

        // A status minted by a future version must not unread the record.
        let mut future = c.clone();
        future.id = "hc-20260823T000000-0002".into();
        future.status = "escalated".into();
        store.write(&future).unwrap();
        assert_eq!(store.all().unwrap().len(), 2);
    }
}

#[cfg(test)]
mod divergence_arm_tests {
    use super::*;

    fn d(episode: &str, arm: Arm) -> Divergence {
        Divergence {
            episode: episode.into(),
            arm,
            reason: "recorded call #0 was `a`, not `b`".into(),
        }
    }

    /// **Review finding.** The arm was encoded into a formatted line and
    /// recovered with `contains("— baseline arm:")` in two places, with no
    /// shared constant and no test. Rewording the prefix — or losing the em
    /// dash — would silently zero the baseline count and make the reporter
    /// assert "every divergence was the candidate arm" for a run in which
    /// every divergence was the baseline.
    ///
    /// Counting from the enum cannot break that way, and this test is what
    /// says so.
    #[test]
    fn the_arm_split_is_counted_from_data_not_from_prose() {
        let all = vec![
            d("a", Arm::Baseline),
            d("b", Arm::Candidate),
            d("c", Arm::Candidate),
        ];
        assert_eq!(Divergence::baseline_count(&all), 1);
        assert_eq!(Divergence::candidate_count(&all), 2);
        // The rendered form is free to change without moving either count.
        assert_eq!(Arm::Baseline.as_str(), "baseline");
        assert_eq!(Arm::Candidate.as_str(), "candidate");
    }

    /// **The swapped-literal case.** Every other test here would still pass
    /// if the two `Arm::` literals were exchanged at the call site — and
    /// that swap is exactly what makes the reporter assert the opposite of
    /// what happened. Pinning the summary to the counts is what catches it.
    #[test]
    fn the_summary_follows_the_arms_and_would_catch_a_swap() {
        // All candidate, nothing skipped: the strong claim is allowed.
        let all_candidate = vec![d("a", Arm::Candidate), d("b", Arm::Candidate)];
        let s = Divergence::arms_summary(&all_candidate, 0);
        assert!(
            s.contains("all 2 divergence(s) were the CANDIDATE arm"),
            "{s}"
        );
        assert!(
            s.contains("moved behaviour on every episode that diverged"),
            "{s}"
        );

        // Swap the arms and the strong claim must disappear.
        let all_baseline = vec![d("a", Arm::Baseline), d("b", Arm::Baseline)];
        let s = Divergence::arms_summary(&all_baseline, 0);
        assert!(
            !s.contains("CANDIDATE arm"),
            "a swap must not keep the claim: {s}"
        );
        assert!(s.contains("2 baseline-arm and 0 candidate-arm"), "{s}");
    }

    /// `skipped > 0` withdraws the strong claim: episodes never driven are
    /// not evidence the change moved anything.
    #[test]
    fn skipped_episodes_withdraw_the_strong_claim() {
        let all = vec![d("a", Arm::Candidate)];
        let s = Divergence::arms_summary(&all, 9);
        assert!(!s.contains("every episode that diverged"), "{s}");
        assert!(s.contains("0 baseline-arm and 1 candidate-arm"), "{s}");
    }

    /// Nothing diverged, nothing to say.
    #[test]
    fn no_divergences_render_no_summary() {
        assert!(Divergence::arms_summary(&[], 0).is_empty());
        assert!(Divergence::arms_summary(&[], 5).is_empty());
    }

    /// A closed enum written into an append-only store is a wire format: a
    /// variant from a newer build degrades rather than failing the record,
    /// and is never silently counted as one of the known arms.
    #[test]
    fn an_unknown_arm_from_a_newer_build_is_neither_arm() {
        let parsed: Divergence =
            serde_json::from_str(r#"{"episode":"e","arm":"third_arm","reason":"r"}"#)
                .expect("an unknown variant must not fail the record");
        assert_eq!(parsed.arm, Arm::Unrecognised);
        let all = vec![parsed];
        assert_eq!(Divergence::baseline_count(&all), 0);
        assert_eq!(Divergence::candidate_count(&all), 0);
    }

    /// Round-trips through the store, since this rides in a persisted
    /// `Measurement`.
    #[test]
    fn a_divergence_survives_the_store() {
        let all = vec![d("ep-1", Arm::Baseline), d("ep-2", Arm::Candidate)];
        let json = serde_json::to_string(&all).unwrap();
        let back: Vec<Divergence> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].arm, Arm::Baseline);
        assert_eq!(back[1].episode, "ep-2");
        assert!(back[0].reason.contains("recorded call #0"));
    }
}
