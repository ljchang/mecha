//! Row 2e-4a's acceptance through the real binary: `mecha sessions
//! successes` derives the owner-verified successes from the stores that own
//! each act — a draft sent unchanged becomes a writing exemplar printed
//! verbatim, a `done` closure the owner reopened is withdrawn — and a
//! session recorded as a smoke test is hidden unless `--include-tests`
//! lifts it, the same admission `sessions appraise` counts under
//! (`docs/APPRAISAL-WIRING-DESIGN.md` 2e-4a, L2, R40). No network, no model.

use mecha_core::session::{Record, RunStats, SessionKind, SessionMeta};
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

const SESSION: &str = "20260920T090000-lakeside";
const BODY: &str = "Hi Sam,\n\nThursday at 2pm works for the Lakeside Institute visit.\n\nDana";

fn home() -> Root {
    let root = std::env::temp_dir().join(format!("mecha-successes-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    for dir in ["outbox", "closures", "sessions"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    std::fs::create_dir_all(root.join("work")).unwrap();
    // A session recorded as a smoke test, which completed.
    let records = [
        Record::Meta(SessionMeta {
            id: SESSION.into(),
            created_at: chrono::Utc::now(),
            provider: "scripted".into(),
            model: "scripted".into(),
            workspace: root.join("work"),
            title: None,
            kind: Some(SessionKind::Test),
        }),
        Record::Outcome(RunStats {
            stop_cause: Some(mecha_core::agent::StopCause::Completed),
            ..Default::default()
        }),
    ];
    std::fs::write(
        home.join("sessions").join(format!("{SESSION}.jsonl")),
        records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let args = json!({"to": "sam@example.edu", "subject": "Lakeside visit", "body_markdown": BODY});
    let item = |id: &str, sent: &str, args: Value| {
        json!({
            "id": id, "status": "sent", "tool": "mail_send", "kind": "message",
            "args_before": {"to": "sam@example.edu", "subject": "Lakeside visit", "body_markdown": BODY},
            "args": args, "summary": "a reply", "session_id": SESSION,
            "created_at": "2026-09-20T09:00:00Z", "resolved_at": sent,
        })
    };
    for (id, sent, args) in [
        ("ob-kept", "2026-09-20T10:00:00Z", args.clone()),
        (
            "ob-edited",
            "2026-09-20T11:00:00Z",
            json!({"to": "sam@example.edu", "subject": "Lakeside visit", "body_markdown": "Sam — 2pm Thursday. Dana"}),
        ),
    ] {
        std::fs::write(
            home.join("outbox").join(format!("{id}.json")),
            serde_json::to_string_pretty(&item(id, sent, args)).unwrap(),
        )
        .unwrap();
    }
    let transition = |id: &str, from: &str, to: &str, mv: &str, at: &str, undoes: Option<&str>| {
        json!({
            "kind": "transition", "id": id, "task": "task-northwind", "from": from, "to": to,
            "move": mv, "actor": "owner", "surface": "cli", "sessions": [SESSION], "at": at,
            "undoes": undoes,
        })
        .to_string()
    };
    std::fs::write(
        home.join("closures").join("closures.jsonl"),
        [
            transition(
                "cl-1",
                "next",
                "done",
                "close",
                "2026-09-21T12:00:00Z",
                None,
            ),
            transition(
                "cl-2",
                "done",
                "next",
                "reopen",
                "2026-09-22T12:00:00Z",
                Some("cl-1"),
            ),
        ]
        .join("\n"),
    )
    .unwrap();
    Root(root)
}

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> std::process::Output {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_OUTBOX_DIR")
            .env_remove("MECHA_QUESTIONS_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("mecha finished")
    .unwrap();
    assert!(
        out.status.success(),
        "mecha {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

async fn json_of(home: &Path, work: &Path, args: &[&str]) -> Value {
    let out = mecha(home, work, args).await;
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

#[tokio::test]
async fn the_success_set_through_the_binary() {
    let root = home();
    let (home, work) = (root.0.join("home"), root.0.join("work"));

    // The draft sent unchanged is the one exemplar, verbatim; the edited one
    // is no success; the reopened closure is withdrawn by its reopen.
    let set = json_of(
        &home,
        &work,
        &[
            "sessions",
            "successes",
            "--json",
            "--include-tests",
            "--exemplars",
        ],
    )
    .await;
    assert_eq!(set["standing"], 1, "{set:#}");
    assert_eq!(set["by_kind"]["sent_unchanged"], 1);
    assert_eq!(set["withdrawn"], 1);
    assert_eq!(set["withdrawals"][0]["pointer"], "closure:cl-1");
    assert_eq!(set["withdrawals"][0]["by"], "closure:cl-2");
    assert_eq!(set["exemplars"]["served"], false);
    let items = set["exemplars"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["item"], "ob-kept");
    assert_eq!(items[0]["args"]["body_markdown"], BODY);
    assert_eq!(items[0]["origin"], "clean");

    // Without the flag, the smoke-test session's successes are hidden.
    let hidden = json_of(&home, &work, &["sessions", "successes", "--json"]).await;
    assert_eq!(hidden["standing"], 0, "{hidden:#}");
    assert_eq!(hidden["hidden"], 2);

    // `sessions appraise` counts the same population it walks.
    let appraise = json_of(
        &home,
        &work,
        &["sessions", "appraise", "--json", "--include-tests"],
    )
    .await;
    assert_eq!(appraise["successes"]["standing"], 1, "{appraise:#}");
    let appraise = json_of(&home, &work, &["sessions", "appraise", "--json"]).await;
    assert_eq!(appraise["successes"]["hidden"], 2, "{appraise:#}");

    // The text readout prints the draft as it went out.
    let text = mecha(
        &home,
        &work,
        &[
            "sessions",
            "successes",
            "--include-tests",
            "--exemplars",
            "-n",
            "1",
        ],
    )
    .await;
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("owner-verified successes: 1 standing (sent_unchanged 1)"),
        "{text}"
    );
    assert!(
        text.contains("closure:cl-1 — taken back by closure:cl-2"),
        "{text}"
    );
    assert!(text.contains("Thursday at 2pm works"), "{text}");
    assert!(text.contains("served to no run"), "{text}");
}
