//! Exercise actual process exit, which cannot be measured by dropping a
//! library client in a test process that keeps running.

use std::time::Duration;
use tokio::process::Command;

#[tokio::test]
async fn refusal_exit_removes_the_docker_mcp_container() {
    run_exit("content_filter", 2).await;
}

#[tokio::test]
async fn no_output_exit_removes_the_docker_mcp_container() {
    run_exit("stop", 3).await;
}

#[tokio::test]
async fn failed_batch_exit_removes_the_docker_mcp_container() {
    run_exit("content_filter", 1).await;
}

async fn run_exit(reason: &'static str, code: i32) {
    let available = Command::new("docker")
        .args(["image", "inspect", "python:3-slim"])
        .output()
        .await
        .is_ok_and(|out| out.status.success());
    if !available {
        assert!(
            std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
            "Docker with python:3-slim is required"
        );
        eprintln!("skipping: Docker with python:3-slim unavailable");
        return;
    }
    let root = std::env::temp_dir().join(format!("mecha-run-exit-{}-{code}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let work = root.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || async move {
        if code == 1 {
            let body = serde_json::json!({"choices": [{"index": 0, "message": {"role": "assistant", "content": ""}, "finish_reason": reason}]});
            return ([("content-type", "application/json")], body.to_string());
        }
        let chunk = serde_json::json!({"choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": reason}]});
        ([("content-type", "text/event-stream")], format!("data: {chunk}\n\ndata: [DONE]\n\n"))
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    std::fs::write(
        work.join("server.py"),
        r#"import json, socket, sys, time
from pathlib import Path
Path('cid').write_text(socket.gethostname())
for line in sys.stdin:
    req = json.loads(line)
    if 'id' in req:
        result = {'tools': []} if req.get('method') == 'tools/list' else {}
        print(json.dumps({'jsonrpc':'2.0', 'id':req['id'], 'result':result}), flush=True)
while True:
    time.sleep(0.01)
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
base_url = "http://{addr}"
model = "fixture"
max_retries = 0
[tools]
enabled = ["fs_read"]
[sandbox]
kind = "docker"
image = "python:3-slim"
[[mcp]]
name = "fixture"
command = "python3"
args = ["-u", "server.py"]
sandbox = true
"#
        ),
    )
    .unwrap();
    std::fs::write(work.join("items.jsonl"), "\"hello\"\n").unwrap();
    let args = if code == 1 {
        vec!["batch", "items.jsonl"]
    } else {
        vec!["run", "hello", "--json", "--no-session"]
    };
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .env("MECHA_HOME", &home)
            .env("MECHA_SESSION_KIND", "test")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .current_dir(&work)
            .kill_on_drop(true)
            .output(),
    )
    .await;
    server.abort();
    // Keep cleanup outside assertions so a reproduced leak leaves no server.
    let cid = std::fs::read_to_string(work.join("cid")).expect("fixture container never started");
    assert!(cid.len() >= 12 && cid.bytes().all(|b| b.is_ascii_hexdigit()));
    let removed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let out = Command::new("docker")
                .args(["ps", "--all", "--quiet", "--filter", &format!("id={cid}")])
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            assert!(out.status.success());
            if out.stdout.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .is_ok();
    let _ = Command::new("docker")
        .args(["rm", "--force", &cid])
        .output()
        .await;
    let _ = std::fs::remove_dir_all(&root);
    let output = output.expect("mecha run hung").unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(removed, "exit {code} left the MCP container running");
}
