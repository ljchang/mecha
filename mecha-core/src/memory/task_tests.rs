//! Authored encrypted fixtures only. These began as compile scaffolds, not behavioral RED.
use super::*;
use anyhow::Result;
use std::path::PathBuf;

struct Fixture {
    scratch: storage_tests::KeyScratch,
    path: PathBuf,
    storage: StorageAccess,
    db: Memory,
}
impl Fixture {
    fn v8() -> Result<Self> {
        let scratch = storage_tests::KeyScratch::new()?;
        let path = scratch
            .keys
            .parent()
            .unwrap()
            .join("store/tasks.memory-prototype.sqlite");
        let storage =
            StorageAccess::generate_key_file(&scratch.keys.join("task.key"), &scratch.policy)?;
        let db =
            Memory::create_supported_situated_with_storage(&path, "task-fixture", storage.clone())?;
        Ok(Self {
            scratch,
            path,
            storage,
            db,
        })
    }
    fn new() -> Result<Self> {
        let mut f = Self::v8()?;
        f.db.enable_session_tasks()?;
        Ok(f)
    }
    fn session(&self) -> Result<TaskSession> {
        self.db
            .task_session("owner", "20261110T120000-abcdef12", access())
    }
    fn output(&self, name: &str) -> PathBuf {
        self.path
            .parent()
            .unwrap()
            .join(format!("{name}.memory-prototype.sqlite"))
    }
    fn key(&self, name: &str) -> Result<StorageAccess> {
        StorageAccess::generate_key_file(&self.scratch.keys.join(name), &self.scratch.policy)
    }
}
fn access() -> Access {
    Access {
        domain: "task-fixture".into(),
        scopes: ["private".into()].into(),
    }
}
fn draft() -> TaskDraft {
    TaskDraft {
        title: "Authored task".into(),
        intent: "Compare synthetic amber fixtures".into(),
    }
}
fn op(id: &str) -> TaskOperationId {
    TaskOperationId::new(id).unwrap()
}
fn inventory(db: &Memory) -> Result<recovery::inventory::Inventory> {
    recovery::inventory::inventory(&db.conn)
}

#[test]
fn task_reopen_preserves_typed_revision() -> Result<()> {
    for terminal in [TaskStatus::Closed, TaskStatus::Cancelled] {
        let mut f = Fixture::new()?;
        let s = f.session()?;
        let first = f.db.task_create(&s, &op("create"), draft())?;
        let next = f.db.task_revise(
            &s,
            &op("revise"),
            &first.task,
            TaskDraft {
                title: "Revised authored task".into(),
                ..draft()
            },
        )?;
        let last =
            f.db.task_set_status(&s, &op("terminal"), &next.task, terminal)?;
        let historical = sql::lookup(&f.db.conn, &first.task.reference)?.unwrap();
        drop(f.db);
        let mut db = Memory::open_with_storage(&f.path, f.storage.clone())?;
        assert_eq!(db.task_read(&s, &last.task)?.value.status, terminal);
        sql::current(&db.conn, &last.task.reference)?;
        assert_eq!(
            sql::head(&db.conn, &last.task.reference.object, i64::MAX)?
                .unwrap()
                .2,
            "active"
        );
        assert_eq!(
            serde_json::to_vec(&historical)?,
            serde_json::to_vec(&sql::lookup(&db.conn, &first.task.reference)?.unwrap())?
        );
        let before = inventory(&db)?;
        assert!(db
            .task_revise(&s, &op("late-edit"), &last.task, draft())
            .is_err());
        assert!(db
            .task_set_status(&s, &op("reopen"), &last.task, TaskStatus::Open)
            .is_err());
        assert_eq!(before, inventory(&db)?);
    }
    Ok(())
}

#[test]
fn task_binding_checks_account_session_domain_scopes() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let before = inventory(&f.db)?;
    for other in [
        f.db.task_session("other", "20261110T120000-abcdef12", access())?,
        f.db.task_session("owner", "20261110T120001-abcdef12", access())?,
        f.db.task_session(
            "owner",
            "20261110T120000-abcdef12",
            Access {
                scopes: Default::default(),
                ..access()
            },
        )?,
    ] {
        assert!(f.db.task_read(&other, &first.task).is_err());
        assert!(f
            .db
            .task_revise(&other, &op("edit"), &first.task, draft())
            .is_err());
        assert!(f
            .db
            .task_set_status(&other, &op("status"), &first.task, TaskStatus::Active)
            .is_err());
        assert!(f.db.task_forget(&other, &first.task).is_err());
    }
    assert!(f
        .db
        .task_session(
            "owner",
            "20261110T120000-abcdef12",
            Access {
                domain: "other".into(),
                ..access()
            }
        )
        .is_err());
    let mut substituted = first.task.clone();
    substituted.reference.object = "unrelated-object".into();
    assert!(f.db.task_read(&s, &substituted).is_err());
    assert_eq!(before, inventory(&f.db)?);
    assert!(f.db.read(&first.task.reference, &access())?.is_none());
    for text in ["", "amber"] {
        assert!(f
            .db
            .search(
                &Query {
                    text: text.into(),
                    known_at: f.db.snapshot()?,
                    valid_at: None,
                    include_possible: true,
                    limit: 100
                },
                &access()
            )?
            .is_empty());
    }
    assert_eq!(
        f.db.task_read(&s, &first.task)?.value.intent,
        draft().intent
    );
    Ok(())
}

#[test]
fn task_cas_across_handles() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let mut second = Memory::open_with_storage(&f.path, f.storage.clone())?;
    second.task_read(&s, &first.task)?;
    let next =
        f.db.task_set_status(&s, &op("first-writer"), &first.task, TaskStatus::Active)?;
    let before = inventory(&f.db)?;
    assert!(second
        .task_revise(&s, &op("second-writer"), &first.task, draft())
        .is_err());
    assert!(second.task_read(&s, &first.task).is_err());
    assert_eq!(
        second.task_read(&s, &next.task)?.value.status,
        TaskStatus::Active
    );
    assert_eq!(before, inventory(&f.db)?);
    Ok(())
}

#[test]
fn task_operation_replay_is_exact() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let next =
        f.db.task_set_status(&s, &op("status"), &first.task, TaskStatus::Active)?;
    let before = inventory(&f.db)?;
    let mut reopened = Memory::open_with_storage(&f.path, f.storage.clone())?;
    assert_eq!(first, reopened.task_create(&s, &op("create"), draft())?);
    assert_eq!(
        next,
        reopened.task_set_status(&s, &op("status"), &first.task, TaskStatus::Active)?
    );
    assert!(reopened
        .task_set_status(&s, &op("status"), &next.task, TaskStatus::Active)
        .is_err());
    assert!(reopened
        .task_create(
            &s,
            &op("create"),
            TaskDraft {
                title: "Conflict".into(),
                ..draft()
            }
        )
        .is_err());
    assert_eq!(before, inventory(&f.db)?);
    f.db.conn.execute(
        "INSERT INTO scope VALUES(?1,'tightened')",
        [&next.task.reference.revision],
    )?;
    assert!(reopened.task_create(&s, &op("create"), draft()).is_err());
    Ok(())
}

#[test]
fn task_commit_failure_is_atomic() -> Result<()> {
    for point in [
        task::FailurePoint::AfterAppend,
        task::FailurePoint::AfterReceipt,
    ] {
        let mut f = Fixture::new()?;
        let s = f.session()?;
        let before = inventory(&f.db)?;
        assert!(f
            .db
            .task_create_failing(&s, &op("create"), draft(), point)
            .is_err());
        assert_eq!(before, inventory(&f.db)?);
        let mut reopened = Memory::open_with_storage(&f.path, f.storage.clone())?;
        let receipt = reopened.task_create(&s, &op("create"), draft())?;
        assert_eq!(receipt.commit, TaskCommit::DatabaseOnly);
        assert_eq!(receipt, reopened.task_create(&s, &op("create"), draft())?);
    }
    Ok(())
}

#[test]
fn task_forget_erases_content_and_suppresses_retry() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let next =
        f.db.task_set_status(&s, &op("status"), &first.task, TaskStatus::Active)?;
    let report = f.db.task_forget(&s, &next.task)?;
    assert_eq!(report.revisions, 2);
    assert!(sql::lookup(&f.db.conn, &first.task.reference)?.is_none());
    assert!(f.db.task_create(&s, &op("create"), draft()).is_err());
    assert!(f
        .db
        .task_set_status(&s, &op("status"), &first.task, TaskStatus::Active)
        .is_err());
    assert_eq!(f.db.conn.query_row("SELECT count(*) FROM task_operation WHERE state='forgotten' AND payload_hash IS NULL AND receipt_id IS NULL AND task_object IS NULL AND before_revision IS NULL AND after_revision IS NULL AND operation IS NULL", [], |r| r.get::<_, i64>(0))?, 2);
    assert!(f.db.task_forget(&s, &next.task).is_err());
    f.db.task_create(&s, &op("new-explicit-create"), draft())?;
    sql::check_identity(&f.db.conn)?;
    Ok(())
}

#[test]
fn task_no_dependency_cycle() -> Result<()> {
    assert!(
        serde_json::from_str::<TaskDraft>(r#"{"title":"x","intent":"y","dependencies":[]}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<TaskDraft>(r#"{"title":"x","intent":"y","outcome":"done"}"#)
            .is_err()
    );
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let value = f.db.task_read(&s, &first.task)?.value;
    let descendant = sql::append(
        &f.db.conn,
        None,
        &Record::Entity {
            label: "Synthetic descendant".into(),
        },
        &access().scopes,
        std::slice::from_ref(&first.task.reference),
        "active",
    )?;
    let before = inventory(&f.db)?;
    assert!(sql::append(
        &f.db.conn,
        Some(&first.task.reference),
        &Record::Task(value),
        &access().scopes,
        &[descendant],
        "active"
    )
    .is_err());
    assert_eq!(before, inventory(&f.db)?);
    Ok(())
}

#[test]
fn task_migration_is_explicit_keyed_and_atomic() -> Result<()> {
    let mut f = Fixture::v8()?;
    let old = f.db.entity("Old wire", access().scopes)?;
    let payload: String = f.db.conn.query_row(
        "SELECT payload FROM revision WHERE id=?1",
        [&old.revision],
        |r| r.get(0),
    )?;
    let before = inventory(&f.db)?;
    let reopened = Memory::open_with_storage(&f.path, f.storage.clone())?;
    assert_eq!(before, inventory(&reopened)?);
    assert!(reopened
        .task_session("owner", "20261110T120000-abcdef12", access())
        .is_err());
    let wrong = f.key("wrong.key")?;
    assert!(Memory::open_with_storage(&f.path, wrong).is_err());
    assert!(f.db.enable_session_tasks_failing().is_err());
    assert_eq!(before, inventory(&f.db)?);
    f.db.enable_session_tasks()?;
    assert_eq!(
        f.db.conn.query_row(
            "SELECT payload FROM revision WHERE id=?1",
            [&old.revision],
            |r| r.get::<_, String>(0)
        )?,
        payload
    );
    let migrated = inventory(&f.db)?;
    assert!(f.db.enable_session_tasks().is_err());
    assert_eq!(migrated, inventory(&f.db)?);
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    f.db.conn
        .execute("UPDATE task_operation SET after_revision='dangling'", [])?;
    assert!(Memory::open_with_storage(&f.path, f.storage.clone()).is_err());
    f.db.conn.execute(
        "UPDATE task_operation SET after_revision=?1",
        [&first.task.reference.revision],
    )?;
    f.db.conn.execute("DELETE FROM task_operation", [])?;
    assert!(Memory::open_with_storage(&f.path, f.storage.clone()).is_err());
    Ok(())
}

#[test]
fn snapshot_preserves_schema_version() -> Result<()> {
    for version in [8, 9] {
        let mut f = Fixture::v8()?;
        if version == 9 {
            f.db.enable_session_tasks()?;
            let s = f.session()?;
            let forgotten = f.db.task_create(&s, &op("forgotten"), draft())?;
            f.db.task_forget(&s, &forgotten.task)?;
            f.db.task_create(&s, &op("active"), draft())?;
        }
        let destination = f.output("backup");
        let storage = f.key("backup.key")?;
        let expected = inventory(&f.db)?;
        let report = f.db.backup_encrypted(&destination, storage.clone())?;
        assert_eq!(report.schema_version, version);
        let copy = Memory::open_with_storage(&destination, storage)?;
        assert_eq!(expected, inventory(&copy)?);
    }
    Ok(())
}

#[test]
fn restored_task_copy_is_historical() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let first = f.db.task_create(&s, &op("create"), draft())?;
    let backup = f.output("backup");
    let backup_key = f.key("backup.key")?;
    f.db.backup_encrypted(&backup, backup_key.clone())?;
    f.db.task_set_status(&s, &op("close"), &first.task, TaskStatus::Closed)?;
    let destination = f.output("restored");
    let restored_key = f.key("restored.key")?;
    let authority = RestoreAuthority::current(&f.path, f.storage.clone());
    let report = restore_encrypted(
        &backup,
        backup_key.clone(),
        &destination,
        restored_key.clone(),
        &authority,
    )?;
    assert_eq!(report.schema_version, 9);
    let mut copy = Memory::open_with_storage(&destination, restored_key.clone())?;
    assert!(copy
        .task_session("owner", "20261110T120000-abcdef12", access())
        .is_err());
    assert!(copy.task_read(&s, &first.task).is_err());
    assert!(copy.task_create(&s, &op("create"), draft()).is_err());
    assert!(copy
        .task_revise(&s, &op("revise"), &first.task, draft())
        .is_err());
    assert!(copy
        .task_set_status(&s, &op("close"), &first.task, TaskStatus::Closed)
        .is_err());
    assert!(copy.task_forget(&s, &first.task).is_err());
    let rejected = f.output("rejected");
    assert!(restore_encrypted(
        &backup,
        backup_key,
        &rejected,
        f.key("rejected.key")?,
        &RestoreAuthority::current(&destination, restored_key)
    )
    .is_err());
    assert!(!rejected.exists());
    Ok(())
}

fn evidence() -> Capture {
    Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "owner".into(),
            id: "synthetic-source".into(),
        },
        text: "amber source".into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    }
}
fn recall(db: &Memory) -> Result<RecallQuery> {
    Ok(RecallQuery {
        text: "amber".into(),
        known_at: db.snapshot()?,
        account: None,
        session_id: None,
        occurred_after: None,
        occurred_before: None,
        limit: 10,
        neighbors: 0,
        max_bytes: 20000,
    })
}
#[test]
fn task_generic_readers_withhold_indirect_task_closure() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let task = f.db.task_create(&s, &op("create"), draft())?.task;
    let source = f.db.capture(evidence())?;
    let entity = f.db.entity("Synthetic amber entity", access().scopes)?;
    let relation = f.db.relate(RelationInput {
        predicate: "references".into(),
        subject: entity,
        object: Target::Literal("amber".into()),
        support: Span {
            evidence: source.clone(),
            start: 0,
            end: 5,
        },
        consulted: vec![task.reference.clone()],
        text: "amber derived task content".into(),
        valid: Interval {
            from: Bound::Unbounded,
            to: Bound::Unbounded,
        },
        polarity: 1,
    })?;
    assert!(f.db.read(&relation, &access())?.is_none());
    // Future descendants are conservatively withheld even if their own kind is
    // otherwise eligible for episodic recall.
    let indirect = sql::append(
        &f.db.conn,
        None,
        &Record::Evidence(Capture {
            text: "amber indirect task content".into(),
            ..evidence()
        }),
        &access().scopes,
        std::slice::from_ref(&relation),
        "active",
    )?;
    assert!(f.db.read(&indirect, &access())?.is_none());
    for text in ["", "amber"] {
        let hits = f.db.search(
            &Query {
                text: text.into(),
                known_at: f.db.snapshot()?,
                valid_at: None,
                include_possible: true,
                limit: 100,
            },
            &access(),
        )?;
        assert!(hits.iter().all(|h| h.reference != task.reference
            && h.reference != relation
            && h.reference != indirect));
    }
    let query = recall(&f.db)?;
    let hits = f.db.recall(&query, &access())?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].anchor.reference, source);
    let policy = context::ContextPolicy {
        access: access(),
        recall: query,
        valid_at: None,
    };
    let situation = context::Situation {
        event: "Synthetic test".into(),
        goal: "Inspect amber".into(),
        needs: None,
    };
    let request = context::ContextRequest {
        query: "amber".into(),
        bindings: vec![],
    };
    let packet = f.db.context(&request, &policy, &situation)?;
    assert!(packet
        .records
        .iter()
        .all(|h| !matches!(h.record, Record::Task(_))
            && h.reference != relation
            && h.reference != indirect));
    f.db.task_read(&s, &task)?;
    Ok(())
}

#[test]
fn task_source_forget_uses_the_same_receipt_scrubbing_hook() -> Result<()> {
    let mut f = Fixture::new()?;
    let s = f.session()?;
    let source = f.db.capture(evidence())?;
    let task =
        f.db.task_create(&s, &op("future-dependent-task"), draft())?
            .task;
    // A bounded future-lifecycle fixture: no public API admits Task dependencies.
    f.db.conn.execute(
        "INSERT INTO dependency VALUES(?1,?2)",
        rusqlite::params![task.reference.revision, source.revision],
    )?;
    let report = f.db.forget(&evidence().source)?;
    assert_eq!(report.objects, 2);
    assert!(sql::lookup(&f.db.conn, &task.reference)?.is_none());
    sql::check_identity(&f.db.conn)?;
    assert!(f
        .db
        .task_create(&s, &op("future-dependent-task"), draft())
        .is_err());
    assert_eq!(f.db.conn.query_row("SELECT count(*) FROM task_operation WHERE state='forgotten' AND receipt_id IS NULL AND task_object IS NULL", [], |r| r.get::<_, i64>(0))?, 1);
    Ok(())
}

#[test]
fn task_migration_refuses_malformed_required_schema_and_lineage() -> Result<()> {
    for corruption in [
        "ALTER TABLE scope RENAME TO malformed_scope",
        "ALTER TABLE host_capture_receipt RENAME COLUMN lineage_version TO missing_lineage",
        "INSERT INTO host_capture_receipt(identity_hash,state,lineage_version) VALUES('invalid-active','active',2)",
    ] {
        let mut f = Fixture::v8()?;
        f.db.conn.execute_batch(corruption)?;
        let before = inventory(&f.db)?;
        assert!(f.db.enable_session_tasks().is_err(), "{corruption}");
        assert_eq!(before, inventory(&f.db)?);
        assert_eq!(task::version(&f.db.conn)?, 8);
    }
    let mut f = Fixture::v8()?;
    f.db.conn.execute_batch("PRAGMA user_version=7")?;
    let before = inventory(&f.db)?;
    assert!(f.db.enable_session_tasks().is_err());
    assert_eq!(before, inventory(&f.db)?);
    Ok(())
}

#[test]
fn task_schema_rejects_forged_receipt_and_missing_state() -> Result<()> {
    for corruption in [
        "UPDATE task_operation SET payload_hash=printf('%064d',0)",
        "UPDATE task_operation SET operation='status'",
        "DROP TABLE task_store_state",
    ] {
        let mut f = Fixture::new()?;
        let s = f.session()?;
        f.db.task_create(&s, &op("create"), draft())?;
        f.db.conn.execute_batch(corruption)?;
        assert!(
            Memory::open_with_storage(&f.path, f.storage.clone()).is_err(),
            "{corruption}"
        );
    }
    Ok(())
}

#[test]
fn restored_tasks_honor_forget_scopes_and_refuse_mixed_versions() -> Result<()> {
    for forget in [true, false] {
        let mut f = Fixture::new()?;
        let s = f.session()?;
        let task = f.db.task_create(&s, &op("create"), draft())?.task;
        let backup = f.output("backup");
        let backup_key = f.key("backup.key")?;
        f.db.backup_encrypted(&backup, backup_key.clone())?;
        if forget {
            f.db.task_forget(&s, &task)?;
        } else {
            f.db.conn.execute(
                "INSERT INTO scope VALUES(?1,'tightened')",
                [&task.reference.revision],
            )?;
        }
        let destination = f.output("restored");
        let restored_key = f.key("restored.key")?;
        let result = restore_encrypted(
            &backup,
            backup_key,
            &destination,
            restored_key.clone(),
            &RestoreAuthority::current(&f.path, f.storage.clone()),
        );
        if forget {
            assert_eq!(result?.purged_objects, 1);
            let copy = Memory::open_with_storage(&destination, restored_key)?;
            assert!(sql::lookup(&copy.conn, &task.reference)?.is_none());
            assert_eq!(
                copy.conn.query_row(
                    "SELECT count(*) FROM task_operation WHERE state='forgotten'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
        } else {
            assert!(result.is_err());
            assert!(!destination.exists());
        }
    }
    let mut f = Fixture::v8()?;
    let backup = f.output("v8-backup");
    let key = f.key("backup.key")?;
    f.db.backup_encrypted(&backup, key.clone())?;
    f.db.enable_session_tasks()?;
    let destination = f.output("mixed");
    assert!(restore_encrypted(
        &backup,
        key,
        &destination,
        f.key("mixed.key")?,
        &RestoreAuthority::current(&f.path, f.storage.clone())
    )
    .is_err());
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn old_record_wire_and_creation_are_unchanged() -> Result<()> {
    let f = Fixture::v8()?;
    assert_eq!(task::version(&f.db.conn)?, 8);
    for wire in [
        r#"{"kind":"entity","data":{"label":"Synthetic entity"}}"#,
        r#"{"kind":"evidence","data":{"source":{"connector":"fixture","account":"owner","id":"synthetic"},"text":"amber source","occurred_at":null,"scopes":["private"]}}"#,
        r#"{"kind":"relation","data":{"predicate":"relates","subject":{"object":"object","revision":"revision"},"object":{"kind":"literal","value":"amber"},"support":{"evidence":{"object":"evidence","revision":"revision"},"start":0,"end":5},"consulted":[],"text":"synthetic relation","valid":{"from":{"kind":"unbounded"},"to":{"kind":"unbounded"}},"polarity":1}}"#,
    ] {
        assert_eq!(
            serde_json::to_string(&serde_json::from_str::<Record>(wire)?)?,
            wire
        );
    }
    Ok(())
}

#[test]
fn task_fresh_same_status_is_not_an_idempotent_operation() -> Result<()> {
    let mut f = Fixture::new()?;
    let session = f.session()?;
    let open = f.db.task_create(&session, &op("create"), draft())?.task;
    let before = inventory(&f.db)?;
    assert!(f
        .db
        .task_set_status(&session, &op("same-open"), &open, TaskStatus::Open)
        .is_err());
    assert_eq!(before, inventory(&f.db)?);
    let active =
        f.db.task_set_status(&session, &op("activate"), &open, TaskStatus::Active)?;
    let before = inventory(&f.db)?;
    assert!(f
        .db
        .task_set_status(
            &session,
            &op("same-active"),
            &active.task,
            TaskStatus::Active
        )
        .is_err());
    assert_eq!(before, inventory(&f.db)?);
    assert_eq!(
        active,
        f.db.task_set_status(&session, &op("activate"), &open, TaskStatus::Active)?
    );
    assert_eq!(before, inventory(&f.db)?);
    f.db.task_set_status(&session, &op("return-open"), &active.task, TaskStatus::Open)?;
    Ok(())
}

#[test]
fn task_input_limits_count_utf8_and_serialized_bytes() -> Result<()> {
    let mut f = Fixture::new()?;
    let session = f.session()?;
    let bounded = TaskDraft {
        title: "é".repeat(256),
        intent: "é".repeat(4096),
    };
    assert_eq!((bounded.title.len(), bounded.intent.len()), (512, 8192));
    let task =
        f.db.task_create(&session, &op("bounds-control"), bounded.clone())?
            .task;
    assert_eq!(f.db.task_read(&session, &task)?.value.title, bounded.title);
    for (id, invalid) in [
        (
            "title-overflow",
            TaskDraft {
                title: format!("{}x", bounded.title),
                ..draft()
            },
        ),
        (
            "intent-overflow",
            TaskDraft {
                intent: format!("{}x", bounded.intent),
                ..draft()
            },
        ),
        (
            "serialized-overflow",
            TaskDraft {
                intent: "\0".repeat(8192),
                ..draft()
            },
        ),
        (
            "empty-title",
            TaskDraft {
                title: String::new(),
                ..draft()
            },
        ),
        (
            "empty-intent",
            TaskDraft {
                intent: " ".into(),
                ..draft()
            },
        ),
    ] {
        let before = inventory(&f.db)?;
        assert!(f
            .db
            .task_create(&session, &op(id), invalid.clone())
            .is_err());
        assert!(f.db.task_revise(&session, &op(id), &task, invalid).is_err());
        assert_eq!(before, inventory(&f.db)?);
    }
    assert!(TaskOperationId::new("").is_err());
    assert!(TaskOperationId::new(&"x".repeat(128)).is_ok());
    assert!(TaskOperationId::new(&"x".repeat(129)).is_err());
    assert!(f
        .db
        .task_session(&"x".repeat(257), "20261110T120000-abcdef12", access())
        .is_err());
    assert!(f.db.task_session("owner", "../foreign", access()).is_err());
    Ok(())
}

#[cfg(feature = "memory-prototype")]
#[test]
fn task_binding_accepts_real_native_session_id() -> Result<()> {
    let f = Fixture::new()?;
    let native = crate::session::Session::new_id();
    assert!(native.contains('T'));
    f.db.task_session("owner", &native, access())?;
    Ok(())
}
