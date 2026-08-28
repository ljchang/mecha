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

/// A `KEY=VALUE` change, parsed and value-validated. The value is kept as the
/// canonical string it parsed from, so one shape serialises to the overrides
/// file and re-parses on load through the same validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigChange {
    pub key: OverrideKey,
    pub value: String,
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
    /// Sessions dropped because an arm left the recording — a divergent
    /// replay's stats describe a truncated run, and scoring one would let a
    /// behaviour-visible change be graded on the fraction it tracked.
    /// **Bare ids, and they stay that way**: `episodes`/`holdout_episodes`'s
    /// own contract (a sample nobody can redraw is one nobody can check)
    /// holds here too, and anything resolving an entry back to a session
    /// path must not have to parse annotation out of it first.
    pub diverged: Vec<String>,
    /// What the replay was compromising on for a dropped episode, when it
    /// was — "id — attached N times; replayed under the first config".
    /// Beside `diverged` rather than folded into it, so the ids stay
    /// joinable; rendered by `mecha harness show`, whose reader — whoever
    /// decides on a staged candidate — is the one the caveat was written
    /// for: such a divergence says something about the replay's compromise,
    /// not necessarily about the change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_caveats: Vec<String>,
    /// Sessions that could not be replayed at all (unreadable, no recorded
    /// calls, tool surface moved). Never evidence for either arm.
    pub skipped: usize,
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
    pub diverged: Vec<String>,
    /// See [`Measurement::replay_caveats`].
    pub replay_caveats: Vec<String>,
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
            diverged,
            replay_caveats,
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
            diverged,
            replay_caveats,
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
    use super::*;

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
