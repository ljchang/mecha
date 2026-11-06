use super::{durable::*, durable_decision::*, *};
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
        let dir = std::env::temp_dir().join(format!("mecha-model-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut db =
            Memory::create_durable_model(&dir.join("test.memory-prototype.sqlite"), "synthetic")
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

#[test]
fn binding_is_unique_and_retains_exact_successful_read() {
    let mut f = Fixture::new();
    let r = f.bind();
    assert!(f
        .db
        .durable_bind_model(&f.read, json!({"different":true}), &f.access)
        .is_err());
    let read = f.db.durable_model_read(&r, &f.access).unwrap();
    assert_eq!(read.content, "Synthetic observed blue");
    assert_eq!(read.sha256.len(), 64);
    assert_eq!(
        f.db.durable_model_spec(&r, &f.access).unwrap()["contract"],
        "synthetic fixed"
    );
    assert!(f
        .db
        .durable_reserve_reconsideration(&f.read, &f.access)
        .is_err());
}
#[test]
fn every_unsuccessful_read_is_ineligible_for_model_binding() {
    let mut f = Fixture::new();
    for status in [
        "refused_policy",
        "refused_user",
        "failed",
        "indeterminate",
        "cancelled_before_dispatch",
    ] {
        let read =
            f.db.durable_create(
                TaskSpec {
                    workspace: f.dir.clone(),
                    path: "other.txt".into(),
                    max_bytes: 4096,
                    max_lines: 16,
                },
                &[],
                &f.access,
            )
            .unwrap();
        let p = f.db.durable_reserve_dispatch(&read, &f.access).unwrap();
        f.db.durable_settle(
            p,
            HostObservation::from_dispatch(json!({"status":status,"content":"synthetic"})),
            &f.access,
        )
        .unwrap();
        assert!(
            f.db.durable_bind_model(&read, json!({}), &f.access)
                .is_err(),
            "{status}"
        );
    }
}
#[test]
fn settled_reply_can_be_assessed_after_restart_without_another_call() {
    let mut f = Fixture::new();
    let r = f.bind();
    f.settle(&r, AttemptKind::Initial);
    let mut db = Memory::open(&f.path()).unwrap();
    let rr = db.durable_model_reopen(r.id(), &f.access).unwrap();
    assert!(db
        .reserve_model_attempt(&rr, AttemptKind::Initial, json!({}), &f.access)
        .is_err());
    db.assess_model_attempt(
        &rr,
        AttemptKind::Initial,
        json!({"status":"recommend"}),
        false,
        &f.access,
    )
    .unwrap();
    // Pure repeated assessment is idempotent, but cannot rewrite the result.
    db.assess_model_attempt(
        &rr,
        AttemptKind::Initial,
        json!({"status":"recommend"}),
        false,
        &f.access,
    )
    .unwrap();
    assert!(db
        .assess_model_attempt(
            &rr,
            AttemptKind::Initial,
            json!({"status":"failed"}),
            false,
            &f.access
        )
        .is_err());
    db.finalize_model(&rr, json!({"status":"recommend"}), &f.access)
        .unwrap();
    assert_eq!(
        db.durable_model_decision(&rr, &f.access).unwrap().unwrap()["status"],
        "recommend"
    );
    assert_eq!(
        db.durable_model_status(&rr, &f.access).unwrap().phase,
        ModelPhase::Finalized
    );
}
#[test]
fn retry_requires_assessed_finding_and_exactly_two_slots() {
    let mut f = Fixture::new();
    let r = f.bind();
    assert!(f
        .db
        .reserve_model_attempt(&r, AttemptKind::Retry, json!({}), &f.access)
        .is_err());
    f.settle(&r, AttemptKind::Initial);
    f.assess(&r, AttemptKind::Initial, true);
    assert!(f.db.finalize_model(&r, json!({}), &f.access).is_err());
    f.settle(&r, AttemptKind::Retry);
    assert!(f
        .db
        .assess_model_attempt(&r, AttemptKind::Retry, json!({}), true, &f.access)
        .is_err());
    f.assess(&r, AttemptKind::Retry, false);
    f.db.finalize_model(&r, json!({"status":"abstain"}), &f.access)
        .unwrap();
    assert!(f
        .db
        .reserve_model_attempt(&r, AttemptKind::Retry, json!({}), &f.access)
        .is_err());
}
#[test]
fn dropped_reservation_stays_indeterminate_across_connections() {
    let mut f = Fixture::new();
    let r = f.bind();
    let p =
        f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
            .unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    let rr = other.durable_model_reopen(r.id(), &f.access).unwrap();
    assert!(other
        .reserve_model_attempt(&rr, AttemptKind::Initial, json!({}), &f.access)
        .is_err());
    drop(p);
    assert_eq!(
        other
            .durable_model_status(&rr, &f.access)
            .unwrap()
            .initial
            .phase,
        AttemptPhase::Indeterminate
    );
    assert!(other.finalize_model(&rr, json!({}), &f.access).is_err());
    assert!(other
        .reserve_model_attempt(&rr, AttemptKind::Retry, json!({}), &f.access)
        .is_err());
}
#[test]
fn cancellation_after_reservation_retains_result_and_blocks_assessment() {
    let mut f = Fixture::new();
    let r = f.bind();
    let p =
        f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
            .unwrap();
    let guard = f.db.model_attempt_guard(&p, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    other.durable_cancel(&f.read, &f.access).unwrap();
    assert!(guard.check().is_err());
    f.db.settle_model_attempt(
        p,
        HostModelResult::new(json!({"reply":"completed before cancellation was observed"})),
        &f.access,
    )
    .unwrap();
    assert_eq!(
        f.db.durable_model_status(&r, &f.access).unwrap().phase,
        ModelPhase::Cancelled
    );
    assert!(f
        .db
        .durable_model_attempt(&r, AttemptKind::Initial, &f.access)
        .unwrap()
        .unwrap()
        .result
        .is_some());
    assert!(f
        .db
        .assess_model_attempt(&r, AttemptKind::Initial, json!({}), false, &f.access)
        .is_err());
    assert!(f.db.finalize_model(&r, json!({}), &f.access).is_err());
}
#[test]
fn changed_or_restricted_original_input_refuses_guard_and_late_settlement() {
    for restrict in [false, true] {
        let mut f = Fixture::new();
        let r = f.bind();
        let p =
            f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
                .unwrap();
        let guard = f.db.model_attempt_guard(&p, &f.access).unwrap();
        let mut other = Memory::open(&f.path()).unwrap();
        let mut changed = f.source.clone();
        if restrict {
            changed.scopes.insert("extra".into());
        } else {
            changed.text = "Synthetic changed input".into();
        }
        other.capture(changed).unwrap();
        assert!(guard.check().is_err());
        assert!(f
            .db
            .settle_model_attempt(
                p,
                HostModelResult::new(json!({"reply":"private late"})),
                &f.access
            )
            .is_err());
        assert!(f.db.durable_model_spec(&r, &f.access).is_err());
        assert_eq!(
            f.db.durable_model_status(&r, &f.access)
                .unwrap()
                .initial
                .phase,
            AttemptPhase::Indeterminate
        );
    }
}
#[test]
fn every_model_sibling_forgets_the_entire_lifecycle_and_suppresses_recapture() {
    for index in 0..8 {
        let mut f = Fixture::new();
        let r = f.bind();
        f.settle(&r, AttemptKind::Initial);
        f.assess(&r, AttemptKind::Initial, true);
        f.settle(&r, AttemptKind::Retry);
        f.assess(&r, AttemptKind::Retry, false);
        f.db.finalize_model(&r, json!({"status":"withheld"}), &f.access)
            .unwrap();
        let s = f.db.durable_model_status(&r, &f.access).unwrap();
        let refs = [
            s.contract.unwrap(),
            s.initial.request.unwrap(),
            s.initial.result.unwrap(),
            s.initial.assessment.unwrap(),
            s.retry.request.unwrap(),
            s.retry.result.unwrap(),
            s.retry.assessment.unwrap(),
            s.decision.unwrap(),
        ];
        let records = refs
            .iter()
            .map(|r| {
                let Some(Record::Evidence(c)) = f.db.read(r, &f.access).unwrap() else {
                    panic!("missing sibling")
                };
                c
            })
            .collect::<Vec<_>>();
        f.db.forget(&records[index].source).unwrap();
        let reopened = f.db.durable_model_reopen(r.id(), &f.access).unwrap();
        let s = f.db.durable_model_status(&reopened, &f.access).unwrap();
        assert_eq!(s.phase, ModelPhase::Forgotten);
        assert!(s.contract.is_none());
        assert!(s.initial.request.is_none());
        assert_eq!(
            f.db.durable_status(&f.read, &f.access).unwrap().phase,
            Phase::Forgotten
        );
        for (reference, record) in refs.iter().zip(records) {
            assert!(f.db.read(reference, &f.access).unwrap().is_none());
            assert!(f.db.capture(record).is_err());
        }
        assert!(f.db.durable_model_spec(&reopened, &f.access).is_err());
    }
}

// Child exits deliberately without running destructors; the parent reads actual
// SQLite state left by the terminated process, never a reconstructed permit.
#[test]
fn model_process_endpoint() {
    let Ok(path) = std::env::var("MECHA_MODEL_PROCESS_PATH") else {
        return;
    };
    let id = std::env::var("MECHA_MODEL_PROCESS_ID").unwrap();
    let mode = std::env::var("MECHA_MODEL_PROCESS_MODE").unwrap();
    let access = Access {
        domain: "synthetic".into(),
        scopes: ["fixture".into()].into(),
    };
    let mut db = Memory::open(std::path::Path::new(&path)).unwrap();
    let r = db.durable_model_reopen(&id, &access).unwrap();
    let kind = if mode == "retry" {
        AttemptKind::Retry
    } else {
        AttemptKind::Initial
    };
    let p = db
        .reserve_model_attempt(&r, kind, json!({"prompt":"process synthetic"}), &access)
        .unwrap();
    if mode == "settled" {
        db.settle_model_attempt(
            p,
            HostModelResult::new(json!({"reply":"process completed"})),
            &access,
        )
        .unwrap();
    }
    std::process::exit(0);
}
#[test]
fn actual_process_loss_preserves_spent_initial_retry_and_settled_results() {
    for mode in ["initial", "retry", "settled"] {
        let mut f = Fixture::new();
        let r = f.bind();
        if mode == "retry" {
            f.settle(&r, AttemptKind::Initial);
            f.assess(&r, AttemptKind::Initial, true);
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "memory::durable_decision_tests::model_process_endpoint",
                "--nocapture",
            ])
            .env("MECHA_MODEL_PROCESS_PATH", f.path())
            .env("MECHA_MODEL_PROCESS_ID", r.id())
            .env("MECHA_MODEL_PROCESS_MODE", mode)
            .status()
            .unwrap();
        assert!(status.success());
        let mut other = Memory::open(&f.path()).unwrap();
        let rr = other.durable_model_reopen(r.id(), &f.access).unwrap();
        let kind = if mode == "retry" {
            AttemptKind::Retry
        } else {
            AttemptKind::Initial
        };
        assert!(other
            .reserve_model_attempt(&rr, kind, json!({}), &f.access)
            .is_err());
        let state = other.durable_model_status(&rr, &f.access).unwrap();
        let attempt = if mode == "retry" {
            state.retry
        } else {
            state.initial
        };
        if mode == "settled" {
            assert_eq!(attempt.phase, AttemptPhase::Settled);
            other
                .assess_model_attempt(&rr, kind, json!({"status":"abstain"}), false, &f.access)
                .unwrap();
            other
                .finalize_model(&rr, json!({"status":"abstain"}), &f.access)
                .unwrap();
        } else {
            assert_eq!(attempt.phase, AttemptPhase::Indeterminate);
            assert!(other
                .assess_model_attempt(&rr, kind, json!({}), false, &f.access)
                .is_err());
            assert!(other.finalize_model(&rr, json!({}), &f.access).is_err());
        }
    }
}
#[test]
fn racing_connections_issue_one_model_permit_for_each_slot() {
    for kind in [AttemptKind::Initial, AttemptKind::Retry] {
        let mut f = Fixture::new();
        let r = f.bind();
        if kind == AttemptKind::Retry {
            f.settle(&r, AttemptKind::Initial);
            f.assess(&r, AttemptKind::Initial, true);
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let path = f.path();
                let id = r.id().to_owned();
                let access = f.access.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut db = Memory::open(&path).unwrap();
                    let rr = db.durable_model_reopen(&id, &access).unwrap();
                    barrier.wait();
                    db.reserve_model_attempt(&rr, kind, json!({}), &access)
                        .is_ok()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        assert_eq!(
            handles
                .into_iter()
                .filter(|h| h.thread().id() != std::thread::current().id())
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }
}
#[test]
fn stale_source_refuses_assessment_and_finalization_but_cancellation_still_works() {
    for at_final in [false, true] {
        let mut f = Fixture::new();
        let r = f.bind();
        f.settle(&r, AttemptKind::Initial);
        if at_final {
            f.assess(&r, AttemptKind::Initial, false);
        }
        let mut changed = f.source.clone();
        changed.text = "Synthetic newer input".into();
        f.db.capture(changed).unwrap();
        assert!(f
            .db
            .assess_model_attempt(&r, AttemptKind::Initial, json!({}), false, &f.access)
            .is_err());
        assert!(f.db.finalize_model(&r, json!({}), &f.access).is_err());
        assert_eq!(
            f.db.durable_model_cancel(&r, &f.access).unwrap().phase,
            ModelPhase::Cancelled
        );
    }
}
#[test]
fn forgetting_during_pending_call_invalidates_guard_and_rejects_late_bytes() {
    let mut f = Fixture::new();
    let r = f.bind();
    let p =
        f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
            .unwrap();
    let guard = f.db.model_attempt_guard(&p, &f.access).unwrap();
    let mut other = Memory::open(&f.path()).unwrap();
    other.forget(&f.source.source).unwrap();
    assert!(guard.check().is_err());
    assert!(f
        .db
        .settle_model_attempt(
            p,
            HostModelResult::new(json!({"reply":"never retain forgotten private bytes"})),
            &f.access
        )
        .is_err());
    assert_eq!(
        f.db.durable_model_status(&r, &f.access).unwrap().phase,
        ModelPhase::Forgotten
    );
}
#[test]
fn forgetting_erases_raw_bytes_from_database_and_keeps_only_opaque_metadata() {
    let mut f = Fixture::new();
    let r = f.bind();
    let p =
        f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
            .unwrap();
    let sentinel = "SYNTHETIC_ERASURE_SENTINEL_621a73b2";
    f.db.settle_model_attempt(
        p,
        HostModelResult::new(json!({"reply":sentinel})),
        &f.access,
    )
    .unwrap();
    assert!(std::fs::read(f.path())
        .unwrap()
        .windows(sentinel.len())
        .any(|w| w == sentinel.as_bytes()));
    f.db.forget(&f.source.source).unwrap();
    assert!(!std::fs::read(f.path())
        .unwrap()
        .windows(sentinel.len())
        .any(|w| w == sentinel.as_bytes()));
    for suffix in ["-journal", "-wal"] {
        let path = PathBuf::from(format!("{}{suffix}", f.path().display()));
        if path.exists() {
            assert!(!std::fs::read(path)
                .unwrap()
                .windows(sentinel.len())
                .any(|w| w == sentinel.as_bytes()));
        }
    }
}
#[test]
fn oversized_payloads_fail_without_spending_a_new_slot_or_saving_result() {
    let mut f = Fixture::new();
    let r = f.bind();
    assert!(f
        .db
        .reserve_model_attempt(
            &r,
            AttemptKind::Initial,
            json!("x".repeat(48_000)),
            &f.access
        )
        .is_err());
    assert_eq!(
        f.db.durable_model_status(&r, &f.access)
            .unwrap()
            .initial
            .phase,
        AttemptPhase::Unused
    );
    let p =
        f.db.reserve_model_attempt(&r, AttemptKind::Initial, json!({}), &f.access)
            .unwrap();
    assert!(f
        .db
        .settle_model_attempt(
            p,
            HostModelResult::new(json!("x".repeat(64_000))),
            &f.access
        )
        .is_err());
    assert_eq!(
        f.db.durable_model_status(&r, &f.access)
            .unwrap()
            .initial
            .phase,
        AttemptPhase::Indeterminate
    );
}

#[test]
fn finalized_model_binding_is_terminal_through_both_cancellation_surfaces() {
    let mut f = Fixture::new();
    let r = f.bind();
    f.settle(&r, AttemptKind::Initial);
    f.assess(&r, AttemptKind::Initial, false);
    f.db.finalize_model(&r, json!({"status":"recommend"}), &f.access)
        .unwrap();
    assert!(f.db.durable_model_cancel(&r, &f.access).is_err());
    assert!(f.db.durable_cancel(&f.read, &f.access).is_err());
    assert!(!f.db.durable_model_status(&r, &f.access).unwrap().cancelled);
}

#[test]
fn explicitly_versioned_action_store_keeps_existing_model_records_readable() {
    let mut f = Fixture::new();
    let r = f.bind();
    f.db.conn.execute_batch("PRAGMA user_version=5;").unwrap();
    assert!(
        Memory::open(&f.path()).is_ok(),
        "explicit v5 must be readable"
    );
    assert!(
        f.db.durable_model_spec(&r, &f.access).is_ok(),
        "v5 must retain model access"
    );
}
