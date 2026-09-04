//! Experiments: a designed comparison over a chosen set of runs — the unit
//! larger than a run that no other store here has (`docs/EXPERIMENT-DESIGN.md`,
//! Part I §3–§4 and Part II §14–§15, §18).
//!
//! What lives here is the store and the pure parts of the runner: the
//! **manifest** (the design, written before anything runs — arms, the
//! control, each treatment arm's falsifiable prediction, the tasks, the
//! seeds, the split seed), the **trial** record (one row per
//! arm × task × seed × repetition), the **isolated home** every trial runs
//! in, the **child invocation** that expresses an arm as a config file and
//! a set of `--no-*` flags, and the **judge** that pairs a treatment arm
//! against the control through `candidate::judge_slices` with a uniformly
//! drawn holdout. The process spawning is `mecha exp`'s, in the CLI, on D3:
//! every actor is its own `mecha` process, and this crate knows nothing
//! about any binary.
//!
//! Three rules carried over from the other stores, restated once because
//! this is the sixth store and the expensive mistake is inventing a
//! seventh convention:
//!
//! - **One pretty JSON per trial, temp-sibling-and-rename**, so a reader
//!   never sees a torn row.
//! - **Closed enums on an append-only store are wire formats**: a trial
//!   status this build does not know reads as unknown, never as a failed
//!   file.
//! - **An arm may only vary the closed set.** Levers by name from
//!   [`crate::harness::Lever`], knobs by `KEY=VALUE` through
//!   [`crate::harness::parse_change`]. An unknown lever name in a manifest
//!   is a load error, never a skipped line (D14) — and `approval_rules` is
//!   refused outright, because a `forbid` is the operator's standing word and
//!   only `mecha eval`'s fixture workspaces justify lifting it.
//!
//! And one rule that is this store's own (D12): **isolation is the whole
//! store, not the mailbox.** Every trial runs with `MECHA_HOME` pointing at a
//! home under the experiment directory, whose `config.toml` *is* the arm, so
//! nothing about an arm is ambient — and the runner refuses a home that is,
//! or contains, the real one. A rule learned inside a trial that landed in
//! `~/.mecha/learning/` would ride every real run's cached prefix from then
//! on, a longer half-life than any injection the interlock guards against.

use crate::candidate::{ChangeClass, Judgement, Metric};
use crate::harness::Lever;
use crate::session::RunStats;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ─── The manifest ───────────────────────────────────────────────────────────

/// The design, written before the run. Loaded through [`Manifest::load`],
/// which is where every rule below is enforced; a `Manifest` value in hand
/// is one that passed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `single` or `lifetime` (Part II §14). Only `single` runs today; a
    /// `lifetime` manifest loads — the design is the design — and `run`
    /// refuses it by name, so the manifest can be written ahead of the driver.
    #[serde(default)]
    pub kind: TrialKind,
    /// The arm every treatment arm is paired against. Must name an arm, and
    /// that arm carries no prediction: the control predicts nothing.
    pub control: String,
    pub arms: BTreeMap<String, Arm>,
    pub tasks: Tasks,
    /// Recorded per trial and set on the child's provider. Empty means one
    /// unseeded run per task — the server chooses, and the trial says so.
    #[serde(default)]
    pub seeds: Vec<u64>,
    #[serde(default = "one")]
    pub repetitions: u32,
    /// The holdout draw's seed (`sample::take_uniform`). Fixed in the
    /// manifest, before any trial runs, on `candidate.rs`'s rule: a holdout
    /// drawn after looking at the trials is the multiple-comparisons trap
    /// it exists to close.
    pub split_seed: u64,
    /// Every `holdout_in`-th pair is held out, at least
    /// `candidate::MIN_HOLDOUT_PAIRS`.
    #[serde(default = "three")]
    pub holdout_in: u64,
}

fn one() -> u32 {
    1
}
fn three() -> u64 {
    3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialKind {
    #[default]
    Single,
    Lifetime,
}

/// One arm: which levers are off, which knobs are moved, and — for a
/// treatment arm — what it predicts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arm {
    /// A named starting point, applied before `levers_off`. `bare` is what
    /// `mecha eval` runs (`Lever::bare`, every lever off but the operator's
    /// rules); `full` is every lever on. Part II §15: "appraisal off" is a
    /// preset over consumer levers, never a lever, and the manifest names
    /// presets as such so a reader a month later can tell what was absent.
    #[serde(default)]
    pub preset: Option<Preset>,
    #[serde(default)]
    pub levers_off: Vec<String>,
    /// Levers turned back *on* after the preset — the add-one-to-bare
    /// design (Part II §15: "only for a lever with a prior worth testing in
    /// isolation"), and how `mecha eval --ab-rules` is spelled as an arm:
    /// `bare` plus `learned_rules`. Applied after `levers_off`, so a name in
    /// both is on; the `approval_rules` refusal applies here too.
    #[serde(default)]
    pub levers_on: Vec<String>,
    /// `KEY=VALUE` over `harness::OverrideKey`, validated at load.
    #[serde(default)]
    pub overrides: Vec<String>,
    /// The provider this arm runs against, by the operator's config key;
    /// the operator's default when absent. The **other axis** of a
    /// condition: an arm is a model and a harness configuration, and an
    /// experiment may vary either or both — eval's model bake-off is the
    /// special case of arms that name models under the `bare` preset.
    #[serde(default)]
    pub provider: Option<String>,
    /// The model id, overriding the provider's default.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub prediction: Option<Prediction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Bare,
    Full,
}

/// What a treatment arm predicts, made before either arm is measured.
/// `candidate::Prediction`'s shape with one more metric: the task outcome,
/// entering as a cost (D4 — every metric is lower-is-better, so a solve
/// rate enters as `1 − solved` and never as a benefit axis).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prediction {
    pub metric: ExpMetric,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpMetric {
    /// `1.0` for a trial whose grader failed it, `0.0` for a pass — the
    /// task outcome as a cost.
    Failure,
    /// `candidate::Metric`'s six, over the trial's folded `RunStats`.
    EndedOnFailedCall,
    ToolErrorRate,
    CutShort,
    Compactions,
    Turns,
    MalformedArgs,
}

impl ExpMetric {
    fn stats_metric(self) -> Option<Metric> {
        Some(match self {
            ExpMetric::Failure => return None,
            ExpMetric::EndedOnFailedCall => Metric::EndedOnFailedCall,
            ExpMetric::ToolErrorRate => Metric::ToolErrorRate,
            ExpMetric::CutShort => Metric::CutShort,
            ExpMetric::Compactions => Metric::Compactions,
            ExpMetric::Turns => Metric::Turns,
            ExpMetric::MalformedArgs => Metric::MalformedArgs,
        })
    }

    /// The cost of one finished trial under this metric. `None` when the
    /// trial cannot answer — no grade for `failure`, no stats for the rest —
    /// which drops the pair rather than scoring an unknown as zero.
    pub fn cost(self, trial: &Trial) -> Option<f64> {
        match self.stats_metric() {
            None => trial.passed.map(|p| if p { 0.0 } else { 1.0 }),
            Some(m) => trial.stats.as_ref().map(|s| m.of(s)),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExpMetric::Failure => "failure",
            ExpMetric::EndedOnFailedCall => "ended_on_failed_call",
            ExpMetric::ToolErrorRate => "tool_error_rate",
            ExpMetric::CutShort => "cut_short",
            ExpMetric::Compactions => "compactions",
            ExpMetric::Turns => "turns",
            ExpMetric::MalformedArgs => "malformed_args",
        }
    }
}

/// Where the tasks come from: an eval case file, as `mecha eval` reads it,
/// and the fixture workspace its cases run against.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tasks {
    pub cases: PathBuf,
    pub fixture: PathBuf,
    /// Only these case ids, when set.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Only cases carrying one of these tags, when set.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Manifest {
    /// A two-arm design built by a front-end rather than written by hand —
    /// how `mecha eval`'s `--ab-config` and `--ab-rules` are spelled as
    /// experiments: the control is `bare` (what eval runs), the treatment
    /// is the control plus one delta, predicting a lower failure cost. The
    /// split seed is derived from the treatment's own description so a
    /// rerun of the same A/B holds out the same tasks — the property the
    /// hash-by-id holdout used to give eval — and nothing else about the
    /// design is chosen after a trial ran. Validated like a parsed one —
    /// `treatment_name` is a directory name, so the delta itself goes in
    /// the prediction's rationale, not the name.
    pub fn two_arm(
        name: &str,
        treatment_name: &str,
        treatment: Arm,
        tasks: Tasks,
        holdout_in: u64,
        repetitions: u32,
    ) -> Result<Manifest> {
        let mut arms = BTreeMap::new();
        arms.insert(
            "bare".to_string(),
            Arm {
                preset: Some(Preset::Bare),
                ..Arm::default()
            },
        );
        let split_seed = {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in format!(
                "{}|{:?}|{:?}|{:?}",
                treatment_name, treatment.levers_off, treatment.levers_on, treatment.overrides
            )
            .bytes()
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            h
        };
        arms.insert(treatment_name.to_string(), treatment);
        let m = Manifest {
            name: name.to_string(),
            description: String::new(),
            kind: TrialKind::Single,
            control: "bare".to_string(),
            arms,
            tasks,
            seeds: Vec::new(),
            repetitions,
            split_seed,
            holdout_in,
        };
        m.validate()?;
        Ok(m)
    }

    /// Parse and validate. Every rule the module doc names is enforced here
    /// and nowhere else, so a `Manifest` value is one that passed.
    pub fn parse(text: &str) -> Result<Manifest> {
        let m: Manifest = toml::from_str(text).context("parsing the manifest")?;
        m.validate()?;
        Ok(m)
    }

    pub fn load(path: &Path) -> Result<Manifest> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Manifest::parse(&text).with_context(|| path.display().to_string())
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.name.is_empty(), "the manifest needs a name");
        crate::work::valid_producer(&self.name)
            .context("the experiment name is a directory name and a producer name")?;
        anyhow::ensure!(
            self.arms.contains_key(&self.control),
            "control `{}` names no arm (arms: {})",
            self.control,
            self.arms.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        anyhow::ensure!(
            self.arms.len() >= 2,
            "an experiment needs the control and at least one treatment arm"
        );
        anyhow::ensure!(self.repetitions >= 1, "repetitions must be at least 1");
        anyhow::ensure!(self.holdout_in >= 2, "holdout_in must be at least 2");
        for (name, arm) in &self.arms {
            crate::work::valid_producer(name)
                .with_context(|| format!("arm `{name}` is a directory name"))?;
            arm.resolve_levers()
                .with_context(|| format!("arm `{name}`"))?;
            for spec in &arm.overrides {
                crate::harness::parse_change(spec)
                    .with_context(|| format!("arm `{name}`, override `{spec}`"))?;
            }
            if name == &self.control {
                anyhow::ensure!(
                    arm.prediction.is_none(),
                    "the control arm `{name}` must not carry a prediction — it is what the others are measured against"
                );
            } else {
                anyhow::ensure!(
                    arm.prediction.is_some(),
                    "treatment arm `{name}` carries no prediction; an arm that predicts nothing cannot be refuted (candidate.rs's rule, carried up a level)"
                );
            }
        }
        Ok(())
    }

    /// Every trial the design calls for, in a stable order, each with its
    /// condition hash. `provider` and `model` are the operator's defaults;
    /// an arm that names its own overrides them, and the hash follows the
    /// arm. Pure: the store decides which have run.
    pub fn trials(&self, task_ids: &[String], provider: &str, model: &str) -> Vec<Trial> {
        let seeds: Vec<Option<u64>> = if self.seeds.is_empty() {
            vec![None]
        } else {
            self.seeds.iter().copied().map(Some).collect()
        };
        let mut out = Vec::new();
        for (arm_name, arm) in &self.arms {
            let resolved = arm.resolve_levers().expect("validated at load");
            let provider = arm.provider.as_deref().unwrap_or(provider);
            let model = arm.model.as_deref().unwrap_or(model);
            for task in task_ids {
                for seed in &seeds {
                    for rep in 1..=self.repetitions {
                        let condition_hash =
                            condition_hash(&resolved, &arm.overrides, provider, model, *seed);
                        out.push(Trial {
                            id: trial_id(arm_name, task, *seed, rep),
                            arm: arm_name.clone(),
                            task: task.clone(),
                            seed: *seed,
                            repetition: rep,
                            condition_hash,
                            status: TrialStatus::Pending,
                            session_id: None,
                            started_at: None,
                            finished_at: None,
                            error: None,
                            passed: None,
                            checks: Vec::new(),
                            stats: None,
                        });
                    }
                }
            }
        }
        out
    }
}

impl Arm {
    /// The levers this arm carries off, in `Lever::ALL`'s order: the preset
    /// first, then `levers_off` by name. Errors are the load-time rules.
    pub fn resolve_levers(&self) -> Result<Vec<Lever>> {
        let mut off: Vec<Lever> = match self.preset {
            Some(Preset::Bare) => Lever::bare(&[]),
            Some(Preset::Full) | None => Vec::new(),
        };
        let parse = |name: &str| -> Result<Lever> {
            let lever = Lever::parse(name).with_context(|| {
                format!(
                    "`{name}` is not a lever (the closed set: {})",
                    Lever::names()
                )
            })?;
            anyhow::ensure!(
                lever != Lever::ApprovalRules,
                "`approval_rules` cannot be a lever in an experiment: a forbid is the operator's standing word, and only mecha eval's fixture workspaces justify lifting it"
            );
            Ok(lever)
        };
        for name in &self.levers_off {
            off.push(parse(name)?);
        }
        let mut on = Vec::new();
        for name in &self.levers_on {
            on.push(parse(name)?);
        }
        Ok(Lever::ALL
            .into_iter()
            .filter(|l| off.contains(l) && !on.contains(l))
            .collect())
    }
}

fn trial_id(arm: &str, task: &str, seed: Option<u64>, rep: u32) -> String {
    let task: String = task
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    match seed {
        Some(s) => format!("{arm}__{task}__s{s}__r{rep}"),
        None => format!("{arm}__{task}__r{rep}"),
    }
}

/// Hash of the resolved arm — levers off, overrides, provider, model, seed.
/// Two runs with the same hash were configured identically; two with
/// different hashes differ somewhere, even if nobody remembers where (§4).
/// FNV-1a over a canonical rendering: an equality key, not a credential, so
/// no hashing dependency is worth adding for it.
pub fn condition_hash(
    levers_off: &[Lever],
    overrides: &[String],
    provider: &str,
    model: &str,
    seed: Option<u64>,
) -> String {
    let mut overrides: Vec<&str> = overrides.iter().map(String::as_str).collect();
    overrides.sort_unstable();
    let canonical = format!(
        "levers_off={}|overrides={}|provider={provider}|model={model}|seed={}",
        levers_off
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(","),
        overrides.join(","),
        seed.map(|s| s.to_string()).unwrap_or_default()
    );
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in canonical.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

// ─── Trials ─────────────────────────────────────────────────────────────────

/// One row per arm × task × seed × repetition. Written by the runner as the
/// trial moves; read by `status` and `judge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    pub id: String,
    pub arm: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub repetition: u32,
    pub condition_hash: String,
    #[serde(default, deserialize_with = "de_lenient_status")]
    pub status: TrialStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The grader's verdict. `None` until graded — never `false`, because
    /// "not yet graded" and "failed" are opposite findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<crate::eval::Check>,
    /// The child run's folded outcome, read off its session in the trial home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<RunStats>,
}

impl Trial {
    /// A trial that finished in-process — how `mecha eval` files each arm's
    /// case result as a row: graded, with the run's folded stats, and no
    /// session (eval writes none).
    pub fn finished(
        planned: &Trial,
        passed: bool,
        checks: Vec<crate::eval::Check>,
        stats: Option<RunStats>,
    ) -> Trial {
        let now = chrono::Utc::now().to_rfc3339();
        Trial {
            status: TrialStatus::Done,
            started_at: Some(now.clone()),
            finished_at: Some(now),
            passed: Some(passed),
            checks,
            stats,
            ..planned.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialStatus {
    #[default]
    Pending,
    Running,
    Done,
    Failed,
    /// A status this build does not know. A trial in this state is neither
    /// rerun nor judged.
    Unknown,
}

fn de_lenient_status<'de, D>(d: D) -> std::result::Result<TrialStatus, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(TrialStatus::Unknown))
}

// ─── The store ──────────────────────────────────────────────────────────────

/// `~/.mecha/experiments/<name>/` — the manifest, the trials, and the
/// isolated homes the trials run in.
pub struct ExperimentStore {
    root: PathBuf,
}

/// The marker the runner writes into a trial home. `runlog::Scan` admits
/// `SessionKind::Experiment` by default only in a home carrying it (D13):
/// an experiment session that leaked into the real store stays hidden where
/// it would contaminate, and is read where it belongs.
pub const HOME_MARKER: &str = "EXPERIMENT";

impl ExperimentStore {
    pub fn root_default() -> Result<PathBuf> {
        Ok(crate::work::mecha_home()?.join("experiments"))
    }

    pub fn open(root: impl Into<PathBuf>, name: &str) -> Result<ExperimentStore> {
        crate::work::valid_producer(name)?;
        Ok(ExperimentStore {
            root: root.into().join(name),
        })
    }

    pub fn open_default(name: &str) -> Result<ExperimentStore> {
        ExperimentStore::open(Self::root_default()?, name)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.toml")
    }

    /// Create the experiment from a manifest. Refuses to overwrite: a design
    /// is written once, before the run, and a second `new` over it would be
    /// the after-the-fact redesign the manifest exists to prevent.
    pub fn create(&self, manifest_text: &str) -> Result<Manifest> {
        let manifest = Manifest::parse(manifest_text)?;
        anyhow::ensure!(
            !self.manifest_path().exists(),
            "{} already exists; an experiment's design is written once",
            self.manifest_path().display()
        );
        std::fs::create_dir_all(self.root.join("trials"))?;
        write_atomic(&self.manifest_path(), manifest_text.as_bytes())?;
        Ok(manifest)
    }

    /// [`Self::create`] for a design built in code (`Manifest::two_arm`).
    pub fn create_manifest(&self, manifest: &Manifest) -> Result<()> {
        let text = toml::to_string_pretty(manifest).context("rendering the manifest")?;
        self.create(&text).map(|_| ())
    }

    pub fn manifest(&self) -> Result<Manifest> {
        Manifest::load(&self.manifest_path())
    }

    pub fn trial_path(&self, id: &str) -> PathBuf {
        self.root.join("trials").join(format!("{id}.json"))
    }

    pub fn save_trial(&self, t: &Trial) -> Result<()> {
        std::fs::create_dir_all(self.root.join("trials"))?;
        write_atomic(
            &self.trial_path(&t.id),
            serde_json::to_string_pretty(t)?.as_bytes(),
        )
    }

    /// Every trial on disk, by id. A file that does not parse is a finding
    /// (`skipped`), not an empty row.
    pub fn trials(&self) -> Result<(BTreeMap<String, Trial>, usize)> {
        let dir = self.root.join("trials");
        let mut out = BTreeMap::new();
        let mut skipped = 0;
        if !dir.exists() {
            return Ok((out, 0));
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<Trial>(&s).ok())
            {
                Some(t) => {
                    out.insert(t.id.clone(), t);
                }
                None => skipped += 1,
            }
        }
        Ok((out, skipped))
    }

    /// The design's trials merged with what is on disk: a trial the store
    /// knows keeps its row; one it does not is pending. The store's rows
    /// never lose to the design — a finished trial stays finished.
    pub fn plan(
        &self,
        manifest: &Manifest,
        task_ids: &[String],
        provider: &str,
        model: &str,
    ) -> Result<(Vec<Trial>, usize)> {
        let (on_disk, skipped) = self.trials()?;
        let planned = manifest
            .trials(task_ids, provider, model)
            .into_iter()
            .map(|t| on_disk.get(&t.id).cloned().unwrap_or(t))
            .collect();
        Ok((planned, skipped))
    }

    /// The isolated home one arm's trials run in (D12). Created with the
    /// marker, and **refused if it is, or contains, the real home**.
    pub fn arm_home(&self, arm: &str) -> Result<PathBuf> {
        let home = self.root.join("homes").join(arm);
        let real = crate::work::mecha_home()?;
        refuse_unsafe_home(&home, &real)?;
        let fresh = !home.join(HOME_MARKER).exists();
        std::fs::create_dir_all(&home)?;
        if fresh {
            seed_home(&real, &home)?;
        }
        std::fs::write(
            home.join(HOME_MARKER),
            b"an experiment home; see mecha exp\n",
        )?;
        Ok(home)
    }

    pub fn workspace_for(&self, trial_id: &str) -> PathBuf {
        self.root.join("trials").join(trial_id).join("workspace")
    }
}

/// D12's refusal, and `setup`'s rule for a workspace applied to the store:
/// a trial home that *is* the real home would learn into it, and one that
/// *contains* it would jail a run over the owner's tokens and transcripts.
pub fn refuse_unsafe_home(home: &Path, real: &Path) -> Result<()> {
    let norm = |p: &Path| -> PathBuf {
        // Lexical, not canonical: the trial home may not exist yet, and a
        // symlink under the real home is still under the real home.
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    };
    let (home, real) = (norm(home), norm(real));
    anyhow::ensure!(
        home != real,
        "the trial home {} is the real mecha home; an experiment never runs in it",
        home.display()
    );
    anyhow::ensure!(
        !real.starts_with(&home),
        "the trial home {} contains the real mecha home {}; a run jailed there would hold the owner's tokens and transcripts",
        home.display(),
        real.display()
    );
    Ok(())
}

/// Is this home an experiment home — does it carry the marker?
pub fn is_experiment_home(home: &Path) -> bool {
    home.join(HOME_MARKER).is_file()
}

/// [`is_experiment_home`] for the home this process runs in. `false` when
/// the home cannot be resolved: unknown is never admitted.
pub fn in_experiment_home() -> bool {
    crate::work::mecha_home()
        .map(|h| is_experiment_home(&h))
        .unwrap_or(false)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
    Ok(())
}

// ─── The child invocation ───────────────────────────────────────────────────

/// Which trial a run is one actor of — carried on `RunConfig::experiment`
/// (§4), so a session can say which trial it belonged to without a scan of
/// the experiment store. Set by the runner on the child's environment as
/// [`EXPERIMENT_REF_ENV`]; nothing else sets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentRef {
    pub exp_id: String,
    pub trial_id: String,
    pub arm: String,
    /// The actor's producer name — for `single`, the trial id.
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub task: String,
    pub repetition: u32,
    pub condition_hash: String,
}

pub const EXPERIMENT_REF_ENV: &str = "MECHA_EXPERIMENT_REF";

impl ExperimentRef {
    /// The reference this process was started under, if any. Read once per
    /// `RunConfig::of`; a malformed value is a warning and `None`, never a
    /// guess at a trial.
    pub fn from_env() -> Option<ExperimentRef> {
        let raw = std::env::var(EXPERIMENT_REF_ENV).ok()?;
        if raw.is_empty() {
            return None;
        }
        match serde_json::from_str(&raw) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("{EXPERIMENT_REF_ENV} ignored: {e}");
                None
            }
        }
    }
}

/// How an arm reaches a child `mecha run`: the trial home's `config.toml`
/// carries the operator's whole posture with the arm's switches applied,
/// and the levers that are CLI-only become `--no-*` flags. Pure — the
/// runner writes the file and spawns the process.
///
/// **The machine's posture travels, the arm varies the closed set.** The
/// child config starts from the operator's config, not from defaults: the
/// sandbox and security sections, the approval `[[rule]]`s and `[approval]`
/// (the first cut dropped them, so every trial ran with the operator's
/// `forbid` list gone *and* `--yes` — the silently-degrading-guard shape,
/// and the exact opposite of what refusing the `approval_rules` lever
/// promises; found on review), the `[mcp]`, `[[hook]]` and `[outbox]`
/// sections a lever left on needs something to be on *of*, search
/// backends, subagent profiles. Every provider's inline `api_key` is
/// scrubbed — the variable `api_key_env` names passes through the child's
/// environment (`passthrough`), and a secret does not belong in a store a
/// trial's artifacts get exported from. The trial's seed pins the default
/// provider.
#[derive(Debug, Clone)]
pub struct ChildInvocation {
    pub config: crate::config::Config,
    pub flags: Vec<String>,
    /// Environment variables the child needs beyond the base set: every
    /// provider's `api_key_env`.
    pub passthrough: Vec<String>,
}

pub fn child_invocation(
    real: &crate::config::Config,
    arm: &Arm,
    seed: Option<u64>,
) -> Result<ChildInvocation> {
    let levers_off = arm.resolve_levers()?;
    let mut config = real.clone();
    // The arm's provider, or the operator's default: the model axis.
    if let Some(p) = &arm.provider {
        config.default_provider = p.clone();
    }
    anyhow::ensure!(
        config.providers.contains_key(&config.default_provider),
        "arm names provider `{}` but the operator's config has no [providers.{}]",
        config.default_provider,
        config.default_provider
    );
    let mut passthrough = Vec::new();
    let default = config.default_provider.clone();
    for (name, provider) in config.providers.iter_mut() {
        provider.api_key = None;
        if let Some(env) = &provider.api_key_env {
            passthrough.push(env.clone());
        }
        if name == &default {
            if seed.is_some() {
                provider.seed = seed;
            }
            if let Some(m) = &arm.model {
                provider.model = Some(m.clone());
            }
        }
    }
    // The other inline-secret surface: a search backend's key travels by
    // its variable, never by value (found on review).
    for backend in config.search.iter_mut() {
        backend.api_key = None;
        if let Some(env) = &backend.api_key_env {
            passthrough.push(env.clone());
        }
    }
    // An MCP server's `env` is where a token is written down, and the
    // server cannot start without it — so it rides into the trial home only
    // when the arm keeps MCP on; an arm with the lever off starts no server
    // and carries none of it. When it does ride, it is the same secret in
    // the same user's home one directory over, and `export` never reads
    // the config file.
    if levers_off.contains(&Lever::Mcp) {
        for server in config.mcp.iter_mut() {
            server.env.clear();
        }
    }
    let mut flags = Vec::new();
    for lever in levers_off {
        match lever {
            Lever::StepEscalation => config.agent.step_escalation = false,
            Lever::Boredom => config.agent.boredom = false,
            Lever::CompactValidate => config.agent.compact_validate = false,
            Lever::PredictiveCompaction => config.agent.predictive_compaction = false,
            Lever::CarriedState => config.agent.carried_state = false,
            Lever::Messages => {
                config.messages.enabled = false;
                flags.push("--no-messages".into());
            }
            Lever::Mcp => flags.push("--no-mcp".into()),
            Lever::LearnedRules => flags.push("--no-learned-rules".into()),
            Lever::Hooks => flags.push("--no-hooks".into()),
            Lever::Outbox => flags.push("--no-outbox".into()),
            Lever::Fallback => flags.push("--no-fallback".into()),
            Lever::Skills => flags.push("--no-skills".into()),
            Lever::Charter => flags.push("--no-charter".into()),
            Lever::CompactTool => flags.push("--no-compact-tool".into()),
            Lever::ApprovalRules => unreachable!("refused at load"),
        }
    }
    for spec in &arm.overrides {
        let change = crate::harness::parse_change(spec)?;
        change.apply_to_agent(&mut config.agent)?;
    }
    Ok(ChildInvocation {
        config,
        flags,
        passthrough,
    })
}

/// The stores a lever left *on* reads: the learning store (rules and
/// reflections), the skills directory, the charter. A fresh trial home has
/// none, so `full` would have meant "the machine's `[agent]` switches and
/// nothing else" (found on review). Seeded once, when the arm's home is
/// first created, from the real home — a snapshot, never written back, so
/// `full` means the harness as this machine had it when the arm started
/// and a trial's `learn` lands in the copy.
pub const SEEDED: [&str; 3] = ["learning", "skills", "charter.toml"];

pub fn seed_home(real: &Path, home: &Path) -> Result<()> {
    for name in SEEDED {
        let from = real.join(name);
        let to = home.join(name);
        if !from.exists() || to.exists() {
            continue;
        }
        copy_tree(&from, &to)
            .with_context(|| format!("seeding {} into {}", name, home.display()))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
    }
    Ok(())
}

// ─── The judge ──────────────────────────────────────────────────────────────

/// One treatment arm's verdict against the control.
#[derive(Debug, Clone, Serialize)]
pub struct ArmJudgement {
    pub arm: String,
    pub metric: ExpMetric,
    pub pairs: usize,
    pub selection: usize,
    pub holdout: usize,
    pub judgement: Judgement,
}

/// A finished control trial and the treatment trial on the same episode.
struct TrialPair<'a> {
    episode: String,
    control: &'a Trial,
    treatment: &'a Trial,
    control_cost: f64,
    treatment_cost: f64,
}

/// Pair every treatment arm's finished trials with the control's by
/// (task, seed, repetition), draw the holdout uniformly with the manifest's
/// split seed (`judge_drawn`'s rule — the pool here is uniform by
/// construction, but the draw is seeded so the same manifest judges the
/// same way twice), and judge each arm through the gate. A trial that
/// cannot answer the metric drops its pair; an arm with no pairs is
/// reported with none rather than omitted.
pub fn judge(manifest: &Manifest, trials: &[Trial]) -> Vec<ArmJudgement> {
    let control: BTreeMap<String, &Trial> = trials
        .iter()
        .filter(|t| t.arm == manifest.control && t.status == TrialStatus::Done)
        .map(|t| (episode_key(t), t))
        .collect();
    let mut out = Vec::new();
    for (name, arm) in &manifest.arms {
        if name == &manifest.control {
            continue;
        }
        let Some(prediction) = &arm.prediction else {
            continue;
        };
        let metric = prediction.metric;
        let mut pairs: Vec<TrialPair<'_>> = trials
            .iter()
            .filter(|t| &t.arm == name && t.status == TrialStatus::Done)
            .filter_map(|t| {
                let c = control.get(&episode_key(t))?;
                Some(TrialPair {
                    episode: episode_key(t),
                    control: c,
                    treatment: t,
                    control_cost: metric.cost(c)?,
                    treatment_cost: metric.cost(t)?,
                })
            })
            .collect();
        pairs.sort_by(|a, b| a.episode.cmp(&b.episode));
        let n = pairs.len();
        let holdout_n = if n == 0 {
            0
        } else {
            ((n as u64).div_ceil(manifest.holdout_in) as usize)
                .max(crate::candidate::MIN_HOLDOUT_PAIRS)
                .min(n)
        };
        let drawn: Vec<usize> =
            crate::sample::take_uniform((0..n).collect(), manifest.split_seed, holdout_n);
        let (mut selection, mut holdout) = (Vec::new(), Vec::new());
        for (i, p) in pairs.iter().enumerate() {
            if drawn.contains(&i) {
                holdout.push(p);
            } else {
                selection.push(p);
            }
        }
        let judgement = crate::candidate::judge_slices(
            ChangeClass::Config,
            &selection,
            &holdout,
            |p| (p.episode.as_str(), p.control_cost, p.treatment_cost),
            |p| {
                (
                    p.control
                        .stats
                        .as_ref()
                        .map_or(0, |s| u64::from(s.tool_calls)),
                    p.treatment
                        .stats
                        .as_ref()
                        .map_or(0, |s| u64::from(s.tool_calls)),
                )
            },
        );
        out.push(ArmJudgement {
            arm: name.clone(),
            metric,
            pairs: n,
            selection: selection.len(),
            holdout: holdout.len(),
            judgement,
        });
    }
    out
}

fn episode_key(t: &Trial) -> String {
    format!(
        "{}|{}|{}",
        t.task,
        t.seed.map(|s| s.to_string()).unwrap_or_default(),
        t.repetition
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
name = "levers"
control = "full"
split_seed = 7
seeds = [1, 2]
repetitions = 1

[tasks]
cases = "eval/cases.jsonl"
fixture = "eval/workspace"

[arms.full]
preset = "full"

[arms.bare]
preset = "bare"
[arms.bare.prediction]
metric = "failure"
rationale = "everything off should fail more"

[arms.quiet]
levers_off = ["boredom", "compact_validate"]
overrides = ["max_turns=20"]
[arms.quiet.prediction]
metric = "turns"
rationale = "no notice, fewer turns"
"#;

    #[test]
    fn a_manifest_loads_and_enumerates_its_trials_in_a_stable_order() {
        let m = Manifest::parse(MANIFEST).unwrap();
        assert_eq!(m.control, "full");
        assert_eq!(m.arms["bare"].resolve_levers().unwrap(), Lever::bare(&[]));
        assert_eq!(
            m.arms["quiet"].resolve_levers().unwrap(),
            vec![Lever::Boredom, Lever::CompactValidate],
            "in ALL's order, whatever the manifest's"
        );
        let tasks = vec!["b".to_string(), "a".to_string()];
        let trials = m.trials(&tasks, "local", "m");
        // 3 arms × 2 tasks × 2 seeds × 1 rep
        assert_eq!(trials.len(), 12);
        assert_eq!(
            trials[0].id, "bare__b__s1__r1",
            "arms sorted, tasks as given"
        );
        assert!(trials.iter().all(|t| t.status == TrialStatus::Pending));
        // Same arm, same seed: same hash across tasks; a different seed or
        // arm: a different hash.
        let h = |arm: &str, seed: u64, task: &str| {
            trials
                .iter()
                .find(|t| t.arm == arm && t.seed == Some(seed) && t.task == task)
                .unwrap()
                .condition_hash
                .clone()
        };
        assert_eq!(h("bare", 1, "a"), h("bare", 1, "b"));
        assert_ne!(h("bare", 1, "a"), h("bare", 2, "a"));
        assert_ne!(h("bare", 1, "a"), h("quiet", 1, "a"));
        assert_eq!(m.trials(&tasks, "local", "m")[3].id, trials[3].id, "stable");
    }

    /// The load-time rules, each of which is a design defect the runner
    /// must never get as far as running: no control, a control that
    /// predicts, a treatment that does not, an unknown lever, the rules
    /// lifted, a bad override.
    #[test]
    fn a_manifest_that_breaks_a_rule_does_not_load() {
        let bad = |edit: &dyn Fn(String) -> String, needle: &str| {
            let text = edit(MANIFEST.to_string());
            // `{:#}`: the rule's own words sit under an arm-naming context.
            let err = format!("{:#}", Manifest::parse(&text).unwrap_err());
            assert!(err.contains(needle), "{needle}: {err}");
        };
        bad(
            &|t| t.replace("control = \"full\"", "control = \"nope\""),
            "names no arm",
        );
        bad(
            &|t| {
                t.replace(
                    "[arms.full]\npreset = \"full\"",
                    "[arms.full]\npreset = \"full\"\n[arms.full.prediction]\nmetric = \"turns\"\nrationale = \"x\"",
                )
            },
            "must not carry a prediction",
        );
        bad(
            &|t| {
                t.replace("[arms.bare.prediction]\nmetric = \"failure\"\nrationale = \"everything off should fail more\"", "")
            },
            "carries no prediction",
        );
        bad(
            &|t| t.replace("\"boredom\"", "\"appraiser\""),
            "not a lever",
        );
        bad(
            &|t| t.replace("\"boredom\"", "\"approval_rules\""),
            "operator's standing word",
        );
        bad(&|t| t.replace("max_turns=20", "max_turns=0"), "at least 1");
        bad(
            &|t| t.replace("name = \"levers\"", "name = \"a/b\""),
            "name",
        );
    }

    #[test]
    fn a_child_invocation_carries_the_machines_posture_and_the_arm_and_no_secret() {
        let m = Manifest::parse(MANIFEST).unwrap();
        let mut real = crate::config::Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        real.providers.insert(
            "local".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("m".into()),
                api_key: Some("secret".into()),
                api_key_env: Some("LOCAL_KEY".into()),
                ..Default::default()
            },
        );
        real.rules.push(crate::policy::RuleConfig {
            tool: "shell".into(),
            decision: crate::policy::RuleDecision::Forbid,
            ..Default::default()
        });
        let quiet = child_invocation(&real, &m.arms["quiet"], Some(3)).unwrap();
        assert!(!quiet.config.agent.boredom);
        assert!(!quiet.config.agent.compact_validate);
        assert_eq!(quiet.config.agent.max_turns, 20, "the override landed");
        assert!(quiet.flags.is_empty(), "both levers are config switches");
        let p = &quiet.config.providers["local"];
        assert_eq!(p.seed, Some(3), "the trial's seed pins the provider");
        assert_eq!(p.api_key, None, "no secret in the store");
        assert!(
            quiet.passthrough.iter().any(|v| v == "LOCAL_KEY"),
            "the key's variable travels: {:?}",
            quiet.passthrough
        );
        assert_eq!(
            quiet.config.rules.len(),
            1,
            "the operator's forbid travels — refusing the lever must mean something"
        );

        // The other secret surfaces: a search key travels by its variable;
        // an MCP server's env rides only when the arm keeps MCP on.
        real.search.push(crate::config::SearchBackendConfig {
            kind: "exa".into(),
            api_key: Some("s3".into()),
            api_key_env: Some("EXA_KEY".into()),
            ..Default::default()
        });
        real.mcp.push(crate::config::McpServerConfig {
            name: "graph".into(),
            env: [("TOKEN".to_string(), "t".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        let quiet = child_invocation(&real, &m.arms["quiet"], None).unwrap();
        assert_eq!(quiet.config.search[0].api_key, None);
        assert!(quiet.passthrough.iter().any(|v| v == "EXA_KEY"));
        assert_eq!(
            quiet.config.mcp[0].env.len(),
            1,
            "MCP on: the server needs its env"
        );

        let bare = child_invocation(&real, &m.arms["bare"], None).unwrap();
        assert!(
            bare.config.mcp[0].env.is_empty(),
            "MCP off: no server starts, no token rides"
        );
        for flag in [
            "--no-mcp",
            "--no-learned-rules",
            "--no-hooks",
            "--no-outbox",
            "--no-fallback",
            "--no-messages",
            "--no-skills",
            "--no-charter",
            "--no-compact-tool",
        ] {
            assert!(
                bare.flags.iter().any(|f| f == flag),
                "{flag}: {:?}",
                bare.flags
            );
        }
        assert!(!bare.config.agent.predictive_compaction);
        assert!(!bare.config.agent.carried_state);
        assert!(!bare.config.messages.enabled);
        assert_eq!(
            bare.config.providers["local"].seed, None,
            "unseeded stays unseeded"
        );
        let text = toml::to_string(&bare.config).unwrap();
        let back: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(back.agent.max_turns, bare.config.agent.max_turns);
        assert_eq!(back.rules.len(), 1);
    }

    /// The stores a lever reads are seeded once from the real home, never
    /// written back: `full` means the harness as this machine has it.
    #[test]
    fn an_arm_home_is_seeded_from_the_real_stores_once() {
        let root = std::env::temp_dir().join(format!("mecha-exp-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let real = root.join("real");
        std::fs::create_dir_all(real.join("learning")).unwrap();
        std::fs::create_dir_all(real.join("skills").join("deploy")).unwrap();
        std::fs::write(real.join("learning").join("rules.jsonl"), b"{}\n").unwrap();
        std::fs::write(
            real.join("skills").join("deploy").join("SKILL.md"),
            b"# deploy\n",
        )
        .unwrap();
        std::fs::write(real.join("charter.toml"), b"[[line]]\n").unwrap();
        std::fs::write(real.join("config.toml"), b"default_provider = \"x\"\n").unwrap();
        let home = root.join("home");
        seed_home(&real, &home).unwrap();
        assert!(home.join("learning").join("rules.jsonl").is_file());
        assert!(home
            .join("skills")
            .join("deploy")
            .join("SKILL.md")
            .is_file());
        assert!(home.join("charter.toml").is_file());
        assert!(
            !home.join("config.toml").exists(),
            "the config is the arm's, not the machine's"
        );
        // Once: a later seed does not overwrite what the trial home has.
        std::fs::write(home.join("charter.toml"), b"[[line]]\n[[line]]\n").unwrap();
        seed_home(&real, &home).unwrap();
        assert_eq!(
            std::fs::read(home.join("charter.toml")).unwrap(),
            b"[[line]]\n[[line]]\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The other axis: an arm that names a provider or a model runs
    /// against it, its trials hash to it, and an arm naming a provider the
    /// operator has not configured is refused where the config is known.
    #[test]
    fn an_arm_can_name_its_model_and_the_hash_follows() {
        let text = MANIFEST.replace(
            "[arms.quiet]\nlevers_off",
            "[arms.quiet]\nprovider = \"small\"\nmodel = \"tiny\"\nlevers_off",
        );
        let m = Manifest::parse(&text).unwrap();
        let trials = m.trials(&["a".to_string()], "local", "m");
        let of = |arm: &str| {
            trials
                .iter()
                .find(|t| t.arm == arm && t.seed == Some(1))
                .unwrap()
                .condition_hash
                .clone()
        };
        assert_ne!(of("quiet"), of("full"));
        let mut real = crate::config::Config {
            default_provider: "local".into(),
            ..Default::default()
        };
        real.providers.insert(
            "local".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("m".into()),
                ..Default::default()
            },
        );
        assert!(child_invocation(&real, &m.arms["quiet"], None)
            .unwrap_err()
            .to_string()
            .contains("no [providers.small]"));
        real.providers.insert(
            "small".into(),
            crate::config::ProviderConfig {
                kind: "local".into(),
                model: Some("big".into()),
                ..Default::default()
            },
        );
        let inv = child_invocation(&real, &m.arms["quiet"], Some(2)).unwrap();
        assert_eq!(inv.config.default_provider, "small");
        assert_eq!(inv.config.providers["small"].model.as_deref(), Some("tiny"));
        assert_eq!(inv.config.providers["small"].seed, Some(2));
        assert_eq!(
            inv.config.providers["local"].seed, None,
            "the seed pins the arm's provider only"
        );
    }

    /// `levers_on` after the preset: the add-one-to-bare design, and how
    /// `--ab-rules` is spelled. The rules refusal reaches it too.
    #[test]
    fn levers_on_reopens_a_preset_and_the_rules_refusal_still_holds() {
        let arm = Arm {
            preset: Some(Preset::Bare),
            levers_on: vec!["learned_rules".into()],
            ..Arm::default()
        };
        let off = arm.resolve_levers().unwrap();
        assert!(!off.contains(&Lever::LearnedRules));
        assert!(!off.contains(&Lever::ApprovalRules), "never in a preset");
        assert_eq!(off.len(), Lever::ALL.len() - 2);
        let both = Arm {
            levers_off: vec!["boredom".into()],
            levers_on: vec!["boredom".into()],
            ..Arm::default()
        };
        assert!(
            both.resolve_levers().unwrap().is_empty(),
            "on wins over off"
        );
        let bad = Arm {
            levers_on: vec!["approval_rules".into()],
            ..Arm::default()
        };
        assert!(bad.resolve_levers().is_err());
    }

    /// A front-end's two-arm design is a manifest like any other: valid,
    /// stored, and its holdout draw fixed by the treatment's description so
    /// a rerun holds out the same tasks.
    #[test]
    fn a_two_arm_design_is_a_manifest_with_a_stable_split() {
        let tasks = Tasks {
            cases: "eval/cases.jsonl".into(),
            fixture: "eval/workspace".into(),
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let treatment = Arm {
            preset: Some(Preset::Bare),
            overrides: vec!["max_turns=40".into()],
            prediction: Some(Prediction {
                metric: ExpMetric::Failure,
                rationale: "--ab-config".into(),
            }),
            ..Arm::default()
        };
        let a = Manifest::two_arm(
            "eval-ab",
            "max_turns=40",
            treatment.clone(),
            tasks.clone(),
            3,
            1,
        )
        .unwrap();
        let b = Manifest::two_arm(
            "eval-ab-again",
            "max_turns=40",
            treatment,
            tasks.clone(),
            3,
            1,
        )
        .unwrap();
        assert_eq!(a.control, "bare");
        assert_eq!(
            a.split_seed, b.split_seed,
            "the same delta draws the same holdout"
        );
        let other = Manifest::two_arm(
            "eval-ab",
            "treatment",
            Arm {
                preset: Some(Preset::Bare),
                overrides: vec!["max_turns=50".into()],
                prediction: Some(Prediction {
                    metric: ExpMetric::Failure,
                    rationale: "x".into(),
                }),
                ..Arm::default()
            },
            tasks,
            3,
            1,
        )
        .unwrap();
        assert_ne!(a.split_seed, other.split_seed);
        let text = toml::to_string_pretty(&a).unwrap();
        let back = Manifest::parse(&text).unwrap();
        assert_eq!(back.arms.len(), 2);
        assert_eq!(back.arms["treatment"].overrides, vec!["max_turns=40"]);
    }

    #[test]
    fn a_home_that_is_or_contains_the_real_one_is_refused() {
        let real = Path::new("/home/x/.mecha");
        assert!(refuse_unsafe_home(Path::new("/home/x/.mecha"), real).is_err());
        assert!(refuse_unsafe_home(Path::new("/home/x"), real).is_err());
        assert!(
            refuse_unsafe_home(Path::new("/home/x/.mecha/experiments/e/../../"), real).is_err()
        );
        assert!(
            refuse_unsafe_home(Path::new("/home/x/.mecha/experiments/e/homes/a"), real).is_ok()
        );
        assert!(refuse_unsafe_home(Path::new("/tmp/e/homes/a"), real).is_ok());
    }

    fn done(arm: &str, task: &str, seed: u64, passed: bool, turns: u32) -> Trial {
        Trial {
            id: trial_id(arm, task, Some(seed), 1),
            arm: arm.into(),
            task: task.into(),
            seed: Some(seed),
            repetition: 1,
            condition_hash: "h".into(),
            status: TrialStatus::Done,
            session_id: None,
            started_at: None,
            finished_at: None,
            error: None,
            passed: Some(passed),
            checks: Vec::new(),
            stats: Some(RunStats {
                turns,
                tool_calls: 3,
                ..RunStats::default()
            }),
        }
    }

    /// The gate over arm sets: each treatment arm paired with the control by
    /// episode, the task outcome entering as a cost, the holdout drawn by the
    /// manifest's seed, and a pair whose trial cannot answer the metric
    /// dropped rather than scored.
    #[test]
    fn the_judge_pairs_each_treatment_arm_against_the_control() {
        let m = Manifest::parse(MANIFEST).unwrap();
        let mut trials = Vec::new();
        for task in 0..12 {
            let task = format!("t{task}");
            for seed in [1, 2] {
                trials.push(done("full", &task, seed, true, 10));
                // bare fails everything: a clear loss on `failure`.
                trials.push(done("bare", &task, seed, false, 10));
                // quiet uses fewer turns: a win on `turns`.
                trials.push(done("quiet", &task, seed, true, 6));
            }
        }
        // One quiet trial never got stats: its pair drops.
        trials.iter_mut().find(|t| t.arm == "quiet").unwrap().stats = None;
        let verdicts = judge(&m, &trials);
        assert_eq!(verdicts.len(), 2);
        let bare = verdicts.iter().find(|v| v.arm == "bare").unwrap();
        assert_eq!(bare.metric, ExpMetric::Failure);
        assert_eq!(bare.pairs, 24);
        assert_eq!(bare.selection + bare.holdout, 24);
        assert!(bare.holdout >= crate::candidate::MIN_HOLDOUT_PAIRS);
        assert!(
            matches!(
                bare.judgement.disposition,
                crate::candidate::Disposition::Reject(_)
            ),
            "{:?}",
            bare.judgement.disposition
        );
        let quiet = verdicts.iter().find(|v| v.arm == "quiet").unwrap();
        assert_eq!(quiet.pairs, 23, "the statless trial dropped its pair");
        assert!(
            !matches!(
                quiet.judgement.disposition,
                crate::candidate::Disposition::Reject(_)
            ),
            "{:?}",
            quiet.judgement.disposition
        );
        // Deterministic: the same manifest draws the same holdout.
        let again = judge(&m, &trials);
        assert_eq!(again[0].holdout, verdicts[0].holdout);
        assert_eq!(
            again[0].judgement.holdout.wins,
            verdicts[0].judgement.holdout.wins
        );
    }

    #[test]
    fn the_store_keeps_its_rows_over_the_design_and_reads_a_torn_row_as_a_finding() {
        let root = std::env::temp_dir().join(format!("mecha-exp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ExperimentStore::open(&root, "levers").unwrap();
        let m = store.create(MANIFEST).unwrap();
        assert!(store.create(MANIFEST).is_err(), "written once");
        let tasks = vec!["a".to_string()];
        let (planned, skipped) = store.plan(&m, &tasks, "local", "m").unwrap();
        assert_eq!(planned.len(), 6);
        assert_eq!(skipped, 0);
        let mut first = planned[0].clone();
        first.status = TrialStatus::Done;
        first.passed = Some(true);
        store.save_trial(&first).unwrap();
        std::fs::write(store.trial_path("torn"), b"{not json").unwrap();
        let (planned, skipped) = store.plan(&m, &tasks, "local", "m").unwrap();
        assert_eq!(planned[0].status, TrialStatus::Done, "the store's row wins");
        assert_eq!(skipped, 1, "a torn row is counted, not read as pending");
        // An unknown status reads as unknown, never as a failed file.
        let raw = serde_json::to_string(&first)
            .unwrap()
            .replace("\"done\"", "\"vanished\"");
        let t: Trial = serde_json::from_str(&raw).unwrap();
        assert_eq!(t.status, TrialStatus::Unknown);
        let home = store.arm_home("bare").unwrap();
        assert!(is_experiment_home(&home));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_experiment_ref_round_trips_and_a_bad_one_is_none() {
        let r = ExperimentRef {
            exp_id: "levers".into(),
            trial_id: "bare__a__r1".into(),
            arm: "bare".into(),
            actor: "bare__a__r1".into(),
            role: None,
            task: "a".into(),
            repetition: 1,
            condition_hash: "h".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ExperimentRef>(&json).unwrap(), r);
        assert!(serde_json::from_str::<ExperimentRef>("{\"exp_id\":1}").is_err());
    }
}
