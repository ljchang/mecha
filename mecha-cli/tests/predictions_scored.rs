//! Row 2b-1's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` X5): over a fixture outbox holding resolved and unresolved
//! predictions, `mecha sessions appraise --json` reports coverage per kind
//! and no rate over nothing — and a store with no outcomes at all reports
//! coverage alone. No model, no network. Fictional cast only.

use mecha_core::agent::Taint;
use mecha_core::anticipation::{Evidence, OutcomeInput, Verdict, Verification};
use mecha_core::outbox::{OutboxKind, OutboxStore, Provenance};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn appraise(home: &Path, work: &Path) -> Value {
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["sessions", "appraise", "--json", "--include-tests"])
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("appraise finished")
    .unwrap();
    assert!(
        out.status.success(),
        "appraise failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn stage(store: &OutboxStore, n: u32) -> String {
    store
        .stage(
            "mail_send",
            OutboxKind::Message,
            json!({"to": "morgan.reyes@example.org", "body": format!("The agenda, part {n}.")}),
            Taint::default(),
            Provenance {
                anticipation: None,
                filled_defaults: Vec::new(),
                session_id: None,
                workspace: None,
                call_id: None,
            },
        )
        .unwrap()
        .id
}

#[tokio::test]
async fn the_readout_reports_prediction_coverage_and_no_rate_over_nothing() {
    let root = Root(std::env::temp_dir().join(format!(
        "mecha-predictions-scored-{}",
        mecha_core::session::Session::new_id()
    )));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let store = OutboxStore::open(home.join("outbox")).unwrap();
    let passed = Evidence {
        verification: Verification::Passed,
        verification_evidence: Some("the agenda matches the calendar".into()),
        ..Evidence::default()
    };

    // Predictions, no outcomes: coverage, and every rate null.
    let waiting = stage(&store, 1);
    store.anticipate(&waiting, passed.clone(), false).unwrap();
    let before = appraise(&home, &work).await;
    let p = &before["predictions"];
    assert_eq!(p["read"], true, "{p:#}");
    assert_eq!(p["total"]["predictions"], 2, "{p:#}");
    assert_eq!(p["total"]["scored"], 0);
    assert!(p["total"]["materialized_rate"].is_null(), "{p:#}");
    for (_, c) in p["by_response"].as_object().unwrap() {
        assert!(
            c["materialized_rate"].is_null(),
            "no rate over nothing: {p:#}"
        );
    }

    // One draft released, delivered, and judged clean by the owner.
    let sent = stage(&store, 2);
    let item = store.anticipate(&sent, passed, false).unwrap();
    let prediction = item.predictions.last().unwrap().known().unwrap().id.clone();
    store.begin_delivery(&sent).unwrap();
    store
        .resolve_with_output(&sent, "sent", None, Some("sent: msg-2".into()))
        .unwrap();
    store
        .record_outcome(
            &sent,
            OutcomeInput {
                prediction_id: prediction,
                verdict: Verdict::NoIssue,
                evidence: "Morgan Reyes confirmed the agenda".into(),
                attributable_to_mecha: false,
                supersedes: None,
            },
        )
        .unwrap();

    let after = appraise(&home, &work).await;
    let p = &after["predictions"];
    let proceed = &p["by_response"]["proceed"];
    assert_eq!(proceed["predictions"], 2, "{p:#}");
    assert_eq!(proceed["scored"], 1);
    assert_eq!(proceed["clean"], 1);
    assert_eq!(proceed["materialized_rate"], 0.0);
    assert_eq!(proceed["unscored"]["pending"], 1);
    let clarify = &p["by_response"]["clarify"];
    assert_eq!(clarify["unscored"]["reassessed"], 2, "{p:#}");
    assert!(clarify["materialized_rate"].is_null());
    assert!(p["by_response"]["replan"]["materialized_rate"].is_null());
}
