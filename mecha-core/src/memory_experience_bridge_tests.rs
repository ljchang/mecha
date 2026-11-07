use super::*;
use crate::{
    agent::Taint,
    message::{Block, Message},
    session::{Record, RunStats, SessionKind, SessionMeta},
};
use std::path::PathBuf;

fn criterion() -> ConfirmedCriterion {
    ConfirmedCriterion {
        goal: "task:convert".into(),
        goal_text: "Convert fixture".into(),
        criterion_text: "Rows match fixture".into(),
        criterion_revision: "a".repeat(64),
        confirmation_digest: "b".repeat(64),
    }
}
fn transcript() -> String {
    let records = [
        Record::Meta(SessionMeta {
            id: "synthetic-session".into(),
            created_at: chrono::Utc::now(),
            provider: "authored".into(),
            model: "fixture".into(),
            workspace: PathBuf::from("/tmp"),
            title: None,
            kind: Some(SessionKind::Test),
        }),
        Record::GoalAnchor {
            goal: Some("task:convert".parse().unwrap()),
        },
        Record::Message(Message::user("Convert the fixture")),
        Record::Message(Message::assistant(vec![Block::ToolUse {
            id: "write-1".into(),
            name: "fs_write".into(),
            input: serde_json::json!({"path":"out.json","content":"[]"}),
        }])),
        Record::Message(Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "write-1".into(),
            content: "wrote out.json".into(),
            is_error: false,
        }])),
        Record::GoalAnchor {
            goal: Some("task:convert".parse().unwrap()),
        },
        Record::Taint(Taint {
            private: true,
            untrusted: true,
        }),
        Record::Outcome(RunStats::default()),
    ];
    records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap() + "\n")
        .collect()
}
#[test]
fn native_receipt_and_original_field_positions_come_from_same_bytes() {
    let e = events(&transcript(), &criterion(), "synthetic").unwrap();
    assert_eq!(e.receipts.len(), 1);
    assert_eq!(e.receipts[0].call_id, "write-1");
    assert_eq!(e.originals.len(), 2);
    assert_eq!(e.originals[1].line, 5);
    assert_eq!(e.originals[1].pointer, "/content/0/content");
    assert_eq!(e.originals[1].text, "wrote out.json");
    assert_eq!(e.originals[1].occurred_at, None);
}
#[test]
fn changed_goal_rewrite_and_orphan_result_refuse_without_leaking() {
    let base = transcript();
    for text in [
        base.replace("task:convert", "task:other"),
        base.clone() + "{\"record\":\"rewrite\",\"messages\":[]}\n",
        base.replace("\"tool_use_id\":\"write-1\"", "\"tool_use_id\":\"unknown\""),
        base.replace(
            "\"path\":\"out.json\"",
            "\"path\":\"other\",\"path\":\"out.json\"",
        ),
        base.replace(
            "\"tool_use_id\":\"write-1\"",
            "\"tool_use_id\":\"orphan\",\"tool_use_id\":\"write-1\"",
        ),
    ] {
        assert!(events(&text, &criterion(), "synthetic").is_err());
    }
}

#[test]
fn native_late_result_is_refused_instead_of_copying_the_superseded_passage() {
    let late = Record::LateResult {
        index: 2,
        tool_use_id: "write-1".into(),
        content: "late replacement".into(),
        is_error: true,
        external: true,
    };
    let text = transcript() + &serde_json::to_string(&late).unwrap() + "\n";
    let parsed = crate::session::Session::parse(Path::new("capture.jsonl"), &text).unwrap();
    assert!(matches!(&parsed.convo.messages[2].content[0],
        Block::ToolResult {content,is_error:true,..} if content=="late replacement"));
    assert!(events(&text, &criterion(), "synthetic").is_err());
}

#[test]
fn native_late_failure_is_refused_instead_of_appraising_a_different_outcome() {
    let late = Record::LateFailure {
        run: 0,
        tool_use_id: "write-1".into(),
    };
    let text = transcript() + &serde_json::to_string(&late).unwrap() + "\n";
    let parsed = crate::session::Session::parse(Path::new("capture.jsonl"), &text).unwrap();
    assert_eq!(parsed.outcomes[0].tool_errors, 1);
    assert!(events(&text, &criterion(), "synthetic").is_err());
}

#[test]
fn native_resume_without_an_outcome_cannot_borrow_the_previous_runs_appraisal() {
    let resumed = [
        Record::Message(Message::user("Resume the conversion")),
        Record::Message(Message::assistant(vec![Block::ToolUse {
            id: "write-2".into(),
            name: "fs_write".into(),
            input: serde_json::json!({"path":"out.json","content":"[{}]"}),
        }])),
        Record::Message(Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "write-2".into(),
            content: "wrote changed out.json".into(),
            is_error: false,
        }])),
        // record_run can finish before a crash prevents record_outcome.
        Record::GoalAnchor {
            goal: Some("task:convert".parse().unwrap()),
        },
    ];
    let text = transcript()
        + &resumed
            .iter()
            .map(|r| serde_json::to_string(r).unwrap() + "\n")
            .collect::<String>();
    let parsed = crate::session::Session::parse(Path::new("capture.jsonl"), &text).unwrap();
    assert_eq!(parsed.outcome_positions, vec![Some(3)]);
    assert_eq!(parsed.convo.messages.len(), 6);
    assert_eq!(
        parsed.anchor_covering(5),
        Some(&"task:convert".parse().unwrap())
    );
    assert!(events(&text, &criterion(), "synthetic").is_err());
}

#[test]
fn native_anchor_after_outcome_requires_another_completed_range() {
    let anchor = Record::GoalAnchor {
        goal: Some("task:convert".parse().unwrap()),
    };
    let text = transcript() + &serde_json::to_string(&anchor).unwrap() + "\n";
    let parsed = crate::session::Session::parse(Path::new("capture.jsonl"), &text).unwrap();
    assert_eq!(parsed.outcome_positions, vec![Some(3)]);
    assert_eq!(parsed.convo.messages.len(), 3);
    assert!(events(&text, &criterion(), "synthetic").is_err());
}
#[test]
fn independent_checker_distinguishes_wrong_missing_and_correct_artifact() {
    let root = temp();
    let path = root.join("out.json");
    assert_eq!(
        artifact_check(&path, &[]).verdict,
        CheckVerdict::Unavailable
    );
    fs::write(&path, b"not JSON").unwrap();
    assert_eq!(artifact_check(&path, &[]).verdict, CheckVerdict::Failed);
    fs::write(&path, b"[]").unwrap();
    assert_eq!(artifact_check(&path, &[]).verdict, CheckVerdict::Passed);
    fs::write(&path, br#"[{"name":"wrong","name":"right"}]"#).unwrap();
    let expected = vec![BTreeMap::from([("name".into(), "right".into())])];
    assert_eq!(
        artifact_check(&path, &expected).verdict,
        CheckVerdict::Failed
    );
    use std::os::unix::fs::symlink;
    fs::remove_file(&path).unwrap();
    symlink(root.join("missing"), &path).unwrap();
    assert_eq!(artifact_check(&path, &[]).verdict, CheckVerdict::Refused);
    fs::remove_dir_all(root).unwrap();
}

fn temp() -> PathBuf {
    let root = std::env::temp_dir().join(format!("bridge-authority-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::canonicalize(root).unwrap()
}
#[test]
fn freezing_refuses_an_audit_directory_reachable_by_task_file_tools() {
    let root = temp();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let authority = LocalAuthority::create(&workspace.join("audit")).unwrap();
    let text = transcript().replace(
        "\"workspace\":\"/tmp\"",
        &format!(
            "\"workspace\":{}",
            serde_json::to_string(&workspace).unwrap()
        ),
    );
    let seed = text
        .lines()
        .take(3)
        .map(|l| l.to_owned() + "\n")
        .collect::<String>();
    let path = root.join("session.jsonl");
    fs::write(&path, seed).unwrap();
    assert!(authority
        .freeze(FreezeRequest {
            transcript: path,
            artifact: workspace.join("out.json"),
            criterion: criterion(),
            expected_rows: vec![]
        })
        .is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn authority_writes_refuse_a_swapped_parent_symlink() {
    use std::os::unix::fs::symlink;
    let root = temp();
    let authority = LocalAuthority::create(&root.join("audit")).unwrap();
    fs::rename(root.join("audit"), root.join("old-audit")).unwrap();
    fs::create_dir(root.join("other")).unwrap();
    symlink(root.join("other"), root.join("audit")).unwrap();
    assert!(authority.withdraw(&"a".repeat(64)).is_err());
    assert_eq!(fs::read_dir(root.join("other")).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

struct Captured {
    root: PathBuf,
    authority: LocalAuthority,
    confirmation: String,
    source: PathBuf,
    artifact: PathBuf,
    memory: Memory,
    receipt: CaptureReceipt,
    access: Access,
}
fn captured() -> Captured {
    let root = temp();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let source = root.join("session.jsonl");
    let artifact = workspace.join("out.json");
    let text = transcript().replace(
        "\"workspace\":\"/tmp\"",
        &format!(
            "\"workspace\":{}",
            serde_json::to_string(&workspace).unwrap()
        ),
    );
    let text = text.replace(
        "\"record\":\"outcome\"",
        "\"record\":\"outcome\",\"host_unknown\":\"SENSOR_HOST_ONLY_72601\"",
    );
    fs::write(
        &source,
        text.lines()
            .take(3)
            .map(|l| l.to_owned() + "\n")
            .collect::<String>(),
    )
    .unwrap();
    let authority = LocalAuthority::create(&root.join("audit")).unwrap();
    let confirmation = authority
        .freeze(FreezeRequest {
            transcript: source.clone(),
            artifact: artifact.clone(),
            criterion: criterion(),
            expected_rows: vec![],
        })
        .unwrap();
    fs::write(&source, text).unwrap();
    fs::write(&artifact, b"[]").unwrap();
    let access = Access {
        domain: "bridge-test".into(),
        scopes: BTreeSet::new(),
    };
    let mut memory =
        Memory::create_host_capture(&root.join("test.memory-prototype.sqlite"), &access.domain)
            .unwrap()
            .with_source_guard(std::sync::Arc::new(authority.clone()));
    let receipt = authority
        .capture(
            &mut memory,
            CaptureRequest {
                confirmation: confirmation.clone(),
                domain: access.domain.clone(),
                account: "synthetic".into(),
                scopes: BTreeSet::new(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &access,
        )
        .unwrap();
    Captured {
        root,
        authority,
        confirmation,
        source,
        artifact,
        memory,
        receipt,
        access,
    }
}
#[test]
fn audit_and_source_binding_fail_closed_on_changes_and_preserve_original_taint() {
    use std::os::unix::fs::symlink;
    let f = captured();
    let binding = f.authority.binding(&f.confirmation).unwrap();
    assert_eq!(f.authority.check(&binding), SourceStatus::Checked);
    let audit = fs::read_to_string(
        f.root
            .join("audit")
            .join(format!("{}.audit", binding.locator)),
    )
    .unwrap();
    assert!(audit.contains("SENSOR_HOST_ONLY_72601"));
    let host: HostAudit = serde_json::from_str(&audit).unwrap();
    assert_eq!(
        host.taint,
        Some(crate::agent::Taint {
            private: true,
            untrusted: true
        })
    );
    assert_eq!(host.origin, crate::learning::Origin::Untrusted);
    let projection =
        serde_json::to_string(&f.memory.read(&f.receipt.projection, &f.access).unwrap()).unwrap();
    assert!(!projection.contains("SENSOR_HOST_ONLY_72601"));
    let original = fs::read(&f.source).unwrap();
    fs::write(&f.source, b"truncated").unwrap();
    assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
    assert!(f
        .memory
        .read(&f.receipt.projection, &f.access)
        .unwrap()
        .is_none());
    fs::write(&f.source, &original).unwrap();
    fs::rename(&f.source, f.root.join("original.jsonl")).unwrap();
    symlink(f.root.join("original.jsonl"), &f.source).unwrap();
    assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
    fs::remove_file(&f.source).unwrap();
    fs::rename(f.root.join("original.jsonl"), &f.source).unwrap();
    fs::write(&f.artifact, b"[{}]").unwrap();
    assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
    fs::write(&f.artifact, b"[]").unwrap();
    f.authority.withdraw(&binding.locator).unwrap();
    assert_eq!(f.authority.check(&binding), SourceStatus::Withheld);
    assert_ne!(
        LocalAuthority::open(&f.root.join("audit"))
            .unwrap()
            .check(&binding),
        SourceStatus::Checked
    );
    fs::remove_dir_all(f.root).unwrap();
}
#[test]
fn capture_only_retry_is_identical_and_changed_audit_is_never_retrusted() {
    let mut f = captured();
    let before = fs::read(&f.source).unwrap();
    let retry = f
        .authority
        .capture(
            &mut f.memory,
            CaptureRequest {
                confirmation: f.confirmation.clone(),
                domain: f.access.domain.clone(),
                account: "synthetic".into(),
                scopes: BTreeSet::new(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &f.access,
        )
        .unwrap();
    assert_eq!(retry, f.receipt);
    assert_eq!(fs::read(&f.source).unwrap(), before);
    let binding = f.authority.binding(&f.confirmation).unwrap();
    let path = f
        .root
        .join("audit")
        .join(format!("{}.audit", binding.locator));
    fs::write(path, b"SENSOR_HOST_ONLY_72601").unwrap();
    let error = f
        .authority
        .capture(
            &mut f.memory,
            CaptureRequest {
                confirmation: f.confirmation.clone(),
                domain: f.access.domain.clone(),
                account: "synthetic".into(),
                scopes: BTreeSet::new(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &f.access,
        )
        .unwrap_err()
        .to_string();
    assert_eq!(error, "host capture unavailable");
    assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn captured_source_corrections_withhold_reads_but_cost_summary_preserves_history() {
    let f = captured();
    let binding = f.authority.binding(&f.confirmation).unwrap();
    let original = fs::read_to_string(&f.source).unwrap();
    for correction in [
        Record::LateResult {
            index: 2,
            tool_use_id: "write-1".into(),
            content: "late corrected result".into(),
            is_error: true,
            external: true,
        },
        Record::LateFailure {
            run: 0,
            tool_use_id: "write-1".into(),
        },
        Record::Rewrite {
            messages: vec![Message::user("replacement history")],
        },
        Record::Extend {
            index: 2,
            blocks: vec![Block::Text {
                text: "additional material".into(),
            }],
        },
    ] {
        fs::write(
            &f.source,
            original.clone() + &serde_json::to_string(&correction).unwrap() + "\n",
        )
        .unwrap();
        assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
        assert!(f
            .memory
            .read(&f.receipt.projection, &f.access)
            .unwrap()
            .is_none());
    }
    let summary = Record::Summary {
        usage: crate::message::Usage::default(),
        turns: 1,
    };
    fs::write(
        &f.source,
        original + &serde_json::to_string(&summary).unwrap() + "\n",
    )
    .unwrap();
    assert_eq!(f.authority.check(&binding), SourceStatus::Checked);
    assert!(f
        .memory
        .read(&f.receipt.projection, &f.access)
        .unwrap()
        .is_some());
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn missing_authority_marker_withholds_previously_captured_source() {
    let f = captured();
    let binding = f.authority.binding(&f.confirmation).unwrap();
    fs::remove_file(f.root.join("audit/authority")).unwrap();
    assert_ne!(f.authority.check(&binding), SourceStatus::Checked);
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn malformed_native_memory_packets_refuse_capture() {
    for result in [
        "not JSON",
        "{}",
        r#"{"memory":{"records":[]}}"#,
        r#"{"memory":{"records":[{"reference":{"object":"x","revision":"y"}}]}}"#,
    ] {
        let text = transcript()
            .lines()
            .map(|line| {
                let mut record: Record = serde_json::from_str(line).unwrap();
                if let Record::Message(message) = &mut record {
                    for block in &mut message.content {
                        match block {
                            Block::ToolUse { name, input, .. } => {
                                *name = "goal_context".into();
                                *input = serde_json::json!({"memory":{"query":"fixture"}});
                            }
                            Block::ToolResult { content, .. } => *content = result.into(),
                            _ => {}
                        }
                    }
                }
                serde_json::to_string(&record).unwrap() + "\n"
            })
            .collect::<String>();
        assert!(
            events(&text, &criterion(), "synthetic").is_err(),
            "malformed native memory packet must not become a lineage-free original"
        );
    }
}

#[tokio::test]
async fn native_memory_packet_supports_exact_envelope_and_rejects_truncation() {
    use crate::tool::{goal_memory::GoalMemory, Tool, ToolCtx};
    let f = captured();
    let tool = GoalMemory::open_with_source_guard(
        &f.root.join("test.memory-prototype.sqlite"),
        f.access.clone(),
        "synthetic".into(),
        crate::memory::context::Situation {
            event: "Convert fixture".into(),
            goal: "task:convert".into(),
            needs: None,
        },
        std::sync::Arc::new(f.authority.clone()),
    )
    .unwrap();
    let output = tool
        .call(
            serde_json::json!({"memory":{"query":"fixture"}}),
            &ToolCtx::default(),
        )
        .await
        .unwrap();
    assert!(!output.is_error && output.external);
    let raw = output.content.clone();
    let refs = memory_packet_references(&raw).unwrap();
    assert!(refs.contains(&f.receipt.projection));
    assert!(f.receipt.originals.iter().all(|r| refs.contains(r)));
    let wrapped = crate::jobs::render(
        "goal_context",
        "memory",
        output,
        &crate::jobs::Terms {
            cap: 100_000,
            ..Default::default()
        },
    )
    .content;
    assert_eq!(refs, memory_packet_references(&wrapped).unwrap());
    assert!(
        memory_packet_references(wrapped.strip_suffix("\n</untrusted-content>").unwrap()).is_err()
    );
    assert!(memory_packet_references(
        &wrapped.replace("source=\"goal_context\"", "source=\"other\"")
    )
    .is_err());
    assert!(memory_packet_references(&raw[..raw.len() - 1]).is_err());
    let mut packet: serde_json::Value = serde_json::from_str(&raw).unwrap();
    packet["memory"]["unsupported"] = serde_json::json!(true);
    assert!(memory_packet_references(&packet.to_string()).is_err());
    packet["memory"]
        .as_object_mut()
        .unwrap()
        .remove("unsupported");
    packet["memory"]["records"][0]["reference"]["authority"] = serde_json::json!("model-claim");
    assert!(memory_packet_references(&packet.to_string()).is_err());
    fs::remove_dir_all(f.root).unwrap();
}
