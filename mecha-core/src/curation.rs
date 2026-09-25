//! The owner's verdicts on what a learner produced — a rule, a reflection, a
//! harness candidate — recorded against that product and **never a run's
//! score** (`docs/APPRAISAL-WIRING-DESIGN.md` S3a, rulings R16f–h).
//!
//! Retiring or restoring a learned rule is a verdict on the rule (its
//! tenure, L3); dropping or editing a reflection is a verdict on the
//! reflector; accepting, rejecting or reverting a harness candidate is
//! credit for that change and the diagnosis behind it (L6). None of them
//! says anything about how a *run* went, so nothing here is ever an
//! [`crate::appraisal::GoalError`]: `appraisal::of_session` takes no input
//! from this module, which is the structural half of "never a run's score".
//!
//! **What was already recorded is read where it lives, and only the lost
//! half is written.** A reflection's drop and edit sit on the reflection
//! (`dropped_at`, `edited_at`), so [`Tally::of`] reads them there. Two acts
//! left no trace an appraisal could read: `mecha rules restore` clears
//! `retired_at` (the restore itself was only a git commit message), and an
//! owner's retire, accept or reject writes the same field a machine does —
//! the retirement scan retires, the rumination gate accepts and rejects.
//! So the owner's verbs, and only those, append one line here, in the store
//! that owns the target: `learning/curation.jsonl` for rules,
//! `learning/harness/curation.jsonl` for candidates. The line is the owner's act; a
//! machine's decision on the same record never writes one, which is how the
//! two are told apart.
//!
//! **Append-only, and every closed enum is a wire format**: an act or target
//! kind from a newer build loads as `unknown` rather than failing the row; a
//! torn line is skipped and counted, never folded into an empty ledger.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file name of the ledger in each store that keeps one.
pub const LEDGER: &str = "curation.jsonl";

/// What the verdict is about, by the id its own store minted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    /// A learned rule, by `Rule::id`.
    Rule(String),
    /// A reflection, by `Reflexion::id`.
    Reflexion(String),
    /// A harness candidate, by `HarnessCandidate::id`.
    Candidate(String),
    /// A target kind from a newer build.
    #[serde(other)]
    Unknown,
}

/// The owner's act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Act {
    /// A rule taken out of every prompt — counts against the rule's tenure.
    Retired,
    /// A retired rule put back — counts for it.
    Restored,
    /// A reflection refused: it never becomes a rule.
    Dropped,
    /// A reflection rewritten in the owner's words.
    Edited,
    /// A staged harness candidate accepted.
    Accepted,
    /// A staged harness candidate rejected.
    Rejected,
    /// An accepted harness candidate's override taken back out.
    Reverted,
    /// An act from a newer build.
    #[serde(other)]
    Unknown,
}

/// One line of a curation ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// Lenient: `#[serde(other)]` on [`Target::Unknown`] matches an unknown
    /// kind only when no id rides beside it, and every target carries one.
    #[serde(deserialize_with = "de_target_lenient")]
    pub target: Target,
    pub act: Act,
    pub at: String,
    /// The owner's own reason, when the verb took one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn de_target_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Target, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or(Target::Unknown))
}

impl Verdict {
    /// A verdict stamped now.
    pub fn now(target: Target, act: Act, reason: Option<String>) -> Verdict {
        Verdict {
            target,
            act,
            at: chrono::Utc::now().to_rfc3339(),
            reason,
        }
    }
}

/// Append one verdict to the ledger under `root`. The caller holds whatever
/// lock its store's writes take (the learning store's, for a rule); the
/// append itself is one `write` of one line in append mode.
pub fn append(root: &Path, v: &Verdict) -> Result<()> {
    use std::io::Write;
    let path = root.join(LEDGER);
    let mut line = serde_json::to_string(v)?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    f.write_all(line.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Every verdict under `root`, oldest first, with how many lines could not
/// be read. A ledger that has never been written is empty, not unreadable.
pub fn read_counting(root: &Path) -> Result<(Vec<Verdict>, usize)> {
    let path: PathBuf = root.join(LEDGER);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Verdict>(line) {
            Ok(v) => out.push(v),
            Err(e) => {
                tracing::warn!("skipping unreadable curation row: {e}");
                skipped += 1;
            }
        }
    }
    Ok((out, skipped))
}

/// The owner's verdicts on the learners, counted — what `mecha sessions
/// appraise` prints beside the runs, and never inside them. Each group is
/// `None` when its store could not be fully read: unknown, never zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// Owner retire/restore of learned rules (R16f).
    pub rules: Option<RuleTally>,
    /// Owner drop/edit of reflections (R16g), read off the reflections.
    pub reflections: Option<ReflectionTally>,
    /// Owner accept/reject/revert of harness candidates (R16h).
    pub harness: Option<HarnessTally>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RuleTally {
    pub retired: usize,
    pub restored: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ReflectionTally {
    pub dropped: usize,
    pub edited: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct HarnessTally {
    pub accepted: usize,
    pub rejected: usize,
    pub reverted: usize,
}

impl Tally {
    /// Pure over what the three stores returned. `rules` and `harness` are
    /// the two ledgers (`None` = unreadable or short); `reflections` is the
    /// reflection store (`None` = unreadable or short). A verdict whose
    /// target is not the ledger's own kind is not counted there — a ledger
    /// row is about the store that holds it.
    pub fn of(
        rules: Option<&[Verdict]>,
        reflections: Option<&[crate::learning::Reflexion]>,
        harness: Option<&[Verdict]>,
    ) -> Tally {
        let rules = rules.map(|vs| {
            let mut t = RuleTally::default();
            for v in vs.iter().filter(|v| matches!(v.target, Target::Rule(_))) {
                match v.act {
                    Act::Retired => t.retired += 1,
                    Act::Restored => t.restored += 1,
                    _ => {}
                }
            }
            t
        });
        let reflections = reflections.map(|rs| ReflectionTally {
            dropped: rs.iter().filter(|r| r.dropped_at.is_some()).count(),
            edited: rs.iter().filter(|r| r.edited_at.is_some()).count(),
        });
        let harness = harness.map(|vs| {
            let mut t = HarnessTally::default();
            for v in vs
                .iter()
                .filter(|v| matches!(v.target, Target::Candidate(_)))
            {
                match v.act {
                    Act::Accepted => t.accepted += 1,
                    Act::Rejected => t.rejected += 1,
                    Act::Reverted => t.reverted += 1,
                    _ => {}
                }
            }
            t
        });
        Tally {
            rules,
            reflections,
            harness,
        }
    }

    /// The owner's standing verdicts on one rule, oldest first — what L3's
    /// tenure will read. A restore after a retire is the later verdict; both
    /// are kept, because the order is the evidence.
    pub fn for_rule<'a>(ledger: &'a [Verdict], rule_id: &str) -> Vec<&'a Verdict> {
        ledger
            .iter()
            .filter(|v| matches!(&v.target, Target::Rule(id) if id == rule_id))
            .collect()
    }
}

/// Read both ledgers and the reflection store from their default homes —
/// best-effort, like every appraisal reader: a store that has never existed
/// reads empty, one that cannot be fully read reads `None`.
pub fn load_default() -> Tally {
    let rules = match crate::learning::LearningStore::default_root() {
        Ok(root) => match read_counting(&root) {
            Ok((vs, 0)) => Some(vs),
            _ => None,
        },
        Err(_) => None,
    };
    let reflections = match crate::learning::LearningStore::open_existing_default() {
        None => Some(Vec::new()),
        Some(store) => match store.reflexions_counting() {
            Ok((rs, 0)) => Some(rs),
            _ => None,
        },
    };
    let harness = match crate::harness::HarnessStore::default_root() {
        Ok(root) => match read_counting(&root) {
            Ok((vs, 0)) => Some(vs),
            _ => None,
        },
        Err(_) => None,
    };
    Tally::of(rules.as_deref(), reflections.as_deref(), harness.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_act_or_target_from_a_newer_build_loads_rather_than_failing_the_row() {
        let v: Verdict = serde_json::from_str(
            r#"{"target":{"kind":"skill","id":"s1"},"act":"pinned","at":"t"}"#,
        )
        .unwrap();
        assert_eq!(v.target, Target::Unknown);
        assert_eq!(v.act, Act::Unknown);
    }

    #[test]
    fn a_ledger_round_trips_and_a_torn_line_is_counted_not_hidden() {
        let root = std::env::temp_dir().join(format!("mecha-curation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.as_path();
        assert_eq!(read_counting(dir).unwrap(), (Vec::new(), 0));
        let v = Verdict::now(Target::Rule("r1".into()), Act::Retired, None);
        append(dir, &v).unwrap();
        append(
            dir,
            &Verdict::now(Target::Rule("r1".into()), Act::Restored, None),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(LEDGER))
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"{torn\n"))
            .unwrap();
        let (vs, skipped) = read_counting(dir).unwrap();
        assert_eq!(vs.len(), 2);
        assert_eq!(skipped, 1);
        assert_eq!(vs[0], v);
        let history: Vec<Act> = Tally::for_rule(&vs, "r1").iter().map(|v| v.act).collect();
        assert_eq!(history, vec![Act::Retired, Act::Restored]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_store_is_unknown_and_a_ledger_counts_only_its_own_kind() {
        let rules = vec![
            Verdict::now(Target::Rule("r1".into()), Act::Retired, None),
            Verdict::now(Target::Rule("r2".into()), Act::Retired, None),
            Verdict::now(Target::Rule("r1".into()), Act::Restored, None),
            // A candidate verdict in the learning ledger is not a rule's.
            Verdict::now(Target::Candidate("c1".into()), Act::Accepted, None),
        ];
        let harness = vec![
            Verdict::now(Target::Candidate("c1".into()), Act::Accepted, None),
            Verdict::now(Target::Candidate("c1".into()), Act::Reverted, None),
            Verdict::now(Target::Candidate("c2".into()), Act::Rejected, None),
        ];
        let t = Tally::of(Some(&rules), None, Some(&harness));
        assert_eq!(
            t.rules,
            Some(RuleTally {
                retired: 2,
                restored: 1
            })
        );
        assert_eq!(t.reflections, None, "unreadable is unknown, never zero");
        assert_eq!(
            t.harness,
            Some(HarnessTally {
                accepted: 1,
                rejected: 1,
                reverted: 1
            })
        );
    }
}
