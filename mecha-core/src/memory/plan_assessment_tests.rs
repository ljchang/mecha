use super::{
    context::*,
    plan_assessment::*,
    transcript_tests::{access, log, Fixture},
    *,
};
use serde_json::{json, Value};
fn setup() -> (
    Fixture,
    ContextPolicy,
    ContextRequest,
    Situation,
    Vec<Candidate>,
    RevisionRef,
) {
    let mut f = Fixture::new();
    let r = f.db.import_session_jsonl(log("customs", &["Customs clearance is pending. No waiver is available. The alternate route has not been assessed."]).as_bytes(), "a", access().scopes).unwrap().records[1].clone();
    let mut recall = super::recall_tests::query(&f.db, "customs");
    recall.account = Some("a".into());
    recall.neighbors = 0;
    let policy = ContextPolicy {
        access: access(),
        recall,
        valid_at: None,
    };
    let request = ContextRequest {
        query: "customs".into(),
        bindings: vec![],
    };
    let situation = Situation {
        event: "Plan a permitted shipment; investigate unknown prerequisites before booking."
            .into(),
        goal: "task:shipment".into(),
        needs: None,
    };
    let candidates = vec![Candidate {
        id: "reroute".into(),
        steps: vec!["Book the alternate route now to bypass customs.".into()],
    }];
    (f, policy, request, situation, candidates, r)
}
fn draft(_r: &RevisionRef) -> Value {
    json!({"assessments":[{"id":"reroute", "requirements":[], "checks":[
        {"dimension":"prerequisites","status":"unknown","reason":"Alternate-route clearance is not established.","citations":[],"concept_citations":[],"questions":["Does the alternate route also require clearance?"]},
        {"dimension":"factual_support","status":"contradicted","reason":"Clearance remains pending.","citations":["p2"],"concept_citations":[],"questions":[]},
        {"dimension":"outcome","status":"unknown","reason":"Delivery outcome is unresolved.","citations":[],"concept_citations":[],"questions":["Will delivery finish?"]}
    ]}],"ranking":["reroute"]})
}

#[test]
fn unknown_and_contradicted_plans_are_not_recommended() {
    let (f, p, q, s, c, r) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let result =
        f.db.finish_plan_assessment(receipt, &draft(&r).to_string(), &access())
            .unwrap();
    assert!(result.recommended.is_none());
    assert_eq!(result.assessments[0].disposition, Disposition::Revise);
    assert!(result.private_data && result.untrusted_input);
    let mut unknown = draft(&r);
    unknown["assessments"][0]["checks"][1]["status"] = json!("supported");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let result =
        f.db.finish_plan_assessment(receipt, &unknown.to_string(), &access())
            .unwrap();
    assert_eq!(
        result.assessments[0].disposition,
        Disposition::NeedsEvidence
    );
    assert!(result.recommended.is_none());
}
#[test]
fn malformed_or_invented_judgments_fail_closed() {
    let (f, p, q, s, c, r) = setup();
    let original = draft(&r);
    let mut bad = Vec::new();
    let mut v = original.clone();
    v["assessments"][0]["checks"][1]["citations"] = json!([{"source_id":r.revision,"field":"/content/0/text","quote":"Customs clearance is pending."}]);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"][1]["citations"] = json!(["p999"]);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"][1]["citations"] = json!(["p2", "p2"]);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"].as_array_mut().unwrap().pop();
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"][0]["questions"] = json!([]);
    bad.push(v);
    let mut v = original.clone();
    v["ranking"] = json!([]);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["id"] = json!("invented");
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"][1]["dimension"] = json!("prerequisites");
    bad.push(v);
    let mut v = original.clone();
    v["trusted"] = json!(true);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["checks"][0]["status"] = json!("supported");
    v["assessments"][0]["checks"][0]["citations"] =
        v["assessments"][0]["checks"][1]["citations"].clone();
    bad.push(v); // A declared supported check cannot still have an unresolved question.
    for v in bad {
        let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
        assert!(
            f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
                .is_err(),
            "{v}"
        );
    }
}
#[test]
fn source_changes_during_assessment_invalidate_even_uncited_evidence() {
    let (mut f, p, q, s, c, r) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    f.db.conn
        .execute("INSERT INTO scope VALUES (?1, 'extra')", [&r.revision])
        .unwrap();
    let mut broad = access();
    broad.scopes.insert("extra".into());
    assert!(f
        .db
        .finish_plan_assessment(receipt, &draft(&r).to_string(), &broad)
        .is_err());
    let receipt =
        f.db.prepare_plan_assessment(
            &q,
            &ContextPolicy {
                access: broad.clone(),
                ..p
            },
            &s,
            &c,
        )
        .unwrap();
    f.db.forget(&transcript_source("a", "customs", 2)).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &draft(&r).to_string(), &broad)
        .is_err());
}

fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::{
        pin::pin,
        task::{Context, Poll, Waker},
    };
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("scripted pass must be ready"),
    }
}
#[test]
fn revision_uses_assessment_and_is_reassessed_without_promoting_unknowns() {
    let (f, p, q, s, c, r) = setup();
    let mut calls = 0;
    let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |call| {
        calls += 1;
        let reply = match calls {
            1 => draft(&r).to_string(),
            2 => {
                assert!(matches!(call.stage, Stage::Revision));
                assert!(call
                    .prompt
                    .contains("Does the alternate route also require clearance?"));
                json!({"candidate":"reroute","edits":[{"step":1,"operation":"defer"}]}).to_string()
            }
            3 => {
                assert!(matches!(call.stage, Stage::Assessment));
                assert!(call.prompt.contains("Defer original step 1"));
                let mut v = draft(&r);
                v["assessments"][0]["id"] = json!("revision");
                v["ranking"] = json!(["revision"]);
                v.to_string()
            }
            _ => panic!("unbounded loop"),
        };
        std::future::ready(Ok(reply))
    }))
    .unwrap();
    assert_eq!(calls, 3);
    assert!(result.initial.recommended.is_none());
    assert!(result.revision.unwrap().recommended.is_none());
}
#[test]
fn forgetting_during_model_call_aborts_the_loop() {
    let (f, p, q, s, c, r) = setup();
    let mut writer = Memory::open(&f.dir.join("test.memory-prototype.sqlite")).unwrap();
    let mut calls = 0;
    let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |_| {
        calls += 1;
        writer
            .forget(&transcript_source("a", "customs", 2))
            .unwrap();
        std::future::ready(Ok(draft(&r).to_string()))
    }));
    assert!(result.is_err());
    assert_eq!(calls, 1);
}
#[test]
fn concept_cannot_alone_support_facts_and_model_sees_its_counterexamples() {
    let (mut f, mut p, q, s, c, r) = setup();
    let other =
        f.db.import_session_jsonl(
            log(
                "other",
                &["Customs held another shipment pending inspection."],
            )
            .as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap()
        .records[1]
            .clone();
    let receipt =
        f.db.prepare_concept(
            &[r.clone(), other.clone()],
            &f.db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let concept=f.db.retain_concept(receipt,&json!({"name":"Customs dependency", "principle":"Clearance can remain a blocking prerequisite.","applicability":"A different route need not remove this requirement.", "support":[{"source_id":r.revision,"field":"/content/0/text","quote":"Customs clearance is pending."},{"source_id":other.revision,"field":"/content/0/text","quote":"Customs held another shipment"}],"counterexamples":[]}).to_string(),&access(),None).unwrap();
    p.recall.known_at = f.db.snapshot().unwrap();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    assert!(receipt
        .generation()
        .prompt
        .contains("\"counterexamples\":[]"));
    let mut v = draft(&r);
    let data: Value =
        serde_json::from_str(receipt.generation().prompt.split_once('\n').unwrap().1).unwrap();
    let concept_source = data["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == concept.revision)
        .unwrap();
    let concept_id = concept_source["fields"][1]["passages"][0]["id"].clone();
    let schema = receipt.generation().response_schema;
    for branch in schema["properties"]["assessments"]["items"]["anyOf"][0]["properties"]["checks"]
        ["items"]["anyOf"]
        .as_array()
        .unwrap()
    {
        let fields = &branch["properties"];
        assert!(
            fields["citations"]["maxItems"].as_u64().unwrap()
                + fields["concept_citations"]["maxItems"].as_u64().unwrap()
                <= 6
        );
    }
    let properties = &schema["properties"]["assessments"]["items"]["anyOf"][0]["properties"]
        ["checks"]["items"]["anyOf"][0]["properties"];
    assert!(!properties["citations"]["items"]["enum"]
        .as_array()
        .unwrap()
        .contains(&concept_id));
    assert!(properties["concept_citations"]["items"]["enum"]
        .as_array()
        .unwrap()
        .contains(&concept_id));
    v["assessments"][0]["checks"][1]["citations"] = json!(["p1", concept_id]);
    v["assessments"][0]["checks"].as_array_mut().unwrap().push(json!({
        "dimension":"concept_applicability","status":"unknown","reason":"Applicability is unresolved.",
        "citations":[],"concept_citations":[],"questions":["Does this concept apply here?"]}));
    assert!(f
        .db
        .finish_plan_assessment(receipt, &v.to_string(), &access())
        .is_err());
    // Putting the concept in its proper category still cannot make it sole factual evidence.
    v["assessments"][0]["checks"][1]["citations"] = json!([]);
    v["assessments"][0]["checks"][1]["concept_citations"] = json!([concept_id]);
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &v.to_string(), &access())
        .is_err());
    v["assessments"][0]["checks"][1]["citations"] = json!(["p1"]);
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &v.to_string(), &access())
        .is_ok());
}

#[test]
fn ranking_cannot_prefer_an_unresolved_plan_over_a_fully_assessed_candidate() {
    let (f, p, q, s, mut candidates, r) = setup();
    candidates.push(Candidate {
        id: "investigate".into(),
        steps: vec!["Ask which customs prerequisites apply before booking.".into()],
    });
    let mut value = draft(&r);
    let mut safe = value["assessments"][0].clone();
    safe["id"] = json!("investigate");
    for index in [0, 1] {
        safe["checks"][index]["status"] = json!("supported");
        safe["checks"][index]["reason"] =
            json!("The situation requests investigation before booking.");
        safe["checks"][index]["questions"] = json!([]);
        safe["checks"][index]["citations"] = json!(["p1"]);
    }
    value["assessments"].as_array_mut().unwrap().push(safe);
    value["ranking"] = json!(["reroute", "investigate"]);
    let receipt =
        f.db.prepare_plan_assessment(&q, &p, &s, &candidates)
            .unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &value.to_string(), &access())
            .unwrap();
    assert_eq!(report.recommended.as_deref(), Some("investigate"));
}

#[test]
fn assessment_schema_enumerates_issued_passage_ids() {
    let (f, p, q, s, c, _) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let schema = receipt.generation().response_schema;
    let fields = &schema["properties"]["assessments"]["items"]["anyOf"][0]["properties"]["checks"]
        ["items"]["anyOf"][0]["properties"]["citations"]["items"]["enum"];
    assert_eq!(fields, &json!(["p1", "p2"]));
}

#[test]
fn selected_passage_constructs_the_quote_and_location_in_the_host() {
    let (f, p, q, s, c, r) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let data: Value =
        serde_json::from_str(receipt.generation().prompt.split_once('\n').unwrap().1).unwrap();
    let passage = data["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["id"] == r.revision)
        .unwrap()["fields"][0]["passages"][0]
        .clone();
    assert!(passage["id"].is_string(), "host must issue passage IDs");
    let mut value = draft(&r);
    value["assessments"][0]["checks"][1]["citations"] = json!([passage["id"]]);
    let report =
        f.db.finish_plan_assessment(receipt, &value.to_string(), &access())
            .unwrap();
    let report = serde_json::to_value(report).unwrap();
    let citation = &report["assessments"][0]["checks"][1]["citations"][0];
    assert_eq!(citation["source_id"], r.revision);
    assert_eq!(citation["field"], "/content/0/text");
    assert_eq!(citation["start"], 0);
    assert_eq!(citation["end"], passage["text"].as_str().unwrap().len());
    assert_eq!(citation["quote"], passage["text"]);
}

#[test]
fn passages_preserve_long_utf8_fields_and_resolve_only_inside_the_receipt() {
    let (f, p, q, mut s, c, r) = setup();
    for text in ["é 🐝 no clearance. ".repeat(120), "蜂".repeat(1200)] {
        s.event = text.clone();
        let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
        let data: Value =
            serde_json::from_str(receipt.generation().prompt.split_once('\n').unwrap().1).unwrap();
        let passages = data["sources"][0]["fields"][0]["passages"]
            .as_array()
            .unwrap();
        assert!(passages.len() > 1);
        let mut end = 0;
        let mut reconstructed = String::new();
        for passage in passages {
            assert_eq!(passage["start"], end);
            let start = end;
            end = passage["end"].as_u64().unwrap() as usize;
            assert!(end - start <= 1024);
            assert_eq!(passage["text"], text[start..end]);
            reconstructed.push_str(passage["text"].as_str().unwrap());
        }
        assert_eq!(reconstructed, text);
        let mut v = draft(&r);
        v["assessments"][0]["checks"][1]["citations"] = json!([passages.last().unwrap()["id"]]);
        let report =
            f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
                .unwrap();
        let citation = &report.assessments[0].checks[1].citations[0];
        assert_eq!(citation.source_id, "situation");
        assert_eq!(citation.quote, text[citation.start..citation.end]);
        assert_eq!(citation.end, text.len());
    }
}

#[test]
fn source_forgetting_during_revision_or_final_assessment_aborts_the_report() {
    for fail_call in [2, 3] {
        let (f, p, q, s, c, r) = setup();
        let mut writer = Memory::open(&f.dir.join("test.memory-prototype.sqlite")).unwrap();
        let mut calls = 0;
        let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |_| {
            calls += 1;
            if calls == fail_call {
                writer
                    .forget(&transcript_source("a", "customs", 2))
                    .unwrap();
            }
            let response = if calls == 2 {
                json!({"candidate":"reroute","edits":[{"step":1,"operation":"defer"}]})
            } else {
                let mut v = draft(&r);
                if calls == 3 {
                    v["assessments"][0]["id"] = json!("revision");
                    v["ranking"] = json!(["revision"]);
                }
                v
            };
            std::future::ready(Ok(response.to_string()))
        }));
        assert!(result.is_err());
        assert_eq!(calls, fail_call);
    }
}

#[test]
fn empty_packet_cannot_issue_an_empty_enum_schema() {
    let (f, p, mut q, _, c, _) = setup();
    q.query = "zxqvzznonsense".into();
    let empty = Situation {
        event: String::new(),
        goal: String::new(),
        needs: None,
    };
    assert!(f.db.prepare_plan_assessment(&q, &p, &empty, &c).is_err());
}

#[test]
fn absent_concept_check_is_supplied_by_the_host() {
    let (f, p, q, s, c, r) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let mut value = draft(&r);
    value["assessments"][0]["checks"]
        .as_array_mut()
        .unwrap()
        .retain(|c| c["dimension"] != "concept_applicability");
    for c in value["assessments"][0]["checks"].as_array_mut().unwrap() {
        c["concept_citations"] = json!([]);
    }
    let report =
        f.db.finish_plan_assessment(receipt, &value.to_string(), &access())
            .unwrap();
    let report = serde_json::to_value(report).unwrap();
    assert_eq!(
        report["assessments"][0]["checks"][3]["status"],
        "not_applicable"
    );
    assert_eq!(
        report["assessments"][0]["checks"][3]["judgment_source"],
        "host"
    );
}

#[test]
fn adequate_candidate_is_preserved_without_revision_calls() {
    let (f, p, q, s, c, r) = setup();
    let mut value = draft(&r);
    for i in [0, 1] {
        value["assessments"][0]["checks"][i]["status"] = json!("supported");
        value["assessments"][0]["checks"][i]["citations"] = json!(["p1"]);
        value["assessments"][0]["checks"][i]["questions"] = json!([]);
    }
    let mut calls = 0;
    let report = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |_| {
        calls += 1;
        assert_eq!(calls, 1, "an adequate candidate must not be rewritten");
        std::future::ready(Ok(value.to_string()))
    }))
    .unwrap();
    assert!(report.revision.is_none());
    assert_eq!(report.initial.candidates[0].steps, c[0].steps);
    assert_eq!(
        serde_json::to_value(report).unwrap()["revision_status"],
        "kept_existing"
    );
}

#[test]
fn model_cannot_supply_absent_concepts_or_host_judgments() {
    let (f, p, q, s, c, r) = setup();
    for status in ["supported", "unknown", "not_applicable"] {
        let mut value = draft(&r);
        value["assessments"][0]["checks"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "dimension":"concept_applicability","status":status,"reason":"Invented task concept.",
            "citations":["p1"],"concept_citations":[],"questions":[]}));
        let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
        assert!(f
            .db
            .finish_plan_assessment(receipt, &value.to_string(), &access())
            .is_err());
    }
    let mut value = draft(&r);
    value["assessments"][0]["checks"][0]["judgment_source"] = json!("host");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &value.to_string(), &access())
        .is_err());
}

#[test]
fn acceptable_alternative_avoids_rewriting_bad_candidates_and_no_request_is_distinct() {
    let (f, p, q, s, mut candidates, r) = setup();
    candidates.push(Candidate {
        id: "investigate".into(),
        steps: vec!["Ask before acting.".into()],
    });
    let mut value = draft(&r);
    let mut safe = value["assessments"][0].clone();
    safe["id"] = json!("investigate");
    for check in safe["checks"].as_array_mut().unwrap() {
        check["status"] = json!("supported");
        check["citations"] = json!(["p1"]);
        check["questions"] = json!([]);
    }
    value["assessments"].as_array_mut().unwrap().push(safe);
    value["ranking"] = json!(["reroute", "investigate"]);
    for revise in [true, false] {
        let mut calls = 0;
        let report = ready(f.db.evaluate_plans(&q, &p, &s, &candidates, revise, |_| {
            calls += 1;
            assert_eq!(calls, 1);
            std::future::ready(Ok(value.to_string()))
        }))
        .unwrap();
        assert_eq!(report.initial.recommended.as_deref(), Some("investigate"));
        assert!(report.revision.is_none());
        assert_eq!(
            serde_json::to_value(report).unwrap()["revision_status"],
            if revise {
                "kept_existing"
            } else {
                "not_requested"
            }
        );
    }
}

fn guarded_draft(r: &RevisionRef) -> Value {
    let mut v = draft(r);
    for check in v["assessments"][0]["checks"].as_array_mut().unwrap() {
        check["status"] = json!("supported");
        check["citations"] = json!(["p1"]);
        check["questions"] = json!([]);
    }
    v["assessments"][0]["requirements"] = json!([{
        "step":1,"condition":"Clearance obtained", "scope":"universal",
        "scope_description":"Every shipment route", "state":"unmet",
        "citations":["p2"],"guard":null
    }]);
    v["assessments"][0]["checks"][2]["status"] = json!("unknown");
    v["assessments"][0]["checks"][2]["questions"] = json!(["Will delivery finish?"]);
    v
}

#[test]
fn explicit_unmet_requirements_override_optimistic_model_checks() {
    let (f, p, q, s, c, r) = setup();
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &guarded_draft(&r).to_string(), &access())
            .unwrap();
    assert_eq!(report.assessments[0].disposition, Disposition::Revise);
    assert!(report.recommended.is_none());
}

#[test]
fn exact_guard_allows_consideration_without_claiming_a_certain_outcome() {
    let (f, p, q, s, mut c, r) = setup();
    c[0].steps = vec!["If clearance is obtained, book; otherwise wait.".into()];
    let mut v = guarded_draft(&r);
    v["assessments"][0]["requirements"][0]["guard"] =
        json!("If clearance is obtained, book; otherwise wait.");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap();
    assert_eq!(report.recommended.as_deref(), Some("reroute"));
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["evaluation_protocol"], "bound-repair-v5");
    assert_eq!(
        value["assessments"][0]["requirements"][0]["citations"][0]["source_id"],
        r.revision
    );
    v["assessments"][0]["requirements"][0]["scope"] = json!("unknown");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap();
    assert_eq!(
        report.assessments[0].disposition,
        Disposition::NeedsEvidence
    );
}

#[test]
fn requirement_guards_cannot_be_borrowed_from_another_step_or_invented() {
    let (f, p, q, s, mut c, r) = setup();
    c[0].steps
        .push("If clearance is obtained, book; otherwise wait.".into());
    let original = guarded_draft(&r);
    let mut bad = Vec::new();
    let mut v = original.clone();
    v["assessments"][0]["requirements"][0]["guard"] =
        json!("If clearance is obtained, book; otherwise wait.");
    bad.push(v);
    for step in [0, 3] {
        let mut v = original.clone();
        v["assessments"][0]["requirements"][0]["step"] = json!(step);
        bad.push(v);
    }
    let mut v = original.clone();
    v["assessments"][0]["requirements"][0]["citations"] = json!(["p999"]);
    bad.push(v);
    let mut v = original.clone();
    v["assessments"][0]["requirements"][0]["citations"] = json!([]);
    bad.push(v);
    let mut v = original.clone();
    let copy = v["assessments"][0]["requirements"][0].clone();
    v["assessments"][0]["requirements"]
        .as_array_mut()
        .unwrap()
        .push(copy);
    bad.push(v);
    for v in bad {
        let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
        assert!(
            f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
                .is_err(),
            "{v}"
        );
    }
}

#[test]
fn omitted_guard_field_is_not_a_complete_requirement_assessment() {
    let (f, p, q, s, c, r) = setup();
    let mut v = guarded_draft(&r);
    v["assessments"][0]["requirements"][0]
        .as_object_mut()
        .unwrap()
        .remove("guard");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &v.to_string(), &access())
        .is_err());
}

#[test]
fn requirement_unknowns_and_contradicted_outcomes_keep_their_distinct_effects() {
    let (f, p, q, s, c, r) = setup();
    let mut v = guarded_draft(&r);
    v["assessments"][0]["requirements"][0]["state"] = json!("unknown");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap();
    assert_eq!(
        report.assessments[0].disposition,
        Disposition::NeedsEvidence
    );
    let mut blocker = v["assessments"][0]["requirements"][0].clone();
    blocker["condition"] = json!("Required approval");
    blocker["state"] = json!("unmet");
    v["assessments"][0]["requirements"]
        .as_array_mut()
        .unwrap()
        .push(blocker);
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap();
    assert_eq!(report.assessments[0].disposition, Disposition::Revise);
    v["assessments"][0]["requirements"] = json!([]);
    v["assessments"][0]["checks"][2]["status"] = json!("contradicted");
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let report =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap();
    assert_eq!(report.assessments[0].disposition, Disposition::Revise);
    assert!(report.recommended.is_none());
}

#[test]
fn requirement_schema_uses_each_candidates_steps_and_requires_original_evidence() {
    let (f, p, q, s, mut c, r) = setup();
    c.push(Candidate {
        id: "guarded".into(),
        steps: vec![
            "Ask about clearance.".into(),
            "If cleared, book; otherwise wait.".into(),
        ],
    });
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let schema = receipt.generation().response_schema;
    let branches = schema["properties"]["assessments"]["items"]["anyOf"]
        .as_array()
        .unwrap();
    assert_eq!(branches.len(), 2);
    for (branch, candidate) in branches.iter().zip(&c) {
        assert_eq!(branch["properties"]["id"]["const"], candidate.id);
        let fields = &branch["properties"]["requirements"]["items"]["properties"];
        assert_eq!(fields["step"]["maximum"], candidate.steps.len());
        assert_eq!(fields["guard"]["anyOf"][1]["enum"], json!(candidate.steps));
        assert_eq!(fields["citations"]["minItems"], 1);
    }
    // Even entirely unknown requirements must identify the source of the question.
    let mut v = guarded_draft(&r);
    v["assessments"][0]["requirements"][0]["scope"] = json!("unknown");
    v["assessments"][0]["requirements"][0]["state"] = json!("unknown");
    v["assessments"][0]["requirements"][0]["citations"] = json!([]);
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c[..1]).unwrap();
    assert!(f
        .db
        .finish_plan_assessment(receipt, &v.to_string(), &access())
        .is_err());
    // A guard owned by a different candidate cannot confer permission here.
    let mut v = guarded_draft(&r);
    v["assessments"][0]["requirements"][0]["guard"] = json!(c[1].steps[1]);
    let mut other = draft(&r)["assessments"][0].clone();
    other["id"] = json!("guarded");
    v["assessments"].as_array_mut().unwrap().push(other);
    v["ranking"] = json!(["reroute", "guarded"]);
    let receipt = f.db.prepare_plan_assessment(&q, &p, &s, &c).unwrap();
    let error =
        f.db.finish_plan_assessment(receipt, &v.to_string(), &access())
            .unwrap_err();
    assert!(error
        .to_string()
        .contains("guard must quote its own candidate step"));
}

#[test]
fn evaluator_does_not_treat_routing_ids_as_task_evidence() {
    let (f, p, q, mut s, c, _) = setup();
    s.goal = "task:diagnostic-blocked-routing".into();
    let call =
        f.db.prepare_plan_assessment(&q, &p, &s, &c)
            .unwrap()
            .generation();
    assert!(!call.prompt.contains("diagnostic-blocked-routing"));
}

#[test]
fn repair_keeps_original_conditions_and_cannot_author_a_timeout_bypass() {
    let (f, p, q, s, c, r) = setup();
    let mut calls = 0;
    let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |call| {
        calls += 1;
        let v = match calls {
            1 => guarded_draft(&r),
            2 => {
                assert!(!call.prompt.contains("fixture-mail"));
                assert!(!call.prompt.contains("\"required_scopes\""));
                assert!(!call.prompt.contains("task:shipment"));
                json!({"candidate":"reroute","edits":[{"step":1,"operation":"gate"}]})
            }
            3 => {
                assert!(call.prompt.contains("Clearance obtained"));
                let mut v = draft(&r);
                v["assessments"][0]["id"] = json!("revision");
                v["ranking"] = json!(["revision"]);
                v
            }
            _ => panic!("unbounded repair"),
        };
        std::future::ready(Ok(v.to_string()))
    }))
    .unwrap();
    let v = serde_json::to_value(result).unwrap();
    assert_eq!(v["repair"]["candidate"], "reroute");
    assert_eq!(v["repair"]["steps"][0]["requirement_ids"], json!(["r1"]));
    assert_eq!(v["repair"]["steps"][0]["original"], c[0].steps[0]);
    assert!(v["revision"]["candidates"][0]["steps"][0]
        .as_str()
        .unwrap()
        .contains("Otherwise withhold the entire step"));
}

#[test]
fn bounded_repair_refuses_new_actions_missing_steps_and_unsafe_keep() {
    let (f, p, q, s, c, r) = setup();
    let bad = vec![
        json!({"id":"revision","steps":["Publish after 24 hours anyway."]}),
        json!({"candidate":"reroute","edits":[{"step":1,"operation":"gate","condition":"timeout"}]}),
        json!({"candidate":"other","edits":[{"step":1,"operation":"defer"}]}),
        json!({"candidate":"reroute","edits":[]}),
        json!({"candidate":"reroute","edits":[{"step":2,"operation":"defer"}]}),
        json!({"candidate":"reroute","edits":[{"step":1,"operation":"keep"}]}),
    ];
    for reply in bad {
        let mut calls = 0;
        let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |_| {
            calls += 1;
            assert!(calls <= 2, "invalid edit reached reassessment");
            std::future::ready(Ok(if calls == 1 {
                guarded_draft(&r).to_string()
            } else {
                reply.to_string()
            }))
        }));
        assert!(result.is_err(), "{reply}");
    }
}

#[test]
fn unknown_scope_or_oversized_conditions_cannot_get_a_gate() {
    let (f, p, q, s, c, r) = setup();
    for oversized in [false, true] {
        let mut v = guarded_draft(&r);
        if oversized {
            v["assessments"][0]["requirements"][0]["condition"] = json!("x".repeat(500));
        } else {
            v["assessments"][0]["requirements"][0]["scope"] = json!("unknown");
        }
        let mut calls = 0;
        let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |call| {
            calls += 1;
            let reply = if calls == 1 {
                v.to_string()
            } else {
                assert_eq!(
                    call.response_schema["anyOf"][0]["properties"]["edits"]["items"]["anyOf"][0]
                        ["properties"]["operation"]["enum"],
                    json!(["defer"])
                );
                json!({"candidate":"reroute","edits":[{"step":1,"operation":"gate"}]}).to_string()
            };
            std::future::ready(Ok(reply))
        }));
        assert!(result.is_err());
        assert_eq!(calls, 2);
    }
}

#[test]
fn assessment_schema_requires_unknown_questions_and_known_evidence() {
    let (f, p, q, s, c, _) = setup();
    let schema =
        f.db.prepare_plan_assessment(&q, &p, &s, &c)
            .unwrap()
            .generation()
            .response_schema;
    let alternatives = &schema["properties"]["assessments"]["items"]["anyOf"][0]["properties"]
        ["checks"]["items"]["anyOf"];
    assert_eq!(alternatives.as_array().unwrap().len(), 3);
    for branch in alternatives.as_array().unwrap() {
        let p = &branch["properties"];
        if p["status"]["const"] == "unknown" {
            assert_eq!(p["questions"]["minItems"], 1);
        } else {
            assert_eq!(p["citations"]["minItems"], 1);
        }
        if p["status"]["const"] == "supported" {
            assert_eq!(p["questions"]["maxItems"], 0);
        }
    }
}

#[test]
fn repaired_gate_carries_each_conditions_target_and_bounds() {
    let (f, p, q, s, mut c, r) = setup();
    c[0].steps = vec!["Use A and B.".into()];
    let mut initial = guarded_draft(&r);
    let mut req = initial["assessments"][0]["requirements"][0].clone();
    req["condition"] = json!("Consent received");
    req["scope_description"] = json!("A for analysis only");
    let mut other = req.clone();
    other["scope_description"] = json!("B for analysis and publication");
    initial["assessments"][0]["requirements"] = json!([req, other]);
    let mut calls = 0;
    let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |call| {
        calls += 1;
        let v = match calls {
            1 => initial.clone(),
            2 => json!({"candidate":"reroute","edits":[{"step":1,"operation":"gate"}]}),
            3 => {
                assert!(call.prompt.contains("A for analysis only"));
                assert!(call.prompt.contains("B for analysis and publication"));
                assert!(call.prompt.contains("Use A and B."));
                let mut v = draft(&r);
                v["assessments"][0]["id"] = json!("revision");
                v["ranking"] = json!(["revision"]);
                v
            }
            _ => panic!("unbounded"),
        };
        std::future::ready(Ok(v.to_string()))
    }))
    .unwrap();
    let v = serde_json::to_value(result).unwrap();
    assert_eq!(
        v["repair"]["steps"][0]["requirement_ids"],
        json!(["r1", "r2"])
    );
}

#[test]
fn all_deferred_repair_is_abstention_even_if_model_endorses_it() {
    let (f, p, q, s, c, r) = setup();
    let mut calls = 0;
    let result = ready(f.db.evaluate_plans(&q, &p, &s, &c, true, |_| {
        calls += 1;
        let v = match calls {
            1 => draft(&r),
            2 => json!({"candidate":"reroute","edits":[{"step":1,"operation":"defer"}]}),
            3 => {
                let mut v = draft(&r);
                v["assessments"][0]["id"] = json!("revision");
                v["ranking"] = json!(["revision"]);
                for check in v["assessments"][0]["checks"].as_array_mut().unwrap() {
                    check["status"] = json!("supported");
                    check["citations"] = json!(["p1"]);
                    check["questions"] = json!([]);
                }
                v
            }
            _ => panic!("unbounded"),
        };
        std::future::ready(Ok(v.to_string()))
    }))
    .unwrap();
    assert!(result.revision.as_ref().unwrap().recommended.is_none());
    assert_eq!(
        serde_json::to_value(result).unwrap()["repair"]["all_deferred"],
        true
    );
}

#[test]
fn repair_rejects_duplicate_reordered_and_all_keep_edits() {
    let (f, p, q, s, mut c, r) = setup();
    c[0].steps.push("Ask for status.".into());
    for (steps, expected) in [
        (vec![1, 1], "repair steps missing, duplicated or reordered"),
        (vec![2, 1], "repair steps missing, duplicated or reordered"),
        (vec![1, 2], "repair made no change"),
    ] {
        let mut calls = 0;
        let result=ready(f.db.evaluate_plans(&q,&p,&s,&c,true,|_|{
            calls+=1;
            assert!(calls<=2);
            let reply=if calls==1 {draft(&r)} else {
                json!({"candidate":"reroute","edits":steps.iter().map(|n|json!({"step":n,"operation":"keep"})).collect::<Vec<_>>()})
            };
            std::future::ready(Ok(reply.to_string()))
        }));
        assert!(result.unwrap_err().to_string().contains(expected));
    }
}
