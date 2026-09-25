//! Row 1h's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` B1): on fixture runs of each kind — a delegated task
//! (`mecha tasks work`), a trigger (`mecha trigger run`) and a web chat
//! turn (`mecha serve`) — over a seeded board, charter and backlog, the
//! recorded situation brief carries every field, and no brief text appears
//! in any request the provider received.
//!
//! No network and no real model: the provider is a loopback fixture that
//! answers `/v1/chat/completions`, answers `/slots` as a four-slot
//! llama-server with one slot busy, and keeps every request body it was
//! sent so the scan reads what actually left the harness. The board is
//! `eval/fixtures/board_server.py`. Everything else — a stale draft, the
//! owner's quiet hours, a held seat, another delegated run in flight, a
//! voice turn a minute ago — is seeded as files under the fixture home.
//! Fictional cast only.

use mecha_core::brief::*;
use mecha_core::session::Session;
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

/// The first line watches the outbox by age, so the stale draft is a
/// commitment owed against the owner's own setpoint.
const CHARTER: &str = "[[line]]\nid = \"replies\"\ntext = \"Answer the people waiting on me.\"\n\
                       [line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n\
                       [[line]]\nid = \"craft\"\ntext = \"Leave work better than you found it.\"\n";

/// Another delegated task, running in this test's own (live) process.
const ELSEWHERE: &str = "task-elsewhere";

fn board_server() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("eval")
        .join("fixtures")
        .join("board_server.py")
}

fn python3() -> bool {
    let ok = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        assert!(
            std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
            "python3 is required for the fixture board server"
        );
        eprintln!("skipping: no python3 for the fixture board server");
    }
    ok
}

/// The fixture model: one prose answer per request, every request kept,
/// and `/slots` answered as llama-server answers it.
async fn fixture_model() -> (String, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let kept = Arc::clone(&seen);
    let app = axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(move |body: String| {
                let kept = Arc::clone(&kept);
                async move {
                    kept.lock().unwrap().push(body.clone());
                    let parsed: Value = serde_json::from_str(&body).unwrap_or_default();
                    let text = "Nothing else needs doing.";
                    if parsed.get("stream").and_then(Value::as_bool) == Some(true) {
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
        )
        .route(
            "/slots",
            axum::routing::get(|| async {
                (
                    [("content-type", "application/json")],
                    json!([
                        {"id": 0, "is_processing": true},
                        {"id": 1, "is_processing": false},
                        {"id": 2, "is_processing": false},
                        {"id": 3, "is_processing": false}
                    ])
                    .to_string(),
                )
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen, server)
}

/// A fixture home: charter, backlog, board, a trigger that serves a line,
/// the owner's quiet hours, a held seat, a run in flight and a voice turn.
fn seed(root: &Path, base_url: &str) -> PathBuf {
    let home = root.join("home");
    let board = root.join("board");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&board).unwrap();
    std::fs::create_dir_all(root.join("work")).unwrap();
    std::fs::write(home.join("charter.toml"), CHARTER).unwrap();

    // The backlog: a draft four days old, past the line's 24h.
    let outbox = mecha_core::outbox::OutboxStore::open(home.join("outbox")).unwrap();
    let stale = outbox
        .stage_by_harness("mail_send", json!({"to": "dana@example.org"}))
        .unwrap();
    let path = home.join("outbox").join(format!("{}.json", stale.id));
    let mut raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    raw["created_at"] = json!((chrono::Utc::now() - chrono::Duration::days(4)).to_rfc3339());
    std::fs::write(&path, raw.to_string()).unwrap();

    // The board: the delegated task under a project, one overdue, one due
    // soon, one the agent already holds, one done. The server caps a list
    // at four rows (`MECHA_FIXTURE_BOARD_CAP`, below), so a read that
    // includes closed rows comes back `truncated` over the done one alone
    // while the open-only read the brief makes is whole — the delegated
    // door once reused the first and recorded its board as a floor.
    std::fs::write(
        board.join("board.json"),
        json!({"v": 1, "next": 6, "tasks": [
            {"id": "task-1", "name": "Write the quarterly summary for Morgan Reyes",
             "status": "next", "project": "Aurora", "project_id": "project-aurora",
             "due_in_days": 5},
            {"id": "task-2", "name": "Reply to Alex Kim about the budget",
             "status": "next", "due_in_days": -3},
            {"id": "task-3", "name": "Book the seminar room",
             "status": "inbox", "due_in_days": 2, "project": "Aurora",
             "project_id": "project-aurora"},
            {"id": "task-4", "name": "Collect the lab reports",
             "status": "waiting", "waiting_on": "mecha"},
            {"id": "task-5", "name": "Old and finished", "status": "done"}
        ]})
        .to_string(),
    )
    .unwrap();

    // A trigger serving the second line.
    std::fs::create_dir_all(home.join("triggers")).unwrap();
    std::fs::write(
        home.join("triggers/digest.toml"),
        "schedule = \"0 7 * * *\"\nprompt = \"Summarise what is waiting on me.\"\n\
         serves = \"charter:craft\"\n",
    )
    .unwrap();

    // The owner's quiet hours.
    std::fs::create_dir_all(home.join("workflows")).unwrap();
    std::fs::write(
        home.join("workflows/attention.toml"),
        "timezone = \"America/New_York\"\nquiet_start = 22\nquiet_end = 7\ndigest_hour = 8\n",
    )
    .unwrap();

    // A held seat and a run in flight, both in this (live) process, and a
    // voice turn a minute ago.
    let me = std::process::id();
    std::fs::create_dir_all(home.join("permits")).unwrap();
    std::fs::write(
        home.join("permits/elsewhere.permit"),
        json!({"pid": me, "taken_at": chrono::Utc::now(), "what": ELSEWHERE}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(home.join("taskruns")).unwrap();
    std::fs::write(
        home.join(format!("taskruns/{ELSEWHERE}.running")),
        json!({"pid": me, "started_at": chrono::Utc::now()}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(home.join("runs")).unwrap();
    std::fs::write(
        home.join("runs/voice.json"),
        json!({"pid": me, "last_turn_at": chrono::Utc::now() - chrono::Duration::seconds(60)})
            .to_string(),
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
             [sandbox]\nkind = \"none\"\n\
             [outbox]\ntools = [\"mail_send\"]\n\
             [[mcp]]\nname = \"graph\"\ncommand = \"python3\"\nargs = [{:?}]\n\
             prefix_tools = false\nsandbox = false\n\
             [mcp.env]\nMECHA_FIXTURE_DIR = {:?}\nMECHA_FIXTURE_BOARD_CAP = \"4\"\n",
            board_server().display().to_string(),
            board.display().to_string(),
        ),
    )
    .unwrap();
    home
}

async fn mecha(home: &Path, work: &Path, args: &[&str]) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(90),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("MECHA_TRIGGERS_DIR")
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

fn ok(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The brief recorded by the session whose title starts with `prefix`.
fn recorded_brief(home: &Path, prefix: &str) -> SituationBrief {
    let listed = Session::list(&home.join("sessions")).unwrap();
    let (_, path) = listed
        .iter()
        .find(|(meta, _)| meta.title.as_deref().is_some_and(|t| t.starts_with(prefix)))
        .unwrap_or_else(|| panic!("no `{prefix}` session: {listed:?}"));
    *Session::outcomes_attributed(path)
        .unwrap()
        .pop()
        .expect("an outcome row")
        .2
        .brief
        .unwrap_or_else(|| panic!("the `{prefix}` run recorded no brief"))
}

/// What every run in this fixture home should read, whatever its kind.
fn the_shared_situation(b: &SituationBrief, what: &str) {
    for (field, state) in b.fields() {
        assert_eq!(
            state,
            FieldState::Known,
            "{what}: `{field}` did not read: {b:#?}"
        );
    }
    let Some(Board::Read(board)) = &b.board else {
        panic!("{what}: {:?}", b.board)
    };
    assert_eq!(board.open, 4, "{what}: the done task is not open");
    assert_eq!(board.overdue_ids, vec!["task-2".to_string()], "{what}");
    assert!(board.due_soon_ids.contains(&"task-3".to_string()), "{what}");
    assert!(!board.truncated);

    let Some(Commitments::Read { stores, withdrawn }) = &b.commitments else {
        panic!("{what}: {:?}", b.commitments)
    };
    assert!(withdrawn.is_empty(), "{what}");
    let drafts = stores
        .iter()
        .find(|s| s.store == mecha_core::guilt::Store::Outbox)
        .expect("the outbox store");
    assert_eq!(drafts.line.as_deref(), Some("replies"), "{what}");
    assert!(
        drafts.owed >= 1,
        "{what}: the four-day draft is owed: {drafts:?}"
    );

    let Some(LocalTime {
        zone: Zone::Set { name, .. },
        quiet: Quiet::Set { start, end, .. },
    }) = &b.time
    else {
        panic!("{what}: {:?}", b.time)
    };
    assert_eq!((name.as_str(), *start, *end), ("America/New_York", 22, 7));

    let Some(Seats::Read { held, holders, .. }) = &b.seats else {
        panic!("{what}: {:?}", b.seats)
    };
    assert!(
        *held >= 1 && holders.contains(&ELSEWHERE.to_string()),
        "{what}"
    );
    let Some(Runs {
        tasks: Flight::Read { others, .. },
        ..
    }) = &b.runs
    else {
        panic!("{what}: {:?}", b.runs)
    };
    assert!(
        others.contains(&ELSEWHERE.to_string()),
        "{what}: {others:?}"
    );
    assert_eq!(b.slots, Some(Slots::Read { total: 4, busy: 1 }), "{what}");
    assert!(
        matches!(b.voice, Some(Voice::InCall { .. })),
        "{what}: {:?}",
        b.voice
    );
}

/// Nothing of the brief reached the model: no pointer it keeps, no row's
/// name it dropped, none of its record keys.
fn no_brief_in_requests(seen: &[String], what: &str) {
    assert!(!seen.is_empty(), "{what}: the fixture model saw no request");
    for body in seen {
        for needle in [
            ELSEWHERE,
            "project-aurora",
            "task-2",
            "task-3",
            "overdue_ids",
            "due_soon_ids",
            "waiting_on_agent",
            "in_call",
            "no_turn_seen",
            "assembled_at",
            "last_turn_secs",
        ] {
            assert!(
                !body.contains(needle),
                "{what}: `{needle}` reached a provider request: {body}"
            );
        }
    }
}

#[tokio::test]
async fn a_delegated_run_records_every_field_and_sends_none() {
    if !python3() {
        return;
    }
    let root = Root(std::env::temp_dir().join(format!("mecha-brief-task-{}", Session::new_id())));
    let (base_url, seen, server) = fixture_model().await;
    let home = seed(&root.0, &base_url);
    let out = mecha(
        &home,
        &root.0.join("work"),
        &["tasks", "work", "task-1", "--unattended"],
    )
    .await;
    server.abort();
    ok(&out, "tasks work");

    let b = recorded_brief(&home, "task: ");
    the_shared_situation(&b, "delegated");
    // The chain: the task, its project off the board row (two open tasks
    // under it), and no charter link — the board carries none.
    assert_eq!(
        b.goal,
        Some(GoalChain::Anchored {
            anchor: "task:task-1".into(),
            project: Tier::Known {
                id: "project-aurora".into(),
                open: Some(2)
            },
            charter: Lines::Unlinked,
        })
    );
    let Some(Board::Read(board)) = &b.board else {
        unreachable!()
    };
    // The brief's own board read, after `tasks work` moved the task to the
    // agent: the board the run is handed.
    assert!(
        matches!(&board.own, OwnTask::Found { status: Some(s), overdue: false, .. } if s == "waiting"),
        "{:?}",
        board.own
    );
    // This run's own seat is held and named; its own marker is not
    // counted as another run.
    let Some(Seats::Read { holders, .. }) = &b.seats else {
        unreachable!()
    };
    assert!(holders.contains(&"task-1".to_string()), "{holders:?}");
    let Some(Runs {
        tasks: Flight::Read { others, .. },
        ..
    }) = &b.runs
    else {
        unreachable!()
    };
    assert!(!others.contains(&"task-1".to_string()), "{others:?}");
    assert_eq!(b.budget.as_ref().map(|x| x.max_turns), Some(200));
    no_brief_in_requests(&seen.lock().unwrap(), "delegated");
}

#[tokio::test]
async fn a_trigger_run_records_every_field_and_sends_none() {
    if !python3() {
        return;
    }
    let root =
        Root(std::env::temp_dir().join(format!("mecha-brief-trigger-{}", Session::new_id())));
    let (base_url, seen, server) = fixture_model().await;
    let home = seed(&root.0, &base_url);
    let out = mecha(&home, &root.0.join("work"), &["trigger", "run", "digest"]).await;
    server.abort();
    ok(&out, "trigger run");

    let b = recorded_brief(&home, "trigger: ");
    the_shared_situation(&b, "trigger");
    // The chain: the trigger, no project tier, and the line its file
    // serves, ranked second in the charter.
    assert_eq!(
        b.goal,
        Some(GoalChain::Anchored {
            anchor: "trigger:digest".into(),
            project: Tier::Absent,
            charter: Lines::Named {
                lines: vec![ServedLine {
                    id: "craft".into(),
                    rank: Some(1),
                    in_charter: Some(true)
                }]
            },
        })
    );
    let Some(Board::Read(board)) = &b.board else {
        unreachable!()
    };
    assert_eq!(board.own, OwnTask::NotATask);
    no_brief_in_requests(&seen.lock().unwrap(), "trigger");
}

/// The web door, through a real `mecha serve`: a chat turn records a brief
/// with no anchor (recorded as such, never an empty chain) — and the
/// commitments are this turn's, not the ones waiting when the daemon
/// started: a draft staged after `serve` came up is in the brief.
#[tokio::test]
async fn a_web_run_records_every_field_and_sends_none() {
    if !python3() {
        return;
    }
    let root = Root(std::env::temp_dir().join(format!("mecha-brief-web-{}", Session::new_id())));
    let (base_url, seen, server) = fixture_model().await;
    let home = seed(&root.0, &base_url);
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let voice_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let log = std::fs::File::create(root.0.join("serve.log")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args([
            "serve",
            "--port",
            &port.to_string(),
            "--voice-port",
            &voice_port.to_string(),
            "--owner-login",
            "test@example.com",
        ])
        .env("MECHA_HOME", &home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("MECHA_SESSION_DIR")
        .env_remove("MECHA_TRIGGERS_DIR")
        .env_remove("MECHA_FIXTURE_CLOCK")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .current_dir(root.0.join("work"))
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = reqwest::Client::builder()
        .default_headers(
            [
                (
                    "Tailscale-User-Login".parse().unwrap(),
                    "test@example.com".parse().unwrap(),
                ),
                ("X-Mecha-Request".parse().unwrap(), "1".parse().unwrap()),
            ]
            .into_iter()
            .collect(),
        )
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if client.get(format!("{base}/api/ping")).send().await.is_ok() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(root.0.join("serve.log")).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();

    // Staged after the daemon started: a brief read off the conditions
    // `serve` sampled at startup would not hold it.
    let late = mecha_core::outbox::OutboxStore::open(home.join("outbox"))
        .unwrap()
        .stage_by_harness("mail_send", json!({"to": "sam@example.org"}))
        .unwrap();

    let opened = client
        .post(format!("{base}/api/chat/brief"))
        .send()
        .await
        .unwrap();
    assert!(
        opened.status().is_success(),
        "{}",
        opened.text().await.unwrap()
    );
    let mut events = client
        .get(format!("{base}/api/chat/brief/events"))
        .send()
        .await
        .unwrap();
    let sent = client
        .post(format!("{base}/api/chat/brief/send"))
        .json(&json!({"text": "What is waiting on me?", "request_id": "r-1"}))
        .send()
        .await
        .unwrap();
    assert!(sent.status().is_success());
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut seen = String::new();
        while let Some(chunk) = events.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"type\":\"done\"") {
                return;
            }
        }
        panic!("the stream ended before the run was done: {seen}");
    })
    .await
    .unwrap();
    // The outcome is recorded before `done` is broadcast.
    let _ = child.start_kill();
    let _ = child.wait().await;
    server.abort();

    let b = recorded_brief(&home, "web: ");
    the_shared_situation(&b, "web");
    assert_eq!(b.goal, Some(GoalChain::NoAnchor));
    let Some(Commitments::Read { stores, .. }) = &b.commitments else {
        unreachable!()
    };
    let drafts = stores
        .iter()
        .find(|s| s.store == mecha_core::guilt::Store::Outbox)
        .unwrap();
    assert_eq!(drafts.waiting, Some(2), "{drafts:?}");
    assert!(
        drafts.items.iter().any(|i| i.id == late.id),
        "a draft staged after `serve` started should be in this turn's brief: {drafts:?}"
    );
    assert_eq!(b.budget.as_ref().map(|x| x.max_turns), Some(40));
    no_brief_in_requests(&seen.lock().unwrap(), "web");
}
