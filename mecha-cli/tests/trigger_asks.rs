//! A scheduled run may ask (`docs/ASSISTANT-FOLLOW-THROUGH-DESIGN.md`
//! §Unattended runs ask, A1), through the real binary: `mecha trigger run`
//! over a loopback fixture model whose first answer calls `ask_user`. The
//! question is parked on the trigger's own session to resume as a chat
//! (A2), the run ends there rather than asking the model again, the ledger
//! row says the run is waiting on the owner with the question's id, the
//! trigger's `notify` says a question is waiting rather than delivering the
//! run's last words as its answer, and the terminal refuses to answer it —
//! without consuming it — because its answer belongs in the web app.
//!
//! No network and no real model; fictional cast only.

use mecha_core::questions::{QuestionStore, Resume};
use mecha_core::session::Session;
use mecha_core::trigger::{RunStatus, TriggerStore};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const QUESTION: &str = "Should the summary go to the whole lab, or only to Morgan Reyes?";

/// The fixture model: the first request is answered with an `ask_user`
/// call, every later one with prose. Every request body is kept.
async fn asking_model() -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = Arc::clone(&seen);
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: String| {
            let kept = Arc::clone(&kept);
            async move {
                let first = {
                    let mut kept = kept.lock().unwrap();
                    kept.push(body.clone());
                    kept.len() == 1
                };
                let parsed: Value = serde_json::from_str(&body).unwrap_or_default();
                let stream = parsed.get("stream").and_then(Value::as_bool) == Some(true);
                let usage = json!({"prompt_tokens": 10, "completion_tokens": 4});
                let arguments = json!({ "question": QUESTION }).to_string();
                if first {
                    if stream {
                        let call = json!({"choices": [{"index": 0, "delta": {"role": "assistant",
                            "tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                            "function": {"name": "ask_user", "arguments": arguments}}]},
                            "finish_reason": null}]});
                        let done = json!({"choices": [{"index": 0, "delta": {},
                            "finish_reason": "tool_calls"}], "usage": usage});
                        (
                            [("content-type", "text/event-stream")],
                            format!("data: {call}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                        )
                    } else {
                        let body = json!({"choices": [{"index": 0, "message": {"role": "assistant",
                            "content": null, "tool_calls": [{"id": "call_1", "type": "function",
                            "function": {"name": "ask_user", "arguments": arguments}}]},
                            "finish_reason": "tool_calls"}], "usage": usage});
                        ([("content-type", "application/json")], body.to_string())
                    }
                } else {
                    let text = "I went on without an answer.";
                    if stream {
                        let chunk = json!({"choices": [{"index": 0, "delta": {"role": "assistant",
                            "content": text}, "finish_reason": null}]});
                        let done = json!({"choices": [{"index": 0, "delta": {},
                            "finish_reason": "stop"}], "usage": usage});
                        (
                            [("content-type", "text/event-stream")],
                            format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                        )
                    } else {
                        let body = json!({"choices": [{"index": 0, "message": {"role": "assistant",
                            "content": text}, "finish_reason": "stop"}], "usage": usage});
                        ([("content-type", "application/json")], body.to_string())
                    }
                }
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen, server)
}

fn seed(root: &Path, base_url: &str) -> (PathBuf, PathBuf) {
    let home = root.join("home");
    let workspace = root.join("work").join("morning");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(home.join("triggers")).unwrap();
    std::fs::write(
        home.join("triggers/morning.toml"),
        format!(
            "schedule = \"0 7 * * *\"\nprompt = \"Summarise the week for the lab.\"\n\
             workspace = {:?}\nnotify = \"cat > notified.txt\"\n",
            workspace.display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"local\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n\
             [agent]\ntimezone = \"America/New_York\"\n\
             [tools]\nenabled = [\"fs_read\"]\n\
             [sandbox]\nkind = \"none\"\n"
        ),
    )
    .unwrap();
    (home, workspace)
}

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(90),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_QUESTIONS_DIR")
            .env_remove("MECHA_TRIGGERS_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("MECHA_COMPARISONS_DIR")
            .env_remove("MECHA_FIXTURE_CLOCK")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap_or_else(|_| panic!("mecha {args:?} hung"))
    .unwrap()
}

#[tokio::test]
async fn a_trigger_that_asks_parks_a_chat_question_and_waits_on_the_owner() {
    let root = Root(std::env::temp_dir().join(format!("mecha-trigger-asks-{}", Session::new_id())));
    let (base_url, seen, server) = asking_model().await;
    let (home, workspace) = seed(&root.0, &base_url);
    let out = mecha(&home, &root.0.join("work"), &["trigger", "run", "morning"]).await;
    server.abort();
    assert!(
        out.status.success(),
        "trigger run failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "the run asked the model again after parking its question"
    );

    // The ledger: waiting on the owner, with the question's id.
    let ledger = TriggerStore::open(home.join("triggers"))
        .unwrap()
        .runs()
        .unwrap();
    assert_eq!(ledger.len(), 1, "{ledger:?}");
    let row = &ledger[0];
    assert_eq!(row.status, RunStatus::Waiting, "{row:?}");
    let qid = row.question_id.clone().expect("the row names its question");
    let session = row.session_id.clone().expect("the row names its session");

    // The question: open, on the trigger's own session, resuming as a chat.
    let store = QuestionStore::open(home.join("questions")).unwrap();
    let q = store.get(&qid).unwrap();
    assert!(q.is_open());
    assert_eq!(q.session_id, session, "parked on the run's own session");
    assert_eq!(q.resume, Resume::Chat);
    assert!(
        q.question.contains("only to Morgan Reyes"),
        "{}",
        q.question
    );

    // The delivery says a question is waiting, not that the run finished.
    let notified = std::fs::read_to_string(workspace.join("notified.txt")).unwrap();
    assert!(notified.contains("has a question for you"), "{notified}");
    assert!(notified.contains("Tasks → Waiting"), "{notified}");

    // The terminal will not answer it, and consumes nothing trying.
    let refused = mecha(
        &home,
        &root.0.join("work"),
        &["questions", "answer", &qid, "only", "to", "Morgan"],
    )
    .await;
    assert!(!refused.status.success());
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("answer it in the web app"), "{said}");
    let after = store.get(&qid).unwrap();
    assert!(after.is_open() && after.answer.is_none(), "{after:?}");
}
