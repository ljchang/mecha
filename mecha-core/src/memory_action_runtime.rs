//! Host-authorized execution of one closed synthetic fixture, with independent
//! observations and resumable feedback. No provider, tool, or executable path.
use crate::{
    memory::{
        decision::Contract,
        durable_action::{ActionGuard, ActionPermit, ActionPhase, ActionReceipt, ActionStatus},
        durable_decision::ModelReceipt,
        Access, Memory,
    },
    memory_decision_runtime::authenticated_recommendation,
    planning::{StepFeedback, Verification},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Closed task identity for this version of the synthetic handler.
pub const WORKLOAD: &str = "durable-action-fixture-v1";
const VERSION: &str = "synthetic-actions-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureVersion {
    MeasurementsV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureState {
    Baseline,
    Refused,
    Unknown,
}

/// Explicit host authorization. Choice is authenticated from the retained model
/// workflow; model text cannot select fixture behavior or any executable input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    pub fixture: FixtureVersion,
    pub state: FixtureState,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Measurements {
    observed_minutes: u32,
    energy: u32,
    quality: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedAction {
    version: String,
    spec: ActionSpec,
    choice: String,
    contract: Contract,
    expected_from_evidence: Option<Measurements>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Observed {
        observed_minutes: u32,
        energy: u32,
        quality: u32,
    },
    Refused {
        reason: String,
    },
    Unknown {
        reason: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    kind: String,
    fixture: FixtureVersion,
    state: FixtureState,
    choice: String,
    expected_from_evidence: Option<Measurements>,
    outcome: Outcome,
}

/// Bind one explicit fixture authorization to a current, authenticated final
/// recommendation. Source expectations are retained separately from actuals.
pub fn bind(
    db: &mut Memory,
    model: &ModelReceipt,
    spec: ActionSpec,
    access: &Access,
) -> Result<ActionReceipt> {
    let recommendation = authenticated_recommendation(db, model, access)?;
    ensure!(
        recommendation.contract.workload == WORKLOAD,
        "action fixture workload mismatch"
    );
    let sealed = SealedAction {
        version: VERSION.into(),
        spec,
        choice: recommendation.choice,
        contract: recommendation.contract,
        expected_from_evidence: recommendation
            .expected_from_evidence
            .map(serde_json::from_value)
            .transpose()?,
    };
    db.bind_action(model, serde_json::to_value(sealed)?, access)
}

fn verified_spec(db: &Memory, receipt: &ActionReceipt, access: &Access) -> Result<SealedAction> {
    let status = db.durable_action_status(receipt, access)?;
    let sealed: SealedAction = serde_json::from_value(db.durable_action_spec(receipt, access)?)?;
    ensure!(
        sealed.version == VERSION,
        "unsupported action runtime version"
    );
    let model = db.durable_model_reopen(&status.model_id, access)?;
    let recommendation = authenticated_recommendation(db, &model, access)?;
    let expected: Option<Measurements> = recommendation
        .expected_from_evidence
        .map(serde_json::from_value)
        .transpose()?;
    ensure!(
        recommendation.contract.workload == WORKLOAD
            && sealed.contract == recommendation.contract
            && sealed.choice == recommendation.choice
            && sealed.expected_from_evidence == expected,
        "action binding differs from authenticated recommendation"
    );
    Ok(sealed)
}

/// A validated retained actual, never reconstructed from arbitrary recalled text.
pub(crate) struct RetainedOutcome {
    pub original: Value,
    pub choice: String,
    pub measurement: Option<(u32, u32, u32)>,
}
/// Authenticate an existing finalized action and its complete model/read lineage.
/// Numeric values come only from its retained observation, never fixture truth.
pub(crate) fn retained_outcome(
    db: &Memory,
    receipt: &ActionReceipt,
    access: &Access,
) -> Result<RetainedOutcome> {
    let status = db.durable_action_status(receipt, access)?;
    ensure!(
        status.phase == ActionPhase::Finalized && !status.cancelled,
        "uncancelled finalized action required"
    );
    let sealed = verified_spec(db, receipt, access)?;
    let original = db
        .durable_action_observation(receipt, access)?
        .context("missing retained action observation")?;
    ensure!(
        original["kind"] == "host_action_observation"
            && original["action_id"] == receipt.id()
            && original["cancelled_action"] == false,
        "invalid host action observation envelope"
    );
    let observation: Observation = serde_json::from_value(original["observation"].clone())?;
    ensure!(
        observation.kind == "host_observed_synthetic_action_v1"
            && observation.fixture == sealed.spec.fixture
            && observation.state == sealed.spec.state
            && observation.choice == sealed.choice
            && observation.expected_from_evidence == sealed.expected_from_evidence,
        "observation differs from sealed action"
    );
    let measurement = match (&sealed.spec.state, &observation.outcome) {
        (
            FixtureState::Baseline,
            Outcome::Observed {
                observed_minutes,
                energy,
                quality,
            },
        ) => Some((*observed_minutes, *energy, *quality)),
        (FixtureState::Refused, Outcome::Refused { .. })
        | (FixtureState::Unknown, Outcome::Unknown { .. }) => None,
        _ => anyhow::bail!("outcome differs from sealed fixture state"),
    };
    Ok(RetainedOutcome {
        original,
        choice: observation.choice,
        measurement,
    })
}

/// A live, one-use reservation. It cannot be cloned, serialized, or recovered
/// from an indeterminate operational row after process loss.
pub struct PendingAction {
    permit: ActionPermit,
    guard: ActionGuard,
    sealed: SealedAction,
}
/// An actual completed host handler result; arbitrary JSON cannot construct one.
pub struct ExecutedAction {
    permit: ActionPermit,
    observation: Observation,
}

pub fn reserve(db: &mut Memory, receipt: &ActionReceipt, access: &Access) -> Result<PendingAction> {
    let sealed = verified_spec(db, receipt, access)?;
    let permit = db.reserve_action(receipt, access)?;
    let guard = db.action_guard(&permit, access)?;
    Ok(PendingAction {
        permit,
        guard,
        sealed,
    })
}
impl PendingAction {
    /// Execute only this immutable host binding. Recheck cancellation and source
    /// authority immediately before the finite handler; no transaction spans it.
    pub fn execute(self) -> Result<ExecutedAction> {
        self.guard.check()?;
        let outcome = match self.sealed.spec.state {
            FixtureState::Baseline => {
                // Independent fixture truth, never read from source expectations.
                let (observed_minutes, energy, quality) = match self.sealed.choice.as_str() {
                    "Nori" => (4, 9, 8),
                    "Arden" => (8, 3, 9),
                    "Quill" => (6, 5, 6),
                    _ => anyhow::bail!("unknown authenticated fixture choice"),
                };
                Outcome::Observed {
                    observed_minutes,
                    energy,
                    quality,
                }
            }
            FixtureState::Refused => Outcome::Refused {
                reason: "Host fixture refused execution.".into(),
            },
            FixtureState::Unknown => Outcome::Unknown {
                reason: "Host fixture returned no measurement.".into(),
            },
        };
        Ok(ExecutedAction {
            permit: self.permit,
            observation: Observation {
                kind: "host_observed_synthetic_action_v1".into(),
                fixture: self.sealed.spec.fixture,
                state: self.sealed.spec.state,
                choice: self.sealed.choice,
                expected_from_evidence: self.sealed.expected_from_evidence,
                outcome,
            },
        })
    }
}
/// Retain the actual result once. Cancellation can preserve an already executed
/// result; changed, forgotten or revoked sources still refuse late content.
pub fn settle(db: &mut Memory, result: ExecutedAction, access: &Access) -> Result<ActionStatus> {
    db.settle_action(
        result.permit,
        serde_json::to_value(result.observation)?,
        access,
    )
}

/// One resumable step: reserve+execute+settle, or pure feedback settlement.
/// Indeterminate reservations remain spent and cannot replay the handler.
pub fn advance(db: &mut Memory, receipt: &ActionReceipt, access: &Access) -> Result<ActionStatus> {
    let status = db.durable_action_status(receipt, access)?;
    match status.phase {
        ActionPhase::Cancelled | ActionPhase::Forgotten | ActionPhase::Indeterminate => Ok(status),
        ActionPhase::Finalized => {
            db.durable_action_feedback(receipt, access)?
                .context("missing action feedback")?;
            Ok(status)
        }
        ActionPhase::Ready => {
            let pending = reserve(db, receipt, access)?;
            settle(db, pending.execute()?, access)
        }
        ActionPhase::Observed => {
            let sealed = verified_spec(db, receipt, access)?;
            let value = db
                .durable_action_observation(receipt, access)?
                .context("missing actual action observation")?;
            ensure!(
                value["kind"] == "host_action_observation",
                "not a host action observation"
            );
            let observation: Observation = serde_json::from_value(value["observation"].clone())?;
            ensure!(
                observation.kind == "host_observed_synthetic_action_v1"
                    && observation.fixture == sealed.spec.fixture
                    && observation.state == sealed.spec.state
                    && observation.choice == sealed.choice
                    && observation.expected_from_evidence == sealed.expected_from_evidence,
                "observation differs from sealed action"
            );
            let feedback = feedback(&sealed, &observation);
            db.finalize_action(receipt, feedback, access)
        }
    }
}
/// Drive the two-step bounded action lifecycle, without inference or reacquisition.
pub fn run(db: &mut Memory, receipt: &ActionReceipt, access: &Access) -> Result<ActionStatus> {
    let status = advance(db, receipt, access)?;
    if status.phase == ActionPhase::Observed {
        advance(db, receipt, access)
    } else {
        Ok(status)
    }
}
fn feedback(sealed: &SealedAction, observation: &Observation) -> Value {
    let (observed, resource, quality, verification, outcome_status) = match observation.outcome {
        Outcome::Observed {
            energy, quality, ..
        } => {
            let resource = energy <= sealed.contract.energy_budget;
            let quality = quality >= sealed.contract.minimum_quality;
            (
                true,
                Some(resource),
                Some(quality),
                if resource && quality {
                    Verification::Passed
                } else {
                    Verification::Failed
                },
                "observed",
            )
        }
        Outcome::Refused { .. } => (false, None, None, Verification::Refused, "refused"),
        Outcome::Unknown { .. } => (false, None, None, Verification::Pending, "unknown"),
    };
    let step_feedback = StepFeedback {
        criterion: None,
        completion_batch: None,
        call_id: None,
        step: format!("Execute synthetic fixture {}", sealed.choice),
        goal: None,
        expected: Some(format!(
            "energy <= {}; quality >= {}",
            sealed.contract.energy_budget, sealed.contract.minimum_quality
        )),
        expected_calls: None,
        actual_calls: None,
        verification,
        check_tampered: false,
    };
    json!({"kind":"synthetic_action_feedback_v1","choice":sealed.choice,
        "outcome_status":outcome_status,"observed":observed,
        "resource_constraint_met":resource,"quality_constraint_met":quality,
        "step_feedback":step_feedback})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_checks_quality_and_inclusive_resource_limits_from_actuals() {
        let sealed = SealedAction {
            version: VERSION.into(),
            spec: ActionSpec {
                fixture: FixtureVersion::MeasurementsV1,
                state: FixtureState::Baseline,
            },
            choice: "Quill".into(),
            contract: Contract {
                workload: WORKLOAD.into(),
                objective: crate::memory::decision::Objective::MinimizeMinutes,
                energy_budget: 5,
                minimum_quality: 7,
            },
            expected_from_evidence: Some(Measurements {
                observed_minutes: 1,
                energy: 1,
                quality: 9,
            }),
        };
        for (energy, quality, want) in [(5, 7, "passed"), (5, 6, "failed"), (6, 7, "failed")] {
            let observation = Observation {
                kind: "host_observed_synthetic_action_v1".into(),
                fixture: FixtureVersion::MeasurementsV1,
                state: FixtureState::Baseline,
                choice: "Quill".into(),
                expected_from_evidence: sealed.expected_from_evidence.clone(),
                outcome: Outcome::Observed {
                    observed_minutes: 6,
                    energy,
                    quality,
                },
            };
            let value = feedback(&sealed, &observation);
            assert_eq!(value["step_feedback"]["verification"], want);
            assert_eq!(value["observed"], true);
        }
    }
}
