#![cfg(feature = "memory-prototype")]
//! The action's measured outcome must remain independent of source expectations.
use mecha_core::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{
        decision::{Contract, EvidenceView, Objective},
        durable::TaskSpec,
        durable_action::ActionPhase,
        durable_decision::ModelReceipt,
        model_evaluation::ModelReply,
        Access, Capture, Memory, SourceKey,
    },
    memory_action_runtime::{self as runtime, ActionSpec, FixtureState, FixtureVersion, WORKLOAD},
    memory_decision_runtime::{self, ModelSpec},
    tool::{ModeApprover, ToolCtx},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
struct Fixture {
    root: PathBuf,
    db: Memory,
    model: ModelReceipt,
    access: Access,
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mecha-action-api-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let access = Access {
            domain: "synthetic-action-test".into(),
            scopes: ["fixture".into()].into(),
        };
        let mut db = Memory::create_durable_action(
            &root.join("trial.memory-prototype.sqlite"),
            &access.domain,
        )
        .unwrap();
        let source = db
            .capture(Capture {
                source: SourceKey {
                    connector: "synthetic".into(),
                    account: "test".into(),
                    id: "source".into(),
                },
                text: "synthetic action source".into(),
                occurred_at: None,
                scopes: access.scopes.clone(),
                transcript: None,
            })
            .unwrap();
        std::fs::write(root.join("measurements.json"),json!({"kind":"host_authored_synthetic_measurements_v1","workload":WORKLOAD,"observations":[{"choice":"Nori","observed_minutes":4,"energy":2,"quality":8},{"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},{"choice":"Quill","observed_minutes":6,"energy":5,"quality":6}],"complete":true}).to_string()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: root.clone(),
                    path: "measurements.json".into(),
                    max_bytes: 16000,
                    max_lines: 128,
                },
                &[source],
                &access,
            )
            .unwrap();
        let cx = RunContext::new(
            ToolCtx {
                workspace: root.clone(),
                spill_dir: None,
                ..Default::default()
            },
            Arc::new(ModeApprover {
                mode: PermissionMode::ReadOnly,
            }),
        );
        mecha_core::memory_runtime::acquire(
            &mut db,
            &read,
            &access,
            &cx,
            &mut Conversation::default(),
        )
        .await
        .unwrap();
        let model = memory_decision_runtime::bind(
            &mut db,
            &read,
            ModelSpec {
                contract: Contract {
                    workload: WORKLOAD.into(),
                    objective: Objective::MinimizeMinutes,
                    energy_budget: 5,
                    minimum_quality: 7,
                },
                model: "scripted-fixture".into(),
                seed: 7,
                view: EvidenceView::Observed,
            },
            &access,
        )
        .unwrap();
        memory_decision_runtime::decide(&mut db,&model,&access,|_,_,guard| async move {guard.check()?;Ok(ModelReply {text:r#"{"choice":"Nori","citations":["e1","e2","e3"],"reason":"Fastest feasible source measurement"}"#.into(),model:"scripted-fixture".into(),input_tokens:None,output_tokens:None,error:None})}).await.unwrap();
        Self {
            root,
            db,
            model,
            access,
        }
    }
    fn bind(&mut self, state: FixtureState) -> mecha_core::memory::durable_action::ActionReceipt {
        runtime::bind(
            &mut self.db,
            &self.model,
            ActionSpec {
                fixture: FixtureVersion::MeasurementsV1,
                state,
            },
            &self.access,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn source_expectation_cannot_become_the_observed_measurement_or_feedback() {
    let mut f = Fixture::new().await;
    let action = f.bind(FixtureState::Baseline);
    assert_eq!(
        runtime::advance(&mut f.db, &action, &f.access)
            .unwrap()
            .phase,
        ActionPhase::Observed
    );
    let observation =
        f.db.durable_action_observation(&action, &f.access)
            .unwrap()
            .unwrap();
    assert_eq!(
        observation["observation"]["expected_from_evidence"]["energy"],
        2
    );
    assert_eq!(observation["observation"]["outcome"]["energy"], 9);
    let id = action.id().to_owned();
    let mut reopened = Memory::open(&f.root.join("trial.memory-prototype.sqlite")).unwrap();
    let action = reopened.durable_action_reopen(&id, &f.access).unwrap();
    assert_eq!(
        runtime::run(&mut reopened, &action, &f.access)
            .unwrap()
            .phase,
        ActionPhase::Finalized
    );
    let feedback = reopened
        .durable_action_feedback(&action, &f.access)
        .unwrap()
        .unwrap();
    assert_eq!(feedback["step_feedback"]["verification"], "failed");
    assert_eq!(feedback["resource_constraint_met"], false);
    assert_eq!(feedback["quality_constraint_met"], true);
    assert!(feedback["step_feedback"]["goal"].is_null());
    assert!(feedback["step_feedback"]["expected_calls"].is_null());
    assert!(feedback["step_feedback"]["actual_calls"].is_null());
    assert_eq!(
        runtime::run(&mut reopened, &action, &f.access)
            .unwrap()
            .phase,
        ActionPhase::Finalized
    );
    assert!(runtime::reserve(&mut reopened, &action, &f.access).is_err());
}
#[tokio::test]
async fn unavailable_outcomes_never_become_measurements_or_policy_failures() {
    for (state, verification) in [
        (FixtureState::Refused, "refused"),
        (FixtureState::Unknown, "pending"),
    ] {
        let mut f = Fixture::new().await;
        let action = f.bind(state);
        runtime::run(&mut f.db, &action, &f.access).unwrap();
        let observation =
            f.db.durable_action_observation(&action, &f.access)
                .unwrap()
                .unwrap();
        assert!(observation["observation"]["outcome"]["energy"].is_null());
        let feedback =
            f.db.durable_action_feedback(&action, &f.access)
                .unwrap()
                .unwrap();
        assert_eq!(feedback["step_feedback"]["verification"], verification);
        assert_eq!(feedback["observed"], false);
        assert!(feedback["resource_constraint_met"].is_null());
        assert!(feedback["quality_constraint_met"].is_null());
    }
}
#[test]
fn spec_rejects_executable_or_model_chosen_fields() {
    assert!(serde_json::from_value::<ActionSpec>(
        json!({"fixture":"measurements_v1","state":"baseline","path":"/tmp/elsewhere"})
    )
    .is_err());
    assert!(serde_json::from_value::<ActionSpec>(
        json!({"fixture":"measurements_v1","state":"baseline","choice":"Nori"})
    )
    .is_err());
}
