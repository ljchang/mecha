use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_guard_coverage"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The bounded reader may close the pipe once it detects oversize input.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    child.wait_with_output().unwrap()
}

#[test]
fn frozen_cases_distinguish_coverage_from_unreachable_and_invalid_plans() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("../guard_cases.json")).unwrap();
    for case in cases {
        let out = run(&case["input"].to_string());
        if case["expected"]["status"] == "invalid" {
            assert!(!out.status.success(), "{}", case["id"]);
            assert!(String::from_utf8_lossy(&out.stderr).contains("unknown action reference"));
        } else {
            assert!(
                out.status.success(),
                "{}: {}",
                case["id"],
                String::from_utf8_lossy(&out.stderr)
            );
            let r: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(r["status"], case["expected"]["status"], "{}", case["id"]);
            let statuses: Vec<_> = r["occurrences"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x["status"].clone())
                .collect();
            assert_eq!(
                json!(statuses),
                case["expected"]["occurrences"],
                "{}",
                case["id"]
            );
        }
    }
}

#[test]
fn adapter_refuses_oversize_and_unrecognized_fields_without_a_report() {
    for input in [
        " ".repeat(65_537),
        json!({"contract":{"conditions":[],"actions":[]},"plan":{"steps":[],"allow":true}})
            .to_string(),
    ] {
        let out = run(&input);
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }
}
