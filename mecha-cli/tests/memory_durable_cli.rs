//! Real binary restart qualification, isolated synthetic files and invalid configs.
#![cfg(feature = "memory-prototype")]
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Output},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("memory-durable-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::write(root.join("home/config.toml"), "not = [valid toml").unwrap();
        std::fs::write(root.join("work/mecha.toml"), "not = [valid toml").unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args([
                "memory",
                "--experimental",
                "--db",
                self.0
                    .join("test.memory-prototype.sqlite")
                    .to_str()
                    .unwrap(),
            ])
            .args(args)
            .current_dir(self.0.join("work"))
            .env("MECHA_HOME", self.0.join("home"))
            .env("MECHA_SESSION_KIND", "test")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .output()
            .unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn durable(&self, args: &[&str]) -> Value {
        let mut argv = vec![
            "durable-read",
            "--synthetic-input",
            "--domain",
            "synthetic-demo",
            "--scope",
            "fixture-mail",
        ];
        argv.extend_from_slice(args);
        self.json(&argv)
    }
    fn setup(&self) -> String {
        self.json(&["init-durable", "--domain", "synthetic-demo"]);
        let source = self.0.join("synthetic.jsonl");
        std::fs::write(&source, concat!("{\"record\":\"meta\",\"id\":\"durable-synthetic\",\"created_at\":\"2026-01-01T00:00:00Z\"}\n", "{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Synthetic prerequisite canary\"}]}\n")).unwrap();
        self.json(&[
            "import-session",
            source.to_str().unwrap(),
            "--account",
            "synthetic",
            "--synthetic-input",
        ]);
        let hits = self.json(&["recall", "prerequisite"]);
        let r = &hits[0]["anchor"]["reference"];
        let reference = format!(
            "{}:{}",
            r["object"].as_str().unwrap(),
            r["revision"].as_str().unwrap()
        );
        std::fs::write(
            self.0.join("work/fixture.txt"),
            "CLI_ORIGINAL_SYNTHETIC_CANARY\n",
        )
        .unwrap();
        self.durable(&[
            "start",
            "--workspace",
            self.0.join("work").to_str().unwrap(),
            "--file",
            "fixture.txt",
            "--source",
            &reference,
        ])["status"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn cli_restarts_reuse_bytes_and_cancel_prevents_proposal_commit() {
    let f = Fixture::new();
    let id = f.setup();
    let workspace = f.0.join("work");
    let first = f.durable(&[
        "resume",
        "--task",
        &id,
        "--workspace",
        workspace.to_str().unwrap(),
    ]);
    assert!(first["observation"]
        .to_string()
        .contains("CLI_ORIGINAL_SYNTHETIC_CANARY"));
    assert_eq!(first["model_called"], false);
    std::fs::write(
        workspace.join("fixture.txt"),
        "CLI_REPLACEMENT_SYNTHETIC_CANARY",
    )
    .unwrap();
    let resumed = f.durable(&[
        "resume",
        "--task",
        &id,
        "--workspace",
        workspace.to_str().unwrap(),
    ]);
    assert_eq!(first["observation"], resumed["observation"]);
    assert_eq!(resumed["taint"]["private"], true);
    std::fs::remove_file(workspace.join("fixture.txt")).unwrap();
    assert_eq!(
        f.durable(&[
            "resume",
            "--task",
            &id,
            "--workspace",
            workspace.to_str().unwrap()
        ])["observation"],
        first["observation"]
    );
    f.durable(&["cancel", "--task", &id]);
    let refused = f.run(&[
        "durable-read",
        "--synthetic-input",
        "--domain",
        "synthetic-demo",
        "--scope",
        "fixture-mail",
        "finish",
        "--task",
        &id,
        "--proposal",
        "{\"status\":\"successful\"}",
    ]);
    assert!(!refused.status.success());
    assert!(f.durable(&["status", "--task", &id])["status"]["decision"].is_null());
}
#[test]
fn cli_requires_current_authority_and_synthetic_acknowledgement() {
    let f = Fixture::new();
    let id = f.setup();
    for prefix in [
        vec![
            "durable-read",
            "--domain",
            "synthetic-demo",
            "--scope",
            "fixture-mail",
        ],
        vec![
            "durable-read",
            "--synthetic-input",
            "--domain",
            "wrong",
            "--scope",
            "fixture-mail",
        ],
        vec![
            "durable-read",
            "--synthetic-input",
            "--domain",
            "synthetic-demo",
            "--scope",
            "wrong",
        ],
    ] {
        let mut args = prefix;
        args.extend(["status", "--task", &id]);
        assert!(!f.run(&args).status.success());
    }
    let wrong = f.0.join("home");
    let out = f.run(&[
        "durable-read",
        "--synthetic-input",
        "--domain",
        "synthetic-demo",
        "--scope",
        "fixture-mail",
        "resume",
        "--task",
        &id,
        "--workspace",
        wrong.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert_eq!(
        f.durable(&["status", "--task", &id])["status"]["acquisition_spent"],
        false
    );
}
