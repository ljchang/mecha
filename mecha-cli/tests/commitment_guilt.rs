//! Row 1f's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` S7, ruling R12): a fixture home holding pending commitments of
//! every kind — staged drafts of three ages, a parked question, a front-door
//! request waiting on the owner — under a charter whose lines watch two of
//! the three stores. One run of `mecha run` against a fixture model on a
//! loopback port (no network, no real model) records, on its homeostat,
//! each commitment with its own guilt — excess over its patience times its
//! line's rank weight — and an `anticipated_guilt` readout equal to the
//! largest of them. The retired scalar was one number over all of them,
//! with the run's context pressure folded in.

use mecha_core::guilt::{Store, StoreGuilt};
use mecha_core::outbox::OutboxStore;
use mecha_core::session::{Session, SessionKind};
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

/// The top line watches the outbox by age, the third the questions; no line
/// watches the front door, so a request's patience is the doctor's 72h and
/// it ranks below all three lines.
const CHARTER: &str = "[[line]]\nid = \"replies\"\ntext = \"Answer the people waiting on me.\"\n\
                       [line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n\
                       [[line]]\nid = \"craft\"\ntext = \"Leave work better than you found it.\"\n\
                       [[line]]\nid = \"unblock\"\ntext = \"Do not leave delegated work parked.\"\n\
                       [line.sensor]\nkind = \"question_latency\"\nsetpoint = \"12h\"\n";

fn backdate(path: &Path, key: &str, hours: i64) {
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    raw[key] = json!((chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339());
    std::fs::write(path, raw.to_string()).unwrap();
}

/// Stage a draft and back-date it by `hours`.
fn draft(outbox: &OutboxStore, hours: i64) -> String {
    let item = outbox
        .stage_by_harness("mail_send", json!({"to": "someone@example.org"}))
        .unwrap();
    if hours > 0 {
        backdate(
            &outbox.root().join(format!("{}.json", item.id)),
            "created_at",
            hours,
        );
    }
    item.id
}

/// The fixture model: one prose answer, ending the run.
async fn fixture_model() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|body: axum::extract::Json<Value>| async move {
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

/// The smoke-test session's recorded run, as the binary wrote it.
fn recorded_run(home: &Path) -> mecha_core::homeostat::Homeostat {
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

fn store(stores: &[StoreGuilt], which: Store) -> &StoreGuilt {
    stores
        .iter()
        .find(|s| s.store == which)
        .unwrap_or_else(|| panic!("no {which:?} store: {stores:#?}"))
}

fn guilt(s: &StoreGuilt, id: &str) -> f32 {
    s.items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("no commitment `{id}`: {s:#?}"))
        .guilt
        .unwrap_or_else(|| panic!("`{id}` has no guilt"))
}

/// Within a thousandth: the run records a few seconds after the fixture
/// is written, which moves each excess by far less than that.
fn close(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 1e-3, "{what}: {got} vs {want}");
}

#[tokio::test]
async fn each_pending_commitment_has_its_own_guilt_and_the_readout_is_the_maximum() {
    let root = Root(std::env::temp_dir().join(format!("mecha-guilt-{}", Session::new_id())));
    let home = root.0.join("home");
    let work = root.0.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(home.join("charter.toml"), CHARTER).unwrap();

    // Three drafts: four days, two days and minutes old.
    let outbox = OutboxStore::open(home.join("outbox")).unwrap();
    let oldest = draft(&outbox, 96);
    let older = draft(&outbox, 48);
    let fresh = draft(&outbox, 0);
    // A question parked a day and a half ago.
    let questions = mecha_core::questions::QuestionStore::open(home.join("questions")).unwrap();
    let q = questions
        .park(
            "Which room should the reading group use?",
            Vec::new(),
            "20260920T000000-fixture",
            None,
            None,
            mecha_core::agent::Taint::default(),
            None,
        )
        .unwrap();
    backdate(
        &home.join("questions").join(format!("{}.json", q.id)),
        "asked_at",
        36,
    );
    // A request that arrived six days ago and awaits triage, and one parked
    // on the stranger (`needs_info`) — owed by nobody here, so no
    // commitment, however old.
    let requests = home.join("requests");
    std::fs::create_dir_all(&requests).unwrap();
    for (seq, state) in [(7, "extracted"), (8, "needs_info")] {
        let at = (chrono::Utc::now() - chrono::Duration::hours(144)).to_rfc3339();
        std::fs::write(
            requests.join(format!("{seq:010}-meeting.json")),
            json!({
                "seq": seq, "type_id": "meeting", "state": state,
                "created_at": at, "drained_at": at,
                "valid": true, "values": {}, "free_text": [],
            })
            .to_string(),
        )
        .unwrap();
    }

    let (base_url, server) = fixture_model().await;
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

    let out = mecha(&home, &work, &["run", "What is waiting on me?", "--json"]).await;
    server.abort();
    assert!(
        out.status.success(),
        "the run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let h = recorded_run(&home);
    let stores = h.commitments.clone().expect("a per-commitment record");

    // The drafts: the top line's 24h, full weight. Four days is 72h past a
    // day (3/4), two days 24h past (1/2), minutes nothing.
    let drafts = store(&stores, Store::Outbox);
    assert_eq!(drafts.line.as_deref(), Some("replies"));
    assert_eq!((drafts.patience.as_str(), drafts.weight), ("24h", 1.0));
    assert_eq!(drafts.waiting, Some(3));
    close(guilt(drafts, &oldest), 0.75, "the four-day draft");
    close(guilt(drafts, &older), 0.5, "the two-day draft");
    assert_eq!(guilt(drafts, &fresh), 0.0, "a fresh draft owes nothing");

    // The question: the third line (weight 1/3), 12h. 36h is 24h past:
    // 2/3 of a third.
    let parked = store(&stores, Store::Questions);
    assert_eq!(parked.line.as_deref(), Some("unblock"));
    close(guilt(parked, &q.id), 2.0 / 9.0, "the parked question");

    // The request: no line, so the doctor's 72h and a rank below all three
    // lines (weight 1/4). Six days is 72h past 72h: a half of a quarter.
    let door = store(&stores, Store::Requests);
    assert_eq!(
        (door.line.as_deref(), door.patience.as_str()),
        (None, "72h")
    );
    assert_eq!(door.weight, 0.25);
    assert_eq!(door.waiting, Some(1), "the needs_info request owes nothing");
    close(guilt(door, "7"), 0.125, "the waiting request");

    // Each pending commitment has its own value, and the readout on the
    // homeostat is exactly the largest of them.
    let values: Vec<f32> = stores
        .iter()
        .flat_map(|s| &s.items)
        .map(|i| i.guilt.expect("every commitment here is dated"))
        .collect();
    assert_eq!(values.len(), 5, "{stores:#?}");
    let max = values.iter().copied().fold(0.0, f32::max);
    assert_eq!(h.anticipated_guilt, Some(max));
    assert_eq!(max, guilt(drafts, &oldest));
    assert_eq!(h.guilt_after_relief, None, "the relief reading is retired");
}
