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
    check_shutdown(Case::Partial).await;
}

#[tokio::test]
async fn sigterm_resolves_a_pending_question() {
    check_shutdown(Case::Question).await;
}

#[tokio::test]
async fn a_second_ctrl_c_forces_shutdown_during_a_blocked_mcp_call() {
    check_shutdown(Case::Force).await;
}

#[tokio::test]
async fn steering_that_arrives_in_the_final_answer_is_retracted_on_every_device() {
    check_shutdown(Case::Undelivered).await;
}

#[tokio::test]
async fn folded_steering_is_acknowledged_once_and_recorded() {
    check_shutdown(Case::Delivered).await;
}

#[derive(Clone, Copy, Debug)]
enum Case {
    Partial,
    Question,
    Force,
    Undelivered,
    Delivered,
}

async fn check_shutdown(case: Case) {
    let available = tokio::process::Command::new("python3")
        .arg("--version")
        .output()
        .await
        .is_ok_and(|out| out.status.success());
    if !available {
        assert!(
            std::env::var_os("MECHA_TEST_REQUIRE_BACKENDS").is_none(),
            "python3 is required for the serve lifecycle fixture"
        );
        eprintln!("skipping: python3 unavailable for the serve lifecycle fixture");
        return;
    }
    let question = matches!(case, Case::Question);
    let force = matches!(case, Case::Force);
    let delivered = matches!(case, Case::Delivered);
    let finish_normally = matches!(case, Case::Delivered | Case::Undelivered);
    let root =
        std::env::temp_dir().join(format!("mecha-serve-exit-{}-{case:?}", std::process::id()));
    let _cleanup = FixtureCleanup(root.clone());
    let home = root.join("home");
    let work = root.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let provider = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_addr = provider.local_addr().unwrap();
    let finish = tokio_util::sync::CancellationToken::new();
    let finish_provider = finish.clone();
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move || {
        let finish = finish_provider.clone();
        let first_call = requests.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
        async move {
            use futures::StreamExt;
            let chunk = if force {
                serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"hang-1","type":"function","function":{"name":"fixture__hang","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
            } else if question {
                serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"ask-1","type":"function","function":{"name":"ask_user","arguments":"{\"question\":\"Continue?\"}"}}]},"finish_reason":"tool_calls"}]})
            } else if delivered && first_call {
                serde_json::json!({"choices":[{"index":0,"delta":{"content":"Saved partial answer","tool_calls":[{"index":0,"id":"read-1","type":"function","function":{"name":"fs_read","arguments":"{\"path\":\"missing.txt\"}"}}]},"finish_reason":null}]})
            } else {
                // A finished sentence, then more to come: a call speaks a
                // sentence once it ends (`voice::speech`), so a phrase still
                // open would never reach the voice stream this test reads
                // before it signals.
                serde_json::json!({"choices":[{"index":0,"delta":{"content":"Saved partial answer. "},"finish_reason":if delivered {Some("stop")} else {None}}]})
            };
            let first = futures::stream::once(async move {
                Ok::<_, std::convert::Infallible>(axum::response::sse::Event::default().data(chunk.to_string()))
            });
            let rest = if question || force || (delivered && !first_call) {
                futures::stream::empty().boxed()
            } else {
                futures::stream::once(async move {
                    finish.cancelled().await;
                    let reason = if delivered { "tool_calls" } else { "stop" };
                    Ok(axum::response::sse::Event::default().data(serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}).to_string()))
                }).boxed()
            };
            axum::response::Sse::new(first.chain(rest))
        }
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
[features]
web = true
# The voice facade these drive is voice calls' (`serve` mounts it only with
# calls on, since step 3b).
voice = true
[[mcp]]
name = "fixture"
command = "python3"
args = [{mcp_script:?}, {mcp_pid:?}]
sandbox = false
"#
        ),
    )
    .unwrap();
    // Both held until both are read: a port dropped before the second is
    // asked for can be handed straight back (macOS does), and serve then
    // binds one address twice — `Address already in use` on 49287, twice.
    let (a, b) = (
        std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
        std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
    );
    let (port, voice_port) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    drop((a, b));
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
    if finish_normally {
        finish.cancel();
        for response in [&mut events, &mut observer] {
            let seen = read_until(response, "\"type\":\"done\"").await;
            let receipt = if delivered {
                "queued_delivered"
            } else {
                "queued_discarded"
            };
            assert!(seen.contains(receipt), "missing {receipt}: {seen}");
            assert_eq!(
                seen.matches("request-2").count(),
                1,
                "receipt must update one existing input"
            );
        }
    }
    // A connection still reading its request must also leave on shutdown.
    let _idle_voice = tokio::net::TcpStream::connect(("127.0.0.1", voice_port))
        .await
        .unwrap();
    let mut voice = if !question && !force && !finish_normally {
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
            transcripts.contains(if finish_normally {
                "\"stop_cause\":\"completed\""
            } else {
                "\"stop_cause\":\"shutdown\""
            }),
            "shutdown outcome was not recorded: {transcripts}"
        );
        assert!(transcripts.contains("\"taint\""), "taint was not recorded");
        if !question && !finish_normally {
            assert_eq!(
                transcripts.matches("\"stop_cause\":\"shutdown\"").count(),
                2,
                "both typed and unhosted voice runs must finish recording"
            );
        }
    }
    if finish_normally {
        assert_eq!(
            transcripts.contains("same words"),
            delivered,
            "only delivered steering belongs in the transcript"
        );
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

/// Step 3b: `serve` mounts its voice facade only with voice calls on at
/// start. The facade is a second listener that cannot refuse per request, so
/// with `voice = false` and a `--voice-port` nothing may listen there, and
/// the note says why (review of #452: the positive case alone left the guard
/// unmeasured — deleting it kept every other test green).
#[tokio::test]
async fn serve_does_not_mount_the_voice_facade_with_calls_off() {
    let root = std::env::temp_dir().join(format!("mecha-serve-nofacade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let work = root.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // A provider that is never asked: the chat only has to build.
    let provider = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let provider_addr = provider.local_addr().unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "default_provider = \"fixture\"\n[providers.fixture]\nkind = \"openai-compatible\"\n\
             base_url = \"http://{provider_addr}\"\nmodel = \"fixture\"\nmax_retries = 0\n\
             [tools]\nenabled = [\"fs_read\"]\n[sandbox]\nkind = \"none\"\n\
             [features]\nweb = true\nvoice = false\n"
        ),
    )
    .unwrap();
    // Both held until both are read: a port dropped before the second is
    // asked for can be handed straight back (macOS does), and serve then
    // binds one address twice — `Address already in use` on 49287, twice.
    let (a, b) = (
        std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
        std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
    );
    let (port, voice_port) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    drop((a, b));
    let log_path = root.join("serve.log");
    let log = std::fs::File::create(&log_path).unwrap();
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
    let client = reqwest::Client::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let ping = client
                .get(format!("http://127.0.0.1:{port}/api/ping"))
                .header("Tailscale-User-Login", "test@example.com")
                .send()
                .await;
            if ping.is_ok() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(&log_path).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("serve came up");
    let facade = tokio::net::TcpStream::connect(("127.0.0.1", voice_port)).await;
    let log = std::fs::read_to_string(&log_path).unwrap();
    let _ = child.kill().await;
    drop(provider);
    let _ = std::fs::remove_dir_all(&root);
    assert!(facade.is_err(), "the facade listened with calls off\n{log}");
    assert!(log.contains("the voice facade is not mounted"), "{log}");
}
