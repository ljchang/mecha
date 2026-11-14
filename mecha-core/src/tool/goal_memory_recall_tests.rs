use super::*;
use crate::{
    embed::EncodingProfileSpec,
    memory::{Access, Capture, SourceKey},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};

struct Fixture {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
    db: Memory,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("native-recall-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("fixture.memory-prototype.sqlite");
        let mut db = Memory::create(&path, "fixture").unwrap();
        db.capture(capture("chat")).unwrap();
        Self { dir, path, db }
    }
    fn tool(&self) -> GoalMemory {
        GoalMemory::open(
            &self.path,
            Access {
                domain: "fixture".into(),
                scopes: ["chat".into()].into(),
            },
            "synthetic".into(),
            Situation {
                event: "migration".into(),
                goal: "task:migration".into(),
                needs: None,
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn capture(scope: &str) -> Capture {
    Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "synthetic".into(),
            id: "one".into(),
        },
        text: "UP4 retained every column".into(),
        occurred_at: None,
        scopes: [scope.into()].into(),
        transcript: None,
    }
}
fn profile() -> EncodingProfile {
    EncodingProfile::new(EncodingProfileSpec {
        expected_alias: "authored-model".into(),
        artifact_revision: "fixture-v1".into(),
        dimensions: 2,
        pooling: "last".into(),
        normalization: "server-l2".into(),
        query_instruction: crate::embed::QUERY_INSTRUCTION.into(),
        query_template: "Instruct: {instruction}\nQuery: {text}".into(),
        preprocessing_version: "verbatim-v1".into(),
        representation_version: "original-evidence-spans-v1".into(),
    })
    .unwrap()
}
const QUERY: &str = "preserving complete structure";
fn input() -> Value {
    json!({"memory":{"query":QUERY}})
}
struct Server {
    endpoint: String,
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(hold: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (c, e, r) = (calls.clone(), entered.clone(), release.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            let body = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    return;
                }
                bytes.extend_from_slice(&buf[..n]);
                if let Some(pos) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..pos]);
                    let len: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= pos + 4 + len {
                        break serde_json::from_slice::<Value>(&bytes[pos + 4..pos + 4 + len])
                            .unwrap();
                    }
                }
            };
            c.fetch_add(1, Ordering::SeqCst);
            e.notify_one();
            if hold {
                r.notified().await;
            }
            let texts = body["input"].as_array().unwrap();
            assert!(texts[0].as_str().unwrap().contains(QUERY));
            let data: Vec<_> = texts
                .iter()
                .enumerate()
                .map(|(index, _)| json!({"index":index,"embedding":[1.0,0.0]}))
                .collect();
            let body = json!({"model":"authored-model","data":data}).to_string();
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    Server {
        endpoint,
        calls,
        entered,
        release,
        task,
    }
}

#[tokio::test]
async fn native_recall_dense_delivers_original_lexical_misses_with_same_visible_contract() {
    let fixture = Fixture::new();
    let server = server(false).await;
    let (lexical, lh) = fixture
        .tool()
        .with_native_recall(NativeRecallConfig::lexical(QUERY.into()).unwrap())
        .unwrap();
    let (dense, dh) = fixture
        .tool()
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    assert_eq!(lexical.description(), dense.description());
    assert_eq!(lexical.input_schema(), dense.input_schema());
    assert!(dense.input_schema()["properties"]["memory"]["properties"]["bindings"].is_null());
    let l = lexical.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(!l.is_error && !l.external);
    assert!(!l.content.contains("UP4 retained"));
    let d = dense.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(!d.is_error && d.external);
    assert!(d.content.contains("UP4 retained every column"));
    assert!(!d.content.contains("authored-model"));
    assert!(lh.report().protocol_satisfied());
    assert!(dh.report().protocol_satisfied());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    assert_eq!(dh.report().embedding.attempts, Some(1));
}

#[tokio::test]
async fn native_recall_invalid_first_call_consumes_slot_and_legacy_remains_repeatable() {
    let fixture = Fixture::new();
    for bad in [
        json!({}),
        json!({"memory":{"query":"wrong"}}),
        json!({"memory":{"query":QUERY,"bindings":[]}}),
        json!({"serves":"task:other","memory":{"query":QUERY}}),
    ] {
        let (tool, handle) = fixture
            .tool()
            .with_native_recall(NativeRecallConfig::lexical(QUERY.into()).unwrap())
            .unwrap();
        assert!(tool.call(bad, &ToolCtx::default()).await.unwrap().is_error);
        let second = tool.call(input(), &ToolCtx::default()).await.unwrap();
        assert!(second.is_error && !second.external);
        let report = handle.report();
        assert_eq!(report.entered_calls, 2);
        assert_eq!(report.rejected_calls, 2);
        assert_eq!(report.admitted_calls, 0);
        assert!(!report.protocol_satisfied());
        assert_eq!(report.status, NativeRecallStatus::Rejected);
    }
    let legacy = fixture.tool();
    for _ in 0..2 {
        assert!(
            !legacy
                .call(json!({}), &ToolCtx::default())
                .await
                .unwrap()
                .is_error
        );
    }
}

#[tokio::test]
async fn native_recall_inflight_call_does_not_hold_sqlite_or_tool_mutex_and_rechecks_scopes() {
    let mut fixture = Fixture::new();
    let server = server(true).await;
    // DELETE journal makes a live reader obstruct EXCLUSIVE: verify the negative control.
    let writer = rusqlite::Connection::open(&fixture.path).unwrap();
    writer.busy_timeout(std::time::Duration::ZERO).unwrap();
    writer.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
    let reader = rusqlite::Connection::open(&fixture.path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM revision")
        .unwrap();
    assert!(writer.execute_batch("BEGIN EXCLUSIVE").is_err());
    reader.execute_batch("ROLLBACK").unwrap();
    let (tool, handle) = fixture
        .tool()
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    let tool = Arc::new(tool);
    let t = tool.clone();
    let task = tokio::spawn(async move { t.call(input(), &ToolCtx::default()).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), server.entered.notified())
        .await
        .expect("authored server must receive checked request");
    assert!(
        tool.memory.try_lock().is_ok(),
        "embedding must release tool mutex"
    );
    writer.execute_batch("BEGIN EXCLUSIVE; ROLLBACK").unwrap();
    let rejected = tool.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(rejected.is_error && !rejected.external);
    fixture.db.capture(capture("restricted")).unwrap();
    server.release.notify_one();
    let output = task.await.unwrap();
    assert!(!output.is_error && !output.external);
    assert!(!output.content.contains("UP4 retained"));
    let report = handle.report();
    assert_eq!(report.entered_calls, 2);
    assert_eq!(report.admitted_calls, 1);
    assert_eq!(report.rejected_calls, 1);
    assert_eq!(report.fallback, Some(HybridFallback::CorpusChanged));
    assert!(!report.protocol_satisfied());
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_recall_unavailable_transport_is_explicit_fallback_with_unknown_cost() {
    let fixture = Fixture::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let (tool, handle) = fixture
        .tool()
        .with_native_recall(NativeRecallConfig::dense(QUERY.into(), endpoint, profile()).unwrap())
        .unwrap();
    let output = tool.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(!output.is_error);
    let report = handle.report();
    assert_eq!(report.status, NativeRecallStatus::Fallback);
    assert_eq!(report.fallback, Some(HybridFallback::EmbeddingsUnavailable));
    assert_eq!(report.embedding.attempts, None);
    assert!(!report.protocol_satisfied());
}

#[tokio::test]
async fn native_recall_cancel_and_drop_record_interruption_without_late_delivery() {
    for drop_future in [false, true] {
        let fixture = Fixture::new();
        let server = server(true).await;
        let (tool, handle) = fixture
            .tool()
            .with_native_recall(
                NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile())
                    .unwrap(),
            )
            .unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let ctx = ToolCtx {
            cancel: Some(token.clone()),
            ..ToolCtx::default()
        };
        let tool = Arc::new(tool);
        let t = tool.clone();
        let task = tokio::spawn(async move { t.call(input(), &ctx).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(5), server.entered.notified())
            .await
            .expect("authored server must receive checked request");
        if drop_future {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert_eq!(handle.report().status, NativeRecallStatus::Interrupted);
        } else {
            token.cancel();
            let output = task.await.unwrap();
            assert!(output.is_error && !output.external);
            assert_eq!(handle.report().status, NativeRecallStatus::Cancelled);
        }
        server.release.notify_one();
        assert!(
            tool.call(input(), &ToolCtx::default())
                .await
                .unwrap()
                .is_error
        );
        assert!(!handle.report().protocol_satisfied());
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn native_recall_deadline_withholds_late_embedding() {
    let fixture = Fixture::new();
    let server = server(true).await;
    let (tool, handle) = fixture
        .tool()
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    let started = std::time::Instant::now();
    let output = tool.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(output.is_error && !output.external);
    assert!(started.elapsed() >= Duration::from_secs(15));
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(handle.report().status, NativeRecallStatus::TimedOut);
    server.release.notify_one();
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn native_recall_constructor_refuses_nonliteral_endpoints_and_bad_queries() {
    for endpoint in [
        "http://localhost:8081",
        "https://127.0.0.1:8081",
        "http://127.0.0.1:8081?key=x",
        "http://example.org",
    ] {
        assert!(NativeRecallConfig::dense(QUERY.into(), endpoint.into(), profile()).is_err());
    }
    for query in ["".to_string(), "  ".to_string(), "x".repeat(4097)] {
        assert!(NativeRecallConfig::lexical(query).is_err());
    }
}

#[tokio::test]
async fn native_recall_live_external_authority_withdrawal_during_embedding_withholds_source() {
    use crate::memory::host_capture::{ExternalBinding, ExternalSourceGuard, SourceStatus};
    struct Guard(std::sync::atomic::AtomicBool);
    impl ExternalSourceGuard for Guard {
        fn check(&self, _: &ExternalBinding) -> SourceStatus {
            if self.0.load(Ordering::SeqCst) {
                SourceStatus::Checked
            } else {
                SourceStatus::Withheld
            }
        }
    }
    let fixture = Fixture::new();
    let sql = rusqlite::Connection::open(&fixture.path).unwrap();
    sql.execute_batch("CREATE TABLE external_binding(revision TEXT PRIMARY KEY,authority TEXT,locator TEXT,binding_revision TEXT); INSERT INTO external_binding SELECT id,'fixture-host','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' FROM revision;").unwrap();
    let guard = Arc::new(Guard(std::sync::atomic::AtomicBool::new(true)));
    let tool = GoalMemory::open_with_source_guard(
        &fixture.path,
        Access {
            domain: "fixture".into(),
            scopes: ["chat".into()].into(),
        },
        "synthetic".into(),
        Situation {
            event: "migration".into(),
            goal: "task:migration".into(),
            needs: None,
        },
        guard.clone(),
    )
    .unwrap();
    let server = server(true).await;
    let (tool, handle) = tool
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    let task = tokio::spawn(async move { tool.call(input(), &ToolCtx::default()).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), server.entered.notified())
        .await
        .expect("authored server must receive checked request");
    guard.0.store(false, Ordering::SeqCst);
    server.release.notify_one();
    let output = task.await.unwrap();
    assert!(!output.is_error && !output.external);
    assert!(!output.content.contains("UP4 retained"));
    assert_eq!(
        handle.report().fallback,
        Some(HybridFallback::CorpusChanged)
    );
    assert!(!handle.report().protocol_satisfied());
}

#[tokio::test]
async fn native_recall_empty_pool_completes_without_http_and_duplicate_preserves_first_status() {
    let mut fixture = Fixture::new();
    fixture.db.forget(&capture("chat").source).unwrap();
    let server = server(false).await;
    let (tool, handle) = fixture
        .tool()
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    let output = tool.call(input(), &ToolCtx::default()).await.unwrap();
    assert!(!output.is_error && !output.external);
    assert!(handle.report().protocol_satisfied());
    assert_eq!(handle.report().embedding.attempts, Some(0));
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(
        tool.call(input(), &ToolCtx::default())
            .await
            .unwrap()
            .is_error
    );
    let report = handle.report();
    assert_eq!(report.status, NativeRecallStatus::Completed);
    assert_eq!(report.rejected_calls, 1);
    assert!(!report.protocol_satisfied());
}

#[test]
fn native_recall_interpretations_refused_in_both_builder_orders() {
    use crate::memory::{host_capture::ConfirmedCriterion, interpretation::InterpretationContext};
    let fixture = Fixture::new();
    let read = InterpretationContext::new(
        ConfirmedCriterion {
            goal: "task:migration".into(),
            goal_text: "migration".into(),
            criterion_text: "preserve all fields".into(),
            criterion_revision: "a".repeat(64),
            confirmation_digest: "b".repeat(64),
        },
        uuid::Uuid::new_v4().to_string(),
    )
    .unwrap()
    .read_policy();
    assert!(fixture
        .tool()
        .with_interpretations(read.clone())
        .unwrap()
        .with_native_recall(NativeRecallConfig::lexical(QUERY.into()).unwrap())
        .is_err());
    let (tool, _) = fixture
        .tool()
        .with_native_recall(NativeRecallConfig::lexical(QUERY.into()).unwrap())
        .unwrap();
    assert!(tool.with_interpretations(read).is_err());
}

#[test]
fn native_recall_preflight_rejects_unsearchable_and_over_term_queries() {
    for query in ["!!!".to_string(), vec!["term"; 33].join(" ")] {
        assert!(NativeRecallConfig::lexical(query.clone()).is_err());
        assert!(
            NativeRecallConfig::dense(query, "http://127.0.0.1:8081".into(), profile()).is_err()
        );
    }
}

#[tokio::test]
async fn native_recall_late_ready_publication_rechecks_deadline_and_cancellation() {
    for cancellation in [false, true] {
        let fixture = Fixture::new();
        let tool = fixture.tool();
        let control = NativeRecallControl::new(NativeRecallConfig::lexical("UP4".into()).unwrap());
        let mut attempt = control.claim().unwrap();
        control.handle.0.lock().unwrap().admitted_calls = 1;
        let mut policy = tool.policy.clone();
        policy.valid_at = Some(tool.clock.now().to_rfc3339());
        let result = tool
            .memory
            .lock()
            .unwrap()
            .lexical_hybrid_context(
                &ContextRequest {
                    query: "UP4".into(),
                    bindings: vec![],
                },
                &policy,
                &tool.situation,
                None,
            )
            .unwrap();
        assert!(!result.ranked_anchors.is_empty());
        let cancel = tokio_util::sync::CancellationToken::new();
        // The future is Ready in its first poll, but expires/cancels during that poll.
        if !cancellation {
            attempt.started = Instant::now() - RETRIEVAL_DEADLINE + Duration::from_millis(10);
        }
        let future = async {
            if cancellation {
                cancel.cancel();
            } else {
                std::thread::sleep(Duration::from_millis(30));
            }
            Ok((
                ToolOutput::ok("must not be delivered").from_outside(),
                result,
            ))
        };
        let output = attempt.complete(future, &cancel).await.unwrap();
        assert!(
            output.is_error && !output.external,
            "a winning Ready future cannot publish after interruption"
        );
        let report = control.handle.report();
        assert!(report.delivered.is_empty());
        assert_eq!(report.effective_mode, None);
        assert_eq!(
            report.status,
            if cancellation {
                NativeRecallStatus::Cancelled
            } else {
                NativeRecallStatus::TimedOut
            }
        );
        assert!(!report.protocol_satisfied());
    }
}

#[tokio::test]
async fn native_recall_dense_refreshes_validity_after_embedding_without_advancing_knowledge() {
    use crate::memory::{Bound, Interval, RelationInput, Span, Target};
    let mut fixture = Fixture::new();
    let source = fixture.db.capture(capture("chat")).unwrap();
    let entity = fixture.db.entity("UP4", Default::default()).unwrap();
    let relation = fixture
        .db
        .relate(RelationInput {
            predicate: "retains".into(),
            subject: entity,
            object: Target::Literal("columns".into()),
            support: Span {
                evidence: source.clone(),
                start: 0,
                end: 24,
            },
            consulted: vec![],
            text: "UP4 retains columns".into(),
            valid: Interval {
                from: Bound::Known("2026-06-01T00:00:00Z".into()),
                to: Bound::Known("2026-07-01T00:00:00Z".into()),
            },
            polarity: 1,
        })
        .unwrap();
    let clock = Arc::new(crate::clock::TestClock::at("2026-06-30T23:59:59Z"));
    let open = || {
        GoalMemory::open_with_clock(
            &fixture.path,
            Access {
                domain: "fixture".into(),
                scopes: ["chat".into()].into(),
            },
            "synthetic".into(),
            Situation {
                event: "migration".into(),
                goal: "task:migration".into(),
                needs: None,
            },
            clock.clone(),
        )
        .unwrap()
    };
    let before = open()
        .call(json!({"memory":{"query":"UP4"}}), &ToolCtx::default())
        .await
        .unwrap();
    assert!(before.content.contains(&relation.revision));
    let snapshot = fixture.db.snapshot().unwrap();
    let server = server(true).await;
    let (tool, handle) = open()
        .with_native_recall(
            NativeRecallConfig::dense(QUERY.into(), server.endpoint.clone(), profile()).unwrap(),
        )
        .unwrap();
    let task = tokio::spawn(async move { tool.call(input(), &ToolCtx::default()).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(5), server.entered.notified())
        .await
        .unwrap();
    clock.advance(chrono::Duration::seconds(1));
    server.release.notify_one();
    let output = task.await.unwrap();
    assert!(!output.is_error && output.external);
    let packet: Value = serde_json::from_str(&output.content).unwrap();
    assert!(output.content.contains(&source.revision));
    assert!(
        !output.content.contains(&relation.revision),
        "relation expired during embedding must not be delivered"
    );
    assert_eq!(packet["memory"]["known_at"], json!(snapshot));
    let valid_at: chrono::DateTime<chrono::Utc> = packet["memory"]["valid_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        valid_at,
        "2026-07-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
    assert!(handle.report().protocol_satisfied());
}
