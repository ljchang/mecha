use super::{context::*, host_capture::*, interpretation::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Guard(Arc<AtomicBool>);
impl ExternalSourceGuard for Guard {
    fn check(&self, _: &ExternalBinding) -> SourceStatus {
        if self.0.load(Ordering::SeqCst) {
            SourceStatus::Checked
        } else {
            SourceStatus::Withheld
        }
    }
}
fn criterion() -> ConfirmedCriterion {
    ConfirmedCriterion {
        goal: "charter:1".into(),
        goal_text: "convert CSV".into(),
        criterion_text: "write a nonempty JSON array containing every required row".into(),
        criterion_revision: "e".repeat(64),
        confirmation_digest: "f".repeat(64),
    }
}
fn input(id: &str) -> HostCapture {
    HostCapture {
        identity: CaptureIdentity {
            domain: "test".into(),
            account: "test".into(),
            session_id: id.into(),
            prefix_digest: "a".repeat(64),
            check_id: "b".repeat(64),
            check_version: "v1".into(),
            evaluator_version: "v1".into(),
        },
        binding: ExternalBinding {
            authority: "host-v1".into(),
            locator: "c".repeat(64),
            revision: "d".repeat(64),
        },
        criterion: criterion(),
        originals: vec![OriginalPassage {
            source: SourceKey {
                connector: "bridge".into(),
                account: "test".into(),
                id: id.into(),
            },
            kind: PassageKind::AssistantText,
            line: 1,
            pointer: "/tool/call1".into(),
            text: "convert synthetic CSV to JSON".into(),
            occurred_at: None,
        }],
        receipts: vec![ToolReceipt {
            call_id: "call1".into(),
            tool_name: "fs_write".into(),
            status: ReceiptStatus::Succeeded,
        }],
        artifact: ArtifactCheck {
            artifact_digest: None,
            verdict: CheckVerdict::Failed,
        },
        consulted: vec![],
        scopes: BTreeSet::new(),
        occurred_at: None,
    }
}
pub(super) fn access() -> Access {
    Access {
        domain: "test".into(),
        scopes: BTreeSet::new(),
    }
}
fn fixture() -> (std::path::PathBuf, Memory, Arc<AtomicBool>) {
    let dir = std::env::temp_dir().join(format!("interpretation-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("x.memory-prototype.sqlite");
    let state = Arc::new(AtomicBool::new(true));
    let db = Memory::create_situated(&path, "test")
        .unwrap()
        .with_source_guard(Arc::new(Guard(state.clone())));
    (path, db, state)
}
fn context() -> InterpretationContext {
    InterpretationContext::new(criterion(), "opaque-host-context".into()).unwrap()
}
fn prepare(db: &Memory, r: &CaptureReceipt) -> PreparedInterpretation {
    db.prepare_interpretation(r, &[], &context(), &db.snapshot().unwrap(), &access())
        .unwrap()
}
fn draft(p: &PreparedInterpretation) -> InterpretationDraft {
    let s = p
        .sources()
        .iter()
        .find(|s| s.text.contains("convert synthetic"))
        .unwrap();
    InterpretationDraft {
        interpretation: "Conversion remains an unverified hypothesis".into(),
        claims: vec![InterpretationClaim {
            source_id: s.id.clone(),
            statement: "The source describes conversion".into(),
            quote: "convert synthetic CSV to JSON".into(),
        }],
        judgments: vec![InterpretationJudgment {
            goal: criterion().goal,
            bearing: InterpretationBearing::Unknown,
            because: vec![0],
        }],
        prediction: None,
        grounding: GroundingReport {
            offered: 1,
            dropped: 0,
            dropped_by: BTreeMap::new(),
            unresolved_goals: 0,
        },
        model: "authored".into(),
        producer_version: "v1".into(),
        support: None,
    }
}
fn read(db: &Memory, c: &InterpretationContext) -> ContextPacket {
    db.context_with_interpretations(
        &ContextRequest {
            query: "conversion".into(),
            bindings: vec![],
        },
        &ContextPolicy {
            access: access(),
            recall: RecallQuery {
                text: "conversion".into(),
                known_at: db.snapshot().unwrap(),
                account: None,
                session_id: None,
                occurred_after: None,
                occurred_before: None,
                limit: 8,
                neighbors: 0,
                max_bytes: 20_000,
            },
            valid_at: None,
        },
        &Situation {
            event: "conversion".into(),
            goal: c.criterion().goal.clone(),
            needs: None,
        },
        &c.read_policy(),
    )
    .unwrap()
}
pub(super) fn supported_fixture() -> (std::path::PathBuf, Memory, Arc<AtomicBool>) {
    let dir = std::env::temp_dir().join(format!("support-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("x.memory-prototype.sqlite");
    let state = Arc::new(AtomicBool::new(true));
    let db = Memory::create_supported_situated(&path, "test")
        .unwrap()
        .with_source_guard(Arc::new(Guard(state.clone())));
    (path, db, state)
}
fn rich_input(id: &str) -> HostCapture {
    use sha2::{Digest, Sha256};
    let mut i = input(id);
    i.identity.check_version = EVIDENCE_CHECK_VERSION.into();
    let action_input = serde_json::json!({"content":"requested different rows","path":"out.json"});
    let input_digest = Sha256::digest(serde_json::to_vec(&action_input).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    i.originals.push(OriginalPassage {
        source: SourceKey {
            connector: "host-action-invocation-v1".into(),
            account: "test".into(),
            id: format!("{id}-action"),
        },
        kind: PassageKind::ActionInvocation,
        line: 1,
        pointer: "/content/0/input".into(),
        text: serde_json::to_string(&ActionInvocation {
            call_id: "call1".into(),
            tool_name: "fs_write".into(),
            input_status: InputStatus::Complete,
            input: action_input,
            input_digest,
        })
        .unwrap(),
        occurred_at: None,
    });
    let digest = Sha256::digest(b"[]")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    i.artifact.artifact_digest = Some(digest.clone());
    i.originals.push(OriginalPassage {
        source: SourceKey {
            connector: "host-artifact-observation-v1".into(),
            account: "test".into(),
            id: format!("{id}-artifact"),
        },
        kind: PassageKind::ArtifactObservation,
        line: 1,
        pointer: "/artifact_observation".into(),
        text: serde_json::to_string(&ArtifactObservation {
            content_status: ContentStatus::Complete,
            artifact_text: Some("[]".into()),
            artifact_digest: Some(digest),
            verdict: CheckVerdict::Failed,
        })
        .unwrap(),
        occurred_at: None,
    });
    i
}
pub(super) fn prepared_supported(db: &mut Memory) -> PreparedInterpretation {
    let r = db
        .admit_host_capture_with_evidence(rich_input("rich"), &access())
        .unwrap();
    prepare(db, &r)
}
fn citation(p: &PreparedInterpretation) -> SourceCitation {
    let s = p
        .sources()
        .iter()
        .find(|s| s.kind == SourceKind::ArtifactObservation)
        .unwrap();
    SourceCitation {
        source_id: s.id.clone(),
        quote: "\"artifact_text\":\"[]\"".into(),
    }
}
pub(super) fn proposal(p: &PreparedInterpretation) -> SupportProposal {
    SupportProposal {
        units: vec![ProposedUnit {
            id: "u1".into(),
            kind: UnitKind::RecordedEvent,
            text: "The observed artifact is an empty array".into(),
            citations: vec![citation(p)],
        }],
        judgments: vec![ProposedJudgment {
            goal: criterion().goal,
            bearing: InterpretationBearing::Good,
            because: vec!["u1".into()],
        }],
    }
}
fn check(target: &str, status: SupportStatus, p: &PreparedInterpretation) -> SupportCheck {
    SupportCheck {
        target: target.into(),
        status,
        unsupported_factual_premise: false,
        citations: if status == SupportStatus::Supported {
            vec![citation(p)]
        } else {
            vec![]
        },
        reason: "AUDIT_REASON_CANARY".into(),
        questions: if status == SupportStatus::Unknown {
            vec!["AUDIT_QUESTION_CANARY".into()]
        } else {
            vec![]
        },
    }
}
pub(super) fn reply(
    p: &PreparedInterpretation,
    unit: SupportStatus,
    judgment: SupportStatus,
) -> SupportReply {
    SupportReply {
        checks: vec![check("unit:u1", unit, p), check("judgment:0", judgment, p)],
    }
}
fn evaluate(
    db: &Memory,
    p: &PreparedInterpretation,
    proposal: SupportProposal,
    reply: SupportReply,
) -> EvaluatedInterpretation {
    finish_support(
        db.prepare_support(p, proposal, &access()).unwrap(),
        reply,
        "evaluator",
    )
    .unwrap()
}
#[test]
fn evaluated_regression_false_claim_with_valid_quote_is_not_retained() {
    let (_, mut legacy, _) = fixture();
    let r = legacy
        .admit_host_capture(input("false"), &access())
        .unwrap();
    let p = prepare(&legacy, &r);
    let mut d = draft(&p);
    d.interpretation = "FALSE_HISTORICAL_CANARY selected a route".into();
    d.claims[0].statement = d.interpretation.clone();
    // Characterize the historical hole, preserving v7 behavior.
    assert!(legacy.retain_interpretation(p, d, &access()).is_ok());
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.units[0].text = "FALSE_HISTORICAL_CANARY selected a route".into();
    let evaluated = evaluate(
        &db,
        &p,
        offered,
        reply(&p, SupportStatus::Contradicted, SupportStatus::Unknown),
    );
    let raw = serde_json::to_string(evaluated.reply()).unwrap();
    assert!(raw.contains("AUDIT_REASON_CANARY"));
    let saved = db
        .retain_evaluated_interpretation(p, evaluated, &access())
        .unwrap();
    assert!(db.read(&saved.reference, &access()).unwrap().is_none());
    let packet = read(&db, &context());
    assert!(packet
        .records
        .iter()
        .any(|h| h.reference == saved.reference));
    let serialized = serde_json::to_string(&packet).unwrap();
    for forbidden in [
        "FALSE_HISTORICAL_CANARY",
        "AUDIT_REASON_CANARY",
        "AUDIT_QUESTION_CANARY",
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
    assert!(serialized.contains("Unknown"));
}
#[test]
fn evaluated_regression_true_observation_does_not_certify_good_bearing() {
    let (path, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let evaluated = evaluate(
        &db,
        &p,
        proposal(&p),
        reply(&p, SupportStatus::Supported, SupportStatus::Contradicted),
    );
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Unknown
    );
    assert!(evaluated.draft().interpretation.contains("empty array"));
    let receipt = db
        .retain_evaluated_interpretation(p, evaluated, &access())
        .unwrap();
    let reopened = Memory::open(&path)
        .unwrap()
        .with_source_guard(Arc::new(Guard(Arc::new(AtomicBool::new(true)))));
    assert!(reopened
        .read(&receipt.reference, &access())
        .unwrap()
        .is_none());
    assert_eq!(read(&reopened, &context()).interpretation_roles.len(), 1);
}
#[test]
fn every_unit_and_judgment_requires_exactly_one_check() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    for mutation in 0..4 {
        let mut r = reply(&p, SupportStatus::Supported, SupportStatus::Supported);
        match mutation {
            0 => {
                r.checks.pop();
            }
            1 => r.checks.push(r.checks[0].clone()),
            2 => r.checks[1].target = "judgment:1".into(),
            _ => r.checks[0].target = "unit:invented".into(),
        }
        assert!(finish_support(
            db.prepare_support(&p, proposal(&p), &access()).unwrap(),
            r,
            "eval"
        )
        .is_err());
    }
}
#[test]
fn strict_json_rejects_unknown_fields_duplicate_keys_and_unissued_source_ids() {
    for text in [
        r#"{"units":[],"judgments":[],"extra":0}"#,
        r#"{"units":[],"units":[],"judgments":[]}"#,
        r#"{"units":[{"id":"x","kind":"RecordedEvent","text":"x","citations":[],"extra":0}],"judgments":[]}"#,
    ] {
        assert!(parse_support_proposal(text).is_err());
    }
    assert!(parse_support_reply(r#"{"checks":[],"checks":[]}"#).is_err());
    assert!(parse_support_reply(r#"{"checks":[{"target":"unit:a","status":"Supported","status":"Unknown","unsupported_factual_premise":false,"citations":[],"reason":"x","questions":[] }]}"#).is_err());
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    for mutation in 0..6 {
        let mut offered = proposal(&p);
        match mutation {
            0 => offered.units[0].citations[0].source_id = "call1".into(),
            1 => offered.units.push(offered.units[0].clone()),
            2 => offered.units[0].id = "bad:id".into(),
            3 => offered.units[0].text = "é".repeat(257),
            4 => offered.judgments[0].goal = "foreign".into(),
            _ => offered.judgments[0].because = vec!["missing".into()],
        }
        assert!(db.prepare_support(&p, offered, &access()).is_err());
    }
}
#[test]
fn supported_checks_need_citations_unknown_checks_need_questions() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    for mutation in 0..6 {
        let mut r = reply(&p, SupportStatus::Supported, SupportStatus::Unknown);
        match mutation {
            0 => r.checks[0].citations.clear(),
            1 => r.checks[0].questions.push("unresolved".into()),
            2 => r.checks[1].questions.clear(),
            3 => r.checks[0].reason = "x".repeat(513),
            4 => r.checks[1].questions = vec!["x".repeat(257)],
            _ => r.checks[0].citations[0].quote = "not present in source at all".into(),
        }
        assert!(finish_support(
            db.prepare_support(&p, proposal(&p), &access()).unwrap(),
            r,
            "eval"
        )
        .is_err());
    }
}
#[test]
fn speculative_units_cannot_support_good_and_past_premise_flags_withhold_text() {
    for kind in [UnitKind::Hypothesis, UnitKind::Prediction] {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let mut offered = proposal(&p);
        offered.units[0].kind = kind;
        let evaluated = evaluate(
            &db,
            &p,
            offered.clone(),
            reply(&p, SupportStatus::Supported, SupportStatus::Supported),
        );
        assert_eq!(
            evaluated.draft().judgments[0].bearing,
            InterpretationBearing::Unknown
        );
        assert!(evaluated.draft().interpretation.contains("Speculative"));
        offered.units[0].text =
            "REJECTED_PAST_PREMISE_CANARY might explain the earlier success".into();
        let mut r = reply(&p, SupportStatus::Unknown, SupportStatus::Supported);
        r.checks[0].unsupported_factual_premise = true;
        let evaluated = evaluate(&db, &p, offered, r);
        assert!(!serde_json::to_string(evaluated.draft())
            .unwrap()
            .contains("REJECTED_PAST_PREMISE_CANARY"));
    }
}
#[test]
fn unknown_without_evidence_and_every_offered_judgment_are_preserved() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let offered = SupportProposal {
        units: vec![],
        judgments: vec![
            ProposedJudgment {
                goal: criterion().goal,
                bearing: InterpretationBearing::Unknown,
                because: vec![]
            };
            2
        ],
    };
    let r = SupportReply {
        checks: vec![
            check("judgment:0", SupportStatus::Unknown, &p),
            check("judgment:1", SupportStatus::Unknown, &p),
        ],
    };
    let evaluated = evaluate(&db, &p, offered, r);
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Unknown
    );
    assert!(evaluated.draft().judgments[0].because.is_empty());
    let report = evaluated.draft().support.as_ref().unwrap();
    assert_eq!(report.judgments.len(), 2);
    assert_eq!(report.judgments[1].reason, JudgmentReason::DuplicateGoal);
    assert!(db
        .retain_evaluated_interpretation(p, evaluated, &access())
        .is_ok());
}
#[test]
fn partly_removed_support_never_silently_weakens_original_judgment() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    let mut second = offered.units[0].clone();
    second.id = "u2".into();
    offered.units.push(second);
    offered.judgments[0].because.push("u2".into());
    let mut r = reply(&p, SupportStatus::Supported, SupportStatus::Supported);
    r.checks.push(check("unit:u2", SupportStatus::Unknown, &p));
    let evaluated = evaluate(&db, &p, offered, r);
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Unknown
    );
    assert_eq!(
        evaluated.draft().support.as_ref().unwrap().judgments[0].because,
        vec!["u1", "u2"]
    );
}
#[test]
fn sources_scopes_forgetting_and_store_replacement_are_rechecked_between_passes() {
    for mode in 0..4 {
        let (path, mut db, state) = supported_fixture();
        let p = prepared_supported(&mut db);
        let evaluated = evaluate(
            &db,
            &p,
            proposal(&p),
            reply(&p, SupportStatus::Supported, SupportStatus::Supported),
        );
        let mut final_access = access();
        match mode {
            0 => state.store(false, Ordering::SeqCst),
            1 => {
                final_access.scopes.insert("different".into());
            }
            2 => {
                db.forget(&rich_input("rich").originals[0].source).unwrap();
            }
            _ => {
                let moved = path.with_extension("old.sqlite");
                std::fs::rename(&path, moved).unwrap();
                Memory::create_supported_situated(&path, "test").unwrap();
            }
        }
        assert!(
            db.retain_evaluated_interpretation(p, evaluated, &final_access)
                .is_err(),
            "mode {mode}"
        );
    }
}
#[test]
fn unevaluated_public_draft_cannot_write_v8_and_v7_metadata_is_absent() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    assert!(db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .is_err());
    let serialized = serde_json::to_string(&draft(&p)).unwrap();
    assert!(!serialized.contains("support"));
    let old = serde_json::from_str::<InterpretationDraft>(&serialized).unwrap();
    assert!(old.support.is_none());
}
#[test]
fn canonical_reopen_rejects_report_protocol_and_rendering_tampering() {
    for mutation in 0..3 {
        let (path, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let evaluated = evaluate(
            &db,
            &p,
            proposal(&p),
            reply(&p, SupportStatus::Supported, SupportStatus::Unknown),
        );
        let saved = db
            .retain_evaluated_interpretation(p, evaluated, &access())
            .unwrap();
        let sql = rusqlite::Connection::open(&path).unwrap();
        let payload: String = sql
            .query_row(
                "SELECT payload FROM revision WHERE id=?1",
                [&saved.reference.revision],
                |r| r.get(0),
            )
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        match mutation {
            0 => v["data"]["draft"]["support"]["protocol"] = "unknown".into(),
            1 => {
                v["data"]["draft"]
                    .as_object_mut()
                    .unwrap()
                    .remove("support");
            }
            _ => v["data"]["draft"]["interpretation"] = "UNRENDERED_CANARY".into(),
        }
        // Also refresh receipt hash: deterministic correspondence must defend independently.
        use sha2::{Digest, Sha256};
        let typed: Record = serde_json::from_value(v.clone()).unwrap();
        let hash = Sha256::digest(serde_json::to_vec(&typed).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        sql.execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![v.to_string(), saved.reference.revision],
        )
        .unwrap();
        sql.execute(
            "UPDATE interpretation_receipt SET payload_hash=?1 WHERE revision_id=?2",
            rusqlite::params![hash, saved.reference.revision],
        )
        .unwrap();
        assert!(Memory::open(&path).is_err());
    }
}

#[test]
fn supported_bad_bearing_roundtrips_with_source_roles_and_exact_retry() {
    let (path, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.judgments[0].bearing = InterpretationBearing::Bad;
    let assessed = reply(&p, SupportStatus::Supported, SupportStatus::Supported);
    let evaluated = evaluate(&db, &p, offered.clone(), assessed.clone());
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Bad
    );
    let receipt = db
        .retain_evaluated_interpretation(p.clone(), evaluated, &access())
        .unwrap();
    let retry = evaluate(&db, &p, offered, assessed);
    assert_eq!(
        receipt,
        db.retain_evaluated_interpretation(p, retry, &access())
            .unwrap()
    );
    let reopened = Memory::open(&path)
        .unwrap()
        .with_source_guard(Arc::new(Guard(Arc::new(AtomicBool::new(true)))));
    let packet = read(&reopened, &context());
    let Record::Interpretation(i) = &packet
        .records
        .iter()
        .find(|h| h.reference == receipt.reference)
        .unwrap()
        .record
    else {
        panic!("missing interpretation")
    };
    assert_eq!(i.draft.judgments[0].because, vec![0]);
    assert_eq!(i.claim_sources[0].kind, SourceKind::ArtifactObservation);
    let report = i.draft.support.as_ref().unwrap();
    assert_eq!(report.evaluator_model, "evaluator");
    assert_eq!(report.protocol, SUPPORT_PROTOCOL);
}
#[test]
fn evaluation_cannot_cross_context_and_preparation_rechecks_current_authority() {
    let (_, mut db, state) = supported_fixture();
    let receipt = db
        .admit_host_capture_with_evidence(rich_input("rich"), &access())
        .unwrap();
    let p = prepare(&db, &receipt);
    let other = InterpretationContext::new(criterion(), "another-private-context".into()).unwrap();
    let foreign = db
        .prepare_interpretation(&receipt, &[], &other, &db.snapshot().unwrap(), &access())
        .unwrap();
    assert_eq!(p.generation_input(), foreign.generation_input());
    let first_packet = db.prepare_support(&p, proposal(&p), &access()).unwrap();
    let second_packet = db
        .prepare_support(&foreign, proposal(&foreign), &access())
        .unwrap();
    assert_eq!(
        first_packet.generation_input(),
        second_packet.generation_input()
    );
    assert!(!first_packet
        .generation_input()
        .contains("another-private-context"));
    assert!(!first_packet
        .generation_input()
        .contains("opaque-host-context"));
    let evaluated = evaluate(
        &db,
        &p,
        proposal(&p),
        reply(&p, SupportStatus::Supported, SupportStatus::Supported),
    );
    assert!(db
        .retain_evaluated_interpretation(foreign, evaluated, &access())
        .is_err());
    state.store(false, Ordering::SeqCst);
    assert!(db.prepare_support(&p, proposal(&p), &access()).is_err());
}
#[test]
fn speculative_unknown_survives_as_speculation_and_packet_bounds_refuse_whole_offering() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.units[0].kind = UnitKind::Hypothesis;
    offered.units[0].citations.clear();
    let evaluated = evaluate(
        &db,
        &p,
        offered,
        reply(&p, SupportStatus::Unknown, SupportStatus::Unknown),
    );
    assert!(evaluated
        .draft()
        .interpretation
        .starts_with("Speculative hypothesis:"));
    assert!(evaluated.draft().claims.is_empty());
    for mode in 0..4 {
        let mut offered = proposal(&p);
        match mode {
            0 => {
                offered.units = (0..9)
                    .map(|i| {
                        let mut u = offered.units[0].clone();
                        u.id = format!("u{i}");
                        u
                    })
                    .collect()
            }
            1 => offered.judgments = vec![offered.judgments[0].clone(); 9],
            2 => offered.judgments[0].because = vec!["u1".into(); 2],
            _ => offered.units[0].citations.clear(),
        }
        assert!(db.prepare_support(&p, offered, &access()).is_err());
    }
}

#[test]
fn evaluated_ancestors_keep_influenced_roles_and_forgetting_withholds_recaptured_descendants() {
    let (path, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.units[0].kind = UnitKind::Hypothesis;
    let evaluated = evaluate(
        &db,
        &p,
        offered,
        reply(&p, SupportStatus::Unknown, SupportStatus::Unknown),
    );
    let first = db
        .retain_evaluated_interpretation(p, evaluated, &access())
        .unwrap();
    let mut next = rich_input("second");
    next.consulted.push(first.reference.clone());
    next.originals[0].text =
        "Speculative hypothesis: The observed artifact is an empty array".into();
    let second = db
        .admit_host_capture_with_evidence(next, &access())
        .unwrap();
    let p2 = prepare(&db, &second);
    for original in &second.originals {
        assert!(p2
            .sources()
            .iter()
            .filter(|s| s.source == *original)
            .all(|s| s.kind == SourceKind::InterpretationInfluenced));
        assert!(p2.sources().iter().any(|s| s.source == *original));
    }
    let source = p2
        .sources()
        .iter()
        .find(|s| {
            second.originals.contains(&s.source) && s.text.starts_with("Speculative hypothesis:")
        })
        .unwrap();
    let offered = SupportProposal {
        units: vec![ProposedUnit {
            id: "u1".into(),
            kind: UnitKind::Hypothesis,
            text: "The earlier hypothesis remains speculative".into(),
            citations: vec![SourceCitation {
                source_id: source.id.clone(),
                quote: source.text.clone(),
            }],
        }],
        judgments: vec![ProposedJudgment {
            goal: criterion().goal,
            bearing: InterpretationBearing::Unknown,
            because: vec![],
        }],
    };
    let assessed = reply(&p2, SupportStatus::Unknown, SupportStatus::Unknown);
    let evaluated = evaluate(&db, &p2, offered, assessed);
    let descendant = db
        .retain_evaluated_interpretation(p2.clone(), evaluated, &access())
        .unwrap();
    let before = read(&db, &context());
    assert!(before
        .records
        .iter()
        .any(|h| h.reference == descendant.reference));
    db.forget(&rich_input("rich").originals[0].source).unwrap();
    assert!(db
        .prepare_support(
            &p2,
            SupportProposal {
                units: vec![],
                judgments: vec![]
            },
            &access()
        )
        .is_err());
    let reopened = Memory::open(&path)
        .unwrap()
        .with_source_guard(Arc::new(Guard(Arc::new(AtomicBool::new(true)))));
    let after = read(&reopened, &context());
    assert!(!after
        .records
        .iter()
        .any(|h| h.reference == first.reference || h.reference == descendant.reference));
}

#[test]
fn revised_selection_replaces_unknown_and_preserves_audit() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.judgments.insert(
        0,
        ProposedJudgment {
            goal: criterion().goal,
            bearing: InterpretationBearing::Unknown,
            because: vec![],
        },
    );
    let mut assessed = reply(&p, SupportStatus::Supported, SupportStatus::Unknown);
    assessed
        .checks
        .push(check("judgment:1", SupportStatus::Supported, &p));
    let evaluated = finish_support_revised(
        db.prepare_support(&p, offered, &access()).unwrap(),
        assessed,
        "evaluator",
    )
    .unwrap();
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Good
    );
    let report = evaluated.draft().support.as_ref().unwrap();
    assert_eq!(report.protocol, SUPPORT_REVISION_PROTOCOL);
    assert_eq!(report.judgments[0].reason, JudgmentReason::ExplicitUnknown);
    assert_eq!(report.judgments[1].reason, JudgmentReason::Retained);
    assert_eq!(report.judgments[1].unsupported_factual_premise, Some(false));
    assert!(db
        .retain_evaluated_interpretation(p, evaluated, &access())
        .is_ok());
}

#[test]
fn revised_selection_conflicts_ties_and_all_premise_gates() {
    for mode in 0..8 {
        let (_, mut db, _) = supported_fixture();
        let p = prepared_supported(&mut db);
        let mut offered = proposal(&p);
        offered.judgments[0].bearing = InterpretationBearing::Unknown;
        offered.judgments.push(ProposedJudgment {
            goal: criterion().goal,
            bearing: InterpretationBearing::Good,
            because: vec!["u1".into()],
        });
        let mut assessed = reply(&p, SupportStatus::Supported, SupportStatus::Unknown);
        assessed
            .checks
            .push(check("judgment:1", SupportStatus::Supported, &p));
        match mode {
            0 | 1 => {
                let mut j = offered.judgments[1].clone();
                if mode == 0 {
                    j.bearing = InterpretationBearing::Bad;
                }
                offered.judgments.push(j);
                assessed
                    .checks
                    .push(check("judgment:2", SupportStatus::Supported, &p));
            }
            2 => assessed.checks[2].unsupported_factual_premise = true,
            3 => offered.units[0].kind = UnitKind::Hypothesis,
            4 => assessed.checks[0] = check("unit:u1", SupportStatus::Contradicted, &p),
            5 => offered.judgments[1].because.clear(),
            6 => {
                let mut u = offered.units[0].clone();
                u.id = "u2".into();
                offered.units.push(u);
                offered.judgments[1].because.push("u2".into());
                assessed
                    .checks
                    .push(check("unit:u2", SupportStatus::Unknown, &p));
            }
            _ => assessed.checks[2] = check("judgment:1", SupportStatus::Unknown, &p),
        }
        let evaluated = finish_support_revised(
            db.prepare_support(&p, offered, &access()).unwrap(),
            assessed,
            "evaluator",
        )
        .unwrap();
        assert_eq!(evaluated.draft().judgments.len(), 1);
        assert_eq!(
            evaluated.draft().judgments[0].bearing,
            if mode == 1 {
                InterpretationBearing::Good
            } else {
                InterpretationBearing::Unknown
            },
            "mode {mode}"
        );
        let report = evaluated.draft().support.as_ref().unwrap();
        if mode == 0 {
            assert_eq!(report.judgments[1].reason, JudgmentReason::Conflicting);
            assert_eq!(report.judgments[2].reason, JudgmentReason::Conflicting);
        }
        if mode == 1 {
            assert_eq!(report.judgments[1].reason, JudgmentReason::Retained);
            assert_eq!(report.judgments[2].reason, JudgmentReason::Superseded);
        }
    }
}

#[test]
fn revised_canonical_validation_rejects_forged_selection_determinants() {
    let (path, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let evaluated = finish_support_revised(
        db.prepare_support(&p, proposal(&p), &access()).unwrap(),
        reply(&p, SupportStatus::Supported, SupportStatus::Supported),
        "evaluator",
    )
    .unwrap();
    let receipt = db
        .retain_evaluated_interpretation(
            p.clone(),
            finish_support_revised(
                db.prepare_support(&p, proposal(&p), &access()).unwrap(),
                reply(&p, SupportStatus::Supported, SupportStatus::Supported),
                "evaluator",
            )
            .unwrap(),
            &access(),
        )
        .unwrap();
    let sql = rusqlite::Connection::open(path).unwrap();
    let payload: String = sql
        .query_row(
            "SELECT payload FROM revision WHERE id=?1",
            [&receipt.reference.revision],
            |r| r.get(0),
        )
        .unwrap();
    for mode in 0..6 {
        let mut draft = evaluated.draft().clone();
        let report = draft.support.as_mut().unwrap();
        match mode {
            0 => report.judgments[0].reason = JudgmentReason::Superseded,
            1 => report.judgments[0].bearing = InterpretationBearing::Bad,
            2 => report.judgments[0].index = 1,
            3 => report.judgments[0].unsupported_factual_premise = None,
            4 => report.judgments[0].unsupported_factual_premise = Some(true),
            _ => report.protocol = SUPPORT_PROTOCOL.into(),
        }
        let mut record: serde_json::Value = serde_json::from_str(&payload).unwrap();
        record["data"]["draft"] = serde_json::to_value(draft).unwrap();
        let typed: Record = serde_json::from_value(record.clone()).unwrap();
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(serde_json::to_vec(&typed).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        sql.execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![record.to_string(), receipt.reference.revision],
        )
        .unwrap();
        sql.execute(
            "UPDATE interpretation_receipt SET payload_hash=?1 WHERE revision_id=?2",
            rusqlite::params![hash, receipt.reference.revision],
        )
        .unwrap();
        assert!(
            interpretation::validate_schema(&sql).is_err(),
            "mode {mode}"
        );
    }
}

#[test]
fn revised_reopen_retry_and_legacy_bytes_remain_distinct() {
    let (path, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let assessed = reply(&p, SupportStatus::Supported, SupportStatus::Supported);
    let legacy = evaluate(&db, &p, proposal(&p), assessed.clone());
    let json = serde_json::to_string(legacy.draft()).unwrap();
    assert!(!json.contains("unsupported_factual_premise"));
    let revised = || {
        finish_support_revised(
            db.prepare_support(&p, proposal(&p), &access()).unwrap(),
            assessed.clone(),
            "evaluator",
        )
        .unwrap()
    };
    let evaluated = revised();
    assert_eq!(
        evaluated.draft().producer_version,
        SUPPORT_REVISION_PRODUCER
    );
    let retry = revised();
    let receipt = db
        .retain_evaluated_interpretation(p.clone(), evaluated, &access())
        .unwrap();
    assert_eq!(
        receipt,
        db.retain_evaluated_interpretation(p.clone(), retry, &access())
            .unwrap()
    );
    assert!(db
        .retain_evaluated_interpretation(p, legacy, &access())
        .is_err());
    assert!(Memory::open(&path).is_ok());
}

#[test]
fn revised_later_known_replaces_downgraded_known_without_relabeling_old_reason() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.judgments.push(ProposedJudgment {
        goal: criterion().goal,
        bearing: InterpretationBearing::Bad,
        because: vec!["u1".into()],
    });
    let mut assessed = reply(&p, SupportStatus::Supported, SupportStatus::Contradicted);
    assessed
        .checks
        .push(check("judgment:1", SupportStatus::Supported, &p));
    let evaluated = finish_support_revised(
        db.prepare_support(&p, offered, &access()).unwrap(),
        assessed,
        "evaluator",
    )
    .unwrap();
    assert_eq!(
        evaluated.draft().judgments[0].bearing,
        InterpretationBearing::Bad
    );
    let report = evaluated.draft().support.as_ref().unwrap();
    assert_eq!(
        report.judgments[0].reason,
        JudgmentReason::UnsupportedJudgment
    );
    assert_eq!(
        report.judgments[0].offered_bearing,
        InterpretationBearing::Good
    );
    assert_eq!(report.judgments[0].because, vec!["u1"]);
}

// Six independently checked observations amplify only canonical draft copies;
// the original seven passages and every uncited dependency must survive recall.
fn large_atomic_fixture() -> (
    std::path::PathBuf,
    Memory,
    Arc<AtomicBool>,
    InterpretationReceipt,
) {
    large_atomic_fixture_with_text(55)
}

fn large_atomic_fixture_with_text(
    repeats: usize,
) -> (
    std::path::PathBuf,
    Memory,
    Arc<AtomicBool>,
    InterpretationReceipt,
) {
    let (path, mut db, guard) = supported_fixture();
    let mut capture = rich_input("large-atomic");
    for index in 0..4 {
        let mut passage = capture.originals[0].clone();
        passage.source.id = format!("large-atomic-extra-{index}");
        passage.line = index + 2;
        passage.pointer = format!("/content/{index}/text");
        passage.text = format!(
            "Synthetic observation {index}: {}",
            "雪\"\\\n".repeat(repeats)
        );
        capture.originals.push(passage);
    }
    let receipt = db
        .admit_host_capture_with_evidence(capture, &access())
        .unwrap();
    let prepared = prepare(&db, &receipt);
    let source = prepared
        .sources()
        .iter()
        .find(|s| s.text.starts_with("Synthetic observation 0:"))
        .unwrap();
    let citation = SourceCitation {
        source_id: source.id.clone(),
        quote: source.text.chars().take(100).collect(),
    };
    let units = (1..=6)
        .map(|index| ProposedUnit {
            id: format!("o{index}"),
            kind: UnitKind::RecordedEvent,
            text: format!(
                "Synthetic recorded observation {index}: {}",
                "bounded evidence ".repeat(24)
            ),
            citations: vec![citation.clone()],
        })
        .collect::<Vec<_>>();
    let checks = SupportReply {
        checks: units
            .iter()
            .map(|unit| SupportCheck {
                target: format!("unit:{}", unit.id),
                status: SupportStatus::Supported,
                unsupported_factual_premise: false,
                citations: unit.citations.clone(),
                reason: "Authored synthetic check".into(),
                questions: vec![],
            })
            .collect(),
    };
    let request = db
        .prepare_support(
            &prepared,
            SupportProposal {
                units,
                judgments: vec![],
            },
            &access(),
        )
        .unwrap();
    let checked = finish_support(request, checks, "synthetic-checker").unwrap();
    let appraisal = db.prepare_appraisal(checked, &access()).unwrap();
    let evaluated = finish_appraisal(
        appraisal,
        AtomicAppraisal {
            state: AtomicCriterionState::Met,
            because: (1..=6).map(|i| format!("o{i}")).collect(),
            reason: "Authored synthetic appraisal".into(),
            questions: vec![],
        },
        "synthetic-appraiser",
    )
    .unwrap();
    let retained = db
        .retain_evaluated_interpretation(prepared, evaluated, &access())
        .unwrap();
    (path, db, guard, retained)
}

#[test]
fn compact_atomic_reader_fits_full_nine_record_closure_and_preserves_storage() {
    let (path, db, _, retained) = large_atomic_fixture();
    let before: (String, String) = db.conn.query_row(
        "SELECT r.payload,i.payload_hash FROM revision r JOIN interpretation_receipt i ON i.revision_id=r.id WHERE r.id=?1",
        [&retained.reference.revision], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    let packet = read(&db, &context());
    assert_eq!(
        packet.records.len(),
        9,
        "entire atomic interpretation closure must fit"
    );
    let encoded = serde_json::to_value(&packet).unwrap();
    let mut legacy = encoded.clone();
    legacy["records"] = serde_json::to_value(&packet.records).unwrap();
    let old_len = serde_json::to_vec(&legacy).unwrap().len();
    let new_len = serde_json::to_vec(&encoded).unwrap().len();
    eprintln!("legacy bytes={old_len}; compact bytes={new_len}");
    assert!(old_len > 20_000);
    assert!(new_len <= 20_000);
    assert_eq!(packet.interpretation_roles.len(), 1);
    assert_eq!(
        packet.interpretation_roles[0].role,
        InterpretationRole::SelectedCurrent
    );
    let canonical = packet
        .records
        .iter()
        .find(|h| h.reference == retained.reference)
        .unwrap();
    let Record::Interpretation(interpretation) = &canonical.record else {
        panic!("internal record must stay canonical")
    };
    let wire = encoded["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["reference"] == serde_json::to_value(&retained.reference).unwrap())
        .unwrap();
    assert_eq!(wire["record"]["kind"], "interpretation_reader_v1");
    assert_eq!(
        wire["record"]["data"]["support"],
        serde_json::to_value(&interpretation.draft.support).unwrap()
    );
    assert_eq!(
        wire["record"]["data"]["claim_sources"],
        serde_json::to_value(&interpretation.claim_sources).unwrap()
    );
    assert!(wire["record"]["data"].get("draft").is_none());
    assert!(serde_json::from_value::<Record>(wire["record"].clone()).is_err());
    for (hit, rendered) in packet
        .records
        .iter()
        .zip(encoded["records"].as_array().unwrap())
    {
        assert!(hit.private_data && hit.untrusted_input);
        if !matches!(hit.record, Record::Interpretation(_)) {
            assert_canonical_or_native_reader(hit, rendered);
        }
    }
    let after: (String, String) = db.conn.query_row(
        "SELECT r.payload,i.payload_hash FROM revision r JOIN interpretation_receipt i ON i.revision_id=r.id WHERE r.id=?1",
        [&retained.reference.revision], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(before, after);
    Memory::open_read_only(&path).unwrap();
}

#[cfg(feature = "memory-prototype")]
#[tokio::test]
async fn compact_atomic_goalmemory_returns_the_full_nine_record_packet() {
    use crate::tool::{goal_memory::GoalMemory, Tool, ToolCtx};
    let (path, db, state, _) = large_atomic_fixture();
    let packet = read(&db, &context());
    assert_eq!(packet.records.len(), 9);
    let encoded = serde_json::to_value(&packet).unwrap();
    let tool = GoalMemory::open_with_source_guard(
        &path,
        access(),
        "test".into(),
        Situation {
            event: "conversion".into(),
            goal: criterion().goal,
            needs: None,
        },
        Arc::new(Guard(state)),
    )
    .unwrap()
    .with_interpretations(context().read_policy())
    .unwrap();
    let output = tool
        .call(
            serde_json::json!({"memory":{"query":"conversion"}}),
            &ToolCtx::default(),
        )
        .await
        .unwrap();
    assert!(!output.is_error && output.external);
    let actual: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(actual["memory"]["records"], encoded["records"]);
    assert!(serde_json::to_vec(&actual["memory"]).unwrap().len() <= 20_000);
}

#[test]
fn compact_atomic_reader_respects_exact_byte_limit_and_uncited_source_withdrawal() {
    let (_, mut db, _, retained) = large_atomic_fixture();
    let packet = read(&db, &context());
    assert_eq!(packet.records.len(), 9);
    let size = serde_json::to_vec(&packet).unwrap().len();
    let mut policy = ContextPolicy {
        access: access(),
        valid_at: None,
        recall: RecallQuery {
            text: "conversion".into(),
            known_at: db.snapshot().unwrap(),
            account: None,
            session_id: None,
            occurred_after: None,
            occurred_before: None,
            limit: 8,
            neighbors: 0,
            max_bytes: size,
        },
    };
    let request = ContextRequest {
        query: "conversion".into(),
        bindings: vec![],
    };
    let situation = Situation {
        event: "conversion".into(),
        goal: criterion().goal,
        needs: None,
    };
    let exact = db
        .context_with_interpretations(&request, &policy, &situation, &context().read_policy())
        .unwrap();
    assert_eq!(exact.records.len(), 9);
    policy.recall.max_bytes -= 1;
    let smaller = db
        .context_with_interpretations(&request, &policy, &situation, &context().read_policy())
        .unwrap();
    assert!(smaller.budget_omitted);
    assert!(!smaller
        .records
        .iter()
        .any(|h| h.reference == retained.reference));
    assert!(serde_json::to_vec(&smaller).unwrap().len() <= policy.recall.max_bytes);
    // The original first passage was not cited by any accepted observation.
    db.forget(&rich_input("large-atomic").originals[0].source)
        .unwrap();
    assert!(read(&db, &context()).interpretation_roles.is_empty());
}

#[test]
fn stored_interpretation_authenticates_receipt_context_and_uncited_sources() {
    let (_, mut db, state, retained) = large_atomic_fixture();
    let canonical = db
        .stored_interpretation(&retained, &access(), &context())
        .unwrap();
    assert_eq!(
        canonical.draft.support.as_ref().unwrap().protocol,
        ATOMIC_SUPPORT_PROTOCOL
    );
    let mut forged = retained.clone();
    forged.id.push_str("forged");
    assert!(db
        .stored_interpretation(&forged, &access(), &context())
        .is_err());
    forged = retained.clone();
    forged.reference.object.push_str("forged");
    assert!(db
        .stored_interpretation(&forged, &access(), &context())
        .is_err());
    let foreign = InterpretationContext::new(criterion(), "foreign-context".into()).unwrap();
    assert!(db
        .stored_interpretation(&retained, &access(), &foreign)
        .is_err());
    state.store(false, Ordering::SeqCst);
    assert!(db
        .stored_interpretation(&retained, &access(), &context())
        .is_err());
    state.store(true, Ordering::SeqCst);
    db.forget(&rich_input("large-atomic").originals[0].source)
        .unwrap();
    assert!(db
        .stored_interpretation(&retained, &access(), &context())
        .is_err());
}

#[test]
fn compact_atomic_reader_withholds_when_even_compact_closure_is_too_large() {
    let (_, db, _, retained) = large_atomic_fixture_with_text(250);
    let stored = db
        .stored_interpretation(&retained, &access(), &context())
        .unwrap();
    assert_eq!(stored.draft.support.as_ref().unwrap().units.len(), 6);
    let packet = read(&db, &context());
    assert!(packet.budget_omitted);
    assert!(packet.interpretation_roles.is_empty());
    assert!(!packet
        .records
        .iter()
        .any(|h| h.reference == retained.reference));
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 20_000);
}

#[test]
fn compact_atomic_reader_and_host_audit_recheck_final_source_authority() {
    use std::sync::atomic::AtomicUsize;
    struct LastCheck {
        calls: Arc<AtomicUsize>,
        deny_at: usize,
    }
    impl ExternalSourceGuard for LastCheck {
        fn check(&self, _: &ExternalBinding) -> SourceStatus {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 >= self.deny_at {
                SourceStatus::Withheld
            } else {
                SourceStatus::Checked
            }
        }
    }
    let (_, mut db, _, retained) = large_atomic_fixture();
    for host_audit in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        db.source_guard = Some(Arc::new(LastCheck {
            calls: calls.clone(),
            deny_at: usize::MAX,
        }));
        if host_audit {
            db.stored_interpretation(&retained, &access(), &context())
                .unwrap();
        } else {
            assert_eq!(read(&db, &context()).records.len(), 9);
        }
        let count = calls.swap(0, Ordering::SeqCst);
        assert!(count >= 2);
        db.source_guard = Some(Arc::new(LastCheck {
            calls,
            deny_at: count,
        }));
        if host_audit {
            assert!(db
                .stored_interpretation(&retained, &access(), &context())
                .is_err());
        } else {
            let packet = read(&db, &context());
            assert!(packet.records.is_empty() && packet.interpretation_roles.is_empty());
        }
    }
}

#[test]
fn compact_serializer_preserves_legacy_supported_interpretation_bytes() {
    let (_, mut db, _) = supported_fixture();
    let prepared = prepared_supported(&mut db);
    let evaluated = evaluate(
        &db,
        &prepared,
        proposal(&prepared),
        reply(
            &prepared,
            SupportStatus::Supported,
            SupportStatus::Supported,
        ),
    );
    db.retain_evaluated_interpretation(prepared, evaluated, &access())
        .unwrap();
    let packet = read(&db, &context());
    assert_eq!(packet.interpretation_roles.len(), 1);
    let encoded = serde_json::to_value(&packet).unwrap();
    for (hit, rendered) in packet
        .records
        .iter()
        .zip(encoded["records"].as_array().unwrap())
    {
        assert_canonical_or_native_reader(hit, rendered);
    }
}

#[test]
fn stored_interpretation_is_independent_of_contextual_record_count_limit() {
    let (_, mut db, _) = supported_fixture();
    let mut previous = None;
    for episode in 0..2 {
        let mut capture = rich_input(&format!("many-records-{episode}"));
        for index in 0..12 {
            let mut passage = capture.originals[0].clone();
            passage.source.id = format!("many-records-{episode}-extra-{index}");
            passage.line = index + 2;
            passage.pointer = format!("/content/{index}/text");
            passage.text = format!("Synthetic uncited passage {episode}/{index}");
            capture.originals.push(passage);
        }
        if let Some(receipt) = &previous {
            let receipt: &CaptureReceipt = receipt;
            capture.consulted.push(receipt.projection.clone());
        }
        previous = Some(
            db.admit_host_capture_with_evidence(capture, &access())
                .unwrap(),
        );
    }
    let receipt = previous.unwrap();
    let prepared = prepare(&db, &receipt);
    let evaluated = evaluate(
        &db,
        &prepared,
        proposal(&prepared),
        reply(
            &prepared,
            SupportStatus::Supported,
            SupportStatus::Supported,
        ),
    );
    let retained = db
        .retain_evaluated_interpretation(prepared, evaluated, &access())
        .unwrap();
    let packet = read(&db, &context());
    assert!(packet.budget_omitted && packet.interpretation_roles.is_empty());
    db.stored_interpretation(&retained, &access(), &context())
        .unwrap();
}

// Native projections have an explicit presentation; all original evidence and
// legacy interpretation bytes retain their canonical representation.
fn assert_canonical_or_native_reader(hit: &Hit, rendered: &serde_json::Value) {
    let canonical = serde_json::to_value(hit).unwrap();
    if rendered["record"]["kind"] != "native_capture_reader_v1" {
        assert_eq!(*rendered, canonical);
        return;
    }
    let Record::Evidence(source) = &hit.record else {
        panic!("native reader requires canonical projection")
    };
    assert_eq!(source.source.connector, "host-capture-projection-v2");
    let projection: serde_json::Value = serde_json::from_str(&source.text).unwrap();
    for field in [
        "identity",
        "criterion",
        "originals",
        "receipts",
        "artifact",
        "consulted",
        "occurred_at",
    ] {
        assert_eq!(
            rendered["record"]["data"][field], projection[field],
            "native field {field}"
        );
    }
    let mut envelope = rendered.clone();
    envelope["record"] = canonical["record"].clone();
    assert_eq!(envelope, canonical);
}
#[test]
fn encrypted_restore_purges_receipt_siblings_interpretations_and_recaptured_descendants(
) -> anyhow::Result<()> {
    let scratch = super::storage_tests::KeyScratch::new()?;
    let data = scratch.keys.parent().unwrap().join("store");
    let key =
        |name: &str| StorageAccess::generate_key_file(&scratch.keys.join(name), &scratch.policy);
    let current_key = key("current.key")?;
    let backup_key = key("backup.key")?;
    let output_key = key("output.key")?;
    let path = data.join("current.memory-prototype.sqlite");
    let backup = data.join("backup.memory-prototype.sqlite");
    let destination = data.join("restored.memory-prototype.sqlite");
    let live = Arc::new(AtomicBool::new(true));
    let guard = Arc::new(Guard(live));
    let mut db =
        Memory::create_supported_situated_with_storage(&path, "test", current_key.clone())?
            .with_source_guard(guard.clone());
    let p = prepared_supported(&mut db);
    let mut offered = proposal(&p);
    offered.units[0].kind = UnitKind::Hypothesis;
    let evaluated = evaluate(
        &db,
        &p,
        offered,
        reply(&p, SupportStatus::Unknown, SupportStatus::Unknown),
    );
    let first = db.retain_evaluated_interpretation(p, evaluated, &access())?;
    let mut next = rich_input("second");
    next.consulted.push(first.reference.clone());
    next.originals[0].text = "Synthetic copied speculation about artifact".into();
    let second = db.admit_host_capture_with_evidence(next.clone(), &access())?;
    // Opaque prepared state predating forgetting is never made valid by restore.
    let pending = prepare(&db, &second);
    let pending_evaluated = evaluate(
        &db,
        &pending,
        proposal(&pending),
        reply(&pending, SupportStatus::Unknown, SupportStatus::Unknown),
    );
    db.backup_encrypted(&backup, backup_key.clone())?;
    db.forget(&rich_input("rich").originals[0].source)?;
    restore_encrypted(
        &backup,
        backup_key,
        &destination,
        output_key.clone(),
        &RestoreAuthority::current(&path, current_key),
    )?;
    let mut restored =
        Memory::open_with_storage(&destination, output_key)?.with_source_guard(guard);
    for reference in second
        .originals
        .iter()
        .chain([&second.projection, &first.reference])
    {
        assert!(restored.read(reference, &access())?.is_none());
    }
    assert!(restored
        .retain_evaluated_interpretation(pending, pending_evaluated, &access())
        .is_err());
    assert!(restored
        .admit_host_capture_with_evidence(next.clone(), &access())
        .is_err());
    next.identity.prefix_digest = "0".repeat(64);
    assert!(restored
        .admit_host_capture_with_evidence(next, &access())
        .is_err());
    assert!(restored
        .admit_host_capture_with_evidence(rich_input("rich"), &access())
        .is_err());
    assert_eq!(
        restored
            .conn
            .query_row("SELECT count(*) FROM external_binding", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        restored
            .conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| r
                .get::<_, i64>(0))?,
        0
    );
    assert_eq!(restored.conn.query_row("SELECT count(*) FROM host_capture_receipt WHERE state='forgotten' AND payload_hash IS NULL AND receipt_id IS NULL AND originals IS NULL",[],|r|r.get::<_,i64>(0))?,2);
    assert_eq!(restored.conn.query_row("SELECT count(*) FROM interpretation_receipt WHERE state='forgotten' AND payload_hash IS NULL AND object_id IS NULL",[],|r|r.get::<_,i64>(0))?,1);
    Ok(())
}
