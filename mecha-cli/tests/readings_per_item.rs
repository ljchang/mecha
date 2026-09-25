//! Row 1e's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` S5): a fixture store with one stale draft and several fresh
//! ones, under a charter line that has read past its setpoint on each of
//! the last ten recorded runs. The level reads saturated throughout, while
//! the per-item reading and each run's delta move as drafts are added and
//! cleared; the line is withdrawn from the runs and reported once by the
//! doctor. The model is a fixture OpenAI-compatible server on a loopback
//! port — no network, no real model — and it is also what moves the queue:
//! it stages and sends drafts while a run is in flight, which is the only
//! way to put a change *inside* a run's window from outside the binary.

use mecha_core::outbox::OutboxStore;
use mecha_core::reading::{LineReading, Observed, Reading, SATURATED_AFTER_RUNS};
use mecha_core::session::{Record, RunStats, Session, SessionKind, SessionMeta};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const CHARTER: &str = "[[line]]\nid = \"replies\"\ntext = \"Answer the people waiting on me.\"\n\
                       [line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n";

/// Stage a draft and back-date it by `days`.
fn draft(outbox: &OutboxStore, days: i64) -> String {
    let item = outbox
        .stage_by_harness("mail_send", json!({"to": "someone@example.org"}))
        .unwrap();
    if days > 0 {
        let path = outbox.root().join(format!("{}.json", item.id));
        let mut raw: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        raw["created_at"] = json!((chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339());
        std::fs::write(&path, raw.to_string()).unwrap();
    }
    item.id
}

/// Ten recorded runs, each reading the line past its setpoint — the
/// history a saturated line has, written as an earlier binary wrote it
/// (level only, no per-item form).
fn saturated_history(home: &Path) {
    for i in 0..SATURATED_AFTER_RUNS {
        let s = Session::create(
            &home.join("sessions"),
            SessionMeta {
                id: format!("20260901T00{i:02}00-hist"),
                created_at: chrono::Utc::now() - chrono::Duration::hours(24 - i as i64),
                provider: "fixture".into(),
                model: "fixture".into(),
                workspace: home.to_path_buf(),
                title: None,
                kind: Some(SessionKind::Web),
            },
        )
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            homeostat: Some(mecha_core::homeostat::Homeostat {
                charter: Some(vec![LineReading {
                    line: "replies".into(),
                    kind: mecha_core::charter::SensorKind::OutboxAge,
                    setpoint: "24h".into(),
                    reading: Reading::Observed {
                        value: Observed::Seconds(4 * 86_400),
                        over: true,
                        excess: 0.75,
                    },
                    items: None,
                    delta: None,
                    withdrawn: false,
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .unwrap();
    }
}

/// The fixture model. On each request it first applies the next queued
/// change to the outbox — inside the run's window, between the homeostat's
/// two reads — and then answers in prose, ending the run.
async fn fixture_model(
    home: PathBuf,
    fresh: Arc<Mutex<Vec<String>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::extract::Json<Value>| {
            let (home, fresh, calls) = (home.clone(), fresh.clone(), calls.clone());
            async move {
                let outbox = OutboxStore::open(home.join("outbox")).unwrap();
                match calls.fetch_add(1, Ordering::SeqCst) {
                    // Run one: two drafts staged.
                    0 => {
                        for _ in 0..2 {
                            fresh.lock().unwrap().push(draft(&outbox, 0));
                        }
                    }
                    // Run two: two fresh drafts sent.
                    1 => {
                        let sent: Vec<String> = fresh.lock().unwrap().drain(..2).collect();
                        for id in sent {
                            outbox.resolve(&id, "sent", None).unwrap();
                        }
                    }
                    _ => {}
                }
                let text = "Nothing else needs doing.";
                if body.get("stream").and_then(Value::as_bool) == Some(true) {
                    let chunk = json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]});
                    let done = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 4}});
                    (
                        [("content-type", "text/event-stream")],
                        format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                    )
                } else {
                    let body = json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 4}});
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

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
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
    .unwrap_or_else(|_| panic!("mecha {args:?} hung"))
    .unwrap()
}

fn json_of(out: &std::process::Output, what: &str) -> Value {
    assert!(
        out.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{what}: not JSON ({e}): {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// The `replies` line as `mecha charter --json` shows it.
async fn charter_reading(home: &Path, work: &Path) -> Value {
    let v = json_of(&mecha(home, work, &["charter", "--json"]).await, "charter");
    v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == "replies")
        .unwrap_or_else(|| panic!("no `replies` line: {v:#}"))["reading"]
        .clone()
}

/// The doctor's charter findings for the line.
async fn saturation_findings(home: &Path, work: &Path) -> Vec<Value> {
    let out = mecha(home, work, &["doctor", "--json"]).await;
    // Findings are the diagnosis, carried in the exit code: 1 with findings,
    // never anything else.
    assert_eq!(
        out.status.code(),
        Some(1),
        "doctor: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v.as_array()
        .unwrap_or_else(|| panic!("no findings array: {v:#}"))
        .iter()
        .filter(|f| f["component"] == "charter")
        .cloned()
        .collect()
}

/// The newest smoke-test session's recorded run, as the binary wrote it.
fn newest_run(home: &Path) -> mecha_core::homeostat::Homeostat {
    let listed = Session::list(&home.join("sessions")).unwrap();
    let (_, path) = listed
        .iter()
        .find(|(meta, _)| meta.kind == Some(SessionKind::Test))
        .expect("the run recorded a session");
    Session::outcomes_attributed(path)
        .unwrap()
        .pop()
        .expect("an outcome row")
        .2
        .homeostat
        .expect("a homeostat on the run")
}

#[tokio::test]
async fn a_saturated_line_is_withdrawn_and_reported_once_while_the_per_item_reading_moves() {
    // The history must stay admitted: CI exports `MECHA_SESSION_KIND=test`,
    // which would mark the sessions this process writes as smoke tests.
    mecha_core::session::ignore_kind_env_for_tests();
    let root = Root(std::env::temp_dir().join(format!("mecha-readings-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(home.join("charter.toml"), CHARTER).unwrap();
    let outbox = OutboxStore::open(home.join("outbox")).unwrap();
    let stale = draft(&outbox, 5);
    let fresh = Arc::new(Mutex::new(
        (0..3).map(|_| draft(&outbox, 0)).collect::<Vec<_>>(),
    ));
    saturated_history(&home);
    let (base_url, server) = fixture_model(home.clone(), fresh.clone()).await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"openai-compatible\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n\
             [tools]\nenabled = [\"fs_read\"]\n"
        ),
    )
    .unwrap();

    // The owner's surface: the level saturated, the per-item form beside
    // it naming the stale draft, and the line marked withdrawn from runs.
    let before = charter_reading(&home, &work).await;
    assert_eq!(before["over"], true, "{before:#}");
    assert_eq!(before["items"]["waiting"], 4, "{before:#}");
    assert_eq!(before["items"]["over"], 1, "{before:#}");
    assert_eq!(before["items"]["stale"], json!([stale]), "{before:#}");
    assert_eq!(before["withdrawn"], true, "{before:#}");

    // Reported once.
    let findings = saturation_findings(&home, &work).await;
    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert!(
        findings[0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("charter line `replies` has read past its 24h setpoint"),
        "{findings:#?}"
    );

    // Run one: two drafts staged inside the run.
    let out = mecha(&home, &work, &["run", "What is waiting on me?", "--json"]).await;
    assert!(
        out.status.success(),
        "run one failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let one = newest_run(&home);
    let line = &one.charter.as_ref().unwrap()[0];
    assert_eq!(line.reading.over(), Some(true), "the level: saturated");
    assert!(line.withdrawn, "withdrawn from the run, kept on the record");
    let items = line.items.clone().unwrap();
    assert_eq!((items.waiting, items.over), (4, 1));
    assert_eq!(items.stale, vec![stale.clone()]);
    let flow = line.delta.unwrap();
    assert_eq!((flow.added, flow.cleared), (2, 0));
    assert_eq!(
        one.backlog_delta.unwrap().flow.unwrap().outbox,
        Some(flow),
        "the backlog's per-item delta is the line's store's"
    );

    // Run two: two fresh drafts sent inside the run.
    let out = mecha(&home, &work, &["run", "Anything else?", "--json"]).await;
    assert!(
        out.status.success(),
        "run two failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    server.abort();
    let two = newest_run(&home);
    let line = &two.charter.as_ref().unwrap()[0];
    assert_eq!(
        line.reading.over(),
        Some(true),
        "the level: still saturated"
    );
    assert!(line.withdrawn);
    let items = line.items.clone().unwrap();
    assert_eq!(
        (items.waiting, items.over),
        (6, 1),
        "the per-item reading moved"
    );
    let flow = line.delta.unwrap();
    assert_eq!((flow.added, flow.cleared), (0, 2), "and so did the delta");

    // Still one finding, and the surface reads the queue as the runs left it.
    assert_eq!(saturation_findings(&home, &work).await.len(), 1);
    let after = charter_reading(&home, &work).await;
    assert_eq!(after["items"]["waiting"], 4, "{after:#}");
    assert_eq!(after["withdrawn"], true, "{after:#}");

    // The phase-1 readout: the level pinned, the per-item reading varying.
    let health = json_of(
        &mecha(
            &home,
            &work,
            &["sessions", "health", "--json", "--include-tests"],
        )
        .await,
        "sessions health",
    );
    let replies = health["charter_readings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["line"] == "replies")
        .unwrap_or_else(|| panic!("no readout for `replies`: {health:#}"))
        .clone();
    assert_eq!(replies["informative"], 12, "{replies:#}");
    assert_eq!(replies["level_over"], 12, "{replies:#}");
    assert_eq!(replies["per_item_runs"], 2, "{replies:#}");
    // Waiting 4 and 6: a population variance of one.
    assert_eq!(replies["waiting_variance"], 1.0, "{replies:#}");
    assert_eq!(replies["over_variance"], 0.0, "{replies:#}");
    assert_eq!(replies["delta_runs"], 2, "{replies:#}");
    assert_eq!(replies["moved_runs"], 2, "{replies:#}");
    assert_eq!(replies["withdrawn_runs"], 2, "{replies:#}");
}
