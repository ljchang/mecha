//! Actual-binary boundary checks; synthetic callbacks never contact a model.
#![cfg(feature = "memory-prototype")]

use mecha_core::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{durable::TaskSpec, model_evaluation::ModelReply, Access, Capture, Memory, SourceKey},
    memory_decision_runtime, memory_runtime,
    tool::{ModeApprover, ToolCtx},
};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Command, Output},
    sync::Arc,
};

const MODEL: &str = "synthetic-cli-model";
const CANARY: &str = "SYNTHETIC_CLI_MODEL_PRIVATE_CANARY";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("memory-model-cli-{}", uuid::Uuid::new_v4()));
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
    fn model(&self, args: &[&str]) -> Value {
        let mut argv = vec![
            "durable-decide",
            "--synthetic-input",
            "--domain",
            "synthetic-cli-qualification",
            "--scope",
            "fixture-private",
        ];
        argv.extend_from_slice(args);
        self.json(&argv)
    }
    async fn read(&self) -> String {
        // Init is exercised through the binary; all following input is synthetic.
        self.json(&["init-model", "--domain", &access().domain]);
        let mut db = Memory::open(&self.db()).unwrap();
        let input = db.capture(capture(CANARY)).unwrap();
        let work = self.0.join("work");
        std::fs::write(work.join("observations.json"), json!({
            "kind":"host_authored_synthetic_measurements_v1", "workload":"Synthetic CLI qualification", "complete":true,
            "observations":[
                {"choice":"Nori","observed_minutes":4,"energy":9,"quality":8},
                {"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},
                {"choice":"Quill","observed_minutes":6,"energy":5,"quality":7}]
        }).to_string()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: work.clone(),
                    path: "observations.json".into(),
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
        read.id().into()
    }
    fn bind(&self, read: &str) -> String {
        let bound = self.model(&[
            "bind",
            "--read",
            read,
            "--contract",
            &contract().to_string(),
            "--model",
            MODEL,
            "--view",
            "observed",
            "--seed",
            "7",
        ]);
        assert_eq!(bound["fixture_only"], true);
        assert_eq!(bound["status"]["initial"]["phase"], "unused");
        assert_eq!(bound["status"]["retry"]["phase"], "unused");
        bound["status"]["id"].as_str().unwrap().into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn access() -> Access {
    Access {
        domain: "synthetic-cli-qualification".into(),
        scopes: ["fixture-private".into()].into(),
    }
}
fn source() -> SourceKey {
    SourceKey {
        connector: "synthetic".into(),
        account: "cli-qualification".into(),
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
fn contract() -> Value {
    json!({"workload":"Synthetic CLI qualification","objective":"minimize_minutes","energy_budget":10,"minimum_quality":6})
}

#[tokio::test]
async fn bind_and_status_need_no_provider_or_valid_local_configuration() {
    let f = Fixture::new();
    let read = f.read().await;
    let id = f.bind(&read);
    let status = f.model(&["status", "--task", &id]);
    assert_eq!(status["spec"]["model"], MODEL);
    assert_eq!(status["spec"]["contract"], contract());
    assert!(status["initial"].is_null());
    assert!(status["decision"].is_null());
}

#[tokio::test]
async fn terminal_reuse_never_contacts_endpoint_or_requires_holds_directory() {
    let f = Fixture::new();
    let read = f.read().await;
    let id = f.bind(&read);
    let mut db = Memory::open(&f.db()).unwrap();
    let receipt = db.durable_model_reopen(&id, &access()).unwrap();
    memory_decision_runtime::decide(&mut db, &receipt, &access(), |_, _, guard| async move {
        guard.check()?;
        Ok(ModelReply {
            text: json!({"choice":"Nori","citations":["e1","e2","e3"],"reason":CANARY}).to_string(),
            model: MODEL.into(),
            input_tokens: Some(11),
            output_tokens: Some(7),
            error: None,
        })
    })
    .await
    .unwrap();
    let original = db
        .durable_model_decision(&receipt, &access())
        .unwrap()
        .unwrap();
    drop(db);
    std::fs::remove_file(f.0.join("work/observations.json")).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let missing_holds = f.0.join("does-not-exist");
    let reused = f.model(&[
        "run",
        "--task",
        &id,
        "--endpoint",
        &endpoint,
        "--holds-dir",
        missing_holds.to_str().unwrap(),
    ]);
    assert_eq!(reused["decision"], original);
    assert_eq!(reused["decision"]["executed"], false);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let reused = f.model(&[
        "run",
        "--task",
        &id,
        "--endpoint",
        "not-a-valid-provider-url",
        "--holds-dir",
        missing_holds.to_str().unwrap(),
    ]);
    assert_eq!(reused["decision"], original);
}

#[tokio::test]
async fn stale_sources_still_allow_content_free_status_and_cancellation() {
    for invalidate in ["change", "restrict", "forget"] {
        let f = Fixture::new();
        let read = f.read().await;
        let id = f.bind(&read);
        let mut db = Memory::open(&f.db()).unwrap();
        match invalidate {
            "change" => {
                db.capture(capture("Synthetic corrected prerequisite"))
                    .unwrap();
            }
            "restrict" => {
                let mut c = capture(CANARY);
                c.scopes = ["restricted-scope".into()].into();
                db.capture(c).unwrap();
            }
            "forget" => {
                db.forget(&source()).unwrap();
            }
            _ => unreachable!(),
        }
        drop(db);
        for command in if invalidate == "forget" {
            vec!["status"]
        } else {
            vec!["status", "cancel"]
        } {
            let report = f.model(&[command, "--task", &id]);
            assert_eq!(report["status"]["id"], id);
            assert!(report["spec"].is_null());
            assert!(report["initial"].is_null());
            assert!(report["retry"].is_null());
            assert!(report["decision"].is_null());
            assert!(!report.to_string().contains(CANARY));
            if command == "cancel" {
                assert_eq!(report["status"]["phase"], "cancelled");
            }
            if invalidate == "forget" {
                assert_eq!(report["status"]["phase"], "forgotten");
            }
        }
    }
}

#[tokio::test]
async fn cli_requires_synthetic_current_access_and_explicit_binding_fields() {
    let f = Fixture::new();
    let read = f.read().await;
    let contract = contract().to_string();
    let common = [
        "durable-decide",
        "--synthetic-input",
        "--domain",
        "synthetic-cli-qualification",
        "--scope",
        "fixture-private",
    ];
    for omitted in ["--synthetic-input", "--domain", "--scope"] {
        let mut prefix = Vec::new();
        let mut i = 0;
        while i < common.len() {
            if common[i] == omitted {
                i += if omitted == "--synthetic-input" { 1 } else { 2 };
            } else {
                prefix.push(common[i]);
                i += 1;
            }
        }
        prefix.extend_from_slice(&[
            "bind",
            "--read",
            &read,
            "--contract",
            &contract,
            "--model",
            MODEL,
            "--view",
            "observed",
        ]);
        assert!(!f.run(&prefix).status.success(), "missing {omitted}");
    }
    for omitted in ["--read", "--contract", "--model", "--view"] {
        let bind = [
            "bind",
            "--read",
            &read,
            "--contract",
            &contract,
            "--model",
            MODEL,
            "--view",
            "observed",
        ];
        let mut args = common.to_vec();
        let mut i = 0;
        while i < bind.len() {
            if bind[i] == omitted {
                i += 2;
            } else {
                args.push(bind[i]);
                i += 1;
            }
        }
        assert!(!f.run(&args).status.success(), "missing {omitted}");
    }
    for domain in ["wrong-domain", "synthetic-cli-qualification"] {
        let scope = if domain == "wrong-domain" {
            "fixture-private"
        } else {
            "wrong-scope"
        };
        assert!(!f
            .run(&[
                "durable-decide",
                "--synthetic-input",
                "--domain",
                domain,
                "--scope",
                scope,
                "bind",
                "--read",
                &read,
                "--contract",
                &contract,
                "--model",
                MODEL,
                "--view",
                "observed"
            ])
            .status
            .success());
    }
}
