//! Row 2e-1's acceptance through the real binary: `mecha learn
//! --compare-sources` drives the validation probe at each intervention the
//! reflector reflected on, three times — no rules, the reflector's lesson,
//! the session appraisal's lessons — against a loopback fixture model, and
//! reports per intervention region each source's validation rate with its
//! counts beneath it; a region with nothing decided reads `null`; a session
//! whose appraisal is tainted is excluded and counted; the verdicts are 1g
//! comparisons a fresh store handle and `sessions appraise` read back; and
//! nothing reaches the rules store (`docs/APPRAISAL-WIRING-DESIGN.md` 2e-1,
//! L2, R25). No network, no real model.

use mecha_core::agent::Taint;
use mecha_core::appraisal_store::{AppraisalStore, Draft, Recorded, SessionEvidence};
use mecha_core::comparison::{ComparisonStore, Kind, Outcome, Role, Validator, Verdict};
use mecha_core::learning::{LearningStore, Reflexion};
use mecha_core::message::{Block, Message, ToolSpec};
use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta};
use mecha_core::situation::Situation;
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
                     you before.\n\n### behavior\n- Keep replies short.";

/// The reflector's lesson — the one the fixture model heeds.
const REFLECTOR_LESSON: &str = "Ask before overwriting a quarter's file.";
/// The appraisal's lessons — which the fixture model does not.
const APPRAISAL_LESSON: &str = "Write the quarter summary straight away.";

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
            json!({"to": {"type": "string"}, "body": {"type": "string"}}),
        ),
    ]
}

/// A session in which the owner refused `tool` with `input` — "not that
/// file" — recorded as `chat` writes one. Returns its id and path.
fn session(home: &Path, tool: &str, input: Value, untrusted: bool) -> (String, PathBuf) {
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
    for m in [
        Message::user("total the quarters and file the summary"),
        Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "fs_list".into(),
            input: json!({}),
        }]),
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: "q3.md q4.md".into(),
            is_error: false,
        }]),
        Message::assistant(vec![Block::ToolUse {
            id: "t2".into(),
            name: tool.into(),
            input,
        }]),
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t2".into(),
            content: "Denied by the user: not that file".into(),
            is_error: true,
        }]),
        Message::assistant(vec![Block::text("Understood; I left it alone.")]),
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
    (session.meta.id.clone(), session.path.clone())
}

/// The reflection the reflector wrote about a session's denial, mined
/// clean (the refusal came before any third-party content), in the region
/// of `tools`.
fn reflection(session_id: &str, tools: &[&str]) -> Reflexion {
    serde_json::from_value(json!({
        "id": format!("refl-{session_id}"),
        "domain": "behavior",
        "session_id": session_id,
        "trigger": "denial",
        "context": "The run tried to overwrite a quarter's file.",
        "intervention": "not that file",
        "reflexion_text": REFLECTOR_LESSON,
        "error_type": null,
        "confidence": null,
        "created_at": "2026-09-25T00:00:00Z",
        "origin": "clean",
        "situation": Situation::recorded(
            &tools.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            "denial",
            Some(SessionKind::Tui),
            None,
        ),
    }))
    .unwrap()
}

/// The session's text appraisal, written through the store's own door.
fn appraise_session(home: &Path, path: &Path, expect_clean: bool) {
    let store = AppraisalStore::open(home.join("appraisals")).unwrap();
    let evidence = SessionEvidence::read(path).unwrap();
    let draft = Draft {
        interpretation: "The owner kept the run off a file they had not asked it to touch.".into(),
        lessons: vec![APPRAISAL_LESSON.into()],
        ..Draft::default()
    };
    match store
        .record(
            &evidence,
            draft,
            "fixture",
            &mecha_core::distill::KnownPointers::default(),
        )
        .unwrap()
    {
        Recorded::Written { clean, .. } => assert_eq!(clean, expect_clean),
        other => panic!("{other:?}"),
    }
}

/// The fixture model. Under a prompt carrying the reflector's lesson it
/// answers in prose and stops; under any other it makes the refused call
/// again. So at the denial the reflector's arm passes and the other two
/// fail.
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
            let heeds = system.contains(REFLECTOR_LESSON);
            let usage = json!({"prompt_tokens": 10, "completion_tokens": 4});
            let call = json!([{"index": 0, "id": "call-1", "type": "function",
                "function": {"name": "fs_write", "arguments": json!({"path": "q3.md"}).to_string()}}]);
            let streaming = body.get("stream").and_then(Value::as_bool) == Some(true);
            if streaming {
                let (delta, finish) = if heeds {
                    (
                        json!({"role": "assistant", "content": "Shall I overwrite q3.md?"}),
                        "stop",
                    )
                } else {
                    (json!({"role": "assistant", "tool_calls": call}), "tool_calls")
                };
                let chunk = json!({"choices": [{"index": 0, "delta": delta, "finish_reason": null}]});
                let done = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": usage});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                )
            } else {
                let (message, finish) = if heeds {
                    (
                        json!({"role": "assistant", "content": "Shall I overwrite q3.md?"}),
                        "stop",
                    )
                } else {
                    (json!({"role": "assistant", "content": null, "tool_calls": call}), "tool_calls")
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

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> Value {
    let out = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
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
    .expect("mecha finished")
    .unwrap();
    assert!(
        out.status.success(),
        "mecha {args:?} failed: {}",
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

/// Every file under `dir` and its bytes — what "nothing was written" is
/// checked against.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap_or_default());
            }
        }
    }
    out
}

fn region<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["region"] == key)
        .unwrap_or_else(|| panic!("no region {key}: {report:#}"))
}

#[tokio::test]
async fn lessons_from_both_sources_are_measured_per_region_and_nothing_is_learned() {
    let root = Root(std::env::temp_dir().join(format!("mecha-lessons-{}", Session::new_id())));
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

    // Clean on both sides: compared.
    let (clean, clean_path) = session(&home, "fs_write", json!({"path": "q3.md"}), false);
    appraise_session(&home, &clean_path, true);
    // The reflection was mined clean, but third-party content reached the
    // session afterwards, so its appraisal is tainted: excluded, counted.
    let (tainted, tainted_path) = session(
        &home,
        "mail_send",
        json!({"to": "cleo@example.invalid", "body": "Q3 is done."}),
        true,
    );
    appraise_session(&home, &tainted_path, false);
    let learning = LearningStore::open(home.join("learning")).unwrap();
    learning
        .append_reflexion(&reflection(&clean, &["fs_list", "fs_write"]))
        .unwrap();
    learning
        .append_reflexion(&reflection(&tainted, &["fs_list", "mail_send"]))
        .unwrap();
    let before = snapshot(&home.join("learning"));

    let first = mecha(
        &home,
        &work,
        &[
            "learn",
            "--compare-sources",
            "--json",
            "--interventions",
            "5",
            "--seed",
            "20260925",
        ],
    )
    .await;
    let pass = &first["pass"];
    assert_eq!(first["seed"], 20260925, "{first:#}");
    assert_eq!(first["model"], "fixture");
    assert_eq!(pass["reflections_read"], 2, "{first:#}");
    assert_eq!(pass["eligible"], 1, "{first:#}");
    assert_eq!(
        pass["excluded"]["clean_for_reflector_only"], 1,
        "the tainted appraisal is excluded and counted: {first:#}"
    );
    assert_eq!(pass["driven"], 1, "{first:#}");
    assert_eq!(pass["arms_driven"], 3, "{first:#}");
    assert_eq!(pass["stored"]["written"], 1, "{first:#}");

    // Per region: each source's rate, the counts beneath it.
    let report = &first["report"];
    assert_eq!(report["model"], "fixture");
    let write = region(report, "fs_list,fs_write on tui");
    assert_eq!(write["eligible"], 1, "{report:#}");
    assert_eq!(write["decided"], 1, "{report:#}");
    assert_eq!(write["reflector"]["rate"], 1.0, "{report:#}");
    assert_eq!(write["reflector"]["pass"], 1);
    assert_eq!(write["reflector"]["improved"], 1);
    assert_eq!(write["appraisal"]["rate"], 0.0, "{report:#}");
    assert_eq!(write["appraisal"]["fail"], 1);
    assert_eq!(write["rules_free"]["rate"], 0.0);
    assert_eq!(write["unavailable"], 0);
    // The region whose only intervention was excluded: nothing decided, so
    // every rate is null — a dash, never zero.
    let mail = region(report, "fs_list,mail_send on tui");
    assert_eq!(mail["eligible"], 0, "{report:#}");
    assert_eq!(mail["decided"], 0);
    for source in ["reflector", "appraisal", "rules_free"] {
        assert!(mail[source]["rate"].is_null(), "{source}: {report:#}");
    }
    assert_eq!(mail["excluded"]["clean_for_reflector_only"], 1);

    // The verdict is a 1g comparison a fresh handle returns.
    let rows = ComparisonStore::open(home.join("comparisons"))
        .unwrap()
        .comparisons()
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    let row = &rows[0];
    assert_eq!(row.kind, Kind::LessonSource);
    assert_eq!(row.validator, Validator::StructuralDenial);
    assert_eq!(
        row.arms
            .iter()
            .map(|a| (a.role, a.outcome))
            .collect::<Vec<_>>(),
        vec![
            (Role::RulesFree, Outcome::Fail),
            (Role::ReflectorLesson, Outcome::Pass),
            (Role::AppraisalLesson, Outcome::Fail),
        ]
    );
    assert_eq!(
        (row.verdict, row.preferred.clone()),
        (Verdict::Separated, vec![1])
    );
    assert_eq!(row.pointers.session_id, clean, "never {tainted}");
    assert_eq!(
        row.pointers.reflection_id.as_deref(),
        Some(format!("refl-{clean}").as_str())
    );
    assert!(row.pointers.appraisal_id.is_some());
    let wire = std::fs::read_to_string(home.join("comparisons").join("comparisons.jsonl")).unwrap();
    for leaked in [REFLECTOR_LESSON, APPRAISAL_LESSON, "not that file", "q3.md"] {
        assert!(!wire.contains(leaked), "{leaked} reached the store");
    }

    // Nothing reached the rules store: the learning store is byte for byte
    // what it was — no rule, no proposal, no validation-ledger row.
    assert_eq!(snapshot(&home.join("learning")), before);

    // The readout re-reads the stores and says the same.
    let appraise = mecha(
        &home,
        &work,
        &["sessions", "appraise", "--json", "--include-tests"],
    )
    .await;
    let readout = &appraise["lesson_sources"];
    assert_eq!(readout["read"], true, "{appraise:#}");
    let again = region(readout, "fs_list,fs_write on tui");
    assert_eq!(again["reflector"]["rate"], 1.0, "{readout:#}");
    assert_eq!(again["appraisal"]["rate"], 0.0);
    assert!(
        again["unavailable"].is_null(),
        "outside a pass, unavailable is unknown: {readout:#}"
    );

    // The text readout names each region with its rates.
    let text = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(["sessions", "appraise", "--include-tests"])
        .env("MECHA_HOME", &home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("MECHA_LEARNING_DIR")
        .current_dir(&work)
        .output()
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("lessons by source"), "{text}");
    assert!(text.contains("reflector   100% (1 of 1 decided)"), "{text}");
    assert!(text.contains("appraisal   0% (0 of 1 decided)"), "{text}");
    assert!(text.contains("— (nothing decided)"), "{text}");

    // A second pass measures nothing twice.
    let second = mecha(&home, &work, &["learn", "--compare-sources", "--json"]).await;
    server.abort();
    assert_eq!(second["pass"]["already_measured"], 1, "{second:#}");
    assert_eq!(second["pass"]["driven"], 0, "{second:#}");
    assert_eq!(snapshot(&home.join("learning")), before);
}

/// R29, structurally: a provider whose endpoint is not this machine refuses
/// the pass before anything is read or written.
#[tokio::test]
async fn a_provider_off_this_machine_refuses_the_pass() {
    let root = Root(std::env::temp_dir().join(format!("mecha-lessons-r29-{}", Session::new_id())));
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
        .args(["learn", "--compare-sources", "--json"])
        .env("MECHA_HOME", &home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("MECHA_LEARNING_DIR")
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
    assert!(!home.join("learning").exists());
}
