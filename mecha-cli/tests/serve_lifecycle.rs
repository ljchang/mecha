//! Drive the real daemon: dropping a library future cannot test SIGTERM.
use std::time::Duration;

// A deliberately failed shutdown regression must not leave its blocking MCP
// fixture alive when kill_on_drop terminates the parent without destructors.
struct FixtureCleanup(std::path::PathBuf);
impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        if let Ok(pid) = std::fs::read_to_string(self.0.join("mcp.pid")) {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", pid.trim()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

#[tokio::test]
async fn sigterm_closes_sse_and_records_the_partial_turn() {
    check_shutdown(false, false).await;
}

#[tokio::test]
async fn sigterm_resolves_a_pending_question() {
    check_shutdown(true, false).await;
}

#[tokio::test]
async fn a_second_ctrl_c_forces_shutdown_during_a_blocked_mcp_call() {
    check_shutdown(false, true).await;
}

async fn check_shutdown(question: bool, force: bool) {
    let root = std::env::temp_dir().join(format!(
        "mecha-serve-exit-{}-{question}-{force}",
        std::process::id()
    ));
    let _cleanup = FixtureCleanup(root.clone());
    let home = root.join("home");
    let work = root.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let provider = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_addr = provider.local_addr().unwrap();
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || async move {
        use futures::StreamExt;
        let chunk = if force {
            serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"hang-1","type":"function","function":{"name":"fixture__hang","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
        } else if question {
            serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"ask-1","type":"function","function":{"name":"ask_user","arguments":"{\"question\":\"Continue?\"}"}}]},"finish_reason":"tool_calls"}]})
        } else {
            serde_json::json!({"choices":[{"index":0,"delta":{"content":"Saved partial answer"},"finish_reason":null}]})
        };
        let first = futures::stream::once(async move {
            Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(chunk.to_string()))
        });
        let rest = if question || force { futures::stream::empty().boxed() } else { futures::stream::pending().boxed() };
        axum::response::Sse::new(first.chain(rest))
    }));
    let provider_task = tokio::spawn(async move { axum::serve(provider, app).await.unwrap() });
    let mcp_script = root.join("mcp.py");
    let mcp_pid = root.join("mcp.pid");
    std::fs::write(
        &mcp_script,
        r#"import json, os, sys, time
from pathlib import Path
Path(sys.argv[1]).write_text(str(os.getpid()))
for line in sys.stdin:
    req = json.loads(line)
    if 'id' in req:
        if req.get('method') == 'tools/call':
            Path(sys.argv[1] + '.called').touch()
            while True:
                time.sleep(0.05)
        result = {'tools': [{'name': 'hang', 'description': 'wait', 'inputSchema': {'type': 'object'}, 'annotations': {'readOnlyHint': True}}]} if req.get('method') == 'tools/list' else {}
        print(json.dumps({'jsonrpc':'2.0', 'id':req['id'], 'result':result}), flush=True)
while True:
    time.sleep(0.05)
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
base_url = "http://{provider_addr}"
model = "fixture"
max_retries = 0
[tools]
enabled = ["fs_read"]
[sandbox]
kind = "none"
[[mcp]]
name = "fixture"
command = "python3"
args = [{mcp_script:?}, {mcp_pid:?}]
sandbox = false
"#
        ),
    )
    .unwrap();
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let voice_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let log = std::fs::File::create(root.join("serve.log")).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_mecha"))
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
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .current_dir(&work)
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
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client.get(format!("{base}/api/ping")).send().await.is_ok() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(root.join("serve.log")).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let opened = client
        .post(format!("{base}/api/chat/shutdown"))
        .send()
        .await
        .unwrap();
    assert!(
        opened.status().is_success(),
        "{}",
        opened.text().await.unwrap()
    );
    let mut events = client
        .get(format!("{base}/api/chat/shutdown/events"))
        .send()
        .await
        .unwrap();
    assert!(events.status().is_success());
    let mut observer = client
        .get(format!("{base}/api/chat/shutdown/events"))
        .send()
        .await
        .unwrap();
    let sent = client
        .post(format!("{base}/api/chat/shutdown/send"))
        .json(&serde_json::json!({"text":"hello", "request_id":"request-1"}))
        .send()
        .await
        .unwrap();
    assert!(sent.status().is_success());
    let until = if force {
        "fixture__hang"
    } else if question {
        "Continue?"
    } else {
        "Saved partial answer"
    };
    for response in [&mut events, &mut observer] {
        let seen = read_until(response, until).await;
        let input = seen
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|ev| ev["type"] == "user")
            .expect("a watching device missed the typed input");
        assert_eq!(input["request_id"], "request-1");
        assert_eq!(input["spoken"], false);
    }
    let steered = client
        .post(format!("{base}/api/chat/shutdown/send"))
        .json(&serde_json::json!({"text":"same words", "request_id":"request-2"}))
        .send()
        .await
        .unwrap();
    assert!(steered.status().is_success());
    for response in [&mut events, &mut observer] {
        let seen = read_until(response, "request-2").await;
        assert!(
            seen.contains("\"type\":\"queued\""),
            "steering was not broadcast: {seen}"
        );
        assert_eq!(seen.matches("request-2").count(), 1);
    }
    // A connection still reading its request must also leave on shutdown.
    let _idle_voice = tokio::net::TcpStream::connect(("127.0.0.1", voice_port))
        .await
        .unwrap();
    let mut voice = if !question && !force {
        let mut response = client.post(format!("http://127.0.0.1:{voice_port}/v1/chat/completions"))
            .json(&serde_json::json!({"model":"fixture", "stream":true, "messages":[{"role":"user", "content":"speak"}]}))
            .send().await.unwrap();
        assert!(response.status().is_success());
        read_until(&mut response, "Saved partial answer").await;
        Some(response)
    } else {
        None
    };
    if force {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join("mcp.pid.called").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the run never entered its blocking tool");
    }
    let signal = if force { "-INT" } else { "-TERM" };
    assert!(tokio::process::Command::new("kill")
        .args([signal, &child.id().unwrap().to_string()])
        .status()
        .await
        .unwrap()
        .success());
    if force {
        // Closing SSE proves the first signal reached the drain. Wait for
        // that observation rather than guessing how fast a loaded CI host is.
        tokio::time::timeout(Duration::from_secs(5), async {
            while events.chunk().await.unwrap().is_some() {}
        })
        .await
        .expect("first signal never closed SSE");
        assert!(
            child.try_wait().unwrap().is_none(),
            "first Ctrl-C should drain cooperatively"
        );
        assert!(tokio::process::Command::new("kill")
            .args([signal, &child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap()
            .success());
    }
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("shutdown hung with SSE open")
        .unwrap();
    provider_task.abort();
    // Keep all response bodies alive until exit; idle/open sockets cannot
    // accidentally make this pass by causing client-side cancellation.
    if !force {
        while events.chunk().await.unwrap().is_some() {}
    }
    if let Some(response) = voice.as_mut() {
        while response.chunk().await.unwrap().is_some() {}
    }
    assert!(
        status.success(),
        "serve exited without graceful shutdown: {status}"
    );
    let transcripts = std::fs::read_dir(home.join("sessions"))
        .unwrap()
        .map(|p| std::fs::read_to_string(p.unwrap().path()).unwrap())
        .collect::<String>();
    if !question && !force {
        assert!(
            transcripts.contains("Saved partial answer"),
            "partial answer was lost: {transcripts}"
        );
    }
    if !force {
        assert!(
            transcripts.contains("\"stop_cause\":\"shutdown\""),
            "shutdown outcome was not recorded: {transcripts}"
        );
        assert!(transcripts.contains("\"taint\""), "taint was not recorded");
        if !question {
            assert_eq!(
                transcripts.matches("\"stop_cause\":\"shutdown\"").count(),
                2,
                "both typed and unhosted voice runs must finish recording"
            );
        }
    }
    let pid = std::fs::read_to_string(mcp_pid).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::process::Command::new("kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap()
            .success()
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("shutdown left its stdio MCP process alive");
    std::fs::remove_dir_all(root).unwrap();
}

async fn read_until(response: &mut reqwest::Response, needle: &str) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut seen = String::new();
        while let Some(chunk) = response.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains(needle) {
                return seen;
            }
        }
        panic!("stream ended before {needle}: {seen}");
    })
    .await
    .unwrap()
}
