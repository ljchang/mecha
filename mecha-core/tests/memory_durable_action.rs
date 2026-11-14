//! Independent process/lifecycle qualification: synthetic evidence and scripted replies only.
#![cfg(feature = "memory-prototype")]
use mecha_core::{
    agent::{Conversation, RunContext},
    config::PermissionMode,
    memory::{
        decision::{Contract, EvidenceView, Objective},
        durable::TaskSpec,
        durable_action::{ActionPhase, ActionReceipt},
        model_evaluation::ModelReply,
        Access, Capture, Memory, RecallQuery, SourceKey,
    },
    memory_action_runtime::{self as action, ActionSpec, FixtureState, FixtureVersion, WORKLOAD},
    memory_decision_runtime::{self as model, ModelSpec},
    memory_runtime,
    tool::{ModeApprover, ToolCtx},
};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
const MODEL: &str = "synthetic-action-scripted";
const CANARY: &str = "SYNTHETIC_ACTION_QUALIFICATION_CANARY";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mecha-action-qual-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn db(&self) -> PathBuf {
        self.0.join("synthetic.memory-prototype.sqlite")
    }
    fn open(&self) -> Memory {
        Memory::open(&self.db()).unwrap()
    }
    async fn model(&self, kind: &str) -> String {
        let mut db = Memory::create_durable_action(&self.db(), &access().domain).unwrap();
        let input = db.capture(capture(CANARY)).unwrap();
        // The independently bound fixture actually uses 9 energy for Nori.
        std::fs::write(self.0.join("measurements.json"),json!({
            "kind":"host_authored_synthetic_measurements_v1","workload":WORKLOAD,"complete":true,
            "observations":[{"choice":"Nori","observed_minutes":4,"energy":2,"quality":8},
                {"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},
                {"choice":"Quill","observed_minutes":6,"energy":5,"quality":7}]
        }).to_string()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: self.0.clone(),
                    path: "measurements.json".into(),
                    max_bytes: 4096,
                    max_lines: 4,
                },
                &[input],
                &access(),
            )
            .unwrap();
        let cx = RunContext::new(
            ToolCtx {
                workspace: self.0.clone(),
                ..ToolCtx::default()
            },
            Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            }),
        );
        memory_runtime::acquire(&mut db, &read, &access(), &cx, &mut Conversation::new())
            .await
            .unwrap();
        let receipt = model::bind(
            &mut db,
            &read,
            ModelSpec {
                contract: Contract {
                    workload: WORKLOAD.into(),
                    objective: Objective::MinimizeMinutes,
                    energy_budget: 5,
                    minimum_quality: 6,
                },
                model: MODEL.into(),
                seed: 7,
                view: EvidenceView::Observed,
            },
            &access(),
        )
        .unwrap();
        model::decide(
            &mut db,
            &receipt,
            &access(),
            |_, schema, guard| async move {
                guard.check()?;
                let description = schema.as_ref().unwrap()["description"].as_str().unwrap();
                let citations: Value = serde_json::from_str(
                    description.strip_prefix("Allowed citation IDs: ").unwrap(),
                )
                .unwrap();
                let text = match kind {
                    "failed" => "synthetic malformed reply".to_owned(),
                    "abstain" => {
                        json!({"choice":"abstain","citations":[],"reason":CANARY}).to_string()
                    }
                    "withheld" => {
                        json!({"choice":"Nori","citations":[],"reason":CANARY}).to_string()
                    }
                    _ => json!({"choice":"Nori","citations":citations,"reason":CANARY}).to_string(),
                };
                Ok(ModelReply {
                    text,
                    model: MODEL.into(),
                    input_tokens: None,
                    output_tokens: None,
                    error: None,
                })
            },
        )
        .await
        .unwrap();
        receipt.id().into()
    }
    async fn action(&self, state: FixtureState) -> String {
        let id = self.model("recommend").await;
        let mut db = self.open();
        let receipt = db.durable_model_reopen(&id, &access()).unwrap();
        action::bind(&mut db, &receipt, spec(state), &access())
            .unwrap()
            .id()
            .into()
    }
    fn receipt(&self, db: &Memory, id: &str) -> ActionReceipt {
        db.durable_action_reopen(id, &access()).unwrap()
    }
    fn count(&self) -> usize {
        std::fs::read_to_string(self.0.join("handler-count"))
            .unwrap_or_default()
            .lines()
            .count()
    }
    fn child(&self, id: &str, mode: &str) -> ChildGuard {
        ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "durable_action_process_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("MECHA_ACTION_QUAL_ROOT", &self.0)
                .env("MECHA_ACTION_QUAL_ID", id)
                .env("MECHA_ACTION_QUAL_MODE", mode)
                .env("MECHA_SESSION_KIND", "test")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        )
    }
    fn wait_barrier(&self, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.0.join("barrier").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before barrier"
            );
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child barrier timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
struct ChildGuard(Child);
impl std::ops::Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn access() -> Access {
    Access {
        domain: "synthetic-action-qualification".into(),
        scopes: ["fixture-private".into()].into(),
    }
}
fn source() -> SourceKey {
    SourceKey {
        connector: "synthetic".into(),
        account: "action-qualification".into(),
        id: "prerequisite".into(),
    }
}
fn capture(text: &str) -> Capture {
    Capture {
        source: source(),
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    }
}
fn spec(state: FixtureState) -> ActionSpec {
    ActionSpec {
        fixture: FixtureVersion::MeasurementsV1,
        state,
    }
}
fn recall(db: &Memory) -> Vec<mecha_core::memory::RecallHit> {
    db.recall(
        &RecallQuery {
            text: "Nori".into(),
            known_at: db.snapshot().unwrap(),
            account: None,
            session_id: None,
            occurred_after: None,
            occurred_before: None,
            limit: 100,
            neighbors: 0,
            max_bytes: 100_000,
        },
        &access(),
    )
    .unwrap()
}
fn invalidate(db: &mut Memory, change: &str) {
    match change {
        "changed" => {
            db.capture(capture("Synthetic corrected prerequisite"))
                .unwrap();
        }
        "restricted" => {
            let mut c = capture(CANARY);
            c.scopes = ["different-scope".into()].into();
            db.capture(c).unwrap();
        }
        "forgotten" => {
            db.forget(&source()).unwrap();
        }
        _ => unreachable!(),
    }
}
#[test]
#[ignore = "self-spawned synthetic process-boundary helper"]
fn durable_action_process_helper() {
    let Ok(root) = std::env::var("MECHA_ACTION_QUAL_ROOT") else {
        return;
    };
    let f = Fixture(PathBuf::from(root));
    let id = std::env::var("MECHA_ACTION_QUAL_ID").unwrap();
    let mode = std::env::var("MECHA_ACTION_QUAL_MODE").unwrap();
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    let pending = action::reserve(&mut db, &receipt, &access()).unwrap();
    if mode != "reserved" {
        let executed = pending.execute().unwrap();
        // A completion marker after the opaque finite handler; not a predicted count.
        let mut count = OpenOptions::new()
            .create(true)
            .append(true)
            .open(f.0.join("handler-count"))
            .unwrap();
        writeln!(count, "completed host handler").unwrap();
        count.sync_all().unwrap();
        if mode == "observed" {
            action::settle(&mut db, executed, &access()).unwrap();
        }
    }
    std::fs::write(f.0.join("barrier"), "safe point reached").unwrap();
    loop {
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[tokio::test]
async fn actual_process_loss_and_concurrent_resumer_never_repeat_effect() {
    let mut audit = Vec::new();
    for mode in ["reserved", "effected", "observed"] {
        let f = Fixture::new();
        let id = f.action(FixtureState::Baseline).await;
        let mut child = f.child(&id, mode);
        f.wait_barrier(&mut child);
        let count = usize::from(mode != "reserved");
        assert_eq!(f.count(), count);
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        let before = db.durable_action_status(&receipt, &access()).unwrap();
        if mode != "observed" {
            assert!(action::reserve(&mut db, &receipt, &access()).is_err());
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        drop(db);
        std::fs::remove_file(f.0.join("measurements.json")).unwrap();
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        let status = action::run(&mut db, &receipt, &access()).unwrap();
        assert_eq!(
            status.phase,
            if mode == "observed" {
                ActionPhase::Finalized
            } else {
                ActionPhase::Indeterminate
            }
        );
        assert_eq!(f.count(), count, "replayed effect after {mode}");
        assert_eq!(
            status.observation, before.observation,
            "observation revision changed on recovery"
        );
        audit.push(json!({"loss_boundary": mode, "completed_host_handler_markers_before_kill": count,
            "completed_host_handler_markers_after_resume": f.count(), "phase_after_resume": status.phase,
            "observation_revision": status.observation, "feedback_settled": status.feedback.is_some()}));
        assert_eq!(
            db.durable_action_observation(&receipt, &access())
                .unwrap()
                .is_some(),
            mode == "observed"
        );
        assert_eq!(
            db.durable_action_feedback(&receipt, &access())
                .unwrap()
                .is_some(),
            mode == "observed"
        );
    }
    println!("ACTION_QUALIFICATION_PROCESS_COUNTS={}", json!(audit));
    if let Ok(path) = std::env::var("MECHA_ACTION_PROCESS_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&audit).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn no_action_decisions_and_stale_original_sources_cannot_bind() {
    for kind in ["abstain", "withheld", "failed"] {
        let f = Fixture::new();
        let id = f.model(kind).await;
        let mut db = f.open();
        let model = db.durable_model_reopen(&id, &access()).unwrap();
        assert!(
            action::bind(&mut db, &model, spec(FixtureState::Baseline), &access()).is_err(),
            "{kind}"
        );
    }
    for change in ["changed", "restricted", "forgotten"] {
        let f = Fixture::new();
        let id = f.model("recommend").await;
        let mut db = f.open();
        let model = db.durable_model_reopen(&id, &access()).unwrap();
        invalidate(&mut db, change);
        assert!(
            action::bind(&mut db, &model, spec(FixtureState::Baseline), &access()).is_err(),
            "{change}"
        );
    }
}
#[tokio::test]
async fn one_binding_per_decision_and_exact_access_are_required() {
    let f = Fixture::new();
    let id = f.model("recommend").await;
    let mut db = f.open();
    let model = db.durable_model_reopen(&id, &access()).unwrap();
    let receipt = action::bind(&mut db, &model, spec(FixtureState::Baseline), &access()).unwrap();
    assert!(action::bind(&mut db, &model, spec(FixtureState::Baseline), &access()).is_err());
    for wrong in [
        Access {
            domain: "wrong-domain".into(),
            scopes: access().scopes,
        },
        Access {
            domain: access().domain,
            scopes: Default::default(),
        },
    ] {
        assert!(db.durable_action_reopen(receipt.id(), &wrong).is_err());
        assert!(action::reserve(&mut db, &receipt, &wrong).is_err());
    }
    assert_eq!(
        db.durable_action_status(&receipt, &access()).unwrap().phase,
        ActionPhase::Ready
    );
}
#[tokio::test]
async fn cancellation_checks_before_handler_and_preserves_already_executed_outcome() {
    for executed in [false, true] {
        let f = Fixture::new();
        let id = f.action(FixtureState::Baseline).await;
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        let pending = action::reserve(&mut db, &receipt, &access()).unwrap();
        if executed {
            let outcome = pending.execute().unwrap();
            let mut other = f.open();
            let other_receipt = f.receipt(&other, &id);
            other
                .durable_action_cancel(&other_receipt, &access())
                .unwrap();
            action::settle(&mut db, outcome, &access()).unwrap();
            assert!(db
                .durable_action_observation(&receipt, &access())
                .unwrap()
                .is_some());
        } else {
            db.durable_action_cancel(&receipt, &access()).unwrap();
            assert!(pending.execute().is_err());
            assert!(db
                .durable_action_observation(&receipt, &access())
                .unwrap()
                .is_none());
        }
        assert_eq!(
            action::run(&mut db, &receipt, &access()).unwrap().phase,
            ActionPhase::Cancelled
        );
        assert!(db
            .durable_action_feedback(&receipt, &access())
            .unwrap()
            .is_none());
    }
}
#[tokio::test]
async fn changed_restricted_forgotten_sources_deny_late_settlement_and_recall() {
    for change in ["changed", "restricted", "forgotten"] {
        for executed in [false, true] {
            let f = Fixture::new();
            let id = f.action(FixtureState::Baseline).await;
            let mut db = f.open();
            let receipt = f.receipt(&db, &id);
            let pending = action::reserve(&mut db, &receipt, &access()).unwrap();
            if executed {
                let outcome = pending.execute().unwrap();
                invalidate(&mut db, change);
                assert!(action::settle(&mut db, outcome, &access()).is_err());
            } else {
                invalidate(&mut db, change);
                assert!(pending.execute().is_err());
            }
            assert!(db.durable_action_observation(&receipt, &access()).is_err());
            assert!(db.durable_action_feedback(&receipt, &access()).is_err());
            assert!(
                recall(&db).is_empty(),
                "inaccessible closure leaked via recall: {change}"
            );
            let status = db.durable_action_status(&receipt, &access()).unwrap();
            assert_eq!(
                status.phase,
                if change == "forgotten" {
                    ActionPhase::Forgotten
                } else {
                    ActionPhase::Indeterminate
                }
            );
        }
    }
}
#[tokio::test]
async fn independent_observation_disagrees_with_source_and_drives_feedback_and_recall() {
    let f = Fixture::new();
    let id = f.action(FixtureState::Baseline).await;
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    assert_eq!(
        action::run(&mut db, &receipt, &access()).unwrap().phase,
        ActionPhase::Finalized
    );
    let observed = db
        .durable_action_observation(&receipt, &access())
        .unwrap()
        .unwrap();
    assert_eq!(
        observed["observation"]["expected_from_evidence"]["energy"],
        2
    );
    assert_eq!(observed["observation"]["outcome"]["energy"], 9);
    let feedback = db
        .durable_action_feedback(&receipt, &access())
        .unwrap()
        .unwrap();
    assert_eq!(feedback["resource_constraint_met"], false);
    assert_eq!(feedback["quality_constraint_met"], true);
    assert_eq!(feedback["step_feedback"]["verification"], "failed");
    for field in ["goal", "expected_calls", "actual_calls"] {
        assert!(feedback["step_feedback"][field].is_null());
    }
    let hits = recall(&db);
    let status = db.durable_action_status(&receipt, &access()).unwrap();
    assert!(hits
        .iter()
        .any(|h| Some(&h.anchor.reference) == status.observation.as_ref()));
    assert!(hits
        .iter()
        .all(|h| h.anchor.private_data && h.anchor.untrusted_input));
    drop(db);
    std::fs::remove_file(f.0.join("measurements.json")).unwrap();
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    action::run(&mut db, &receipt, &access()).unwrap();
    assert_eq!(
        db.durable_action_observation(&receipt, &access())
            .unwrap()
            .unwrap(),
        observed
    );
    db.forget(&source()).unwrap();
    assert!(recall(&db).is_empty());
    assert!(db.capture(capture(CANARY)).is_err());
}
#[tokio::test]
async fn refused_and_unknown_never_become_numeric_measurement_or_measured_failure() {
    for (state, status, verification) in [
        (FixtureState::Refused, "refused", "refused"),
        (FixtureState::Unknown, "unknown", "pending"),
    ] {
        let f = Fixture::new();
        let id = f.action(state).await;
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        action::run(&mut db, &receipt, &access()).unwrap();
        let observed = db
            .durable_action_observation(&receipt, &access())
            .unwrap()
            .unwrap();
        let outcome = &observed["observation"]["outcome"];
        assert_eq!(outcome["status"], status);
        for field in ["energy", "quality", "observed_minutes"] {
            assert!(outcome.get(field).is_none() || outcome[field].is_null());
        }
        let feedback = db
            .durable_action_feedback(&receipt, &access())
            .unwrap()
            .unwrap();
        assert_eq!(feedback["observed"], false);
        assert_eq!(feedback["step_feedback"]["verification"], verification);
        assert!(feedback["resource_constraint_met"].is_null());
        assert!(feedback["quality_constraint_met"].is_null());
    }
}

#[tokio::test]
async fn finalized_siblings_disappear_after_source_revocation_and_forget_erases_bytes() {
    for change in ["changed", "restricted", "forgotten"] {
        let f = Fixture::new();
        let id = f.action(FixtureState::Baseline).await;
        let mut db = f.open();
        let receipt = f.receipt(&db, &id);
        action::run(&mut db, &receipt, &access()).unwrap();
        assert!(!recall(&db).is_empty());
        invalidate(&mut db, change);
        assert!(db.durable_action_spec(&receipt, &access()).is_err());
        assert!(db.durable_action_observation(&receipt, &access()).is_err());
        assert!(db.durable_action_feedback(&receipt, &access()).is_err());
        assert!(
            recall(&db).is_empty(),
            "finalized closure recalled after {change}"
        );
        let status =
            serde_json::to_string(&db.durable_action_status(&receipt, &access()).unwrap()).unwrap();
        assert!(!status.contains(CANARY));
        drop(db);
        if change == "forgotten" {
            for entry in std::fs::read_dir(&f.0).unwrap() {
                let path = entry.unwrap().path();
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("sqlite")
                {
                    assert!(!std::fs::read(path)
                        .unwrap()
                        .windows(CANARY.len())
                        .any(|w| w == CANARY.as_bytes()));
                }
            }
        }
    }
}

#[tokio::test]
async fn a_forged_final_label_does_not_override_the_retained_model_response() {
    for kind in ["abstain", "failed", "recommend"] {
        let f = Fixture::new();
        let id = f.model(kind).await;
        let db = f.open();
        let model = db.durable_model_reopen(&id, &access()).unwrap();
        let revision = db
            .durable_model_status(&model, &access())
            .unwrap()
            .decision
            .unwrap()
            .revision;
        drop(db);
        // Deliberately corrupt a persisted record to test independent reconstruction.
        // This is not a supported write API or a claim of tamper-proof SQLite storage.
        let conn = rusqlite::Connection::open(f.db()).unwrap();
        let payload: String = conn
            .query_row(
                "SELECT payload FROM revision WHERE id=?1",
                [&revision],
                |r| r.get(0),
            )
            .unwrap();
        let mut record: Value = serde_json::from_str(&payload).unwrap();
        let mut decision: Value =
            serde_json::from_str(record["data"]["text"].as_str().unwrap()).unwrap();
        decision["disposition"] = json!("recommend");
        decision["proposed_choice"] = json!("Arden");
        record["data"]["text"] = json!(decision.to_string());
        conn.execute(
            "UPDATE revision SET payload=?1 WHERE id=?2",
            rusqlite::params![record.to_string(), revision],
        )
        .unwrap();
        drop(conn);
        let mut db = f.open();
        let model = db.durable_model_reopen(&id, &access()).unwrap();
        assert!(
            action::bind(&mut db, &model, spec(FixtureState::Baseline), &access()).is_err(),
            "forged {kind}"
        );
    }
}

#[tokio::test]
async fn simultaneous_connections_reserve_exactly_one_host_execution() {
    let f = Fixture::new();
    let id = f.action(FixtureState::Baseline).await;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let path = f.db();
        let id = id.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut db = Memory::open(&path).unwrap();
            let receipt = db.durable_action_reopen(&id, &access()).unwrap();
            barrier.wait();
            match action::reserve(&mut db, &receipt, &access()) {
                Ok(pending) => {
                    let executed = pending.execute().unwrap();
                    action::settle(&mut db, executed, &access()).unwrap();
                    1
                }
                Err(_) => 0,
            }
        }));
    }
    let completed: usize = workers.into_iter().map(|w| w.join().unwrap()).sum();
    assert_eq!(
        completed, 1,
        "concurrent reservations duplicated or lost the action"
    );
    println!("ACTION_QUALIFICATION_CONCURRENT_HANDLER_COMPLETIONS={completed}");
    let mut db = f.open();
    let receipt = f.receipt(&db, &id);
    assert_eq!(
        action::run(&mut db, &receipt, &access()).unwrap().phase,
        ActionPhase::Finalized
    );
}
