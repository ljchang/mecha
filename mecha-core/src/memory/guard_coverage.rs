//! Structural experiment over fixture-authored contracts. Never action authority.
//! Conditions are immutable within a plan; no effects, loops or live observations.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Met,
    Unmet,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Applicable,
    NotApplicable,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub id: String,
    pub description: String,
    pub observed: State,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub condition: String,
    pub applicability: Applicability,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub description: String,
    pub requirements: Vec<Requirement>,
}
/// Supplied independently from the plan. Empty requirements mean only that this
/// contract declares none, not that the real action requires no permission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub steps: Vec<Node>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Action {
        action: String,
    },
    Branch {
        condition: String,
        when_met: Vec<Node>,
        when_unmet: Vec<Node>,
        when_unknown: Vec<Node>,
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    contract: Contract,
    plan: Plan,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Covered,
    Uncovered,
    Unresolved,
    Unreachable,
    NoReachableActions,
}
#[derive(Clone, Debug, Serialize)]
pub struct PathCondition {
    pub condition: String,
    pub state: State,
}
#[derive(Debug, Serialize)]
pub struct Issue {
    pub condition: String,
    pub reason: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Occurrence {
    pub location: String,
    pub action: String,
    pub path: Vec<PathCondition>,
    pub status: Coverage,
    pub matches_observation: bool,
    pub issues: Vec<Issue>,
}
#[derive(Debug, Serialize)]
pub struct CoverageReport {
    pub protocol: &'static str,
    pub status: Coverage,
    pub occurrences: Vec<Occurrence>,
    pub limit: &'static str,
}

/// Check all feasible paths, not just the branch matching the supplied snapshot.
/// This establishes coverage only relative to the supplied immutable contract.
pub fn check(contract: &Contract, plan: &Plan) -> Result<CoverageReport> {
    ensure!(
        contract.conditions.len() <= 32 && contract.actions.len() <= 32,
        "contract exceeds declaration budget"
    );
    let mut conditions = BTreeMap::new();
    for condition in &contract.conditions {
        declaration(&condition.id, &condition.description)?;
        ensure!(
            conditions
                .insert(condition.id.as_str(), condition)
                .is_none(),
            "duplicate condition ID"
        );
    }
    let mut actions = BTreeMap::new();
    for action in &contract.actions {
        declaration(&action.id, &action.description)?;
        ensure!(
            actions.insert(action.id.as_str(), action).is_none(),
            "duplicate action ID"
        );
        ensure!(
            action.requirements.len() <= 16,
            "action exceeds requirement budget"
        );
        let mut seen = BTreeSet::new();
        for requirement in &action.requirements {
            ensure!(
                conditions.contains_key(requirement.condition.as_str()),
                "unknown requirement condition"
            );
            ensure!(
                seen.insert(&requirement.condition),
                "duplicate action requirement"
            );
        }
    }
    // Check even unreachable arms. The plan must not hide invalid references.
    ensure!(plan.steps.len() <= 128, "plan exceeds node budget");
    let mut pending: Vec<_> = plan.steps.iter().map(|node| (node, 0usize)).collect();
    let mut count = 0;
    while let Some((node, depth)) = pending.pop() {
        count += 1;
        ensure!(count <= 128 && depth <= 8, "plan exceeds node/depth budget");
        match node {
            Node::Action { action } => ensure!(
                actions.contains_key(action.as_str()),
                "unknown action reference"
            ),
            Node::Branch {
                condition,
                when_met,
                when_unmet,
                when_unknown,
            } => {
                ensure!(
                    conditions.contains_key(condition.as_str()),
                    "unknown branch condition"
                );
                for arm in [when_met, when_unmet, when_unknown] {
                    ensure!(
                        count + pending.len() + arm.len() <= 128,
                        "plan exceeds node budget"
                    );
                    pending.extend(arm.iter().map(|node| (node, depth + 1)));
                }
                ensure!(count + pending.len() <= 128, "plan exceeds node budget");
            }
        }
    }
    let mut occurrences = Vec::new();
    walk(
        &plan.steps,
        "/steps",
        &mut Vec::new(),
        &conditions,
        &actions,
        &mut occurrences,
    );
    let status = if occurrences.iter().any(|x| x.status == Coverage::Uncovered) {
        Coverage::Uncovered
    } else if occurrences.iter().any(|x| x.status == Coverage::Unresolved) {
        Coverage::Unresolved
    } else if occurrences.iter().any(|x| x.status == Coverage::Covered) {
        Coverage::Covered
    } else {
        Coverage::NoReachableActions
    };
    Ok(CoverageReport { protocol: "guard-coverage-v1", status, occurrences,
        limit: "Fixture-authored contract and immutable condition semantics; coverage is not semantic entailment, contract completeness, goal achievement, live observation freshness or execution authority. Snapshot observations do not supply guards. No actions execute." })
}

/// Bounded JSON adapter for synthetic qualification inputs, not a model tool.
pub fn check_json(input: &str) -> Result<CoverageReport> {
    ensure!(input.len() <= 65_536, "guard input exceeds 64 KiB");
    let input: Input = serde_json::from_str(input).context("invalid guard input")?;
    check(&input.contract, &input.plan)
}

fn declaration(id: &str, description: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b)),
        "invalid declaration ID"
    );
    ensure!(
        !description.trim().is_empty() && description.len() <= 500,
        "invalid declaration description"
    );
    Ok(())
}

fn walk(
    nodes: &[Node],
    location: &str,
    path: &mut Vec<PathCondition>,
    conditions: &BTreeMap<&str, &Condition>,
    actions: &BTreeMap<&str, &Action>,
    out: &mut Vec<Occurrence>,
) {
    for (i, node) in nodes.iter().enumerate() {
        let location = format!("{location}/{i}");
        match node {
            Node::Action { action } => {
                // Repeated observations are of the same immutable condition in
                // this experiment; preserve contradictory paths in the report.
                let reachable = !path.iter().enumerate().any(|(i, a)| {
                    path[..i]
                        .iter()
                        .any(|b| a.condition == b.condition && a.state != b.state)
                });
                let mut issues = Vec::new();
                let mut status = Coverage::Covered;
                if !reachable {
                    status = Coverage::Unreachable;
                } else {
                    for requirement in &actions[action.as_str()].requirements {
                        match requirement.applicability {
                            Applicability::NotApplicable => {}
                            Applicability::Unknown => {
                                if status != Coverage::Uncovered {
                                    status = Coverage::Unresolved;
                                }
                                issues.push(Issue {
                                    condition: requirement.condition.clone(),
                                    reason: "rule_applicability_unknown",
                                });
                            }
                            Applicability::Applicable => {
                                if !path.iter().any(|p| {
                                    p.condition == requirement.condition && p.state == State::Met
                                }) {
                                    status = Coverage::Uncovered;
                                    issues.push(Issue {
                                        condition: requirement.condition.clone(),
                                        reason: "required_met_guard_missing",
                                    });
                                }
                            }
                        }
                    }
                }
                out.push(Occurrence {
                    location,
                    action: action.clone(),
                    path: path.clone(),
                    status,
                    matches_observation: reachable
                        && path
                            .iter()
                            .all(|p| conditions[p.condition.as_str()].observed == p.state),
                    issues,
                });
            }
            Node::Branch {
                condition,
                when_met,
                when_unmet,
                when_unknown,
            } => {
                for (state, name, arm) in [
                    (State::Met, "when_met", when_met),
                    (State::Unmet, "when_unmet", when_unmet),
                    (State::Unknown, "when_unknown", when_unknown),
                ] {
                    path.push(PathCondition {
                        condition: condition.clone(),
                        state,
                    });
                    walk(
                        arm,
                        &format!("{location}/{name}"),
                        path,
                        conditions,
                        actions,
                        out,
                    );
                    // Neither a guard nor the last branch's state flows onward.
                    path.pop();
                }
            }
        }
    }
}
