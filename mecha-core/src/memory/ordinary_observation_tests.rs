use super::{host_capture::*, *};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const ORDINARY_PROTOCOL: &str = "ordinary-session-observation-v1";
const ORDINARY_RECORDER: &str = "ordinary-session-recorder-v1";

#[derive(Default)]
struct Guard(Mutex<BTreeSet<String>>);
impl ExternalSourceGuard for Guard {
    fn check(&self, binding: &ExternalBinding) -> SourceStatus {
        if self.0.lock().unwrap().contains(&binding.locator) {
            SourceStatus::Withheld
        } else {
            SourceStatus::Checked
        }
    }
}

struct Fixture {
    dir: PathBuf,
    db: Memory,
    access: Access,
    guard: Arc<Guard>,
}

impl Fixture {
    fn new(create: impl FnOnce(&Path, &str) -> anyhow::Result<Memory>) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ordinary-observation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let guard = Arc::new(Guard::default());
        let db = create(&dir.join("capture.memory-prototype.sqlite"), "test")
            .unwrap()
            .with_source_guard(guard.clone());
        Self {
            dir,
            db,
            access: Access {
                domain: "test".into(),
                scopes: BTreeSet::new(),
            },
            guard,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn passage(
    session: &str,
    kind: PassageKind,
    connector: &str,
    pointer: &str,
    text: &str,
) -> OriginalPassage {
    OriginalPassage {
        source: SourceKey {
            connector: connector.into(),
            account: "test".into(),
            id: format!("{session}:{pointer}"),
        },
        kind,
        line: 1,
        pointer: pointer.into(),
        text: text.into(),
        occurred_at: None,
        external: None,
    }
}

fn capture(session: &str, locator: &str, prompt: &str) -> HostCapture {
    HostCapture {
        identity: CaptureIdentity {
            domain: "test".into(),
            account: "test".into(),
            session_id: session.into(),
            prefix_digest: "a".repeat(64),
            check_id: "b".repeat(64),
            check_version: ORDINARY_PROTOCOL.into(),
            evaluator_version: ORDINARY_RECORDER.into(),
        },
        binding: ExternalBinding {
            authority: "test-host".into(),
            locator: locator.repeat(64),
            revision: "d".repeat(64),
        },
        criterion: Some(ConfirmedCriterion {
            goal: "charter:1".into(),
            goal_text: "Sort fixture records".into(),
            criterion_text: prompt.into(),
            criterion_revision: "e".repeat(64),
            confirmation_digest: "f".repeat(64),
        }),
        chat_context: None,
        originals: vec![passage(
            session,
            PassageKind::UserRequest,
            "test-host",
            "/content/0/text",
            prompt,
        )],
        receipts: vec![],
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Unavailable,
        },
        consulted: vec![],
        scopes: BTreeSet::new(),
        occurred_at: None,
    }
}

#[test]
fn ordinary_labels_cannot_enter_through_legacy_admission() {
    let mut f = Fixture::new(Memory::create_situated);
    for (check, evaluator) in [
        (ORDINARY_PROTOCOL, "previous-evaluator"),
        ("previous-check", ORDINARY_RECORDER),
        (ORDINARY_PROTOCOL, ORDINARY_RECORDER),
        ("ordinary-chat-observation-v1", "previous-evaluator"),
        ("previous-check", "ordinary-chat-recorder-v1"),
        ("ordinary-chat-observation-v1", "ordinary-chat-recorder-v1"),
    ] {
        let mut value = capture("spoof", "a", "Group fixture records");
        value.identity.check_version = check.into();
        value.identity.evaluator_version = evaluator.into();
        value.receipts.push(ToolReceipt {
            call_id: "tool-1".into(),
            tool_name: "fs_write".into(),
            status: ReceiptStatus::Succeeded,
        });
        assert!(
            f.db.admit_host_capture(value, &f.access).is_err(),
            "legacy admission accepted reserved labels {check} / {evaluator}"
        );
    }
}

use super::ordinary_observation::OrdinaryObservationCapture;
use serde_json::json;
use sha2::{Digest, Sha256};

fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn with_invocation(mut value: HostCapture, status: ReceiptStatus) -> HostCapture {
    let input = json!({"path":"fixture.json"});
    let action = serde_json::to_string(&ActionInvocation {
        call_id: "tool-1".into(),
        tool_name: "fs_write".into(),
        input_status: InputStatus::Complete,
        input_digest: digest_bytes(&serde_json::to_vec(&input).unwrap()),
        input,
    })
    .unwrap();
    let session = &value.identity.session_id;
    value.originals.push(passage(
        session,
        PassageKind::ActionInvocation,
        "host-action-invocation-v1",
        "/content/1/input",
        &action,
    ));
    value.receipts.push(ToolReceipt {
        call_id: "tool-1".into(),
        tool_name: "fs_write".into(),
        status,
    });
    value
}

fn admit(f: &mut Fixture, value: HostCapture) -> CaptureReceipt {
    f.db.admit_ordinary_observation(
        OrdinaryObservationCapture::try_from_host(value).unwrap(),
        &f.access,
    )
    .unwrap()
}

fn checked_capture(value: &HostCapture) -> HostCapture {
    let mut rich = with_invocation(value.clone(), ReceiptStatus::Succeeded);
    rich.identity.check_version = EVIDENCE_CHECK_VERSION.into();
    rich.identity.evaluator_version = "previous-evaluator".into();
    let artifact_text = "[]";
    let artifact_digest = Some(digest_bytes(artifact_text.as_bytes()));
    rich.artifact = ArtifactCheck {
        artifact_digest: artifact_digest.clone(),
        verdict: CheckVerdict::Passed,
    };
    rich.originals.push(passage(
        &rich.identity.session_id,
        PassageKind::ArtifactObservation,
        "host-artifact-observation-v1",
        "/artifact_observation",
        &serde_json::to_string(&ArtifactObservation {
            content_status: ContentStatus::Complete,
            artifact_text: Some(artifact_text.into()),
            artifact_digest,
            verdict: CheckVerdict::Passed,
        })
        .unwrap(),
    ));
    rich
}

#[test]
fn successful_tool_and_assistant_claim_still_yield_not_evaluated() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let mut input = with_invocation(
        capture("ordinary", "a", "Write the fixture record"),
        ReceiptStatus::Succeeded,
    );
    input.originals.push(passage(
        "ordinary",
        PassageKind::ToolResult,
        "test-host",
        "/content/2/content",
        "Wrote fixture.json",
    ));
    input.originals.push(passage(
        "ordinary",
        PassageKind::AssistantText,
        "test-host",
        "/content/3/text",
        "All other records were independently checked.",
    ));
    let receipt = admit(&mut f, input.clone());
    assert_eq!(receipt, admit(&mut f, input));
    let assessment =
        f.db.capture_semantic_assessment(&receipt, &f.access)
            .unwrap();
    assert!(matches!(assessment, SemanticAssessment::NotEvaluated));
    assert_eq!(serde_json::to_value(assessment).unwrap(), "not_evaluated");
    let Some(Record::Evidence(projection)) = f.db.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("missing ordinary projection");
    };
    let canonical = projection_input(&f.db.conn, &projection).unwrap();
    assert_eq!(canonical.artifact.verdict, CheckVerdict::Unavailable);
    assert!(canonical.artifact.artifact_digest.is_none());
    assert_eq!(canonical.originals.len(), 4);
    assert!(canonical
        .originals
        .iter()
        .all(|p| p.kind != PassageKind::ArtifactObservation));
    let mut forged = receipt;
    forged.originals.pop();
    assert!(f
        .db
        .capture_semantic_assessment(&forged, &f.access)
        .is_err());
}

/// What the transcript recorded about where a result came from is stored
/// with the passage and read back unchanged — outside, local, and unknown
/// kept apart, because unknown is never read as local.
#[test]
fn where_a_result_came_from_survives_admission_and_the_canonical_reader() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let mut input = with_invocation(
        capture("provenance", "a", "Read the fixture record"),
        ReceiptStatus::Succeeded,
    );
    for (pointer, text, external) in [
        (
            "/content/2/content",
            "fetched from a third party",
            Some(true),
        ),
        ("/content/3/content", "read from the workspace", Some(false)),
        ("/content/4/content", "recorded before provenance", None),
    ] {
        let mut result = passage(
            "provenance",
            PassageKind::ToolResult,
            "test-host",
            pointer,
            text,
        );
        result.external = external;
        input.originals.push(result);
    }
    let receipt = admit(&mut f, input);
    let Some(Record::Evidence(projection)) = f.db.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("missing ordinary projection");
    };
    let canonical = projection_input(&f.db.conn, &projection).unwrap();
    let stored: Vec<_> = canonical
        .originals
        .iter()
        .filter(|p| p.kind == PassageKind::ToolResult)
        .map(|p| (p.text.as_str(), p.external))
        .collect();
    assert_eq!(
        stored,
        [
            ("fetched from a third party", Some(true)),
            ("read from the workspace", Some(false)),
            ("recorded before provenance", None),
        ]
    );
    // And it is part of what the store authenticates: the same passages with
    // a different answer are a different capture.
    let wire = serde_json::to_string(&canonical).unwrap();
    assert!(wire.contains(r#""external":true"#) && wire.contains(r#""external":false"#));
}

#[test]
fn no_tool_run_is_an_observation_in_v8_only() {
    let mut older = Fixture::new(Memory::create_situated);
    let input = capture("no-tool", "a", "Explain the fixture");
    assert!(older
        .db
        .admit_ordinary_observation(
            OrdinaryObservationCapture::try_from_host(input.clone()).unwrap(),
            &older.access,
        )
        .is_err());
    let mut f = Fixture::new(Memory::create_supported_situated);
    let receipt = admit(&mut f, input);
    assert!(matches!(
        f.db.capture_semantic_assessment(&receipt, &f.access)
            .unwrap(),
        SemanticAssessment::NotEvaluated
    ));
}

#[test]
fn ordinary_rejects_artifact_semantics_and_protocol_spoofing() {
    let valid = with_invocation(
        capture("ordinary", "a", "Write the fixture record"),
        ReceiptStatus::Succeeded,
    );
    let mut f = Fixture::new(Memory::create_supported_situated);
    let mut exact_gold = checked_capture(&capture("gold", "b", "Check fixture rows"));
    exact_gold.identity.evaluator_version = ORDINARY_RECORDER.into();
    assert!(f
        .db
        .admit_host_capture_with_evidence(exact_gold, &f.access)
        .is_err());

    for (label, modify) in [
        (
            "passed",
            (|v: &mut HostCapture| v.artifact.verdict = CheckVerdict::Passed)
                as fn(&mut HostCapture),
        ),
        ("digest", |v| {
            v.artifact.artifact_digest = Some("a".repeat(64))
        }),
        ("artifact passage", |v| {
            v.originals
                .push(checked_capture(v).originals.pop().unwrap())
        }),
        ("different protocol", |v| {
            v.identity.check_version = EVIDENCE_CHECK_VERSION.into()
        }),
        ("different recorder", |v| {
            v.identity.evaluator_version = "previous-evaluator".into()
        }),
    ] {
        let mut value = valid.clone();
        modify(&mut value);
        assert!(
            OrdinaryObservationCapture::try_from_host(value).is_err(),
            "accepted {label}"
        );
    }
}

#[test]
fn ordinary_receipts_match_unique_invocations_by_id_and_name() {
    let input = capture("ordinary", "a", "Write the fixture record");
    for status in [
        ReceiptStatus::Succeeded,
        ReceiptStatus::Failed,
        ReceiptStatus::Unavailable,
    ] {
        assert!(
            OrdinaryObservationCapture::try_from_host(with_invocation(input.clone(), status))
                .is_ok()
        );
    }
    let valid = with_invocation(input, ReceiptStatus::Failed);
    for (label, modify) in [
        (
            "duplicate receipt",
            (|v: &mut HostCapture| v.receipts.push(v.receipts[0].clone())) as fn(&mut HostCapture),
        ),
        ("duplicate invocation", |v| {
            let mut original = v.originals[1].clone();
            original.source.id.push_str("-second");
            v.originals.push(original);
        }),
        ("missing receipt", |v| v.receipts.clear()),
        ("missing invocation", |v| {
            v.originals.pop();
        }),
        ("different tool", |v| {
            v.receipts[0].tool_name = "fs_read".into()
        }),
        ("different id", |v| v.receipts[0].call_id = "tool-2".into()),
        ("guessed refusal", |v| {
            v.receipts[0].status = ReceiptStatus::Refused
        }),
        ("pending", |v| v.receipts[0].status = ReceiptStatus::Pending),
    ] {
        let mut value = valid.clone();
        modify(&mut value);
        assert!(
            OrdinaryObservationCapture::try_from_host(value).is_err(),
            "accepted {label}"
        );
    }
}

#[test]
fn ordinary_overflow_is_refused_before_admission() {
    let valid = capture("ordinary", "a", "Write the fixture record");
    let mut too_many_passages = valid.clone();
    let mut too_many_bytes = valid.clone();
    for n in 1..=16 {
        let p = passage(
            "ordinary",
            PassageKind::AssistantText,
            "test-host",
            &format!("/text/{n}"),
            "fixture",
        );
        too_many_passages.originals.push(p.clone());
        if n <= 9 {
            too_many_bytes.originals.push(OriginalPassage {
                text: "a".repeat(4096),
                ..p
            });
        }
    }
    let mut too_many_receipts = with_invocation(valid.clone(), ReceiptStatus::Succeeded);
    too_many_receipts.receipts = vec![too_many_receipts.receipts[0].clone(); 33];
    let mut too_many_consulted = valid;
    too_many_consulted.consulted = (0..65)
        .map(|n| RevisionRef {
            object: format!("object-{n}"),
            revision: format!("revision-{n}"),
        })
        .collect();

    for input in [too_many_passages, too_many_receipts, too_many_consulted] {
        let error = OrdinaryObservationCapture::try_from_host(input)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "invalid host capture bounds");
    }
    let error = OrdinaryObservationCapture::try_from_host(too_many_bytes)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "oversized host projection");
}

#[test]
fn ordinary_and_rich_reopen_with_unchanged_rich_reader_bytes() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let ordinary = capture("ordinary", "a", "Explain the fixture");
    let observation = admit(&mut f, ordinary);
    let rich = checked_capture(&capture("rich", "b", "Check fixture rows"));
    let receipt =
        f.db.admit_host_capture_with_evidence(rich.clone(), &f.access)
            .unwrap();
    let live = Memory::open(&f.dir.join("capture.memory-prototype.sqlite"))
        .unwrap()
        .with_source_guard(f.guard.clone());
    assert!(matches!(
        live.capture_semantic_assessment(&observation, &f.access)
            .unwrap(),
        SemanticAssessment::NotEvaluated
    ));
    let SemanticAssessment::ArtifactCheck(check) = live
        .capture_semantic_assessment(&receipt, &f.access)
        .unwrap()
    else {
        panic!("expected exact-gold artifact check");
    };
    assert_eq!(check.verdict, CheckVerdict::Passed);
    assert_eq!(check.artifact_digest, rich.artifact.artifact_digest);
    let Some(Record::Evidence(projection)) = live.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("missing rich projection");
    };
    let reader = native_reader(&live.conn, &receipt.projection, &projection)
        .unwrap()
        .unwrap();
    let expected = json!({
        "identity": rich.identity,
        "criterion": rich.criterion,
        "originals": [
            {"reference": receipt.originals[0], "kind": "UserRequest", "line": 1, "pointer": "/content/0/text"},
            {"reference": receipt.originals[1], "kind": "ActionInvocation", "line": 1, "pointer": "/content/1/input"},
            {"reference": receipt.originals[2], "kind": "ArtifactObservation", "line": 1, "pointer": "/artifact_observation"}
        ],
        "receipts": [{"call_id":"tool-1","tool_name":"fs_write","status":"Succeeded"}],
        "artifact": {"artifact_digest": digest_bytes(b"[]"), "verdict": "Passed"},
        "consulted": [],
        "occurred_at": null
    });
    assert_eq!(
        serde_json::to_string(&reader).unwrap(),
        expected.to_string()
    );
}

#[test]
fn historical_unavailable_remains_an_artifact_check() {
    let mut f = Fixture::new(Memory::create_situated);
    let mut legacy = capture("legacy", "a", "Explain the fixture");
    legacy.identity.check_version = "historical-custom-check".into();
    legacy.identity.evaluator_version = "historical-custom-evaluator".into();
    legacy.receipts.push(ToolReceipt {
        call_id: "tool-1".into(),
        tool_name: "fs_write".into(),
        status: ReceiptStatus::Unavailable,
    });
    let receipt = f.db.admit_host_capture(legacy, &f.access).unwrap();
    let SemanticAssessment::ArtifactCheck(check) =
        f.db.capture_semantic_assessment(&receipt, &f.access)
            .unwrap()
    else {
        panic!("historical unavailable was reinterpreted");
    };
    assert_eq!(check.verdict, CheckVerdict::Unavailable);
    assert!(check.artifact_digest.is_none());
}

#[test]
fn source_withdrawal_and_forgetting_reach_every_ordinary_passage() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let ancestor = capture("ancestor", "a", "Retain the violet fixture");
    let ancestor_receipt = admit(&mut f, ancestor.clone());
    let mut derived = with_invocation(
        capture("dependent", "b", "Compare fixture labels"),
        ReceiptStatus::Failed,
    );
    derived.consulted = vec![ancestor_receipt.projection];
    let receipt = admit(&mut f, derived.clone());
    let path = f.dir.join("capture.memory-prototype.sqlite");
    let mut live = Memory::open(&path)
        .unwrap()
        .with_source_guard(f.guard.clone());
    let snapshot = live.snapshot().unwrap();
    let concept_sources = [
        ancestor_receipt.originals[0].clone(),
        receipt.originals[0].clone(),
    ];
    live.prepare_native_concept(&concept_sources, &snapshot, &f.access)
        .unwrap();
    let context = interpretation::InterpretationContext::new(
        derived.criterion.clone().unwrap(),
        "fixture-context".into(),
    )
    .unwrap();
    let prepared = live
        .prepare_interpretation(&receipt, &[], &context, &snapshot, &f.access)
        .unwrap();
    assert!(prepared
        .sources()
        .iter()
        .any(|s| s.kind == interpretation::SourceKind::ActionInvocation));
    assert!(prepared
        .sources()
        .iter()
        .any(|s| s.text == "Retain the violet fixture"));
    for reference in &receipt.originals {
        assert!(live.read(reference, &f.access).unwrap().is_some());
    }
    let without_guard = Memory::open(&path).unwrap();
    assert!(without_guard
        .capture_semantic_assessment(&receipt, &f.access)
        .is_err());

    f.guard
        .0
        .lock()
        .unwrap()
        .insert(ancestor.binding.locator.clone());
    for reference in &receipt.originals {
        assert!(live.read(reference, &f.access).unwrap().is_none());
    }
    assert!(live
        .capture_semantic_assessment(&receipt, &f.access)
        .is_err());
    assert!(live
        .prepare_native_concept(&concept_sources, &snapshot, &f.access)
        .is_err());
    assert!(live
        .prepare_interpretation(&receipt, &[], &context, &snapshot, &f.access)
        .is_err());
    assert!(live
        .admit_ordinary_observation(
            OrdinaryObservationCapture::try_from_host(derived.clone()).unwrap(),
            &f.access
        )
        .is_err());

    f.guard.0.lock().unwrap().clear();
    assert!(live.forget(&ancestor.originals[0].source).unwrap().objects >= 5);
    assert!(live
        .capture_semantic_assessment(&receipt, &f.access)
        .is_err());
    for reference in &receipt.originals {
        assert!(live.read(reference, &f.access).unwrap().is_none());
    }
    assert!(live
        .admit_ordinary_observation(
            OrdinaryObservationCapture::try_from_host(derived).unwrap(),
            &f.access
        )
        .is_err());
}

#[test]
fn canonical_ordinary_projection_authenticates_payload_then_checks_protocol() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let mut input = capture("ordinary", "a", "Explain the fixture");
    let receipt = admit(&mut f, input.clone());
    let Some(Record::Evidence(mut projection)) = f.db.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("missing ordinary projection");
    };
    let mut value: serde_json::Value = serde_json::from_str(&projection.text).unwrap();
    value["artifact"]["verdict"] = json!("Passed");
    projection.text = value.to_string();
    f.db.conn
        .execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&Record::Evidence(projection.clone())).unwrap(),
                receipt.projection.revision
            ],
        )
        .unwrap();
    let error = projection_input(&f.db.conn, &projection).unwrap_err();
    assert_eq!(error.to_string(), "host projection payload mismatch");

    input.artifact.verdict = CheckVerdict::Passed;
    let corrupt_hash = digest_bytes(&serde_json::to_vec(&input).unwrap());
    f.db.conn
        .execute(
            "UPDATE host_capture_receipt SET payload_hash=?1 WHERE receipt_id=?2",
            rusqlite::params![corrupt_hash, receipt.id],
        )
        .unwrap();
    let error = projection_input(&f.db.conn, &projection).unwrap_err();
    assert_eq!(
        error.to_string(),
        "ordinary observation cannot supply an artifact check"
    );
}

fn chat_capture() -> HostCapture {
    use super::chat_observation::*;
    let mut value = capture("chat-fixture", "a", "Sort synthetic cards");
    value.identity.check_version = CHAT_OBSERVATION_PROTOCOL.into();
    value.identity.evaluator_version = CHAT_RECORDER_VERSION.into();
    value.criterion = None;
    value.chat_context = Some(ChatObservationContext {
        conversation_id: "chat-fixture".into(),
        turn_ordinal: 1,
        previous_turn: None,
        prefix_start: 0,
        prefix_length: 4096,
        prefix_digest: value.identity.prefix_digest.clone(),
        owner_messages: vec![ChatOwnerMessageLocator {
            line: 1,
            pointer: "/content/0/text".into(),
            context_revision: 1,
            owner_input_index: 0,
        }],
        versions: vec![serde_json::from_value(json!({
            "revision": 1,
            "predecessor": null,
            "context_digest": "c".repeat(64),
            "rendered_charter": null,
            "owner_inputs": [{"source": "cli_chat", "text": "Sort synthetic cards"}],
        }))
        .unwrap()],
        decisions: vec![ChatDecisionLocator {
            line: 2,
            ordinal: 0,
            context_revision: 1,
            binding_digest: "d".repeat(64),
        }],
    });
    value
}

#[test]
fn chat_context_requires_exact_protocol_and_exclusive_context() {
    let valid = chat_capture();
    OrdinaryObservationCapture::try_from_host(valid.clone()).unwrap();
    for modify in [
        |v: &mut HostCapture| v.chat_context = None,
        |v: &mut HostCapture| v.criterion = capture("old", "a", "fixture").criterion,
        |v: &mut HostCapture| v.identity.check_version = ORDINARY_PROTOCOL.into(),
        |v: &mut HostCapture| v.identity.evaluator_version = ORDINARY_RECORDER.into(),
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().conversation_id = "other".into(),
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().prefix_digest = "b".repeat(64),
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().turn_ordinal = 0,
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().prefix_length = 0,
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().decisions.clear(),
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().decisions[0].context_revision = 99,
        |v: &mut HostCapture| {
            v.chat_context.as_mut().unwrap().owner_messages[0].owner_input_index = 99
        },
    ] {
        let mut invalid = valid.clone();
        modify(&mut invalid);
        assert!(OrdinaryObservationCapture::try_from_host(invalid).is_err());
    }
    let mut legacy = capture("old", "a", "fixture");
    legacy.chat_context = valid.chat_context;
    assert!(OrdinaryObservationCapture::try_from_host(legacy).is_err());
}

#[test]
fn chat_context_is_not_evaluated_and_survives_canonical_reader() {
    let mut f = Fixture::new(Memory::create_supported_situated);
    let value = chat_capture();
    let expected = serde_json::to_value(&value.chat_context).unwrap();
    let receipt = admit(&mut f, value.clone());
    assert!(matches!(
        f.db.capture_semantic_assessment(&receipt, &f.access)
            .unwrap(),
        SemanticAssessment::NotEvaluated
    ));
    let Some(Record::Evidence(projection)) = f.db.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("projection missing")
    };
    let canonical = projection_input(&f.db.conn, &projection).unwrap();
    assert_eq!(
        serde_json::to_vec(&canonical).unwrap(),
        serde_json::to_vec(&value).unwrap()
    );
    let reader = native_reader(&f.db.conn, &receipt.projection, &projection)
        .unwrap()
        .unwrap();
    let bytes = serde_json::to_vec(&reader).unwrap();
    let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["chat_context"], expected);
    assert!(wire.get("criterion").is_none());
    let decoded: super::interpretation_reader::NativeCaptureReaderV1 =
        serde_json::from_slice(&bytes).unwrap();
    decoded.validate_manifest(&receipt.originals).unwrap();
    let criterion = capture("target", "a", "Count synthetic cards")
        .criterion
        .unwrap();
    let context =
        interpretation::InterpretationContext::new(criterion, "explicit-target".into()).unwrap();
    let snapshot = f.db.snapshot().unwrap();
    let prepared =
        f.db.prepare_interpretation(&receipt, &[], &context, &snapshot, &f.access)
            .unwrap();
    assert!(prepared
        .sources()
        .iter()
        .any(|s| s.field == "/chat_context"));
    assert!(!prepared.sources().iter().any(|s| s.field == "/criterion"));
    assert!(prepared
        .evidence_metadata()
        .iter()
        .any(|m| m.role == interpretation::EvidenceRole::HistoricalChatContext));
}

#[test]
fn chat_context_raw_snapshot_fields_refuse_decoding() {
    let value = chat_capture();
    for field in ["source", "charter", "sensor", "messages"] {
        let mut wire = serde_json::to_value(&value).unwrap();
        wire["chat_context"]["versions"][0][field] = json!("SYNTHETIC-PRIVATE-SENSOR");
        assert!(serde_json::from_value::<HostCapture>(wire).is_err());
    }
}

#[test]
fn legacy_host_payload_bytes_and_digest_remain_unchanged() {
    let value = capture("legacy-golden", "a", "Sort fixture rows");
    let bytes = serde_json::to_vec(&value).unwrap();
    const GOLDEN: &str = r#"{"identity":{"domain":"test","account":"test","session_id":"legacy-golden","prefix_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","check_id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","check_version":"ordinary-session-observation-v1","evaluator_version":"ordinary-session-recorder-v1"},"binding":{"authority":"test-host","locator":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revision":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"},"criterion":{"goal":"charter:1","goal_text":"Sort fixture records","criterion_text":"Sort fixture rows","criterion_revision":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","confirmation_digest":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"},"originals":[{"source":{"connector":"test-host","account":"test","id":"legacy-golden:/content/0/text"},"kind":"UserRequest","line":1,"pointer":"/content/0/text","text":"Sort fixture rows","occurred_at":null}],"receipts":[],"artifact":{"artifact_digest":null,"verdict":"Unavailable"},"consulted":[],"scopes":[],"occurred_at":null}"#;
    assert_eq!(bytes, GOLDEN.as_bytes());
    assert_eq!(
        digest_bytes(&bytes),
        "0f7c89bac7ef862baba5f87e7231b332fb4dd854691c25d628ab1d7a40ae8baa"
    );
    let decoded: HostCapture = serde_json::from_slice(&bytes).unwrap();
    assert!(decoded.criterion.is_some());
    assert!(decoded.chat_context.is_none());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn chat_version_summary_keeps_owner_order_but_excludes_raw_sensor_configuration() {
    use super::chat_observation::ChatContextVersion;
    use crate::goal_context::{CharterSnapshot, GoalContextState, OwnerInput, OwnerSource};
    let source = "[[line]]\nid='fixture'\ntext='Preserve synthetic order'\n[line.sensor]\nkind='outbox_waiting'\nsetpoint=137\n";
    let charter = CharterSnapshot::from_source(Some(source.into()), true).unwrap();
    let mut state = GoalContextState::default();
    for text in [
        "First synthetic instruction",
        "Second synthetic instruction",
    ] {
        state
            .transition(
                charter.clone(),
                Some(OwnerInput::new(OwnerSource::CliChat, text.into()).unwrap()),
            )
            .unwrap();
    }
    let summary = ChatContextVersion::from_version(state.current().unwrap()).unwrap();
    let text = serde_json::to_string(&summary).unwrap();
    assert!(text.contains("Preserve synthetic order"));
    assert!(
        text.find("First synthetic instruction").unwrap()
            < text.find("Second synthetic instruction").unwrap()
    );
    for secret in ["outbox_waiting", "setpoint", "[[line]]"] {
        assert!(!text.contains(secret));
    }
}

#[test]
fn chat_context_bounds_and_locator_integrity_fail_closed() {
    let valid = chat_capture();
    for modify in [
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().versions.clear(),
        |v: &mut HostCapture| {
            let c = v.chat_context.as_mut().unwrap();
            c.versions.push(c.versions[0].clone());
        },
        |v: &mut HostCapture| {
            let c = v.chat_context.as_mut().unwrap();
            c.decisions.push(c.decisions[0].clone());
        },
        |v: &mut HostCapture| {
            let c = v.chat_context.as_mut().unwrap();
            c.owner_messages.push(c.owner_messages[0].clone());
        },
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().previous_turn = Some("b".repeat(64)),
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().turn_ordinal = 2,
        |v: &mut HostCapture| {
            v.chat_context.as_mut().unwrap().decisions[0].binding_digest = "bad".into()
        },
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().decisions[0].line = 0,
        |v: &mut HostCapture| {
            v.chat_context.as_mut().unwrap().owner_messages[0].pointer =
                "/not-an-owner-message".into()
        },
        |v: &mut HostCapture| v.chat_context.as_mut().unwrap().owner_messages[0].line = 999999,
        |v: &mut HostCapture| v.originals[0].text = "Different owner text".into(),
    ] {
        let mut invalid = valid.clone();
        modify(&mut invalid);
        assert!(OrdinaryObservationCapture::try_from_host(invalid).is_err());
    }
    let mut legacy = capture("old", "a", "fixture");
    legacy.criterion = None;
    assert!(OrdinaryObservationCapture::try_from_host(legacy).is_err());
}

#[test]
fn chat_owner_dto_rejects_unsupported_sources_and_empty_evidence() {
    let mut invalid = serde_json::to_value(chat_capture()).unwrap();
    invalid["chat_context"]["versions"][0]["owner_inputs"][0]["source"] = json!("tui");
    assert!(serde_json::from_value::<HostCapture>(invalid).is_err());
    let mut invalid = serde_json::to_value(chat_capture()).unwrap();
    invalid["chat_context"]["versions"][0]["owner_inputs"][0]["text"] = json!(" ");
    let invalid: HostCapture = serde_json::from_value(invalid).unwrap();
    assert!(OrdinaryObservationCapture::try_from_host(invalid).is_err());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn chat_native_conversion_preserves_owner_and_context_wire_bytes() {
    use super::chat_observation::{ChatContextVersion, ChatDecisionLocator};
    use crate::goal_context::{
        CharterSnapshot, DecisionBinding, GoalContextState, OwnerInput, OwnerSource,
    };
    #[derive(serde::Serialize)]
    struct PreviousSummary<'a> {
        revision: u64,
        predecessor: Option<u64>,
        context_digest: String,
        rendered_charter: Option<&'a str>,
        owner_inputs: &'a [OwnerInput],
    }
    let mut state = GoalContextState::default();
    let version = state
        .transition(
            CharterSnapshot::disabled(),
            Some(OwnerInput::new(OwnerSource::CliChat, "Sort synthetic cards".into()).unwrap()),
        )
        .unwrap()
        .unwrap();
    let summary = ChatContextVersion::from_version(&version).unwrap();
    let previous = PreviousSummary {
        revision: version.revision(),
        predecessor: version.predecessor(),
        context_digest: digest_bytes(&serde_json::to_vec(&version).unwrap()),
        rendered_charter: version.charter().prompt_block(),
        owner_inputs: version.owner_inputs(),
    };
    assert_eq!(
        serde_json::to_vec(&summary).unwrap(),
        serde_json::to_vec(&previous).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(summary.owner_inputs()).unwrap(),
        serde_json::to_vec(version.owner_inputs()).unwrap()
    );
    let binding = DecisionBinding::new(0, version.revision(), &[], None, true, 0).unwrap();
    let locator = ChatDecisionLocator::from_binding(2, &binding).unwrap();
    assert_eq!(
        locator.binding_digest,
        digest_bytes(&serde_json::to_vec(&binding).unwrap())
    );
    assert_eq!(
        (locator.line, locator.ordinal, locator.context_revision),
        (2, 0, version.revision())
    );
}

fn authored_web_observation_wire() -> serde_json::Value {
    let mut wire = serde_json::to_value(chat_capture()).unwrap();
    wire["identity"]["check_version"] = json!("ordinary-chat-observation-v2");
    wire["identity"]["evaluator_version"] = json!("ordinary-chat-recorder-v2");
    wire["chat_context"]["versions"][0]["owner_inputs"][0]["source"] = json!("authenticated_web");
    wire
}

#[test]
fn web_observation_v2_retains_authenticated_source_without_a_success_verdict() {
    let value: HostCapture = serde_json::from_value(authored_web_observation_wire())
        .expect("the versioned web DTO must decode authenticated owner evidence");
    OrdinaryObservationCapture::try_from_host(value.clone()).unwrap();
    let mut f = Fixture::new(Memory::create_supported_situated);
    let receipt = admit(&mut f, value.clone());
    assert!(matches!(
        f.db.capture_semantic_assessment(&receipt, &f.access)
            .unwrap(),
        SemanticAssessment::NotEvaluated
    ));
    let Some(Record::Evidence(projection)) = f.db.read(&receipt.projection, &f.access).unwrap()
    else {
        panic!("web observation projection missing");
    };
    let canonical = projection_input(&f.db.conn, &projection).unwrap();
    assert_eq!(
        serde_json::to_vec(&canonical).unwrap(),
        serde_json::to_vec(&value).unwrap()
    );
    let reader = native_reader(&f.db.conn, &receipt.projection, &projection)
        .unwrap()
        .unwrap();
    let wire = serde_json::to_value(&reader).unwrap();
    assert_eq!(
        wire["chat_context"]["versions"][0]["owner_inputs"][0]["source"],
        "authenticated_web"
    );
    assert!(wire.get("criterion").is_none());
    let decoded: super::interpretation_reader::NativeCaptureReaderV1 =
        serde_json::from_value(wire).unwrap();
    decoded.validate_manifest(&receipt.originals).unwrap();
}

#[test]
fn web_observation_v2_reserves_each_protocol_label_against_legacy_admission() {
    for (check, recorder) in [
        (
            "ordinary-chat-observation-v2",
            "authored-unrelated-recorder",
        ),
        ("authored-unrelated-check", "ordinary-chat-recorder-v2"),
    ] {
        let mut value = capture("web-protocol-spoof", "a", "Sort synthetic rows");
        value.identity.check_version = check.into();
        value.identity.evaluator_version = recorder.into();
        assert!(
            super::ordinary_observation::claims_ordinary_protocol(&value.identity),
            "either reserved web label must prevent legacy artifact admission"
        );
        assert!(OrdinaryObservationCapture::try_from_host(value).is_err());
    }
}

#[test]
fn web_observation_v2_rejects_crossed_protocol_source_pairs() {
    for (check, recorder, source) in [
        (
            "ordinary-chat-observation-v1",
            "ordinary-chat-recorder-v1",
            "authenticated_web",
        ),
        (
            "ordinary-chat-observation-v2",
            "ordinary-chat-recorder-v2",
            "cli_chat",
        ),
        (
            "ordinary-chat-observation-v1",
            "ordinary-chat-recorder-v2",
            "authenticated_web",
        ),
        (
            "ordinary-chat-observation-v2",
            "ordinary-chat-recorder-v1",
            "cli_chat",
        ),
        (
            "ordinary-chat-observation-v2",
            "ordinary-chat-recorder-v2",
            "tui",
        ),
        (
            "ordinary-chat-observation-v2",
            "ordinary-chat-recorder-v2",
            "unverified",
        ),
    ] {
        let mut wire = authored_web_observation_wire();
        wire["identity"]["check_version"] = json!(check);
        wire["identity"]["evaluator_version"] = json!(recorder);
        wire["chat_context"]["versions"][0]["owner_inputs"][0]["source"] = json!(source);
        let accepted = serde_json::from_value::<HostCapture>(wire)
            .ok()
            .is_some_and(|v| OrdinaryObservationCapture::try_from_host(v).is_ok());
        assert!(!accepted, "accepted {check}/{recorder}/{source}");
    }
    // The old CLI pair remains valid: rejecting every context is not a fix.
    OrdinaryObservationCapture::try_from_host(chat_capture()).unwrap();
}

#[test]
fn web_observation_v2_refuses_mixed_owner_history_and_private_fields() {
    let mut mixed = authored_web_observation_wire();
    mixed["chat_context"]["versions"][0]["owner_inputs"] = json!([
        {"source":"cli_chat","text":"Earlier synthetic instruction"},
        {"source":"authenticated_web","text":"Sort synthetic cards"}
    ]);
    mixed["chat_context"]["owner_messages"][0]["owner_input_index"] = json!(1);
    let accepted = serde_json::from_value::<HostCapture>(mixed)
        .ok()
        .is_some_and(|v| OrdinaryObservationCapture::try_from_host(v).is_ok());
    assert!(
        !accepted,
        "web capture must reject mixed historical ingress"
    );
    for field in ["source", "charter", "sensor", "messages"] {
        let mut wire = authored_web_observation_wire();
        wire["chat_context"]["versions"][0][field] = json!("AUTHORED-PRIVATE-FIELD");
        assert!(serde_json::from_value::<HostCapture>(wire).is_err());
    }
}

#[cfg(feature = "memory-prototype")]
#[test]
fn cli_chat_converter_does_not_accept_authenticated_web_owner_input() {
    use super::chat_observation::ChatContextVersion;
    use crate::goal_context::{CharterSnapshot, GoalContextState, OwnerInput, OwnerSource};
    let mut state = GoalContextState::default();
    state
        .transition(
            CharterSnapshot::disabled(),
            Some(
                OwnerInput::new(OwnerSource::AuthenticatedWeb, "Sort synthetic cards".into())
                    .unwrap(),
            ),
        )
        .unwrap();
    assert!(ChatContextVersion::from_version(state.current().unwrap()).is_err());
}

#[cfg(feature = "memory-prototype")]
#[test]
fn web_observation_native_converter_preserves_exact_owner_order_and_rejects_cli_history() {
    use super::chat_observation::ChatContextVersion;
    use crate::goal_context::{CharterSnapshot, GoalContextState, OwnerInput, OwnerSource};
    let mut state = GoalContextState::default();
    for text in [
        "First authored web objective",
        "Revised authored web objective",
    ] {
        state
            .transition(
                CharterSnapshot::disabled(),
                Some(OwnerInput::new(OwnerSource::AuthenticatedWeb, text.into()).unwrap()),
            )
            .unwrap();
    }
    let version = state.current().unwrap();
    let summary = ChatContextVersion::from_authenticated_web_version(version).unwrap();
    assert_eq!(
        serde_json::to_vec(summary.owner_inputs()).unwrap(),
        serde_json::to_vec(version.owner_inputs()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&summary).unwrap()["context_digest"],
        digest_bytes(&serde_json::to_vec(version).unwrap())
    );
    state
        .transition(
            CharterSnapshot::disabled(),
            Some(OwnerInput::new(OwnerSource::CliChat, "A different ingress".into()).unwrap()),
        )
        .unwrap();
    assert!(ChatContextVersion::from_authenticated_web_version(state.current().unwrap()).is_err());
}

#[test]
fn task_schema_preserves_ordinary_projection_and_recall() {
    for version in [8, 9, 10] {
        let mut f = Fixture::new(Memory::create_supported_situated);
        if version >= 9 {
            f.db.conn
                .execute_batch(include_str!("task_schema.sql"))
                .unwrap();
        }
        if version == 10 {
            f.db.conn
                .execute_batch(include_str!("plan_schema.sql"))
                .unwrap();
        }
        let receipt = admit(
            &mut f,
            capture("ordinary-task-schema", "a", "Sort synthetic amber records"),
        );
        let record = f.db.read(&receipt.projection, &f.access).unwrap().unwrap();
        let Record::Evidence(value) = record else {
            panic!("expected evidence")
        };
        assert_eq!(value.source.connector, "host-capture-projection-v2");
        let source = f.db.reopen_read_only().unwrap();
        assert!(source
            .read(&receipt.projection, &f.access)
            .unwrap()
            .is_some());
    }
}
