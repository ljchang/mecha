//! Row 2a-3's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` I1, R25): the counts-only appraiser is retired, so no second
//! model pass reads a session for its label. `mecha sessions appraise
//! --appraise` still runs — a script that passes it keeps working — but
//! the flag is a no-op that says so and points at the text appraisal, and
//! the model is never asked. On the old build the same command sent one
//! quarantined request per session.
//!
//! No network and no real model: the provider is a loopback fixture that
//! counts what it is asked. Fictional cast only.

use mecha_core::agent::Taint;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunStats, Session, SessionKind, SessionMeta};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A session with an outcome, so the readout has something to appraise.
fn session(home: &Path) {
    let session = Session::create(
        &home.join("sessions"),
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: home.to_path_buf(),
            title: None,
            kind: Some(SessionKind::Tui),
        },
    )
    .unwrap();
    for m in [
        Message::user("summarise the notes from Alex Kim"),
        Message::assistant(vec![Block::text("The notes say the review is Thursday.")]),
    ] {
        session.append(&Record::Message(m)).unwrap();
    }
    session
        .append(&Record::Outcome(RunStats {
            turns: 1,
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::Taint(Taint {
            untrusted: false,
            private: true,
        }))
        .unwrap();
}

/// The fixture model: answers as the old appraiser's model would have, and
/// counts every request.
async fn fixture_model() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |_body: String| {
            let counted = Arc::clone(&counted);
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                let text = r#"{"reasoning": "a fixture", "verdict": "strongly_negative", "agency": "other"}"#;
                let body = json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 4}});
                ([("content-type", "application/json")], body.to_string())
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), asked, server)
}

#[tokio::test]
async fn the_retired_appraise_flag_asks_no_model_and_says_where_the_appraisal_went() {
    let root =
        Root(std::env::temp_dir().join(format!("mecha-appraiser-retired-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let (base_url, asked, server) = fixture_model().await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"local\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n"
        ),
    )
    .unwrap();
    session(&home);
    session(&home);

    let out = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            // The exact form `scripts/appraisal-validity.py` used to pass.
            .args([
                "sessions",
                "appraise",
                "--json",
                "--include-tests",
                "--appraise",
                "--max-appraisals",
                "5",
            ])
            .env("MECHA_HOME", &home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(&work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("appraise finished")
    .unwrap();
    server.abort();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the flag still runs: {stderr}");
    assert_eq!(
        asked.load(Ordering::SeqCst),
        0,
        "no second model pass reads a session for its label"
    );
    assert!(stderr.contains("--appraise is retired"), "{stderr}");
    assert!(
        stderr.contains("mecha sessions appraise <session>"),
        "{stderr}"
    );
    let readout: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "--json stays parseable ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(readout["appraised"], 2, "{readout:#}");
    assert!(readout["appraiser"].is_null(), "{readout:#}");
    assert!(
        readout["channels"]
            .as_object()
            .is_none_or(|c| !c.contains_key("appraisal")),
        "nothing wrote the retired channel: {readout:#}"
    );
}
