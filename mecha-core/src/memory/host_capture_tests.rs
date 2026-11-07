use super::*;
use std::collections::BTreeSet;
#[test]
fn missing_guard_withholds_bound_evidence_but_not_legacy() {
    let dir = std::env::temp_dir().join(format!("bridge-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let mut db = Memory::create(&dir.join("x.memory-prototype.sqlite"), "test").unwrap();
    let r = db
        .capture(Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "test".into(),
                id: "one".into(),
            },
            text: "guarded evidence".into(),
            occurred_at: None,
            scopes: BTreeSet::new(),
            transcript: None,
        })
        .unwrap();
    let a = Access {
        domain: "test".into(),
        scopes: BTreeSet::new(),
    };
    assert!(db.read(&r, &a).unwrap().is_some());
    db.conn.execute_batch("CREATE TABLE external_binding(revision TEXT PRIMARY KEY,authority TEXT,locator TEXT,binding_revision TEXT);").unwrap();
    db.conn
        .execute(
            "INSERT INTO external_binding VALUES (?1,'host-v1',?2,?2)",
            rusqlite::params![r.revision, "a".repeat(64)],
        )
        .unwrap();
    assert!(
        db.read(&r, &a).unwrap().is_none(),
        "missing authority must withhold bound evidence"
    );
}

use super::host_capture::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
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
fn input() -> HostCapture {
    HostCapture {
        identity: CaptureIdentity {
            domain: "test".into(),
            account: "test".into(),
            session_id: "s1".into(),
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
        criterion: ConfirmedCriterion {
            goal: "charter:1".into(),
            goal_text: "convert CSV".into(),
            criterion_text: "write JSON".into(),
            criterion_revision: "e".repeat(64),
            confirmation_digest: "f".repeat(64),
        },
        originals: vec![OriginalPassage {
            source: SourceKey {
                connector: "bridge".into(),
                account: "test".into(),
                id: "s1:1:text".into(),
            },
            kind: PassageKind::UserRequest,
            line: 1,
            pointer: "/message/content/0/text".into(),
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
    let dir = std::env::temp_dir().join(format!("bridge-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("x.memory-prototype.sqlite");
    let state = Arc::new(AtomicBool::new(true));
    let db = Memory::create_host_capture(&path, "test")
        .unwrap()
        .with_source_guard(Arc::new(Guard(state.clone())));
    (path, db, state)
}
#[test]
fn host_admission_atomic_retry_and_conflict() {
    let (_p, mut db, _s) = fixture();
    let capture = input();
    let receipt = db.admit_host_capture(capture.clone(), &access()).unwrap();
    assert_eq!(
        receipt,
        db.admit_host_capture(capture.clone(), &access()).unwrap()
    );
    assert_eq!(
        super::sql::closure(&db.conn, &receipt.projection)
            .unwrap()
            .len(),
        2
    );
    let seq = db.snapshot().unwrap().sequence;
    let mut changed = capture;
    changed.scopes.insert("private".into());
    assert!(db.admit_host_capture(changed, &access()).is_err());
    assert_eq!(seq, db.snapshot().unwrap().sequence);
}
#[test]
fn host_admission_refuses_unsafe_consulted_and_revoked_retry() {
    let (_p, mut db, state) = fixture();
    let mut capture = input();
    let legacy = db
        .capture(Capture {
            source: SourceKey {
                connector: "legacy".into(),
                account: "test".into(),
                id: "sensor".into(),
            },
            text: "HOST SENSOR SECRET".into(),
            occurred_at: None,
            scopes: BTreeSet::new(),
            transcript: None,
        })
        .unwrap();
    capture.consulted.push(legacy);
    let before = db.snapshot().unwrap().sequence;
    assert!(db.admit_host_capture(capture, &access()).is_err());
    assert_eq!(before, db.snapshot().unwrap().sequence);
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    state.store(false, Ordering::SeqCst);
    assert!(db.admit_host_capture(input(), &access()).is_err());
    assert!(db.read(&receipt.projection, &access()).unwrap().is_none());
}
#[test]
fn forgetting_projection_suppresses_all_originals_and_new_prefix() {
    let (path, mut db, _s) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let Record::Evidence(projection) = db.read(&receipt.projection, &access()).unwrap().unwrap()
    else {
        panic!()
    };
    db.forget(&projection.source).unwrap();
    assert!(db.read(&receipt.originals[0], &access()).unwrap().is_none());
    assert!(db.admit_host_capture(input(), &access()).is_err());
    let mut changed = input();
    changed.identity.prefix_digest = "0".repeat(64);
    assert!(db.admit_host_capture(changed, &access()).is_err());
    let count: i64 = db
        .conn
        .query_row("SELECT count(*) FROM external_binding", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let db = Memory::open(&path).unwrap();
    assert!(db.read(&receipt.projection, &access()).unwrap().is_none());
}

#[test]
fn unsupported_relation_producer_cannot_consult_bridge_records() {
    let (_p, mut db, _s) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let entity = db.entity("synthetic", BTreeSet::new()).unwrap();
    let result = db.relate(RelationInput {
        predicate: "observed".into(),
        subject: entity,
        object: Target::Literal("synthetic".into()),
        support: Span {
            evidence: receipt.originals[0].clone(),
            start: 0,
            end: 7,
        },
        consulted: vec![],
        text: "synthetic relation".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Unknown,
            to: Bound::Unknown,
        },
    });
    assert!(
        result.is_err(),
        "unsupported producer must not consume guarded closure"
    );
}

#[test]
fn v6_missing_binding_table_never_becomes_legacy_authority() {
    let (path, mut db, _state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    db.conn
        .execute_batch("DROP TABLE external_binding;")
        .unwrap();
    assert!(
        db.read(&receipt.projection, &access()).is_err()
            || db.read(&receipt.projection, &access()).unwrap().is_none(),
        "broken v6 schema must withhold"
    );
    assert!(
        Memory::open(&path).is_err(),
        "broken v6 schema must not reopen as legacy"
    );
}

#[test]
fn absent_or_malformed_current_binding_withholds_whole_capture() {
    let (_path, mut db, _state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    db.conn
        .execute(
            "DELETE FROM external_binding WHERE revision=?1",
            [&receipt.originals[0].revision],
        )
        .unwrap();
    let result = db.read(&receipt.originals[0], &access());
    assert!(
        result.is_err() || result.unwrap().is_none(),
        "an original with a missing binding must not become legacy"
    );
}

fn recall(db: &Memory, text: &str) -> RecallQuery {
    RecallQuery {
        text: text.into(),
        known_at: db.snapshot().unwrap(),
        account: Some("test".into()),
        session_id: None,
        occurred_after: None,
        occurred_before: None,
        limit: 20,
        neighbors: 2,
        max_bytes: 100_000,
    }
}
fn policy(db: &Memory, text: &str) -> context::ContextPolicy {
    context::ContextPolicy {
        access: access(),
        recall: recall(db, text),
        valid_at: None,
    }
}
fn situation() -> context::Situation {
    context::Situation {
        event: "another conversion".into(),
        goal: "charter:2".into(),
        needs: None,
    }
}
#[test]
fn every_supported_read_door_rechecks_authority_after_reopen() {
    let (path, mut db, state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let query = Query {
        text: "synthetic".into(),
        known_at: db.snapshot().unwrap(),
        valid_at: None,
        include_possible: true,
        limit: 50,
    };
    let request = context::ContextRequest {
        query: "synthetic".into(),
        bindings: vec![],
    };
    assert!(!db.search(&query, &access()).unwrap().is_empty());
    assert!(!db
        .recall(&recall(&db, "synthetic"), &access())
        .unwrap()
        .is_empty());
    assert!(!db
        .recall_mention(&recall(&db, "CSV"), "synthetic", &access())
        .unwrap()
        .is_empty());
    assert!(!db
        .context(&request, &policy(&db, "synthetic"), &situation())
        .unwrap()
        .records
        .is_empty());
    let without = Memory::open_read_only(&path).unwrap();
    assert!(without
        .read(&receipt.projection, &access())
        .unwrap()
        .is_none());
    assert!(without.search(&query, &access()).unwrap().is_empty());
    assert!(without
        .recall(&recall(&without, "synthetic"), &access())
        .unwrap()
        .is_empty());
    assert!(without
        .recall_mention(&recall(&without, "CSV"), "synthetic", &access())
        .unwrap()
        .is_empty());
    assert!(without
        .context(&request, &policy(&without, "synthetic"), &situation())
        .unwrap()
        .records
        .is_empty());
    state.store(false, Ordering::SeqCst);
    assert!(db.read(&receipt.projection, &access()).unwrap().is_none());
    assert!(db.search(&query, &access()).unwrap().is_empty());
    assert!(db
        .recall(&recall(&db, "synthetic"), &access())
        .unwrap()
        .is_empty());
    assert!(db
        .recall_mention(&recall(&db, "CSV"), "synthetic", &access())
        .unwrap()
        .is_empty());
    assert!(db
        .context(&request, &policy(&db, "synthetic"), &situation())
        .unwrap()
        .records
        .is_empty());
}
struct ChangingGuard(std::sync::atomic::AtomicUsize, usize);
impl ExternalSourceGuard for ChangingGuard {
    fn check(&self, _: &ExternalBinding) -> SourceStatus {
        if self.0.fetch_add(1, Ordering::SeqCst) < self.1 {
            SourceStatus::Checked
        } else {
            SourceStatus::Unavailable
        }
    }
}
#[test]
fn final_checks_remove_capture_revoked_after_search_selection_and_recall_ranking() {
    let (_path, mut db, _state) = fixture();
    db.admit_host_capture(input(), &access()).unwrap();
    // One matching projection has a two-member closure. The first two checks
    // admit selection, and authority disappears before the final pass.
    let query = Query {
        text: "criterion_text".into(),
        known_at: db.snapshot().unwrap(),
        valid_at: None,
        include_possible: true,
        limit: 10,
    };
    let db = db.with_source_guard(Arc::new(ChangingGuard(
        std::sync::atomic::AtomicUsize::new(0),
        2,
    )));
    assert!(db.search(&query, &access()).unwrap().is_empty());
    // Eligible-index construction checks all three closure memberships (one
    // original plus projection+original); candidate validation adds two more.
    let db = db.with_source_guard(Arc::new(ChangingGuard(
        std::sync::atomic::AtomicUsize::new(0),
        5,
    )));
    assert!(db
        .recall(&recall(&db, "criterion_text"), &access())
        .unwrap()
        .is_empty());
    let db = db.with_source_guard(Arc::new(ChangingGuard(
        std::sync::atomic::AtomicUsize::new(0),
        // Context also checks bundle authorization and every dependency:
        // keep authority through those five extra checks, then revoke.
        10,
    )));
    let packet = db
        .context(
            &context::ContextRequest {
                query: "criterion_text".into(),
                bindings: vec![],
            },
            &policy(&db, "criterion_text"),
            &situation(),
        )
        .unwrap();
    assert!(packet.records.is_empty());
    assert!(packet.bindings.is_empty());
}
#[test]
fn withheld_bridge_records_do_not_change_eligible_ranking_statistics() {
    let (_path, mut db, state) = fixture();
    for (id, text) in [("one", "alpha beta beta"), ("two", "alpha alpha beta")] {
        db.capture(Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "test".into(),
                id: id.into(),
            },
            text: text.into(),
            occurred_at: None,
            scopes: BTreeSet::new(),
            transcript: None,
        })
        .unwrap();
    }
    let snapshot = db.snapshot().unwrap();
    let before = db.recall(&recall(&db, "alpha beta"), &access()).unwrap();
    for n in 0..12 {
        let mut c = input();
        c.identity.session_id = format!("s{n}");
        c.identity.prefix_digest = format!("{n:064x}");
        c.originals[0].source.id = format!("s{n}:1:text");
        c.originals[0].text = "beta beta beta beta beta".into();
        db.admit_host_capture(c, &access()).unwrap();
    }
    state.store(false, Ordering::SeqCst);
    let after = db.recall(&recall(&db, "alpha beta"), &access()).unwrap();
    assert_eq!(
        before
            .iter()
            .map(|h| (&h.anchor.reference, h.bm25))
            .collect::<Vec<_>>(),
        after
            .iter()
            .map(|h| (&h.anchor.reference, h.bm25))
            .collect::<Vec<_>>()
    );
    assert_eq!(snapshot.sequence, 2);
}
#[test]
fn v6_refuses_legacy_lifecycle_surfaces_and_nested_unknown_fields() {
    let (_path, mut db, _state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let read = durable::Receipt {
        instance: db.instance.clone(),
        id: "fake".into(),
        generation: 0,
    };
    let model = durable_decision::ModelReceipt {
        instance: db.instance.clone(),
        id: "fake".into(),
        generation: 0,
    };
    let errors = [
        db.durable_reopen("fake", &access())
            .unwrap_err()
            .to_string(),
        db.durable_status(&read, &access()).unwrap_err().to_string(),
        db.durable_spec(&read, &access()).unwrap_err().to_string(),
        db.durable_model_reopen("fake", &access())
            .unwrap_err()
            .to_string(),
        db.durable_model_status(&model, &access())
            .unwrap_err()
            .to_string(),
        db.durable_model_spec(&model, &access())
            .unwrap_err()
            .to_string(),
        db.durable_action_reopen("fake", &access())
            .unwrap_err()
            .to_string(),
    ];
    assert!(
        errors
            .iter()
            .all(|e| e.contains("requires an explicitly created version")),
        "{errors:?}"
    );
    assert!(db
        .prepare_concept(
            &receipt
                .originals
                .iter()
                .cloned()
                .chain(std::iter::once(receipt.projection))
                .collect::<Vec<_>>(),
            &db.snapshot().unwrap(),
            &access()
        )
        .is_err());
    let version: i64 = db
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 6);
    let mut value = serde_json::to_value(input()).unwrap();
    value["originals"][0]["source"]["host_sensor"] = serde_json::json!("SECRET");
    assert!(serde_json::from_value::<HostCapture>(value).is_err());
    let mut value = serde_json::to_value(input()).unwrap();
    value["consulted"] = serde_json::json!([{"object":"x","revision":"y","host_sensor":"SECRET"}]);
    assert!(serde_json::from_value::<HostCapture>(value).is_err());
}
#[test]
fn failed_new_schema_initialization_leaves_no_lower_version_file() {
    let dir = std::env::temp_dir().join(format!("bridge-init-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("bad.memory-prototype.sqlite");
    assert!(Memory::create_schema(&path, "test", "CREATE TABLE invalid(").is_err());
    assert!(!path.exists());
    let db = Memory::create_host_capture(&path, "test").unwrap();
    assert_eq!(db.snapshot().unwrap().sequence, 0);
}

#[test]
fn safe_consulted_closure_inherits_scopes_and_forget_invalidates_dependents_only() {
    let (_path, mut db, _state) = fixture();
    let private = Access {
        domain: "test".into(),
        scopes: ["private".into()].into(),
    };
    let mut first = input();
    first.scopes = private.scopes.clone();
    let first = db.admit_host_capture(first, &private).unwrap();
    let mut second = input();
    second.identity.session_id = "second".into();
    second.originals[0].source.id = "second:1:text".into();
    second.consulted.push(first.projection.clone());
    let before = db.snapshot().unwrap().sequence;
    assert!(db.admit_host_capture(second.clone(), &access()).is_err());
    assert_eq!(before, db.snapshot().unwrap().sequence);
    let second = db.admit_host_capture(second, &private).unwrap();
    assert_eq!(
        super::sql::required_scopes(&db.conn, &second.projection.revision).unwrap(),
        private.scopes
    );
    assert!(db.read(&second.projection, &access()).unwrap().is_none());
    let mut unrelated = input();
    unrelated.identity.session_id = "unrelated".into();
    unrelated.originals[0].source.id = "unrelated:1:text".into();
    let unrelated = db.admit_host_capture(unrelated, &access()).unwrap();
    let Record::Evidence(source) = db.read(&first.originals[0], &private).unwrap().unwrap() else {
        panic!()
    };
    db.forget(&source.source).unwrap();
    for r in first
        .originals
        .iter()
        .chain([&first.projection])
        .chain(second.originals.iter())
        .chain([&second.projection])
    {
        assert!(db.read(r, &private).unwrap().is_none());
    }
    assert!(db.read(&unrelated.projection, &access()).unwrap().is_some());
    let count:i64=db.conn.query_row("SELECT count(*) FROM host_capture_receipt WHERE state='forgotten' AND payload_hash IS NULL AND receipt_id IS NULL AND projection_revision IS NULL AND originals IS NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
}
#[test]
fn transitive_unbound_descendant_and_context_bundle_share_external_authority() {
    let (_path, mut db, state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let entity = db.entity("synthetic entity", BTreeSet::new()).unwrap();
    // Fixture an existing descendant directly: unsupported producer APIs must
    // refuse creating one, but serving must still guard legacy descendants.
    let relation = RelationInput {
        predicate: "observed".into(),
        subject: entity.clone(),
        object: Target::Literal("CSV".into()),
        support: Span {
            evidence: receipt.originals[0].clone(),
            start: 0,
            end: 7,
        },
        consulted: vec![],
        text: "synthetic derived observation".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Unknown,
            to: Bound::Unknown,
        },
    };
    let derived = super::sql::append(
        &db.conn,
        None,
        &Record::Relation(relation),
        &BTreeSet::new(),
        &[entity, receipt.originals[0].clone()],
        "active",
    )
    .unwrap();
    assert!(db.read(&derived, &access()).unwrap().is_some());
    let request = context::ContextRequest {
        query: "synthetic".into(),
        bindings: vec![],
    };
    let packet = db
        .context(&request, &policy(&db, "synthetic"), &situation())
        .unwrap();
    assert!(packet.records.iter().any(|h| h.reference == derived));
    state.store(false, Ordering::SeqCst);
    assert!(db.read(&derived, &access()).unwrap().is_none());
    let packet = db
        .context(&request, &policy(&db, "synthetic"), &situation())
        .unwrap();
    assert!(packet.records.is_empty());
}

#[test]
fn projection_context_contains_complete_original_and_consulted_closure() {
    let (_path, mut db, _state) = fixture();
    let first = db.admit_host_capture(input(), &access()).unwrap();
    let mut second = input();
    second.identity.session_id = "second".into();
    second.originals[0].source.id = "second:1:text".into();
    second.consulted.push(first.projection.clone());
    second.criterion.criterion_text = "secondcriterionneedle".into();
    let second = db.admit_host_capture(second, &access()).unwrap();
    let mut p = policy(&db, "secondcriterionneedle");
    p.recall.limit = 1;
    p.recall.neighbors = 0;
    let packet = db
        .context(
            &context::ContextRequest {
                query: "secondcriterionneedle".into(),
                bindings: vec![],
            },
            &p,
            &situation(),
        )
        .unwrap();
    assert!(packet
        .records
        .iter()
        .any(|h| h.reference == second.projection));
    for reference in super::sql::closure(&db.conn, &second.projection).unwrap() {
        assert!(
            packet.records.iter().any(|h| h.reference == reference),
            "projection omitted dependency {reference:?}"
        );
    }
}
#[test]
fn projection_context_withholds_original_outside_occurrence_filter() {
    let (_path, mut db, _state) = fixture();
    let mut c = input();
    c.originals[0].occurred_at = Some("2026-11-01T00:00:00Z".into());
    c.occurred_at = Some("2026-11-07T00:00:00Z".into());
    db.admit_host_capture(c, &access()).unwrap();
    let mut p = policy(&db, "criterion_text");
    p.recall.limit = 1;
    p.recall.neighbors = 0;
    p.recall.occurred_after = Some("2026-11-06T00:00:00Z".into());
    let packet = db
        .context(
            &context::ContextRequest {
                query: "criterion_text".into(),
                bindings: vec![],
            },
            &p,
            &situation(),
        )
        .unwrap();
    assert!(
        packet.records.is_empty(),
        "filtered original text survived inside projection"
    );
}
#[test]
fn projection_context_withholds_consulted_closure_outside_account_filter() {
    let (_path, mut db, _state) = fixture();
    let mut first = input();
    first.identity.account = "other".into();
    first.originals[0].source.account = "other".into();
    let first = db.admit_host_capture(first, &access()).unwrap();
    let mut second = input();
    second.identity.session_id = "second".into();
    second.originals[0].source.id = "second:1:text".into();
    second.consulted.push(first.projection);
    db.admit_host_capture(second, &access()).unwrap();
    let mut p = policy(&db, "criterion_text");
    p.recall.limit = 1;
    p.recall.neighbors = 0;
    let packet = db
        .context(
            &context::ContextRequest {
                query: "criterion_text".into(),
                bindings: vec![],
            },
            &p,
            &situation(),
        )
        .unwrap();
    assert!(
        packet.records.is_empty(),
        "projection consulted another account outside policy"
    );
}
#[test]
fn projection_context_budget_cannot_return_partial_dependency_bundle() {
    let (_path, mut db, _state) = fixture();
    let receipt = db.admit_host_capture(input(), &access()).unwrap();
    let request = context::ContextRequest {
        query: "criterion_text".into(),
        bindings: vec![],
    };
    let mut p = policy(&db, "criterion_text");
    p.recall.limit = 1;
    p.recall.neighbors = 0;
    let mut singleton = db.context(&request, &p, &situation()).unwrap();
    singleton
        .records
        .retain(|h| h.reference == receipt.projection);
    assert_eq!(singleton.records.len(), 1);
    p.recall.max_bytes = serde_json::to_vec(&singleton).unwrap().len() + 1;
    let packet = db.context(&request, &p, &situation()).unwrap();
    assert!(
        packet.records.is_empty(),
        "budget returned a projection without its complete closure"
    );
    assert!(packet.budget_omitted);
}
#[test]
fn relation_revision_cannot_drop_authority_acquired_by_existing_support() {
    let (_path, mut db, state) = fixture();
    let c = input();
    let passage = &c.originals[0];
    let support = db
        .capture(Capture {
            source: passage.source.clone(),
            text: passage.text.clone(),
            occurred_at: passage.occurred_at.clone(),
            scopes: c.scopes.clone(),
            transcript: None,
        })
        .unwrap();
    let entity = db.entity("synthetic entity", BTreeSet::new()).unwrap();
    let mut relation = RelationInput {
        predicate: "observed".into(),
        subject: entity,
        object: Target::Literal("CSV".into()),
        support: Span {
            evidence: support.clone(),
            start: 0,
            end: 7,
        },
        consulted: vec![],
        text: "synthetic observation".into(),
        polarity: 1,
        valid: Interval {
            from: Bound::Unknown,
            to: Bound::Unknown,
        },
    };
    let guarded = db.relate(relation.clone()).unwrap();
    let receipt = db.admit_host_capture(c, &access()).unwrap();
    assert_eq!(receipt.originals[0], support);
    relation.support.evidence = db
        .capture(Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "test".into(),
                id: "replacement".into(),
            },
            text: "another source".into(),
            occurred_at: None,
            scopes: BTreeSet::new(),
            transcript: None,
        })
        .unwrap();
    state.store(false, Ordering::SeqCst);
    assert!(db.read(&guarded, &access()).unwrap().is_none());
    let before = db.snapshot().unwrap().sequence;
    assert!(
        db.revise_relation(&guarded, relation).is_err(),
        "revision removed prior relation's authority"
    );
    assert_eq!(db.snapshot().unwrap().sequence, before);
}

#[test]
fn consulted_authority_protects_each_original_not_only_projection() {
    struct SelectiveGuard(Arc<AtomicBool>);
    impl ExternalSourceGuard for SelectiveGuard {
        fn check(&self, binding: &ExternalBinding) -> SourceStatus {
            if binding.locator == "c".repeat(64) && !self.0.load(Ordering::SeqCst) {
                SourceStatus::Withheld
            } else {
                SourceStatus::Checked
            }
        }
    }
    let (path, db, state) = fixture();
    let mut db = db.with_source_guard(Arc::new(SelectiveGuard(state.clone())));
    let a = db.admit_host_capture(input(), &access()).unwrap();
    let mut b = input();
    b.identity.session_id = "second-session".into();
    b.binding.locator = "1".repeat(64);
    b.originals[0].source.id = "second-memory-result".into();
    b.originals[0].kind = PassageKind::ToolResult;
    b.originals[0].text = "Copied synthetic CSV evidence from first session".into();
    b.consulted = vec![a.projection.clone()];
    let b = db.admit_host_capture(b, &access()).unwrap();
    assert!(db.read(&b.originals[0], &access()).unwrap().is_some());
    state.store(false, Ordering::SeqCst);
    assert!(db.read(&b.projection, &access()).unwrap().is_none());
    assert!(
        db.read(&b.originals[0], &access()).unwrap().is_none(),
        "copied original must inherit consulted authority"
    );
    assert!(db
        .search(
            &Query {
                text: "Copied".into(),
                known_at: db.snapshot().unwrap(),
                valid_at: None,
                include_possible: false,
                limit: 100
            },
            &access()
        )
        .unwrap()
        .is_empty());
    drop(db);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn source_reuse_cannot_change_immutable_original_lineage() {
    let (_path, mut db, _state) = fixture();
    let a = db.admit_host_capture(input(), &access()).unwrap();
    let mut b = input();
    b.identity.session_id = "second-session".into();
    b.originals[0].source.id = "second-original".into();
    let first_b = db.admit_host_capture(b.clone(), &access()).unwrap();
    b.identity.prefix_digest = "1".repeat(64);
    b.consulted = vec![a.projection];
    let mut new_passage = b.originals[0].clone();
    new_passage.source.id = "must-roll-back".into();
    b.originals.insert(0, new_passage);
    let before = db.snapshot().unwrap().sequence;
    assert!(
        db.admit_host_capture(b, &access()).is_err(),
        "same original with changed lineage must refuse rather than mutate history"
    );
    assert_eq!(before, db.snapshot().unwrap().sequence);
    assert_eq!(
        super::sql::closure(&db.conn, &first_b.originals[0])
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn revised_original_cannot_consult_its_own_previous_capture() {
    let (_path, mut db, _state) = fixture();
    let a = db.admit_host_capture(input(), &access()).unwrap();
    let mut changed = input();
    changed.identity.prefix_digest = "1".repeat(64);
    changed.originals[0].text = "revised original".into();
    changed.consulted = vec![a.projection];
    let before = db.snapshot().unwrap().sequence;
    assert!(db.admit_host_capture(changed, &access()).is_err());
    assert_eq!(before, db.snapshot().unwrap().sequence);
    assert!(db.read(&a.originals[0], &access()).unwrap().is_some());
}

#[test]
fn unchanged_original_reuse_preserves_consulted_lineage() {
    let (_path, mut db, _state) = fixture();
    let a = db.admit_host_capture(input(), &access()).unwrap();
    let mut b = input();
    b.identity.session_id = "second-session".into();
    b.originals[0].source.id = "second-original".into();
    b.consulted = vec![a.projection];
    let first_b = db.admit_host_capture(b.clone(), &access()).unwrap();
    let closure = super::sql::closure(&db.conn, &first_b.originals[0]).unwrap();
    b.identity.prefix_digest = "1".repeat(64);
    let second_b = db.admit_host_capture(b, &access()).unwrap();
    assert_eq!(first_b.originals, second_b.originals);
    assert_eq!(
        closure,
        super::sql::closure(&db.conn, &second_b.originals[0]).unwrap()
    );
}

#[test]
fn pre_lineage_v6_files_refuse_instead_of_serving_old_copied_passages() {
    let dir = std::env::temp_dir().join(format!("bridge-old-format-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("old.memory-prototype.sqlite");
    let db = Memory::create(&path, "test").unwrap();
    db.conn.execute_batch("CREATE TABLE external_binding(revision TEXT PRIMARY KEY REFERENCES revision(id),authority TEXT NOT NULL,locator TEXT NOT NULL,binding_revision TEXT NOT NULL);
        CREATE TABLE host_capture_receipt(identity_hash TEXT PRIMARY KEY,payload_hash TEXT,receipt_id TEXT,projection_object TEXT,projection_revision TEXT,originals TEXT,state TEXT NOT NULL CHECK(state IN ('active','forgotten')));
        PRAGMA user_version=6;").unwrap();
    drop(db);
    assert!(
        Memory::open(&path).is_err(),
        "unsealed intermediate v6 must not be served"
    );
    assert!(Memory::open_read_only(&path).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
