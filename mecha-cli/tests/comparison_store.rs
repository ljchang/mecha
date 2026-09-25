//! Row 1g's acceptance through the real binary: `mecha sessions appraise
//! --probe` over a clean steered session leaves a counterfactual comparison
//! that a second read returns, and over a tainted one leaves none
//! (`docs/APPRAISAL-WIRING-DESIGN.md` X1/O4). The model is a fixture
//! OpenAI-compatible server on a loopback port — no network, no real model.

use mecha_core::agent::Taint;
use mecha_core::comparison::{ComparisonStore, Kind, Role, Validator, Verdict};
use mecha_core::message::{Block, Message, ToolSpec};
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

fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "fs_list".into(),
            description: "List files.".into(),
            input_schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "fs_read".into(),
            description: "Read a file.".into(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        },
    ]
}

/// A steered session as `chat` records one, with its outcome so the
/// appraisal has something to appraise. Returns the session id.
fn steered_session(home: &Path, untrusted: bool) -> String {
    let dir = home.join("sessions");
    let session = Session::create(
        &dir,
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
            tools: specs().iter().map(|s| s.name.clone()).collect(),
            tools_hash: Some(mecha_core::surface::fingerprint(&specs())),
            rules_surface: Some(SessionKind::Tui),
            max_tokens: 1024,
            max_turns: 4,
            ..Default::default()
        }))
        .unwrap();
    let tool_use = |id: &str, name: &str, input: Value| Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    };
    let result = |id: &str, content: &str| Block::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        is_error: false,
    };
    let mut steer = Message::tool_results(vec![result("t1", "a.md b.md"), result("t2", "x")]);
    steer
        .content
        .push(Block::text("change of plan: only summarize b.md"));
    for m in [
        Message::user("audit the reports"),
        Message::assistant(vec![
            tool_use("t1", "fs_list", json!({})),
            tool_use("t2", "fs_read", json!({"path": "a.md"})),
        ]),
        steer,
        Message::assistant(vec![tool_use("t3", "fs_read", json!({"path": "b.md"}))]),
        Message::tool_results(vec![result("t3", "b")]),
        Message::assistant(vec![Block::text("b.md says b")]),
    ] {
        session.append(&Record::Message(m)).unwrap();
    }
    session
        .append(&Record::Outcome(RunStats {
            turns: 3,
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::Taint(Taint {
            untrusted,
            private: true,
        }))
        .unwrap();
    session.meta.id.clone()
}

/// The fixture model: whatever it is asked, it answers in prose — so the
/// unsteered replay stops without the recorded call, and the probe grades.
async fn fixture_model() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::extract::Json<Value>| async move {
            if body.get("stream").and_then(Value::as_bool) == Some(true) {
                let chunk = json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "summarized a.md instead"}, "finish_reason": null}]});
                let done = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 4}});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                )
            } else {
                let body = json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": "summarized a.md instead"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 4}});
                ([("content-type", "application/json")], body.to_string())
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), server)
}

async fn appraise_probe(home: &Path, work: &Path) -> Value {
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(["sessions", "appraise", "--probe", "--json"])
            .env("MECHA_HOME", home)
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(work)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("appraise --probe finished")
    .unwrap();
    assert!(
        out.status.success(),
        "appraise --probe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[tokio::test]
async fn a_probe_run_leaves_comparisons_a_second_read_returns_and_tainted_sessions_none() {
    let root =
        Root(std::env::temp_dir().join(format!("mecha-comparison-store-{}", Session::new_id())));
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
             model = \"fixture\"\nmax_retries = 0\n\
             [tools]\nenabled = [\"fs_read\", \"fs_list\"]\n"
        ),
    )
    .unwrap();
    // The recorded surface, kept where the probe will look for it.
    mecha_core::surface::SurfaceStore::open(home.join("surfaces"))
        .unwrap()
        .record(&specs())
        .unwrap();
    let clean = steered_session(&home, false);
    let tainted = steered_session(&home, true);

    let first = appraise_probe(&home, &work).await;
    server.abort();

    let probe = &first["probe"];
    assert_eq!(probe["driven"], 2, "both steers drove: {first:#}");
    assert_eq!(probe["stored"]["written"], 1, "{first:#}");
    assert_eq!(probe["stored"]["refused_not_clean"], 1, "{first:#}");
    assert_eq!(first["comparisons"]["records"], 1, "{first:#}");
    assert_eq!(first["comparisons"]["read"], true);

    // The second read: a fresh handle on the store the binary wrote.
    let rows = ComparisonStore::open(home.join("comparisons"))
        .unwrap()
        .comparisons()
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.pointers.session_id, clean, "never {tainted}");
    assert_eq!(row.kind, Kind::SteerProbe);
    assert_eq!(row.validator, Validator::StructuralSteer);
    // The fixture model never makes the steered call, so the steer was
    // load-bearing: the recording is preferred over the unsteered replay.
    assert_eq!(probe["mattered"], 2, "{first:#}");
    assert_eq!(row.verdict, Verdict::Separated);
    assert_eq!(row.preferred, vec![0]);
    assert_eq!(row.arms.len(), 2);
    assert_eq!(row.arms[0].role, Role::Recorded);
    assert_eq!(row.arms[1].role, Role::WithoutIntervention);
    assert_eq!(row.model, "fixture");
    assert_eq!(
        row.call.as_ref().map(|c| c.tool.as_str()),
        Some("fs_read"),
        "{row:?}"
    );
}
