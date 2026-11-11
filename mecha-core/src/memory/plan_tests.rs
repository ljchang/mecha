//! Authored fixtures for durable planning; runtime feedback semantics belong to the adapter.
use super::{plan::*, *};
use anyhow::Result;

fn access() -> Access {
    Access {
        domain: "plans".into(),
        scopes: ["private".into()].into(),
    }
}
fn fixture() -> Result<(storage_tests::KeyScratch, Memory, TaskSession, TaskRef)> {
    let scratch = storage_tests::KeyScratch::new()?;
    let storage =
        StorageAccess::generate_key_file(&scratch.keys.join("plan.key"), &scratch.policy)?;
    let path = scratch
        .keys
        .parent()
        .unwrap()
        .join("store/plan.memory-prototype.sqlite");
    let mut db = Memory::create_supported_situated_with_storage(&path, "plans", storage)?;
    db.enable_session_tasks()?;
    db.enable_session_plans()?;
    let session = db.task_session("owner", "20261110T120000-abcdef12", access())?;
    let task = db
        .task_create(
            &session,
            &TaskOperationId::new("create")?,
            TaskDraft {
                title: "Synthetic task".into(),
                intent: "Inspect authored fixture".into(),
            },
        )?
        .task;
    Ok((scratch, db, session, task))
}
fn request(turn: &str, ordinal: u32) -> Result<PlanCommitRequest> {
    Ok(PlanCommitRequest {
        occurrence: PlanOccurrence {
            turn_id: turn.into(),
            ordinal,
        },
        snapshot: PlanSnapshot {
            items: vec![PlanItem {
                content: "Inspect fixture".into(),
                status: PlanItemStatus::Pending,
                expect: None,
                check: None,
                expect_calls: None,
            }],
            serves: None,
            declarations: Default::default(),
        },
        goal_version: 1,
        goal_decision_digest: "a".repeat(64),
        goal_version_digest: "b".repeat(64),
        provider_call_id: "todo-call".into(),
        input_json: "{\"todos\":[]}".into(),
        result_text: "[ ] Inspect fixture".into(),
        feedback: StoredPlanFeedback::from_host_text("{\"steps\":[],\"decisions\":[]}".into())?,
    })
}
#[test]
fn plan_commits_replay_and_live_pending_siblings() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let permit = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    assert!(db.plan_read(&s, &t)?.is_none());
    let a = db.plan_commit(&s, &permit, &t, None, request("turn-one", 0)?)?;
    let b = db.plan_commit(
        &s,
        &permit,
        &t,
        Some(&a.view.reference),
        request("turn-one", 1)?,
    )?;
    let replay = db.plan_commit(&s, &permit, &t, None, request("turn-one", 0)?)?;
    assert!(replay.replayed);
    assert_eq!(replay.view, a.view);
    assert_ne!(a.view.reference, b.view.reference);
    let forged = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    assert!(db
        .plan_commit(
            &s,
            &forged,
            &t,
            Some(&b.view.reference),
            request("turn-one", 2)?
        )
        .is_err());
    permit.revoke()?;
    assert!(db
        .plan_commit(&s, &permit, &t, None, request("turn-one", 0)?)
        .is_err());
    Ok(())
}
#[test]
fn plan_feedback_preserves_exact_text_and_refuses_ambiguous_json() -> Result<()> {
    let text = "{\"steps\":[], \"score\":0.1,\"decisions\":[]}";
    let feedback = StoredPlanFeedback::from_host_text(text.into())?;
    assert_eq!(feedback.text(), text);
    for text in ["[]", "{\"x\":{\"a\":1,\"a\":2}}"] {
        assert!(StoredPlanFeedback::from_host_text(text.into()).is_err());
    }
    let mut value = serde_json::to_value(&feedback)?;
    value["digest"] = "0".repeat(64).into();
    let forged: StoredPlanFeedback = serde_json::from_value(value)?;
    assert!(forged.validate().is_err());
    Ok(())
}
#[test]
fn plan_pending_replay_requires_original_live_permit() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    db.plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?;
    let recreated = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    assert!(db
        .plan_commit(&s, &recreated, &t, None, request("turn-one", 0)?)
        .is_err());
    Ok(())
}
#[test]
fn plan_atomic_faults_preserve_inventory() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    for fault in [
        FailurePoint::AfterAppend,
        FailurePoint::BeforeReceipt,
        FailurePoint::BeforeCommit,
    ] {
        let before = recovery::inventory::inventory(&db.conn)?;
        assert!(db
            .write_plan(&s, &p, &t, None, request("turn-one", 0)?, fault)
            .is_err());
        assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
        assert!(db.plan_read(&s, &t)?.is_none());
    }
    db.plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?;
    Ok(())
}
#[test]
fn plan_audit_matches_exact_original_occurrences_atomically() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let attachment = "c".repeat(64);
    let p = PlanTurnPermit::from_host(&s, &t, &attachment, "turn-one")?;
    let a = db
        .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?
        .view;
    let b = db
        .plan_commit(&s, &p, &t, Some(&a.reference), request("turn-one", 1)?)?
        .view;
    let mut wrong = b.clone();
    wrong.value.request.result_text.push('!');
    let bad = VerifiedPlanAudit::from_host_verified(
        &s,
        &t,
        &attachment,
        "turn-one",
        vec![a.clone(), wrong],
    )?;
    assert!(db.plan_reconcile_audit(&s, &t, &bad).is_err());
    assert_eq!(db.plan_pending(&s, &t)?.len(), 2);
    for entries in [
        vec![b.clone(), b.clone()],
        vec![PlanView {
            reference: b.reference.clone(),
            ..a.clone()
        }],
    ] {
        let bad = VerifiedPlanAudit::from_host_verified(&s, &t, &attachment, "turn-one", entries)?;
        assert!(db.plan_reconcile_audit(&s, &t, &bad).is_err());
    }
    let proof = VerifiedPlanAudit::from_host_verified(&s, &t, &attachment, "turn-one", vec![a, b])?;
    db.plan_reconcile_audit(&s, &t, &proof)?;
    db.plan_reconcile_audit(&s, &t, &proof)?;
    assert!(db.plan_pending(&s, &t)?.is_empty());
    assert!(db.plan_read(&s, &t)?.unwrap().audited);
    Ok(())
}
#[test]
fn plan_two_handles_cas_and_conflicting_retry() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let mut other = Memory::open_with_storage(
        std::path::Path::new(db.conn.path().unwrap()),
        db.storage_access(),
    )?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    let a = db
        .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?
        .view;
    assert!(other
        .plan_commit(&s, &p, &t, None, request("turn-one", 1)?)
        .is_err());
    let mut changed = request("turn-one", 0)?;
    changed.result_text.push('!');
    assert!(other.plan_commit(&s, &p, &t, None, changed).is_err());
    assert!(other
        .plan_commit(&s, &p, &t, Some(&a.reference), request("turn-one", 0)?)
        .is_err());
    other.plan_commit(&s, &p, &t, Some(&a.reference), request("turn-one", 1)?)?;
    Ok(())
}
#[test]
fn plan_closed_forgotten_or_foreign_authority_refuses() -> Result<()> {
    for terminal in [false, true] {
        let (_scratch, mut db, s, t) = fixture()?;
        let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
        db.plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?;
        for (account, native, scopes) in [
            ("other", "20261110T120000-abcdef12", access().scopes),
            ("owner", "other-session", access().scopes),
            ("owner", "20261110T120000-abcdef12", Default::default()),
        ] {
            let foreign = db.task_session(account, native, Access { scopes, ..access() })?;
            assert!(db.plan_read(&foreign, &t).is_err());
            assert!(db.plan_pending(&foreign, &t).is_err());
            assert!(db
                .plan_commit(&foreign, &p, &t, None, request("turn-one", 0)?)
                .is_err());
        }
        assert!(db
            .task_session(
                "owner",
                "20261110T120000-abcdef12",
                Access {
                    domain: "foreign".into(),
                    ..access()
                }
            )
            .is_err());
        if terminal {
            db.task_set_status(&s, &TaskOperationId::new("close")?, &t, TaskStatus::Closed)?;
        } else {
            db.task_forget(&s, &t)?;
        }
        assert!(db
            .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)
            .is_err());
        assert!(db.plan_read(&s, &t).is_err());
    }
    Ok(())
}
#[test]
fn plan_declarations_survive_drop_restore_and_reject_frozen_changes() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    let mut initial = request("turn-one", 0)?;
    initial.snapshot.items[0].check = Some("test fixture".into());
    initial.snapshot.declarations.insert(
        "Inspect fixture".into(),
        PlanCheckDeclaration {
            check: "test fixture".into(),
            frozen: false,
        },
    );
    let a = db.plan_commit(&s, &p, &t, None, initial.clone())?.view;
    let mut dropped = initial.clone();
    dropped.occurrence.ordinal = 1;
    dropped.snapshot.items.clear();
    let b = db
        .plan_commit(&s, &p, &t, Some(&a.reference), dropped.clone())?
        .view;
    let path = db.conn.path().unwrap().to_string();
    let storage = db.storage_access();
    drop(db);
    let mut db = Memory::open_with_storage(std::path::Path::new(&path), storage)?;
    assert_eq!(
        db.plan_read(&s, &t)?.unwrap().value.request.snapshot,
        dropped.snapshot
    );
    let mut completed = initial;
    completed.occurrence.ordinal = 2;
    completed.snapshot.items[0].status = PlanItemStatus::Completed;
    completed
        .snapshot
        .declarations
        .get_mut("Inspect fixture")
        .unwrap()
        .frozen = true;
    let c = db
        .plan_commit(&s, &p, &t, Some(&b.reference), completed.clone())?
        .view;
    completed.occurrence.ordinal = 3;
    completed.snapshot.items.clear();
    completed.snapshot.declarations.clear();
    assert!(db
        .plan_commit(&s, &p, &t, Some(&c.reference), completed)
        .is_err());
    Ok(())
}
#[test]
fn plan_strict_bounds_and_duplicate_nested_fields_refuse() -> Result<()> {
    let r = request("turn-one", 0)?;
    let text = serde_json::to_string(&r.snapshot)?;
    assert!(serde_json::from_str::<PlanSnapshot>(&text.replacen(
        "\"items\":",
        "\"unknown\":0,\"items\":",
        1
    ))
    .is_err());
    assert!(serde_json::from_str::<PlanSnapshot>(&text.replacen(
        "\"content\":",
        "\"content\":\"duplicate\",\"content\":",
        1
    ))
    .is_err());
    for mode in 0..6 {
        let mut snapshot = r.snapshot.clone();
        match mode {
            0 => snapshot.items.push(snapshot.items[0].clone()),
            1 => snapshot.items[0].content = "x".repeat(513),
            2 => snapshot.items[0].expect = Some("x".repeat(513)),
            3 => {
                snapshot.items = (0..33)
                    .map(|n| PlanItem {
                        content: format!("step {n}"),
                        ..snapshot.items[0].clone()
                    })
                    .collect()
            }
            4 => {
                snapshot.declarations = (0..257)
                    .map(|n| {
                        (
                            format!("step {n}"),
                            PlanCheckDeclaration {
                                check: "test".into(),
                                frozen: false,
                            },
                        )
                    })
                    .collect()
            }
            _ => {
                snapshot.declarations = (0..100)
                    .map(|n| {
                        (
                            format!("step {n}"),
                            PlanCheckDeclaration {
                                check: "x".repeat(512),
                                frozen: false,
                            },
                        )
                    })
                    .collect()
            }
        };
        assert!(snapshot.validate().is_err());
    }
    Ok(())
}
#[test]
fn plan_corrupt_ledger_or_canonical_payload_refuses_open() -> Result<()> {
    for corruption in [
  "UPDATE plan_operation SET payload_hash=printf('%064d',0)",
  "UPDATE plan_operation SET ordinal=2",
  "UPDATE plan_operation SET turn_id='other'",
  "UPDATE plan_operation SET attachment_digest=printf('%064d',0)",
  "DELETE FROM plan_operation",
  "UPDATE revision SET payload=json_set(payload,'$.data.request.goal_version',2) WHERE object_id IN (SELECT plan_object FROM plan_operation)",
  "UPDATE revision SET payload=replace(payload,'\"content\":','\"content\":\"shadow\",\"content\":') WHERE object_id IN (SELECT plan_object FROM plan_operation)",
  "DELETE FROM dependency WHERE output IN (SELECT after_revision FROM plan_operation)",
 ] {
  let (_scratch,mut db,s,t)=fixture()?;let p=PlanTurnPermit::from_host(&s,&t,&"c".repeat(64),"turn-one")?;
  db.plan_commit(&s,&p,&t,None,request("turn-one",0)?)?;db.conn.execute_batch(corruption)?;
  assert!(Memory::open_with_storage(std::path::Path::new(db.conn.path().unwrap()),db.storage_access()).is_err(),"{corruption}");
 }
    Ok(())
}
#[test]
fn plan_forgetting_purges_content_and_scrubs_receipts_and_generic_closure() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    let a = db
        .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?
        .view;
    let indirect = sql::append(
        &db.conn,
        None,
        &Record::Evidence(Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "owner".into(),
                id: "indirect".into(),
            },
            text: "Inspect fixture indirect".into(),
            occurred_at: None,
            scopes: access().scopes,
            transcript: None,
        }),
        &access().scopes,
        std::slice::from_ref(&a.reference),
        "active",
    )?;
    assert!(db.read(&a.reference, &access())?.is_none());
    assert!(db.read(&indirect, &access())?.is_none());
    assert!(db
        .search(
            &Query {
                text: "Inspect".into(),
                known_at: db.snapshot()?,
                valid_at: None,
                include_possible: true,
                limit: 100
            },
            &access()
        )?
        .is_empty());
    assert!(db
        .recall(
            &RecallQuery {
                text: "Inspect".into(),
                known_at: db.snapshot()?,
                account: None,
                session_id: None,
                occurred_after: None,
                occurred_before: None,
                limit: 10,
                neighbors: 2,
                max_bytes: 20000,
            },
            &access()
        )?
        .is_empty());
    let report = db.task_forget(&s, &t)?;
    assert_eq!(report.objects, 3);
    assert!(sql::lookup(&db.conn, &a.reference)?.is_none());
    assert!(sql::lookup(&db.conn, &indirect)?.is_none());
    let state: String = db
        .conn
        .query_row("SELECT state FROM plan_operation", [], |r| r.get(0))?;
    assert_eq!(state, "forgotten");
    plan::validate_schema(&db.conn)?;
    assert!(db
        .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)
        .is_err());
    Ok(())
}
#[test]
fn plan_explicit_keyed_migration_is_atomic_and_never_on_open() -> Result<()> {
    let scratch = storage_tests::KeyScratch::new()?;
    let storage =
        StorageAccess::generate_key_file(&scratch.keys.join("migration.key"), &scratch.policy)?;
    let path = scratch
        .keys
        .parent()
        .unwrap()
        .join("store/migration.memory-prototype.sqlite");
    let mut db = Memory::create_supported_situated_with_storage(&path, "plans", storage.clone())?;
    assert!(db.enable_session_plans().is_err());
    assert_eq!(task::version(&db.conn)?, 8);
    db.enable_session_tasks()?;
    drop(db);
    let mut db = Memory::open_with_storage(&path, storage.clone())?;
    assert_eq!(task::version(&db.conn)?, 9);
    let before = recovery::inventory::inventory(&db.conn)?;
    assert!(db.migrate_plans(FailurePoint::Migration).is_err());
    assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
    db.enable_session_plans()?;
    assert_eq!(task::version(&db.conn)?, 10);
    assert!(db.enable_session_plans().is_err());
    drop(db);
    let db = Memory::open_with_storage(&path, storage)?;
    assert_eq!(task::version(&db.conn)?, 10);
    let bytes = std::fs::read(path)?;
    assert!(!bytes.windows(6).any(|w| w == b"SQLite"));
    Ok(())
}
#[test]
fn plan_encrypted_snapshot_inventory_and_restore_are_historical() -> Result<()> {
    for forget in [false, true] {
        let (scratch, mut db, s, t) = fixture()?;
        let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
        let a = db
            .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?
            .view;
        let source = std::path::PathBuf::from(db.conn.path().unwrap());
        let backup = source.with_file_name("backup.memory-prototype.sqlite");
        let restored = source.with_file_name("restored.memory-prototype.sqlite");
        let bk =
            StorageAccess::generate_key_file(&scratch.keys.join("backup.key"), &scratch.policy)?;
        let rk =
            StorageAccess::generate_key_file(&scratch.keys.join("restore.key"), &scratch.policy)?;
        assert_eq!(db.backup_encrypted(&backup, bk.clone())?.schema_version, 10);
        let copy = Memory::open_with_storage(&backup, bk.clone())?;
        assert_eq!(
            recovery::inventory::inventory(&db.conn)?,
            recovery::inventory::inventory(&copy.conn)?
        );
        drop(copy);
        if forget {
            db.task_forget(&s, &t)?;
        }
        let report = restore_encrypted(
            &backup,
            bk,
            &restored,
            rk.clone(),
            &RestoreAuthority::current(&source, db.storage_access()),
        )?;
        assert_eq!(report.schema_version, 10);
        let copy = Memory::open_with_storage(&restored, rk)?;
        assert!(copy
            .task_session("owner", "20261110T120000-abcdef12", access())
            .is_err());
        assert!(copy.plan_read(&s, &t).is_err());
        assert!(copy.plan_pending(&s, &t).is_err());
        if forget {
            assert!(sql::lookup(&copy.conn, &a.reference)?.is_none());
            assert_eq!(report.purged_objects, 2);
        } else {
            assert!(sql::lookup(&copy.conn, &a.reference)?.is_some());
        }
    }
    Ok(())
}
#[test]
fn plan_snapshot_duplicate_declaration_keys_refuse_typed_decode() {
    let text = r#"{"items":[],"serves":null,"declarations":{"step":{"check":"one","frozen":false},"step":{"check":"two","frozen":false}}}"#;
    assert!(serde_json::from_str::<PlanSnapshot>(text).is_err());
}
#[test]
fn plan_occurrences_cannot_commit_out_of_authored_order() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let p = PlanTurnPermit::from_host(&s, &t, &"c".repeat(64), "turn-one")?;
    let a = db
        .plan_commit(&s, &p, &t, None, request("turn-one", 2)?)?
        .view;
    assert!(db
        .plan_commit(&s, &p, &t, Some(&a.reference), request("turn-one", 1)?)
        .is_err());
    Ok(())
}
#[test]
fn plan_schema_getter_validates_store_without_migrating() -> Result<()> {
    let (_scratch, db, _s, _t) = fixture()?;
    assert_eq!(db.session_schema_version()?, 10);
    db.conn.execute_batch("DROP TABLE plan_operation")?;
    assert!(db.session_schema_version().is_err());
    Ok(())
}

fn checked_request(ordinal: u32, check: &str, completed: bool) -> Result<PlanCommitRequest> {
    let mut value = request("turn-one", ordinal)?;
    value.snapshot.items[0].check = Some(check.into());
    if completed {
        value.snapshot.items[0].status = PlanItemStatus::Completed;
    }
    value.snapshot.declarations.insert(
        "Inspect fixture".into(),
        PlanCheckDeclaration {
            check: check.into(),
            frozen: completed,
        },
    );
    Ok(value)
}

fn refuse_completing_check_replacement(dropped: bool) -> Result<()> {
    let (_scratch, mut db, session, task) = fixture()?;
    let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
    let original = checked_request(0, "test fixture", false)?;
    let mut head = db
        .plan_commit(&session, &permit, &task, None, original.clone())?
        .view;
    if dropped {
        let mut removed = original;
        removed.occurrence.ordinal = 1;
        removed.snapshot.items.clear();
        head = db
            .plan_commit(&session, &permit, &task, Some(&head.reference), removed)?
            .view;
    }
    let before = recovery::inventory::inventory(&db.conn)?;
    let changed = checked_request(2, "true", true)?;
    assert!(db
        .plan_commit(&session, &permit, &task, Some(&head.reference), changed)
        .is_err());
    assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
    assert_eq!(db.plan_read(&session, &task)?.unwrap(), head);
    let completed = db.plan_commit(
        &session,
        &permit,
        &task,
        Some(&head.reference),
        checked_request(2, "test fixture", true)?,
    )?;
    assert!(completed.view.value.request.snapshot.declarations["Inspect fixture"].frozen);
    Ok(())
}

#[test]
fn plan_completion_preserves_last_open_check() -> Result<()> {
    refuse_completing_check_replacement(false)
}

#[test]
fn plan_completion_preserves_dropped_open_check_when_readded() -> Result<()> {
    refuse_completing_check_replacement(true)
}

#[test]
fn plan_completion_guard_preserves_open_edits_and_explicit_withdrawal() -> Result<()> {
    let (_scratch, mut db, session, task) = fixture()?;
    let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
    let a = db
        .plan_commit(
            &session,
            &permit,
            &task,
            None,
            checked_request(0, "first check", false)?,
        )?
        .view;
    let b = db
        .plan_commit(
            &session,
            &permit,
            &task,
            Some(&a.reference),
            checked_request(1, "revised check", false)?,
        )?
        .view;
    assert_eq!(
        b.value.request.snapshot.declarations["Inspect fixture"].check,
        "revised check"
    );
    let c = db
        .plan_commit(
            &session,
            &permit,
            &task,
            Some(&b.reference),
            request("turn-one", 2)?,
        )?
        .view;
    assert!(c.value.request.snapshot.declarations.is_empty());
    assert!(c.value.request.snapshot.items[0].check.is_none());
    plan::validate_schema(&db.conn)?;
    Ok(())
}

#[test]
fn plan_completion_guard_rejects_coherent_historical_replacement() -> Result<()> {
    use sha2::{Digest, Sha256};
    for dropped in [false, true] {
        let (_scratch, mut db, session, task) = fixture()?;
        let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
        let original = checked_request(0, "test fixture", false)?;
        let mut head = db
            .plan_commit(&session, &permit, &task, None, original.clone())?
            .view;
        if dropped {
            let mut removed = original;
            removed.occurrence.ordinal = 1;
            removed.snapshot.items.clear();
            head = db
                .plan_commit(&session, &permit, &task, Some(&head.reference), removed)?
                .view;
        }
        let completed = db
            .plan_commit(
                &session,
                &permit,
                &task,
                Some(&head.reference),
                checked_request(2, "test fixture", true)?,
            )?
            .view;
        // Simulate a coherently rewritten historical payload and receipt digest:
        // only transition semantics, not the canonical-envelope hash, can refuse it.
        let changed = checked_request(2, "true", true)?;
        let payload_hash: String = Sha256::digest(serde_json::to_vec(&(
            "session-plan-request-v1",
            &task,
            Some(&head.reference),
            &changed,
        ))?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
        db.conn.execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&Record::Plan(PlanRevision {
                    task: task.clone(),
                    request: changed
                }))?,
                completed.reference.revision,
            ],
        )?;
        db.conn.execute(
            "UPDATE plan_operation SET payload_hash=?1 WHERE after_revision=?2",
            rusqlite::params![payload_hash, completed.reference.revision],
        )?;
        assert!(plan::validate_schema(&db.conn).is_err());
        assert!(Memory::open_with_storage(
            std::path::Path::new(db.conn.path().unwrap()),
            db.storage_access()
        )
        .is_err());
    }
    Ok(())
}

fn refuse_absent_declaration_mutation(freeze: bool) -> Result<()> {
    for already_dropped in [false, true] {
        let (_scratch, mut db, session, task) = fixture()?;
        let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
        let original = checked_request(0, "test fixture", false)?;
        let mut head = db
            .plan_commit(&session, &permit, &task, None, original.clone())?
            .view;
        let mut dropped = original;
        dropped.snapshot.items.clear();
        if already_dropped {
            dropped.occurrence.ordinal = 1;
            head = db
                .plan_commit(
                    &session,
                    &permit,
                    &task,
                    Some(&head.reference),
                    dropped.clone(),
                )?
                .view;
        }
        dropped.occurrence.ordinal = 2;
        let mut forged = dropped.clone();
        let declaration = forged
            .snapshot
            .declarations
            .get_mut("Inspect fixture")
            .unwrap();
        if freeze {
            declaration.frozen = true;
        } else {
            declaration.check = "true".into();
        }
        let before = recovery::inventory::inventory(&db.conn)?;
        assert!(db
            .plan_commit(&session, &permit, &task, Some(&head.reference), forged)
            .is_err());
        assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
        // Legitimate absence carries the exact declaration; re-added completion
        // still freezes the actual last check rather than the rejected replacement.
        let kept = db
            .plan_commit(&session, &permit, &task, Some(&head.reference), dropped)?
            .view;
        db.plan_commit(
            &session,
            &permit,
            &task,
            Some(&kept.reference),
            checked_request(3, "test fixture", true)?,
        )?;
    }
    Ok(())
}
#[test]
fn plan_absent_declaration_cannot_replace_check_during_or_after_drop() -> Result<()> {
    refuse_absent_declaration_mutation(false)
}
#[test]
fn plan_absent_declaration_cannot_freeze_during_or_after_drop() -> Result<()> {
    refuse_absent_declaration_mutation(true)
}

fn refuse_new_phantom_declaration(initial: bool) -> Result<()> {
    let (_scratch, mut db, session, task) = fixture()?;
    let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
    let head = if initial {
        None
    } else {
        Some(
            db.plan_commit(&session, &permit, &task, None, request("turn-one", 0)?)?
                .view
                .reference,
        )
    };
    let mut forged = request("turn-one", 1)?;
    forged.snapshot.declarations.insert(
        "Absent step".into(),
        PlanCheckDeclaration {
            check: "true".into(),
            frozen: false,
        },
    );
    let before = recovery::inventory::inventory(&db.conn)?;
    assert!(db
        .plan_commit(&session, &permit, &task, head.as_ref(), forged)
        .is_err());
    assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
    Ok(())
}
#[test]
fn plan_absent_declaration_cannot_originate_in_initial_plan() -> Result<()> {
    refuse_new_phantom_declaration(true)
}
#[test]
fn plan_absent_declaration_cannot_originate_in_later_plan() -> Result<()> {
    refuse_new_phantom_declaration(false)
}

#[test]
fn plan_open_declaration_cannot_gain_frozen_state_without_completion() -> Result<()> {
    for existing in [false, true] {
        let (_scratch, mut db, session, task) = fixture()?;
        let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
        let head = if existing {
            Some(
                db.plan_commit(
                    &session,
                    &permit,
                    &task,
                    None,
                    checked_request(0, "test fixture", false)?,
                )?
                .view
                .reference,
            )
        } else {
            None
        };
        let mut forged = checked_request(1, "test fixture", false)?;
        forged
            .snapshot
            .declarations
            .get_mut("Inspect fixture")
            .unwrap()
            .frozen = true;
        let before = recovery::inventory::inventory(&db.conn)?;
        assert!(db
            .plan_commit(&session, &permit, &task, head.as_ref(), forged)
            .is_err());
        assert_eq!(before, recovery::inventory::inventory(&db.conn)?);
        let complete = db
            .plan_commit(
                &session,
                &permit,
                &task,
                head.as_ref(),
                checked_request(1, "test fixture", true)?,
            )?
            .view;
        let mut reopened = complete.value.request;
        reopened.occurrence.ordinal = 2;
        reopened.snapshot.items[0].status = PlanItemStatus::InProgress;
        let open = db
            .plan_commit(
                &session,
                &permit,
                &task,
                Some(&complete.reference),
                reopened,
            )?
            .view;
        assert!(open.value.request.snapshot.declarations["Inspect fixture"].frozen);
        plan::validate_schema(&db.conn)?;
    }
    Ok(())
}

#[test]
fn plan_absent_and_open_declaration_history_requires_real_origin() -> Result<()> {
    use sha2::{Digest, Sha256};
    for mode in 0..5 {
        let (_scratch, mut db, session, task) = fixture()?;
        let permit = PlanTurnPermit::from_host(&session, &task, &"c".repeat(64), "turn-one")?;
        let before = if mode == 0 || mode == 4 {
            None
        } else {
            Some(
                db.plan_commit(
                    &session,
                    &permit,
                    &task,
                    None,
                    checked_request(0, "test fixture", false)?,
                )?
                .view
                .reference,
            )
        };
        let mut valid = checked_request(1, "test fixture", false)?;
        if mode == 2 || mode == 3 {
            valid.snapshot.items.clear();
        }
        let committed = db
            .plan_commit(&session, &permit, &task, before.as_ref(), valid.clone())?
            .view;
        let mut forged = valid;
        match mode {
            0 | 1 => {
                forged.snapshot.declarations.insert(
                    "Absent step".into(),
                    PlanCheckDeclaration {
                        check: "true".into(),
                        frozen: false,
                    },
                );
            }
            2 => {
                forged
                    .snapshot
                    .declarations
                    .get_mut("Inspect fixture")
                    .unwrap()
                    .check = "true".into()
            }
            _ => {
                forged
                    .snapshot
                    .declarations
                    .get_mut("Inspect fixture")
                    .unwrap()
                    .frozen = true
            }
        }
        let hash: String = Sha256::digest(serde_json::to_vec(&(
            "session-plan-request-v1",
            &task,
            before.as_ref(),
            &forged,
        ))?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
        db.conn.execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&Record::Plan(PlanRevision {
                    task: task.clone(),
                    request: forged
                }))?,
                committed.reference.revision
            ],
        )?;
        db.conn.execute(
            "UPDATE plan_operation SET payload_hash=?1 WHERE after_revision=?2",
            rusqlite::params![hash, committed.reference.revision],
        )?;
        assert!(
            plan::validate_schema(&db.conn).is_err(),
            "forged history mode {mode}"
        );
        assert!(Memory::open_with_storage(
            std::path::Path::new(db.conn.path().unwrap()),
            db.storage_access()
        )
        .is_err());
    }
    Ok(())
}

#[test]
fn plan_turn_inventory_retains_original_audited_siblings() -> Result<()> {
    let (_scratch, mut db, s, t) = fixture()?;
    let attachment = "c".repeat(64);
    let p = PlanTurnPermit::from_host(&s, &t, &attachment, "turn-one")?;
    let a = db
        .plan_commit(&s, &p, &t, None, request("turn-one", 0)?)?
        .view;
    let b = db
        .plan_commit(&s, &p, &t, Some(&a.reference), request("turn-one", 1)?)?
        .view;
    let proof =
        VerifiedPlanAudit::from_host_verified(&s, &t, &attachment, "turn-one", vec![a.clone()])?;
    db.plan_reconcile_audit(&s, &t, &proof)?;
    let entries = db.plan_turn_entries(&s, &t, &attachment, "turn-one")?;
    assert_eq!(
        entries.len(),
        2,
        "recovery needs audited and pending originals"
    );
    assert_eq!(entries[0].reference, a.reference);
    assert_eq!(entries[1].reference, b.reference);
    assert!(entries[0].audited && !entries[1].audited);
    let proof = VerifiedPlanAudit::from_host_verified(&s, &t, &attachment, "turn-one", entries)?;
    db.plan_reconcile_audit(&s, &t, &proof)?;
    let audited = db.plan_turn_entries(&s, &t, &attachment, "turn-one")?;
    assert!(audited.iter().all(|entry| entry.audited));
    assert!(db
        .plan_turn_entries(&s, &t, &attachment, "empty-turn")?
        .is_empty());
    assert!(db
        .plan_turn_entries(&s, &t, &"d".repeat(64), "turn-one")
        .is_err());
    for (account, native, scopes) in [
        ("other", "20261110T120000-abcdef12", access().scopes),
        ("owner", "foreign", access().scopes),
        ("owner", "20261110T120000-abcdef12", Default::default()),
    ] {
        let foreign = db.task_session(account, native, Access { scopes, ..access() })?;
        assert!(db
            .plan_turn_entries(&foreign, &t, &attachment, "turn-one")
            .is_err());
    }
    let (_other_scratch, other_db, other_session, other_task) = fixture()?;
    assert!(db
        .plan_turn_entries(&other_session, &t, &attachment, "turn-one")
        .is_err());
    assert!(db
        .plan_turn_entries(&s, &other_task, &attachment, "turn-one")
        .is_err());
    assert!(other_db
        .plan_turn_entries(&s, &t, &attachment, "turn-one")
        .is_err());
    let next = PlanTurnPermit::from_host(&s, &t, &attachment, "turn-two")?;
    db.plan_commit(&s, &next, &t, Some(&b.reference), request("turn-two", 0)?)?;
    assert_eq!(
        db.plan_turn_entries(&s, &t, &attachment, "turn-one")?,
        audited
    );
    assert_eq!(
        db.plan_turn_entries(&s, &t, &attachment, "turn-two")?.len(),
        1
    );
    db.task_forget(&s, &t)?;
    assert!(db
        .plan_turn_entries(&s, &t, &attachment, "turn-one")
        .is_err());
    Ok(())
}
