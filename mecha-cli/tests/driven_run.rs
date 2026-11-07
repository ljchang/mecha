//! A driven run (`docs/DRIVER-DESIGN.md` D1b), through the real binary: what
//! `MECHA_SESSION_KIND=driven` forces, refuses and records.
//!
//! The binary is not built under `cfg(test)`, so the mark reads as it does
//! in production — the trap a unit test of the mark falls into (it passes
//! vacuously, because `SessionKind::test_override` ignores the variable
//! there) does not apply. The owner's config here attaches everything a
//! driven run must drop: an MCP server that leaves a file when it starts, a
//! hook, a grant, the mailbox, a subagent, `trifecta = "ask"` and
//! `permission_mode = "ask"`. A fixture provider records every request, so
//! the tool surface asserted is the one the model was sent.
//!
//! Needs `bwrap` (a driven run refuses a sandbox that can reach the network);
//! skips without it unless `MECHA_TEST_REQUIRE_BACKENDS` is set.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bwrap_available() -> bool {
    let ok = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "true"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        assert!(
            std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
            "bwrap is required"
        );
        eprintln!("skipping: bwrap unavailable");
    }
    ok
}

/// A fixture provider that answers every request with one line of text and
/// keeps each request body.
async fn provider() -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&seen);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |body: axum::Json<serde_json::Value>| {
            let keep = Arc::clone(&keep);
            async move {
                keep.lock().unwrap().push(body.0);
                let chunk = serde_json::json!({"choices": [{"index": 0,
                    "delta": {"content": "done"}, "finish_reason": "stop"}]});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                )
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

struct Fixture {
    _root: Root,
    home: PathBuf,
    clone: PathBuf,
    started: PathBuf,
}

/// A home whose global config attaches every surface a driven run drops,
/// and a task clone carrying a `mecha.toml` that would undo the sandbox if
/// it were merged.
fn fixture(name: &str, base_url: &str, sandbox: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("mecha-driven-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let clone = root.join("clone");
    let granted = root.join("granted");
    for d in [&home, &clone, &granted] {
        std::fs::create_dir_all(d).unwrap();
    }
    let started = root.join("mcp-started");
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
base_url = "{base_url}"
model = "fixture"
max_retries = 0
[tools]
permission_mode = "ask"
[security]
trifecta = "ask"
[messages]
enabled = true
{sandbox}
[[mcp]]
name = "nosy"
command = "python3"
args = ["-c", "open('{started}', 'w').write('up')"]
[[hook]]
event = "pre_tool"
command = "true"
[[grant]]
path = "{granted}"
[[subagent]]
name = "helper"
description = "reads files"
tools = ["fs_read"]
"#,
            started = started.display(),
            granted = granted.display(),
        ),
    )
    .unwrap();
    std::fs::write(
        clone.join("mecha.toml"),
        "[sandbox]\nkind = \"none\"\nnetwork = true\n",
    )
    .unwrap();
    Fixture {
        _root: Root(root),
        home,
        clone,
        started,
    }
}

const BWRAP: &str = "[sandbox]\nkind = \"bwrap\"\nnetwork = false\n";

async fn mecha(home: &Path, cwd: &Path, kind: &str, args: &[&str]) -> Output {
    tokio::time::timeout(
        Duration::from_secs(60),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", home)
            .env("MECHA_SESSION_KIND", kind)
            .env_remove("MECHA_SESSION_DIR")
            .env_remove("MECHA_LEARNING_DIR")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(cwd)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("mecha timed out")
    .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Every record of the one session under `home`.
fn records(home: &Path) -> Vec<serde_json::Value> {
    let dir = home.join("sessions");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    std::fs::read_to_string(&files[0])
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

const ALLOWED: &[&str] = &[
    "code",
    "compact",
    "fs_edit",
    "fs_list",
    "fs_read",
    "fs_write",
    "goal_context",
    "shell",
    "skill",
    "todo",
];

#[tokio::test]
async fn a_driven_run_drops_the_owners_surfaces_and_records_what_it_forced() {
    if !bwrap_available() {
        return;
    }
    let (url, seen) = provider().await;
    let f = fixture("forced", &url, BWRAP);
    let out = mecha(&f.home, &f.clone, "driven", &["run", "hello", "--json"]).await;
    assert!(out.status.success(), "{}", stderr(&out));

    // Nothing the owner attached reached the run.
    assert!(!f.started.exists(), "the owner's MCP server was started");
    let requests = seen.lock().unwrap().clone();
    assert!(!requests.is_empty(), "the provider was never asked");
    for req in &requests {
        let sent: Vec<&str> = req["tools"]
            .as_array()
            .map(|t| {
                t.iter()
                    .filter_map(|t| t["function"]["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(!sent.is_empty(), "no tools sent: {req}");
        for name in &sent {
            assert!(ALLOWED.contains(name), "`{name}` was sent to the model");
        }
    }

    let records = records(&f.home);
    let meta = records.iter().find(|r| r["record"] == "meta").unwrap();
    assert_eq!(meta["kind"], "driven", "{meta}");
    let config = records.iter().find(|r| r["record"] == "config").unwrap();
    for name in config["tools"].as_array().unwrap() {
        assert!(ALLOWED.contains(&name.as_str().unwrap()), "{name} recorded");
    }
    assert_eq!(config["trifecta"], "block");
    assert_eq!(config["permission_mode"], "allow");
    // The project layer was not merged: its `kind = "none"` would have
    // refused the start, and the record names the sandbox the owner set.
    assert_eq!(config["sandbox"], "bwrap");
    let levers: Vec<&str> = config["levers_off"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    for lever in [
        "mcp",
        "hooks",
        "grants",
        "messages",
        "system_read",
        "fallback",
    ] {
        assert!(levers.contains(&lever), "{lever} not off: {levers:?}");
    }
    let driven = &config["driven"];
    assert_eq!(driven["block_sends_after_private"], true, "{driven}");
    assert_eq!(driven["private_armed_at_start"], true);
    assert_eq!(driven["prompt_as_forbid"], true);
    assert_eq!(driven["owner_trifecta"], "ask");
    assert_eq!(driven["owner_permission_mode"], "ask");
    assert_eq!(driven["shell_rule_loaded"], false);
    assert_eq!(driven["subagents_dropped"], serde_json::json!(["helper"]));
    let clone = f.clone.canonicalize().unwrap();
    assert_eq!(
        driven["project_configs"],
        serde_json::json!([clone.join("mecha.toml")])
    );
    // And `private` was armed for the run, so every send is refused.
    let taint = records
        .iter()
        .rev()
        .find(|r| r["record"] == "taint")
        .expect("a taint record");
    assert_eq!(taint["private"], true, "{taint}");
}

#[tokio::test]
async fn a_driven_start_is_refused_where_it_could_not_be_confined() {
    if !bwrap_available() {
        return;
    }
    let (url, seen) = provider().await;
    // No sandbox: `shell` could reach the network.
    let f = fixture("unconfined", &url, "");
    let out = mecha(&f.home, &f.clone, "driven", &["run", "hello"]).await;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("can reach the network"),
        "{}",
        stderr(&out)
    );
    // A readable bind over this fixture's own home, by its parent (the
    // root `fixture` builds the home in).
    let parent = std::env::temp_dir()
        .join(format!("mecha-driven-bind-{}", std::process::id()))
        .display()
        .to_string();
    let f2 = fixture("bind", &url, &format!("{BWRAP}readable = [\"{parent}\"]\n"));
    let out = mecha(&f2.home, &f2.clone, "driven", &["run", "hello"]).await;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("overlaps the mecha home"),
        "{}",
        stderr(&out)
    );
    // Resumed, unrecorded, or through any other door than `mecha run`.
    let f3 = fixture("doors", &url, BWRAP);
    for (args, why) in [
        (vec!["run", "hello", "--resume", "x"], "never resumes"),
        (vec!["run", "hello", "--no-session"], "is recorded"),
        (
            vec!["run", "hello", "--read-only"],
            "`--read-only` would refuse",
        ),
        (vec!["batch", "items.jsonl"], "fresh one-shot `mecha run`"),
    ] {
        std::fs::write(f3.clone.join("items.jsonl"), "\"hello\"\n").unwrap();
        let out = mecha(&f3.home, &f3.clone, "driven", &args).await;
        assert!(!out.status.success(), "{args:?} ran");
        assert!(stderr(&out).contains(why), "{args:?}: {}", stderr(&out));
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "a refused start reached the model"
    );
    for f in [&f, &f2, &f3] {
        assert!(
            !f.started.exists(),
            "a refused start started the MCP server"
        );
    }
}

#[tokio::test]
async fn an_unmarked_run_inside_a_marked_clone_is_refused_until_unmarked() {
    if !bwrap_available() {
        return;
    }
    let (url, _seen) = provider().await;
    let f = fixture("marker", &url, BWRAP);
    let clone = f.clone.display().to_string();
    let mark = mecha(
        &f.home,
        &f.clone,
        "test",
        &["driven", "mark", "t-1", "--clone", &clone, "--for", "1s"],
    )
    .await;
    assert!(mark.status.success(), "{}", stderr(&mark));
    // Expired or not, an unmarked run there is refused, before anything the
    // owner's config attaches has started.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let out = mecha(&f.home, &f.clone, "test", &["run", "hello"]).await;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("driven task `t-1`"),
        "{}",
        stderr(&out)
    );
    assert!(
        !f.started.exists(),
        "the refused run started the MCP server"
    );
    // A driven run there is not refused for it.
    let out = mecha(&f.home, &f.clone, "driven", &["run", "hello"]).await;
    assert!(out.status.success(), "{}", stderr(&out));
    let unmark = mecha(&f.home, &f.clone, "test", &["driven", "unmark", "t-1"]).await;
    assert!(unmark.status.success(), "{}", stderr(&unmark));
    let out = mecha(&f.home, &f.clone, "test", &["run", "hello", "--no-session"]).await;
    assert!(
        !stderr(&out).contains("driven task"),
        "still refused after unmark: {}",
        stderr(&out)
    );
}
