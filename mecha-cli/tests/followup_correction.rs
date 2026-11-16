//! Corrections' follow-up join through the real binary: `mecha reflect`
//! over a web chat whose second message the fixture reflector judges a
//! correction writes the owner's verdict on the attempt kept for the turn
//! before it (`attempts::of_followup`), once; a skip judges nothing; and
//! with `[features] corrections` off the file is left as it was. No
//! network, no real model.

use mecha_core::agent::Taint;
use mecha_core::attempts;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ASK: &str = "when does the harbour office berth open";
const SAID: &str = "no, the winter berth, not the summer one";

/// A web chat: an ask answered with one lookup, then the owner's next
/// message. The attempt is kept as the web chat's turn keeps it.
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
    let mut result = Message::tool_results(vec![Block::ToolResult {
        tool_use_id: "look-1".into(),
        content: "summer berth: 08:00".into(),
        is_error: false,
    }]);
    result.tool_provenance.insert("look-1".into(), true);
    let messages = vec![
        Message::user(ASK),
        Message::assistant(vec![Block::ToolUse {
            id: "look-1".into(),
            name: "schedule_lookup".into(),
            input: json!({"what": "berth"}),
        }]),
        result,
        Message::assistant(vec![Block::text("The berth opens at 08:00.")]),
        Message::user(SAID),
        Message::assistant(vec![Block::text("The winter berth opens at 09:30.")]),
    ];
    for m in &messages {
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
    let id = session.meta.id.clone();
    let attempt = attempts::of_turn(&id, &messages, 0, "t").expect("the turn used a tool");
    attempts::Store::in_home(home).keep(&attempt, &[]).unwrap();
    id
}

/// A reflector that draws a lesson from every moment, or skips every one.
async fn fixture_model(lesson: Arc<AtomicBool>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::extract::Json<Value>| {
            let lesson = lesson.load(Ordering::SeqCst);
            async move {
                let reply = if lesson {
                    r#"{"skip": false, "reflexion": "Ask which berth is meant before looking one up.", "error_type": "missed-context", "confidence": 0.8, "fact": false}"#
                } else {
                    r#"{"skip": true}"#
                };
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

/// A home with one web chat and its kept attempt, and the fixture as its
/// provider.
fn home(root: &Root, name: &str, base_url: &str, corrections: bool) -> (PathBuf, PathBuf, String) {
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
             [features]\ncorrections = {corrections}\n"
        ),
    )
    .unwrap();
    let id = session(&home);
    (home, work, id)
}

#[tokio::test]
async fn a_follow_up_the_reflector_judges_a_correction_is_a_verdict_on_the_attempt() {
    // The fixture session is the owner's, not a smoke test: CI's exported
    // mark must not relabel it, or `reflect` would pass it over.
    mecha_core::session::ignore_kind_env_for_tests();
    let root = Root(std::env::temp_dir().join(format!("mecha-followup-{}", Session::new_id())));
    let lesson = Arc::new(AtomicBool::new(true));
    let (base_url, server) = fixture_model(Arc::clone(&lesson)).await;

    // On, and judged a correction: the attempt is judged against, with the
    // owner's own words and nothing a tool returned or a model said.
    let (on, work, id) = home(&root, "on", &base_url, true);
    let out = reflect(&on, &work).await;
    assert!(
        out.contains("corrections: 1 earlier attempt(s) judged"),
        "{out}"
    );
    let store = attempts::Store::in_home(&on);
    let kept = store.load().unwrap();
    assert_eq!(kept.verdicts.len(), 1, "{kept:?}");
    let v = &kept.verdicts[0];
    assert_eq!(v.session, id);
    assert_eq!(v.key, kept.attempts[0].key);
    assert!(!v.good && v.source == attempts::Source::Followup);
    assert_eq!(v.why.as_deref(), Some(SAID));
    assert_eq!(v.before, vec!["schedule_lookup"]);
    let wire = std::fs::read_to_string(store.path()).unwrap();
    assert!(
        !wire.contains("08:00") && !wire.contains("Ask which berth"),
        "{wire}"
    );
    let readout = kept.readout(5);
    assert_eq!((readout.judged_against, readout.unjudged), (1, 0));
    // The session is mined now: a second pass judges nothing again.
    reflect(&on, &work).await;
    assert_eq!(store.load().unwrap().verdicts.len(), 1);

    // On, and the reflector saw no correction: no verdict, never a good one.
    lesson.store(false, Ordering::SeqCst);
    let (skipped, work, _) = home(&root, "skipped", &base_url, true);
    let out = reflect(&skipped, &work).await;
    assert!(
        out.contains("corrections: 0 earlier attempt(s) judged"),
        "{out}"
    );
    let kept = attempts::Store::in_home(&skipped).load().unwrap();
    assert!(kept.verdicts.is_empty(), "{kept:?}");

    // Off: the lesson is still drawn, and the corrections file is as it was.
    lesson.store(true, Ordering::SeqCst);
    let (off, work, _) = home(&root, "off", &base_url, false);
    let out = reflect(&off, &work).await;
    server.abort();
    assert!(out.contains("1 reflection(s)"), "{out}");
    assert!(!out.contains("corrections:"), "{out}");
    let kept = attempts::Store::in_home(&off).load().unwrap();
    assert_eq!((kept.attempts.len(), kept.verdicts.len()), (1, 0));
}
