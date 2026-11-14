//! Actual-binary action/recall demo. The model setup is scripted; zero new inference.
#![cfg(feature = "memory-prototype")]
use mecha_core::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{
        decision::{Contract, EvidenceView, Objective},
        durable::TaskSpec,
        model_evaluation::ModelReply,
        Access, Capture, Memory, SourceKey,
    },
    memory_action_runtime::WORKLOAD,
    memory_decision_runtime::{self, ModelSpec},
    memory_runtime,
    tool::{ModeApprover, ToolCtx},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::Arc,
};
const DOMAIN: &str = "synthetic-demo";
const CANARY: &str = "SYNTHETIC_ACTION_CLI_CANARY";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mecha-action-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::write(root.join("home/config.toml"), "not = [valid toml").unwrap();
        std::fs::write(root.join("work/mecha.toml"), "not = [valid toml").unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn db(&self) -> PathBuf {
        self.0.join("synthetic.memory-prototype.sqlite")
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args([
                "memory",
                "--experimental",
                "--db",
                self.db().to_str().unwrap(),
            ])
            .args(args)
            .current_dir(self.0.join("work"))
            .env("MECHA_HOME", self.0.join("home"))
            .env("MECHA_SESSION_KIND", "test")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn act(&self, args: &[&str]) -> Value {
        let mut argv = vec![
            "durable-act",
            "--synthetic-input",
            "--domain",
            DOMAIN,
            "--scope",
            "fixture-mail",
        ];
        argv.extend_from_slice(args);
        self.json(&argv)
    }
    async fn model(&self) -> String {
        self.json(&["init-action", "--synthetic-input", "--domain", DOMAIN]);
        let mut db = Memory::open(&self.db()).unwrap();
        let input = db.capture(capture(CANARY)).unwrap();
        let work = self.0.join("work");
        std::fs::write(work.join("measurements.json"),json!({
            "kind":"host_authored_synthetic_measurements_v1","workload":WORKLOAD,"complete":true,
            "observations":[{"choice":"Nori","observed_minutes":4,"energy":2,"quality":8},
                {"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},
                {"choice":"Quill","observed_minutes":6,"energy":5,"quality":7}]
        }).to_string()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: work.clone(),
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
                workspace: work,
                ..ToolCtx::default()
            },
            Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            }),
        );
        memory_runtime::acquire(&mut db, &read, &access(), &cx, &mut Conversation::new())
            .await
            .unwrap();
        let receipt = memory_decision_runtime::bind(
            &mut db,
            &read,
            ModelSpec {
                contract: Contract {
                    workload: WORKLOAD.into(),
                    objective: Objective::MinimizeMinutes,
                    energy_budget: 5,
                    minimum_quality: 6,
                },
                model: "synthetic-cli-action-scripted".into(),
                seed: 7,
                view: EvidenceView::Observed,
            },
            &access(),
        )
        .unwrap();
        memory_decision_runtime::decide(&mut db, &receipt, &access(), |_, _, guard| async move {
            guard.check()?;
            Ok(ModelReply {
                text: json!({"choice":"Nori","citations":["e1","e2","e3"],"reason":CANARY})
                    .to_string(),
                model: "synthetic-cli-action-scripted".into(),
                input_tokens: None,
                output_tokens: None,
                error: None,
            })
        })
        .await
        .unwrap();
        receipt.id().into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn access() -> Access {
    Access {
        domain: DOMAIN.into(),
        scopes: ["fixture-mail".into()].into(),
    }
}
fn capture(text: &str) -> Capture {
    Capture {
        source: SourceKey {
            connector: "synthetic".into(),
            account: "action-cli".into(),
            id: "prerequisite".into(),
        },
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    }
}
const SPEC: &str = r#"{"fixture":"measurements_v1","state":"baseline"}"#;
#[tokio::test]
async fn end_to_end_cli_demo_retains_actual_failure_and_recalls_without_inference() {
    let f = Fixture::new();
    let model = f.model().await;
    let bound = f.act(&["bind", "--model", &model, "--spec", SPEC]);
    let id = bound["status"]["id"].as_str().unwrap();
    assert_eq!(bound["status"]["phase"], "ready");
    let observed = f.act(&["run", "--task", id]);
    assert_eq!(observed["status"]["phase"], "finalized");
    assert_eq!(
        observed["observation"]["observation"]["expected_from_evidence"]["energy"],
        2
    );
    assert_eq!(
        observed["observation"]["observation"]["outcome"]["energy"],
        9
    );
    assert_eq!(
        observed["feedback"]["step_feedback"]["verification"],
        "failed"
    );
    std::fs::remove_file(f.0.join("work/measurements.json")).unwrap();
    let resumed = f.act(&["run", "--task", id]);
    assert_eq!(observed["observation"], resumed["observation"]);
    assert_eq!(observed["feedback"], resumed["feedback"]);
    // Recall's existing explicit account/domain adapter returns source-linked untrusted evidence.
    let recall = f.json(&[
        "recall",
        "Nori",
        "--limit",
        "100",
        "--neighbors",
        "0",
        "--max-bytes",
        "100000",
    ]);
    assert!(recall
        .to_string()
        .contains("host_observed_synthetic_action_v1"));
    let report = json!({"protocol":"durable-synthetic-action-cli-qualification-v1","input":"synthetic","model_reply":"scripted",
        "new_model_inference_calls":0,"frozen_model_artifact_used":false,"source_expected_energy":2,"bound_energy_budget":5,
        "actual_host_fixture_energy":9,"verified_actual_outcome":"failed","bind":bound,"run":observed,"resume":resumed,"recall":recall});
    if let Ok(path) = std::env::var("MECHA_ACTION_QUAL_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn cli_requires_explicit_synthetic_ack_host_binding_and_current_access() {
    let f = Fixture::new();
    assert!(!f.run(&["init-action", "--domain", DOMAIN]).status.success());
    let model = f.model().await;
    for args in [
        vec![
            "durable-act",
            "--domain",
            DOMAIN,
            "--scope",
            "fixture-mail",
            "bind",
            "--model",
            &model,
            "--spec",
            SPEC,
        ],
        vec![
            "durable-act",
            "--synthetic-input",
            "--domain",
            DOMAIN,
            "--scope",
            "fixture-mail",
            "bind",
            "--model",
            &model,
        ],
        vec![
            "durable-act",
            "--synthetic-input",
            "--domain",
            "wrong-domain",
            "--scope",
            "fixture-mail",
            "bind",
            "--model",
            &model,
            "--spec",
            SPEC,
        ],
        vec![
            "durable-act",
            "--synthetic-input",
            "--domain",
            DOMAIN,
            "--scope",
            "wrong-scope",
            "bind",
            "--model",
            &model,
            "--spec",
            SPEC,
        ],
    ] {
        assert!(!f.run(&args).status.success());
    }
    let bound = f.act(&["bind", "--model", &model, "--spec", SPEC]);
    let id = bound["status"]["id"].as_str().unwrap();
    let mut db = Memory::open(&f.db()).unwrap();
    let mut restricted = capture(CANARY);
    restricted.scopes = ["restricted".into()].into();
    db.capture(restricted).unwrap();
    drop(db);
    for command in ["status", "cancel"] {
        let report = f.act(&[command, "--task", id]);
        assert_eq!(report["status"]["id"], id);
        for field in ["spec", "observation", "feedback"] {
            assert!(report[field].is_null());
        }
        assert!(!report.to_string().contains(CANARY));
    }
}
