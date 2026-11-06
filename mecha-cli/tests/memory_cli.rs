//! Black-box contract for the actual `mecha` binary; synthetic files only.
use std::{
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mecha-memory-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        // These commands must be independent of both config layers.
        std::fs::write(root.join("home/config.toml"), "not = [valid toml").unwrap();
        std::fs::write(root.join("work/mecha.toml"), "not = [valid toml").unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .current_dir(self.0.join("work"))
            .env("MECHA_HOME", self.0.join("home"))
            .env("MECHA_SESSION_KIND", "test")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .output()
            .unwrap()
    }
    #[cfg(feature = "memory-prototype")]
    fn memory(&self, args: &[&str]) -> serde_json::Value {
        let db = self.0.join("test.memory-prototype.sqlite");
        let mut argv = vec!["memory", "--experimental", "--db", db.to_str().unwrap()];
        argv.extend_from_slice(args);
        let output = self.run(&argv);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(feature = "memory-prototype"))]
#[test]
fn normal_build_does_not_expose_experimental_memory() {
    let f = Fixture::new();
    let output = f.run(&["memory", "--help"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
}

#[cfg(feature = "memory-prototype")]
#[test]
fn main_binary_runs_temporal_demo_and_forget_without_loading_config() {
    let f = Fixture::new();
    let unrelated = f.0.join("graph.db");
    std::fs::write(&unrelated, b"graph stand-in").unwrap();
    let demo = f.memory(&["demo"]);
    assert_eq!(demo["fixture_only"], true);
    let old = demo["snapshots"]["before_departure"]["sequence"].to_string();
    let inspect = f.memory(&["inspect"]);
    assert_eq!(inspect["snapshot"], demo["snapshots"]["after_departure"]);
    assert_eq!(
        f.memory(&["query", "works", "--valid-at", "2026-07-01T00:00:00Z"])
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        f.memory(&[
            "query",
            "works",
            "--known-at",
            &old,
            "--valid-at",
            "2026-07-01T00:00:00Z"
        ])
        .as_array()
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        f.memory(&["query", "works", "--valid-at", "2026-05-15T00:00:00Z"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.memory(&["forget", "employment"])["revisions"]
            .as_u64()
            .unwrap()
            >= 3
    );
    assert_eq!(
        f.memory(&[
            "query",
            "works",
            "--known-at",
            &old,
            "--valid-at",
            "2026-07-01T00:00:00Z"
        ])
        .as_array()
        .unwrap()
        .len(),
        0
    );
    assert_eq!(std::fs::read(unrelated).unwrap(), b"graph stand-in");
    assert_eq!(
        std::fs::read_dir(f.0.join("home")).unwrap().count(),
        1,
        "no holds, transcripts, or other live stores created"
    );
}

#[cfg(feature = "memory-prototype")]
#[test]
fn opt_in_and_database_are_required_before_any_write() {
    let f = Fixture::new();
    let db = f.0.join("no.memory-prototype.sqlite");
    let path = db.to_str().unwrap();
    for args in [
        vec!["memory", "--db", path, "demo"],
        vec!["memory", "--experimental", "demo"],
    ] {
        let output = f.run(&args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("required"));
    }
    assert!(!db.exists());
    assert!(!f.0.join("work/no.memory-prototype.sqlite").exists());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn foreign_files_and_invalid_query_parameters_fail_without_mutating_content() {
    let f = Fixture::new();
    let db = f.0.join("test.memory-prototype.sqlite");
    let path = db.to_str().unwrap();
    std::fs::write(&db, b"foreign database").unwrap();
    for verb in ["demo", "inspect"] {
        assert!(!f
            .run(&["memory", "--experimental", "--db", path, verb])
            .status
            .success());
        assert_eq!(std::fs::read(&db).unwrap(), b"foreign database");
    }
    std::fs::remove_file(&db).unwrap();
    f.memory(&["demo"]);
    for args in [
        vec!["--known-at", "999999"],
        vec!["--valid-at", "not-a-date"],
        vec!["--limit", "0"],
    ] {
        let mut argv = vec!["memory", "--experimental", "--db", path, "query", "works"];
        argv.extend(args);
        assert!(!f.run(&argv).status.success());
    }
}

#[cfg(feature = "memory-prototype")]
#[test]
fn native_session_import_recall_and_forget_are_explicit_and_source_grounded() {
    let f = Fixture::new();
    f.memory(&["init"]);
    let source = f.0.join("synthetic.jsonl");
    let raw = concat!(
        "{\"record\":\"meta\",\"id\":\"session-1\",\"created_at\":\"2026-01-01T00:00:00Z\"}\n",
        "{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"The test permit is delayed.\"}]}\n"
    );
    std::fs::write(&source, raw).unwrap();
    let db = f.0.join("test.memory-prototype.sqlite");
    let denied = f.run(&[
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "import-session",
        source.to_str().unwrap(),
        "--account",
        "synthetic",
    ]);
    assert!(
        !denied.status.success(),
        "import requires the synthetic-input acknowledgement"
    );
    let report = f.memory(&[
        "import-session",
        source.to_str().unwrap(),
        "--account",
        "synthetic",
        "--synthetic-input",
    ]);
    assert_eq!(report["inserted"], 2);
    let report = f.memory(&[
        "import-session",
        source.to_str().unwrap(),
        "--account",
        "synthetic",
        "--synthetic-input",
    ]);
    assert_eq!(report["unchanged"], 2);
    let hits = f.memory(&[
        "recall",
        "permit",
        "--account",
        "synthetic",
        "--session",
        "session-1",
    ]);
    assert_eq!(
        hits[0]["anchor"]["record"]["data"]["text"],
        raw.lines().nth(1).unwrap().to_owned() + "\n"
    );
    assert_eq!(
        hits[0]["anchor"]["record"]["data"]["transcript"]["fields"][0]["pointer"],
        "/content/0/text"
    );
    f.memory(&[
        "forget-session-record",
        "--account",
        "synthetic",
        "--session",
        "session-1",
        "--line",
        "2",
    ]);
    assert_eq!(f.memory(&["recall", "permit"]), serde_json::json!([]));
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        raw,
        "source input is never rewritten by local forgetting"
    );
}

#[cfg(feature = "memory-prototype")]
#[test]
fn memory_evaluation_is_a_reproducible_evidence_report_without_a_model() {
    let f = Fixture::new();
    let report = f.memory(&["evaluate"]);
    assert_eq!(report["cases"].as_array().unwrap().len(), 32);
    assert_eq!(report["oracle_complete"], 32);
    assert_eq!(report["reasoning_evaluated"], false);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn model_evaluation_rejects_remote_endpoint_before_creating_any_store() {
    let f = Fixture::new();
    let db = f.0.join("model.memory-prototype.sqlite");
    let out = f.run(&[
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "evaluate-model",
        "--endpoint",
        "https://example.com",
        "--model",
        "m",
        "--holds-dir",
        f.0.join("holds").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("loopback"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!db.exists());
    assert!(!f.0.join("holds").exists());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn model_comparison_uses_only_explicit_isolated_local_requests() {
    check_model_comparison_wire(0);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn evidence_hop_sends_explicit_citation_schemas_and_matched_search_calls() {
    check_model_comparison_wire(1);
}

#[cfg(feature = "memory-prototype")]
fn check_model_comparison_wire(mode: u8) {
    let follow = mode != 0;
    let automatic = mode == 2;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stopping = done.clone();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut requests = Vec::new();
        while !stopping.load(Ordering::Relaxed) && Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut data = Vec::new();
            let mut chunk = [0; 4096];
            let (header_end, length) = loop {
                let n = stream.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                data.extend_from_slice(&chunk[..n]);
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .map(|s| s.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    break (pos + 4, length);
                }
            };
            while data.len() < header_end + length {
                let n = stream.read(&mut chunk).unwrap();
                assert_ne!(n, 0);
                data.extend_from_slice(&chunk[..n]);
            }
            let response=if data.starts_with(b"GET /props ") {
                serde_json::json!({"role":"router"})
            } else if data.starts_with(b"GET /models ") {
                serde_json::json!({"data":[{"id":"test-model","status":{"value":"loaded"}}]})
            } else {
                requests.push(serde_json::from_slice::<serde_json::Value>(&data[header_end..header_end+length]).unwrap());
                serde_json::json!({"model":"test-model","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":
                    if requests.last().unwrap()["messages"][0]["content"].as_str().unwrap().contains("BINDING_PLANNER") {r#"{"searches":[]}"#}
                    else if requests.last().unwrap()["messages"][0]["content"].as_str().unwrap().contains("SEARCH_PLANNER") {r#"{"queries":[]}"#}
                    else {r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#}}}],"usage":{"prompt_tokens":12,"completion_tokens":8}})
            }.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
        requests
    });
    let f = Fixture::new();
    let db = f.0.join("model.memory-prototype.sqlite");
    let holds = f.0.join("holds");
    let mut args = vec![
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "evaluate-model",
        "--endpoint",
        &endpoint,
        "--model",
        "test-model",
        "--holds-dir",
        holds.to_str().unwrap(),
    ];
    if follow {
        args.push(if automatic {
            "--auto-bindings"
        } else {
            "--follow-evidence"
        });
    }
    let output = f.run(&args);
    done.store(true, Ordering::Relaxed);
    let requests = server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let report = if automatic {
        assert_eq!(report["cases"].as_array().unwrap().len(), 14);
        serde_json::json!({"rows":report["cases"].as_array().unwrap().iter().flat_map(|c|c["rows"].as_array().unwrap().iter().cloned()).collect::<Vec<_>>()})
    } else {
        report
    };
    assert_eq!(
        report["rows"].as_array().unwrap().len(),
        if automatic {
            42
        } else if follow {
            36
        } else {
            48
        }
    );
    assert!(
        report["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["error"].is_null()),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.len(),
        if automatic {
            56
        } else if follow {
            60
        } else {
            64
        }
    );
    for req in requests {
        if follow {
            let prompt = req["messages"][0]["content"].as_str().unwrap();
            let schema = &req["response_format"]["schema"];
            assert_eq!(req["response_format"]["type"], "json_object");
            if prompt.starts_with("ANSWER") {
                let input: serde_json::Value =
                    serde_json::from_str(prompt.rsplit_once('\n').unwrap().1).unwrap();
                let ids = &input["allowed_citation_ids"];
                if ids.as_array().unwrap().is_empty() {
                    assert_eq!(schema["properties"]["citations"]["maxItems"], 0);
                } else {
                    assert_eq!(&schema["properties"]["citations"]["items"]["enum"], ids);
                }
            } else {
                assert_eq!(
                    schema["properties"][if automatic { "searches" } else { "queries" }]
                        ["maxItems"],
                    2
                );
            }
        } else {
            assert!(req.get("response_format").is_none());
        }
        assert_eq!(req["model"], "test-model");
        assert_eq!(req["seed"], 7);
        assert_eq!(req["max_tokens"], 8192);
        assert_eq!(req["temperature"], 0.6);
        assert!(req
            .get("tools")
            .is_none_or(|v| v.as_array().is_some_and(|a| a.is_empty())));
        assert_eq!(req["messages"].as_array().unwrap().len(), 1);
        assert_eq!(req["messages"][0]["role"], "user");
    }
    assert_eq!(
        std::fs::read_dir(f.0.join("holds")).unwrap().count(),
        0,
        "holds released after requests"
    );
}

#[cfg(feature = "memory-prototype")]
#[test]
fn mention_diagnostic_runs_without_model_or_config_and_never_overwrites() {
    let f = Fixture::new();
    let report = f.memory(&["evaluate-mentions"]);
    assert_eq!(report["protocol"], "mention-replay-v1");
    assert_eq!(report["model_calls"], 0);
    assert_eq!(report["rows"].as_array().unwrap().len(), 24);
    let db = f.0.join("test.memory-prototype.sqlite");
    let before = std::fs::read(&db).unwrap();
    let result = f.run(&[
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "evaluate-mentions",
    ]);
    assert!(!result.status.success());
    assert_eq!(std::fs::read(db).unwrap(), before);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn automatic_binding_protocol_has_one_shared_planner_on_the_wire() {
    check_model_comparison_wire(2);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn plan_evaluator_requires_explicit_input_and_rejects_oversized_files_before_network() {
    let f = Fixture::new();
    f.memory(&["init"]);
    let db = f.0.join("test.memory-prototype.sqlite");
    let input = f.0.join("plans.json");
    std::fs::write(&input, vec![b' '; 20_001]).unwrap();
    let holds = f.0.join("holds");
    let args = [
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "evaluate-plans",
        input.to_str().unwrap(),
        "--synthetic-input",
        "--account",
        "synthetic",
        "--endpoint",
        "http://127.0.0.1:1",
        "--model",
        "test-model",
        "--holds-dir",
        holds.to_str().unwrap(),
    ];
    let output = f.run(&args);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("20000 bytes"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(
        &input,
        r#"{"query":"customs","event":"ship","goal":"task:ship","candidates":[],"trusted":true}"#,
    )
    .unwrap();
    let output = f.run(&args);
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
    let args: Vec<_> = args
        .into_iter()
        .filter(|arg| *arg != "--synthetic-input")
        .collect();
    let output = f.run(&args);
    assert!(String::from_utf8_lossy(&output.stderr).contains("--synthetic-input"));
}
