//! Independent public-API/process qualification. Every input is synthetic.
#![cfg(feature = "memory-prototype")]

use mecha_core::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{
        decision::{Contract, EvidenceView, Objective},
        durable::TaskSpec,
        durable_decision::{AttemptKind, ModelReceipt},
        model_evaluation::ModelReply,
        Access, Capture, Memory, SourceKey,
    },
    memory_decision_runtime::{self as runtime, ModelSpec},
    memory_runtime,
    tool::{ModeApprover, ToolCtx},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

const MODEL: &str = "synthetic-scripted-model";
const CANARY: &str = "SYNTHETIC_MODEL_AUDIT_CANARY";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mecha-model-qual-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn db(&self) -> PathBuf {
        self.0.join("synthetic.memory-prototype.sqlite")
    }
    fn open(&self) -> Memory {
        Memory::open(&self.db()).unwrap()
    }
    async fn setup(&self) -> String {
        let mut db = Memory::create_durable_model(&self.db(), &access().domain).unwrap();
        let input = db
            .capture(capture("Synthetic original prerequisite"))
            .unwrap();
        std::fs::write(self.0.join("measurements.json"), json!({
            "kind":"host_authored_synthetic_measurements_v1", "workload":"Synthetic qualification",
            "complete":true,"observations":[
                {"choice":"Nori","observed_minutes":4,"energy":9,"quality":8},
                {"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},
                {"choice":"Quill","observed_minutes":6,"energy":5,"quality":7}]
        }).to_string()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: self.0.clone(),
                    path: "measurements.json".into(),
                    max_bytes: 4096,
                    max_lines: 4,
                },
                &[input],
                &access(),
            )
            .unwrap();
        let cx = RunContext::new(
            ToolCtx {
                workspace: self.0.clone(),
                ..ToolCtx::default()
            },
            Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            }),
        );
        memory_runtime::acquire(&mut db, &read, &access(), &cx, &mut Conversation::new())
            .await
            .unwrap();
        runtime::bind(
            &mut db,
            &read,
            ModelSpec {
                contract: Contract {
                    workload: "Synthetic qualification".into(),
                    objective: Objective::MinimizeMinutes,
                    energy_budget: 10,
                    minimum_quality: 6,
                },
                model: MODEL.into(),
                seed: 7,
                view: EvidenceView::Observed,
            },
            &access(),
        )
        .unwrap()
        .id()
        .into()
    }
    fn receipt(&self, db: &Memory, id: &str) -> ModelReceipt {
        db.durable_model_reopen(id, &access()).unwrap()
    }
    fn child(&self, id: &str, mode: &str) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "durable_model_process_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("MECHA_MODEL_QUAL_ROOT", &self.0)
            .env("MECHA_MODEL_QUAL_ID", id)
            .env("MECHA_MODEL_QUAL_MODE", mode)
            .env("MECHA_SESSION_KIND", "test")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
    fn wait_barrier(&self, child: &mut std::process::Child) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.0.join("barrier").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before barrier"
            );
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child barrier timeout");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn access() -> Access {
    Access {
        domain: "synthetic-model-qualification".into(),
        scopes: ["fixture-private".into()].into(),
    }
}
fn source() -> SourceKey {
    SourceKey {
        connector: "synthetic".into(),
        account: "model-qualification".into(),
        id: "prerequisite".into(),
    }
}
fn capture(text: &str) -> Capture {
    Capture {
        source: source(),
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    }
}
fn citation_ids(schema: &Value) -> Vec<String> {
    // The schema description exposes every visible citation, never the gold subset.
    let description = schema["description"].as_str().unwrap_or("");
    let ids = description
        .strip_prefix("Allowed citation IDs: ")
        .unwrap_or("[]");
    serde_json::from_str(ids).unwrap()
}
fn answer(schema: &Value, choice: &str) -> ModelReply {
    ModelReply {
        text: json!({"choice":choice,"citations":citation_ids(schema),"reason":CANARY}).to_string(),
        model: MODEL.into(),
        input_tokens: Some(11),
        output_tokens: Some(7),
        error: None,
    }
}
fn decision(db: &Memory, receipt: &ModelReceipt) -> Value {
    db.durable_model_decision(receipt, &access())
        .unwrap()
        .unwrap()
}
fn disposition(value: &Value) -> &str {
    value["disposition"].as_str().expect("decision disposition")
}

#[test]
#[ignore = "self-spawned synthetic qualification helper"]
fn durable_model_process_helper() {
    let Ok(root) = std::env::var("MECHA_MODEL_QUAL_ROOT") else {
        return;
    };
    let f = Fixture(PathBuf::from(root));
    let id = std::env::var("MECHA_MODEL_QUAL_ID").unwrap();
    let mode = std::env::var("MECHA_MODEL_QUAL_MODE").unwrap();
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut calls = 0;
        let steps = if mode.starts_with("retry-") { 3 } else { 1 };
        for _ in 0..steps {
            runtime::advance(&mut db, &receipt, &access(), |_, schema, guard| {
                calls += 1;
                let barrier = f.0.join("barrier");
                let block =
                    (mode == "inflight" && calls == 1) || (mode == "retry-inflight" && calls == 2);
                let choice = if mode.starts_with("retry-") && calls == 1 {
                    "Arden"
                } else {
                    "Nori"
                };
                async move {
                    guard.check()?;
                    if block {
                        std::fs::write(barrier, "reserved, before response").unwrap();
                        std::future::pending::<()>().await;
                    }
                    Ok(answer(&schema.unwrap(), choice))
                }
            })
            .await
            .unwrap();
        }
    });
    std::fs::write(
        f.0.join("barrier"),
        "raw response settled, before assessment",
    )
    .unwrap();
    // Actual process loss, rather than dropping a scripted future.
    loop {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn process_loss_never_replays_inflight_and_recovers_settled_response() {
    for mode in ["inflight", "settled", "retry-inflight", "retry-settled"] {
        let f = Fixture::new();
        let id = f.setup().await;
        let mut child = f.child(&id, mode);
        f.wait_barrier(&mut child);
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        runtime::decide(&mut db, &receipt, &access(), |_, _, _| async {
            panic!("reopened inference was replayed")
        })
        .await
        .unwrap();
        let result = db.durable_model_decision(&receipt, &access()).unwrap();
        if mode.ends_with("settled") {
            assert_eq!(disposition(result.as_ref().unwrap()), "recommend");
        } else {
            assert!(result.is_none());
        }
    }
}

#[tokio::test]
async fn model_abstention_and_host_withholding_are_distinct_decisions() {
    for choice in ["abstain", "Nori"] {
        let f = Fixture::new();
        let id = f.setup().await;
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        let mut calls = 0;
        runtime::decide(&mut db, &receipt, &access(), |_, schema, guard| {
            calls += 1;
            async move {
                guard.check()?;
                let mut reply = answer(&schema.unwrap(), choice);
                reply.text = json!({"choice":choice,"citations":[],"reason":CANARY}).to_string();
                Ok(reply)
            }
        })
        .await
        .unwrap();
        assert!((1..=2).contains(&calls));
        assert_eq!(
            disposition(&decision(&db, &receipt)),
            if choice == "abstain" {
                "abstain"
            } else {
                "withheld"
            }
        );
    }
}

#[tokio::test]
async fn persisted_final_reuses_read_and_model_after_fixture_deleted() {
    let f = Fixture::new();
    let id = f.setup().await;
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    let mut calls = 0;
    runtime::decide(&mut db, &receipt, &access(), |_, schema, guard| {
        calls += 1;
        async move {
            guard.check()?;
            Ok(answer(&schema.unwrap(), "Nori"))
        }
    })
    .await
    .unwrap();
    assert_eq!(calls, 1);
    let original = decision(&db, &receipt);
    assert_eq!(disposition(&original), "recommend");
    drop(db);
    std::fs::remove_file(f.0.join("measurements.json")).unwrap();
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    runtime::decide(&mut db, &receipt, &access(), |_, _, _| async {
        panic!("final result invoked model")
    })
    .await
    .unwrap();
    assert_eq!(decision(&db, &receipt), original);
}

#[tokio::test]
async fn malformed_refused_wrong_model_and_transport_errors_are_audited_not_retried() {
    for failure in [
        "malformed",
        "duplicate",
        "unknown",
        "empty",
        "refused",
        "wrong-model",
        "transport",
        "oversized",
    ] {
        let f = Fixture::new();
        let id = f.setup().await;
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        let mut calls = 0;
        runtime::decide(&mut db,&receipt,&access(), |_,schema,guard| {calls+=1; async move {
            guard.check()?; if failure=="transport" {anyhow::bail!("synthetic transport failure {CANARY}");}
            let mut reply=answer(&schema.unwrap(),"Nori");
            match failure {
                "malformed"=>reply.text=format!("{CANARY} {{"),
                "duplicate"=>reply.text="{\"choice\":\"Nori\",\"choice\":\"Arden\",\"citations\":[],\"reason\":\"synthetic\"}".into(),
                "unknown"=>reply.text=json!({"choice":"not-a-method","citations":[],"reason":CANARY}).to_string(),
                "empty"=>reply.text.clear(), "refused"=>reply.error=Some("synthetic refusal".into()),
                "oversized"=>reply.text=CANARY.repeat(3000),
                "wrong-model"=>reply.model="different-synthetic-model".into(), _=>{}
            }; Ok(reply)
        }}).await.unwrap();
        assert_eq!(calls, 1, "{failure}");
        assert_eq!(disposition(&decision(&db, &receipt)), "failed", "{failure}");
        assert!(db
            .durable_model_attempt(&receipt, AttemptKind::Initial, &access())
            .unwrap()
            .unwrap()
            .result
            .is_some());
        runtime::decide(&mut db, &receipt, &access(), |_, _, _| async {
            panic!("failed call retried")
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn both_attempts_are_auditable_and_forgetting_erases_every_sibling() {
    let f = Fixture::new();
    let id = f.setup().await;
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    let mut calls = 0;
    runtime::decide(&mut db, &receipt, &access(), |_, schema, guard| {
        calls += 1;
        let choice = if calls == 1 { "Arden" } else { "Nori" };
        async move {
            guard.check()?;
            Ok(answer(&schema.unwrap(), choice))
        }
    })
    .await
    .unwrap();
    assert_eq!(calls, 2);
    assert_eq!(disposition(&decision(&db, &receipt)), "recommend");
    for kind in [AttemptKind::Initial, AttemptKind::Retry] {
        let audit = db
            .durable_model_attempt(&receipt, kind, &access())
            .unwrap()
            .unwrap();
        assert!(audit.result.unwrap().to_string().contains(CANARY));
        assert!(audit.assessment.is_some());
    }
    db.forget(&source()).unwrap();
    drop(db);
    let db = f.open();
    let receipt = f.receipt(&db, &id);
    assert!(db.durable_model_decision(&receipt, &access()).is_err());
    for kind in [AttemptKind::Initial, AttemptKind::Retry] {
        assert!(db.durable_model_attempt(&receipt, kind, &access()).is_err());
    }
    drop(db);
    for entry in std::fs::read_dir(&f.0).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("sqlite")
        {
            assert!(!std::fs::read(path)
                .unwrap()
                .windows(CANARY.len())
                .any(|w| w == CANARY.as_bytes()));
        }
    }
}

#[tokio::test]
async fn lifecycle_changes_during_either_model_slot_block_late_recommendations() {
    for slot in [1, 2] {
        for change in ["cancel", "forget", "source-change", "scope-change"] {
            let f = Fixture::new();
            let id = f.setup().await;
            let mut db = f.open();
            let receipt = f.receipt(&db, &id);
            let mut calls = 0;
            let result = runtime::decide(&mut db, &receipt, &access(), |_, schema, guard| {
                calls += 1;
                let current = calls;
                let path = f.db();
                let id = id.clone();
                async move {
                    guard.check()?;
                    if current == slot {
                        let mut other = Memory::open(&path).unwrap();
                        let receipt = other.durable_model_reopen(&id, &access()).unwrap();
                        match change {
                            "cancel" => {
                                other.durable_model_cancel(&receipt, &access()).unwrap();
                            }
                            "forget" => {
                                other.forget(&source()).unwrap();
                            }
                            "source-change" => {
                                other
                                    .capture(capture("Synthetic corrected prerequisite"))
                                    .unwrap();
                            }
                            "scope-change" => {
                                let mut changed = capture("Synthetic original prerequisite");
                                changed.scopes = ["different-scope".into()].into();
                                other.capture(changed).unwrap();
                            }
                            _ => unreachable!(),
                        }
                        assert!(guard.check().is_err(), "guard survived {change}");
                    }
                    Ok(answer(
                        &schema.unwrap(),
                        if current == 1 { "Arden" } else { "Nori" },
                    ))
                }
            })
            .await;
            assert_eq!(calls, slot, "{change}");
            let stored = db.durable_model_decision(&receipt, &access());
            assert!(
                stored.is_err() || stored.unwrap().is_none(),
                "late decision after {change}"
            );
            if change == "cancel" {
                assert!(result.is_ok());
                let kind = if slot == 1 {
                    AttemptKind::Initial
                } else {
                    AttemptKind::Retry
                };
                assert!(
                    db.durable_model_attempt(&receipt, kind, &access())
                        .unwrap()
                        .unwrap()
                        .result
                        .is_some(),
                    "cancelled actual response lost its audit"
                );
            }
        }
    }
}
