//! A conversation that goes on after `mecha reflect` has read it is read
//! again, for what is new only. Through the real binary against a loopback
//! fixture reflector that counts what it is asked: the first pass asks about
//! the one follow-up there is; after the chat grows by a turn the next pass
//! asks about the new follow-up alone, both passes' replies count, and the
//! correction is joined to the attempt before it; a pass with nothing added
//! asks nothing. A session mined before lengths were kept is given one
//! without a model call and is then read the same way. No network.

use mecha_core::agent::Taint;
use mecha_core::attempts;
use mecha_core::learning::LearningStore;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta};
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

/// One turn of the chat: the owner's words, a lookup that worked, an answer.
fn turn(said: &str, call: &str, answer: &str) -> Vec<Message> {
    let mut result = Message::tool_results(vec![Block::ToolResult {
        tool_use_id: call.into(),
        content: "08:00".into(),
        is_error: false,
    }]);
    result.tool_provenance.insert(call.into(), true);
    vec![
        Message::user(said),
        Message::assistant(vec![Block::ToolUse {
            id: call.into(),
            name: "schedule_lookup".into(),
            input: json!({"what": "berth"}),
        }]),
        result,
        Message::assistant(vec![Block::text(answer)]),
    ]
}

fn append(session: &Session, messages: &[Message]) {
    for m in messages {
        session.append(&Record::Message(m.clone())).unwrap();
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
}

/// A web chat of two turns, the second a follow-up to the first, with both
/// kept as attempts. Returned with every message so far, to grow it by.
fn chat(home: &Path) -> (Session, Vec<Message>) {
    let session = Session::create(
        &home.join("sessions"),
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: home.to_path_buf(),
            title: None,
            kind: Some(SessionKind::Web),
        },
    )
    .unwrap();
    session
        .append(&Record::Config(RunConfig {
            tools: vec!["schedule_lookup".into()],
            system_prompt: Some("You are mecha.".into()),
            rules_surface: Some(SessionKind::Web),
            max_tokens: 1024,
            max_turns: 8,
            ..Default::default()
        }))
        .unwrap();
    let mut messages = turn(
        "when does the harbour office berth open",
        "look-1",
        "The berth opens at 08:00.",
    );
    messages.extend(turn(
        "no, the winter berth, not the summer one",
        "look-2",
        "The winter berth opens at 08:00.",
    ));
    append(&session, &messages);
    let store = attempts::Store::in_home(home);
    for start in [0, 4] {
        let attempt = attempts::of_turn(&session.meta.id, &messages, start, "t").unwrap();
        store.keep(&attempt, &[]).unwrap();
    }
    (session, messages)
}

/// A reflector that draws a lesson from every moment and counts them.
async fn fixture_model(asked: Arc<AtomicUsize>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::extract::Json<Value>| {
            asked.fetch_add(1, Ordering::SeqCst);
            async move {
                let reply = r#"{"skip": false, "reflexion": "Ask which berth is meant before looking one up.", "error_type": "missed-context", "confidence": 0.8, "fact": false}"#;
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
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), server)
}

async fn reflect(home: &Path, work: &Path) -> String {
    reflect_with(home, work, &[]).await
}

async fn reflect_with(home: &Path, work: &Path, flags: &[&str]) -> String {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .arg("reflect")
            .args(flags)
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

fn home(root: &Root, name: &str, base_url: &str) -> (PathBuf, PathBuf) {
    let home = root.0.join(name).join("home");
    let work = root.0.join(name).join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"openai-compatible\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n\
             [features]\ncorrections = true\n"
        ),
    )
    .unwrap();
    (home, work)
}

fn third_turn() -> Vec<Message> {
    vec![
        Message::user("still wrong, I asked about the east quay"),
        Message::assistant(vec![Block::text(
            "The east quay's winter berth opens at 09:30.",
        )]),
    ]
}

#[tokio::test]
async fn a_chat_that_grew_is_read_again_for_what_is_new_only() {
    // The fixture sessions are the owner's, not smoke tests: CI's exported
    // mark must not relabel them, or `reflect` would pass them over.
    mecha_core::session::ignore_kind_env_for_tests();
    let root = Root(std::env::temp_dir().join(format!("mecha-grown-{}", Session::new_id())));
    let asked = Arc::new(AtomicUsize::new(0));
    let (base_url, server) = fixture_model(Arc::clone(&asked)).await;
    let count = || asked.load(Ordering::SeqCst);

    // The first pass: the one follow-up there is, judged on the first turn.
    let (grown, work) = home(&root, "grown", &base_url);
    let (session, _) = chat(&grown);
    reflect(&grown, &work).await;
    assert_eq!(count(), 1);
    let attempts = attempts::Store::in_home(&grown);
    let kept = attempts.load().unwrap();
    assert_eq!(kept.verdicts.len(), 1, "{kept:?}");
    assert_eq!(kept.verdicts[0].key, kept.attempts[0].key);
    // Nothing added: the next pass asks nothing.
    reflect(&grown, &work).await;
    assert_eq!(count(), 1);
    // Grown by a turn with nothing in it to ask about: the pass says there
    // is nothing to mine, as one that needs no provider does, and the
    // session is not read again for it.
    append(
        &session,
        &[Message::assistant(vec![Block::text("A note to self.")])],
    );
    let out = reflect(&grown, &work).await;
    assert!(out.contains("nothing to mine"), "{out}");
    assert_eq!(count(), 1);

    // The chat goes on. The next pass asks about the new message alone,
    // and it is the second turn's attempt that it judges.
    append(&session, &third_turn());
    let out = reflect(&grown, &work).await;
    assert_eq!(count(), 2, "{out}");
    assert!(
        out.contains("corrections: 1 earlier attempt(s) judged"),
        "{out}"
    );
    let kept = attempts.load().unwrap();
    assert_eq!(kept.verdicts.len(), 2, "{kept:?}");
    assert_eq!(kept.verdicts[1].key, kept.attempts[1].key);
    assert_eq!(
        kept.verdicts[1].why.as_deref(),
        Some("still wrong, I asked about the east quay")
    );
    // Both passes' replies count, and each moment's lesson is on file once.
    let learning = LearningStore::open(grown.join("learning")).unwrap();
    let (rows, unreadable) = learning.replies_counting().unwrap();
    assert_eq!((rows.len(), unreadable), (2, 0), "{rows:#?}");
    assert_ne!(rows[0].pass, rows[1].pass);
    assert_eq!(learning.reflexions().unwrap().len(), 2);
    let mined = std::fs::read_to_string(grown.join("learning").join("mined.jsonl")).unwrap();
    assert_eq!(mined.lines().count(), 1, "{mined}");
    // The record of the passes holds hashes and the session, never the words.
    let moments =
        std::fs::read_to_string(grown.join("learning").join("mined_passes.jsonl")).unwrap();
    assert!(
        !moments.contains("berth") && !moments.contains("quay"),
        "{moments}"
    );
    reflect(&grown, &work).await;
    assert_eq!(count(), 2);

    // A session an older build mined: marked, with no length on record.
    // The first pass asks nothing about it and records where it ends; what
    // is added afterwards is asked about, and nothing from before.
    let (old, work) = home(&root, "old", &base_url);
    let (session, _) = chat(&old);
    std::fs::create_dir_all(old.join("learning")).unwrap();
    std::fs::write(
        old.join("learning").join("mined.jsonl"),
        format!("{}\n", session.meta.id),
    )
    .unwrap();
    // A dry run writes nothing: a line made here would mark every moment
    // of every such session asked, with no model call and no way back.
    reflect_with(&old, &work, &["--dry-run"]).await;
    assert!(!old.join("learning").join("mined_passes.jsonl").exists());
    let out = reflect(&old, &work).await;
    assert_eq!(count(), 2, "{out}");
    assert!(
        out.contains("recorded where 1 already-mined session(s) end"),
        "{out}"
    );
    append(&session, &third_turn());
    reflect(&old, &work).await;
    server.abort();
    assert_eq!(count(), 3);
    let kept = attempts::Store::in_home(&old).load().unwrap();
    assert_eq!(kept.verdicts.len(), 1, "{kept:?}");
    assert_eq!(kept.verdicts[0].key, kept.attempts[1].key);
}
