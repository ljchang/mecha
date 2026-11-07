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
        criterion_text: "write JSON".into(),
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
#[test]
fn situated_schema_and_exact_retry() {
    let (path, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let d = draft(&p);
    let a = db
        .retain_interpretation(p.clone(), d.clone(), &access())
        .unwrap();
    assert_eq!(
        a.reference,
        db.retain_interpretation(p.clone(), d.clone(), &access())
            .unwrap()
            .reference
    );
    let mut conflict = d;
    conflict.interpretation.push('!');
    assert!(db.retain_interpretation(p, conflict, &access()).is_err());
    assert!(Memory::open(&path).is_ok());
    assert_eq!(read(&db, &context()).interpretation_roles.len(), 1);
}
#[test]
fn forged_receipt_quote_goal_and_foreign_store_refuse() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let mut forged = r.clone();
    forged.originals.clear();
    assert!(db
        .prepare_interpretation(&forged, &[], &context(), &db.snapshot().unwrap(), &access())
        .is_err());
    let p = prepare(&db, &r);
    let mut d = draft(&p);
    d.claims[0].quote = "completely fabricated quotation".into();
    assert!(db.retain_interpretation(p.clone(), d, &access()).is_err());
    let mut d = draft(&p);
    d.judgments[0].goal = "foreign".into();
    assert!(db.retain_interpretation(p.clone(), d, &access()).is_err());
    let (_, mut other, _) = fixture();
    assert!(other
        .retain_interpretation(p.clone(), draft(&p), &access())
        .is_err());
}
#[test]
fn authority_change_and_forgetting_refuse_late_retention() {
    let (_, mut db, state) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    state.store(false, Ordering::SeqCst);
    assert!(db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .is_err());
    state.store(true, Ordering::SeqCst);
    let saved = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    db.forget(&input("one").originals[0].source).unwrap();
    assert!(db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .is_err());
    assert!(db.read(&saved.reference, &access()).unwrap().is_none());
}
#[test]
fn source_ids_are_qualified_and_private_context_is_absent() {
    let (_, mut db, _) = fixture();
    let a = db.admit_host_capture(input("one"), &access()).unwrap();
    let b = db.admit_host_capture(input("two"), &access()).unwrap();
    let p = db
        .prepare_interpretation(
            &a,
            &[b.projection],
            &context(),
            &db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let ids = p.sources().iter().map(|s| &s.id).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), p.sources().len());
    assert!(!p.generation_input().contains("opaque-host-context"));
    let other = InterpretationContext::new(criterion(), "other-private-context".into()).unwrap();
    let q = db
        .prepare_interpretation(&a, &[], &other, &db.snapshot().unwrap(), &access())
        .unwrap();
    assert_eq!(prepare(&db, &a).generation_input(), q.generation_input());
}
#[test]
fn changed_goal_recapture_labels_historical_ancestors_and_default_hides_descendants() {
    let (_, mut db, _) = fixture();
    let first = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &first);
    let original = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let mut next = input("two");
    next.consulted = read(&db, &context())
        .records
        .iter()
        .map(|h| h.reference.clone())
        .collect();
    let recaptured = db.admit_host_capture(next, &access()).unwrap();
    assert!(db
        .read(&recaptured.projection, &access())
        .unwrap()
        .is_none());
    let mut c = criterion();
    c.goal = "charter:2".into();
    c.criterion_revision = "1".repeat(64);
    let c = InterpretationContext::new(c, "second-context".into()).unwrap();
    let p = db
        .prepare_interpretation(&recaptured, &[], &c, &db.snapshot().unwrap(), &access())
        .unwrap();
    let mut d = draft(&p);
    d.judgments[0].goal = c.criterion().goal.clone();
    let current = db.retain_interpretation(p, d, &access()).unwrap();
    let packet = read(&db, &c);
    assert!(packet
        .interpretation_roles
        .iter()
        .any(|r| r.reference == original.reference
            && r.role == InterpretationRole::HistoricalDependency));
    assert!(
        packet
            .interpretation_roles
            .iter()
            .any(|r| r.reference == current.reference
                && r.role == InterpretationRole::SelectedCurrent)
    );
}
#[test]
fn offered_claims_can_exceed_retained_bound() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let mut d = draft(&p);
    d.grounding.offered = 32;
    d.grounding.dropped = 31;
    d.grounding.dropped_by.insert("unsupported".into(), 31);
    assert!(db.retain_interpretation(p, d, &access()).is_ok());
}
#[test]
fn uncited_supplied_input_and_current_revision_are_checked() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let ancestor = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let second = db.admit_host_capture(input("two"), &access()).unwrap();
    let p = db
        .prepare_interpretation(
            &second,
            std::slice::from_ref(&ancestor.reference),
            &context(),
            &db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let mut d = draft(&p);
    let source = p
        .sources()
        .iter()
        .find(|s| s.source == second.originals[0])
        .unwrap();
    d.claims[0].source_id = source.id.clone();
    db.withdraw_interpretation(&ancestor.reference, &access())
        .unwrap();
    assert!(db.retain_interpretation(p, d, &access()).is_err());
    let p = prepare(&db, &second);
    let mut updated = input("two");
    updated.identity.prefix_digest = "3".repeat(64);
    updated.originals[0].text = "new synthetic source observation".into();
    db.admit_host_capture(updated, &access()).unwrap();
    assert!(db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .is_err());
}
#[test]
fn defaults_withhold_all_interpretation_bearing_descendants() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let saved = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let mut next = input("two");
    next.originals[0].text = "hypothesis copied into new observation".into();
    next.consulted = vec![saved.reference.clone()];
    let next = db.admit_host_capture(next, &access()).unwrap();
    let forbidden = [saved.reference, next.projection, next.originals[0].clone()];
    let query = RecallQuery {
        text: "hypothesis".into(),
        known_at: db.snapshot().unwrap(),
        account: None,
        session_id: None,
        occurred_after: None,
        occurred_before: None,
        limit: 100,
        neighbors: 0,
        max_bytes: 20000,
    };
    for text in ["", "hypothesis"] {
        let hits = db
            .search(
                &Query {
                    text: text.into(),
                    known_at: db.snapshot().unwrap(),
                    valid_at: None,
                    include_possible: true,
                    limit: 1000,
                },
                &access(),
            )
            .unwrap();
        assert!(hits.iter().all(|h| !forbidden.contains(&h.reference)));
    }
    assert!(db
        .recall(&query, &access())
        .unwrap()
        .iter()
        .all(|h| !forbidden.contains(&h.anchor.reference)));
    let packet = db
        .context(
            &ContextRequest {
                query: "hypothesis".into(),
                bindings: vec![],
            },
            &ContextPolicy {
                recall: query,
                access: access(),
                valid_at: None,
            },
            &Situation {
                goal: criterion().goal,
                event: "hypothesis".into(),
                needs: None,
            },
        )
        .unwrap();
    assert!(packet
        .records
        .iter()
        .all(|h| !forbidden.contains(&h.reference)));
    assert!(!serde_json::to_string(&packet)
        .unwrap()
        .contains("interpretation_roles"));
}
#[test]
fn explicit_schema_and_access_are_required() {
    let (path, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let mut wrong = access();
    wrong.scopes.insert("extra".into());
    assert!(db
        .retain_interpretation(p.clone(), draft(&p), &wrong)
        .is_err());
    wrong.domain = "other".into();
    assert!(db
        .prepare_interpretation(&r, &[], &context(), &db.snapshot().unwrap(), &wrong)
        .is_err());
    let legacy = path.with_file_name("legacy.memory-prototype.sqlite");
    let mut old = Memory::create_host_capture(&legacy, "test")
        .unwrap()
        .with_source_guard(Arc::new(Guard(Arc::new(AtomicBool::new(true)))));
    let r = old.admit_host_capture(input("old"), &access()).unwrap();
    assert!(old
        .prepare_interpretation(&r, &[], &context(), &old.snapshot().unwrap(), &access())
        .is_err());
    assert_eq!(
        old.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6
    );
    db.conn
        .execute_batch("DROP TABLE interpretation_receipt")
        .unwrap();
    drop(db);
    assert!(Memory::open(&path).is_err());
}
#[test]
fn mismatched_context_and_complete_bundle_budget() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    db.retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let c = InterpretationContext::new(criterion(), "different-host-context".into()).unwrap();
    assert!(read(&db, &c).interpretation_roles.is_empty());
    let policy = ContextPolicy {
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
            max_bytes: 1500,
        },
        valid_at: None,
    };
    let packet = db
        .context_with_interpretations(
            &ContextRequest {
                query: "conversion".into(),
                bindings: vec![],
            },
            &policy,
            &Situation {
                event: "conversion".into(),
                goal: criterion().goal,
                needs: None,
            },
            &context().read_policy(),
        )
        .unwrap();
    assert!(packet.interpretation_roles.is_empty());
    assert!(packet.budget_omitted);
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 1500);
}
#[test]
fn contextual_reader_honors_source_filters_and_packet_envelope_bound() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    db.retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let mut policy = ContextPolicy {
        access: access(),
        recall: RecallQuery {
            text: "conversion".into(),
            known_at: db.snapshot().unwrap(),
            account: Some("different-account".into()),
            session_id: None,
            occurred_after: None,
            occurred_before: None,
            limit: 8,
            neighbors: 0,
            max_bytes: 20000,
        },
        valid_at: None,
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
    assert!(db
        .context_with_interpretations(&request, &policy, &situation, &context().read_policy())
        .unwrap()
        .interpretation_roles
        .is_empty());
    policy.recall.account = None;
    policy.recall.max_bytes = 1;
    assert!(db
        .context_with_interpretations(&request, &policy, &situation, &context().read_policy())
        .is_err());
}
#[test]
fn source_withdrawn_at_final_return_withholds_entire_bundle() {
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
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    db.retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    db.source_guard = Some(Arc::new(LastCheck {
        calls: calls.clone(),
        deny_at: usize::MAX,
    }));
    assert_eq!(read(&db, &context()).interpretation_roles.len(), 1);
    let count = calls.swap(0, Ordering::SeqCst);
    assert!(count >= 2);
    db.source_guard = Some(Arc::new(LastCheck {
        calls,
        deny_at: count,
    }));
    let packet = read(&db, &context());
    assert!(packet.records.is_empty());
    assert!(packet.interpretation_roles.is_empty());
}
#[test]
fn oversized_generation_and_unsupported_judgments_refuse() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let mut d = draft(&p);
    d.judgments[0].because.clear();
    assert!(db.retain_interpretation(p.clone(), d, &access()).is_err());
    let mut d = draft(&p);
    d.interpretation = "x".repeat(8193);
    assert!(db.retain_interpretation(p, d, &access()).is_err());
    let mut roots = vec![];
    for n in 0..8 {
        let mut h = input(&format!("large-{n}"));
        h.originals[0].text = "x".repeat(4096);
        roots.push(db.admit_host_capture(h, &access()).unwrap().projection);
    }
    assert!(db
        .prepare_interpretation(&r, &roots, &context(), &db.snapshot().unwrap(), &access())
        .is_err());
    assert!(db
        .prepare_interpretation(
            &r,
            &vec![r.projection.clone(); 16],
            &context(),
            &db.snapshot().unwrap(),
            &access()
        )
        .is_err());
}
#[test]
fn copied_interpretation_text_never_acquires_observation_kind() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let saved = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let mut next = input("two");
    next.originals[0].kind = PassageKind::ToolResult;
    next.originals[0].text = "a provisional hypothesis copied from memory".into();
    next.consulted = vec![saved.reference];
    let next = db.admit_host_capture(next, &access()).unwrap();
    let p = prepare(&db, &next);
    assert!(p
        .sources()
        .iter()
        .filter(|s| s.source == next.originals[0])
        .all(|s| s.kind == SourceKind::InterpretationInfluenced));
}
#[test]
fn reopened_identical_quotes_resolve_exact_durable_claim_source() {
    let (path, mut db, state) = fixture();
    let first = db.admit_host_capture(input("one"), &access()).unwrap();
    let second = db.admit_host_capture(input("two"), &access()).unwrap();
    let p = db
        .prepare_interpretation(
            &first,
            std::slice::from_ref(&second.projection),
            &context(),
            &db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let chosen = p
        .sources()
        .iter()
        .find(|s| s.source == second.originals[0])
        .unwrap()
        .clone();
    let mut d = draft(&p);
    d.claims[0].source_id = chosen.id.clone();
    let saved = db.retain_interpretation(p, d, &access()).unwrap();
    drop(db);
    let db = Memory::open(&path)
        .unwrap()
        .with_source_guard(Arc::new(Guard(state)));
    let packet = read(&db, &context());
    let record = &packet
        .records
        .iter()
        .find(|h| h.reference == saved.reference)
        .unwrap()
        .record;
    let json = serde_json::to_value(record).unwrap();
    let maps = json["data"]["claim_sources"]
        .as_array()
        .expect("durable claim attribution must be returned");
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0]["id"], chosen.id);
    assert_eq!(
        maps[0]["source"],
        serde_json::to_value(&chosen.source).unwrap()
    );
    assert_eq!(maps[0]["field"], chosen.field);
    assert_eq!(maps[0]["kind"], serde_json::to_value(chosen.kind).unwrap());
    assert!(maps[0].get("text").is_none());
}
#[test]
fn contextual_time_windows_compare_normalized_source_times() {
    for time in ["2026-11-07T10:00:00-05:00", "2026-11-07T15:00:00.000Z"] {
        let (_, mut db, _) = fixture();
        let mut h = input("one");
        h.occurred_at = Some(time.into());
        h.originals[0].occurred_at = Some(time.into());
        let r = db.admit_host_capture(h, &access()).unwrap();
        let p = prepare(&db, &r);
        db.retain_interpretation(p.clone(), draft(&p), &access())
            .unwrap();
        let mut policy = ContextPolicy {
            access: access(),
            recall: RecallQuery {
                text: "conversion".into(),
                known_at: db.snapshot().unwrap(),
                account: None,
                session_id: None,
                occurred_after: Some("2026-11-07T14:00:00Z".into()),
                occurred_before: Some("2026-11-07T16:00:00Z".into()),
                limit: 8,
                neighbors: 0,
                max_bytes: 20000,
            },
            valid_at: None,
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
        assert_eq!(
            db.context_with_interpretations(
                &request,
                &policy,
                &situation,
                &context().read_policy()
            )
            .unwrap()
            .interpretation_roles
            .len(),
            1,
            "{time}"
        );
        policy.recall.occurred_after = Some("2026-11-07T16:00:00Z".into());
        policy.recall.occurred_before = Some("2026-11-07T18:00:00Z".into());
        assert!(db
            .context_with_interpretations(&request, &policy, &situation, &context().read_policy())
            .unwrap()
            .interpretation_roles
            .is_empty());
    }
}
#[test]
fn v7_projection_deduplicates_text_losslessly_and_v6_bytes_stay_exact() {
    let (path, mut db, _) = fixture();
    let mut h = input("one");
    h.originals[0].text.push_str(&"x".repeat(3000));
    let r = db.admit_host_capture(h.clone(), &access()).unwrap();
    let Some(Record::Evidence(projection)) = db.read(&r.projection, &access()).unwrap() else {
        panic!()
    };
    let json: serde_json::Value = serde_json::from_str(&projection.text).unwrap();
    assert!(
        json["originals"][0].get("text").is_none(),
        "v7 projection must reference original bytes rather than duplicate them"
    );
    assert_eq!(
        json["originals"][0]["reference"],
        serde_json::to_value(&r.originals[0]).unwrap()
    );
    let Some(Record::Evidence(original)) = db.read(&r.originals[0], &access()).unwrap() else {
        panic!()
    };
    assert_eq!(original.text, h.originals[0].text);
    let p = prepare(&db, &r);
    db.retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let first = read(&db, &context());
    let mut h = input("two");
    h.originals[0].text = "hypothesis memory response ".repeat(120);
    h.consulted = first.records.iter().map(|h| h.reference.clone()).collect();
    let r = db.admit_host_capture(h, &access()).unwrap();
    let mut c = criterion();
    c.goal = "charter:2".into();
    let c = InterpretationContext::new(c, "changed-context".into()).unwrap();
    let p = db
        .prepare_interpretation(&r, &[], &c, &db.snapshot().unwrap(), &access())
        .unwrap();
    let mut d = draft(&p);
    d.judgments[0].goal = c.criterion().goal.clone();
    db.retain_interpretation(p, d, &access()).unwrap();
    let packet = read(&db, &c);
    assert_eq!(packet.interpretation_roles.len(), 2);
    assert!(packet
        .records
        .iter()
        .any(|hit| hit.reference == r.originals[0]));
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 20_000);
    let mut legacy =
        Memory::create_host_capture(&path.with_file_name("old.memory-prototype.sqlite"), "test")
            .unwrap()
            .with_source_guard(Arc::new(Guard(Arc::new(AtomicBool::new(true)))));
    let h = input("legacy");
    let r = legacy.admit_host_capture(h.clone(), &access()).unwrap();
    let Some(Record::Evidence(e)) = legacy.read(&r.projection, &access()).unwrap() else {
        panic!()
    };
    assert_eq!(e.text, serde_json::to_string(&h).unwrap());
}
#[test]
fn durable_claim_map_is_revalidated_against_actual_source_fields() {
    use sha2::Digest;
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let saved = db
        .retain_interpretation(p.clone(), draft(&p), &access())
        .unwrap();
    let mut record = super::sql::lookup(&db.conn, &saved.reference)
        .unwrap()
        .unwrap();
    let Record::Interpretation(i) = &mut record else {
        panic!()
    };
    i.claim_sources[0].field = "/unrelated-field".into();
    let payload = serde_json::to_string(&record).unwrap();
    let digest: String = sha2::Sha256::digest(payload.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    db.conn
        .execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![payload, saved.reference.revision],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE interpretation_receipt SET payload_hash=?1 WHERE revision_id=?2",
            rusqlite::params![digest, saved.reference.revision],
        )
        .unwrap();
    assert!(!super::interpretation::canonical(&db.conn, &saved.reference).unwrap());
}
#[test]
fn v7_descriptor_pointer_and_reference_tampering_refuse_reconstruction() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let Some(Record::Evidence(mut projection)) =
        super::sql::lookup(&db.conn, &r.projection).unwrap()
    else {
        panic!()
    };
    let mut json: serde_json::Value = serde_json::from_str(&projection.text).unwrap();
    json["originals"][0]["pointer"] = "/forged".into();
    projection.text = serde_json::to_string(&json).unwrap();
    db.conn
        .execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&Record::Evidence(projection)).unwrap(),
                r.projection.revision
            ],
        )
        .unwrap();
    assert!(db
        .prepare_interpretation(&r, &[], &context(), &db.snapshot().unwrap(), &access())
        .is_err());
}
#[test]
fn malformed_grounding_counts_refuse_without_overflow() {
    let (_, mut db, _) = fixture();
    let r = db.admit_host_capture(input("one"), &access()).unwrap();
    let p = prepare(&db, &r);
    let mut d = draft(&p);
    d.grounding.dropped = usize::MAX;
    assert!(db.retain_interpretation(p, d, &access()).is_err());
}
