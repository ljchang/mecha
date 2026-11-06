use super::{durable::*, durable_action::*, durable_decision::*, *};
use serde_json::json;
use std::path::PathBuf;

struct Fixture {
    dir: PathBuf,
    db: Memory,
    access: Access,
    source: Capture,
    read: Receipt,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("mecha-action-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut db =
            Memory::create_durable_action(&dir.join("test.memory-prototype.sqlite"), "synthetic")
                .unwrap();
        let access = Access {
            domain: "synthetic".into(),
            scopes: ["fixture".into()].into(),
        };
        let source = Capture {
            source: SourceKey {
                connector: "fixture".into(),
                account: "synthetic".into(),
                id: "input".into(),
            },
            text: "Synthetic input only".into(),
            occurred_at: None,
            scopes: access.scopes.clone(),
            transcript: None,
        };
        let input = db.capture(source.clone()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: dir.clone(),
                    path: "input.txt".into(),
                    max_bytes: 4096,
                    max_lines: 16,
                },
                &[input],
                &access,
            )
            .unwrap();
        let permit = db.durable_reserve_dispatch(&read, &access).unwrap();
        db.durable_settle(permit,HostObservation::from_dispatch(json!({"status":"successful","content":"Synthetic observed blue","cancelled":false})),&access).unwrap();
        Self {
            dir,
            db,
            access,
            source,
            read,
        }
    }
    fn path(&self) -> PathBuf {
        self.dir.join("test.memory-prototype.sqlite")
    }
    fn bind(&mut self) -> ModelReceipt {
        self.db
            .durable_bind_model(
                &self.read,
                json!({"contract":"synthetic fixed"}),
                &self.access,
            )
            .unwrap()
    }
    fn settle(&mut self, r: &ModelReceipt, kind: AttemptKind) {
        let p = self
            .db
            .reserve_model_attempt(r, kind, json!({"prompt":"synthetic request"}), &self.access)
            .unwrap();
        self.db
            .model_attempt_guard(&p, &self.access)
            .unwrap()
            .check()
            .unwrap();
        self.db
            .settle_model_attempt(
                p,
                HostModelResult::new(json!({"reply":"synthetic reply"})),
                &self.access,
            )
            .unwrap();
    }
    fn assess(&mut self, r: &ModelReceipt, kind: AttemptKind, retry: bool) {
        self.db
            .assess_model_attempt(r, kind, json!({"status":"recommend"}), retry, &self.access)
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn finalized(f: &mut Fixture) -> ModelReceipt {
    let model = f.bind();
    f.settle(&model, AttemptKind::Initial);
    f.assess(&model, AttemptKind::Initial, false);
    f.db.finalize_model(
        &model,
        json!({"status":"recommend","choice":"nori"}),
        &f.access,
    )
    .unwrap();
    model
}
fn action(f: &mut Fixture) -> ActionReceipt {
    let model = finalized(f);
    f.db.bind_action(
        &model,
        json!({"fixture":"synthetic","choice":"nori"}),
        &f.access,
    )
    .unwrap()
}
#[test]
fn binding_requires_finalized_model_and_is_unique() {
    let mut f = Fixture::new();
    let model = f.bind();
    assert!(f.db.bind_action(&model, json!({}), &f.access).is_err());
    f.settle(&model, AttemptKind::Initial);
    f.assess(&model, AttemptKind::Initial, false);
    f.db.finalize_model(&model, json!({"status":"recommend"}), &f.access)
        .unwrap();
    let r =
        f.db.bind_action(&model, json!({"fixture":"synthetic"}), &f.access)
            .unwrap();
    assert_eq!(
        f.db.durable_action_status(&r, &f.access).unwrap().phase,
        ActionPhase::Ready
    );
    assert_eq!(
        f.db.durable_action_spec(&r, &f.access).unwrap(),
        json!({"fixture":"synthetic"})
    );
    assert!(f.db.bind_action(&model, json!({}), &f.access).is_err());
}
#[test]
fn lost_reservation_cannot_be_replayed_after_reopen() {
    let mut f = Fixture::new();
    let r = action(&mut f);
    let permit = f.db.reserve_action(&r, &f.access).unwrap();
    f.db.action_guard(&permit, &f.access)
        .unwrap()
        .check()
        .unwrap();
    drop(permit);
    let mut reopened = Memory::open(&f.path()).unwrap();
    let rr = reopened.durable_action_reopen(r.id(), &f.access).unwrap();
    assert_eq!(
        reopened
            .durable_action_status(&rr, &f.access)
            .unwrap()
            .phase,
        ActionPhase::Indeterminate
    );
    assert!(reopened.reserve_action(&rr, &f.access).is_err());
}
#[test]
fn cancellation_retains_live_actual_outcome_but_denies_feedback() {
    let mut f = Fixture::new();
    let r = action(&mut f);
    let p = f.db.reserve_action(&r, &f.access).unwrap();
    let guard = f.db.action_guard(&p, &f.access).unwrap();
    f.db.durable_action_cancel(&r, &f.access).unwrap();
    assert!(guard.check().is_err());
    let s =
        f.db.settle_action(p, json!({"actual_minutes":17}), &f.access)
            .unwrap();
    assert!(s.cancelled);
    assert!(s.observation.is_some());
    assert_eq!(
        f.db.durable_action_observation(&r, &f.access)
            .unwrap()
            .unwrap()["observation"]["actual_minutes"],
        17
    );
    assert!(f
        .db
        .finalize_action(&r, json!({"feedback":"passed"}), &f.access)
        .is_err());
    assert!(f.db.reserve_action(&r, &f.access).is_err());
}
#[test]
fn feedback_resumes_after_restart_without_another_reservation() {
    let mut f = Fixture::new();
    let r = action(&mut f);
    let p = f.db.reserve_action(&r, &f.access).unwrap();
    f.db.settle_action(p, json!({"actual_minutes":17}), &f.access)
        .unwrap();
    let mut reopened = Memory::open(&f.path()).unwrap();
    let rr = reopened.durable_action_reopen(r.id(), &f.access).unwrap();
    assert!(reopened.reserve_action(&rr, &f.access).is_err());
    assert_eq!(
        reopened
            .finalize_action(&rr, json!({"feedback":"passed"}), &f.access)
            .unwrap()
            .phase,
        ActionPhase::Finalized
    );
    assert_eq!(
        reopened.durable_action_feedback(&rr, &f.access).unwrap(),
        Some(json!({"feedback":"passed"}))
    );
    assert!(reopened.finalize_action(&rr, json!({}), &f.access).is_err());
}
#[test]
fn forgotten_action_sibling_erases_all_model_and_read_content_and_suppresses_recapture() {
    let mut f = Fixture::new();
    let r = action(&mut f);
    let p = f.db.reserve_action(&r, &f.access).unwrap();
    f.db.settle_action(p, json!({"actual_minutes":17}), &f.access)
        .unwrap();
    f.db.finalize_action(&r, json!({"feedback":"passed"}), &f.access)
        .unwrap();
    let s = f.db.durable_action_status(&r, &f.access).unwrap();
    let obs = s.observation.unwrap();
    let Record::Evidence(capture) = super::sql::lookup(&f.db.conn, &obs).unwrap().unwrap() else {
        panic!("observation must be evidence")
    };
    let lifecycle_captures = {
        let mut stmt = f.db.conn.prepare("SELECT payload FROM revision").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str::<Record>(&r.unwrap()).unwrap())
            .filter_map(|r| match r {
                Record::Evidence(c) if c.source.connector == "durable-read" => Some(c),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    f.db.forget(&capture.source).unwrap();
    assert_eq!(
        f.db.durable_action_status(&r, &f.access).unwrap().phase,
        ActionPhase::Forgotten
    );
    assert!(f.db.durable_action_spec(&r, &f.access).is_err());
    assert!(f.db.durable_action_observation(&r, &f.access).is_err());
    assert!(f.db.durable_observation(&f.read, &f.access).is_err());
    let count: i64 =
        f.db.conn
            .query_row("SELECT count(*) FROM revision", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 1, "only the independent original input remains");
    assert!(lifecycle_captures.len() >= 8);
    for capture in lifecycle_captures {
        assert!(f.db.capture(capture).is_err());
    }
}
#[test]
fn changed_restricted_and_forgotten_sources_refuse_late_writes_but_keep_status() {
    for mode in ["changed", "restricted", "forgotten", "scopes"] {
        let mut f = Fixture::new();
        let r = action(&mut f);
        let p = f.db.reserve_action(&r, &f.access).unwrap();
        let guard = f.db.action_guard(&p, &f.access).unwrap();
        let source = f.db.capture(f.source.clone()).unwrap();
        match mode {
            "changed" => {
                let mut c = f.source.clone();
                c.text = "changed synthetic source".into();
                f.db.capture(c).unwrap();
            }
            "restricted" => {
                f.db.conn
                    .execute("INSERT INTO restriction VALUES (?1)", [&source.object])
                    .unwrap();
            }
            "forgotten" => {
                f.db.forget(&f.source.source).unwrap();
            }
            "scopes" => {
                let mut c = f.source.clone();
                c.scopes.clear();
                f.db.capture(c).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(guard.check().is_err(), "{mode}");
        assert!(
            f.db.settle_action(p, json!({"actual_minutes":17}), &f.access)
                .is_err(),
            "{mode}"
        );
        assert!(f.db.durable_action_status(&r, &f.access).is_ok(), "{mode}");
        assert!(f.db.durable_action_spec(&r, &f.access).is_err(), "{mode}");
    }
}

#[test]
fn v5_cannot_finish_through_legacy_supplied_json() {
    let mut f = Fixture::new();
    assert!(f
        .db
        .durable_reserve_reconsideration(&f.read, &f.access)
        .is_err());
}
#[test]
fn v4_remains_usable_and_is_never_implicitly_upgraded() {
    let f = Fixture::new();
    let path = f.dir.join("older.memory-prototype.sqlite");
    let db = Memory::create_durable_model(&path, "synthetic").unwrap();
    assert_eq!(
        db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    drop(db);
    let db = Memory::open(&path).unwrap();
    assert_eq!(
        db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert!(Memory::create_durable_action(&path, "synthetic").is_err());
}
