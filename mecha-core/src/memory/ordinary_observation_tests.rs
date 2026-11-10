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
        criterion: ConfirmedCriterion {
            goal: "charter:1".into(),
            goal_text: "Sort fixture records".into(),
            criterion_text: prompt.into(),
            criterion_revision: "e".repeat(64),
            confirmation_digest: "f".repeat(64),
        },
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
        derived.criterion.clone(),
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
