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

fn evidence_events(text: &str) -> Events {
    events_with_evidence(text, &criterion(), "synthetic").unwrap()
}

#[test]
fn rich_action_evidence_preserves_requested_write_without_claiming_artifact() {
    let events = evidence_events(&transcript());
    let invocation = events.originals.iter().find_map(|p| {
        serde_json::from_str::<serde_json::Value>(&p.text)
            .ok()
            .filter(|v| v.get("call_id") == Some(&serde_json::json!("write-1")))
            .map(|value| (p, value))
    });
    assert!(
        invocation.is_some(),
        "native ToolUse input is missing from evidence"
    );
    let (passage, value) = invocation.unwrap();
    assert_eq!(passage.line, 4);
    assert_eq!(passage.pointer, "/content/0/input");
    assert_eq!(value["tool_name"], "fs_write");
    assert_eq!(value["input_status"], "Complete");
    assert_eq!(value["input"]["content"], "[]");
    assert_eq!(
        value["input_digest"],
        "af0d9c6a676e2975813dc6fcd333abd3fcf8b33a566c904198f1c3165884d607"
    );
    assert!(value.get("verdict").is_none());
}

#[test]
fn legacy_capture_keeps_exact_original_passages_without_invocation_envelope() {
    let events = events(&transcript(), &criterion(), "synthetic").unwrap();
    assert_eq!(events.originals.len(), 2);
    assert_eq!(events.originals[0].text, "Convert the fixture");
    assert_eq!(events.originals[1].text, "wrote out.json");
    assert!(events
        .originals
        .iter()
        .all(|p| !p.text.contains("input_digest")));
}

#[test]
fn artifact_evidence_retains_empty_json_array_as_actual_content() {
    let root = temp();
    let path = root.join("out.json");
    fs::write(&path, b"[]").unwrap();
    let value = serde_json::to_value(artifact_observation(&path, &[])).unwrap();
    assert_eq!(
        value["artifact_text"], "[]",
        "actual artifact bytes are missing"
    );
    assert_eq!(value["content_status"], "Complete");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rich_action_omits_oversized_input_honestly_and_refuses_clipped_ordinary_text() {
    let text = transcript();
    let long = "x".repeat(1500);
    let value: serde_json::Value = serde_json::from_str(text.lines().nth(3).unwrap()).unwrap();
    let mut changed = value;
    changed["content"][0]["input"]["content"] = serde_json::json!(long);
    let changed = text.replace(
        text.lines().nth(3).unwrap(),
        &serde_json::to_string(&changed).unwrap(),
    );
    let rich = evidence_events(&changed);
    let p = rich
        .originals
        .iter()
        .find(|p| p.kind == PassageKind::ActionInvocation)
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&p.text).unwrap();
    assert_eq!(v["input_status"], "Oversized");
    assert!(v["input"].is_null());
    assert_eq!(v["input_digest"].as_str().unwrap().len(), 64);
    assert!(!p.text.contains(&"x".repeat(30)));
    let text = transcript().replace("Convert the fixture", &"é".repeat(2049));
    assert!(events_with_evidence(&text, &criterion(), "synthetic").is_err());
    let legacy = events(&text, &criterion(), "synthetic").unwrap();
    assert_eq!(legacy.originals[0].text.len(), 4096);
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
    captured_mode(false)
}
fn captured_mode(rich: bool) -> Captured {
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
    let text = if rich {
        text.replace("\"content\":\"[]\"", "\"content\":\"requested route\"")
    } else {
        text
    };
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
            expected_rows: if rich {
                vec![BTreeMap::from([(
                    "host-only".into(),
                    "GOLD_ONLY_91827".into(),
                )])]
            } else {
                vec![]
            },
        })
        .unwrap();
    fs::write(&source, text).unwrap();
    fs::write(&artifact, b"[]").unwrap();
    let access = Access {
        domain: "bridge-test".into(),
        scopes: BTreeSet::new(),
    };
    let create = if rich {
        Memory::create_supported_situated
    } else {
        Memory::create_host_capture
    };
    let mut memory = create(&root.join("test.memory-prototype.sqlite"), &access.domain)
        .unwrap()
        .with_source_guard(std::sync::Arc::new(authority.clone()));
    let input = CaptureRequest {
        confirmation: confirmation.clone(),
        domain: access.domain.clone(),
        account: "synthetic".into(),
        scopes: BTreeSet::new(),
        consulted: vec![],
        records: SessionRecords::default(),
    };
    let receipt = if rich {
        authority.capture_with_evidence(&mut memory, input, &access)
    } else {
        authority.capture(&mut memory, input, &access)
    }
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
fn rich_capture_distinguishes_requested_contents_and_observed_bytes_without_private_gold() {
    use crate::memory::interpretation::{InterpretationContext, SourceKind};
    let mut f = captured_mode(true);
    let context = InterpretationContext::new(criterion(), "opaque-context".into()).unwrap();
    let prepared = f
        .memory
        .prepare_interpretation(
            &f.receipt,
            &[],
            &context,
            &f.memory.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    let action = prepared
        .sources()
        .iter()
        .find(|p| p.kind == SourceKind::ActionInvocation)
        .unwrap();
    let artifact = prepared
        .sources()
        .iter()
        .find(|p| p.kind == SourceKind::ArtifactObservation)
        .unwrap();
    let requested: serde_json::Value = serde_json::from_str(&action.text).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&artifact.text).unwrap();
    assert_eq!(requested["input"]["content"], "requested route");
    assert_eq!(actual["artifact_text"], "[]");
    assert_eq!(actual["verdict"], "Failed");
    assert_eq!(
        actual["artifact_digest"],
        "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
    );
    for secret in ["GOLD_ONLY_91827", "SENSOR_HOST_ONLY_72601", "expected_rows"] {
        assert!(!prepared.generation_input().contains(secret));
        for reference in f
            .receipt
            .originals
            .iter()
            .chain(std::iter::once(&f.receipt.projection))
        {
            assert!(
                !serde_json::to_string(&f.memory.read(reference, &f.access).unwrap())
                    .unwrap()
                    .contains(secret)
            );
        }
    }
    assert!(!prepared
        .generation_input()
        .contains(f.root.to_str().unwrap()));
    let again = f
        .authority
        .capture_with_evidence(
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
    assert_eq!(again, f.receipt);
    fs::write(&f.artifact, b"[1]").unwrap();
    for reference in f
        .receipt
        .originals
        .iter()
        .chain(std::iter::once(&f.receipt.projection))
    {
        assert!(f.memory.read(reference, &f.access).unwrap().is_none());
    }
    assert!(f
        .authority
        .capture_with_evidence(
            &mut f.memory,
            CaptureRequest {
                confirmation: f.confirmation.clone(),
                domain: f.access.domain.clone(),
                account: "synthetic".into(),
                scopes: BTreeSet::new(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &f.access
        )
        .is_err());
    fs::write(&f.artifact, b"[]").unwrap();
    assert!(f
        .memory
        .read(&f.receipt.projection, &f.access)
        .unwrap()
        .is_some());
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn rich_and_legacy_public_producers_refuse_the_other_store_version() {
    let mut f = captured_mode(true);
    let mut legacy = Memory::create_situated(
        &f.root.join("legacy.memory-prototype.sqlite"),
        &f.access.domain,
    )
    .unwrap()
    .with_source_guard(std::sync::Arc::new(f.authority.clone()));
    assert!(f
        .authority
        .capture_with_evidence(
            &mut legacy,
            CaptureRequest {
                confirmation: f.confirmation.clone(),
                domain: f.access.domain.clone(),
                account: "synthetic".into(),
                scopes: BTreeSet::new(),
                consulted: vec![],
                records: SessionRecords::default(),
            },
            &f.access
        )
        .is_err());
    assert_eq!(legacy.snapshot().unwrap().sequence, 0);
    assert!(f
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
            &f.access
        )
        .is_err());
    fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn rich_artifact_statuses_preserve_complete_bytes_and_distinguish_omission_reasons() {
    let root = temp();
    let path = root.join("out.json");
    let missing = artifact_observation(&path, &[]);
    assert_eq!(missing.content_status, ContentStatus::Missing);
    assert_eq!(missing.verdict, CheckVerdict::Unavailable);
    assert!(missing.artifact_text.is_none() && missing.artifact_digest.is_none());
    let cases = [
        (
            b"[]".to_vec(),
            ContentStatus::Complete,
            Some("[]"),
            CheckVerdict::Passed,
        ),
        (
            vec![],
            ContentStatus::Complete,
            Some(""),
            CheckVerdict::Failed,
        ),
        (
            vec![0xff, 0xfe],
            ContentStatus::NonUtf8,
            None,
            CheckVerdict::Failed,
        ),
        (
            [b"[]".as_slice(), vec![b' '; 2046].as_slice()].concat(),
            ContentStatus::Complete,
            None,
            CheckVerdict::Passed,
        ),
        (
            [b"[]".as_slice(), vec![b' '; 2047].as_slice()].concat(),
            ContentStatus::Oversized,
            None,
            CheckVerdict::Passed,
        ),
        (
            br#"[{"name":"wrong","name":"right"}]"#.to_vec(),
            ContentStatus::Complete,
            None,
            CheckVerdict::Failed,
        ),
    ];
    for (bytes, status, text, verdict) in cases {
        fs::write(&path, &bytes).unwrap();
        let observed = artifact_observation(&path, &[]);
        assert_eq!(observed.content_status, status);
        assert_eq!(observed.verdict, verdict);
        assert_eq!(observed.artifact_digest, Some(digest(&bytes)));
        if status == ContentStatus::Complete {
            assert_eq!(
                observed.artifact_text.as_deref(),
                Some(std::str::from_utf8(&bytes).unwrap())
            );
        } else {
            assert!(observed.artifact_text.is_none());
        }
        if let Some(text) = text {
            assert_eq!(observed.artifact_text.as_deref(), Some(text));
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, vec![b' '; ARTIFACT_MAX + 1]).unwrap();
    let oversized = artifact_observation(&path, &[]);
    assert_eq!(oversized.content_status, ContentStatus::Oversized);
    assert_eq!(oversized.verdict, CheckVerdict::Refused);
    assert!(oversized.artifact_text.is_none() && oversized.artifact_digest.is_none());
    let checker_boundary = [b"[]".as_slice(), vec![b' '; ARTIFACT_MAX - 2].as_slice()].concat();
    fs::write(&path, &checker_boundary).unwrap();
    let boundary = artifact_observation(&path, &[]);
    assert_eq!(boundary.content_status, ContentStatus::Oversized);
    assert_eq!(boundary.verdict, CheckVerdict::Passed);
    assert_eq!(boundary.artifact_digest, Some(digest(&checker_boundary)));
    let unavailable = artifact_observation(&root.join("x".repeat(256)), &[]);
    assert_eq!(unavailable.content_status, ContentStatus::Unavailable);
    assert_eq!(unavailable.verdict, CheckVerdict::Unavailable);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rich_artifact_refuses_final_and_parent_symlinks_and_fifo_without_reading() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let root = temp();
    let parent = root.join("real");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("out.json");
    fs::write(&path, b"[]").unwrap();
    symlink(&parent, root.join("linked")).unwrap();
    symlink(&path, root.join("out.json")).unwrap();
    let fifo = root.join("pipe");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: the NUL-terminated path remains live, and the mode is ordinary.
    assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
    for path in [root.join("linked/out.json"), root.join("out.json"), fifo] {
        let observed = artifact_observation(&path, &[]);
        assert_eq!(observed.content_status, ContentStatus::Refused);
        assert_eq!(observed.verdict, CheckVerdict::Refused);
        assert!(observed.artifact_text.is_none() && observed.artifact_digest.is_none());
    }
    fs::remove_dir_all(root).unwrap();
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
        assert!(
            events_with_evidence(&text, &criterion(), "synthetic").is_err(),
            "omitted rich memory results must still validate the complete packet"
        );
    }
}

#[tokio::test]
async fn rich_memory_result_keeps_all_references_and_receipt_without_duplicate_packet_passage() {
    use crate::tool::{goal_memory::GoalMemory, Tool, ToolCtx};
    let f = captured_mode(true);
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
    assert!(
        output.content.len() > 4096,
        "exercise a complete packet beyond the ordinary passage bound"
    );
    let raw = output.content;
    let native_with_result = |content: &str, memory_read: bool| {
        transcript()
            .lines()
            .map(|line| {
                let mut record: Record = serde_json::from_str(line).unwrap();
                if let Record::Message(message) = &mut record {
                    for block in &mut message.content {
                        match block {
                            Block::ToolUse { name, input, .. } if memory_read => {
                                *name = "goal_context".into();
                                *input = serde_json::json!({"memory":{"query":"fixture"}});
                            }
                            Block::ToolResult {
                                content: result, ..
                            } => *result = content.into(),
                            _ => {}
                        }
                    }
                }
                serde_json::to_string(&record).unwrap() + "\n"
            })
            .collect::<String>()
    };
    let text = native_with_result(&raw, true);
    let rich = events_with_evidence(&text, &criterion(), "synthetic").unwrap();
    let packet: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let expected = packet["memory"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| RevisionRef {
            object: hit["reference"]["object"].as_str().unwrap().into(),
            revision: hit["reference"]["revision"].as_str().unwrap().into(),
        })
        .collect::<Vec<_>>();
    assert_eq!(rich.consulted, expected);
    assert!(rich.consulted.contains(&f.receipt.projection));
    assert!(f
        .receipt
        .originals
        .iter()
        .all(|r| rich.consulted.contains(r)));
    assert_eq!(rich.receipts.len(), 1);
    assert_eq!(rich.receipts[0].call_id, "write-1");
    assert_eq!(rich.receipts[0].tool_name, "goal_context");
    assert_eq!(rich.receipts[0].status, ReceiptStatus::Succeeded);
    assert_eq!(rich.originals.len(), 2);
    assert!(rich
        .originals
        .iter()
        .all(|p| p.kind != PassageKind::ToolResult));
    assert!(rich
        .originals
        .iter()
        .all(|p| !p.text.contains(&f.receipt.projection.revision)));
    let legacy = events(&text, &criterion(), "synthetic").unwrap();
    assert_eq!(legacy.originals[1].kind, PassageKind::ToolResult);
    assert_eq!(legacy.originals[1].text.len(), 4096);
    assert!(raw.starts_with(&legacy.originals[1].text));
    let ordinary_long = native_with_result(&"x".repeat(4097), false);
    assert!(events_with_evidence(&ordinary_long, &criterion(), "synthetic").is_err());
    assert_eq!(
        events(&ordinary_long, &criterion(), "synthetic")
            .unwrap()
            .originals[1]
            .text
            .len(),
        4096
    );
    let mut unexpected: serde_json::Value = serde_json::from_str(&raw).unwrap();
    unexpected["memory"]["unknown_shape"] = serde_json::json!(true);
    assert!(events_with_evidence(
        &native_with_result(&unexpected.to_string(), true),
        &criterion(),
        "synthetic"
    )
    .is_err());
    fs::remove_dir_all(f.root).unwrap();
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

// Exercise the native ProgramCall/Record wire format with an actual GoalMemory
// result. The interpreter itself is deliberately not required by this refusal
// test: even a lost program record must not admit an untracked `code` result.
async fn unsupported_program_capture(tool_name: &str, record_program: bool, legacy_binding: bool) {
    use crate::tool::{code::record, goal_memory::GoalMemory, Tool, ToolCtx};
    let mut f = captured();
    let db_path = f.root.join("test.memory-prototype.sqlite");
    let memory = GoalMemory::open_with_source_guard(
        &db_path,
        f.access.clone(),
        "synthetic".into(),
        crate::memory::context::Situation {
            event: "Read prior fixture".into(),
            goal: "task:convert".into(),
            needs: None,
        },
        std::sync::Arc::new(f.authority.clone()),
    )
    .unwrap();
    let input = serde_json::json!({"memory":{"query":"fixture"}});
    let result = memory
        .call(input.clone(), &ToolCtx::default())
        .await
        .unwrap();
    assert!(!result.is_error && result.external);
    let copied = crate::jobs::render(
        tool_name,
        "write-1",
        crate::tool::ToolOutput::ok(result.content.clone() + "\n").from_outside(),
        &crate::jobs::Terms {
            cap: 100_000,
            ..Default::default()
        },
    )
    .content;
    let mut call = record::ProgramCall::ran(
        "write-1.host-1",
        "write-1",
        "goal_context",
        input,
        true,
        false,
        std::sync::Arc::from(result.content.as_str()),
    );
    call.blob = Some(
        record::Store::at(f.root.join("program-blobs"))
            .put("synthetic-program-session", &result.content)
            .unwrap(),
    );
    if tool_name != crate::tool::code::NAME {
        call.under = Some(record::Under {
            call: "write-1".into(),
            tool: tool_name.into(),
            trusted: false,
        });
    }
    let mut records: Vec<Record> = transcript()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for record in &mut records {
        match record {
            Record::Meta(meta) => {
                meta.id = "synthetic-program-session".into();
                meta.workspace = f.root.join("workspace");
            }
            Record::Message(message) => {
                for block in &mut message.content {
                    match block {
                        Block::ToolUse { name, input, .. } => {
                            *name = tool_name.into();
                            *input = serde_json::json!({"program":"print(goal_context(memory={\"query\": \"fixture\"}))"});
                        }
                        Block::ToolResult { content, .. } => *content = copied.clone(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if record_program {
        records.insert(6, Record::Program { calls: vec![call] });
    }
    let path = f.root.join("program-session.jsonl");
    let serialize = |records: &[Record]| {
        records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap() + "\n")
            .collect::<String>()
    };
    fs::write(&path, serialize(&records[..3])).unwrap();
    let confirmation = f
        .authority
        .freeze(FreezeRequest {
            transcript: path.clone(),
            artifact: f.artifact.clone(),
            criterion: criterion(),
            expected_rows: vec![],
        })
        .unwrap();
    fs::write(&path, serialize(&records)).unwrap();
    if legacy_binding {
        // Reconstruct a previously issued binding using the native completed
        // trace. Seed through typed admission with the former permissive guard;
        // the current public capture path is tested separately below.
        struct FormerGuard;
        impl ExternalSourceGuard for FormerGuard {
            fn check(&self, _: &ExternalBinding) -> SourceStatus {
                SourceStatus::Checked
            }
        }
        let record_projection = f
            .memory
            .read(&f.receipt.projection, &f.access)
            .unwrap()
            .unwrap();
        let crate::memory::Record::Evidence(projection) = record_projection else {
            panic!("projection missing")
        };
        let mut input: HostCapture = serde_json::from_str(&projection.text).unwrap();
        let old_binding = f.authority.binding(&f.confirmation).unwrap();
        let mut audit: HostAudit = serde_json::from_slice(
            &fs::read(f.authority.file(&old_binding.locator, "audit").unwrap()).unwrap(),
        )
        .unwrap();
        let (_, frozen_bytes) = f.authority.frozen(&confirmation).unwrap();
        audit.confirmation = confirmation;
        audit.frozen_digest = digest(&frozen_bytes);
        audit.transcript = serialize(&records);
        audit.prefix_digest = digest(audit.transcript.as_bytes());
        let audit_bytes = serde_json::to_vec(&audit).unwrap();
        let binding = ExternalBinding {
            authority: AUTHORITY.into(),
            locator: digest(
                serde_json::to_string(&(&audit.confirmation, &audit.prefix_digest))
                    .unwrap()
                    .as_bytes(),
            ),
            revision: digest(&audit_bytes),
        };
        write_once(
            &f.authority.file(&binding.locator, "audit").unwrap(),
            &audit_bytes,
        )
        .unwrap();
        input.identity.session_id = "synthetic-program-session".into();
        input.identity.prefix_digest = audit.prefix_digest;
        input.binding = binding.clone();
        for original in &mut input.originals {
            original.source.id = original
                .source
                .id
                .replace("synthetic-session", "synthetic-program-session");
            if original.kind == PassageKind::ToolResult {
                original.text = copied.chars().take(4096).collect();
            }
        }
        input.receipts[0].tool_name = tool_name.into();
        let legacy_path = f.root.join("legacy.memory-prototype.sqlite");
        let mut legacy = Memory::create_host_capture(&legacy_path, &f.access.domain)
            .unwrap()
            .with_source_guard(std::sync::Arc::new(FormerGuard));
        let receipt = legacy.admit_host_capture(input, &f.access).unwrap();
        assert!(legacy
            .read(&receipt.projection, &f.access)
            .unwrap()
            .is_some());
        drop(legacy);
        let reopened = Memory::open(&legacy_path)
            .unwrap()
            .with_source_guard(std::sync::Arc::new(f.authority.clone()));
        assert_ne!(
            f.authority.check(&binding),
            SourceStatus::Checked,
            "previously issued native program binding must be withheld too"
        );
        for reference in receipt
            .originals
            .iter()
            .chain(std::iter::once(&receipt.projection))
        {
            assert!(reopened.read(reference, &f.access).unwrap().is_none());
        }
        let tool = GoalMemory::open_with_source_guard(
            &legacy_path,
            f.access.clone(),
            "synthetic".into(),
            crate::memory::context::Situation {
                event: "Read prior fixture".into(),
                goal: "task:convert".into(),
                needs: None,
            },
            std::sync::Arc::new(f.authority.clone()),
        )
        .unwrap();
        let result = tool
            .call(
                serde_json::json!({"memory":{"query":"fixture"}}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            serde_json::from_str::<serde_json::Value>(&result.content).unwrap()["memory"]
                ["records"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(f.root).unwrap();
        return;
    }
    // Both first capture and persistence-only retry must refuse before writing
    // an audit, binding, receipt, original, projection, or change sequence.
    let before_db = fs::read(&db_path).unwrap();
    let before_sequence = f.memory.snapshot().unwrap().sequence;
    let authority_files = || {
        fs::read_dir(f.root.join("audit"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<BTreeSet<_>>()
    };
    let before_files = authority_files();
    for _ in 0..2 {
        let error = f
            .authority
            .capture(
                &mut f.memory,
                CaptureRequest {
                    confirmation: confirmation.clone(),
                    domain: f.access.domain.clone(),
                    account: "synthetic".into(),
                    scopes: BTreeSet::new(),
                    consulted: vec![],
                    records: SessionRecords::default(),
                },
                &f.access,
            )
            .expect_err("unsupported program output must not become lineage-free memory");
        assert_eq!(error.to_string(), "host capture unavailable");
        assert_eq!(f.memory.snapshot().unwrap().sequence, before_sequence);
        assert_eq!(fs::read(&db_path).unwrap(), before_db);
        assert_eq!(authority_files(), before_files);
    }
    fs::remove_dir_all(f.root).unwrap();
}

#[tokio::test]
async fn native_code_memory_read_refuses_before_any_capture_writes() {
    unsupported_program_capture(crate::tool::code::NAME, true, false).await;
}

#[tokio::test]
async fn native_code_without_program_evidence_still_refuses_capture() {
    unsupported_program_capture(crate::tool::code::NAME, false, false).await;
}

#[tokio::test]
async fn native_program_carried_by_another_tool_refuses_capture() {
    unsupported_program_capture("synthetic_delegate", true, false).await;
}

#[tokio::test]
async fn previously_issued_program_bindings_are_withheld_by_read_and_goalmemory() {
    unsupported_program_capture(crate::tool::code::NAME, false, true).await;
    unsupported_program_capture("synthetic_delegate", true, true).await;
}

#[test]
fn host_checker_uses_frozen_gold_independently_of_capture() {
    let f = captured();
    assert_eq!(
        f.authority.check_artifact(&f.confirmation).unwrap().verdict,
        CheckVerdict::Passed
    );
    fs::write(&f.artifact, b"[{}]").unwrap();
    assert_eq!(
        f.authority.check_artifact(&f.confirmation).unwrap().verdict,
        CheckVerdict::Failed
    );
    fs::remove_file(&f.artifact).unwrap();
    assert_eq!(
        f.authority.check_artifact(&f.confirmation).unwrap().verdict,
        CheckVerdict::Unavailable
    );
    assert!(f.authority.check_artifact(&"0".repeat(64)).is_err());
}

#[test]
fn budget_notice_harness_passages_are_not_admitted_as_owner_requests() {
    let notice="Run budget: two model turns remain, including this one. Preserve time for the requested deliverable and an honest account of unresolved requirements. Continue gathering evidence when a concrete available source or new result justifies it. Otherwise finish from supported evidence and state what is missing, using any fallback the user specified. Do not invent missing facts.";
    let mut records: Vec<Record> = transcript()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    if let Record::Message(message) = &mut records[4] {
        message.content.push(Block::text(notice));
        message.content.push(Block::text(
            "Use the revised fixture supplied by the owner.",
        ));
    } else {
        panic!("fixture tool result missing");
    }
    let text: String = records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap() + "\n")
        .collect();
    let projected = events(&text, &criterion(), "synthetic").unwrap();
    assert!(text.contains(notice), "raw audit must retain the advice");
    assert!(!projected.originals.iter().any(|p| p.text == notice));
    let user: Vec<_> = projected
        .originals
        .iter()
        .filter(|p| matches!(p.kind, PassageKind::UserRequest))
        .collect();
    assert_eq!(user.len(), 2);
    assert_eq!(user[0].text, "Convert the fixture");
    assert_eq!(
        user[1].text,
        "Use the revised fixture supplied by the owner."
    );
    assert_eq!(user[1].pointer, "/content/2/text");
    assert_eq!(user[1].line, 5);
}

#[test]
fn contextual_packet_requires_an_exhaustive_consistent_role_map() {
    let reference =
        serde_json::json!({"object":"interpretation-object","revision":"interpretation-revision"});
    let mut packet = serde_json::json!({"memory":{
        "known_at":{"instance":"fixture","sequence":1},"valid_at":null,
        "situation":{"event":"fixture","goal":"task:convert","needs":null},
        "bindings":[],"records":[{"reference":reference,"sequence":1,
          "record":{"kind":"interpretation","data":{"context_key":"context","criterion":criterion(),"host_context_id":"opaque","claim_sources":[],
            "draft":{"interpretation":"provisional","claims":[],"judgments":[],"prediction":null,
              "grounding":{"offered":0,"dropped":0,"dropped_by":{},"unresolved_goals":0},"model":"authored","producer_version":"v1"}}},
          "possible_time":false,"required_scopes":[],"private_data":true,"untrusted_input":true}],
        "incomplete":true,"needs_unavailable":true,"budget_omitted":false,"evidence_limit":"fixture"
    }});
    assert!(
        memory_packet_references(&packet.to_string()).is_err(),
        "missing roles must refuse"
    );
    let selected = serde_json::json!({"reference":reference,"role":"SelectedCurrent"});
    packet["memory"]["interpretation_roles"] = serde_json::json!([selected]);
    assert_eq!(
        memory_packet_references(&packet.to_string()).unwrap().len(),
        1
    );
    for roles in [
        serde_json::json!([]),
        serde_json::json!([selected, selected]),
        serde_json::json!([{"reference":{"object":"foreign","revision":"foreign"},"role":"SelectedCurrent"}]),
        serde_json::json!([{"reference":reference,"role":"HistoricalDependency"}]),
    ] {
        let mut bad = packet.clone();
        bad["memory"]["interpretation_roles"] = roles;
        assert!(memory_packet_references(&bad.to_string()).is_err());
    }
    let mut bad = packet.clone();
    bad["memory"]["situation"]["goal"] = serde_json::json!("task:other");
    assert!(memory_packet_references(&bad.to_string()).is_err());
    let mut bad = packet.clone();
    bad["memory"]["interpretation_roles"][0]["extra"] = serde_json::json!(true);
    assert!(memory_packet_references(&bad.to_string()).is_err());
    packet["memory"]["records"] = serde_json::json!([]);
    assert!(memory_packet_references(&packet.to_string()).is_err());
    packet["memory"]["interpretation_roles"] = serde_json::json!([]);
    assert!(memory_packet_references(&packet.to_string())
        .unwrap()
        .is_empty());
}
