//! Model-extracted constraints with host-checked references, not verified rules.
use super::{bounded, Candidate, Citation, Status};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementScope {
    Universal,
    Specific,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Met,
    Unmet,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    /// The host copies the complete constrained step; adequacy is model judged.
    pub quote: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftRequirement {
    step: usize,
    condition: String,
    scope: RequirementScope,
    scope_description: String,
    state: RequirementState,
    citations: Vec<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    guard: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Requirement {
    /// Host-assigned, local to the assessed candidate.
    pub id: String,
    pub step: usize,
    pub condition: String,
    pub scope: RequirementScope,
    pub scope_description: String,
    pub state: RequirementState,
    pub citations: Vec<Citation>,
    pub guard: Option<Guard>,
}

pub(super) fn schema(evidence: &Value, candidate: &Candidate) -> Value {
    let mut required_evidence = evidence.clone();
    required_evidence["minItems"] = json!(1);
    json!({"type":"array","maxItems":16,"items":{
        "type":"object","additionalProperties":false,
        "required":["step","condition","scope","scope_description","state","citations","guard"],
        "properties":{
            "step":{"type":"integer","minimum":1,"maximum":candidate.steps.len()},
            "condition":{"type":"string","minLength":1,"maxLength":500},
            "scope":{"enum":["universal","specific","unknown"]},
            "scope_description":{"type":"string","minLength":1,"maxLength":500},
            "state":{"enum":["met","unmet","unknown"]},
            "citations":required_evidence,
            "guard":{"anyOf":[{"type":"null"},{"type":"string","enum":candidate.steps}]}
        }
    }})
}

pub(super) fn validate(
    drafts: Vec<DraftRequirement>,
    candidate: &Candidate,
    passages: &[Citation],
) -> Result<(Vec<Requirement>, Status)> {
    ensure!(drafts.len() <= 16, "too many requirements");
    let mut seen = BTreeSet::new();
    let mut requirements = Vec::new();
    let mut blocked = false;
    let mut unknown = false;
    for d in drafts {
        bounded(&d.condition, 500)?;
        bounded(&d.scope_description, 500)?;
        ensure!(
            seen.insert((d.step, d.condition.clone(), d.scope_description.clone())),
            "duplicate requirement"
        );
        let step = d
            .step
            .checked_sub(1)
            .and_then(|i| candidate.steps.get(i))
            .context("requirement step not in candidate")?;
        ensure!(d.citations.len() <= 6, "too many requirement citations");
        let mut cited = BTreeSet::new();
        let mut citations = Vec::new();
        for id in &d.citations {
            ensure!(cited.insert(id), "duplicate requirement citation");
            let passage = passages
                .iter()
                .find(|p| &p.passage_id == id)
                .context("requirement passage not visible")?;
            ensure!(
                matches!(passage.kind.as_str(), "episode" | "situation"),
                "requirement needs original evidence"
            );
            citations.push(passage.clone());
        }
        ensure!(!citations.is_empty(), "requirement needs original evidence");
        if let Some(quote) = &d.guard {
            ensure!(
                quote == step,
                "guard must quote its own candidate step verbatim"
            );
        }
        // A guard may postpone action; it cannot certify applicability or the
        // condition's eventual satisfaction. Semantic guard adequacy stays model judged.
        blocked |= d.state == RequirementState::Unmet && d.guard.is_none();
        unknown |= d.scope == RequirementScope::Unknown
            || (d.state == RequirementState::Unknown && d.guard.is_none());
        requirements.push(Requirement {
            id: format!("r{}", requirements.len() + 1),
            step: d.step,
            condition: d.condition,
            scope: d.scope,
            scope_description: d.scope_description,
            state: d.state,
            citations,
            guard: d.guard.map(|quote| Guard { quote }),
        });
    }
    Ok((
        requirements,
        if blocked {
            Status::Contradicted
        } else if unknown {
            Status::Unknown
        } else {
            Status::Supported
        },
    ))
}
