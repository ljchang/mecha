use super::*;
use crate::{
    memory::{
        decision::{EvidenceView, Objective},
        durable::{HostObservation, TaskSpec},
        Capture, SourceKey,
    },
    memory_action_runtime::{self as action, ActionSpec, FixtureState, FixtureVersion, WORKLOAD},
    memory_decision_runtime::{self as model, ModelSpec},
};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    db: Memory,
    access: Access,
    action: ActionReceipt,
    source: Capture,
}
impl Fixture {
    async fn new(state: FixtureState) -> Self {
        Self::with_completion(state, true).await
    }
    async fn with_completion(state: FixtureState, complete: bool) -> Self {
        let root = std::env::temp_dir().join(format!("mecha-reappraisal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let access = Access {
            domain: "synthetic-reappraisal".into(),
            scopes: ["fixture".into()].into(),
        };
        let mut db = Memory::create_durable_action(
            &root.join("test.memory-prototype.sqlite"),
            &access.domain,
        )
        .unwrap();
        let source = Capture {
            source: SourceKey {
                connector: "synthetic".into(),
                account: "test".into(),
                id: "source".into(),
            },
            text: "Synthetic outcome reappraisal source".into(),
            occurred_at: None,
            scopes: access.scopes.clone(),
            transcript: None,
        };
        let input = db.capture(source.clone()).unwrap();
        let read = db
            .durable_create(
                TaskSpec {
                    workspace: root.clone(),
                    path: "measurements.json".into(),
                    max_bytes: 16000,
                    max_lines: 128,
                },
                &[input],
                &access,
            )
            .unwrap();
        let content=json!({"kind":"host_authored_synthetic_measurements_v1","workload":WORKLOAD,"complete":true,"observations":[{"choice":"Nori","observed_minutes":4,"energy":2,"quality":8},{"choice":"Arden","observed_minutes":8,"energy":3,"quality":9},{"choice":"Quill","observed_minutes":6,"energy":5,"quality":7}]}).to_string();
        let permit = db.durable_reserve_dispatch(&read, &access).unwrap();
        db.durable_settle(
            permit,
            HostObservation::from_dispatch(
                json!({"status":"successful","content":content,"cancelled":false}),
            ),
            &access,
        )
        .unwrap();
        let m = model::bind(
            &mut db,
            &read,
            ModelSpec {
                contract: contract(5, 7),
                model: "scripted-fixture".into(),
                seed: 7,
                view: EvidenceView::Observed,
            },
            &access,
        )
        .unwrap();
        model::decide(&mut db, &m, &access, |_, _, guard| async move {
            guard.check()?;
            Ok(reply("Nori", 3))
        })
        .await
        .unwrap();
        let action = action::bind(
            &mut db,
            &m,
            ActionSpec {
                fixture: FixtureVersion::MeasurementsV1,
                state,
            },
            &access,
        )
        .unwrap();
        if complete {
            action::run(&mut db, &action, &access).unwrap();
        }
        Self {
            root,
            db,
            access,
            action,
            source,
        }
    }
    fn path(&self) -> PathBuf {
        self.root.join("test.memory-prototype.sqlite")
    }
    fn query(&self) -> RecallQuery {
        RecallQuery {
            text: "Nori".into(),
            known_at: self.db.snapshot().unwrap(),
            account: None,
            session_id: None,
            occurred_after: None,
            occurred_before: None,
            limit: 100,
            neighbors: 0,
            max_bytes: 100000,
        }
    }
    fn pair(&self, budget: u32, quality: u32) -> PreparedPair {
        prepare_pair(
            &self.db,
            &self.action,
            &contract(budget, quality),
            &self.query(),
            &self.access,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn contract(budget: u32, quality: u32) -> Contract {
    Contract {
        workload: WORKLOAD.into(),
        objective: Objective::MinimizeMinutes,
        energy_budget: budget,
        minimum_quality: quality,
    }
}
fn reply(choice: &str, count: usize) -> ModelReply {
    ModelReply{text:json!({"choice":choice,"citations":(1..=count).map(|i|format!("e{i}")).collect::<Vec<_>>(),"reason":"Comparison uses visible same-workload measurements"}).to_string(),model:"scripted-fixture".into(),input_tokens:None,output_tokens:None,error:None}
}

// Replacing the source-only control with an empty or modified source breaks this.
#[tokio::test]
async fn both_arms_preserve_expectations_and_only_treatment_adds_actual() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let bytes = std::fs::read(f.path()).unwrap();
    let pair = f.pair(4, 7);
    let s = pair.request(Arm::SourceOnly);
    let o = pair.request(Arm::SourceAndOutcome);
    assert_eq!(s.passages.len(), 3);
    assert_eq!(o.passages.len(), 4);
    for (a, b) in s.passages.iter().zip(&o.passages) {
        assert_eq!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(b).unwrap()
        );
        assert!(a.text.contains("host_authored_synthetic_measurements_v1"));
        let p = a.projection.as_ref().unwrap();
        assert!(p.text.contains("source_expectation"));
    }
    assert_eq!(o.passages[3].id, "e4");
    assert!(o.passages[3].text.contains("host_action_observation"));
    assert!(o.passages[3]
        .projection
        .as_ref()
        .unwrap()
        .text
        .contains("actual_observation"));
    assert!(!o.prompt.to_lowercase().contains("failed"));
    assert!(!o.prompt.contains(r#"\"quality\":6"#));
    assert!(o.prompt.contains("supersedes"));
    let audit = serde_json::to_value(&pair).unwrap();
    assert!(!audit["raw_recall"].as_array().unwrap().is_empty());
    assert_eq!(audit["executed"], false);
    assert_eq!(std::fs::read(f.path()).unwrap(), bytes);
}

// Using historical failure, duplicate methods or hidden Quill truth breaks these literals.
#[tokio::test]
async fn current_arithmetic_uses_actual_override_without_copying_old_failure() {
    let f = Fixture::new(FixtureState::Baseline).await;
    for (budget, quality, source, outcome) in [
        (4, 7, "Nori", "Arden"),
        (10, 7, "Nori", "Nori"),
        (2, 7, "Nori", "abstain"),
        (4, 9, "Arden", "Arden"),
        (5, 7, "Nori", "Quill"),
    ] {
        let pair = f.pair(budget, quality);
        let mut n = 0;
        let report = evaluate_pair(
            &f.db,
            &pair,
            &f.access,
            "scripted-fixture",
            [Arm::SourceOnly, Arm::SourceAndOutcome],
            |_, _, guard| {
                let choice = if n == 0 { source } else { outcome };
                n += 1;
                async move {
                    guard.check()?;
                    Ok(reply(
                        choice,
                        if choice == source && n == 1 { 3 } else { 4 },
                    ))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(n, 2);
        let rows = serde_json::to_value(report).unwrap();
        assert_eq!(rows["rows"][0]["decision"]["proposed_choice"], source);
        assert_eq!(rows["rows"][1]["decision"]["proposed_choice"], outcome);
        for row in rows["rows"].as_array().unwrap() {
            assert!(row["error"].is_null(), "{row}");
            let d = &row["decision"];
            assert_eq!(d["assessment"]["candidates"].as_array().unwrap().len(), 3);
            assert_eq!(d["assessment"]["admissible"], true);
            assert_eq!(d["assessment"]["findings"], json!([]));
            assert_eq!(d["executed"], false);
        }
    }
}

#[tokio::test]
async fn unavailable_actuals_do_not_override_source_expectations() {
    for state in [FixtureState::Refused, FixtureState::Unknown] {
        let f = Fixture::new(state).await;
        let pair = f.pair(4, 7);
        let report = evaluate_pair(
            &f.db,
            &pair,
            &f.access,
            "scripted-fixture",
            [Arm::SourceOnly, Arm::SourceAndOutcome],
            |prompt, _, guard| async move {
                guard.check()?;
                assert!(!prompt.contains("\"energy\":9"));
                Ok(reply(
                    "Nori",
                    if prompt.contains("actual_observation") {
                        4
                    } else {
                        3
                    },
                ))
            },
        )
        .await
        .unwrap();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["rows"][1]["decision"]["disposition"], "recommend");
    }
}

#[tokio::test]
async fn actual_must_be_authentic_recalled_exact_revision_for_same_workload() {
    let mut f = Fixture::new(FixtureState::Baseline).await;
    let _valid = f.pair(4, 7);
    let mut q = f.query();
    q.text = "unmatched_unique_query".into();
    assert!(prepare_pair(&f.db, &f.action, &contract(4, 7), &q, &f.access).is_err());
    let actual =
        f.db.durable_action_observation(&f.action, &f.access)
            .unwrap()
            .unwrap();
    let mut fake = f.source.clone();
    fake.source.id = "fake".into();
    fake.text = actual.to_string();
    f.db.capture(fake).unwrap();
    let mut q = f.query();
    q.account = Some("test".into());
    assert!(prepare_pair(&f.db, &f.action, &contract(4, 7), &q, &f.access).is_err());
    let mut c = contract(4, 7);
    c.workload = "foreign-workload".into();
    assert!(prepare_pair(&f.db, &f.action, &c, &f.query(), &f.access).is_err());
    let other = Fixture::new(FixtureState::Baseline).await;
    assert!(prepare_pair(
        &other.db,
        &f.action,
        &contract(4, 7),
        &other.query(),
        &other.access
    )
    .is_err());
    let mut foreign = f.access.clone();
    foreign.scopes.insert("extra".into());
    assert!(prepare_pair(&f.db, &f.action, &contract(4, 7), &f.query(), &foreign).is_err());
}

#[tokio::test]
async fn failures_remain_rows_without_retry_and_missing_usage_stays_null() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(4, 7);
    let before = std::fs::read(f.path()).unwrap();
    let mut n = 0;
    let report = evaluate_pair(
        &f.db,
        &pair,
        &f.access,
        "scripted-fixture",
        [Arm::SourceAndOutcome, Arm::SourceOnly],
        |_, _, guard| {
            n += 1;
            let i = n;
            async move {
                guard.check()?;
                let mut r = reply("Nori", 3);
                if i == 1 {
                    r.error = Some("truncated".into());
                } else {
                    r.text = "invalid JSON".into();
                }
                Ok(r)
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(n, 2);
    let value = serde_json::to_value(report).unwrap();
    for row in value["rows"].as_array().unwrap() {
        assert!(row["error"].is_string());
        assert!(row["decision"].is_null());
        assert!(row["call"]["reply"]["input_tokens"].is_null());
    }
    assert_eq!(std::fs::read(f.path()).unwrap(), before);
}

#[tokio::test]
async fn source_changes_and_corrupted_cancelled_phase_before_inference_stop_callbacks() {
    for kind in ["revision", "scope", "forget", "corrupt_cancelled_phase"] {
        let mut f = Fixture::new(FixtureState::Baseline).await;
        let pair = f.pair(4, 7);
        match kind {
            "revision" => {
                let mut c = f.source.clone();
                c.text = "revised source".into();
                f.db.capture(c).unwrap();
            }
            "scope" => {
                let mut c = f.source.clone();
                c.scopes.insert("secret".into());
                f.db.capture(c).unwrap();
            }
            "forget" => {
                f.db.forget(&f.source.source).unwrap();
            }
            _ => {
                rusqlite::Connection::open(f.path())
                    .unwrap()
                    .execute(
                        "UPDATE durable_action SET phase='cancelled' WHERE id=?1",
                        [f.action.id()],
                    )
                    .unwrap();
            }
        }
        let mut n = 0;
        assert!(evaluate_pair(
            &f.db,
            &pair,
            &f.access,
            "scripted-fixture",
            [Arm::SourceOnly, Arm::SourceAndOutcome],
            |_, _, _| {
                n += 1;
                async { Ok(reply("Nori", 3)) }
            }
        )
        .await
        .is_err());
        assert_eq!(n, 0, "{kind}");
        assert!(pair.guard(&f.db, &f.access).is_err());
    }
}

#[tokio::test]
async fn invalidation_during_await_discards_reply_and_stops_next_arm() {
    for kind in ["revision", "scope", "forget", "corrupt_cancelled_phase"] {
        let f = Fixture::new(FixtureState::Baseline).await;
        let pair = f.pair(4, 7);
        let path = f.path();
        let source = f.source.clone();
        let access = f.access.clone();
        let id = f.action.id().to_owned();
        let mut n = 0;
        let result = evaluate_pair(
            &f.db,
            &pair,
            &f.access,
            "scripted-fixture",
            [Arm::SourceOnly, Arm::SourceAndOutcome],
            |_, _, guard| {
                n += 1;
                let path = path.clone();
                let source = source.clone();
                let _access = access.clone();
                let id = id.clone();
                async move {
                    tokio::task::yield_now().await;
                    let mut db = Memory::open(&path)?;
                    match kind {
                        "revision" => {
                            let mut c = source;
                            c.text = "new revision".into();
                            db.capture(c)?;
                        }
                        "scope" => {
                            let mut c = source;
                            c.scopes.insert("secret".into());
                            db.capture(c)?;
                        }
                        "forget" => {
                            db.forget(&source.source)?;
                        }
                        _ => {
                            rusqlite::Connection::open(&path)?.execute(
                                "UPDATE durable_action SET phase='cancelled' WHERE id=?1",
                                [&id],
                            )?;
                        }
                    }
                    assert!(guard.check().is_err());
                    Ok(reply("Nori", 3))
                }
            },
        )
        .await;
        assert!(result.is_err(), "{kind}");
        assert_eq!(n, 1, "{kind}");
    }
}

// Finalized actions are terminal through public cancellation; already cancelled
// and unfinished actions must nevertheless never prepare a pair. The above phase
// injection additionally checks defensive handling of persisted cancellation.
#[tokio::test]
async fn unfinished_and_cancelled_actions_cannot_prepare() {
    let complete = Fixture::new(FixtureState::Baseline).await;
    let _positive = complete.pair(4, 7);
    let mut f = Fixture::with_completion(FixtureState::Baseline, false).await;
    assert!(prepare_pair(&f.db, &f.action, &contract(4, 7), &f.query(), &f.access).is_err());
    f.db.durable_action_cancel(&f.action, &f.access).unwrap();
    assert!(prepare_pair(&f.db, &f.action, &contract(4, 7), &f.query(), &f.access).is_err());
}

#[tokio::test]
async fn every_visible_comparison_source_is_required_and_old_expectation_cannot_win() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(4, 7);
    let report = evaluate_pair(
        &f.db,
        &pair,
        &f.access,
        "scripted-fixture",
        [Arm::SourceOnly, Arm::SourceAndOutcome],
        |_, _, guard| async move {
            guard.check()?;
            Ok(ModelReply {
                text: json!({"choice":"Nori","citations":["e1"],"reason":"Incomplete support"})
                    .to_string(),
                model: "scripted-fixture".into(),
                input_tokens: None,
                output_tokens: None,
                error: None,
            })
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["rows"][0]["decision"]["disposition"], "withheld");
    assert_eq!(value["rows"][1]["decision"]["disposition"], "withheld");
    assert_eq!(
        value["rows"][1]["decision"]["assessment"]["findings"],
        json!(["infeasible_choice", "missing_citations"])
    );
}

#[tokio::test]
async fn malformed_duplicate_foreign_wrong_model_and_refusal_are_failed_rows() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(4, 7);
    for kind in [
        "transport",
        "duplicate",
        "foreign",
        "wrong_model",
        "refusal",
    ] {
        let mut n = 0;
        let report=evaluate_pair(&f.db,&pair,&f.access,"scripted-fixture",[Arm::SourceOnly,Arm::SourceAndOutcome],|_,_,guard|{
            n+=1;async move {guard.check()?;if kind=="transport"{anyhow::bail!("scripted transport failure");}let mut r=reply("Nori",3);match kind{
                "duplicate"=>r.text=r#"{"choice":"Nori","choice":"Arden","citations":[],"reason":"duplicate"}"#.into(),
                "foreign"=>r.text=r#"{"choice":"Nori","citations":["foreign"],"reason":"foreign"}"#.into(),
                "wrong_model"=>r.model="foreign-model".into(),
                "refusal"=>r.error=Some("provider refusal".into()),_=>unreachable!()};Ok(r)}
        }).await.unwrap();
        assert_eq!(n, 2, "{kind}");
        let value = serde_json::to_value(report).unwrap();
        for row in value["rows"].as_array().unwrap() {
            assert!(row["decision"].is_null(), "{kind}");
            assert!(row["error"].is_string(), "{kind}");
        }
    }
}

#[tokio::test]
async fn abstaining_from_complete_comparison_still_requires_visible_sources() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(2, 7);
    let report = evaluate_pair(
        &f.db,
        &pair,
        &f.access,
        "scripted-fixture",
        [Arm::SourceOnly, Arm::SourceAndOutcome],
        |_, _, guard| async move {
            guard.check()?;
            Ok(reply("abstain", 0))
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["rows"][1]["decision"]["disposition"], "withheld");
    assert_eq!(
        value["rows"][1]["decision"]["assessment"]["findings"],
        json!(["missing_citations"])
    );
}

// An audit-only recall hit is still newly disclosed by the returned report.
// Omitting it from the current-source closure must fail this regression.
#[tokio::test]
async fn unrelated_raw_recall_invalidation_during_call_aborts_without_next_arm() {
    for mutation in ["restrict", "forget", "revise"] {
        let mut f = Fixture::new(FixtureState::Baseline).await;
        let mut unrelated = f.source.clone();
        unrelated.source.id = "unrelated-recall".into();
        unrelated.text = "Nori UNRELATED_RAW_RECALL_CANARY".into();
        let reference = f.db.capture(unrelated.clone()).unwrap();
        let pair = f.pair(4, 7);
        assert!(pair
            .raw_recall
            .iter()
            .any(|hit| hit.anchor.reference == reference));
        pair.recheck(&f.db, &f.access).unwrap();
        let path = f.path();
        let mut calls = 0;
        let result = evaluate_pair(
            &f.db,
            &pair,
            &f.access,
            "scripted-fixture",
            [Arm::SourceOnly, Arm::SourceAndOutcome],
            |_, _, _| {
                calls += 1;
                let path = path.clone();
                let mut source = unrelated.clone();
                async move {
                    tokio::task::yield_now().await;
                    let mut db = Memory::open(&path)?;
                    match mutation {
                        "restrict" => {
                            source.scopes.insert("restricted".into());
                            db.capture(source)?;
                        }
                        "forget" => {
                            db.forget(&source.source)?;
                        }
                        "revise" => {
                            source.text = "Nori corrected unrelated source".into();
                            db.capture(source)?;
                        }
                        _ => unreachable!(),
                    }
                    Ok(reply("Nori", 3))
                }
            },
        )
        .await;
        assert!(result.is_err(), "{mutation}: stale audit was returned");
        assert_eq!(calls, 1, "{mutation}: second arm must not dispatch");
        let error = result.err().unwrap().to_string();
        assert!(error.contains("outcome sources invalidated"));
        assert!(!error.contains("UNRELATED_RAW_RECALL_CANARY"));
    }
}

// Replacement on the final arm cannot be caught by a subsequent pre-call guard.
#[tokio::test]
async fn final_arm_canonical_store_replacement_aborts_without_report() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(4, 7);
    pair.recheck(&f.db, &f.access).unwrap();
    let path = f.path();
    let replacement = f.root.join("replacement.memory-prototype.sqlite");
    let foreign = Memory::create_durable_action(&replacement, &f.access.domain).unwrap();
    assert_ne!(foreign.snapshot().unwrap().instance, pair.snapshot.instance);
    drop(foreign);
    let original = f.root.join("original.memory-prototype.sqlite");
    let mut calls = 0;
    let result = evaluate_pair(
        &f.db,
        &pair,
        &f.access,
        "scripted-fixture",
        [Arm::SourceOnly, Arm::SourceAndOutcome],
        |_, _, _| {
            calls += 1;
            let count = calls;
            let path = path.clone();
            let replacement = replacement.clone();
            let original = original.clone();
            async move {
                tokio::task::yield_now().await;
                if count == 2 {
                    std::fs::rename(&path, original)?;
                    std::fs::rename(replacement, path)?;
                }
                Ok(reply(
                    if count == 1 { "Nori" } else { "Arden" },
                    if count == 1 { 3 } else { 4 },
                ))
            }
        },
    )
    .await;
    assert!(
        result.is_err(),
        "stale original connection returned a report"
    );
    assert_eq!(calls, 2);
    assert!(result
        .err()
        .unwrap()
        .to_string()
        .contains("outcome sources invalidated"));
}

// A v2 Unicode explanation must not inherit v1's 1000-byte limit.
#[tokio::test]
async fn diagnostic_accepts_unicode_reason_and_rejects_false_checks() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = prepare_pair_with_options(
        &f.db,
        &f.action,
        &contract(3, 9),
        &f.query(),
        &f.access,
        PairOptions {
            response_format: ResponseFormat::FeasibilityFirst,
        },
    )
    .unwrap();
    let mut value = json!({"choice":"Arden","citations":["e1","e2","e3"],"reason":"界".repeat(4000),
        "candidate_checks":{
            "Nori":{"minutes":4,"energy":2,"quality":8,"source":"e1","energy_ok":true,"quality_ok":false,"feasible":false},
            "Arden":{"minutes":8,"energy":3,"quality":9,"source":"e2","energy_ok":true,"quality_ok":true,"feasible":true},
            "Quill":{"minutes":6,"energy":5,"quality":7,"source":"e3","energy_ok":false,"quality_ok":false,"feasible":false}}});
    let checked = pair.finish(Arm::SourceOnly, &value.to_string()).unwrap();
    assert_eq!(checked.disposition, "recommend");
    let escaped = value
        .to_string()
        .replace(&"界".repeat(4000), &"\\ud83d\\ude00".repeat(4000));
    assert_eq!(
        pair.finish(Arm::SourceOnly, &escaped).unwrap().disposition,
        "recommend"
    );
    value["choice"] = json!("Nori");
    let infeasible = pair.finish(Arm::SourceOnly, &value.to_string()).unwrap();
    assert_eq!(infeasible.checks_consistent, Some(true));
    assert_eq!(infeasible.disposition, "withheld");
    value["choice"] = json!("Arden");
    value["candidate_checks"]["Nori"]["feasible"] = json!(true);
    let checked = pair.finish(Arm::SourceOnly, &value.to_string()).unwrap();
    assert_eq!(checked.proposed_choice, "Arden");
    assert_eq!(checked.disposition, "withheld");
    let report = evaluate_pair(
        &f.db,
        &pair,
        &f.access,
        "scripted-fixture",
        [Arm::SourceOnly, Arm::SourceAndOutcome],
        |_, _, guard| {
            let text = value.to_string();
            async move {
                guard.check()?;
                let mut response = reply("Arden", 3);
                response.text = text;
                Ok(response)
            }
        },
    )
    .await
    .unwrap();
    let audit = serde_json::to_value(report).unwrap();
    assert_eq!(audit["protocol"], "outcome-reappraisal-v2");
    assert_eq!(audit["response_format"], "feasibility_first");
    assert_eq!(audit["reason_max_chars"], 4000);
    assert_eq!(audit["rows"][0]["decision"]["proposed_choice"], "Arden");
    assert_eq!(audit["rows"][0]["decision"]["checks_consistent"], false);
    assert_eq!(audit["rows"][0]["decision"]["disposition"], "withheld");
    let legacy = serde_json::to_value(f.pair(3, 9)).unwrap();
    for i in 0..2 {
        assert_eq!(
            audit["preparation"]["arms"][i]["request"]["passages"],
            legacy["arms"][i]["request"]["passages"]
        );
    }
}

#[tokio::test]
async fn legacy_request_stays_exact() {
    let f = Fixture::new(FixtureState::Baseline).await;
    let pair = f.pair(4, 7);
    for (arm, expected_prompt_hash, ids) in [
        (
            Arm::SourceOnly,
            "4c14b581bc98a0e6240da5e07502ab24b3dc6df869c617459833d4a225af1417",
            vec!["e1", "e2", "e3"],
        ),
        (
            Arm::SourceAndOutcome,
            "14b2ab2143a2585276d0ab3592d0acd8ad801c3e1b1ef0ec542c6f530c865bb9",
            vec!["e1", "e2", "e3", "e4"],
        ),
    ] {
        let request = pair.request(arm);
        assert_eq!(sha(&request.prompt), expected_prompt_hash);
        assert_eq!(
            request.response_schema,
            json!({"type":"object","additionalProperties":false,"required":["choice","citations","reason"],"properties":{"choice":{"type":"string","enum":["Nori","Arden","Quill","abstain"]},"citations":{"type":"array","maxItems":4,"uniqueItems":true,"items":{"type":"string","enum":ids}},"reason":{"type":"string","maxLength":1000}}})
        );
    }
    let audit = serde_json::to_value(&pair).unwrap();
    assert_eq!(audit["protocol"], "outcome-reappraisal-v1");
    assert!(audit.get("response_format").is_none());
}

#[tokio::test]
async fn diagnostic_checks_schema_bounds_and_abstention_use_current_contract() {
    let f = Fixture::new(FixtureState::Baseline).await;
    for (budget, quality, proposed, disposition, finding) in [
        (2, 8, "Nori", "recommend", None),
        (1, 8, "abstain", "abstain", None),
        (5, 7, "abstain", "abstain", Some("unnecessary_abstention")),
        (5, 7, "Arden", "recommend", Some("suboptimal_choice")),
    ] {
        let pair = prepare_pair_with_options(
            &f.db,
            &f.action,
            &contract(budget, quality),
            &f.query(),
            &f.access,
            PairOptions {
                response_format: ResponseFormat::ChoiceFirst,
            },
        )
        .unwrap();
        let checked = pair
            .finish(Arm::SourceOnly, &reply(proposed, 3).text)
            .unwrap();
        assert_eq!(checked.disposition, disposition);
        if let Some(finding) = finding {
            assert!(
                serde_json::to_value(&checked.assessment).unwrap()["findings"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(finding))
            );
        }
    }
    let pair = prepare_pair_with_options(
        &f.db,
        &f.action,
        &contract(5, 7),
        &f.query(),
        &f.access,
        PairOptions {
            response_format: ResponseFormat::FeasibilityFirst,
        },
    )
    .unwrap();
    let schema = &pair.request(Arm::SourceAndOutcome).response_schema;
    for method in experience::METHODS {
        assert_eq!(
            schema["properties"]["candidate_checks"]["properties"][method]["properties"]["source"]
                ["enum"],
            json!(["e1", "e2", "e3", "e4"])
        );
    }
    assert!(
        serde_json::to_string(schema)
            .unwrap()
            .find("candidate_checks")
            .unwrap()
            < serde_json::to_string(schema)
                .unwrap()
                .find("choice")
                .unwrap()
    );
    let mut value = json!({"choice":"Nori","citations":["e1","e2","e3"],"reason":"valid","candidate_checks":{
        "Nori":{"minutes":4,"energy":2,"quality":8,"source":"e1","energy_ok":true,"quality_ok":true,"feasible":true},
        "Arden":{"minutes":8,"energy":3,"quality":9,"source":"e2","energy_ok":true,"quality_ok":true,"feasible":true},
        "Quill":{"minutes":6,"energy":5,"quality":7,"source":"e3","energy_ok":true,"quality_ok":true,"feasible":true}}});
    assert_eq!(
        pair.finish(Arm::SourceOnly, &value.to_string())
            .unwrap()
            .disposition,
        "recommend"
    );
    // Visible but inapplicable provenance is a withheld proposal, not a substituted choice.
    value["candidate_checks"]["Nori"]["source"] = json!("e2");
    let d = pair.finish(Arm::SourceOnly, &value.to_string()).unwrap();
    assert_eq!(d.checks_consistent, Some(false));
    assert_eq!(d.proposed_choice, "Nori");
    assert_eq!(d.disposition, "withheld");
    value["candidate_checks"]["Nori"]["source"] = json!("e99");
    assert!(pair.finish(Arm::SourceOnly, &value.to_string()).is_err());
    value["candidate_checks"]["Nori"]["source"] = json!("e1");
    for field in [
        "minutes",
        "energy",
        "quality",
        "energy_ok",
        "quality_ok",
        "feasible",
    ] {
        let mut wrong = value.clone();
        wrong["candidate_checks"]["Nori"][field] = if field.ends_with("ok") || field == "feasible" {
            json!(false)
        } else {
            json!(99)
        };
        assert_eq!(
            pair.finish(Arm::SourceOnly, &wrong.to_string())
                .unwrap()
                .disposition,
            "withheld",
            "{field}"
        );
    }
    for bad in [
        value
            .to_string()
            .replacen("\"minutes\":4", "\"minutes\":4,\"minutes\":4", 1),
        value.to_string().replacen(
            "\"choice\":\"Nori\"",
            "\"choice\":\"Nori\",\"choice\":\"Nori\"",
            1,
        ),
    ] {
        assert!(pair.finish(Arm::SourceOnly, &bad).is_err());
    }
    for reason in ["界".repeat(4001), " ".repeat(4000)] {
        value["reason"] = json!(reason);
        assert!(pair.finish(Arm::SourceOnly, &value.to_string()).is_err());
    }
    assert!(pair.finish(Arm::SourceOnly, &" ".repeat(65537)).is_err());
    value["reason"] = json!("valid");
    value["candidate_checks"]
        .as_object_mut()
        .unwrap()
        .remove("Quill");
    assert!(pair.finish(Arm::SourceOnly, &value.to_string()).is_err());
}

#[tokio::test]
async fn diagnostic_actual_precedence_and_source_guards_remain_independent_of_claims() {
    let mut f = Fixture::new(FixtureState::Baseline).await;
    let pair = prepare_pair_with_options(
        &f.db,
        &f.action,
        &contract(5, 7),
        &f.query(),
        &f.access,
        PairOptions {
            response_format: ResponseFormat::FeasibilityFirst,
        },
    )
    .unwrap();
    let value = json!({"choice":"Arden","citations":["e1","e2","e3","e4"],"reason":"actual rules out Nori", "candidate_checks":{
        "Nori":{"minutes":4,"energy":9,"quality":8,"source":"e4","energy_ok":false,"quality_ok":true,"feasible":false},
        "Arden":{"minutes":8,"energy":3,"quality":9,"source":"e2","energy_ok":true,"quality_ok":true,"feasible":true},
        "Quill":{"minutes":6,"energy":5,"quality":7,"source":"e3","energy_ok":true,"quality_ok":true,"feasible":true}}});
    let decision = pair
        .finish(Arm::SourceAndOutcome, &value.to_string())
        .unwrap();
    assert_eq!(decision.checks_consistent, Some(true));
    assert_eq!(decision.disposition, "recommend"); // feasible suboptimal retains prior semantics
    let guard = pair.guard(&f.db, &f.access).unwrap();
    let mut corrected = f.source.clone();
    corrected.text = "changed source".into();
    f.db.capture(corrected).unwrap();
    assert!(guard.check().is_err());
    assert!(pair.recheck(&f.db, &f.access).is_err());
}
