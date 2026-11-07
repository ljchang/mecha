use super::*;
use std::{
    future::Future,
    pin::pin,
    task::{Context as TaskContext, Poll, Waker},
};

fn ready<T>(f: impl Future<Output = T>) -> T {
    match pin!(f).poll(&mut TaskContext::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixture callback must be ready"),
    }
}
fn fixture() -> (super::super::transcript_tests::Fixture, Memory, Vec<Trace>) {
    let f = super::super::transcript_tests::Fixture::new();
    let path = f.dir.join("workflow.memory-prototype.sqlite");
    let mut db = Memory::create(&path, &access().domain).unwrap();
    let training = train(&mut db).unwrap();
    drop(db);
    let db = Memory::open(&path).unwrap();
    (f, db, training)
}
fn answer(prepared: &Prepared, choice: &str) -> String {
    json!({"choice":choice,"citations":prepared.request.passages.iter().map(|p| &p.id).collect::<Vec<_>>(),"reason":"Compare observed costs under the current goal."}).to_string()
}
#[test]
fn training_survives_restart_and_goals_change_the_best_choice() {
    let (_f, db, traces) = fixture();
    assert_eq!(traces.len(), 12);
    for trace in &traces {
        verify_trace(&db, trace).unwrap();
    }
    for workload in 0..4 {
        let choices: BTreeSet<_> = (0..3)
            .map(|context| World::new(workload, context).best())
            .collect();
        assert_eq!(choices.len(), 3);
    }
}
#[test]
fn retrieved_and_oracle_match_and_none_sees_no_metrics() {
    let (_f, db, training) = fixture();
    let at = db.snapshot().unwrap();
    for workload in 0..4 {
        let w = World::new(workload, 0);
        let retrieved = w.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
        let oracle = w.prepare(&db, &at, &training, Arm::Oracle).unwrap();
        assert_eq!(retrieved.references(), oracle.references());
        assert_eq!(retrieved.request.passages.len(), 3);
        let none = w.prepare(&db, &at, &training, Arm::None).unwrap();
        assert!(none.request.passages.is_empty());
        assert!(!none.request.prompt.contains("observed_minutes"));
        assert!(!none.request.prompt.contains("expected_choice"));
    }
}
#[test]
fn forged_outcomes_duplicate_keys_and_foreign_citations_cannot_write() {
    let (_f, mut db, training) = fixture();
    let before = db.snapshot().unwrap();
    let mut w = World::new(0, 0);
    for reply in [
        r#"{"choice":"Nori","choice":"Arden","citations":[],"reason":"x"}"#,
        r#"{"choice":"Nori","citations":[],"reason":"x","outcome":{"success":true}}"#,
        r#"{"choice":"Nori","citations":["foreign"],"reason":"x"}"#,
        r#"{"choice":"invented","citations":[],"reason":"x"}"#,
    ] {
        let p = w.prepare(&db, &before, &training, Arm::Retrieved).unwrap();
        assert!(w.finish(&mut db, p, reply).is_err());
        assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
    }
}
#[test]
fn host_computes_outcome_and_links_all_consulted_sources() {
    let (_f, mut db, training) = fixture();
    let at = db.snapshot().unwrap();
    let mut w = World::new(0, 2);
    let p = w.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
    let refs = p.references();
    let result = w
        .finish(
            &mut db,
            p,
            r#"{"choice":"Nori","citations":[],"reason":"I think this is best"}"#,
        )
        .unwrap();
    assert!(!result.optimal);
    assert_eq!(result.trace.outcome.energy, Some(9));
    assert_eq!(result.trace.outcome.feasible, Some(false));
    assert!(!result.citations_complete);
    let Some(Record::Evidence(decision)) = db.read(&result.trace.decision, &access()).unwrap()
    else {
        panic!()
    };
    let saved: Value = serde_json::from_str(&decision.text).unwrap();
    assert_eq!(saved["source_bindings"].as_array().unwrap().len(), 3);
    assert_eq!(saved["source_bindings"][0]["id"], "e1");
    assert_eq!(
        saved["source_bindings"][0]["source"],
        serde_json::to_value(&refs[0]).unwrap()
    );
    let deps = closure(&db.conn, &result.trace.observation).unwrap();
    for r in refs {
        assert!(deps.contains(&r));
    }
    verify_trace(&db, &result.trace).unwrap();
}
#[test]
fn stale_world_or_changed_source_rejects_without_partial_trace() {
    for mutation in ["state", "withdraw", "scope", "forget"] {
        let (_f, mut db, training) = fixture();
        let at = db.snapshot().unwrap();
        let mut w = World::new(0, 0);
        let p = w.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
        let reply = answer(&p, "Nori");
        let r = p.request.passages[0].source.clone();
        match mutation {
            "state" => w.revision += 1,
            "withdraw" => {
                // Evidence withdrawal is not public; exercise the persisted inactive
                // revision invariant directly (claims expose withdrawal).
                let record = db.read(&r, &access()).unwrap().unwrap();
                append(
                    &db.conn,
                    Some(&r),
                    &record,
                    &access().scopes,
                    &[],
                    "withdrawn",
                )
                .unwrap();
            }
            "scope" => {
                let Record::Evidence(mut e) = db.read(&r, &access()).unwrap().unwrap() else {
                    panic!()
                };
                e.scopes.insert("revoked".into());
                db.capture(e).unwrap();
            }
            "forget" => {
                let Record::Evidence(e) = db.read(&r, &access()).unwrap().unwrap() else {
                    panic!()
                };
                db.forget(&e.source).unwrap();
            }
            _ => unreachable!(),
        }
        let before = db.snapshot().unwrap();
        assert!(w.finish(&mut db, p, &reply).is_err(), "{mutation}");
        assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
    }
}
#[test]
fn forgetting_a_consulted_source_removes_dependent_trace() {
    let (_f, mut db, training) = fixture();
    let at = db.snapshot().unwrap();
    let mut w = World::new(0, 0);
    let p = w.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
    let r = p.request.passages[0].source.clone();
    let reply = answer(&p, "Nori");
    let result = w.finish(&mut db, p, &reply).unwrap();
    let Record::Evidence(e) = db.read(&r, &access()).unwrap().unwrap() else {
        panic!()
    };
    db.forget(&e.source).unwrap();
    assert!(db
        .read(&result.trace.observation, &access())
        .unwrap()
        .is_none());
    let p = w
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    assert_eq!(p.request.passages.len(), 2);
}
#[test]
fn complete_run_preserves_failures_and_scores_abstention_separately() {
    let f = super::super::transcript_tests::Fixture::new();
    let path = f.dir.join("run.memory-prototype.sqlite");
    let mut calls = 0;
    let mut observed = 0;
    let report = ready(run(&path, "fixture-model", |_, _| {
        calls += 1;
        std::future::ready(if calls == 1 { Err(anyhow::anyhow!("transport failed")) } else {
            Ok(ModelReply { text:r#"{"choice":"abstain","citations":[],"reason":"insufficient observations"}"#.into(), model: if calls == 2 { "wrong" } else { "fixture-model" }.into(), input_tokens:None,output_tokens:None,error:None })
        })
    }, |_| { observed += 1; Ok(()) })).unwrap();
    assert_eq!((calls, observed, report.rows.len()), (36, 36, 36));
    assert!(report.readback_verified);
    assert_eq!(report.rows.iter().filter(|r| r.error.is_some()).count(), 2);
    assert!(report
        .rows
        .iter()
        .filter_map(|r| r.result.as_ref())
        .all(|r| !r.optimal));
    assert!(report
        .rows
        .iter()
        .filter(|r| r.arm == Arm::None && r.error.is_none())
        .all(|r| r.result.as_ref().unwrap().appropriate_abstention));
    assert!(ready(run(
        &path,
        "fixture-model",
        |_, _| async { unreachable!() },
        |_| Ok(())
    ))
    .is_err());
}

#[test]
fn indirect_provenance_change_invalidates_receipt_recall_and_saved_trace() {
    let (_f, mut db, training) = fixture();
    let at = db.snapshot().unwrap();
    let mut world = World::new(0, 0);
    let pending = world.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
    let mut another = World::new(0, 0);
    let prepared = another
        .prepare(&db, &at, &training, Arm::Retrieved)
        .unwrap();
    let reply = answer(&prepared, "Nori");
    let recorded = another.finish(&mut db, prepared, &reply).unwrap();
    // Restrict an ancestor event, not the recalled outcome itself.
    let Record::Evidence(mut event) = db.read(&training[0].event, &access()).unwrap().unwrap()
    else {
        panic!()
    };
    event.scopes.insert("restricted".into());
    db.capture(event).unwrap();
    let before = db.snapshot().unwrap();
    assert!(world.finish(&mut db, pending, &reply).is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
    assert!(db
        .read(&recorded.trace.observation, &access())
        .unwrap()
        .is_none());
    for arm in [Arm::Retrieved, Arm::Oracle] {
        let next = world.prepare(&db, &at, &training, arm).unwrap();
        assert_eq!(next.request.passages.len(), 2);
        assert!(!next.evidence_complete);
    }
}

#[test]
fn receipt_cannot_cross_world_or_database_boundaries() {
    let (_f, mut db, training) = fixture();
    let (_g, mut other_db, _) = fixture();
    let at = db.snapshot().unwrap();
    let mut world = World::new(0, 0);
    let mut other_world = World::new(0, 0);
    let p = world.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
    let reply = answer(&p, "Nori");
    let before = db.snapshot().unwrap();
    assert!(other_world.finish(&mut db, p, &reply).is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
    let p = world.prepare(&db, &at, &training, Arm::Retrieved).unwrap();
    let before = other_db.snapshot().unwrap();
    assert!(world.finish(&mut other_db, p, &reply).is_err());
    assert_eq!(other_db.snapshot().unwrap().sequence, before.sequence);
}

#[test]
fn recall_views_preserve_sources_and_separate_prior_appraisal_from_measurements() {
    let (_f, db, training) = fixture();
    let before = db.snapshot().unwrap();
    let world = World::new(0, 2);
    let original = world
        .prepare(&db, &before, &training, Arm::Retrieved)
        .unwrap();
    let legacy = world
        .prepare_view(&db, &before, &training, Arm::Retrieved, RecallView::Legacy)
        .unwrap();
    assert_eq!(original.request.prompt, legacy.request.prompt);
    for view in [RecallView::Measurements, RecallView::Scoped] {
        let projected = world
            .prepare_view(&db, &before, &training, Arm::Retrieved, view)
            .unwrap();
        for (p, old) in projected
            .request
            .passages
            .iter()
            .zip(&original.request.passages)
        {
            assert_eq!(
                (p.text.as_str(), &p.source, &p.sha256),
                (old.text.as_str(), &old.source, &old.sha256)
            );
            let projection = p.projection.as_ref().unwrap();
            let shown: Value = serde_json::from_str(&projection.text).unwrap();
            assert!(shown["outcome"].get("feasible").is_none());
            assert_eq!(
                shown["outcome"]["energy"],
                serde_json::from_str::<Value>(&old.text).unwrap()["outcome"]["energy"]
            );
            if view == RecallView::Scoped {
                assert_eq!(
                    shown["historical_appraisal"]["original_state"]["energy_budget"],
                    10
                );
                assert!(projection.context.is_some());
            } else {
                assert!(shown.get("historical_appraisal").is_none());
                assert!(projection.context.is_none());
            }
        }
        assert!(
            projected
                .request
                .prompt
                .contains("\\\"energy_budget\\\":10")
                == (view == RecallView::Scoped)
        );
    }
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
}

#[test]
fn scoped_context_is_persisted_and_rechecked_as_a_source() {
    let (_f, mut db, training) = fixture();
    let at = db.snapshot().unwrap();
    let mut world = World::new(0, 2);
    let prepared = world
        .prepare_view(&db, &at, &training, Arm::Retrieved, RecallView::Scoped)
        .unwrap();
    let context = prepared.request.passages[0]
        .projection
        .as_ref()
        .unwrap()
        .context
        .clone()
        .unwrap();
    assert!(prepared.references().contains(&context.source));
    let reply = answer(&prepared, "Quill");
    let result = world.finish(&mut db, prepared, &reply).unwrap();
    let Record::Evidence(decision) = db.read(&result.trace.decision, &access()).unwrap().unwrap()
    else {
        panic!()
    };
    let saved: Value = serde_json::from_str(&decision.text).unwrap();
    assert_eq!(
        saved["source_bindings"][0]["projection"]["context"]["source"],
        serde_json::to_value(&context.source).unwrap()
    );
    let pending = world
        .prepare_view(&db, &at, &training, Arm::Retrieved, RecallView::Scoped)
        .unwrap();
    let Record::Evidence(mut event) = db.read(&context.source, &access()).unwrap().unwrap() else {
        panic!()
    };
    event.text.push(' ');
    db.capture(event).unwrap();
    let before = db.snapshot().unwrap();
    assert!(world.finish(&mut db, pending, &reply).is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
}

#[test]
fn view_matrix_has_matched_calls_and_unseen_evidence_is_incomplete_or_current() {
    for trial in [Trial::RecallViews, Trial::Unseen] {
        let f = super::super::transcript_tests::Fixture::new();
        let path = f.dir.join("views.memory-prototype.sqlite");
        let report = ready(run_trial(&path, "fixture", trial, |_, _| async {
            Ok(ModelReply {text:r#"{"choice":"abstain","citations":[],"reason":"insufficient observations"}"#.into(),model:"fixture".into(),input_tokens:None,output_tokens:None,error:None})
        }, |_| Ok(()))).unwrap();
        assert_eq!(report.rows.len(), 36);
        if trial == Trial::RecallViews {
            assert!(report.rows.iter().all(|r| r.arm == Arm::Retrieved));
            for view in [
                RecallView::Legacy,
                RecallView::Measurements,
                RecallView::Scoped,
            ] {
                assert_eq!(
                    report.rows.iter().filter(|r| r.view == Some(view)).count(),
                    12
                );
            }
        } else {
            assert!(report
                .rows
                .iter()
                .all(|r| r.view == Some(RecallView::Measurements)));
            assert_eq!(report.training.len(), 11);
            assert!(report
                .training
                .iter()
                .any(|t| t.previous_observation.is_some()));
            for row in report.rows.iter().filter(|r| r.arm != Arm::None) {
                assert_eq!(
                    row.request.passages.len(),
                    if row.case.starts_with("vireo") { 2 } else { 3 }
                );
                if row.case.starts_with("willow") {
                    let p = &row.request.passages[0];
                    assert_eq!(
                        serde_json::from_str::<Value>(&p.text).unwrap()["outcome"]
                            ["observed_minutes"],
                        7
                    );
                }
            }
        }
    }
}

#[test]
fn decision_checks_use_observations_and_current_constraints_not_hidden_outcomes() {
    let (_f, db, training) = fixture();
    let world = World::new(0, 2);
    let mut p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    let check = checks::assess(
        &world.state(),
        &p.request,
        &parse_choice(&p.request, &answer(&p, "Nori")).unwrap(),
    )
    .unwrap();
    assert!(!check.admissible); // recorded feasible=true belongs to old budget10
    assert!(check.findings.contains(&checks::Finding::InfeasibleChoice));
    let check = checks::assess(
        &world.state(),
        &p.request,
        &parse_choice(&p.request, &answer(&p, "Arden")).unwrap(),
    )
    .unwrap();
    assert!(check.admissible); // slower but safe is not the same as infeasible
    assert!(check.findings.contains(&checks::Finding::SuboptimalChoice));
    // Change only supplied measurements: a hidden truth-table reader would still reject Nori.
    let mut value: Value = serde_json::from_str(&p.request.passages[0].text).unwrap();
    value["outcome"]["energy"] = json!(1);
    p.request.passages[0].text = value.to_string();
    let check = checks::assess(
        &world.state(),
        &p.request,
        &parse_choice(&p.request, &answer(&p, "Nori")).unwrap(),
    )
    .unwrap();
    assert!(check.admissible && check.findings.is_empty());
}

#[test]
fn incomplete_or_ambiguous_observations_do_not_support_action() {
    let (_f, db, training) = fixture();
    let world = World::new(0, 0);
    let mut p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    p.request.passages.pop();
    let check = checks::assess(
        &world.state(),
        &p.request,
        &parse_choice(&p.request, &answer(&p, "Nori")).unwrap(),
    )
    .unwrap();
    assert!(!check.admissible);
    assert_eq!(check.missing, vec!["Quill"]);
    let abstain = checks::assess(
        &world.state(),
        &p.request,
        &parse_choice(&p.request, &answer(&p, "abstain")).unwrap(),
    )
    .unwrap();
    assert!(abstain.admissible && abstain.findings.is_empty());
    p.request.passages.push(p.request.passages[0].clone());
    assert!(checks::assess(
        &world.state(),
        &p.request,
        &Choice {
            choice: "abstain".into(),
            citations: vec![],
            reason: "unknown".into()
        }
    )
    .is_err());
}

#[test]
fn checked_commit_withholds_infeasible_action_and_retains_proposal() {
    let (_f, mut db, training) = fixture();
    let mut world = World::new(0, 2);
    let p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    let reply = answer(&p, "Nori");
    let result = world
        .finish_with_review(&mut db, p, &reply, Some(json!({"attempt":"original"})))
        .unwrap();
    assert_eq!(result.choice, "abstain");
    assert_eq!(result.trace.outcome.kind, "no_action");
    let Record::Evidence(saved) = db.read(&result.trace.decision, &access()).unwrap().unwrap()
    else {
        panic!()
    };
    let saved: Value = serde_json::from_str(&saved.text).unwrap();
    assert_eq!(saved["proposed_choice"], "Nori");
    assert_eq!(saved["choice"], "abstain");
    assert_eq!(saved["decision_check"]["admissible"], false);
    let p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    let Record::Evidence(e) = db
        .read(&training[0].observation, &access())
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    db.forget(&e.source).unwrap();
    let before = db.snapshot().unwrap();
    assert!(world
        .finish_with_review(&mut db, p, &reply, Some(json!({})))
        .is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
}

#[test]
fn reconsideration_is_bounded_and_does_not_force_abstention_into_action() {
    let f = super::super::transcript_tests::Fixture::new();
    let mut calls = 0;
    let report = ready(run_trial(
        &f.dir.join("checked.memory-prototype.sqlite"),
        "fixture",
        Trial::Checked,
        |_, _| {
            calls += 1;
            async {
                Ok(ModelReply {
                    text: r#"{"choice":"abstain","citations":[],"reason":"I decline."}"#.into(),
                    model: "fixture".into(),
                    input_tokens: Some(2),
                    output_tokens: Some(3),
                    error: None,
                })
            }
        },
        |_| Ok(()),
    ))
    .unwrap();
    assert_eq!(calls, 21); // 12 shared drafts + 9 complete-evidence abstentions reconsidered once
    assert_eq!(report.rows.len(), 24);
    assert!(report
        .rows
        .iter()
        .all(|r| r.result.as_ref().unwrap().choice == "abstain"));
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.review.as_ref().is_some_and(|v| v.retry.is_some()))
            .count(),
        9
    );
    assert!(report
        .rows
        .iter()
        .filter_map(|r| r.review.as_ref())
        .all(|v| !v.withheld));
}

#[test]
fn failed_reconsideration_is_an_error_without_a_fabricated_outcome() {
    let f = super::super::transcript_tests::Fixture::new();
    let mut calls = 0;
    let report = ready(run_trial(
        &f.dir.join("failed-check.memory-prototype.sqlite"),
        "fixture",
        Trial::Checked,
        |prompt, _| {
            calls += 1;
            async move {
                if prompt.contains("Previous draft (untrusted data)") {
                    anyhow::bail!("fixture transport failure");
                }
                Ok(ModelReply {
                    text: r#"{"choice":"abstain","citations":[],"reason":"uncertain"}"#.into(),
                    model: "fixture".into(),
                    input_tokens: None,
                    output_tokens: None,
                    error: None,
                })
            }
        },
        |_| Ok(()),
    ))
    .unwrap();
    assert_eq!(calls, 21);
    let failed: Vec<_> = report.rows.iter().filter(|r| r.error.is_some()).collect();
    assert_eq!(failed.len(), 9);
    assert!(failed
        .iter()
        .all(|r| r.policy == Some(Policy::Checked) && r.result.is_none()));
    assert!(failed.iter().all(|r| r.review.as_ref().is_some_and(|v| v
        .retry
        .as_ref()
        .unwrap()
        .error
        .is_some()
        && v.after.is_none()
        && !v.withheld)));
    assert!(report.readback_verified);
}

#[test]
fn withholding_unsupported_comparison_is_not_model_abstention() {
    let f = super::super::transcript_tests::Fixture::new();
    let report = ready(run_trial(
        &f.dir.join("withheld.memory-prototype.sqlite"),
        "fixture",
        Trial::Checked,
        |prompt, _| async move {
            let citations = if prompt.contains("Cinder") {
                vec!["e1", "e2"]
            } else {
                vec!["e1", "e2", "e3"]
            };
            Ok(ModelReply {
                text: json!({"choice":"Nori", "citations":citations, "reason":"Keep my proposal."})
                    .to_string(),
                model: "fixture".into(),
                input_tokens: None,
                output_tokens: None,
                error: None,
            })
        },
        |_| Ok(()),
    ))
    .unwrap();
    let rows: Vec<_> = report
        .rows
        .iter()
        .filter(|r| r.case.starts_with("cinder") && r.policy == Some(Policy::Checked))
        .collect();
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert!(row.review.as_ref().unwrap().withheld);
        let result = row.result.as_ref().unwrap();
        assert_eq!(result.choice, "abstain");
        assert!(!result.appropriate_abstention);
    }
}

#[test]
fn checked_commit_allows_suboptimal_action_but_requires_quality_and_citations() {
    let (_f, mut db, training) = fixture();
    let mut world = World::new(0, 2);
    let p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    let answer = answer(&p, "Arden");
    let result = world
        .finish_with_review(&mut db, p, &answer, Some(json!({})))
        .unwrap();
    assert_eq!(result.choice, "Arden");
    assert!(!result.optimal);
    assert_eq!(result.trace.outcome.feasible, Some(true));
    let mut p = world
        .prepare(&db, &db.snapshot().unwrap(), &training, Arm::Retrieved)
        .unwrap();
    let mut choice = parse_choice(&p.request, &answer).unwrap();
    choice.citations.clear();
    let check = checks::assess(&world.state(), &p.request, &choice).unwrap();
    assert!(!check.admissible && check.findings.contains(&checks::Finding::MissingCitations));
    choice.citations = p.request.passages.iter().map(|p| p.id.clone()).collect();
    let mut value: Value = serde_json::from_str(&p.request.passages[1].text).unwrap();
    value["outcome"]["quality"] = json!(0);
    p.request.passages[1].text = value.to_string();
    let check = checks::assess(&world.state(), &p.request, &choice).unwrap();
    assert!(!check.admissible && check.findings.contains(&checks::Finding::InfeasibleChoice));
}

#[test]
fn observation_request_is_bounded_and_only_fills_a_missing_method() {
    let (_f, mut db, training) = fixture();
    let mut world = World::new(0, 0);
    let at = db.snapshot().unwrap();
    let seed = &training[..2];
    let p = world
        .prepare_view(&db, &at, seed, Arm::Oracle, RecallView::Measurements)
        .unwrap();
    let before = db.snapshot().unwrap();
    let wrong = answer(&p, "Nori");
    assert!(observations::acquire(&mut db, &mut world, p, &wrong, true, true).is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
    let p = world
        .prepare_view(&db, &at, seed, Arm::Oracle, RecallView::Measurements)
        .unwrap();
    let valid = answer(&p, "Quill");
    let receipt = observations::acquire(&mut db, &mut world, p, &valid, true, true).unwrap();
    assert_eq!(receipt.status, observations::Status::Observed);
    assert_eq!(receipt.units, 1);
    let mut updated = seed.to_vec();
    updated.push(receipt.trace.clone());
    let fresh = world
        .prepare_view(
            &db,
            &db.snapshot().unwrap(),
            &updated,
            Arm::Oracle,
            RecallView::Measurements,
        )
        .unwrap();
    assert!(fresh.evidence_complete);
    let again = answer(&fresh, "Quill");
    assert!(observations::acquire(&mut db, &mut world, fresh, &again, true, true).is_err());
}

#[test]
fn observation_unavailable_consumes_allowance_decline_costs_nothing() {
    for (method, available, status, units) in [
        ("Quill", false, observations::Status::Unavailable, 1),
        ("abstain", true, observations::Status::Declined, 0),
    ] {
        let (_f, mut db, training) = fixture();
        let mut world = World::new(0, 0);
        let p = world
            .prepare_view(
                &db,
                &db.snapshot().unwrap(),
                &training[..2],
                Arm::Oracle,
                RecallView::Measurements,
            )
            .unwrap();
        let text = answer(&p, method);
        let a = observations::acquire(&mut db, &mut world, p, &text, true, available).unwrap();
        assert_eq!(a.status, status);
        assert_eq!(a.units, units);
        assert!(a.trace.outcome.observed_minutes.is_none());
        let p = world
            .prepare_view(
                &db,
                &db.snapshot().unwrap(),
                &training[..2],
                Arm::Oracle,
                RecallView::Measurements,
            )
            .unwrap();
        let text = answer(&p, "Quill");
        let before = db.snapshot().unwrap();
        assert!(observations::acquire(&mut db, &mut world, p, &text, true, true).is_err());
        assert_eq!(before.sequence, db.snapshot().unwrap().sequence);
    }
}

#[test]
fn observation_rejects_foreign_stale_and_malformed_requests_atomically() {
    for defect in 0..7 {
        let (_f, mut db, training) = fixture();
        let mut world = World::new(0, 0);
        let mut p = world
            .prepare_view(
                &db,
                &db.snapshot().unwrap(),
                &training[..2],
                Arm::Oracle,
                RecallView::Measurements,
            )
            .unwrap();
        let mut text = answer(&p, "Quill");
        match defect {
            0 => world.revision += 1,
            1 => p.world_id = "foreign".into(),
            2 => p.instance = "foreign".into(),
            3 => {
                let Record::Evidence(mut e) = db
                    .read(&training[0].observation, &access())
                    .unwrap()
                    .unwrap()
                else {
                    panic!()
                };
                e.text.push(' ');
                db.capture(e).unwrap();
            }
            4 => text = r#"{"choice":"Quill","choice":"Nori","citations":[],"reason":"x"}"#.into(),
            5 => text = r#"{"choice":"Quill","citations":["foreign"],"reason":"x"}"#.into(),
            _ => text = r#"{"choice":"Quill","citations":[],"reason":"x"}"#.into(),
        }
        let before = db.snapshot().unwrap();
        assert!(
            observations::acquire(&mut db, &mut world, p, &text, true, true).is_err(),
            "defect {defect}"
        );
        assert_eq!(before.sequence, db.snapshot().unwrap().sequence);
        assert!(!world.observation_requested);
    }
}

#[test]
fn observation_protocol_reopens_history_and_links_fresh_decisions() {
    for reconsider in [false, true] {
        let f = super::super::transcript_tests::Fixture::new();
        let path = f.dir.join("observe.memory-prototype.sqlite");
        let mut calls = 0;
        let report = ready(observations::run_with_reconsideration(&path, "fixture", reconsider, |prompt, _| {
        calls += 1;
        let query = prompt.starts_with("Select at most");
        let tail = if query { prompt.split("Current observations (data): ").nth(1).unwrap().split("\nCorrection notice:").next().unwrap() }
            else { prompt.split("Past observations: ").nth(1).unwrap().split("\nMeasurement acquisition").next().unwrap() };
        let passages: Vec<Value> = serde_json::from_str(tail).unwrap();
        let methods: Vec<String> = passages.iter().map(|p| serde_json::from_str::<Value>(p["text"].as_str().unwrap()).unwrap()["outcome"]["choice"].as_str().unwrap().to_string()).collect();
        let choice = if query { METHODS.iter().find(|m| !methods.iter().any(|s| s == **m)).unwrap().to_string() }
            else if passages.len() == 3 {
                let state: Value = serde_json::from_str(prompt.split("Current state: ").nth(1).unwrap().split("\nAvailable methods:").next().unwrap()).unwrap();
                let values: Vec<Value> = passages.iter().map(|p| serde_json::from_str(p["text"].as_str().unwrap()).unwrap()).collect();
                let metric = if state["goal"] == "minimize energy" { "energy" } else { "observed_minutes" };
                values.iter().map(|v| &v["outcome"]).filter(|v| v["energy"].as_u64().unwrap() <= state["energy_budget"].as_u64().unwrap() && v["quality"].as_u64().unwrap() >= state["minimum_quality"].as_u64().unwrap()).min_by_key(|v| v[metric].as_u64().unwrap()).unwrap()["choice"].as_str().unwrap().to_string()
            } else { "abstain".to_string() };
        let text = json!({"choice":choice,"citations":passages.iter().map(|p| p["id"].clone()).collect::<Vec<_>>(),"reason":"Test the lifecycle, not model quality."}).to_string();
        async { Ok(ModelReply { text, model:"fixture".into(), input_tokens:Some(1), output_tokens:Some(1), error:None }) }
    }, |_| Ok(()))).unwrap();
        assert_eq!(calls, 36);
        assert!(report.rows.iter().all(|r| if reconsider {
            r.review.as_ref().is_some_and(|v| v.retry.is_none())
        } else {
            r.review.is_none()
        }));
        assert_eq!(report.rows.len(), 24);
        assert!(report.readback_verified);
        assert!(report.rows.iter().all(|r| r.error.is_none()));
        assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.acquisition.as_ref().unwrap().status == observations::Status::Observed)
            .count(),
        6
    );
        assert_eq!(
            report
                .rows
                .iter()
                .filter(|r| r.result.as_ref().unwrap().evidence_complete)
                .count(),
            6
        );
        assert_eq!(
            report
                .rows
                .iter()
                .filter(|r| r.result.as_ref().unwrap().optimal)
                .count(),
            6
        );
        for row in &report.rows {
            let seed = report.seeds.iter().find(|s| s.case == row.case).unwrap();
            assert!(row
                .request
                .passages
                .iter()
                .all(|p| seed.observations.iter().any(|t| t.observation == p.source)));
            assert!(row
                .decision_request
                .as_ref()
                .unwrap()
                .passages
                .iter()
                .all(
                    |p| seed.observations.iter().any(|t| t.observation == p.source)
                        || p.source == row.acquisition.as_ref().unwrap().trace.observation
                ));
        }
        let mut db = Memory::open(&path).unwrap();
        let row = report
            .rows
            .iter()
            .find(|r| r.case == "grove-1" && r.policy == "observe")
            .unwrap();
        let receipt = row.acquisition.as_ref().unwrap();
        assert_eq!(receipt.trace.outcome.observed_minutes, Some(12));
        let notice = report
            .seeds
            .iter()
            .find(|s| s.case == "grove-1")
            .unwrap()
            .notice
            .as_ref()
            .unwrap();
        assert!(closure(&db.conn, &receipt.trace.observation)
            .unwrap()
            .contains(notice));
        let Record::Evidence(mut e) = db.read(notice, &access()).unwrap().unwrap() else {
            panic!()
        };
        e.scopes.insert("restricted".into());
        db.capture(e).unwrap();
        assert!(!fresh(
            &db.conn,
            &row.result.as_ref().unwrap().trace.observation,
            i64::MAX
        )
        .unwrap());
    }
}

#[test]
fn failed_observation_query_remains_error_without_final_call_or_trace() {
    let f = super::super::transcript_tests::Fixture::new();
    let mut calls = 0;
    let r = ready(observations::run(
        &f.dir.join("failed.memory-prototype.sqlite"),
        "fixture",
        |_, _| {
            calls += 1;
            async { anyhow::bail!("transport failure") }
        },
        |_| Ok(()),
    ))
    .unwrap();
    assert_eq!(calls, 12);
    assert_eq!(r.rows.len(), 24);
    assert!(r.rows.iter().all(|r| r.error.is_some()
        && r.result.is_none()
        && r.call.is_none()
        && r.acquisition.is_none()));
}

#[test]
fn acquisition_reconsideration_is_once_and_does_not_bypass_stale_sources() {
    let (_f, mut db, training) = fixture();
    let mut world = World::new(0, 0);
    let p = world
        .prepare_view(
            &db,
            &db.snapshot().unwrap(),
            &training[..2],
            Arm::Oracle,
            RecallView::Measurements,
        )
        .unwrap();
    let initial = answer(&p, "Nori");
    let call = Call {
        prompt: p.request.prompt.clone(),
        response_schema: Some(p.request.response_schema.clone()),
        reply: Some(ModelReply {
            text: initial,
            model: "fixture".into(),
            input_tokens: Some(1),
            output_tokens: Some(1),
            error: None,
        }),
        error: None,
        millis: 0,
    };
    let mut review = None;
    let mut calls = 0;
    let corrected = answer(&p, "abstain");
    let text = ready(checks::reconsider(
        &world.state(),
        &p.request,
        &call,
        "fixture",
        &mut |prompt, schema| {
            calls += 1;
            assert!(prompt.starts_with(&p.request.prompt));
            assert_eq!(schema, Some(p.request.response_schema.clone()));
            std::future::ready(Ok(ModelReply {
                text: corrected.clone(),
                model: "fixture".into(),
                input_tokens: Some(1),
                output_tokens: Some(1),
                error: None,
            }))
        },
        &mut review,
    ))
    .unwrap();
    assert_eq!(calls, 1);
    let review = review.unwrap();
    assert!(!review.before.admissible);
    assert!(review.after.as_ref().unwrap().admissible);
    assert!(!review.withheld);
    let Record::Evidence(mut e) = db
        .read(&training[0].observation, &access())
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    e.text.push(' ');
    db.capture(e).unwrap();
    let before = db.snapshot().unwrap();
    assert!(world
        .finish_with_review(&mut db, p, &text, Some(json!(review)))
        .is_err());
    assert_eq!(db.snapshot().unwrap().sequence, before.sequence);
}

#[test]
fn reconsideration_retains_failed_retry_and_never_loops() {
    for failure in [false, true] {
        let (_f, db, training) = fixture();
        let world = World::new(0, 0);
        let p = world
            .prepare_view(
                &db,
                &db.snapshot().unwrap(),
                &training[..2],
                Arm::Oracle,
                RecallView::Measurements,
            )
            .unwrap();
        let initial = answer(&p, "Nori");
        let call = Call {
            prompt: p.request.prompt.clone(),
            response_schema: Some(p.request.response_schema.clone()),
            reply: Some(ModelReply {
                text: initial.clone(),
                model: "fixture".into(),
                input_tokens: None,
                output_tokens: None,
                error: None,
            }),
            error: None,
            millis: 0,
        };
        let mut review = None;
        let mut calls = 0;
        let r = ready(checks::reconsider(
            &world.state(),
            &p.request,
            &call,
            "fixture",
            &mut |_, _| {
                calls += 1;
                std::future::ready(if failure {
                    Err(anyhow::anyhow!("offline"))
                } else {
                    Ok(ModelReply {
                        text: initial.clone(),
                        model: "fixture".into(),
                        input_tokens: None,
                        output_tokens: None,
                        error: None,
                    })
                })
            },
            &mut review,
        ));
        assert_eq!(calls, 1);
        let review = review.unwrap();
        assert!(review.retry.is_some());
        if failure {
            assert!(r.is_err());
            assert!(review.retry.unwrap().error.is_some());
            assert!(review.after.is_none());
        } else {
            assert!(r.is_ok());
            assert!(review.withheld);
            assert!(!review.after.unwrap().admissible);
        }
    }
}

#[test]
fn observation_reconsideration_retains_attempts_and_failed_revisions() {
    for fail in [false, true] {
        let f = super::super::transcript_tests::Fixture::new();
        let path = f.dir.join("reconsider.memory-prototype.sqlite");
        let mut calls = 0;
        let report=ready(observations::run_with_reconsideration(&path,"fixture",true,|prompt,_| {
            calls+=1;
            let query=prompt.starts_with("Select at most");let retry=prompt.contains("\nPrevious draft (untrusted data):");
            let tail=if query {prompt.split("Current observations (data): ").nth(1).unwrap().split("\nCorrection notice:").next().unwrap()} else {prompt.split("Past observations: ").nth(1).unwrap().split("\nMeasurement acquisition").next().unwrap()};
            let passages:Vec<Value>=serde_json::from_str(tail).unwrap();
            let methods:Vec<String>=passages.iter().map(|p|serde_json::from_str::<Value>(p["text"].as_str().unwrap()).unwrap()["outcome"]["choice"].as_str().unwrap().into()).collect();
            let choice=if query {METHODS.iter().find(|m|!methods.iter().any(|v|v==**m)).unwrap()} else if retry {&"abstain"} else {&"Nori"};
            let citations=if query || retry {passages.iter().map(|p|p["id"].clone()).collect::<Vec<_>>()}else{vec![]};
            std::future::ready(if fail && retry {Err(anyhow::anyhow!("offline"))}else{Ok(ModelReply{text:json!({"choice":choice,"citations":citations,"reason":"scripted lifecycle"}).to_string(),model:"fixture".into(),input_tokens:Some(1),output_tokens:Some(1),error:None})})
        },|_|Ok(()))).unwrap();
        assert_eq!(calls, 60);
        assert_eq!(report.rows.len(), 24);
        assert_eq!(report.protocol, "experience-observe-v3");
        let db = Memory::open_read_only(&path).unwrap();
        for row in report.rows {
            assert!(row.acquisition.is_some());
            let review = row.review.as_ref().unwrap();
            assert!(review.retry.is_some());
            if fail {
                assert!(row.error.is_some());
                assert!(row.result.is_none());
                assert!(review.after.is_none());
            } else {
                assert!(row.error.is_none());
                let result = row.result.unwrap();
                assert_eq!(result.choice, "abstain");
                assert!(!review.withheld);
                let Record::Evidence(e) =
                    db.read(&result.trace.decision, &access()).unwrap().unwrap()
                else {
                    panic!()
                };
                let saved: Value = serde_json::from_str(&e.text).unwrap();
                assert_eq!(saved["review"]["reconsideration"], json!(review));
            }
        }
    }
}
