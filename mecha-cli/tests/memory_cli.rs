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

#[cfg(feature = "memory-prototype")]
#[test]
fn native_session_requires_a_fresh_root_and_records_discovery_failure() {
    let f = Fixture::new();
    let database = f.0.join("test.memory-prototype.sqlite");
    let authority = f.0.join("authority");
    let init = f.memory(&[
        "init-native",
        "--synthetic-input",
        "--domain",
        "native-cli",
        "--authority",
        authority.to_str().unwrap(),
    ]);
    assert_eq!(init["native_capture"], true);
    let request = f.0.join("request.json");
    std::fs::write(&request, serde_json::to_vec(&serde_json::json!({
        "prompt":"Save a synthetic checked artifact.", "files":{},
        "criterion":{"goal":"task:check", "goal_text":"Check synthetic rows.", "criterion_text":"Keep approved rows.", "criterion_revision":"a".repeat(64), "confirmation_digest":"b".repeat(64)},
        "expected_rows":[], "domain":"native-cli", "scopes":["synthetic:test"], "account":"synthetic"
    })).unwrap()).unwrap();
    let root = f.0.join("native-task");
    let holds = f.0.join("holds");
    let args = [
        "memory",
        "--experimental",
        "--db",
        database.to_str().unwrap(),
        "native-session",
        "--synthetic-input",
        "--request",
        request.to_str().unwrap(),
        "--authority",
        authority.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--endpoint",
        "http://example.invalid",
        "--model",
        "unused",
        "--holds-dir",
        holds.to_str().unwrap(),
    ];
    let failed = f.run(&args);
    assert!(!failed.status.success());
    assert!(root.join("started.json").is_file());
    let report = std::fs::read(root.join("report.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&report).unwrap()["status"],
        "failed"
    );
    let replay = f.run(&args);
    assert!(!replay.status.success());
    assert!(String::from_utf8_lossy(&replay.stderr).contains("NEW absolute task directory"));
    assert_eq!(std::fs::read(root.join("report.json")).unwrap(), report);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn native_session_execution_failure_is_nonzero_with_preserved_report() {
    native_session_execution_failure(false);
}

#[cfg(feature = "memory-prototype")]
#[test]
fn native_controlled_session_execution_failure_preserves_uncalled_recall_report() {
    native_session_execution_failure(true);
}

#[cfg(feature = "memory-prototype")]
fn native_session_execution_failure(controlled: bool) {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.local_addr().unwrap());
    server.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stopping = done.clone();
    let serving = std::thread::spawn(move || {
        let mut completions = 0;
        while !stopping.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = server.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            let (header_end, length) = loop {
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..pos]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .map(|s| s.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    break (pos + 4, length);
                }
            };
            while bytes.len() < header_end + length {
                let n = stream.read(&mut buf).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request = String::from_utf8_lossy(&bytes[..header_end]);
            let (status, body) = if request.starts_with("GET /models ") {
                (
                    200,
                    r#"{"data":[{"id":"test-model","status":{"value":"loaded"}}]}"#,
                )
            } else if request.starts_with("GET /props ") {
                (200, r#"{"role":"router"}"#)
            } else {
                assert!(request.contains("autoload=false"));
                let body: serde_json::Value =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
                assert_eq!(body["max_tokens"], 4096);
                completions += 1;
                (
                    500,
                    r#"{"error":{"message":"authored unavailable completion"}}"#,
                )
            };
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        completions
    });
    let f = Fixture::new();
    let database = f.0.join("test.memory-prototype.sqlite");
    let authority = f.0.join("authority");
    f.memory(&[
        "init-native",
        "--synthetic-input",
        "--domain",
        "native-cli",
        "--authority",
        authority.to_str().unwrap(),
    ]);
    let request = f.0.join("request.json");
    std::fs::write(&request, serde_json::to_vec(&serde_json::json!({
        "prompt":"Save a synthetic checked artifact.", "files":{},
        "criterion":{"goal":"task:check", "goal_text":"Check synthetic rows.", "criterion_text":"Keep approved rows.", "criterion_revision":"a".repeat(64), "confirmation_digest":"b".repeat(64)},
        "expected_rows":[], "domain":"native-cli", "scopes":["synthetic:test"], "account":"synthetic"
    })).unwrap()).unwrap();
    let root = f.0.join("native-task");
    let holds = f.0.join("holds");
    let recall = f.0.join("recall.json");
    std::fs::write(&recall, r#"{"mode":"lexical","query":"calibration"}"#).unwrap();
    let mut args = vec![
        "memory",
        "--experimental",
        "--db",
        database.to_str().unwrap(),
        "native-session",
        "--synthetic-input",
        "--request",
        request.to_str().unwrap(),
        "--authority",
        authority.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--endpoint",
        &endpoint,
        "--model",
        "test-model",
        "--holds-dir",
        holds.to_str().unwrap(),
    ];
    if controlled {
        args.extend(["--recall-request", recall.to_str().unwrap()]);
    }
    let output = f.run(&args);
    done.store(true, Ordering::Relaxed);
    assert_eq!(
        serving.join().unwrap(),
        1,
        "failed actions must not retry inference"
    );
    assert!(
        !output.status.success(),
        "incomplete execution must exit nonzero"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
    assert!(report["run_error"].is_string());
    if controlled {
        assert_eq!(report["recall"]["status"], "not_called");
        assert_eq!(report["recall"]["entered_calls"], 0);
        assert_eq!(report["recall"]["identity"]["mode"], "lexical");
    } else {
        assert!(report.get("recall").is_none());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("recall protocol"));
    }
    assert!(std::path::Path::new(report["transcript"].as_str().unwrap()).is_file());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        report
    );
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains(root.join("report.json").to_str().unwrap()));
}

#[cfg(feature = "memory-prototype")]
#[test]
fn native_controlled_cli_rejects_recall_before_discovery_and_preserves_started_roots() {
    let f = Fixture::new();
    let database = f.0.join("test.memory-prototype.sqlite");
    let authority = f.0.join("authority");
    f.memory(&[
        "init-native",
        "--synthetic-input",
        "--domain",
        "native-cli",
        "--authority",
        authority.to_str().unwrap(),
    ]);
    let request = f.0.join("request.json");
    std::fs::write(&request, serde_json::to_vec(&serde_json::json!({
        "prompt":"Save a synthetic checked artifact.", "files":{},
        "criterion":{"goal":"task:check", "goal_text":"Check rows.", "criterion_text":"Keep approved rows.", "criterion_revision":"a".repeat(64), "confirmation_digest":"b".repeat(64)},
        "expected_rows":[], "domain":"native-cli", "scopes":["synthetic:test"], "account":"synthetic"
    })).unwrap()).unwrap();
    let recall = f.0.join("recall.json");
    let root = f.0.join("controlled");
    let holds = f.0.join("holds");
    let args = [
        "memory",
        "--experimental",
        "--db",
        database.to_str().unwrap(),
        "native-session",
        "--synthetic-input",
        "--request",
        request.to_str().unwrap(),
        "--recall-request",
        recall.to_str().unwrap(),
        "--authority",
        authority.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--endpoint",
        "http://example.invalid",
        "--model",
        "unused",
        "--holds-dir",
        holds.to_str().unwrap(),
    ];
    for invalid in [
        r#"{"mode":"lexical","query":" "}"#.to_owned(),
        r#"{"mode":"lexical","query":"calibration","surprise":true}"#.to_owned(),
        " ".repeat(1_048_577),
        r#"{"mode":"dense","query":"calibration","endpoint":"http://example.invalid","expected_alias":"fixture","artifact_revision":"fixture-1","dimensions":2,"pooling":"last","normalization":"l2"}"#.to_owned(),
    ] {
        std::fs::write(&recall, invalid).unwrap();
        assert!(!f.run(&args).status.success());
        assert!(!root.exists(), "invalid host recall created a durable start");
        assert!(!holds.exists(), "invalid host recall reached provider discovery");
    }
    std::fs::write(&recall, r#"{"mode":"lexical","query":"calibration"}"#).unwrap();
    assert!(!f.run(&args).status.success());
    let started: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("started.json")).unwrap()).unwrap();
    assert_eq!(started["recall"]["mode"], "lexical");
    let retained = std::fs::read(root.join("report.json")).unwrap();
    assert!(!f.run(&args).status.success());
    assert_eq!(std::fs::read(root.join("report.json")).unwrap(), retained);
}

#[cfg(all(unix, feature = "memory-prototype"))]
#[test]
fn key_generate_init_and_inspect_are_separate_processes() {
    use std::{fs, os::unix::fs::DirBuilderExt};

    let f = Fixture::new();
    let root = f.0.canonicalize().unwrap();
    let data = root.join("data");
    let owner = root.join("owner");
    fs::create_dir(&data).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&owner).unwrap();
    let key = owner.join("memory.key");
    let db = data.join("encrypted.memory-prototype.sqlite");
    let authority = data.join("authority");
    let args = [
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
    ];
    let invoke = |suffix: &[&str]| {
        let mut combined = args.to_vec();
        combined.extend_from_slice(suffix);
        f.run(&combined)
    };

    let excluded = invoke(&[
        "--key-exclude-root",
        data.to_str().unwrap(),
        "--key-exclude-root",
        owner.to_str().unwrap(),
        "key-generate",
    ]);
    assert!(!excluded.status.success());
    assert!(!key.exists());
    let generated = invoke(&["key-generate"]);
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let secret = fs::read_to_string(&key).unwrap();
    let secret = secret.trim_end();
    assert!(!secret.is_empty());
    assert!(!String::from_utf8_lossy(&generated.stdout).contains(secret));
    assert!(!db.exists());

    let legacy = invoke(&["init-durable", "--domain", "synthetic-demo"]);
    assert!(!legacy.status.success());
    assert!(String::from_utf8_lossy(&legacy.stderr).contains("does not support"));
    assert!(!db.exists());
    assert!(!authority.exists());

    let init = invoke(&[
        "init-native",
        "--synthetic-input",
        "--domain",
        "synthetic-demo",
        "--authority",
        authority.to_str().unwrap(),
    ]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let initial: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    let bytes = fs::read(&db).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3\0"));
    let inspect = invoke(&["inspect"]);
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let read: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(initial["snapshot"], read["snapshot"]);
    assert_eq!(fs::read(&db).unwrap(), bytes);
    assert!(!String::from_utf8_lossy(&inspect.stdout).contains(secret));
    {
        use mecha_core::memory::{fixture, Capture, KeyFilePolicy, Memory, StorageAccess};
        let policy = KeyFilePolicy::new(std::slice::from_ref(&data)).unwrap();
        let storage = StorageAccess::load_key_file(&key, &policy).unwrap();
        let mut memory = Memory::open_with_storage(&db, storage).unwrap();
        memory
            .capture(Capture {
                source: fixture::source_key("cli-evidence"),
                text: "The authored zeolite calibration succeeded.".into(),
                occurred_at: None,
                scopes: fixture::access().scopes,
                transcript: None,
            })
            .unwrap();
    }
    for command in ["query", "recall"] {
        let found = invoke(&[command, "zeolite"]);
        assert!(
            found.status.success(),
            "{}",
            String::from_utf8_lossy(&found.stderr)
        );
        assert!(String::from_utf8_lossy(&found.stdout).contains("zeolite"));
    }
    let forgotten = invoke(&["forget", "cli-evidence"]);
    assert!(
        forgotten.status.success(),
        "{}",
        String::from_utf8_lossy(&forgotten.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&forgotten.stdout).unwrap();
    assert_eq!(report["revisions"], 1);
    let absent = invoke(&["query", "zeolite"]);
    assert!(absent.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&absent.stdout).unwrap(),
        serde_json::json!([])
    );
    let unkeyed = f.run(&[
        "memory",
        "--experimental",
        "--db",
        db.to_str().unwrap(),
        "inspect",
    ]);
    assert!(!unkeyed.status.success());
}

#[cfg(all(unix, feature = "memory-prototype"))]
#[test]
fn encrypted_backup_is_explicit_no_clobber_and_reopens_in_another_process() {
    use std::{fs, os::unix::fs::DirBuilderExt};
    let f = Fixture::new();
    let root = f.0.canonicalize().unwrap();
    let data = root.join("snapshot-data");
    let owner = root.join("snapshot-owner");
    fs::create_dir(&data).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&owner).unwrap();
    let db = data.join("source.memory-prototype.sqlite");
    let destination = data.join("snapshot.memory-prototype.sqlite");
    let source_key = owner.join("source.key");
    let target_key = owner.join("target.key");
    let invoke = |database: &std::path::Path, key: &std::path::Path, suffix: &[&str]| {
        let mut args = vec![
            "memory",
            "--experimental",
            "--db",
            database.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ];
        args.extend_from_slice(suffix);
        f.run(&args)
    };
    for key in [&source_key, &target_key] {
        let result = invoke(&db, key, &["key-generate"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let authority = data.join("authority");
    let init = invoke(
        &db,
        &source_key,
        &[
            "init-native",
            "--synthetic-input",
            "--domain",
            "synthetic-snapshot",
            "--authority",
            authority.to_str().unwrap(),
        ],
    );
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let before = fs::read(&db).unwrap();
    let args = [
        "backup",
        "--destination",
        destination.to_str().unwrap(),
        "--destination-key",
        target_key.to_str().unwrap(),
    ];
    let backup = invoke(&db, &source_key, &args);
    assert!(
        backup.status.success(),
        "{}",
        String::from_utf8_lossy(&backup.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&backup.stdout).unwrap();
    assert_eq!(report["schema_version"], 8);
    assert!(report["scope"]
        .as_str()
        .unwrap()
        .contains("excludes keys, JSONL"));
    let inspect = invoke(&destination, &target_key, &["inspect"]);
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&init.stdout).unwrap()["snapshot"],
        serde_json::from_slice::<serde_json::Value>(&inspect.stdout).unwrap()["snapshot"]
    );
    let snapshot_bytes = fs::read(&destination).unwrap();
    assert!(!snapshot_bytes.starts_with(b"SQLite format 3\0"));
    assert!(!invoke(&destination, &source_key, &["inspect"])
        .status
        .success());
    assert!(!invoke(&db, &source_key, &args).status.success());
    assert_eq!(fs::read(&destination).unwrap(), snapshot_bytes);
    assert_eq!(fs::read(&db).unwrap(), before);
    let unsafe_key = data.join("unsafe.key");
    fs::copy(&target_key, &unsafe_key).unwrap();
    let unsafe_destination = data.join("unsafe.memory-prototype.sqlite");
    let denied = invoke(
        &db,
        &source_key,
        &[
            "backup",
            "--destination",
            unsafe_destination.to_str().unwrap(),
            "--destination-key",
            unsafe_key.to_str().unwrap(),
        ],
    );
    assert!(!denied.status.success());
    assert!(!unsafe_destination.exists());
    for output in [&backup, &inspect, &denied] {
        for key in [&source_key, &target_key] {
            let secret = fs::read_to_string(key).unwrap();
            assert!(!String::from_utf8_lossy(&output.stdout).contains(secret.trim()));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(secret.trim()));
        }
    }
}
#[cfg(all(unix, feature = "memory-prototype"))]
#[test]
fn encrypted_restore_requires_current_authority_and_preserves_forgetting_across_processes() {
    use mecha_core::memory::{fixture, Capture, KeyFilePolicy, Memory, StorageAccess};
    use std::{fs, os::unix::fs::DirBuilderExt};
    let f = Fixture::new();
    let root = f.0.canonicalize().unwrap();
    let data = root.join("restore-data");
    let owner = root.join("restore-owner");
    fs::create_dir(&data).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&owner).unwrap();
    let current = data.join("current.memory-prototype.sqlite");
    let backup = data.join("backup.memory-prototype.sqlite");
    let destination = data.join("restored.memory-prototype.sqlite");
    let keys: Vec<_> = ["current", "backup", "output"]
        .iter()
        .map(|s| owner.join(format!("{s}.key")))
        .collect();
    let policy = KeyFilePolicy::new(std::slice::from_ref(&data)).unwrap();
    let caps: Vec<_> = keys
        .iter()
        .map(|k| StorageAccess::generate_key_file(k, &policy).unwrap())
        .collect();
    let mut db =
        Memory::create_supported_situated_with_storage(&current, "synthetic-demo", caps[0].clone())
            .unwrap();
    let input = |id: &str| Capture {
        source: fixture::source_key(id),
        text: format!("Synthetic zeolite {id}"),
        occurred_at: None,
        scopes: fixture::access().scopes,
        transcript: None,
    };
    db.capture(input("old")).unwrap();
    let invoke = |database: &std::path::Path, key: &std::path::Path, suffix: &[&str]| {
        let mut args = vec![
            "memory",
            "--experimental",
            "--db",
            database.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ];
        args.extend_from_slice(suffix);
        f.run(&args)
    };
    assert!(invoke(
        &current,
        &keys[0],
        &[
            "backup",
            "--destination",
            backup.to_str().unwrap(),
            "--destination-key",
            keys[1].to_str().unwrap()
        ]
    )
    .status
    .success());
    db.capture(input("later")).unwrap();
    for id in ["old", "later"] {
        assert!(invoke(&current, &keys[0], &["forget", id]).status.success());
    }
    drop(db);
    let args = [
        "restore",
        "--current",
        current.to_str().unwrap(),
        "--current-key",
        keys[0].to_str().unwrap(),
        "--destination",
        destination.to_str().unwrap(),
        "--destination-key",
        keys[2].to_str().unwrap(),
    ];
    let result = invoke(&backup, &keys[1], &args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["snapshot_sequence"], 1);
    assert_eq!(report["authority_sequence"], 2);
    assert_eq!(report["security_epoch"], 2);
    assert!(report["scope"]
        .as_str()
        .unwrap()
        .contains("offline historical"));
    assert!(invoke(&destination, &keys[2], &["inspect"])
        .status
        .success());
    let query = invoke(&destination, &keys[2], &["query", "zeolite"]);
    assert!(query.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&query.stdout).unwrap(),
        serde_json::json!([])
    );
    let mut restored = Memory::open_with_storage(&destination, caps[2].clone()).unwrap();
    for id in ["old", "later"] {
        assert!(restored.capture(input(id)).is_err());
    }
    let bytes = fs::read(&destination).unwrap();
    assert!(!invoke(&backup, &keys[1], &args).status.success());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    let unsafe_key = data.join("unsafe.key");
    fs::copy(&keys[0], &unsafe_key).unwrap();
    let denied = invoke(
        &backup,
        &keys[1],
        &[
            "restore",
            "--current",
            current.to_str().unwrap(),
            "--current-key",
            unsafe_key.to_str().unwrap(),
            "--destination",
            data.join("denied.memory-prototype.sqlite")
                .to_str()
                .unwrap(),
            "--destination-key",
            keys[2].to_str().unwrap(),
        ],
    );
    assert!(!denied.status.success());
    assert!(!data.join("denied.memory-prototype.sqlite").exists());
    for output in [&result, &query, &denied] {
        for key in &keys {
            let secret = fs::read_to_string(key).unwrap();
            assert!(!String::from_utf8_lossy(&output.stdout).contains(secret.trim()));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(secret.trim()));
        }
    }
}

#[cfg(feature = "memory-prototype")]
#[test]
fn hybrid_cli_requires_explicit_fixture_and_encoding_identity() {
    let f = Fixture::new();
    let db = f.0.join("hybrid.memory-prototype.sqlite");
    let base = ["memory", "--experimental", "--db", db.to_str().unwrap()];
    let flags = [
        "--synthetic-input",
        "--endpoint",
        "http://127.0.0.1:1",
        "--expected-alias",
        "synthetic-encoder",
        "--artifact-revision",
        "synthetic-artifact",
        "--dimensions",
        "2",
        "--pooling",
        "last",
        "--normalization",
        "l2",
    ];
    for (flag, width) in [(0, 1), (1, 2), (3, 2), (5, 2), (7, 2), (9, 2), (11, 2)] {
        let mut argv = base.to_vec();
        argv.push("evaluate-hybrid");
        argv.extend(&flags[..flag]);
        argv.extend(&flags[flag + width..]);
        let output = f.run(&argv);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("required arguments") && stderr.contains(flags[flag]),
            "{stderr}"
        );
    }
    let mut argv = vec!["memory", "--db", db.to_str().unwrap(), "evaluate-hybrid"];
    argv.extend(flags);
    let output = f.run(&argv);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--experimental"));
    assert!(!db.exists());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn hybrid_cli_refusals_leave_destination_and_config_untouched() {
    let f = Fixture::new();
    let db = f.0.join("hybrid.memory-prototype.sqlite");
    let wrong_suffix = f.0.join("hybrid.sqlite");
    let existing = f.0.join("existing.memory-prototype.sqlite");
    std::fs::write(&existing, b"existing file").unwrap();
    for (path, endpoint, dimensions, key, expected) in [
        (&db, "http://localhost:1", "2", false, "literal loopback"),
        (&db, "http://192.0.2.1:1", "2", false, "literal loopback"),
        (
            &db,
            "http://name:secret@127.0.0.1:1",
            "2",
            false,
            "literal loopback",
        ),
        (&db, "http://127.0.0.1:1", "0", false, "dimensions"),
        (
            &db,
            "http://127.0.0.1:1",
            "2",
            true,
            "does not support --key-file",
        ),
        (
            &wrong_suffix,
            "http://127.0.0.1:1",
            "2",
            false,
            "memory-prototype.sqlite",
        ),
        (
            &existing,
            "http://127.0.0.1:1",
            "2",
            false,
            "requires a new file",
        ),
    ] {
        let mut argv = vec!["memory", "--experimental", "--db", path.to_str().unwrap()];
        if key {
            argv.extend(["--key-file", "unreadable-key-must-not-be-opened"]);
        }
        argv.extend([
            "evaluate-hybrid",
            "--synthetic-input",
            "--endpoint",
            endpoint,
            "--expected-alias",
            "synthetic-encoder",
            "--artifact-revision",
            "synthetic-artifact",
            "--dimensions",
            dimensions,
            "--pooling",
            "last",
            "--normalization",
            "l2",
        ]);
        let output = f.run(&argv);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.contains("secret"), "{stderr}");
    }
    assert_eq!(std::fs::read(existing).unwrap(), b"existing file");
    assert!(!wrong_suffix.exists());
    assert!(!db.exists());
    assert_eq!(std::fs::read_dir(f.0.join("home")).unwrap().count(), 1);
}

#[cfg(feature = "memory-prototype")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hybrid_cli_uses_one_checked_loopback_operation_per_nonempty_pool() {
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let submitted = Arc::new(Mutex::new(Vec::new()));
    let spy = Arc::clone(&submitted);
    let app = Router::new().route(
        "/v1/embeddings",
        post(move |Json(request): Json<Value>| {
            let spy = Arc::clone(&spy);
            async move {
                let inputs = request["input"].as_array().unwrap();
                let data: Vec<_> = (0..inputs.len())
                    .rev()
                    .map(|index| json!({"index":index,"embedding":[1.0,0.0]}))
                    .collect();
                spy.lock().unwrap().push(request);
                Json(json!({"model":"synthetic-encoder","data":data}))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let f = Fixture::new();
    let args = [
        "evaluate-hybrid",
        "--synthetic-input",
        "--endpoint",
        &endpoint,
        "--expected-alias",
        "synthetic-encoder",
        "--artifact-revision",
        "synthetic-artifact",
        "--dimensions",
        "2",
        "--pooling",
        "last",
        "--normalization",
        "l2",
    ];
    let report = f.memory(&args);
    assert_eq!(report["fixture_only"], true);
    assert_eq!(
        report["encoding"]["host_declared_artifact_revision"],
        "synthetic-artifact"
    );
    for arm in report["modes"].as_array().unwrap() {
        assert_eq!(arm["planned"], 18);
        assert_eq!(arm["completed"], 18);
        assert_eq!(arm["fallback"], 0);
        assert_eq!(arm["unauthorized_returns"], 0);
    }
    assert_eq!(
        report["cases"][0]["shared"]["embedding"]["server_reported_alias"],
        "synthetic-encoder"
    );
    assert_eq!(report["cases"][0]["shared"]["embedding"]["attempts"], 3);
    assert_eq!(report["cases"][17]["shared"]["embedding"]["attempts"], 0);

    let db = f.0.join("test.memory-prototype.sqlite");
    let original = std::fs::read(&db).unwrap();
    let mut repeated = vec!["memory", "--experimental", "--db", db.to_str().unwrap()];
    repeated.extend(args);
    assert!(!f.run(&repeated).status.success());
    assert_eq!(std::fs::read(db).unwrap(), original);
    assert_eq!(std::fs::read_dir(f.0.join("home")).unwrap().count(), 1);

    server.abort();
    let requests = submitted.lock().unwrap();
    assert_eq!(
        requests.len(),
        43,
        "no duplicate arm calls or overwritten fixture"
    );
    let mut query_count = 0;
    for request in requests.iter() {
        assert_eq!(request["model"], "synthetic-encoder");
        let keys = request.as_object().unwrap();
        assert_eq!(
            keys.len(),
            3,
            "only model identity, encoding format and text reach the server"
        );
        assert_eq!(request["encoding_format"], "float");
        let input = request["input"].as_array().unwrap();
        assert!((1..=16).contains(&input.len()));
        for (index, text) in input.iter().enumerate() {
            let text = text.as_str().unwrap();
            if text.starts_with("Instruct: ") {
                assert_eq!(index, 0);
                query_count += 1;
            }
            assert!(!text.contains("compartment 17"));
        }
    }
    assert_eq!(query_count, 17);
    assert_eq!(
        requests[0]["input"][0],
        "Instruct: Given a question about a document, retrieve the passages of it that answer the question\nQuery: How should outdoor electronics stay dry through downpours?"
    );
    assert!(requests[..3].iter().any(|request| {
        request["input"].as_array().unwrap().iter().any(|text| {
            text == "A rubber gasket seats inside the enclosure lid and blocks rainwater ingress."
        })
    }));
}
