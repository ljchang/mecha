use serde_json::Value;
use std::{
    path::PathBuf,
    process::{Command, Output},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("memory-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mecha-memory-prototype"))
        .args(args)
        .output()
        .unwrap()
}
fn json(args: &[&str]) -> Value {
    let out = run(args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn demo_queries_history_then_forgets_without_touching_other_files() {
    let temp = Temp::new();
    let p = temp.0.join("demo.memory-prototype.sqlite");
    let path = p.to_str().unwrap();
    let neighbor = temp.0.join("graph.db");
    std::fs::write(&neighbor, b"live graph stand-in").unwrap();
    let demo = json(&["demo", path]);
    let before = demo["snapshots"]["before_departure"]["sequence"].to_string();
    let current = json(&["query", path, "works", "latest", "2026-07-01T00:00:00Z"]);
    assert_eq!(current.as_array().unwrap().len(), 0);
    let historic = json(&["query", path, "works", &before, "2026-07-01T00:00:00Z"]);
    assert_eq!(historic.as_array().unwrap().len(), 1);
    let during = json(&["query", path, "works", "latest", "2026-05-15T00:00:00Z"]);
    assert_eq!(during.as_array().unwrap().len(), 1);
    assert!(!run(&["demo", path]).status.success());
    let forgotten = json(&["forget", path, "employment"]);
    assert!(forgotten["revisions"].as_u64().unwrap() >= 3);
    assert_eq!(
        json(&["query", path, "works", &before, "2026-07-01T00:00:00Z"])
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(std::fs::read(neighbor).unwrap(), b"live graph stand-in");
}
#[test]
fn cli_refuses_foreign_files_and_missing_or_extra_arguments() {
    let temp = Temp::new();
    let p = temp.0.join("foreign.memory-prototype.sqlite");
    let path = p.to_str().unwrap();
    std::fs::write(&p, b"unrelated").unwrap();
    assert!(!run(&["inspect", path]).status.success());
    assert_eq!(std::fs::read(&p).unwrap(), b"unrelated");
    assert!(!run(&["demo"]).status.success());
    assert!(!run(&["demo", path, "ignored"]).status.success());
}
