#[tokio::test]
async fn native_controlled_recall_records_one_completed_empty_read() -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    let task_root = root.join("controlled");
    let report = run_native_session_with_recall(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        NativeRecallConfig::lexical("calibration".into())?,
        &task_root,
        |_| async { Ok(script(Some("Quill"))) },
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(report.check.verdict, CheckVerdict::Passed);
    assert!(report.receipt.is_some(), "{report:?}");
    let value = serde_json::to_value(&report)?;
    assert!(
        value["recall"].is_object(),
        "controlled read must retain its host audit"
    );
    assert!(report.recall.as_ref().unwrap().protocol_satisfied());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn native_recall_wire_rejects_unknown_fields_and_invalid_host_settings() {
    for value in [
        json!({"mode":"lexical","query":"calibration","endpoint":"http://127.0.0.1"}),
        json!({"mode":"lexical","query":"calibration","bindings":[]}),
        json!({"mode":"hybrid","query":"calibration"}),
    ] {
        assert!(serde_json::from_value::<NativeRecallRequest>(value).is_err());
    }
    for value in [
        json!({"mode":"lexical","query":" "}),
        json!({"mode":"dense","query":"calibration","endpoint":"https://example.invalid","expected_alias":"fixture","artifact_revision":"fixture-1","dimensions":2,"pooling":"last","normalization":"l2"}),
        json!({"mode":"dense","query":"calibration","endpoint":"http://127.0.0.1:1","expected_alias":"fixture","artifact_revision":"fixture-1","dimensions":0,"pooling":"last","normalization":"l2"}),
    ] {
        assert!(serde_json::from_value::<NativeRecallRequest>(value)
            .unwrap()
            .into_config()
            .is_err());
    }
}

#[tokio::test]
async fn native_controlled_recall_refuses_interpretations_before_discovery_or_root() -> Result<()> {
    let (root, database, authority, mut request) = native_fixture();
    request.interpretation_context_id = Some("incompatible".into());
    let task = root.join("never-started");
    let result = run_native_session_with_recall(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        NativeRecallConfig::lexical("calibration".into())?,
        &task,
        |_| async { panic!("incompatible recall must not discover a provider") },
        &CancellationToken::new(),
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("cannot combine"));
    assert!(!task.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_controlled_recall_identity_precedes_discovery_and_cannot_replay() -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    let task = root.join("started");
    let cancel = CancellationToken::new();
    let run = || {
        run_native_session_with_recall(
            &database,
            StorageAccess::plaintext_fixture(),
            &authority,
            &request,
            NativeRecallConfig::lexical("calibration".into()).unwrap(),
            &task,
            |_| async {
                let started: serde_json::Value =
                    serde_json::from_slice(&fs::read(task.join("started.json"))?)?;
                assert_eq!(started["recall"]["protocol"], "native-recall-v1");
                assert_eq!(started["recall"]["mode"], "lexical");
                assert_eq!(
                    started["recall"]["query_digest"].as_str().unwrap().len(),
                    64
                );
                anyhow::bail!("authored discovery refusal")
            },
            &cancel,
        )
    };
    assert!(run()
        .await
        .unwrap_err()
        .to_string()
        .contains("authored discovery refusal"));
    let original = fs::read(task.join("report.json"))?;
    assert!(run()
        .await
        .unwrap_err()
        .to_string()
        .contains("NEW absolute task directory"));
    assert_eq!(fs::read(task.join("report.json"))?, original);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_controlled_recall_does_not_qualify_correct_guesses_or_extra_calls() -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    for extra in [false, true] {
        let mut messages = vec![];
        if extra {
            messages.push(Message::assistant(vec![
                Block::ToolUse {
                    id: "read-1".into(),
                    name: "goal_context".into(),
                    input: json!({"memory":{"query":"calibration"}}),
                },
                Block::ToolUse {
                    id: "read-2".into(),
                    name: "goal_context".into(),
                    input: json!({"memory":{"query":"calibration"}}),
                },
            ]));
        }
        messages.push(call(
            "write",
            "fs_write",
            json!({"path":"output.json","content":r#"[{"route":"Quill"}]"#}),
        ));
        messages.push(Message::assistant(vec![Block::text("Done.")]));
        let report = run_native_session_with_recall(
            &database,
            StorageAccess::plaintext_fixture(),
            &authority,
            &request,
            NativeRecallConfig::lexical("calibration".into())?,
            &root.join(if extra { "extra" } else { "missing" }),
            |_| async { Ok(Box::new(Script::new(messages)) as Box<dyn Provider>) },
            &CancellationToken::new(),
        )
        .await?;
        assert_eq!(report.check.verdict, CheckVerdict::Passed);
        let recall = report.recall.as_ref().unwrap();
        assert!(!recall.protocol_satisfied());
        assert_eq!(recall.entered_calls, if extra { 2 } else { 0 });
        if extra {
            assert_eq!(recall.rejected_calls, 1);
            assert!(report.receipt.is_none() && report.capture_error.is_some());
        }
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

struct EvidenceChoosingProvider {
    step: Mutex<usize>,
    query: String,
    withdraw: Option<(LocalAuthority, String)>,
}

#[async_trait::async_trait]
impl Provider for EvidenceChoosingProvider {
    fn id(&self) -> &str {
        "authored-evidence-choice"
    }
    fn default_model(&self) -> &str {
        "authored"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        request: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        assert_eq!(request.think, Some(false));
        assert!(request.local_only);
        let step = {
            let mut step = self.step.lock().unwrap();
            let current = *step;
            *step += 1;
            current
        };
        let message = match step {
            0 => call(
                "recall",
                "goal_context",
                json!({"memory":{"query":self.query}}),
            ),
            1 => {
                let observed = request.messages.iter().flat_map(|m| &m.content).any(|block|
                    matches!(block, Block::ToolResult {tool_use_id, content, is_error:false} if tool_use_id == "recall" && content.contains("Quill finishes in six hours and preserves calibration rows.")));
                if let Some((authority, locator)) = &self.withdraw {
                    authority.withdraw(locator)?;
                }
                call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":json!([{"route":if observed {"Quill"} else {"unknown"}}]).to_string()}),
                )
            }
            _ => Message::assistant(vec![Block::text("Done.")]),
        };
        Script::new(vec![message]).complete(request, sink).await
    }
}

#[tokio::test]
async fn native_controlled_recall_uses_observations_and_recaptures_uncited_originals() -> Result<()>
{
    let mut fixture = EncryptedFixture::new()?;
    fixture
        .request
        .files
        .insert("observed.txt".into(), ROUTES.into());
    let source = fixture
        .run(
            "source",
            vec![
                call("observe", "fs_read", json!({"path":"observed.txt"})),
                call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":r#"[{"route":"Ilex"}]"#}),
                ),
                Message::assistant(vec![Block::text("Done.")]),
            ],
        )
        .await?;
    assert!(source.receipt.is_some(), "{source:?}");
    fixture.request.files.clear();
    let run = async |name: &'static str, withdraw: Option<(LocalAuthority, String)>| {
        run_native_session_with_recall(
            &fixture.database,
            fixture.storage().unwrap(),
            &fixture.authority,
            &fixture.request,
            NativeRecallConfig::lexical("calibration".into()).unwrap(),
            &fixture.root.join(name),
            |_| async {
                Ok(Box::new(EvidenceChoosingProvider {
                    step: Mutex::new(0),
                    query: "calibration".into(),
                    withdraw,
                }) as Box<dyn Provider>)
            },
            &CancellationToken::new(),
        )
        .await
    };
    let report = run("followup", None).await?;
    assert_eq!(report.check.verdict, CheckVerdict::Passed);
    assert!(report.recall.as_ref().unwrap().protocol_satisfied());
    assert!(report.taint.private && report.taint.untrusted);
    let receipt = report.receipt.as_ref().context("followup not captured")?;
    let memory = Memory::open_with_storage(&fixture.database, fixture.storage()?)?
        .with_source_guard(Arc::new(fixture.authority.clone()));
    let prepared = memory.prepare_interpretation(
        receipt,
        &[],
        &InterpretationContext::new(fixture.request.criterion.clone(), "retained".into())?,
        &memory.snapshot()?,
        &fixture.request.access(),
    )?;
    let source_receipt = source.receipt.as_ref().unwrap();
    assert!(prepared
        .sources()
        .iter()
        .any(|field| source_receipt.originals.contains(&field.source)
            && field.text.contains("preserves calibration rows")));
    // The provider never cites a reference; the complete consulted closure is
    // retained by the native bridge, not manufactured by the assistant.
    let locator = fixture.authority.binding(&source.confirmation)?.locator;
    let interrupted = run(
        "withdrawn-after-delivery",
        Some((fixture.authority.clone(), locator)),
    )
    .await?;
    assert_eq!(interrupted.check.verdict, CheckVerdict::Passed);
    assert!(interrupted.recall.as_ref().unwrap().protocol_satisfied());
    assert!(interrupted.receipt.is_none() && interrupted.capture_error.is_some());
    assert!(memory
        .read(&receipt.projection, &fixture.request.access())?
        .is_none());
    let audit = fs::read_to_string(fixture.root.join("followup/provider-audit.jsonl"))?;
    assert!(!audit.contains(&fs::read_to_string(&fixture.key)?.trim().to_owned()));
    Ok(())
}

struct NativeEmbeddingServer {
    endpoint: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for NativeEmbeddingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn native_embedding_server() -> NativeEmbeddingServer {
    native_embedding_server_with_hold(None).await
}
async fn native_embedding_server_with_hold(
    hold: Option<(
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
    )>,
) -> NativeEmbeddingServer {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut bytes = Vec::new();
            let request: serde_json::Value = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(pos) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..pos]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= pos + 4 + length {
                        break serde_json::from_slice(&bytes[pos + 4..pos + 4 + length]).unwrap();
                    }
                }
            };
            if let Some((entered, release, _)) = &hold {
                entered.notify_one();
                release.notified().await;
            }
            let data: Vec<_> = request["input"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let text = text.as_str().unwrap();
                    let vector = if text.starts_with("Instruct:")
                        || text.contains("Quill finishes in six hours")
                    {
                        [1.0, 0.0]
                    } else {
                        [0.0, 1.0]
                    };
                    json!({"index":index,"embedding":vector})
                })
                .collect();
            let body = json!({"model":"native-authored-embedding","data":data}).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = socket.write_all(response.as_bytes()).await;
            if let Some((_, _, sent)) = &hold {
                sent.notify_one();
            }
        }
    });
    NativeEmbeddingServer { endpoint, task }
}

#[tokio::test]
async fn native_controlled_dense_recall_crosses_vocabulary_gap_in_actual_agent_and_capture(
) -> Result<()> {
    let mut fixture = EncryptedFixture::new()?;
    fixture
        .request
        .files
        .insert("observed.txt".into(), ROUTES.into());
    let source = fixture
        .run(
            "source",
            vec![
                call("observe", "fs_read", json!({"path":"observed.txt"})),
                call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":"[]"}),
                ),
                Message::assistant(vec![Block::text("Done.")]),
            ],
        )
        .await?;
    assert!(source.receipt.is_some(), "{source:?}");
    fixture.request.files.clear();
    let server = native_embedding_server().await;
    let query = "retaining entire measurement information";
    for dense in [false, true] {
        let mode = if dense { "dense" } else { "lexical" };
        let database = fixture.root.join(format!("{mode}.memory-prototype.sqlite"));
        fs::copy(&fixture.database, &database)?;
        let wire = if dense {
            json!({"mode":"dense","query":query,"endpoint":server.endpoint,"expected_alias":"native-authored-embedding","artifact_revision":"fixture-v1","dimensions":2,"pooling":"last","normalization":"l2"})
        } else {
            json!({"mode":"lexical","query":query})
        };
        let recall = serde_json::from_value::<NativeRecallRequest>(wire)?.into_config()?;
        let report = run_native_session_with_recall(
            &database,
            fixture.storage()?,
            &fixture.authority,
            &fixture.request,
            recall,
            &fixture.root.join(mode),
            |_| async {
                Ok(Box::new(EvidenceChoosingProvider {
                    step: Mutex::new(0),
                    query: query.into(),
                    withdraw: None,
                }) as Box<dyn Provider>)
            },
            &CancellationToken::new(),
        )
        .await?;
        assert!(
            report.recall.as_ref().unwrap().protocol_satisfied(),
            "{report:?}"
        );
        assert_eq!(
            report.check.verdict,
            if dense {
                CheckVerdict::Passed
            } else {
                CheckVerdict::Failed
            }
        );
        let receipt = report
            .receipt
            .as_ref()
            .context("controlled capture failed")?;
        if dense {
            assert!(report.taint.private && report.taint.untrusted);
            let memory = Memory::open_with_storage(&database, fixture.storage()?)?
                .with_source_guard(Arc::new(fixture.authority.clone()));
            let prepared = memory.prepare_interpretation(
                receipt,
                &[],
                &InterpretationContext::new(
                    fixture.request.criterion.clone(),
                    "dense-recapture".into(),
                )?,
                &memory.snapshot()?,
                &fixture.request.access(),
            )?;
            assert!(prepared.sources().iter().any(|field| source
                .receipt
                .as_ref()
                .unwrap()
                .originals
                .contains(&field.source)
                && field.text.contains("preserves calibration rows")));
        }
        let audit = fs::read_to_string(fixture.root.join(mode).join("provider-audit.jsonl"))?;
        assert!(
            !audit.contains(&server.endpoint),
            "host embedding endpoint entered model input"
        );
        assert!(
            !audit.contains("native-authored-embedding"),
            "host encoding identity entered model input"
        );
    }
    Ok(())
}

struct RenameStoreAfterRecall {
    inner: EvidenceChoosingProvider,
    database: PathBuf,
}
#[async_trait::async_trait]
impl Provider for RenameStoreAfterRecall {
    fn id(&self) -> &str {
        "authored-rename-after-recall"
    }
    fn default_model(&self) -> &str {
        "authored"
    }
    fn local(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        request: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let reply = self.inner.complete(request, sink).await?;
        if reply
            .message
            .content
            .iter()
            .any(|block| matches!(block, Block::ToolUse {name,..} if name == "fs_write"))
        {
            fs::rename(&self.database, self.database.with_extension("removed"))?;
        }
        Ok(reply)
    }
}

#[tokio::test]
async fn native_controlled_capture_open_failure_preserves_delivered_audit_and_legacy_error_wire(
) -> Result<()> {
    let mut fixture = EncryptedFixture::new()?;
    fixture
        .request
        .files
        .insert("observed.txt".into(), ROUTES.into());
    let source = fixture
        .run(
            "source",
            vec![
                call("observe", "fs_read", json!({"path":"observed.txt"})),
                call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":"[]"}),
                ),
                Message::assistant(vec![Block::text("Done.")]),
            ],
        )
        .await?;
    assert!(source.receipt.is_some());
    fixture.request.files.clear();
    for controlled in [false, true] {
        let name = if controlled {
            "controlled-missing-store"
        } else {
            "legacy-missing-store"
        };
        let database = fixture.root.join(format!("{name}.memory-prototype.sqlite"));
        fs::copy(&fixture.database, &database)?;
        let task_root = fixture.root.join(name);
        let factory = |_| async {
            Ok(Box::new(RenameStoreAfterRecall {
                inner: EvidenceChoosingProvider {
                    step: Mutex::new(0),
                    query: "calibration".into(),
                    withdraw: None,
                },
                database: database.clone(),
            }) as Box<dyn Provider>)
        };
        let result = if controlled {
            run_native_session_with_recall(
                &database,
                fixture.storage()?,
                &fixture.authority,
                &fixture.request,
                NativeRecallConfig::lexical("calibration".into())?,
                &task_root,
                factory,
                &CancellationToken::new(),
            )
            .await
        } else {
            run_native_session_with_storage(
                &database,
                fixture.storage()?,
                &fixture.authority,
                &fixture.request,
                &task_root,
                factory,
                &CancellationToken::new(),
            )
            .await
        };
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(task_root.join("report.json"))?)?;
        if controlled {
            let report =
                result.expect("capture-open failure must preserve the known controlled result");
            assert_eq!(report.check.verdict, CheckVerdict::Passed);
            assert!(report.run_error.is_none());
            assert!(report.capture_error.is_some() && report.receipt.is_none());
            assert!(report.transcript.is_file());
            let recall = report.recall.unwrap();
            assert!(recall.protocol_satisfied());
            assert!(!recall.delivered.is_empty());
            assert_eq!(saved["recall"]["status"], "completed");
            assert!(saved["capture_error"].is_string());
        } else {
            assert!(result.is_err());
            assert_eq!(saved["status"], "failed");
            assert!(saved.get("recall").is_none() && saved.get("check").is_none());
        }
    }
    Ok(())
}

#[tokio::test]
async fn native_controlled_cancel_during_embedding_preserves_unsuccessful_attempt_without_late_packet(
) -> Result<()> {
    let mut fixture = EncryptedFixture::new()?;
    fixture
        .request
        .files
        .insert("observed.txt".into(), ROUTES.into());
    let source = fixture
        .run(
            "source",
            vec![
                call("observe", "fs_read", json!({"path":"observed.txt"})),
                call(
                    "write",
                    "fs_write",
                    json!({"path":"output.json","content":"[]"}),
                ),
                Message::assistant(vec![Block::text("Done.")]),
            ],
        )
        .await?;
    assert!(source.receipt.is_some());
    fixture.request.files.clear();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let sent = Arc::new(tokio::sync::Notify::new());
    let server =
        native_embedding_server_with_hold(Some((entered.clone(), release.clone(), sent.clone())))
            .await;
    let recall = serde_json::from_value::<NativeRecallRequest>(json!({"mode":"dense","query":"calibration","endpoint":server.endpoint,"expected_alias":"native-authored-embedding","artifact_revision":"fixture-v1","dimensions":2,"pooling":"last","normalization":"l2"}))?.into_config()?;
    let cancel = CancellationToken::new();
    let task_root = fixture.root.join("cancelled-embedding");
    let run = run_native_session_with_recall(
        &fixture.database,
        fixture.storage()?,
        &fixture.authority,
        &fixture.request,
        recall,
        &task_root,
        |_| async {
            Ok(Box::new(EvidenceChoosingProvider {
                step: Mutex::new(0),
                query: "calibration".into(),
                withdraw: None,
            }) as Box<dyn Provider>)
        },
        &cancel,
    );
    let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(run, async {
            entered.notified().await;
            cancel.cancel();
            release.notify_one();
            sent.notified().await;
        })
    })
    .await
    .context("native cancellation did not drain")?;
    let report = result?;
    let recall = report
        .recall
        .as_ref()
        .context("cancelled recall audit absent")?;
    assert_eq!(recall.entered_calls, 1);
    assert_eq!(recall.admitted_calls, 1);
    assert!(matches!(
        recall.status,
        crate::tool::goal_memory::NativeRecallStatus::Cancelled
            | crate::tool::goal_memory::NativeRecallStatus::Interrupted
    ));
    assert!(!recall.protocol_satisfied() && recall.delivered.is_empty());
    assert_eq!(report.run_error.as_deref(), Some("task cancelled"));
    let transcript = Session::read(&report.transcript)?;
    assert!(!transcript.convo.messages.iter().flat_map(|m| &m.content).any(|b| matches!(b,Block::ToolResult {tool_use_id,is_error:false,..} if tool_use_id == "recall")));
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(task_root.join("report.json"))?)?;
    assert_eq!(saved["recall"]["admitted_calls"], 1);
    let audit = fs::read_to_string(task_root.join("provider-audit.jsonl"))?;
    let starts = audit
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["event"] == "request_started")
        .count();
    assert_eq!(
        starts, 1,
        "cancellation must not send a follow-up model request"
    );
    Ok(())
}
