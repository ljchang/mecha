use super::*;
use crate::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{durable::TaskSpec, Capture, SourceKey},
    tool::{ModeApprover, ToolCtx},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct Fixture {
    root: PathBuf,
    db: Memory,
    receipt: ModelReceipt,
    access: Access,
}
impl Fixture {
    async fn new(view: EvidenceView) -> Self {
        let root =
            std::env::temp_dir().join(format!("mecha-model-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let access = Access {
            domain: "synthetic-model-runtime".into(),
            scopes: ["fixture".into()].into(),
        };
        let mut db = Memory::create_durable_model(
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
                text: "synthetic source binding".into(),
                occurred_at: None,
                scopes: access.scopes.clone(),
                transcript: None,
            })
            .unwrap();
        std::fs::write(root.join("measurements.json"), r#"{"kind":"host_authored_synthetic_measurements_v1","workload":"runtime-fixture","observations":[{"choice":"Nori","observed_minutes":4,"energy":8,"quality":8},{"choice":"Arden","observed_minutes":9,"energy":2,"quality":9},{"choice":"Quill","observed_minutes":6,"energy":4,"quality":8}],"complete":true}"#).unwrap();
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
        crate::memory_runtime::acquire(&mut db, &read, &access, &cx, &mut Conversation::default())
            .await
            .unwrap();
        let receipt = bind(
            &mut db,
            &read,
            ModelSpec {
                contract: Contract {
                    workload: "runtime-fixture".into(),
                    objective: crate::memory::decision::Objective::MinimizeMinutes,
                    energy_budget: 10,
                    minimum_quality: 7,
                },
                model: "fixture-model".into(),
                seed: 7,
                view,
            },
            &access,
        )
        .unwrap();
        Self {
            root,
            db,
            receipt,
            access,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn reply(text: &str) -> ModelReply {
    ModelReply {
        text: text.into(),
        model: "fixture-model".into(),
        input_tokens: Some(20),
        output_tokens: Some(10),
        error: None,
    }
}

#[tokio::test]
async fn settled_response_can_be_assessed_after_reopen_without_more_inference() {
    let mut f = Fixture::new(EvidenceView::None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let status = advance(&mut f.db, &f.receipt, &f.access, move |_, _, guard| {
        count.fetch_add(1, Ordering::SeqCst);
        async move {
            guard.check()?;
            Ok(reply(
                r#"{"choice":"abstain","citations":[],"reason":"Missing comparison evidence"}"#,
            ))
        }
    })
    .await
    .unwrap();
    assert_eq!(status.initial.phase, AttemptPhase::Settled);
    let id = f.receipt.id().to_owned();
    let mut reopened = Memory::open(&f.root.join("trial.memory-prototype.sqlite")).unwrap();
    let receipt = reopened.durable_model_reopen(&id, &f.access).unwrap();
    let status = decide(&mut reopened, &receipt, &f.access, |_, _, _| async {
        panic!("settled inference must not repeat")
    })
    .await
    .unwrap();
    assert_eq!(status.phase, ModelPhase::Finalized);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        reopened
            .durable_model_decision(&receipt, &f.access)
            .unwrap()
            .unwrap()["disposition"],
        "abstain"
    );
}

#[tokio::test]
async fn malformed_reply_is_a_retained_failure_without_hidden_retry() {
    let mut f = Fixture::new(EvidenceView::Observed).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let status = decide(&mut f.db, &f.receipt, &f.access, move |_, _, guard| {
        count.fetch_add(1, Ordering::SeqCst);
        async move {
            guard.check()?;
            Ok(reply("not JSON"))
        }
    })
    .await
    .unwrap();
    assert_eq!(status.phase, ModelPhase::Finalized);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(status.retry.phase, AttemptPhase::Unused);
    let attempt =
        f.db.durable_model_attempt(&f.receipt, AttemptKind::Initial, &f.access)
            .unwrap()
            .unwrap();
    assert_eq!(attempt.result.unwrap()["reply"]["text"], "not JSON");
    assert_eq!(
        f.db.durable_model_decision(&f.receipt, &f.access)
            .unwrap()
            .unwrap()["disposition"],
        "failed"
    );
}
