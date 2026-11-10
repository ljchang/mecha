// Synthetic fixtures only. The first terminal answer omits the requested write;
// completion must continue the original run and preserve actual capture.
#[tokio::test]
async fn native_completion_repairs_missing_write_and_captures_original_coordinates() -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    let messages = vec![
        Message::assistant(vec![Block::text("Done.")]),
        call(
            "repair",
            "fs_write",
            json!({"path":"output.json","content":r#"[{"route":"Quill"}]"#}),
        ),
        Message::assistant(vec![Block::text("Saved.")]),
    ];
    let report = run_native_session_with_completion(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        None,
        NativeCompletionRequest {
            rows: 1,
            keys: vec!["route".into()],
        }
        .into_config()?,
        &root.join("repair"),
        |_| async { Ok(Box::new(Script::new(messages)) as Box<dyn Provider>) },
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(
        report.check.verdict,
        CheckVerdict::Passed,
        "missing output must receive one repair chance"
    );
    assert!(report.receipt.is_some(), "{report:?}");
    let transcript = Session::read(&report.transcript)?;
    assert_eq!(
        transcript.outcome_positions,
        [Some(transcript.convo.messages.len())]
    );
    assert_eq!(
        transcript
            .convo
            .messages
            .iter()
            .filter(|m| m.harness)
            .count(),
        1
    );
    let memory = Memory::open(&database)?.with_source_guard(Arc::new(authority.clone()));
    let raw = fs::read_to_string(&report.transcript)?;
    let repair_line = raw.lines().enumerate().find_map(|(index, line)| {
        let record: Record = serde_json::from_str(line).ok()?;
        matches!(record, Record::Message(message) if message.content.iter().any(|b| matches!(b, Block::ToolUse {id,..} if id == "repair"))).then_some(index + 1)
    }).context("repair missing from raw transcript")?;
    let projection = memory
        .read(
            &report.receipt.as_ref().unwrap().projection,
            &request.access(),
        )?
        .unwrap();
    let crate::memory::Record::Evidence(projection) = projection else {
        panic!("capture projection must be evidence")
    };
    let projection: serde_json::Value = serde_json::from_str(&projection.text)?;
    assert!(projection["originals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field["kind"] == "ActionInvocation" && field["line"] == repair_line));
    let prepared = memory.prepare_interpretation(
        report.receipt.as_ref().unwrap(),
        &[],
        &InterpretationContext::new(request.criterion.clone(), "completion-capture".into())?,
        &memory.snapshot()?,
        &request.access(),
    )?;
    for message in transcript.convo.messages.iter().filter(|m| m.harness) {
        for block in &message.content {
            if let Block::Text { text } = block {
                assert!(
                    !prepared.sources().iter().any(|s| s.text.contains(text)),
                    "harness feedback must not become owner evidence"
                );
            }
        }
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_completion_accepts_wrong_and_unknown_values_independently_of_gold() -> Result<()> {
    let (root, database, authority, mut request) = native_fixture();
    for (index, value) in ["incorrect", "unknown"].iter().enumerate() {
        // Hidden gold has an unrelated shape; completion must never derive from it.
        request.expected_rows =
            vec![BTreeMap::from([("hidden_gold".into(), "not-visible".into())]); 3];
        let messages = vec![
            call(
                "write",
                "fs_write",
                json!({"path":"output.json","content":json!([{"route":value}]).to_string()}),
            ),
            Message::assistant(vec![Block::text("Saved.")]),
        ];
        let report = run_native_session_with_completion(
            &database,
            StorageAccess::plaintext_fixture(),
            &authority,
            &request,
            None,
            NativeCompletionRequest {
                rows: 1,
                keys: vec!["route".into()],
            }
            .into_config()?,
            &root.join(format!("wrong-{index}")),
            |_| async { Ok(Box::new(Script::new(messages)) as Box<dyn Provider>) },
            &CancellationToken::new(),
        )
        .await?;
        assert_eq!(report.check.verdict, CheckVerdict::Failed);
        assert!(report.completion.as_ref().unwrap().protocol_satisfied());
        assert!(
            !report
                .completion
                .as_ref()
                .unwrap()
                .report
                .feedback_delivered
        );
        assert!(report.receipt.is_some());
        let audit = fs::read_to_string(root.join(format!("wrong-{index}/provider-audit.jsonl")))?;
        assert!(!audit.contains("hidden_gold") && !audit.contains("not-visible"));
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_completion_identity_precedes_discovery_and_replay_is_refused() -> Result<()> {
    let (root, database, authority, mut request) = native_fixture();
    let task = root.join("identity");
    let result = run_native_session_with_completion(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        None,
        NativeCompletionRequest {
            rows: 1,
            keys: vec!["route".into()],
        }
        .into_config()?,
        &task,
        |_| async {
            let checkpoint: serde_json::Value =
                serde_json::from_slice(&fs::read(task.join("started.json"))?)?;
            assert_eq!(checkpoint["protocol"], "native-memory-session-v2");
            assert_eq!(
                checkpoint["completion"]["protocol"],
                "native-completion-feedback-v1"
            );
            assert_eq!(
                checkpoint["completion"]["shape_digest"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
            anyhow::bail!("authored discovery failure")
        },
        &CancellationToken::new(),
    )
    .await;
    assert!(result.is_err());
    let saved = fs::read(task.join("report.json"))?;
    let failure: serde_json::Value = serde_json::from_slice(&saved)?;
    assert_eq!(failure["protocol"], "native-memory-session-v2");
    assert_eq!(
        failure["completion"]["protocol"],
        "native-completion-feedback-v1"
    );
    assert!(run_native_session_with_completion(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        None,
        NativeCompletionRequest {
            rows: 1,
            keys: vec!["route".into()]
        }
        .into_config()?,
        &task,
        |_| async { panic!("never replay") },
        &CancellationToken::new(),
    )
    .await
    .is_err());
    assert_eq!(saved, fs::read(task.join("report.json"))?);
    request.interpretation_context_id = Some("incompatible".into());
    let invalid = root.join("invalid");
    assert!(run_native_session_with_completion(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        None,
        NativeCompletionRequest {
            rows: 1,
            keys: vec!["route".into()]
        }
        .into_config()?,
        &invalid,
        |_| async { panic!("never discover incompatible context") },
        &CancellationToken::new(),
    )
    .await
    .is_err());
    assert!(!invalid.exists());
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_completion_requires_uncalled_recall_and_never_repeats_consumed_failure(
) -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    for consumed in [false, true] {
        let mut messages = vec![];
        if consumed {
            messages.push(call(
                "bad-read",
                "goal_context",
                json!({"memory":{"query":"wrong query"}}),
            ));
        }
        messages.push(Message::assistant(vec![Block::text("Done.")]));
        if !consumed {
            messages.push(call(
                "required-read",
                "goal_context",
                json!({"memory":{"query":"calibration"}}),
            ));
        }
        messages.push(call(
            "write",
            "fs_write",
            json!({"path":"output.json","content":r#"[{"route":"unknown"}]"#}),
        ));
        messages.push(Message::assistant(vec![Block::text("Saved.")]));
        let report = run_native_session_with_completion(
            &database,
            StorageAccess::plaintext_fixture(),
            &authority,
            &request,
            Some(NativeRecallConfig::lexical("calibration".into())?),
            NativeCompletionRequest {
                rows: 1,
                keys: vec!["route".into()],
            }
            .into_config()?,
            &root.join(if consumed { "consumed" } else { "uncalled" }),
            |_| async { Ok(Box::new(Script::new(messages)) as Box<dyn Provider>) },
            &CancellationToken::new(),
        )
        .await?;
        let completion = report.completion.as_ref().unwrap();
        let feedback = completion
            .report
            .first
            .as_ref()
            .unwrap()
            .feedback
            .as_ref()
            .unwrap();
        assert_eq!(feedback.required_read_missing, !consumed);
        assert_eq!(feedback.do_not_repeat_read, consumed);
        assert!(completion.report.feedback_delivered);
        assert_eq!(completion.protocol_satisfied(), !consumed);
        let recall = report.recall.as_ref().unwrap();
        assert_eq!(recall.entered_calls, 1);
        assert_eq!(recall.protocol_satisfied(), !consumed);
    }
    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn native_completion_later_provider_failure_keeps_accepted_report() -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    // Script returns an error when exhausted, after the feedback was appended.
    let report = run_native_session_with_completion(
        &database,
        StorageAccess::plaintext_fixture(),
        &authority,
        &request,
        None,
        NativeCompletionRequest {
            rows: 1,
            keys: vec!["route".into()],
        }
        .into_config()?,
        &root.join("provider-failure"),
        |_| async {
            Ok(
                Box::new(Script::new(vec![Message::assistant(vec![Block::text(
                    "Done.",
                )])])) as Box<dyn Provider>,
            )
        },
        &CancellationToken::new(),
    )
    .await?;
    let completion = report.completion.as_ref().unwrap();
    assert!(completion.report.first.is_some());
    assert!(completion.report.feedback_delivered);
    assert!(!completion.protocol_satisfied());
    assert!(report.run_error.is_some());
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("provider-failure/report.json"))?)?;
    assert_eq!(saved["completion"]["report"]["feedback_delivered"], true);
    fs::remove_dir_all(root)?;
    Ok(())
}

struct CompletionAfterRecall {
    step: Mutex<usize>,
    withdraw: Option<(LocalAuthority, String)>,
    rename_database: Option<PathBuf>,
}
#[async_trait::async_trait]
impl Provider for CompletionAfterRecall {
    fn id(&self) -> &str {
        "authored-completion-recall"
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
        let step = {
            let mut step = self.step.lock().unwrap();
            let old = *step;
            *step += 1;
            old
        };
        let message = match step {
            0 => call(
                "recall",
                "goal_context",
                json!({"memory":{"query":"calibration"}}),
            ),
            1 => Message::assistant(vec![Block::text("Done.")]),
            2 => {
                assert!(request.messages.iter().any(|m| m.harness));
                if let Some((authority, locator)) = &self.withdraw {
                    authority.withdraw(locator)?;
                }
                if let Some(database) = &self.rename_database {
                    fs::rename(database, database.with_extension("moved"))?;
                }
                call(
                    "repair",
                    "fs_write",
                    json!({"path":"output.json","content":r#"[{"route":"unknown"}]"#}),
                )
            }
            _ => Message::assistant(vec![Block::text("Saved.")]),
        };
        Script::new(vec![message]).complete(request, sink).await
    }
}

#[tokio::test]
async fn native_completion_encrypted_repair_preserves_source_closure_and_capture_failures(
) -> Result<()> {
    for failure in ["none", "withdraw", "store"] {
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
                    Message::assistant(vec![Block::text("Saved.")]),
                ],
            )
            .await?;
        assert!(source.receipt.is_some());
        fixture.request.files.clear();
        let locator = fixture.authority.binding(&source.confirmation)?.locator;
        let report = run_native_session_with_completion(
            &fixture.database,
            fixture.storage()?,
            &fixture.authority,
            &fixture.request,
            Some(NativeRecallConfig::lexical("calibration".into())?),
            NativeCompletionRequest {
                rows: 1,
                keys: vec!["route".into()],
            }
            .into_config()?,
            &fixture.root.join("repair"),
            |_| async {
                Ok(Box::new(CompletionAfterRecall {
                    step: Mutex::new(0),
                    withdraw: (failure == "withdraw").then(|| (fixture.authority.clone(), locator)),
                    rename_database: (failure == "store").then(|| fixture.database.clone()),
                }) as Box<dyn Provider>)
            },
            &CancellationToken::new(),
        )
        .await?;
        assert!(report.completion.as_ref().unwrap().protocol_satisfied());
        assert!(
            report
                .completion
                .as_ref()
                .unwrap()
                .report
                .feedback_delivered
        );
        assert!(report.recall.as_ref().unwrap().protocol_satisfied());
        assert!(report.taint.private && report.taint.untrusted);
        assert_eq!(report.capture_error.is_some(), failure != "none");
        assert_eq!(report.receipt.is_some(), failure == "none");
        if failure == "none" {
            let memory = Memory::open_with_storage(&fixture.database, fixture.storage()?)?
                .with_source_guard(Arc::new(fixture.authority.clone()));
            let prepared = memory.prepare_interpretation(
                report.receipt.as_ref().unwrap(),
                &[],
                &InterpretationContext::new(fixture.request.criterion.clone(), "repaired".into())?,
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
    }
    Ok(())
}

#[tokio::test]
async fn native_completion_repairs_malformed_artifact_once_without_second_intervention(
) -> Result<()> {
    let (root, database, authority, request) = native_fixture();
    for repaired in [true, false] {
        let mut messages = vec![
            call(
                "malformed",
                "fs_write",
                json!({"path":"output.json","content":r#"[{"route":"first","route":"second"}]"#}),
            ),
            Message::assistant(vec![Block::text("Done.")]),
        ];
        if repaired {
            messages.push(call(
                "repair",
                "fs_write",
                json!({"path":"output.json","content":r#"[{"route":"unknown"}]"#}),
            ));
        }
        messages.push(Message::assistant(vec![Block::text("Finished.")]));
        let report = run_native_session_with_completion(
            &database,
            StorageAccess::plaintext_fixture(),
            &authority,
            &request,
            None,
            NativeCompletionRequest {
                rows: 1,
                keys: vec!["route".into()],
            }
            .into_config()?,
            &root.join(if repaired { "fixed" } else { "still-malformed" }),
            |_| async { Ok(Box::new(Script::new(messages)) as Box<dyn Provider>) },
            &CancellationToken::new(),
        )
        .await?;
        let completion = report.completion.as_ref().unwrap();
        assert!(completion.report.feedback_delivered);
        assert_eq!(
            completion
                .report
                .first
                .as_ref()
                .unwrap()
                .feedback
                .as_ref()
                .unwrap()
                .artifact,
            Some(crate::agent::CompletionArtifactIssue::Malformed)
        );
        assert_eq!(completion.protocol_satisfied(), repaired);
        assert!(
            report.run_error.is_none(),
            "a second intervention would exhaust the authored provider"
        );
        let transcript = Session::read(&report.transcript)?;
        assert_eq!(
            transcript
                .convo
                .messages
                .iter()
                .filter(|message| message.harness)
                .count(),
            1
        );
    }
    fs::remove_dir_all(root)?;
    Ok(())
}
