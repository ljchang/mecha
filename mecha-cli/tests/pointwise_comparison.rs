//! Row 2d-1's acceptance through the real binary: `mecha sessions compare`
//! drives K policies from each informative decision point of a clean
//! recorded session against a loopback fixture model, grades each arm by the
//! owner's recorded verdict, and leaves one comparison per point that a
//! fresh store handle returns; a point no structural validator can pose is
//! stored inconclusive with nothing driven; a tainted session leaves
//! nothing; and a second pass compares nothing twice
//! (`docs/APPRAISAL-WIRING-DESIGN.md` O1). No network, no real model.

use mecha_core::agent::Taint;
use mecha_core::comparison::{ComparisonStore, Kind, Role, Validator, Verdict};
use mecha_core::message::{Block, Message, ToolSpec};
use mecha_core::planning::{Feedback, StepFeedback, Verification};
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

const RULES: &str = "## Learned rules\n\nRules distilled from how this user has corrected \
                     you before.\n\n### behavior\n- Ask before mailing anyone outside the team.";

/// The draft the owner rejected, exactly as the run staged it — what the
/// fixture model drafts again whenever the rules block rides.
fn rejected_draft() -> Value {
    json!({"to": "cleo@example.invalid", "body": "Q3 is done."})
}

fn specs() -> Vec<ToolSpec> {
    let spec = |name: &str, properties: Value| ToolSpec {
        name: name.into(),
        description: format!("{name} things."),
        input_schema: json!({"type": "object", "properties": properties}),
    };
    vec![
        spec("fs_list", json!({})),
        spec("fs_read", json!({"path": {"type": "string"}})),
        spec("fs_write", json!({"path": {"type": "string"}})),
        spec(
            "mail_send",
            json!({"to": {"type": "string"}, "body": {"type": "string"},
                   "urgent": {"type": "boolean", "default": false}}),
        ),
        spec("todo", json!({"op": {"type": "string"}})),
    ]
}

fn step(verification: Verification, expected: Option<u32>, actual: Option<u32>) -> StepFeedback {
    StepFeedback {
        criterion: None,
        completion_batch: None,
        call_id: Some("t5".into()),
        step: "total the quarter".into(),
        goal: None,
        expected: None,
        expected_calls: expected,
        actual_calls: actual,
        verification,
        check_tampered: false,
    }
}

/// A session with a steer, a denial, two staged drafts, a declared check
/// that failed and a forecast that missed, recorded as `chat` writes one.
fn session(home: &Path, untrusted: bool) -> String {
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
            tools: specs().iter().map(|s| s.name.clone()).collect(),
            tools_hash: Some(mecha_core::surface::fingerprint(&specs())),
            system_prompt: Some(format!("You are mecha.\n\n{RULES}")),
            rules_hash: Some(mecha_core::learning::rules_hash(RULES)),
            rules_surface: Some(SessionKind::Tui),
            max_tokens: 1024,
            max_turns: 8,
            ..Default::default()
        }))
        .unwrap();
    let tool_use = |id: &str, name: &str, input: Value| Block::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    };
    let result = |id: &str, content: &str, is_error: bool| Block::ToolResult {
        tool_use_id: id.into(),
        content: content.into(),
        is_error,
    };
    let mut steer = Message::tool_results(vec![result("t1", "q3.csv q4.csv", false)]);
    steer
        .content
        .push(Block::text("only the third quarter, please"));
    let mut feedback = Message::tool_results(vec![result("t5", "done", false)]);
    feedback.planning = Some(Feedback {
        steps: vec![
            step(Verification::Failed, None, None),
            step(Verification::Passed, Some(2), Some(9)),
        ],
        ..Default::default()
    });
    for m in [
        Message::user("total the quarters and mail Dirk"),
        Message::assistant(vec![tool_use("t1", "fs_list", json!({}))]),
        steer,
        Message::assistant(vec![tool_use("t2", "fs_write", json!({"path": "q3.md"}))]),
        Message::tool_results(vec![result(
            "t2",
            "Denied by the user: not that file",
            true,
        )]),
        Message::assistant(vec![tool_use(
            "t3",
            "mail_send",
            json!({"to": "dirk@example.invalid", "body": "Totals attached."}),
        )]),
        Message::tool_results(vec![result("t3", "staged for review", false)]),
        Message::assistant(vec![tool_use("t4", "mail_send", rejected_draft())]),
        Message::tool_results(vec![result("t4", "staged for review", false)]),
        Message::assistant(vec![tool_use("t5", "todo", json!({"op": "complete"}))]),
        feedback,
        Message::assistant(vec![Block::text("Done.")]),
    ] {
        session.append(&Record::Message(m)).unwrap();
    }
    session
        .append(&Record::Outcome(RunStats {
            turns: 6,
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

/// The owner's acts on the clean session's two drafts: the one to Dirk
/// rewritten and sent, the one to Cleo rejected.
fn owner_acts(home: &Path, session_id: &str) {
    let outbox = mecha_core::outbox::OutboxStore::open(home.join("outbox")).unwrap();
    let stage = |call: &str, mut args: Value| {
        args["urgent"] = json!(false);
        outbox
            .stage(
                "mail_send",
                mecha_core::outbox::OutboxKind::Message,
                args,
                Taint::default(),
                mecha_core::outbox::Provenance {
                    session_id: Some(session_id.into()),
                    call_id: Some(call.into()),
                    filled_defaults: vec!["urgent".into()],
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let edited = stage(
        "t3",
        json!({"to": "dirk@example.invalid", "body": "Totals attached."}),
    );
    outbox
        .update_args(
            &edited.id,
            json!({"to": "dirk@example.invalid", "body": "Totals attached; the Q3 sheet follows.", "urgent": false}),
        )
        .unwrap();
    outbox.resolve(&edited.id, "sent", None).unwrap();
    let rejected = stage("t4", rejected_draft());
    outbox
        .resolve(&rejected.id, "rejected", Some("not yet".into()))
        .unwrap();
}

/// The fixture model. Under a prompt carrying the rules block it drafts the
/// rejected mail again, word for word; under any other it answers in prose
/// and stops. So at the rejected draft the rules arm fails and the
/// rules-free arm passes — a separated verdict the owner's act decided.
async fn fixture_model() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::extract::Json<Value>| async move {
            let system = body["messages"]
                .as_array()
                .and_then(|m| m.first())
                .filter(|m| m["role"] == "system")
                .map(|m| m["content"].to_string())
                .unwrap_or_default();
            let drafts = system.contains("Learned rules");
            let usage = json!({"prompt_tokens": 10, "completion_tokens": 4});
            let call = json!([{"index": 0, "id": "call-1", "type": "function",
                "function": {"name": "mail_send", "arguments": rejected_draft().to_string()}}]);
            let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);
            if streaming {
                let (delta, finish) = if drafts {
                    (json!({"role": "assistant", "tool_calls": call}), "tool_calls")
                } else {
                    (
                        json!({"role": "assistant", "content": "I would hold that mail."}),
                        "stop",
                    )
                };
                let chunk = json!({"choices": [{"index": 0, "delta": delta, "finish_reason": null}]});
                let done = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": usage});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                )
            } else {
                let (message, finish) = if drafts {
                    (json!({"role": "assistant", "content": null, "tool_calls": call}), "tool_calls")
                } else {
                    (
                        json!({"role": "assistant", "content": "I would hold that mail."}),
                        "stop",
                    )
                };
                let body = json!({"choices": [{"index": 0, "message": message, "finish_reason": finish}], "usage": usage});
                ([("content-type", "application/json")], body.to_string())
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), server)
}

async fn compare(home: &Path, work: &Path) -> Value {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            // `--include-tests`: CI exports `MECHA_SESSION_KIND=test`, which
            // marks the fixture sessions this process creates as smoke
            // tests, and the corpus readers hide those by default.
            .args([
                "sessions",
                "compare",
                "--json",
                "--include-tests",
                "--points",
                "10",
                "--seed",
                "20260925",
            ])
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
    .expect("sessions compare finished")
    .unwrap();
    assert!(
        out.status.success(),
        "sessions compare failed: {}",
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
async fn a_compare_pass_leaves_a_comparison_per_point_a_second_read_returns() {
    let root = Root(std::env::temp_dir().join(format!("mecha-pointwise-{}", Session::new_id())));
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
    mecha_core::surface::SurfaceStore::open(home.join("surfaces"))
        .unwrap()
        .record(&specs())
        .unwrap();
    let clean = session(&home, false);
    let tainted = session(&home, true);
    owner_acts(&home, &clean);

    let first = compare(&home, &work).await;
    let pass = &first["pass"];
    assert_eq!(first["seed"], 20260925, "{first:#}");
    assert_eq!(pass["found"]["steer"], 2, "{first:#}");
    assert_eq!(pass["found"]["edited-draft"], 1, "{first:#}");
    assert_eq!(pass["found"]["rejected-draft"], 1, "{first:#}");
    assert_eq!(
        pass["not_clean"], 4,
        "the tainted session's points: {first:#}"
    );
    assert_eq!(pass["driven"], 4, "{first:#}");
    assert_eq!(pass["arms_driven"], 8, "{first:#}");
    assert_eq!(pass["unposed"], 2, "{first:#}");
    assert_eq!(pass["stored"]["written"], 6, "{first:#}");
    assert_eq!(first["comparisons"]["records"], 6, "{first:#}");
    assert_eq!(first["comparisons"]["unposed"], 2, "{first:#}");

    // The second read: a fresh handle on the store the binary wrote.
    let rows = ComparisonStore::open(home.join("comparisons"))
        .unwrap()
        .comparisons()
        .unwrap();
    assert_eq!(rows.len(), 6, "{rows:#?}");
    assert!(
        rows.iter().all(|r| r.pointers.session_id == clean),
        "never {tainted}"
    );
    let row = |k: Kind| rows.iter().find(|r| r.kind == k).unwrap();

    let rejected = row(Kind::PointRejectedDraft);
    assert_eq!(rejected.validator, Validator::RejectedDraft);
    assert_eq!(
        rejected.arms.iter().map(|a| a.role).collect::<Vec<_>>(),
        vec![Role::WithoutIntervention, Role::RulesFree]
    );
    assert_eq!(
        (rejected.verdict, rejected.preferred.clone()),
        (Verdict::Separated, vec![1]),
        "the arm that drafted the rejected mail again failed; the one that held it passed"
    );
    assert_eq!(rejected.model, "fixture");
    // A draft the owner rewrote: neither arm wrote the owner's words, so
    // the owner never judged what they did.
    assert_eq!(
        row(Kind::PointEditedDraft).validator,
        Validator::ReleasedDraft
    );
    assert_eq!(row(Kind::PointEditedDraft).verdict, Verdict::Inconclusive);
    assert_eq!(row(Kind::PointSteer).validator, Validator::StructuralSteer);
    assert_eq!(
        row(Kind::PointDenial).validator,
        Validator::StructuralDenial
    );
    for k in [Kind::PointCheck, Kind::PointSurprise] {
        let r = row(k);
        assert_eq!(
            (r.validator, r.verdict, r.arms.len()),
            (Validator::Unposed, Verdict::Inconclusive, 0),
            "{k:?} is never judged"
        );
    }
    let wire = std::fs::read_to_string(home.join("comparisons").join("comparisons.jsonl")).unwrap();
    for leaked in [
        "Q3 is done",
        "Totals attached",
        "cleo@",
        "only the third quarter",
    ] {
        assert!(!wire.contains(leaked), "{leaked} reached the store");
    }

    // A second pass compares nothing twice.
    let second = compare(&home, &work).await;
    server.abort();
    assert_eq!(second["pass"]["already_compared"], 6, "{second:#}");
    assert_eq!(second["pass"]["stored"]["written"], 0, "{second:#}");
    assert_eq!(second["pass"]["driven"], 0, "{second:#}");
    assert_eq!(
        ComparisonStore::open(home.join("comparisons"))
            .unwrap()
            .comparisons()
            .unwrap()
            .len(),
        6
    );
}

/// R29, structurally: a provider whose endpoint is not this machine refuses
/// the pass before anything is read or written.
#[tokio::test]
async fn a_provider_off_this_machine_refuses_the_pass() {
    let root =
        Root(std::env::temp_dir().join(format!("mecha-pointwise-r29-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "default_provider = \"remote\"\n\
         [providers.remote]\nkind = \"openai-compatible\"\nbase_url = \"https://models.example.invalid/v1\"\n\
         model = \"remote\"\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(["sessions", "compare", "--json"])
        .env("MECHA_HOME", &home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("OPENAI_API_KEY")
        .current_dir(&work)
        .output()
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("R29"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.join("comparisons").exists());
}
