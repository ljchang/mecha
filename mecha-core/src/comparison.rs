//! Every counterfactual comparison, stored.
//!
//! `docs/APPRAISAL-WIRING-DESIGN.md` X1 extended by O4, row 1g. A steer probe
//! (`sessions appraise --probe`) used to compute whether an owner's steer was
//! load-bearing and then discard the answer; `mecha validate` and the
//! `learn` proposal gate kept their verdicts only in ledgers keyed to a rule
//! set, with no situation, goal or call attached. Each of those verdicts cost
//! a model run. This store keeps them — one [`Comparison`] per comparison, in
//! `~/.mecha/comparisons/comparisons.jsonl` — keyed so the pre-action marker
//! (X2) and plan-time comparison (N1) can one day look a situation up.
//!
//! **Built for K arms, not two.** Today's comparisons have two arms each (the
//! recording against its unsteered replay; rules-free against the rules;
//! current rules against a candidate), and phases 2, 3 and 5 add point-wise,
//! in-run and mid-run comparisons with more. So an arm is a role, the policy
//! it carried and its own outcome under the deciding validator, and the
//! comparison's [`Verdict`] is **derived** from the arms by one function
//! ([`Verdict::of`]) — never authored by a caller, so a stored verdict cannot
//! disagree with the arms beside it.
//!
//! **Closed sets only.** Situation keys are registry names and matched keys
//! ([`Situation`]); the goal is its kind word ([`GoalKind`]), not its id; the
//! call is a tool name and the *declared* argument names it was given
//! ([`CallClass`]), never an argument value; arms, validators and verdicts
//! are enums. No prose — not even the harness's own inconclusive reasons —
//! so a record carries nothing a model authored.
//!
//! **Provenance is the store's own door.** [`ComparisonStore::record`] is the
//! only way to write, and it takes a [`Provenance`] read from the recording:
//! only a session whose recorded taint is clean at its end (the learning
//! gate's rule, [`crate::learning::classify_origin`] — unknown is never clean)
//! *and* whose recorded tool surface is still readable
//! ([`crate::surface::Fidelity::Matches`] — before the surface store, 12 of 13
//! probes were inconclusive for reasons that said nothing about the probe)
//! writes a row. A refusal is returned as a value, so a caller counts it
//! rather than mistaking it for a write.
//!
//! **It is a wire format.** Every enum degrades to `Unknown` on a variant a
//! newer build wrote, optional fields default, and a torn line costs that
//! line — counted, so a reader can say the store was not fully read.

use crate::goal::GoalKind;
use crate::learning::{classify_origin, Origin};
use crate::message::ToolSpec;
use crate::situation::Situation;
use crate::surface::Fidelity;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What produced a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// `sessions appraise --probe`: was an owner's steer or denial
    /// load-bearing?
    SteerProbe,
    /// `mecha validate`: do the deployed rules do better than none?
    Validation,
    /// The `mecha learn --propose/--auto` gate: does a candidate rule set do
    /// better than the deployed one?
    Gate,
    /// `mecha sessions compare` (row 2d-1, O1): K policies driven a short
    /// horizon from an informative decision point of a recorded session,
    /// graded by the owner's recorded verdict. One kind per point kind, so
    /// `Summary::by_kind` reads the mix without a new field: an owner's
    /// steer, an owner's denial, a failed check, a draft the owner rewrote
    /// before sending, a draft the owner rejected, and a surprise (a
    /// forecast the run's own record missed).
    PointSteer,
    PointDenial,
    PointCheck,
    PointEditedDraft,
    PointRejectedDraft,
    PointSurprise,
    /// `mecha learn --compare-sources` (row 2e-1, R25): at an intervention
    /// the reflector reflected on, the recorded prompt with no rules against
    /// the same with the reflector's lesson and with the session appraisal's
    /// lessons — shadow, measurement only (`lesson_source`).
    LessonSource,
    /// A kind a newer build wrote (phases 3 and 5 add in-run and mid-run
    /// comparisons).
    #[default]
    #[serde(other)]
    Unknown,
}

/// The validator that decided every arm's outcome.
///
/// Kept on the record because the validators are not equally trustworthy
/// (inventory §10): a structural verdict may decide, a model judge never
/// should, and a reader must be able to leave the judge-decided rows out.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Validator {
    /// `counterfactual::steer_verdict`: did the continuation track the
    /// recording from the steer point, without the steer.
    StructuralSteer,
    /// `counterfactual::denial_verdict`: did the continuation avoid repeating
    /// the refused call.
    StructuralDenial,
    /// A model judge graded the answers (followup validation). Never a
    /// deciding vote by the design's own table; recorded so it can be
    /// excluded.
    Judge,
    /// An owner-supplied artifact fixture against pinned gold (mismatch
    /// probes).
    ArtifactGold,
    /// `counterfactual::draft_verdict` on a draft the owner rewrote and
    /// sent: an arm passes by drafting the released text and fails by
    /// drafting the text the owner rewrote, both in `draft_form`; anything
    /// else is a draft the owner never judged (row 2d-1).
    ReleasedDraft,
    /// The same validator on a draft the owner rejected: an arm fails by
    /// drafting the rejected text and passes by ending without drafting.
    RejectedDraft,
    /// No structural validator can pose this point's question — a surprise
    /// with no owner act behind it, or a check that is the agent's own or
    /// cannot be re-run on a branch that executes nothing. **The arms are
    /// not driven and none is recorded**, so the verdict derives
    /// `Inconclusive`: stored so the point is counted, never judged (R27).
    Unposed,
    #[default]
    #[serde(other)]
    Unknown,
}

/// Which policy an arm ran.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The run as it actually happened, the owner's intervention included.
    /// Its outcome is `Pass` by construction: the owner's intervention *is*
    /// the target the structural validators grade against.
    Recorded,
    /// The recorded prefix branched without the intervention, under the
    /// system prompt as recorded (rules block and all). At a point-wise
    /// comparison's draft or check point the owner's act is not in the
    /// transcript at all, so this is simply the policy the run ran under.
    WithoutIntervention,
    /// The recorded prompt with every rules block removed.
    RulesFree,
    /// The deployed rules, rendered for the recorded run's situation.
    Rules,
    /// A candidate that exists only in memory until its gate disposes of
    /// it: a rule set (the learn gate), or a harness config change applied
    /// over the recorded config (R26's point-wise half, row 2d-2).
    Candidate,
    /// The recorded prompt with no rules but the reflector's lesson for this
    /// intervention, in the learned-rules frame (row 2e-1).
    ReflectorLesson,
    /// The recorded prompt with no rules but the lessons of the session's
    /// clean text appraisal, in the same frame (row 2e-1).
    AppraisalLesson,
    #[default]
    #[serde(other)]
    Unknown,
}

/// One arm's outcome under the deciding validator.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    Fail,
    /// Driven, and the validator could not pose its question.
    Inconclusive,
    #[default]
    #[serde(other)]
    Unknown,
}

impl From<&crate::counterfactual::ProbeVerdict> for Outcome {
    fn from(v: &crate::counterfactual::ProbeVerdict) -> Outcome {
        use crate::counterfactual::ProbeVerdict as V;
        match v {
            V::Pass => Outcome::Pass,
            V::Fail => Outcome::Fail,
            V::Inconclusive(_) => Outcome::Inconclusive,
        }
    }
}

/// One arm of a comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arm {
    #[serde(default)]
    pub role: Role,
    /// What the arm carried, by hash: [`crate::learning::rules_hash`] of the
    /// rules block, on `RunConfig::rules_hash`'s convention — an arm that
    /// carried no block is the hash of the empty string ([`Arm::no_block`]),
    /// and `None` is *unknown* (an arm replaying a recording from before
    /// that field), never "no rules".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default)]
    pub outcome: Outcome,
}

impl Arm {
    pub fn new(role: Role, policy: Option<String>, outcome: Outcome) -> Arm {
        Arm {
            role,
            policy,
            outcome,
        }
    }

    /// The policy of an arm that carried no rules block: recorded and
    /// empty, which is a different fact from unknown.
    pub fn no_block() -> Option<String> {
        Some(crate::learning::rules_hash(""))
    }
}

/// What the comparison concluded, derived from its arms by [`Verdict::of`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// The validator separated the arms: some passed and some failed.
    /// `Comparison::preferred` names the ones that passed. For a steer probe
    /// this is *load-bearing* (the recording passes, the unsteered replay
    /// does not); for validation it is *improved* or *regressed*, by which
    /// arm is preferred.
    Separated,
    /// Every arm reached the same outcome — all passed or all failed. For a
    /// steer probe, *not load-bearing*: the run would have got there anyway.
    Tied,
    /// At least one arm posed no question. Not evidence either way.
    Inconclusive,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Verdict {
    /// The verdict and the preferred arms (indices into `arms`), from the
    /// arms alone. Fewer than two arms is no comparison and reads
    /// `Inconclusive`; an arm this build cannot read makes the whole
    /// comparison inconclusive rather than guessing its outcome.
    pub fn of(arms: &[Arm]) -> (Verdict, Vec<usize>) {
        if arms.len() < 2
            || arms
                .iter()
                .any(|a| matches!(a.outcome, Outcome::Inconclusive | Outcome::Unknown))
        {
            return (Verdict::Inconclusive, Vec::new());
        }
        let passed: Vec<usize> = arms
            .iter()
            .enumerate()
            .filter(|(_, a)| a.outcome == Outcome::Pass)
            .map(|(i, _)| i)
            .collect();
        if passed.is_empty() || passed.len() == arms.len() {
            (Verdict::Tied, Vec::new())
        } else {
            (Verdict::Separated, passed)
        }
    }
}

/// The call a comparison's decision point is about: a registry tool name and
/// the argument names it was given, restricted to the names that tool's
/// recorded schema declares — the argument *shape*, never a value.
///
/// For a denial it is the refused call; for a steer, the last call before
/// the steer — the results the steer rode in beside, which is what the
/// model was mid-way through ([`Situation::focus`] names the same tool).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallClass {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl CallClass {
    /// `None` when `name` is not on the recorded surface: a name the
    /// registry does not own is not a key. An argument name the schema does
    /// not declare is dropped — a model authors the keys of its own input,
    /// and only the declared ones are a closed set.
    pub fn of(name: &str, input: &serde_json::Value, specs: &[ToolSpec]) -> Option<CallClass> {
        let spec = specs.iter().find(|s| s.name == name)?;
        let declared = spec
            .input_schema
            .get("properties")
            .and_then(serde_json::Value::as_object);
        let mut args: Vec<String> = input
            .as_object()
            .map(|o| {
                o.keys()
                    .filter(|k| declared.is_some_and(|d| d.contains_key(k.as_str())))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        args.sort();
        Some(CallClass {
            tool: name.to_string(),
            args,
        })
    }
}

/// Where a comparison's evidence lives. Pointers, never content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointers {
    /// The recorded session the comparison branched from.
    #[serde(default)]
    pub session_id: String,
    /// The message the intervention rides in, in the loaded list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_index: Option<usize>,
    /// The recording's call index at the decision point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_index: Option<usize>,
    /// The reflection probed, for validation and the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_id: Option<String>,
    /// The proposal the gate wrote about the candidate measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<String>,
    /// The text appraisal whose lessons an arm carried (row 2e-1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appraisal_id: Option<String>,
    /// The recorded tool surface the replay was faithful to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_hash: Option<String>,
    /// Identity of the recorded inputs (`ProbePrep::input_hash` in the CLI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hash: Option<String>,
}

/// One counterfactual comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub id: String,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub kind: Kind,
    /// The recorded situation of the decision point: the tool window, the
    /// trigger, and the matched surface and workspace. A reader keys on
    /// [`Situation::scope`] (the trigger is recorded, never a key). `None`
    /// is **unknown** — a reflection mined before situations were recorded —
    /// and never "everywhere": an empty-keyed `Situation` is a standing
    /// scope that matches every run, so an unknown one must not be spelled
    /// as it, or a lesson about one tool would read as evidence about all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub situation: Option<Situation>,
    /// The kind of goal the session was anchored to; `None` when it named
    /// none — absent is recorded, never guessed. The session's **last**
    /// anchor (`Transcript::convo.goal_anchor`): the transcript keeps no
    /// anchor positions the way it keeps `config_positions`, so a session
    /// re-anchored after the decision point stamps the later kind.
    #[serde(default)]
    pub goal_kind: Option<GoalKind>,
    #[serde(default)]
    pub call: Option<CallClass>,
    #[serde(default)]
    pub arms: Vec<Arm>,
    #[serde(default)]
    pub validator: Validator,
    #[serde(default)]
    pub verdict: Verdict,
    /// Indices into `arms` of the arms the validator preferred; empty unless
    /// `verdict` is `Separated`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred: Vec<usize>,
    #[serde(default)]
    pub pointers: Pointers,
    /// The model every arm drove — comparisons are only comparable within
    /// one.
    #[serde(default)]
    pub model: String,
}

impl Comparison {
    /// A comparison over `arms`, its verdict derived from them.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: Kind,
        situation: Option<Situation>,
        goal_kind: Option<GoalKind>,
        call: Option<CallClass>,
        arms: Vec<Arm>,
        validator: Validator,
        pointers: Pointers,
        model: &str,
    ) -> Comparison {
        let (verdict, preferred) = Verdict::of(&arms);
        Comparison {
            id: format!("cmp-{}", uuid::Uuid::new_v4()),
            at: Utc::now(),
            kind,
            situation,
            goal_kind,
            call,
            arms,
            validator,
            verdict,
            preferred,
            pointers,
            model: model.to_string(),
        }
    }
}

/// What the recording says about whether a comparison drawn from it may be
/// kept. Read from the transcript by [`Provenance::of_transcript`]; the
/// fields are public so a caller that already holds both answers can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    /// Of the session as a whole — the taint covering its last message.
    pub origin: Origin,
    /// Of the recorded tool surface. On a trace probe the specs are the blob
    /// loaded *by* the recorded hash, so `Matches` reduces to "the surface
    /// store still holds it"; on an artifact probe they are rebuilt from the
    /// fixture's registry today, so it keeps [`Fidelity`]'s full meaning,
    /// "today's surface still hashes to the recorded value". Both fail closed.
    pub surface: Fidelity,
}

impl Provenance {
    /// The session's provenance: taint at its end (taint only grows, so the
    /// last checkpoint covers everything before it; no checkpoint after the
    /// last message is unknown, which is untrusted), and whether the surface
    /// the recording cites is the one the store can still hand back.
    pub fn of_transcript(
        transcript: &crate::session::Transcript,
        tools_hash: Option<&str>,
        recorded_specs: &[ToolSpec],
    ) -> Provenance {
        let messages = transcript.convo.messages.len();
        let end = (messages > 0)
            .then(|| transcript.taint_timeline.covering(messages - 1))
            .flatten();
        Provenance {
            origin: classify_origin(end),
            surface: Fidelity::of(tools_hash, recorded_specs),
        }
    }

    /// Whether a comparison from this recording may be written.
    pub fn admit(self) -> Result<(), Refusal> {
        if self.origin != Origin::Clean {
            return Err(Refusal::NotClean);
        }
        if self.surface != Fidelity::Matches {
            return Err(Refusal::SurfaceUnreadable);
        }
        Ok(())
    }
}

/// Why a comparison was not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Third-party content entered the session, or its taint is unknown.
    NotClean,
    /// The recorded tool surface is gone, changed, or was never kept.
    SurfaceUnreadable,
}

/// What [`ComparisonStore::record`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Written,
    Refused(Refusal),
}

/// The append-only comparison store.
pub struct ComparisonStore {
    root: PathBuf,
}

struct StoreLock {
    _file: std::fs::File,
}

impl ComparisonStore {
    /// `~/.mecha/comparisons`, under [`crate::work::mecha_home`] (which
    /// honours `MECHA_HOME`, so a trial home keeps its own).
    pub fn default_root() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("comparisons"))
    }

    /// Open, creating the directory. The write paths open before they drive
    /// any arm, so a store that cannot be created fails the command before it
    /// has paid for a verdict it could not keep.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        crate::create_private_dir(&root).with_context(|| format!("creating {}", root.display()))?;
        Ok(ComparisonStore { root })
    }

    /// [`Self::open`] at the default root.
    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_root()?)
    }

    /// Open at the default location only if it already exists — a read path
    /// must not create the store it is about to report on.
    pub fn open_existing_default() -> Option<Self> {
        let root = Self::default_root().ok()?;
        root.is_dir().then_some(ComparisonStore { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn ledger(&self) -> PathBuf {
        self.root.join("comparisons.jsonl")
    }

    fn lock(&self) -> Result<StoreLock> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(".lock"))?;
        // SAFETY: flock on an fd we own, held open by the returned guard.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).context("locking the comparison store");
        }
        Ok(StoreLock { _file: file })
    }

    /// The one write door. A comparison from a session that is not clean, or
    /// whose recorded surface is not readable, is refused and nothing is
    /// written; an admitted one is appended under the lock and synced. An
    /// I/O failure is an `Err`, never a quiet refusal: a caller that paid a
    /// model run for this verdict must hear that it was not kept.
    pub fn record(&self, provenance: Provenance, comparison: &Comparison) -> Result<Recorded> {
        if let Err(refusal) = provenance.admit() {
            return Ok(Recorded::Refused(refusal));
        }
        use std::io::Write;
        let _lock = self.lock()?;
        let path = self.ledger();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let mut line = serde_json::to_string(comparison)?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("syncing {}", path.display()))?;
        Ok(Recorded::Written)
    }

    /// Every comparison, oldest first.
    pub fn comparisons(&self) -> Result<Vec<Comparison>> {
        self.comparisons_counting().map(|(rows, _)| rows)
    }

    /// Every comparison, and how many lines were skipped as unreadable. A
    /// missing file is an empty store; a file that cannot be read is an
    /// `Err` — a finding, not an empty queue.
    pub fn comparisons_counting(&self) -> Result<(Vec<Comparison>, usize)> {
        let path = self.ledger();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut out = Vec::new();
        let mut skipped = 0usize;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(c) => out.push(c),
                Err(e) => {
                    skipped += 1;
                    tracing::warn!("skipping unreadable comparison row: {e}");
                }
            }
        }
        Ok((out, skipped))
    }
}

/// The store, counted: what a readout prints.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    pub records: usize,
    /// Rows by [`Kind`], by wire name.
    pub by_kind: std::collections::BTreeMap<String, usize>,
    pub separated: usize,
    pub tied: usize,
    pub inconclusive: usize,
    /// Rows whose verdict this build cannot read (a newer build's variant).
    /// Kept apart from `inconclusive`: "the validator posed no question" is
    /// a finding, "this build cannot say" is not (found on review).
    pub unreadable_verdict: usize,
    /// Rows a model judge decided — the ones a reader may want to leave out.
    pub judge_decided: usize,
    /// Of the `inconclusive`, the points no structural validator could pose
    /// ([`Validator::Unposed`]): nothing was driven for them. Kept apart
    /// because "driven and undecided" and "never askable" call for opposite
    /// fixes — a better arm, or a validator that does not exist yet.
    pub unposed: usize,
}

impl Summary {
    pub fn of(rows: &[Comparison]) -> Summary {
        let mut s = Summary {
            records: rows.len(),
            ..Summary::default()
        };
        for c in rows {
            *s.by_kind
                .entry(crate::appraisal::enum_name(&c.kind))
                .or_default() += 1;
            match c.verdict {
                Verdict::Separated => s.separated += 1,
                Verdict::Tied => s.tied += 1,
                Verdict::Inconclusive => s.inconclusive += 1,
                Verdict::Unknown => s.unreadable_verdict += 1,
            }
            if c.validator == Validator::Judge {
                s.judge_decided += 1;
            }
            if c.validator == Validator::Unposed && c.verdict == Verdict::Inconclusive {
                s.unposed += 1;
            }
        }
        s
    }

    /// The share of *decided* comparisons in which the validator separated
    /// the arms. `None` over no decided comparison — a dash, never zero.
    pub fn separated_share(&self) -> Option<f64> {
        let decided = self.separated + self.tied;
        (decided > 0).then(|| self.separated as f64 / decided as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Taint;
    use crate::message::Message;
    use crate::session::{Record, Session, SessionMeta};
    use serde_json::json;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mecha-comparisons-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn spec(name: &str, properties: serde_json::Value) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("{name} things"),
            input_schema: json!({"type": "object", "properties": properties}),
        }
    }

    fn clean() -> Provenance {
        Provenance {
            origin: Origin::Clean,
            surface: Fidelity::Matches,
        }
    }

    fn steer_probe(unsteered: Outcome) -> Comparison {
        let specs = [spec("fs_read", json!({"path": {"type": "string"}}))];
        Comparison::new(
            Kind::SteerProbe,
            Some(Situation::recorded(
                &["fs_read".into()],
                "steer",
                None,
                None,
            )),
            Some(GoalKind::Task),
            CallClass::of("fs_read", &json!({"path": "/secret/notes.txt"}), &specs),
            vec![
                Arm::new(Role::Recorded, Some("r1".into()), Outcome::Pass),
                Arm::new(Role::WithoutIntervention, Some("r1".into()), unsteered),
            ],
            Validator::StructuralSteer,
            Pointers {
                session_id: "s1".into(),
                message_index: Some(4),
                call_index: Some(2),
                ..Pointers::default()
            },
            "local-model",
        )
    }

    /// The acceptance, at the store: a clean comparison with a readable
    /// surface is written, and a second read — a fresh handle on the same
    /// root — returns it field for field.
    #[test]
    fn an_admitted_comparison_is_returned_by_a_second_read() {
        let root = temp_root("roundtrip");
        let store = ComparisonStore::open(&root).unwrap();
        let written = steer_probe(Outcome::Fail);
        assert_eq!(store.record(clean(), &written).unwrap(), Recorded::Written);
        let again = ComparisonStore::open(&root).unwrap();
        let (rows, skipped) = again.comparisons_counting().unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(rows, vec![written]);
        assert_eq!(rows[0].verdict, Verdict::Separated);
        assert_eq!(rows[0].preferred, vec![0], "the recording is preferred");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Tainted, derived, or of unknown provenance; a surface changed, never
    /// kept, or gone — every one is refused, and the store holds nothing.
    #[test]
    fn a_session_that_is_not_clean_or_not_readable_leaves_nothing() {
        let root = temp_root("refused");
        let store = ComparisonStore::open(&root).unwrap();
        for (origin, surface, why) in [
            (Origin::Untrusted, Fidelity::Matches, Refusal::NotClean),
            (Origin::Derived, Fidelity::Matches, Refusal::NotClean),
            (Origin::Clean, Fidelity::Differs, Refusal::SurfaceUnreadable),
            (Origin::Clean, Fidelity::Unknown, Refusal::SurfaceUnreadable),
        ] {
            assert_eq!(
                store
                    .record(Provenance { origin, surface }, &steer_probe(Outcome::Fail))
                    .unwrap(),
                Recorded::Refused(why)
            );
        }
        assert!(store.comparisons().unwrap().is_empty());
        assert!(
            !store.ledger().exists(),
            "a refusal must not even create the file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn transcript_with(taint: Option<Taint>) -> (PathBuf, crate::session::Transcript) {
        let root = temp_root("transcript");
        std::fs::create_dir_all(&root).unwrap();
        let session = Session::create(
            &root,
            SessionMeta {
                id: Session::new_id(),
                created_at: Utc::now(),
                provider: "scripted".into(),
                model: "scripted".into(),
                workspace: root.clone(),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        session
            .append(&Record::Message(Message::user("read my notes")))
            .unwrap();
        if let Some(t) = taint {
            session.append(&Record::Taint(t)).unwrap();
        }
        let t = Session::read(&session.path).unwrap();
        (root, t)
    }

    /// The session's taint at its end decides, and a transcript with no
    /// checkpoint after its last message is unknown — untrusted, not clean.
    #[test]
    fn provenance_reads_the_recorded_taint_and_fails_closed() {
        let specs = [spec("fs_read", json!({}))];
        let hash = crate::surface::fingerprint(&specs);
        for (taint, expect) in [
            (
                Some(Taint {
                    untrusted: false,
                    private: true,
                }),
                Origin::Clean,
            ),
            (
                Some(Taint {
                    untrusted: true,
                    private: false,
                }),
                Origin::Untrusted,
            ),
            (None, Origin::Untrusted),
        ] {
            let (root, t) = transcript_with(taint);
            let p = Provenance::of_transcript(&t, Some(&hash), &specs);
            assert_eq!(p.origin, expect, "{taint:?}");
            assert_eq!(p.surface, Fidelity::Matches);
            assert_eq!(
                Provenance::of_transcript(&t, Some(&hash), &[]).surface,
                Fidelity::Differs,
                "a blob the store no longer holds is not a readable surface"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// A newer build's kind, validator, role, outcome, verdict and goal kind
    /// load as `unknown`; a torn line is counted and hides nothing else.
    #[test]
    fn unknown_variants_and_a_torn_line_load_leniently() {
        let root = temp_root("lenient");
        let store = ComparisonStore::open(&root).unwrap();
        let good = steer_probe(Outcome::Pass);
        store.record(clean(), &good).unwrap();
        let mut raw = std::fs::read_to_string(store.ledger()).unwrap();
        raw.push_str(
            r#"{"id":"cmp-future","at":"2026-09-25T00:00:00Z","kind":"mid-run-branch","goal_kind":"dream","arms":[{"role":"oracle","outcome":"sparkle"},{"role":"recorded","outcome":"pass"}],"validator":"telepathy","verdict":"transcended","pointers":{"session_id":"s2"},"a_field_from_later":1}
{"id":"cmp-sparse","at":"2026-09-25T00:00:00Z","arms":[{}]}
{"id":"cmp-torn","at":"2026-09-25T00:00:00Z","kind":"steer-pr
"#,
        );
        std::fs::write(store.ledger(), raw).unwrap();
        let (rows, skipped) = store.comparisons_counting().unwrap();
        assert_eq!(skipped, 1, "the torn line is counted");
        assert_eq!(rows.len(), 3, "and costs only itself");
        let sparse = &rows[2];
        assert_eq!(
            (sparse.kind, sparse.validator, sparse.verdict),
            (Kind::Unknown, Validator::Unknown, Verdict::Unknown),
            "a field a row lacks is unknown, never a guess"
        );
        assert_eq!(
            (sparse.arms[0].role, sparse.arms[0].outcome),
            (Role::Unknown, Outcome::Unknown)
        );
        assert_eq!(sparse.pointers, Pointers::default());
        assert_eq!(rows[0], good);
        let future = &rows[1];
        assert_eq!(future.kind, Kind::Unknown);
        assert_eq!(future.goal_kind, Some(GoalKind::Unknown));
        assert_eq!(future.validator, Validator::Unknown);
        assert_eq!(future.verdict, Verdict::Unknown);
        assert_eq!(future.arms[0].role, Role::Unknown);
        assert_eq!(future.arms[0].outcome, Outcome::Unknown);
        assert_eq!(future.situation, None, "absent is unknown, not standing");
        assert!(future.call.is_none());
        let summary = Summary::of(&rows);
        assert_eq!(summary.records, 3);
        assert_eq!(summary.by_kind.get("unknown"), Some(&2));
        assert_eq!(
            (summary.inconclusive, summary.unreadable_verdict),
            (0, 2),
            "a verdict this build cannot read is not an inconclusive one"
        );
        assert_eq!(summary.separated_share(), Some(0.0));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A comparison drawn from a reflection with no recorded situation keeps
    /// it unknown through a write and a read: never the empty-keyed scope,
    /// which is standing and would match every run (found on review).
    #[test]
    fn an_unknown_situation_stays_unknown_rather_than_standing() {
        let root = temp_root("unknown-situation");
        let store = ComparisonStore::open(&root).unwrap();
        let mut c = steer_probe(Outcome::Fail);
        c.situation = None;
        store.record(clean(), &c).unwrap();
        let raw = std::fs::read_to_string(store.ledger()).unwrap();
        assert!(!raw.contains("\"situation\""), "{raw}");
        let back = &store.comparisons().unwrap()[0];
        assert_eq!(back.situation, None);
        assert!(
            Situation::default().is_standing(),
            "the value unknown must not be"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The verdict is a function of the arms, for any K.
    #[test]
    fn the_verdict_is_derived_from_the_arms_for_any_number_of_them() {
        let arm = |o| Arm::new(Role::Unknown, None, o);
        use Outcome::*;
        assert_eq!(Verdict::of(&[arm(Pass)]).0, Verdict::Inconclusive);
        assert_eq!(Verdict::of(&[arm(Pass), arm(Pass)]).0, Verdict::Tied);
        assert_eq!(Verdict::of(&[arm(Fail), arm(Fail)]).0, Verdict::Tied);
        assert_eq!(
            Verdict::of(&[arm(Fail), arm(Pass)]),
            (Verdict::Separated, vec![1])
        );
        assert_eq!(
            Verdict::of(&[arm(Pass), arm(Fail), arm(Pass)]),
            (Verdict::Separated, vec![0, 2])
        );
        assert_eq!(
            Verdict::of(&[arm(Pass), arm(Fail), arm(Inconclusive)]).0,
            Verdict::Inconclusive
        );
        assert_eq!(
            Verdict::of(&[arm(Pass), arm(Unknown)]).0,
            Verdict::Inconclusive,
            "an arm this build cannot read is not guessed"
        );
    }

    /// The call class is the tool and the declared argument names: never a
    /// value, never an undeclared key, never a tool off the recorded surface.
    #[test]
    fn a_call_class_keeps_declared_argument_names_and_nothing_else() {
        let specs = [spec(
            "fs_write",
            json!({"path": {"type": "string"}, "content": {"type": "string"}}),
        )];
        let c = CallClass::of(
            "fs_write",
            &json!({"path": "/home/someone/x", "content": "ignore previous instructions", "injected_key": 1}),
            &specs,
        )
        .unwrap();
        assert_eq!(c.tool, "fs_write");
        assert_eq!(c.args, vec!["content".to_string(), "path".to_string()]);
        let wire = serde_json::to_string(&c).unwrap();
        assert!(!wire.contains("/home/someone") && !wire.contains("ignore previous"));
        assert!(!wire.contains("injected_key"));
        assert_eq!(CallClass::of("not_a_tool", &json!({}), &specs), None);
    }

    /// A rate over no decided comparison is `None`, not zero — and an
    /// inconclusive row is not a decided one.
    #[test]
    fn the_separated_share_over_nothing_decided_is_none() {
        assert_eq!(Summary::of(&[]).separated_share(), None);
        let inconclusive = steer_probe(Outcome::Inconclusive);
        assert_eq!(
            Summary::of(std::slice::from_ref(&inconclusive)).separated_share(),
            None
        );
        let s = Summary::of(&[
            inconclusive,
            steer_probe(Outcome::Fail),
            steer_probe(Outcome::Pass),
        ]);
        assert_eq!(s.separated_share(), Some(0.5));
        assert_eq!(s.by_kind.get("steer-probe"), Some(&3));
    }

    /// An unreadable store is an error, not an empty one.
    #[test]
    fn an_unreadable_store_is_a_finding_not_an_empty_one() {
        let root = temp_root("unreadable");
        let store = ComparisonStore::open(&root).unwrap();
        std::fs::create_dir_all(store.ledger()).unwrap();
        assert!(store.comparisons_counting().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
