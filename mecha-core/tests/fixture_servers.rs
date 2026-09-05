//! The fixture servers a lifetime home carries in place of the operator's
//! (`eval/fixtures/board_server.py`, `eval/fixtures/mail_server.py`), driven
//! through mecha's own MCP client — the handshake, the tool shapes `mecha
//! tasks` parses, and the one property that makes them fixtures for a
//! *lifetime* rather than for an eval: **state survives the process**. The
//! run child, the principal's verb and the next task each spawn their own
//! server, and the board one of them wrote is the board the next one reads.
//!
//! Skips without `python3`; `MECHA_TEST_REQUIRE_BACKENDS=1` fails instead.

mod support;

use mecha_core::config::McpServerConfig;
use mecha_core::mcp::McpClient;
use mecha_core::sandbox::{Sandbox, SandboxConfig};
use mecha_core::tool::{Tool, ToolCtx};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use support::*;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("eval")
        .join("fixtures")
}

fn server(name: &str, script: &str, store: &Path, prefix: Option<bool>) -> McpServerConfig {
    let mut env = BTreeMap::new();
    env.insert(
        mecha_core::experiment::FIXTURE_DIR_ENV.to_string(),
        store.to_string_lossy().into_owned(),
    );
    McpServerConfig {
        name: name.into(),
        command: "python3".into(),
        args: vec![fixtures_dir().join(script).display().to_string()],
        env,
        prefix_tools: prefix,
        ..Default::default()
    }
}

fn unconfined() -> Sandbox {
    Sandbox::new(SandboxConfig::default())
}

async fn connect(cfg: &McpServerConfig, workspace: &Path) -> Vec<Arc<dyn Tool>> {
    let client = McpClient::connect(cfg, &unconfined(), workspace)
        .await
        .expect("handshake failed");
    // The client owns the process; keep it alive for as long as the tools.
    let tools = client.list_tools().await.expect("tools/list failed");
    // Leak the client on purpose: dropping it kills the server, and the
    // tools hold their own reference to it for the calls below.
    std::mem::forget(client);
    tools
}

fn tool_named(tools: &[Arc<dyn Tool>], name: &str) -> Arc<dyn Tool> {
    tools
        .iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| {
            panic!(
                "no tool named {name}; have {:?}",
                tools.iter().map(|t| t.name()).collect::<Vec<_>>()
            )
        })
        .clone()
}

async fn call(tool: &Arc<dyn Tool>, input: Value, workspace: &Path) -> (bool, String) {
    let ctx = ToolCtx {
        workspace: workspace.to_path_buf(),
        ..Default::default()
    };
    let out = tool
        .call(input, &ctx)
        .await
        .expect("the call itself failed");
    (out.is_error, out.content)
}

fn seed_board(store: &Path) {
    std::fs::create_dir_all(store).unwrap();
    std::fs::write(
        store.join("board.json"),
        json!({
            "v": 1, "next": 1,
            "tasks": [
                {"id": "task-late", "name": "the late one", "status": "next", "due_in_days": -2,
                 "about": [{"name": "Priya Nair", "unreviewed": false}]},
                {"id": "task-soon", "name": "the soon one", "status": "inbox", "due_in_days": 3}
            ]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        store.join("entities.json"),
        json!({
            "entities": {
                "person:priya-nair": {"id": "person:priya-nair", "kind": "person", "name": "Priya Nair",
                                      "summary": "Postdoc.", "keywords": ["priya", "nair"]},
                "project:aurora": {"id": "project:aurora", "kind": "project", "name": "Aurora grant proposal",
                                   "summary": "R01.", "keywords": ["aurora"]}
            },
            "facts": {}, "related": {"person:priya-nair": ["project:aurora"]}, "timeline": {}
        })
        .to_string(),
    )
    .unwrap();
}

fn seed_mail(store: &Path) {
    std::fs::create_dir_all(store).unwrap();
    std::fs::write(
        store.join("mailbox.json"),
        json!({
            "v": 1, "next": 1,
            "accounts": [{"name": "work", "address": "ada.okafor@example.edu", "display_name": "Ada Okafor", "default": true}],
            "threads": [{
                "id": "t-1", "account": "work", "subject": "Aurora aims",
                "messages": [{"id": "m-1", "from_name": "Priya Nair", "from_address": "priya.nair@example.edu",
                              "to": ["ada.okafor@example.edu"], "days_ago": 1, "body": "Can we meet Thursday?"}]
            }]
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        store.join("calendar.json"),
        json!({"v": 1, "next": 1, "events": []}).to_string(),
    )
    .unwrap();
}

/// The board answers in the real server's shapes, refuses what it refuses,
/// and — the fixture's whole point — a second process reads what the first
/// one wrote.
#[tokio::test]
async fn the_board_fixture_persists_across_processes_in_the_real_servers_shapes() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("fixture-board");
    let store = dir.join("store");
    seed_board(&store);
    let cfg = server("graph", "board_server.py", &store, Some(false));

    let tools = connect(&cfg, &dir).await;
    let list = tool_named(&tools, "kg_task_list");
    let (err, text) = call(&list, json!({}), &dir).await;
    assert!(!err, "{text}");
    let board: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(board["v"], 1);
    assert!(
        board["today"].as_str().unwrap().len() == 10,
        "today is a date"
    );
    let items = board["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["id"], "task-late",
        "actionable `next` before `inbox`"
    );
    assert_eq!(
        items[0]["overdue"], true,
        "a seed's due_in_days is resolved against the clock"
    );
    assert_eq!(items[1]["overdue"], false);
    for key in [
        "status",
        "due_at",
        "defer_until",
        "context",
        "project",
        "waiting_on",
        "about",
        "session",
        "completed_at",
        "captured_from",
    ] {
        assert!(
            items[0].get(key).is_some(),
            "the row carries `{key}`, which `mecha tasks` reads"
        );
    }

    let create = tool_named(&tools, "kg_task_create");
    let (err, text) = call(&create, json!({"name": "call Priya", "due": "+1d", "about": ["Priya Nair"], "project": "Aurora grant proposal"}), &dir).await;
    assert!(!err, "{text}");
    let created: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(created["status"], "created");
    let id = created["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("task-"));
    assert_eq!(created["about"][0]["name"], "Priya Nair");
    let (err, text) = call(&create, json!({"name": "x", "project": "Nobody"}), &dir).await;
    assert!(
        err && text.contains("no node"),
        "an unknown project is an error, never an implicit node: {text}"
    );

    let update = tool_named(&tools, "kg_task_update");
    let (err, text) = call(
        &update,
        json!({"task": "task-late", "session": "s-42", "status": "done"}),
        &dir,
    )
    .await;
    assert!(!err, "{text}");
    let updated: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(updated["status"], "updated");
    assert_eq!(updated["task"]["session"], "s-42");
    assert!(
        updated["task"]["completed_at"].is_string(),
        "done stamps completed_at"
    );
    let (err, text) = call(
        &update,
        json!({"task": "task-nope", "status": "done"}),
        &dir,
    )
    .await;
    assert!(err && text.contains("no such task"), "{text}");
    let (err, _) = call(
        &update,
        json!({"task": "task-soon", "status": "later"}),
        &dir,
    )
    .await;
    assert!(err, "an unknown status is refused");

    // A second process — the next task's run, or the principal's verb.
    let again = connect(&cfg, &dir).await;
    let list = tool_named(&again, "kg_task_list");
    let (_, text) = call(&list, json!({}), &dir).await;
    let open: Value = serde_json::from_str(&text).unwrap();
    let ids: Vec<&str> = open["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&id.as_str()),
        "the created task is there: {ids:?}"
    );
    assert!(
        !ids.contains(&"task-late"),
        "the closed one is off the open board"
    );
    let (_, text) = call(&list, json!({"include_closed": true}), &dir).await;
    let all: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(all["items"].as_array().unwrap().len(), 3);
    let (_, text) = call(
        &list,
        json!({"entity": "Priya Nair", "include_closed": true}),
        &dir,
    )
    .await;
    let priya: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        priya["items"].as_array().unwrap().len(),
        2,
        "about-association survives completion"
    );
    let (err, text) = call(&list, json!({"entity": "Nadia"}), &dir).await;
    assert!(
        err && text.contains("no node matches"),
        "an unknown entity is an error, not an empty board: {text}"
    );

    // The graph reads and the staged write.
    let search = tool_named(&again, "kg_search");
    let (_, text) = call(&search, json!({"query": "priya"}), &dir).await;
    assert!(text.contains("person:priya-nair"));
    let upsert = tool_named(&again, "kg_upsert");
    let (err, text) = call(
        &upsert,
        json!({"kind": "fact", "content": "Ada likes Thursdays", "source": "mail"}),
        &dir,
    )
    .await;
    assert!(!err && text.contains("staged"), "{text}");
    let staged = std::fs::read_to_string(store.join("staged.jsonl")).unwrap();
    assert_eq!(
        staged.lines().count(),
        1,
        "the write is recorded, never entered"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A send lands in the store and nowhere else, in the real server's words;
/// the reply threads; the next process sees both.
#[tokio::test]
async fn the_mail_fixture_records_every_send_and_delivers_nothing() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("fixture-mail");
    let store = dir.join("store");
    seed_mail(&store);
    let cfg = server("mail", "mail_server.py", &store, None);

    let tools = connect(&cfg, &dir).await;
    let send = tool_named(&tools, "mail__mail_send");
    assert!(
        send.capabilities().external_send,
        "openWorldHint makes a send a sink"
    );
    let search = tool_named(&tools, "mail__mail_search");
    assert!(search.read_only());

    let (err, text) = call(&search, json!({"query": "from:priya thursday"}), &dir).await;
    assert!(!err, "{text}");
    let rows: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(rows[0]["thread_id"], "t-1");
    assert_eq!(rows[0]["account"], "work");
    assert_eq!(rows[0]["from"], "Priya Nair <priya.nair@example.edu>");
    assert_eq!(rows[0]["unread"], true);

    let reply = tool_named(&tools, "mail__mail_reply");
    let (err, text) = call(
        &reply,
        json!({"thread_id": "t-1", "body_markdown": "Thursday works."}),
        &dir,
    )
    .await;
    assert!(!err, "{text}");
    assert!(
        text.starts_with(
            "replied (message id fx-0001) in thread t-1 from `work` to priya.nair@example.edu"
        ),
        "{text}"
    );
    let (err, text) = call(
        &send,
        json!({"to": "tal@example.org", "subject": "hi", "body_markdown": "x"}),
        &dir,
    )
    .await;
    assert!(!err, "{text}");
    assert_eq!(
        text, "sent (message id fx-0002) from `work` to tal@example.org",
        "the real server's sentence"
    );
    let (err, text) = call(
        &reply,
        json!({"thread_id": "t-9", "body_markdown": "x"}),
        &dir,
    )
    .await;
    assert!(err && text.contains("no thread"), "{text}");

    let sent = std::fs::read_to_string(store.join("sent.jsonl")).unwrap();
    assert_eq!(
        sent.lines().count(),
        2,
        "every send is a line, and nothing left the machine"
    );
    let first: Value = serde_json::from_str(sent.lines().next().unwrap()).unwrap();
    assert_eq!(first["tool"], "mail_reply");
    assert_eq!(first["produced"]["to"][0], "priya.nair@example.edu");

    let again = connect(&cfg, &dir).await;
    let thread = tool_named(&again, "mail__mail_get_thread");
    let (err, text) = call(&thread, json!({"thread_id": "t-1"}), &dir).await;
    assert!(!err, "{text}");
    assert!(
        text.contains("Message id (for mail_reply): m-1") && text.contains("fx-0001"),
        "the reply threaded and persisted: {text}"
    );
    let create = tool_named(&again, "mail__calendar_create_event");
    let (err, text) = call(&create, json!({"title": "Aurora check-in", "start_time": "2026-09-08T10:00:00Z", "end_time": "2026-09-08T10:30:00Z", "attendees": ["priya.nair@example.edu"]}), &dir).await;
    assert!(!err && text.contains("created event ev-fx0001"), "{text}");
    let (err, _) = call(&create, json!({"title": "x", "start_time": "2026-09-08T10:00:00Z", "end_time": "2026-09-08T09:00:00Z"}), &dir).await;
    assert!(err, "an end before its start is refused");

    std::fs::remove_dir_all(&dir).ok();
}

/// No store is a refusal to start, not a server that forgets.
#[tokio::test]
async fn a_fixture_server_without_a_store_refuses_to_start() {
    if unavailable("python3", python3_available()) {
        return;
    }
    let dir = tmpdir("fixture-nostore");
    let mut cfg = server("graph", "board_server.py", &dir, Some(false));
    cfg.env.clear();
    assert!(
        McpClient::connect(&cfg, &unconfined(), &dir).await.is_err(),
        "a board with nowhere to persist must not answer the handshake"
    );
    std::fs::remove_dir_all(&dir).ok();
}
