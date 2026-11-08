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
fn access() -> Access {
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
fn supported_fixture() -> (std::path::PathBuf, Memory, Arc<AtomicBool>) {
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
fn prepared_supported(db: &mut Memory) -> PreparedInterpretation {
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
fn proposal(p: &PreparedInterpretation) -> SupportProposal {
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
fn reply(p: &PreparedInterpretation, unit: SupportStatus, judgment: SupportStatus) -> SupportReply {
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
