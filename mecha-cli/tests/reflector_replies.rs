//! P7 through the real binary (`docs/HARNESS-EVOLUTION-DESIGN.md`): `mecha
//! reflect` over a clean session holding one steer, against a loopback
//! fixture reflector that skips, records the skip in
//! `reflector_replies.jsonl` (once, beside the session's mined mark) with
//! the prompt's hash and no text, and a second pass records nothing more.
//! No network, no real model.

use mecha_core::agent::Taint;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta};
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

const STEER: &str = "only the third quarter, please";

fn session(home: &Path) -> String {
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
    session
        .append(&Record::Config(RunConfig {
            tools: vec!["fs_list".into()],
            system_prompt: Some("You are mecha.".into()),
            rules_surface: Some(SessionKind::Tui),
            max_tokens: 1024,
            max_turns: 8,
            ..Default::default()
        }))
        .unwrap();
    let mut steer = Message::tool_results(vec![Block::ToolResult {
        tool_use_id: "t1".into(),
        content: "q3.csv q4.csv".into(),
        is_error: false,
    }]);
    steer.content.push(Block::text(STEER));
    for m in [
        Message::user("total the quarters"),
        Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "fs_list".into(),
            input: json!({}),
        }]),
        steer,
        Message::assistant(vec![Block::text("The third quarter totals 12.")]),
    ] {
        session.append(&Record::Message(m)).unwrap();
    }
    session
        .append(&Record::Outcome(RunStats {
            turns: 2,
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::Taint(Taint {
            untrusted: false,
            private: true,
        }))
        .unwrap();
    session.meta.id.clone()
}

/// A reflector that judges every moment lesson-free.
async fn fixture_model() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::extract::Json<Value>| async move {
            let reply = r#"{"skip": true}"#;
            let usage = json!({"prompt_tokens": 10, "completion_tokens": 4});
            if body.get("stream").and_then(Value::as_bool) == Some(true) {
                let chunk = json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": reply}, "finish_reason": null}]});
                let done = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": usage});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                )
            } else {
                let body = json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": reply}, "finish_reason": "stop"}], "usage": usage});
                ([("content-type", "application/json")], body.to_string())
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), server)
}

async fn reflect(home: &Path, work: &Path) -> String {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["reflect"])
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("MECHA_OUTBOX_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("reflect finished")
    .unwrap();
    assert!(
        out.status.success(),
        "reflect failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn rows(home: &Path) -> Vec<Value> {
    std::fs::read_to_string(home.join("learning").join("reflector_replies.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn a_skip_is_recorded_once_with_its_prompt_and_no_text() {
    // The fixture session is the owner's, not a smoke test: CI's exported
    // mark must not relabel it, or `reflect` would pass it over.
    mecha_core::session::ignore_kind_env_for_tests();
    let root = Root(std::env::temp_dir().join(format!("mecha-replies-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let (base_url, server) = fixture_model().await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"openai-compatible\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n"
        ),
    )
    .unwrap();
    let id = session(&home);

    let out = reflect(&home, &work).await;
    assert!(
        out.contains("reflector replies: 0 lesson(s), 1 skip(s), 0 unusable"),
        "{out}"
    );
    let first = rows(&home);
    assert_eq!(first.len(), 1, "{first:#?}");
    let r = &first[0];
    assert_eq!(r["outcome"], "skip");
    assert_eq!(r["trigger"], "steer");
    assert_eq!(r["domain"], "behavior");
    assert_eq!(r["session_id"], id.as_str());
    assert_eq!(r["model"], "fixture");
    // A clean session's reflector saw the full excerpts, and the prompt
    // tells it to prefer a skip when they are withheld: the class rides.
    assert_eq!(r["evidence"], "full");
    assert!(r["prompt"].as_str().is_some_and(|p| !p.is_empty()), "{r}");
    // The pass that wrote it: a retry's rows supersede a stopped pass's.
    assert!(r["pass"].as_str().is_some_and(|p| !p.is_empty()), "{r}");
    let wire =
        std::fs::read_to_string(home.join("learning").join("reflector_replies.jsonl")).unwrap();
    assert!(
        !wire.contains("third quarter"),
        "the steer's words reached the ledger"
    );

    // The session is mined now: a second pass asks nothing and adds no row.
    reflect(&home, &work).await;
    server.abort();
    assert_eq!(rows(&home).len(), 1);
}
