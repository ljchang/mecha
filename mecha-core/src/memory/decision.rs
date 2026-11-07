//! Pure recommendations over a host-bound synthetic measurement read.
//!
//! This is no inference or action surface. The envelope's `complete` member is a
//! format terminator, never proof of whole-file completeness: FsRead can return a
//! valid line-bounded prefix. The binding hashes only the exact received text.
use super::{
    experience::{
        self, checks, Assessment, Passage, Projection, RecallView, Request, State, METHODS,
    },
    plan_extraction::UniqueValue,
    RevisionRef,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Objective {
    MinimizeEnergy,
    MinimizeMinutes,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub workload: String,
    pub objective: Objective,
    pub energy_budget: u32,
    pub minimum_quality: u32,
}
impl Contract {
    pub(crate) fn state(&self) -> Result<State> {
        ensure!(
            (1..=128).contains(&self.workload.len()) && !self.workload.trim().is_empty(),
            "workload must contain 1..128 bytes of nonblank text"
        );
        Ok(State {
            workload: self.workload.clone(),
            goal: match self.objective {
                Objective::MinimizeEnergy => "minimize energy",
                Objective::MinimizeMinutes => "minimize minutes",
            }
            .into(),
            energy_budget: self.energy_budget,
            minimum_quality: self.minimum_quality,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceView {
    None,
    Observed,
}

/// Only a durable successful-read accessor supplies this provenance. It is not
/// deserializable from captured evidence or a model response.
#[derive(Clone, Serialize)]
pub(crate) struct ReadBinding {
    pub task: RevisionRef,
    pub observation: RevisionRef,
    pub read_sha256: String,
}

/// Immutable preparation; no public constructor or deserialization path.
#[derive(Serialize)]
pub struct Prepared {
    contract: Contract,
    binding: ReadBinding,
    view: EvidenceView,
    state: State,
    request: Request,
    request_sha256: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Recommend,
    Abstain,
    Withheld,
}
/// Advisory result only. A recommendation is never execution or a measurement.
#[derive(Serialize)]
pub struct Decision {
    pub proposed_choice: String,
    pub citations: Vec<String>,
    pub reason: String,
    pub assessment: Assessment,
    pub disposition: Disposition,
    binding: ReadBinding,
    pub request_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    kind: String,
    workload: String,
    observations: Vec<Measurement>,
    complete: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Measurement {
    choice: String,
    observed_minutes: u32,
    energy: u32,
    quality: u32,
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Validate all received bytes even when the model will receive no observations.
/// The caller must separately establish successful read provenance and recheck
/// source freshness/access when reserving or settling durable inference.
pub(crate) fn prepare(
    contract: &Contract,
    binding: ReadBinding,
    content: &str,
    view: EvidenceView,
) -> Result<Prepared> {
    let state = contract.state()?;
    ensure!(
        content.len() <= 24_000,
        "oversized received measurement text"
    );
    ensure!(
        binding.read_sha256 == sha(content),
        "received read hash mismatch"
    );
    let UniqueValue(value) = serde_json::from_str(content)?;
    let envelope: Envelope = serde_json::from_value(value)?;
    ensure!(
        envelope.kind == "host_authored_synthetic_measurements_v1" && envelope.complete,
        "unknown measurement envelope or missing format terminator"
    );
    ensure!(
        envelope.workload == contract.workload,
        "measurement workload mismatch"
    );
    ensure!(
        envelope.observations.len() <= 3,
        "too many method observations"
    );
    let mut seen = BTreeSet::new();
    let mut passages = Vec::new();
    for m in envelope.observations {
        ensure!(
            METHODS.contains(&m.choice.as_str()) && seen.insert(m.choice.clone()),
            "unknown or duplicate method observation"
        );
        if view == EvidenceView::None {
            continue;
        }
        let text = json!({"workload":envelope.workload,"outcome":{
            "choice":m.choice,"observed_minutes":m.observed_minutes,"energy":m.energy,
            "quality":m.quality,"kind":"host_observed_fixture_outcome"
        }})
        .to_string();
        passages.push(Passage {
            id: format!("e{}", passages.len() + 1),
            text: content.to_owned(),
            source: binding.observation.clone(),
            sha256: binding.read_sha256.clone(),
            projection: Some(Projection {
                view: RecallView::Measurements,
                sha256: sha(&text),
                text,
                context: None,
            }),
        });
    }
    let request = experience::request(&state, passages)?;
    let request_sha256 = sha(&serde_json::to_string(&request)?);
    Ok(Prepared {
        contract: contract.clone(),
        binding,
        view,
        state,
        request,
        request_sha256,
    })
}
impl Prepared {
    pub fn request(&self) -> &Request {
        &self.request
    }
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }

    /// Malformed replies are terminal errors, never a request to repair JSON.
    pub fn assess(&self, reply: &str) -> Result<Assessment> {
        let choice = experience::parse_choice(&self.request, reply)?;
        let observations = self
            .request
            .passages
            .iter()
            .map(|p| {
                let projection = p
                    .projection
                    .as_ref()
                    .context("missing sealed measurement projection")?;
                Ok((p.id.as_str(), projection.text.as_str()))
            })
            .collect::<Result<Vec<_>>>()?;
        checks::assess_observations(&self.state, &self.request, &choice, observations)
    }
    /// Pure request preparation. The durable caller owns the one-retry allowance
    /// and must reserve the exact returned request before invoking its model.
    pub fn reconsideration(&self, reply: &str) -> Result<Option<Request>> {
        let assessment = self.assess(reply)?;
        checks::revision_request(&self.request, reply, &assessment)
    }
    pub fn finish(&self, reply: &str) -> Result<Decision> {
        let assessment = self.assess(reply)?;
        let choice = experience::parse_choice(&self.request, reply)?;
        let disposition = if choice.choice == "abstain" {
            Disposition::Abstain
        } else if assessment.admissible {
            Disposition::Recommend
        } else {
            Disposition::Withheld
        };
        Ok(Decision {
            proposed_choice: choice.choice,
            citations: choice.citations,
            reason: choice.reason,
            assessment,
            disposition,
            binding: self.binding.clone(),
            request_sha256: self.request_sha256.clone(),
        })
    }
}

/// Canonical file identity for a read-only reopened check. This does not grant writes.
pub(crate) fn source_store_path(db: &super::Memory) -> Result<std::path::PathBuf> {
    Ok(
        std::path::Path::new(db.conn.path().context("memory store has no file path")?)
            .canonicalize()?,
    )
}

/// Revalidate the frozen audit without rerunning or replacing its retrieval.
/// Every retained hit and its dependency closure must remain current and authorized.
pub(crate) fn recheck_frozen_hits(
    db: &super::Memory,
    hits: &[super::Hit],
    access: &super::Access,
) -> Result<()> {
    use super::sql::{authorized, current, fresh, lookup, required_scopes};
    db.check_access(access)?;
    let tx = db.conn.unchecked_transaction()?;
    for hit in hits {
        current(&tx, &hit.reference)?;
        ensure!(
            authorized(&tx, &hit.reference, access)? && fresh(&tx, &hit.reference, i64::MAX)?,
            "frozen recall source unavailable"
        );
        ensure!(
            required_scopes(&tx, &hit.reference.revision)? == hit.required_scopes,
            "frozen recall source scopes changed"
        );
        let record = lookup(&tx, &hit.reference)?.context("missing frozen recall source")?;
        ensure!(
            serde_json::to_value(record)? == serde_json::to_value(&hit.record)?,
            "frozen recall source content changed"
        );
    }
    tx.commit()?;
    Ok(())
}
