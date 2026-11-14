use super::*;
use crate::{
    message::{Block, CompletionResponse, Message, StopReason, Usage},
    provider::{Provider, StreamSink},
};
use std::sync::{Arc, Mutex};

fn fixture() -> Value {
    json!({"version":1,"training":[{"id":"train_north","text":"North route was blocked; a clear alternate delivered the crate."},{"id":"train_south","text":"South route was blocked; a clear alternate delivered the crate."}],"counterexample":{"id":"counter_shared","text":"Both routes shared a failed gate; repairing the shared gate delivered the crate."},"cases":(0..6).map(|i|json!({"id":format!("case_{i}"),"phase":if i<3 {"initial"} else {"revised"},"question":format!("Untouched decision question {i}?"),"options":["primary","alternate","insufficient_evidence"],"facts":[{"id":format!("fact_{i}"),"text":format!("Current observation {i}.")}]})).collect::<Vec<_>>()})
}
fn temporary() -> PathBuf {
    let p = std::env::temp_dir().join(format!("concept-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&p).unwrap();
    p.canonicalize().unwrap().join("trial")
}
struct Factory {
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    mode: String,
    target: String,
    root: PathBuf,
}
impl Factory {
    fn new(root: &Path, mode: &str, target: &str) -> Self {
        Self {
            seen: Arc::default(),
            mode: mode.into(),
            target: target.into(),
            root: root.into(),
        }
    }
}
struct Script {
    stage: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    mode: String,
    root: PathBuf,
    cancel: CancellationToken,
}
#[async_trait::async_trait]
impl StageProviders for Factory {
    async fn create(&self, stage: &str, cancel: &CancellationToken) -> Result<Box<dyn Provider>> {
        if stage == self.target && self.mode == "discovery_mutation" {
            mutate(&self.root, "scope")?;
        }
        if stage == self.target && self.mode == "discovery_timeout" {
            std::future::pending::<()>().await;
        }
        Ok(Box::new(Script {
            stage: stage.into(),
            seen: self.seen.clone(),
            mode: if stage == self.target {
                self.mode.clone()
            } else {
                String::new()
            },
            root: self.root.clone(),
            cancel: cancel.clone(),
        }))
    }
}
fn mutate(root: &Path, mode: &str) -> Result<()> {
    if mode == "manifest" {
        fs::write(root.join("induction-phase.json"), b"changed")?;
        return Ok(());
    }
    let conn = rusqlite::Connection::open(root.join("concept.memory-prototype.sqlite"))?;
    match mode {
        "scope" => {
            conn.execute(
                "INSERT INTO scope(revision,name) SELECT id,'forbidden' FROM revision",
                [],
            )?;
        }
        "revision" => {
            conn.execute("UPDATE revision SET lifecycle='withdrawn'", [])?;
        }
        _ => {
            conn.execute(
                "INSERT INTO restriction(object_id) SELECT id FROM object",
                [],
            )?;
        }
    }
    Ok(())
}
fn draft(request: &CompletionRequest, revised: bool) -> Value {
    let prompt = request.messages[0].text();
    let pos = prompt.rfind("\n[").unwrap();
    let sources: Value = serde_json::from_str(&prompt[pos + 1..]).unwrap();
    let rows = sources.as_array().unwrap();
    let selection =
        |r: &Value| json!({"source_id":r["id"],"field":"","quote":r["fields"][0]["text"]});
    let is_counter = |r: &&Value| {
        let text = r["fields"][0]["text"].as_str().unwrap();
        text.starts_with("Both routes") || text.starts_with("Synthetic counterexample")
    };
    let originals: Vec<_> = rows.iter().filter(|r| !is_counter(r)).collect();
    let counters: Vec<_> = rows.iter().filter(is_counter).map(selection).collect();
    json!({"name":"Alternate dependency","principle":"An alternate route can preserve delivery.","applicability":if revised {"Only when the shared gate is ready."} else {"Blocked primary routes."},"support":originals.into_iter().map(selection).collect::<Vec<_>>(),"counterexamples":counters})
}
#[async_trait::async_trait]
impl Provider for Script {
    fn id(&self) -> &str {
        "script"
    }
    fn default_model(&self) -> &str {
        MODEL
    }
    fn local(&self) -> bool {
        true
    }
    fn structured_output(&self) -> bool {
        true
    }
    async fn complete(
        &self,
        r: &CompletionRequest,
        _: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        self.seen
            .lock()
            .unwrap()
            .push((self.stage.clone(), request_value(r)));
        assert!(r.local_only && r.tools.is_empty() && r.think == Some(false));
        let durable: Value =
            serde_json::from_slice(&fs::read(self.root.join("report.json"))?).unwrap();
        assert_eq!(durable["stages"].as_array().unwrap().len(), 14);
        assert_eq!(row(&durable, &self.stage)["status"], "started");
        if self.mode == "timeout" {
            std::future::pending::<()>().await;
        }
        if self.mode == "cancel" {
            self.cancel.cancel();
            std::future::pending::<()>().await;
        }
        if self.mode.starts_with("mutate_") {
            mutate(&self.root, self.mode.trim_start_matches("mutate_"))?;
            if self.mode == "mutate_scope_error" {
                anyhow::bail!("transport failed after mutation");
            }
        }
        if self.mode == "export_failure" {
            fs::create_dir(self.root.join("initial-concept.json"))?;
        }
        let value = if self.stage == "induction" {
            draft(r, false)
        } else if self.stage == "rumination" {
            if self.mode == "unchanged" || self.mode == "abstained" {
                json!({"outcome":self.mode,"reason":"No supported change."})
            } else {
                json!({"outcome":"revised_provisional","draft":draft(r,true)})
            }
        } else {
            let packet: Value = serde_json::from_str(&r.messages[0].text()).unwrap();
            let id = packet["evidence"]["facts"][0]["id"].clone();
            json!({"request_key":packet["request_key"],"choice":"alternate","because":[id],"reason":"The observed premises determine the route.","question":null})
        };
        Ok(CompletionResponse {
            message: Message::assistant(vec![Block::text(if self.mode == "malformed" {
                "not JSON".into()
            } else if self.mode == "oversize" {
                "x".repeat(16385)
            } else {
                value.to_string()
            })]),
            stop_reason: if self.mode == "refused" {
                StopReason::Refusal
            } else {
                StopReason::EndTurn
            },
            usage: Usage {
                input_tokens: 17,
                output_tokens: 9,
                ..Usage::default()
            },
            refusal: None,
            model: if self.mode == "model_mismatch" {
                "wrong"
            } else {
                MODEL
            }
            .into(),
            malformed_tool_args: 0,
        })
    }
}
async fn run(mode: &str, target: &str) -> (PathBuf, Factory, Value) {
    let root = temporary();
    let f = Factory::new(&root, mode, target);
    let report = run_trial(
        &root,
        &fixture(),
        &f,
        GenerationBudget::new(512, Duration::from_millis(80)).unwrap(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    (root, f, report)
}
fn row<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()
}
fn clean(root: PathBuf) {
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}
#[tokio::test]
async fn complete_lifecycle_retains_reopens_and_keeps_pairs_equal_without_future_leakage() {
    let (root, f, r) = run("", "").await;
    assert_eq!(r["stages"].as_array().unwrap().len(), 14);
    assert_eq!(r["actual_calls"], 14, "{r:#}");
    assert_eq!(r["revision_outcome"], "revised", "{r:#}");
    assert!(
        r["stages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["status"] == "accepted"),
        "{r:#}"
    );
    for stage in r["stages"].as_array().unwrap() {
        assert_eq!(stage["source_hashes_before"], stage["source_hashes_after"]);
        assert_eq!(
            stage["source_hashes_before"],
            stage["source_hashes_dispatch"]
        );
        assert_eq!(stage["post_source_check"], "accepted");
    }
    let seen = f.seen.lock().unwrap();
    for (stage, request) in seen
        .iter()
        .filter(|(s, _)| s == "induction" || s == "rumination")
    {
        let text = request.to_string();
        assert!(!text.contains("Untouched decision question"), "{stage}");
        assert!(!text.contains("fact_"));
        if stage == "induction" {
            assert!(!text.contains("Both routes"));
        }
    }
    for i in 0..6 {
        let phase = if i < 3 { "initial" } else { "revised" };
        let packets: Vec<Value> = ["baseline", "concept"]
            .into_iter()
            .map(|a| {
                let id = format!("{phase}.case_{i}.{a}");
                let (_, q) = seen.iter().find(|(s, _)| s == &id).unwrap();
                serde_json::from_str(q["messages"][0]["content"][0]["text"].as_str().unwrap())
                    .unwrap()
            })
            .collect();
        assert_eq!(packets[0]["evidence"], packets[1]["evidence"]);
        assert!(packets[0]["provisional_concept"].is_null());
        assert!(packets[1]["provisional_concept"].is_object());
        assert_eq!(
            packets[0]["evidence"]["episodes"].as_array().unwrap().len(),
            if i < 3 { 2 } else { 3 }
        );
    }
    let db = Memory::open(&root.join("concept.memory-prototype.sqlite")).unwrap();
    let reference: RevisionRef =
        serde_json::from_value(row(&r, "rumination")["retention"]["reference"].clone()).unwrap();
    let Some(Record::Concept(c)) = db.read(&reference, &access()).unwrap() else {
        panic!()
    };
    assert_eq!(c.consulted.len(), 3);
    assert_eq!(c.counterexamples.len(), 1);
    assert_eq!(
        c.counterexamples[0].quote,
        fixture()["counterexample"]["text"].as_str().unwrap()
    );
    drop(db);
    drop(seen);
    clean(root);
}
#[tokio::test]
async fn all_slots_survive_induction_envelope_and_structural_failures_without_retries() {
    for mode in [
        "malformed",
        "refused",
        "model_mismatch",
        "timeout",
        "oversize",
        "discovery_timeout",
    ] {
        let (root, _, r) = run(mode, "induction").await;
        assert_eq!(r["stages"].as_array().unwrap().len(), 14);
        assert_eq!(row(&r, "induction")["status"], "failed", "{mode}: {r:#}");
        assert_eq!(row(&r, "rumination")["status"], "withheld");
        assert_eq!(
            r["actual_calls"],
            if mode == "discovery_timeout" { 0 } else { 1 }
        );
        if ["malformed", "refused", "model_mismatch", "oversize"].contains(&mode) {
            assert!(row(&r, "induction")["response_path"].is_string());
        }
        clean(root);
    }
}
#[tokio::test]
async fn revision_failure_withholds_only_postphase_and_bad_decision_allows_sibling() {
    let (root, _, r) = run("malformed", "rumination").await;
    assert_eq!(r["actual_calls"], 8);
    assert_eq!(row(&r, "revised.case_3.baseline")["status"], "withheld");
    clean(root);
    let (root, _, r) = run("malformed", "initial.case_0.baseline").await;
    assert_eq!(r["actual_calls"], 14);
    assert_eq!(row(&r, "initial.case_0.concept")["status"], "accepted");
    clean(root);
}
#[tokio::test]
async fn unchanged_and_abstained_keep_old_revision_and_do_not_claim_revision_benefit() {
    for mode in ["unchanged", "abstained"] {
        let (root, _, r) = run(mode, "rumination").await;
        assert_eq!(r["actual_calls"], 14);
        assert_eq!(r["revision_outcome"], mode);
        assert_eq!(
            row(&r, "induction")["retention"],
            row(&r, "rumination")["retention"]["reference"]
        );
        clean(root);
    }
}
#[tokio::test]
async fn cancellation_stops_new_dispatch_and_keeps_started_audit() {
    let (root, _, r) = run("cancel", "induction").await;
    assert_eq!(r["cancelled"], true);
    assert_eq!(r["actual_calls"], 1);
    assert_eq!(row(&r, "rumination")["status"], "withheld");
    assert!(fs::read_to_string(root.join("induction-provider.jsonl"))
        .unwrap()
        .contains("request_started"));
    clean(root);
}
#[tokio::test]
async fn immutable_phase_and_current_authorization_block_discovery_and_inference_mutations() {
    for (mode, target) in [
        ("discovery_mutation", "induction"),
        ("mutate_scope", "induction"),
        ("mutate_revision", "induction"),
        ("mutate_source", "induction"),
        ("mutate_manifest", "induction"),
        ("mutate_scope_error", "induction"),
        ("mutate_scope", "initial.case_0.baseline"),
        ("mutate_revision", "rumination"),
    ] {
        let (root, _, r) = run(mode, target).await;
        assert_eq!(row(&r, target)["status"], "failed", "{mode}: {r:#}");
        assert!(row(&r, target)["retention"].is_null());
        assert_eq!(
            row(&r, target)["post_source_check"],
            "rejected",
            "{mode} {target}: {r:#}"
        );
        assert_ne!(
            row(&r, target)["source_hashes_before"],
            row(&r, target)["source_hashes_after"]
        );
        if mode == "discovery_mutation" {
            assert_eq!(r["actual_calls"], 0);
        }
        if target == "induction" {
            assert_eq!(row(&r, "rumination")["status"], "withheld");
        }
        clean(root);
    }
}
#[tokio::test]
async fn successful_retention_survives_a_later_derived_export_failure() {
    let (root, _, r) = run("export_failure", "induction").await;
    assert!(row(&r, "induction")["retention"].is_object(), "{r:#}");
    let db = Memory::open(&root.join("concept.memory-prototype.sqlite")).unwrap();
    let reference: RevisionRef =
        serde_json::from_value(row(&r, "induction")["retention"].clone()).unwrap();
    assert!(matches!(
        db.read(&reference, &access()).unwrap(),
        Some(Record::Concept(_))
    ));
    assert!(!r["export_errors"].as_array().unwrap().is_empty());
    drop(db);
    clean(root);
}

/// One-shot authored transport/schema artifact for offline scorer integration.
/// Explicitly ignored: ordinary tests must not populate qualification results.
#[tokio::test]
#[ignore = "writes a fresh authored-only qualification cohort; no model calls"]
async fn write_authored_public_fixture_report_for_offline_integration() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let parent = checkout.join("results/memory-concept-benefit-20261108");
    fs::create_dir_all(&parent).unwrap();
    let root = parent.canonicalize().unwrap().join("authored");
    let public: Value = serde_json::from_slice(include_bytes!(
        "../memory/fixtures/concept_benefit_pilot.json"
    ))
    .unwrap();
    let factory = Factory::new(&root, "", "");
    let report = run_trial(
        &root,
        &public,
        &factory,
        GenerationBudget::new(6144, Duration::from_secs(540)).unwrap(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(report["status"], "completed", "{report:#}");
    assert_eq!(report["actual_calls"], 14);
    eprintln!(
        "Authored report only: {}",
        root.join("report.json").display()
    );
}

#[test]
fn decision_contract_rejects_forged_duplicate_foreign_or_missing_fields() {
    let c: DecisionCase = serde_json::from_value(fixture()["cases"][0].clone()).unwrap();
    let ids = vec!["fact_0".into()];
    let valid = json!({"request_key":"bound","choice":"alternate","because":["fact_0"],"reason":"Bounded explanation.","question":null});
    assert!(parse_decision(&valid.to_string(), "bound", &c, &ids).is_ok());
    for mode in 0..8 {
        let mut v = valid.clone();
        match mode {
            0 => {
                v["request_key"] = json!("forged");
            }
            1 => {
                v["because"] = json!(["fact_0", "fact_0"]);
            }
            2 => {
                v["because"] = json!(["Alternate dependency"]);
            }
            3 => {
                v["choice"] = json!("unoffered");
            }
            4 => {
                v["reason"] = json!("x".repeat(513));
            }
            5 => {
                v.as_object_mut().unwrap().remove("question");
            }
            6 => {
                v["choice"] = json!("insufficient_evidence");
            }
            _ => {
                v["authority"] = json!(true);
            }
        }
        assert!(
            parse_decision(&v.to_string(), "bound", &c, &ids).is_err(),
            "{v}"
        );
    }
    let duplicate = r#"{"request_key":"bound","request_key":"bound","choice":"alternate","because":[],"reason":"A reason","question":null}"#;
    assert!(parse_decision(duplicate, "bound", &c, &ids).is_err());
}
#[tokio::test]
async fn precancelled_trial_is_recorded_without_dispatch_and_root_cannot_resume() {
    let root = temporary();
    let factory = Factory::new(&root, "", "");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let budget = GenerationBudget::new(512, Duration::from_secs(1)).unwrap();
    let report = run_trial(&root, &fixture(), &factory, budget, &cancel)
        .await
        .unwrap();
    assert_eq!(report["actual_calls"], 0);
    assert_eq!(report["stages"].as_array().unwrap().len(), 14);
    assert_eq!(report["pairs"].as_array().unwrap().len(), 6);
    assert!(factory.seen.lock().unwrap().is_empty());
    assert!(run_trial(
        &root,
        &fixture(),
        &factory,
        budget,
        &CancellationToken::new()
    )
    .await
    .is_err());
    clean(root);
}
