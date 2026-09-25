//! Row 2a-2's acceptance through the real binary (`docs/APPRAISAL-WIRING-
//! DESIGN.md` I1, R18, R19, R25): `mecha distill` over fixture sessions
//! asks each session's appraisal in a follow-up turn on the episode call's
//! own conversation, and records it through the store's one door.
//!
//! - a clean session's appraisal is readable through the clean door, and a
//!   tainted one's is stored, labelled, and never served clean;
//! - a malformed reply stores nothing and is counted;
//! - a past clean appraisal of the same situation and goal reaches the next
//!   session's input, and a tainted one never does;
//! - the follow-up the server received opens with the episode call's bytes;
//! - a re-run does not append a second appraisal of any session, and pays
//!   no follow-up call for one already on record;
//! - `mecha sessions appraise <id>` shows the owner the prose with its
//!   taint label.
//!
//! No network and no real model: the provider is a loopback fixture that
//! keeps every request body, and the graph is `eval/fixtures/board_server.py`.
//! Fictional cast only.

use mecha_core::agent::Taint;
use mecha_core::appraisal_store::{AppraisalStore, ExpectedAct, Pointer};
use mecha_core::goal::GoalRef;
use mecha_core::message::{Block, Message};
use mecha_core::session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta};
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

const EPISODE: &str = "Riley Park asked when the budget review is; it moved to Thursday.";

/// The fixture model. The episode call (system + one user turn) gets an
/// episode; the follow-up (four messages) gets an appraisal naming the
/// session's marker word, citing the owner's own turn — or junk, for the
/// session whose owner turn says MALFORMED.
async fn fixture_model() -> (String, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let kept = Arc::clone(&seen);
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: String| {
            let kept = Arc::clone(&kept);
            async move {
                let parsed: Value = serde_json::from_str(&body).unwrap_or_default();
                kept.lock().unwrap().push(parsed.clone());
                let messages = parsed["messages"].as_array().cloned().unwrap_or_default();
                let transcript = messages
                    .get(1)
                    .and_then(|m| m["content"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = if messages.len() <= 2 {
                    json!({"skip": false, "episode": EPISODE, "corrections": []}).to_string()
                } else if transcript.contains("MALFORMED") {
                    "I would rather describe it in prose.".to_string()
                } else {
                    let marker = ["alpha", "bravo", "charlie"]
                        .into_iter()
                        .find(|m| transcript.contains(&format!("review date for {m}")))
                        .unwrap_or("nobody");
                    json!({
                        "interpretation": format!("Interpretation of {marker}: the run answered from the owner's own mail."),
                        "judgments": [
                            {"goal": "task:task-1", "bearing": "good", "because": [0, 0]},
                            {"goal": "task:t-ghost", "bearing": "bad"}
                        ],
                        "claims": [
                            {"statement": "The owner asked for the date",
                             "pointer": "turn:0", "quote": "check the budget review date"},
                            {"statement": "A call that never happened",
                             "pointer": "result:t9", "quote": "the room is confirmed"}
                        ],
                        "prediction": "The owner will ask for the agenda next.",
                        "expected_act": "no_act",
                        "lessons": ["Quote the date from the mail."]
                    })
                    .to_string()
                };
                let reply = json!({
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": text},
                                 "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 1000, "completion_tokens": 40,
                              "prompt_tokens_details": {"cached_tokens": if messages.len() > 2 { 900 } else { 0 }}}
                });
                ([("content-type", "application/json")], reply.to_string())
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), seen, server)
}

/// A delegated session on task-1 that searched the owner's mail, as a
/// front-end records one. `marker` names it in the owner's turn; `age`
/// orders the sessions.
fn session(home: &Path, marker: &str, untrusted: bool, age_mins: i64) -> String {
    let session = Session::create(
        &home.join("sessions"),
        SessionMeta {
            id: Session::new_id(),
            created_at: chrono::Utc::now() - chrono::Duration::minutes(age_mins),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: home.to_path_buf(),
            title: None,
            kind: Some(SessionKind::Task),
        },
    )
    .unwrap();
    session
        .append(&Record::Config(RunConfig {
            tools: vec!["mail_search".into()],
            rules_surface: Some(SessionKind::Task),
            ..Default::default()
        }))
        .unwrap();
    session
        .append(&Record::GoalAnchor {
            goal: Some(GoalRef::Task("task-1".into())),
        })
        .unwrap();
    for m in [
        Message::user(format!("Please check the budget review date for {marker}.")),
        Message::assistant(vec![Block::ToolUse {
            id: "t1".into(),
            name: "mail_search".into(),
            input: json!({"query": "budget review"}),
        }]),
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: "From Dana Rowe: the budget review moved to Thursday at 10.".into(),
            is_error: false,
        }]),
        Message::assistant(vec![Block::text("Thursday at 10.")]),
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
            untrusted,
            private: true,
        }))
        .unwrap();
    session.meta.id.clone()
}

fn seed(root: &Path, base_url: &str) -> PathBuf {
    let home = root.join("home");
    let board = root.join("board");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&board).unwrap();
    std::fs::create_dir_all(root.join("work")).unwrap();
    std::fs::write(
        board.join("board.json"),
        json!({"v": 1, "next": 2, "tasks": [
            {"id": "task-1", "name": "Confirm the budget review with Riley Park", "status": "next"}
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n\
             [providers.fixture]\nkind = \"local\"\nbase_url = \"{base_url}\"\n\
             model = \"fixture\"\nmax_retries = 0\n\
             [sandbox]\nkind = \"none\"\n\
             [[mcp]]\nname = \"graph\"\ncommand = \"python3\"\nargs = [{:?}]\n\
             prefix_tools = false\nsandbox = false\n\
             [mcp.env]\nMECHA_FIXTURE_DIR = {:?}\n",
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

fn ok(out: &std::process::Output, what: &str) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "{what} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The follow-up requests the fixture saw, from `from` on.
fn follow_ups(seen: &Arc<Mutex<Vec<Value>>>, from: usize) -> Vec<Value> {
    seen.lock().unwrap()[from..]
        .iter()
        .filter(|b| b["messages"].as_array().is_some_and(|m| m.len() > 2))
        .cloned()
        .collect()
}

#[tokio::test]
async fn distill_appraises_each_session_once_behind_the_right_door() {
    if !python3() {
        return;
    }
    let root =
        Root(std::env::temp_dir().join(format!("mecha-distill-appraises-{}", Session::new_id())));
    let (base_url, seen, server) = fixture_model().await;
    let home = seed(&root.0, &base_url);
    let work = root.0.join("work");

    let alpha = session(&home, "alpha", false, 30);
    let charlie = session(&home, "charlie", true, 20);
    let broken = session(&home, "MALFORMED", false, 10);

    // First night: a clean, a tainted and a malformed session.
    let first = ok(&mecha(&home, &work, &["distill"]).await, "distill");
    assert!(
        first.contains("appraised 2 session(s) (1 clean, 1 not clean"),
        "{first}"
    );
    assert!(
        first.contains("1 malformed, nothing stored (no_json 1)"),
        "{first}"
    );
    assert!(
        first.contains(
            "over 3 follow-up call(s), 2700 of 3000 prompt token(s) from the server's cache"
        ),
        "{first}"
    );

    let store = AppraisalStore::open(home.join("appraisals")).unwrap();
    let clean = store.clean().unwrap();
    assert_eq!(clean.appraisals.len(), 1, "{:?}", clean.appraisals);
    assert_eq!(clean.withheld, 1);
    let a = &clean.appraisals[0];
    assert_eq!(a.session_id, alpha, "never {charlie}");
    assert!(a.interpretation.starts_with("Interpretation of alpha"));
    assert_eq!(
        a.claims.len(),
        1,
        "the ungrounded claim is dropped before storage"
    );
    assert_eq!(a.claims[0].pointer, Pointer::Turn(0));
    assert_eq!(a.judgments[0].goal, Some(GoalRef::Task("task-1".into())));
    assert_eq!(a.judgments[0].because, vec![0], "deduplicated");
    assert_eq!(a.judgments[1].goal, None, "t-ghost is not on the board");
    assert_eq!(a.goals_unresolved, 1);
    assert_eq!(a.expected_act, Some(ExpectedAct::NoAct));
    assert_eq!(a.model, "fixture");
    let (owner, _) = store.for_owner().unwrap();
    assert_eq!(owner.len(), 2, "the malformed reply stored nothing");
    assert!(owner
        .iter()
        .any(|r| r.session_id == charlie && !r.is_clean()));
    assert!(owner.iter().all(|r| r.session_id != broken));

    // The follow-up opens with the episode call's bytes, as the server saw them.
    {
        let bodies = seen.lock().unwrap().clone();
        let episode = bodies
            .iter()
            .find(|b| b["messages"].as_array().is_some_and(|m| m.len() == 2))
            .unwrap();
        let follow = follow_ups(&seen, 0)
            .into_iter()
            .find(|b| b["messages"][1]["content"] == episode["messages"][1]["content"])
            .expect("a follow-up on the same transcript");
        assert_eq!(follow["messages"][0], episode["messages"][0]);
        assert_eq!(follow["messages"][1], episode["messages"][1]);
        assert_eq!(follow["messages"][2]["role"], "assistant");
        assert!(follow["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains(EPISODE));
    }

    // The episode leg is unchanged: every session's episode reached the graph.
    let staged = std::fs::read_to_string(root.0.join("board").join("staged.jsonl")).unwrap();
    assert_eq!(staged.lines().count(), 3, "{staged}");

    // Second night: a later session in the same situation and goal is shown
    // the clean appraisal of alpha, and never the tainted one of charlie.
    let bravo = session(&home, "bravo", false, 0);
    let before = seen.lock().unwrap().len();
    let second = ok(&mecha(&home, &work, &["distill"]).await, "distill");
    assert!(
        second.contains("appraised 1 session(s) (1 clean, 0 not clean"),
        "{second}"
    );
    let asked = follow_ups(&seen, before);
    assert_eq!(asked.len(), 1);
    let inputs = asked[0]["messages"][3]["content"].as_str().unwrap();
    assert!(inputs.contains("Interpretation of alpha"), "{inputs}");
    assert!(!inputs.contains("Interpretation of charlie"), "{inputs}");
    assert!(inputs.contains(
        "[turn:0] the owner's own words:\nPlease check the budget review date for bravo."
    ));
    assert!(inputs.contains("Goal pointers a judgment may name: task:task-1."));
    assert!(AppraisalStore::open(home.join("appraisals"))
        .unwrap()
        .on_record(&bravo)
        .unwrap()
        .is_some());

    // A re-run of every session — the distill ledger lost — appends no
    // second appraisal and pays no follow-up for a session on record; the
    // malformed one is asked again, and again stores nothing.
    std::fs::remove_file(home.join("learning").join("distilled.jsonl")).unwrap();
    let before = seen.lock().unwrap().len();
    let third = ok(&mecha(&home, &work, &["distill"]).await, "distill");
    assert!(third.contains("appraised 0 session(s)"), "{third}");
    assert!(third.contains("3 already on record"), "{third}");
    assert_eq!(
        follow_ups(&seen, before).len(),
        1,
        "only the unappraised session is asked"
    );
    let (owner, _) = AppraisalStore::open(home.join("appraisals"))
        .unwrap()
        .for_owner()
        .unwrap();
    assert_eq!(owner.len(), 3, "one appraisal per session");
    server.abort();

    // The owner's readout: the prose, with its taint label.
    let shown = ok(
        &mecha(
            &home,
            &work,
            &["sessions", "appraise", &charlie[..charlie.len() - 2]],
        )
        .await,
        "sessions appraise <id>",
    );
    assert!(
        shown.contains("NOT CLEAN — third-party content entered the run"),
        "{shown}"
    );
    assert!(shown.contains("Interpretation of charlie"), "{shown}");
    assert!(
        shown.contains("claim 1: The owner asked for the date — turn:0"),
        "{shown}"
    );
    let json = ok(
        &mecha(&home, &work, &["sessions", "appraise", &alpha, "--json"]).await,
        "sessions appraise <id> --json",
    );
    let rows: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["clean"], true);
    assert_eq!(rows[0]["expected_act"], "no_act");
    let every = ok(
        &mecha(&home, &work, &["sessions", "appraise", "--text"]).await,
        "sessions appraise --text",
    );
    assert_eq!(every.matches("text appraisal apr-").count(), 3, "{every}");
}
