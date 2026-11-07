use super::{durable::*, *};
use serde_json::json;
use std::path::PathBuf;

struct Fixture {
    dir: PathBuf,
    db: Memory,
    access: Access,
    source: Capture,
    input: RevisionRef,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("mecha-durable-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut db =
            Memory::create_durable(&dir.join("fixture.memory-prototype.sqlite"), "synthetic")
                .unwrap();
        let access = Access {
            domain: "synthetic".into(),
            scopes: ["fixture".into()].into(),
        };
        let source = Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "synthetic".into(),
                id: "original".into(),
            },
            text: "Synthetic source blue".into(),
            occurred_at: None,
            scopes: access.scopes.clone(),
            transcript: None,
        };
        let input = db.capture(source.clone()).unwrap();
        Self {
            dir,
            db,
            access,
            source,
            input,
        }
    }
    fn path(&self) -> PathBuf {
        self.dir.join("fixture.memory-prototype.sqlite")
    }
    fn spec(&self) -> TaskSpec {
        TaskSpec {
            workspace: self.dir.clone(),
            path: PathBuf::from("sample.txt"),
            max_bytes: 4096,
            max_lines: 16,
        }
    }
    fn create(&mut self) -> Receipt {
        self.db
            .durable_create(self.spec(), std::slice::from_ref(&self.input), &self.access)
            .unwrap()
    }
    fn settle(&mut self, r: &Receipt) -> Status {
        let p = self.db.durable_reserve_dispatch(r, &self.access).unwrap();
        self.db.durable_settle(p,HostObservation::from_dispatch(json!({"status":"successful","content":"Synthetic file blue","private_taint":true,"untrusted_taint":true,"cancelled":false})),&self.access).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn dispatch_reservation_is_spent_across_connections_and_restart() {
    let mut f = Fixture::new();
    let r = f.create();
    let mut other = Memory::open(&f.path()).unwrap();
    let other_r = other.durable_reopen(r.id(), &f.access).unwrap();
    let permit = f.db.durable_reserve_dispatch(&r, &f.access).unwrap();
    assert!(other.durable_reserve_dispatch(&other_r, &f.access).is_err());
    drop(permit);
    drop(other);
    let mut reopened = Memory::open(&f.path()).unwrap();
    let r2 = reopened.durable_reopen(r.id(), &f.access).unwrap();
    let status = reopened.durable_status(&r2, &f.access).unwrap();
    assert_eq!(status.phase, Phase::Indeterminate);
    assert!(status.acquisition_spent);
    assert!(reopened.durable_reserve_dispatch(&r2, &f.access).is_err());
}
#[test]
fn settled_result_and_final_decision_survive_reopen_without_dispatch() {
    let mut f = Fixture::new();
    let r = f.create();
    let observation = f.settle(&r).observation.unwrap();
    let mut db = Memory::open(&f.path()).unwrap();
    let r2 = db.durable_reopen(r.id(), &f.access).unwrap();
    assert_eq!(
        db.durable_status(&r2, &f.access).unwrap().observation,
        Some(observation)
    );
    assert_eq!(
        db.durable_observation(&r2, &f.access).unwrap().unwrap()["observation"]["content"],
        "Synthetic file blue"
    );
    assert!(db.durable_reserve_dispatch(&r2, &f.access).is_err());
    let p = db.durable_reserve_reconsideration(&r2, &f.access).unwrap();
    let committed = db
        .durable_commit(
            p,
            json!({"interpretation":"read completed, broader goal unknown"}),
            &f.access,
        )
        .unwrap();
    assert_eq!(committed.phase, Phase::Committed);
    assert!(db.durable_reserve_reconsideration(&r2, &f.access).is_err());
}
#[test]
fn original_uncited_input_change_prevents_final_commit() {
    let mut f = Fixture::new();
    let r = f.create();
    f.settle(&r);
    let p = f.db.durable_reserve_reconsideration(&r, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    let mut changed = f.source.clone();
    changed.text = "Synthetic source red".into();
    other.capture(changed).unwrap();
    assert!(f
        .db
        .durable_commit(p, json!({"citations":[]}), &f.access)
        .is_err());
    assert_eq!(
        f.db.durable_status(&r, &f.access).unwrap().phase,
        Phase::Reconsidering
    );
}
#[test]
fn cancellation_before_dispatch_and_after_result_never_commits() {
    let mut f = Fixture::new();
    let before = f.create();
    f.db.durable_cancel(&before, &f.access).unwrap();
    assert!(f.db.durable_reserve_dispatch(&before, &f.access).is_err());
    let after = f.create();
    let obs = f.settle(&after).observation.unwrap();
    let p =
        f.db.durable_reserve_reconsideration(&after, &f.access)
            .unwrap();
    f.db.durable_cancel(&after, &f.access).unwrap();
    assert!(f
        .db
        .durable_commit(p, json!({"answer":"done"}), &f.access)
        .is_err());
    let s = f.db.durable_status(&after, &f.access).unwrap();
    assert_eq!(s.phase, Phase::Cancelled);
    assert_eq!(s.observation, Some(obs));
    assert!(f
        .db
        .durable_observation(&after, &f.access)
        .unwrap()
        .is_some());
}
#[test]
fn cancellation_during_dispatch_preserves_actual_observation() {
    let mut f = Fixture::new();
    let r = f.create();
    let p = f.db.durable_reserve_dispatch(&r, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    let r2 = other.durable_reopen(r.id(), &f.access).unwrap();
    other.durable_cancel(&r2, &f.access).unwrap();
    let s =
        f.db.durable_settle(
            p,
            HostObservation::from_dispatch(
                json!({"content":"actual synthetic read","cancelled":false}),
            ),
            &f.access,
        )
        .unwrap();
    assert_eq!(s.phase, Phase::Cancelled);
    assert_eq!(
        f.db.durable_observation(&r, &f.access).unwrap().unwrap()["cancelled_task"],
        true
    );
}
#[test]
fn forgotten_original_input_invalidates_pending_settlement_and_erases_task() {
    let mut f = Fixture::new();
    let r = f.create();
    let p = f.db.durable_reserve_dispatch(&r, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    other.forget(&f.source.source).unwrap();
    assert!(f
        .db
        .durable_settle(
            p,
            HostObservation::from_dispatch(json!({"content":"must never resurrect"})),
            &f.access
        )
        .is_err());
    let reopened = other.durable_reopen(r.id(), &f.access).unwrap();
    let s = other.durable_status(&reopened, &f.access).unwrap();
    assert_eq!(s.phase, Phase::Forgotten);
    assert!(s.task.is_none());
    assert!(other.durable_spec(&reopened, &f.access).is_err());
    assert!(other.durable_observation(&reopened, &f.access).is_err());
    let count: i64 = other
        .conn
        .query_row("SELECT count(*) FROM revision", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn forgetting_observation_also_erases_sibling_attempt_and_task_content() {
    let mut f = Fixture::new();
    let r = f.create();
    let s = f.settle(&r);
    let observation = s.observation.unwrap();
    let p = f.db.durable_reserve_reconsideration(&r, &f.access).unwrap();
    f.db.durable_commit(p, json!({"attempt":"synthetic interpretation"}), &f.access)
        .unwrap();
    let Record::Evidence(record) = f.db.read(&observation, &f.access).unwrap().unwrap() else {
        panic!()
    };
    f.db.forget(&record.source).unwrap();
    assert_eq!(
        f.db.durable_status(&r, &f.access).unwrap().phase,
        Phase::Forgotten
    );
    let count: i64 =
        f.db.conn
            .query_row("SELECT count(*) FROM revision", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 1, "only independent original source remains");
}
#[test]
fn exact_access_and_scope_changes_fail_closed() {
    let mut f = Fixture::new();
    let r = f.create();
    let wrong = Access {
        domain: f.access.domain.clone(),
        scopes: ["fixture".into(), "extra".into()].into(),
    };
    assert!(f.db.durable_reopen(r.id(), &wrong).is_err());
    f.db.conn
        .execute("DELETE FROM scope WHERE revision=?1", [&f.input.revision])
        .unwrap();
    assert!(f.db.durable_reserve_dispatch(&r, &f.access).is_err());
}
#[test]
fn missing_or_restricted_input_cannot_create_or_reserve() {
    let mut f = Fixture::new();
    let missing = RevisionRef {
        object: "absent".into(),
        revision: "absent".into(),
    };
    assert!(f
        .db
        .durable_create(f.spec(), &[missing], &f.access)
        .is_err());
    let r = f.create();
    f.db.conn
        .execute("INSERT INTO restriction VALUES (?1)", [&f.input.object])
        .unwrap();
    assert!(f.db.durable_reserve_dispatch(&r, &f.access).is_err());
}
#[test]
fn old_schema_is_readable_but_lifecycle_use_requires_explicit_new_store() {
    let f = Fixture::new();
    let oldpath = f.dir.join("old.memory-prototype.sqlite");
    let mut old = Memory::create(&oldpath, "synthetic").unwrap();
    assert!(old
        .durable_create(f.spec(), &[], &f.access)
        .unwrap_err()
        .to_string()
        .contains("version 3"));
    assert_eq!(
        old.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn racing_connections_issue_exactly_one_dispatch_permit() {
    let mut f = Fixture::new();
    let r = f.create();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let gate = gate.clone();
            let path = f.path();
            let id = r.id().to_owned();
            let access = f.access.clone();
            std::thread::spawn(move || {
                let mut db = Memory::open(&path).unwrap();
                let r = db.durable_reopen(&id, &access).unwrap();
                gate.wait();
                db.durable_reserve_dispatch(&r, &access).is_ok()
            })
        })
        .collect::<Vec<_>>();
    gate.wait();
    let successes = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(successes, 1);
}

// Subprocess endpoint: exit immediately after commit without dropping the Memory
// or permit. The parent inspects actual reopened SQLite state, not a mocked crash.
#[test]
fn reservation_process_endpoint() {
    let Ok(path) = std::env::var("MECHA_DURABLE_CRASH_DB") else {
        return;
    };
    let id = std::env::var("MECHA_DURABLE_CRASH_TASK").unwrap();
    let access = Access {
        domain: "synthetic".into(),
        scopes: ["fixture".into()].into(),
    };
    let mut db = Memory::open(std::path::Path::new(&path)).unwrap();
    let r = db.durable_reopen(&id, &access).unwrap();
    if std::env::var("MECHA_DURABLE_CRASH_PHASE").unwrap() == "dispatch" {
        let _permit = db.durable_reserve_dispatch(&r, &access).unwrap();
        std::process::exit(87)
    }
    let _permit = db.durable_reserve_reconsideration(&r, &access).unwrap();
    std::process::exit(87)
}
fn crash_after_reservation(f: &Fixture, r: &Receipt, phase: &str) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "memory::durable_tests::reservation_process_endpoint",
            "--nocapture",
        ])
        .env("MECHA_DURABLE_CRASH_DB", f.path())
        .env("MECHA_DURABLE_CRASH_TASK", r.id())
        .env("MECHA_DURABLE_CRASH_PHASE", phase)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(87));
}
#[test]
fn process_loss_after_reservation_never_recovers_an_execution_capability() {
    let mut f = Fixture::new();
    let dispatch = f.create();
    crash_after_reservation(&f, &dispatch, "dispatch");
    let mut reopened = Memory::open(&f.path()).unwrap();
    let r = reopened.durable_reopen(dispatch.id(), &f.access).unwrap();
    assert_eq!(
        reopened.durable_status(&r, &f.access).unwrap().phase,
        Phase::Indeterminate
    );
    assert!(reopened.durable_reserve_dispatch(&r, &f.access).is_err());
    let reconsider = f.create();
    f.settle(&reconsider);
    crash_after_reservation(&f, &reconsider, "reconsider");
    let r = reopened.durable_reopen(reconsider.id(), &f.access).unwrap();
    let status = reopened.durable_status(&r, &f.access).unwrap();
    assert_eq!(status.phase, Phase::Reconsidering);
    assert!(status.reconsideration_spent);
    assert!(status.observation.is_some());
    assert!(reopened
        .durable_reserve_reconsideration(&r, &f.access)
        .is_err());
}
#[test]
fn forgotten_input_prevents_pending_decision_commit_and_generic_recreation() {
    let mut f = Fixture::new();
    let r = f.create();
    let status = f.settle(&r);
    let Record::Evidence(task) =
        f.db.read(status.task.as_ref().unwrap(), &f.access)
            .unwrap()
            .unwrap()
    else {
        panic!()
    };
    let Record::Evidence(observation) =
        f.db.read(status.observation.as_ref().unwrap(), &f.access)
            .unwrap()
            .unwrap()
    else {
        panic!()
    };
    let p = f.db.durable_reserve_reconsideration(&r, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    other.forget(&f.source.source).unwrap();
    assert!(f
        .db
        .durable_commit(p, json!({"content":"must not return"}), &f.access)
        .is_err());
    // Old lifecycle source identities cannot be re-ingested through generic capture.
    assert!(f.db.capture(task).is_err());
    assert!(f.db.capture(observation).is_err());
    let mut unrelated = f.source.clone();
    unrelated.source.id = "independent".into();
    assert!(f.db.capture(unrelated).is_ok());
    assert_eq!(
        f.db.durable_status(&r, &f.access).unwrap().phase,
        Phase::Forgotten
    );
    let r2 = f.db.durable_reopen(r.id(), &f.access).unwrap();
    assert!(f.db.durable_reserve_dispatch(&r2, &f.access).is_err());
}
#[test]
fn host_cancellation_and_refusal_are_observations_not_model_success() {
    let mut f = Fixture::new();
    let r = f.create();
    let p = f.db.durable_reserve_dispatch(&r, &f.access).unwrap();
    let s =
        f.db.durable_settle(
            p,
            HostObservation::from_dispatch(json!({"status":"refused_policy","cancelled":true})),
            &f.access,
        )
        .unwrap();
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(s.observation.is_some());
    assert!(s.decision.is_none());
    assert!(f.db.durable_reserve_reconsideration(&r, &f.access).is_err());
}
#[test]
fn lifecycle_creation_rejects_unbound_paths_and_unbounded_limits() {
    let mut f = Fixture::new();
    for path in ["../sample.txt", "/sample.txt", ""] {
        let mut spec = f.spec();
        spec.path = PathBuf::from(path);
        assert!(f.db.durable_create(spec, &[], &f.access).is_err());
    }
    for bytes in [0, 1023, 24_001] {
        let mut spec = f.spec();
        spec.max_bytes = bytes;
        assert!(f.db.durable_create(spec, &[], &f.access).is_err());
    }
    for lines in [0, 257] {
        let mut spec = f.spec();
        spec.max_lines = lines;
        assert!(f.db.durable_create(spec, &[], &f.access).is_err());
    }
}
#[test]
fn retaining_concept_cannot_downgrade_lifecycle_version_gate() {
    let mut f = Fixture::new();
    let mut refs = Vec::new();
    for (id, text) in [
        ("one", "A pending permit delayed the synthetic route."),
        (
            "two",
            "The synthetic route needed a backup while a permit was pending.",
        ),
    ] {
        let mut source = f.source.clone();
        source.source.id = id.into();
        source.text = text.into();
        refs.push(f.db.capture(source).unwrap());
    }
    let req =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    let draft = json!({"name":"Synthetic permit planning","principle":"A pending permit can require an alternative route.","applicability":"Synthetic routes with permits.","support":[{"source_id":refs[0].revision,"field":"","quote":"pending permit"},{"source_id":refs[1].revision,"field":"","quote":"permit was pending"}],"counterexamples":[]});
    f.db.retain_concept(req, &draft.to_string(), &f.access, None)
        .unwrap();
    assert_eq!(
        f.db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    let r = f.create();
    assert_eq!(
        f.db.durable_status(&r, &f.access).unwrap().phase,
        Phase::Ready
    );
}

#[test]
fn live_dispatch_guard_rechecks_other_connection_cancellation_forget_and_sources() {
    for change in ["cancel", "forget", "source", "scope", "restriction"] {
        let mut f = Fixture::new();
        let receipt = f.create();
        let permit = f.db.durable_reserve_dispatch(&receipt, &f.access).unwrap();
        let guard = f.db.durable_dispatch_guard(&permit, &f.access).unwrap();
        guard.check().unwrap();
        let mut other = Memory::open(&f.path()).unwrap();
        match change {
            "cancel" => {
                let r = other.durable_reopen(receipt.id(), &f.access).unwrap();
                other.durable_cancel(&r, &f.access).unwrap();
            }
            "forget" => {
                other.forget(&f.source.source).unwrap();
            }
            "source" => {
                let mut source = f.source.clone();
                source.text = "Updated synthetic source".into();
                other.capture(source).unwrap();
            }
            "scope" => {
                other
                    .conn
                    .execute("DELETE FROM scope WHERE revision=?1", [&f.input.revision])
                    .unwrap();
            }
            "restriction" => {
                other
                    .conn
                    .execute("INSERT INTO restriction VALUES (?1)", [&f.input.object])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            guard.check().is_err(),
            "guard must reject {change} after reservation"
        );
    }
}

#[test]
fn live_dispatch_guard_rejects_wrong_context_and_replaced_store() {
    let mut f = Fixture::new();
    let receipt = f.create();
    let permit = f.db.durable_reserve_dispatch(&receipt, &f.access).unwrap();
    let wrong = Access {
        domain: f.access.domain.clone(),
        scopes: ["other".into()].into(),
    };
    assert!(f.db.durable_dispatch_guard(&permit, &wrong).is_err());
    let guard = f.db.durable_dispatch_guard(&permit, &f.access).unwrap();
    std::fs::rename(f.path(), f.dir.join("original.memory-prototype.sqlite")).unwrap();
    let _replacement = Memory::create_durable(&f.path(), "synthetic").unwrap();
    assert!(guard.check().is_err());
}

#[test]
fn explicit_v4_is_readable_and_legacy_finish_is_refused() {
    let mut f = Fixture::new();
    let r = f.create();
    f.settle(&r);
    f.db.conn.execute_batch("PRAGMA user_version=4;").unwrap();
    assert!(
        Memory::open(&f.path()).is_ok(),
        "explicit v4 must be supported"
    );
    assert!(f.db.durable_reserve_reconsideration(&r, &f.access).is_err());
}
