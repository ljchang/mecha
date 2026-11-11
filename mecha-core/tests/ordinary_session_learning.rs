#![cfg(feature = "memory-prototype")]
use chrono::Utc;
use mecha_core::{
    agent::{StopCause, Taint},
    memory_session::{SessionMemory, SessionMemoryRequest, SessionMemoryStatus},
    message::{Block, Message},
    session::{Record, RunConfig, RunStats, Session, SessionKind, SessionMeta},
};
use std::{collections::BTreeSet, fs, path::PathBuf};
struct Fixture {
    root: PathBuf,
    request: SessionMemoryRequest,
    workspace: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("ordinary-lifecycle-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        for p in ["db", "keys", "work", "sessions"] {
            mecha_core::create_private_dir(&root.join(p)).unwrap();
        }
        let request = SessionMemoryRequest {
            protocol: "ordinary-session-memory-v1".into(),
            db: root.join("db/test.memory-prototype.sqlite"),
            key_file: root.join("keys/key"),
            authority: root.join("authority"),
            domain: "fixture".into(),
            account: "fixture".into(),
            scopes: BTreeSet::new(),
        };
        let workspace = root.join("work");
        Self {
            root,
            request,
            workspace,
        }
    }
    fn session(&self) -> Session {
        let meta = SessionMeta {
            id: Session::new_id(),
            created_at: Utc::now(),
            provider: "fixture".into(),
            model: "fixture".into(),
            workspace: self.workspace.clone(),
            title: None,
            kind: Some(SessionKind::Run),
        };
        let session = Session {
            path: self
                .root
                .join("sessions")
                .join(format!("{}.jsonl", meta.id)),
            meta: meta.clone(),
        };
        session.append(&Record::Meta(meta)).unwrap();
        session
            .append(&Record::Config(RunConfig {
                workspace: self.workspace.clone(),
                tools: vec!["fs_read".into(), "fs_write".into(), "goal_context".into()],
                ..Default::default()
            }))
            .unwrap();
        session
            .append(&Record::GoalAnchor {
                goal: Some("charter:1".parse().unwrap()),
            })
            .unwrap();
        session
    }
    fn tail_with(&self, s: &Session, content: Vec<Block>) {
        self.tail_with_outcome(
            s,
            content,
            RunStats {
                stop_cause: Some(StopCause::Completed),
                ..Default::default()
            },
        );
    }
    fn tail_with_outcome(&self, s: &Session, content: Vec<Block>, outcome: RunStats) {
        s.append(&Record::Message(Message::assistant(content)))
            .unwrap();
        s.append(&Record::Grants { grants: vec![] }).unwrap();
        s.append(&Record::GoalAnchor {
            goal: Some("charter:1".parse().unwrap()),
        })
        .unwrap();
        let recalled = fs::read_to_string(&s.path)
            .unwrap()
            .contains("\"name\":\"goal_context\"");
        s.append(&Record::Taint(Taint {
            private: recalled,
            untrusted: recalled,
        }))
        .unwrap();
        s.append(&Record::Outcome(outcome)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
use mecha_core::memory::model_evaluation::ModelReply;
use mecha_core::memory_session::learning::{
    LearningInput, LearningOperation, Opposition, OriginalSelection,
};

fn observe(f: &Fixture, memory: &SessionMemory, text: &str) -> (OriginalSelection, Session) {
    let session = f.session();
    let run = memory
        .begin(&session, "Write fixture", &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user("Write fixture")))
        .unwrap();
    f.tail_with(&session, vec![Block::text(text)]);
    let report = memory.finish(&run);
    assert_eq!(report.status, SessionMemoryStatus::Retained);
    (
        OriginalSelection {
            run_id: run.run_id().into(),
            original_index: 1,
        },
        session,
    )
}
fn induction(sources: Vec<OriginalSelection>) -> LearningInput {
    LearningInput {
        protocol: "ordinary-session-learning-v1".into(),
        operation: LearningOperation::Induce { sources },
    }
}
fn reply(text: serde_json::Value) -> ModelReply {
    ModelReply {
        text: text.to_string(),
        model: "authored".into(),
        input_tokens: None,
        output_tokens: None,
        error: None,
    }
}
fn draft(
    descriptors: &[mecha_core::memory_session::learning::OriginalDescriptor],
) -> serde_json::Value {
    serde_json::json!({"name":"bounded fixture policy", "principle":"Fixture queues can require approval.", "applicability":"Observed fixture queues only.", "support":descriptors.iter().map(|s| serde_json::json!({"source_id":s.reference.revision,"field":s.field,"quote":s.text})).collect::<Vec<_>>(), "counterexamples":[]})
}

#[tokio::test]
async fn ordinary_learning_induces_reopens_and_preserves_exact_opposition() {
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, _) = observe(&f, &memory, "Fixture queue alpha requires approval.");
    let (b, _) = observe(&f, &memory, "Fixture queue beta requires approval.");
    let sources = vec![a, b];
    let descriptors = memory.learning_sources(&sources).unwrap();
    assert!(descriptors.iter().all(|s| s.field == "/content/0/text"));
    let proposed = draft(&descriptors);
    let report = memory
        .learn(induction(sources), |generation, guard| async move {
            guard.check().unwrap();
            assert!(generation.prompt.contains("AssistantText"));
            Ok(reply(proposed))
        })
        .await;
    assert_eq!(report.status, "retained", "{report:?}");
    let original = report.retained.unwrap();
    drop(memory);
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (c, _) = observe(
        &f,
        &memory,
        "Fixture queue gamma bypasses approval for emergencies.",
    );
    let opposite = memory
        .learning_sources(std::slice::from_ref(&c))
        .unwrap()
        .remove(0);
    let mut revised = draft(&descriptors);
    revised["counterexamples"] = serde_json::json!([{"source_id":opposite.reference.revision,"field":opposite.field,"quote":opposite.text}]);
    revised["applicability"] = serde_json::json!("Except emergency fixture queues.");
    let report = memory
        .learn(
            LearningInput {
                protocol: "ordinary-session-learning-v1".into(),
                operation: LearningOperation::Ruminate {
                    expected: original.clone(),
                    counterexamples: vec![Opposition {
                        source: c,
                        quote: opposite.text,
                    }],
                },
            },
            |_, guard| async move {
                guard.check().unwrap();
                Ok(reply(
                    serde_json::json!({"outcome":"revised_provisional","draft":revised}),
                ))
            },
        )
        .await;
    assert_eq!(report.status, "revised_provisional", "{report:?}");
    let retained = report.retained.unwrap();
    assert_eq!(retained.object, original.object);
    assert_ne!(retained.revision, original.revision);
}

#[tokio::test]
async fn ordinary_learning_denies_recovery_foreign_and_invalid_originals_before_callback() {
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, _) = observe(&f, &memory, "Fixture observation one.");
    let (b, _) = observe(&f, &memory, "Fixture observation two.");
    let recovery = SessionMemory::open_for_recovery(&f.request).unwrap();
    assert!(recovery.learning_sources(std::slice::from_ref(&a)).is_err());
    let foreign = Fixture::new();
    SessionMemory::initialize(&foreign.request).unwrap();
    let foreign_memory = SessionMemory::open(&foreign.request, &foreign.workspace).unwrap();
    let mut invalid = a.clone();
    invalid.original_index = usize::MAX;
    for (handle, sources) in [
        (&recovery, vec![a.clone(), b.clone()]),
        (&foreign_memory, vec![a.clone(), b.clone()]),
        (&memory, vec![invalid, b]),
        (&memory, vec![a.clone(), a]),
    ] {
        let report = handle
            .learn(induction(sources), |_, _| async {
                panic!("denied source reached inference")
            })
            .await;
        assert_eq!(report.status, "failed");
        assert_eq!(report.failure_stage.as_deref(), Some("prepare"));
        assert!(!report.callback_invoked);
    }
}

#[tokio::test]
async fn ordinary_learning_rechecks_withdrawal_after_success_error_and_malformed_reply() {
    for mode in ["success", "error", "malformed"] {
        let f = Fixture::new();
        SessionMemory::initialize(&f.request).unwrap();
        let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
        let (a, session) = observe(&f, &memory, "Fixture observation one.");
        let (b, _) = observe(&f, &memory, "Fixture observation two.");
        let sources = vec![a, b];
        let proposed = draft(&memory.learning_sources(&sources).unwrap());
        let report = memory
            .learn(induction(sources), |_, guard| async move {
                guard.check().unwrap();
                fs::remove_file(session.path).unwrap();
                match mode {
                    "error" => anyhow::bail!("authored transport failure"),
                    "malformed" => Ok(reply(serde_json::json!({"invalid":true}))),
                    _ => Ok(reply(proposed)),
                }
            })
            .await;
        assert_eq!(report.status, "failed");
        assert_eq!(report.failure_stage.as_deref(), Some("recheck"));
        assert!(report.callback_invoked);
        assert!(report.retained.is_none());
        assert_eq!(report.reply.is_some(), mode != "error");
        if mode == "error" {
            assert!(report
                .inference_error
                .unwrap()
                .contains("authored transport failure"));
        }
    }
}

#[tokio::test]
async fn ordinary_learning_failed_rumination_rechecks_old_uncited_sources() {
    for outcome in ["unchanged", "abstained", "error"] {
        let f = Fixture::new();
        SessionMemory::initialize(&f.request).unwrap();
        let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
        let (a, old) = observe(&f, &memory, "Fixture observation alpha.");
        let (b, _) = observe(&f, &memory, "Fixture observation beta.");
        let sources = vec![a, b];
        let proposed = draft(&memory.learning_sources(&sources).unwrap());
        let previous = memory
            .learn(
                induction(sources),
                |_, _| async move { Ok(reply(proposed)) },
            )
            .await
            .retained
            .unwrap();
        let (c, _) = observe(&f, &memory, "Fixture opposition gamma.");
        let input = LearningInput {
            protocol: "ordinary-session-learning-v1".into(),
            operation: LearningOperation::Ruminate {
                expected: previous,
                counterexamples: vec![Opposition {
                    source: c,
                    quote: "Fixture opposition gamma.".into(),
                }],
            },
        };
        let report = memory
            .learn(input, |_, guard| async move {
                guard.check().unwrap();
                fs::remove_file(old.path).unwrap();
                if outcome == "error" {
                    anyhow::bail!("failed revision transport")
                }
                Ok(reply(
                    serde_json::json!({"outcome":outcome,"reason":"authored no change"}),
                ))
            })
            .await;
        assert_eq!(
            report.failure_stage.as_deref(),
            Some("recheck"),
            "{report:?}"
        );
        assert!(report.retained.is_none());
        assert!(report.rumination.is_none());
    }
}

#[tokio::test]
async fn ordinary_learning_no_change_and_abstention_preserve_reference_without_new_revision() {
    for outcome in ["unchanged", "abstained"] {
        let f = Fixture::new();
        SessionMemory::initialize(&f.request).unwrap();
        let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
        let (a, _) = observe(&f, &memory, "Fixture observation alpha.");
        let (b, _) = observe(&f, &memory, "Fixture observation beta.");
        let sources = vec![a, b];
        let proposed = draft(&memory.learning_sources(&sources).unwrap());
        let previous = memory
            .learn(
                induction(sources),
                |_, _| async move { Ok(reply(proposed)) },
            )
            .await
            .retained
            .unwrap();
        let (c, _) = observe(&f, &memory, "Fixture opposition gamma.");
        let input = LearningInput {
            protocol: "ordinary-session-learning-v1".into(),
            operation: LearningOperation::Ruminate {
                expected: previous.clone(),
                counterexamples: vec![Opposition {
                    source: c,
                    quote: "Fixture opposition gamma.".into(),
                }],
            },
        };
        let report = memory
            .learn(input, |_, _| async move {
                Ok(reply(
                    serde_json::json!({"outcome":outcome,"reason":"authored no change"}),
                ))
            })
            .await;
        assert_eq!(report.status, outcome, "{report:?}");
        assert!(
            report.retained.is_none(),
            "no-write outcomes are not retention"
        );
        let wire = serde_json::to_value(report.rumination.unwrap()).unwrap();
        assert_eq!(wire["reference"], serde_json::to_value(previous).unwrap());
    }
}

#[tokio::test]
async fn ordinary_learning_describes_exact_fs_read_result_and_receipt_status() {
    use mecha_core::tool::{builtin::FsRead, Tool, ToolCtx};
    for succeeds in [true, false] {
        let f = Fixture::new();
        SessionMemory::initialize(&f.request).unwrap();
        let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
        let session = f.session();
        let run = memory
            .begin(&session, "Write fixture", &"charter:1".parse().unwrap())
            .unwrap();
        session
            .append(&Record::Message(Message::user("Write fixture")))
            .unwrap();
        if succeeds {
            fs::write(
                f.workspace.join("observation.json"),
                "{\"fixture_readout\":7}",
            )
            .unwrap();
        }
        let input = serde_json::json!({"path":"observation.json"});
        let output = FsRead
            .call(
                input.clone(),
                &ToolCtx {
                    workspace: f.workspace.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(!output.is_error, succeeds);
        session
            .append(&Record::Message(Message::assistant(vec![Block::ToolUse {
                id: "read-observation".into(),
                name: "fs_read".into(),
                input,
            }])))
            .unwrap();
        let mut result = Message::user("");
        result.content = vec![Block::ToolResult {
            tool_use_id: "read-observation".into(),
            content: output.content.clone(),
            is_error: output.is_error,
        }];
        session.append(&Record::Message(result)).unwrap();
        f.tail_with(
            &session,
            vec![Block::text("The fixture read was attempted.")],
        );
        assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
        let descriptors = memory.learning_run_sources(run.run_id()).unwrap();
        let result = descriptors
            .iter()
            .find(|s| s.kind == mecha_core::memory::host_capture::PassageKind::ToolResult)
            .unwrap();
        assert_eq!(result.source.original_index, 2);
        assert_eq!(result.field, "/content/0/content");
        assert_eq!(result.text, output.content);
        let action = result.tool.as_ref().unwrap();
        assert_eq!(action.call_id, "read-observation");
        assert_eq!(action.tool_name, "fs_read");
        assert_eq!(action.input["path"], "observation.json");
        assert_eq!(
            result.receipt_status,
            Some(if succeeds {
                mecha_core::memory::host_capture::ReceiptStatus::Succeeded
            } else {
                mecha_core::memory::host_capture::ReceiptStatus::Failed
            })
        );
    }
}

#[tokio::test]
async fn ordinary_learning_cannot_reuse_a_concept_influenced_original_as_independent_support() {
    use mecha_core::tool::ToolCtx;
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, _) = observe(&f, &memory, "Fixture observation alpha.");
    let (b, _) = observe(&f, &memory, "Fixture observation beta.");
    let sources = vec![a.clone(), b];
    let proposed = draft(&memory.learning_sources(&sources).unwrap());
    let previous = memory
        .learn(
            induction(sources),
            |_, _| async move { Ok(reply(proposed)) },
        )
        .await
        .retained
        .unwrap();
    let session = f.session();
    let run = memory
        .begin(&session, "Write fixture", &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user("Write fixture")))
        .unwrap();
    let input = serde_json::json!({"memory":{"query":"fixture"}});
    let result = memory
        .goal_memory(&run)
        .unwrap()
        .call(
            input.clone(),
            &ToolCtx {
                workspace: f.workspace.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(result.external && !result.is_error, "{}", result.content);
    assert!(
        result.content.contains(&previous.revision),
        "concept must actually be delivered"
    );
    session
        .append(&Record::Message(Message::assistant(vec![Block::ToolUse {
            id: "recall".into(),
            name: "goal_context".into(),
            input,
        }])))
        .unwrap();
    let mut message = Message::user("");
    message.content = vec![Block::ToolResult {
        tool_use_id: "recall".into(),
        content: result.content,
        is_error: result.is_error,
    }];
    session.append(&Record::Message(message)).unwrap();
    f.tail_with(
        &session,
        vec![Block::text("Fixture conclusion after hypothesis recall.")],
    );
    assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
    let inventory = memory.learning_run_sources(run.run_id()).unwrap();
    let matching: Vec<_> = inventory
        .iter()
        .filter(|descriptor| {
            descriptor.kind == mecha_core::memory::host_capture::PassageKind::AssistantText
                && descriptor.text == "Fixture conclusion after hypothesis recall."
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected one authenticated final statement"
    );
    let derived = matching[0].source.clone();
    let report = memory
        .learn(induction(vec![a, derived]), |_, _| async {
            panic!("derived original reached inference")
        })
        .await;
    assert_eq!(
        report.failure_stage.as_deref(),
        Some("prepare"),
        "{report:?}"
    );
    assert!(!report.callback_invoked);
    assert!(report
        .error
        .unwrap()
        .contains("concept or interpretation ancestors"));
}

#[tokio::test]
async fn ordinary_learning_dispatch_guard_observes_withdrawal_after_preparation() {
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, session) = observe(&f, &memory, "Fixture observation alpha.");
    let (b, _) = observe(&f, &memory, "Fixture observation beta.");
    let report = memory
        .learn(induction(vec![a, b]), |_, guard| async move {
            fs::remove_file(session.path).unwrap();
            assert!(
                guard.check().is_err(),
                "dispatch guard refreshed away a withdrawn source"
            );
            anyhow::bail!("dispatch refused")
        })
        .await;
    assert_eq!(report.failure_stage.as_deref(), Some("recheck"));
    assert!(report.reply.is_none());
    assert_eq!(report.inference_error.as_deref(), Some("dispatch refused"));
}

#[tokio::test]
async fn ordinary_learning_opposition_uses_exact_unambiguous_utf8_bytes() {
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, _) = observe(&f, &memory, "Fixture observation alpha.");
    let (b, _) = observe(&f, &memory, "Fixture observation beta.");
    let sources = vec![a, b];
    let proposed = draft(&memory.learning_sources(&sources).unwrap());
    let previous = memory
        .learn(
            induction(sources),
            |_, _| async move { Ok(reply(proposed)) },
        )
        .await
        .retained
        .unwrap();
    let (c, _) = observe(&f, &memory, "émergency fixture, repeat repeat, aaa.");
    for (quote, admitted) in [
        ("émergency", true),
        ("repeat", false),
        ("aa", false),
        ("invented", false),
        ("", false),
    ] {
        let input = LearningInput {
            protocol: "ordinary-session-learning-v1".into(),
            operation: LearningOperation::Ruminate {
                expected: previous.clone(),
                counterexamples: vec![Opposition {
                    source: c.clone(),
                    quote: quote.into(),
                }],
            },
        };
        let report = memory
            .learn(input, |_, guard| async move {
                assert!(admitted, "ambiguous or absent quote reached inference");
                guard.check().unwrap();
                Ok(reply(
                    serde_json::json!({"outcome":"unchanged","reason":"authored no change"}),
                ))
            })
            .await;
        assert_eq!(report.callback_invoked, admitted, "{quote}: {report:?}");
        assert_eq!(report.status, if admitted { "unchanged" } else { "failed" });
    }
}

#[tokio::test]
async fn ordinary_learning_inventory_allows_evidence_only_recalled_closure() {
    use mecha_core::tool::ToolCtx;
    let f = Fixture::new();
    SessionMemory::initialize(&f.request).unwrap();
    let memory = SessionMemory::open(&f.request, &f.workspace).unwrap();
    let (a, _) = observe(&f, &memory, "Fixture observation alpha.");
    let (b, _) = observe(&f, &memory, "Fixture observation beta.");
    let session = f.session();
    let run = memory
        .begin(&session, "Write fixture", &"charter:1".parse().unwrap())
        .unwrap();
    session
        .append(&Record::Message(Message::user("Write fixture")))
        .unwrap();
    let input = serde_json::json!({"memory":{"query":"alpha"}});
    let output = memory
        .goal_memory(&run)
        .unwrap()
        .call(
            input.clone(),
            &ToolCtx {
                workspace: f.workspace.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Both observations were recorded as local, so the recall is too.
    assert!(!output.external && !output.is_error);
    let original = memory
        .learning_sources(std::slice::from_ref(&a))
        .unwrap()
        .remove(0);
    assert!(
        output.content.contains(&original.reference.revision),
        "prior evidence must actually be recalled: {}",
        output.content
    );
    session
        .append(&Record::Message(Message::assistant(vec![Block::ToolUse {
            id: "evidence-recall".into(),
            name: "goal_context".into(),
            input,
        }])))
        .unwrap();
    let mut message = Message::user("");
    message.content = vec![Block::ToolResult {
        tool_use_id: "evidence-recall".into(),
        content: output.content,
        is_error: output.is_error,
    }];
    session.append(&Record::Message(message)).unwrap();
    f.tail_with(
        &session,
        vec![Block::text(
            "Fixture claim informed by evidence-only recall.",
        )],
    );
    assert_eq!(memory.finish(&run).status, SessionMemoryStatus::Retained);
    let inventory = memory.learning_run_sources(run.run_id()).unwrap();
    let matching: Vec<_> = inventory
        .iter()
        .filter(|descriptor| {
            descriptor.kind == mecha_core::memory::host_capture::PassageKind::AssistantText
                && descriptor.text == "Fixture claim informed by evidence-only recall."
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected one authenticated final statement"
    );
    let sources = vec![b, matching[0].source.clone()];
    let descriptors = memory.learning_sources(&sources).unwrap();
    assert_eq!(
        descriptors[1].text,
        "Fixture claim informed by evidence-only recall."
    );
    let proposed = draft(&descriptors);
    let report = memory
        .learn(induction(sources), |_, guard| async move {
            guard.check().unwrap();
            Ok(reply(proposed))
        })
        .await;
    assert_eq!(report.status, "retained", "{report:?}");
    assert!(report.retained.is_some());
}
