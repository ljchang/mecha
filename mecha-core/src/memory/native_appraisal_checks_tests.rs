use super::*;
use crate::memory::support_tests::{access, prepared_supported, supported_fixture};

fn response(r: &NativeEvidenceRequest) -> Value {
    json!({"request_key":r.checked_request_key().unwrap(),"artifact_content":"not_assessed","artifact_facts":[],"comparison":"supports","because":[r.view.facts[0].id],"reason":"Synthetic assessment","questions":[],"state":"met"})
}
#[test]
fn mappings_and_execution_only_assessment_are_read_only() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let before = serde_json::to_value(db.snapshot().unwrap()).unwrap();
    for (comparison, state) in [
        ("supports", "met"),
        ("contradicts", "not_met"),
        ("insufficient", "unknown"),
    ] {
        let mut a = response(&r);
        a["comparison"] = json!(comparison);
        a["state"] = json!(state);
        if state == "unknown" {
            a["questions"] = json!(["What observation is missing?"]);
        }
        assert!(db
            .finish_native_checked_appraisal(&r, &a.to_string(), &access())
            .is_ok());
        for wrong in ["met", "not_met", "unknown"] {
            if wrong == state {
                continue;
            }
            a["state"] = json!(wrong);
            assert!(parse_checked(&r, &a.to_string()).is_err());
        }
    }
    assert_eq!(
        before,
        serde_json::to_value(db.snapshot().unwrap()).unwrap()
    );
}
#[test]
fn artifact_ids_require_authenticated_observation_and_matching_status() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let observed = r
        .view
        .facts
        .iter()
        .find(|f| complete_observation(f))
        .unwrap();
    for f in &r.view.facts {
        let mut a = response(&r);
        a["artifact_content"] = json!("complete");
        a["artifact_facts"] = json!([f.id]);
        a["because"] = json!([f.id]);
        assert_eq!(
            parse_checked(&r, &a.to_string()).is_ok(),
            complete_observation(f)
        );
    }
    let mut a = response(&r);
    a["artifact_content"] = json!("unavailable");
    assert!(parse_checked(&r, &a.to_string()).is_err());
    a["artifact_facts"] = json!([observed.id]);
    a["because"] = json!([observed.id]);
    assert!(parse_checked(&r, &a.to_string()).is_err());
    a["artifact_content"] = json!("complete");
    assert!(parse_checked(&r, &a.to_string()).is_ok());
    a["because"] = json!([]);
    a["comparison"] = json!("insufficient");
    a["state"] = json!("unknown");
    a["questions"] = json!(["Missing?"]);
    assert!(parse_checked(&r, &a.to_string()).is_err());
    // Private request mutations exercise empty complete vs unavailable envelopes;
    // authenticated capture/finish paths still recheck the unchanged receipt.
    for (status, text, complete) in [
        (ContentStatus::Complete, Some(String::new()), true),
        (ContentStatus::Complete, None, false),
        (ContentStatus::Missing, None, false),
    ] {
        let mut changed = r.clone();
        let f = changed
            .view
            .facts
            .iter_mut()
            .find(|f| f.id == observed.id)
            .unwrap();
        let NativeValue::ArtifactObservation(o) = &mut f.value else {
            unreachable!()
        };
        o.content_status = status;
        o.artifact_text = text;
        let mut a = response(&changed);
        a["because"] = json!([observed.id]);
        a["artifact_facts"] = json!([observed.id]);
        a["artifact_content"] = json!(if complete { "complete" } else { "unavailable" });
        assert!(parse_checked(&changed, &a.to_string()).is_ok());
        a["artifact_facts"] = json!([]);
        a["artifact_content"] = json!("unavailable");
        assert_eq!(parse_checked(&changed, &a.to_string()).is_ok(), !complete);
    }
}
#[test]
fn protocol_identity_duplicates_and_bounds_are_enforced() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let (input, schema) = r.checked_input(NativeEvidenceFormat::Typed).unwrap();
    let input: Value = serde_json::from_str(&input).unwrap();
    assert_ne!(input["request_key"], json!(r.view.request_key));
    assert_eq!(input["facts"], serde_json::to_value(&r.view.facts).unwrap());
    assert_eq!(
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "request_key",
            "artifact_content",
            "artifact_facts",
            "comparison",
            "because",
            "reason",
            "questions",
            "state"
        ]
    );
    let valid = response(&r);
    let legacy = json!({"request_key":r.view.request_key,"state":"met","because":[r.view.facts[0].id],"reason":"Legacy synthetic assessment","questions":[]});
    assert!(parse_appraisal(&r, &legacy.to_string()).is_ok());
    assert!(parse_checked(&r, &legacy.to_string()).is_err());
    let mut missing_premises = valid.clone();
    missing_premises["because"] = json!([]);
    assert!(parse_checked(&r, &missing_premises.to_string()).is_err());
    let mut missing_question = valid.clone();
    missing_question["state"] = json!("unknown");
    missing_question["comparison"] = json!("insufficient");
    assert!(parse_checked(&r, &missing_question.to_string()).is_err());
    assert!(parse_appraisal(&r, &valid.to_string()).is_err());
    for (field, value) in [
        ("request_key", json!(r.view.request_key)),
        ("because", json!([r.view.facts[0].id, r.view.facts[0].id])),
        ("artifact_facts", json!(["foreign"])),
        ("reason", json!("x".repeat(513))),
        ("reason", json!(" ")),
        ("questions", json!(["x".repeat(65)])),
        ("extra", json!(true)),
    ] {
        let mut a = valid.clone();
        a[field] = value;
        assert!(parse_checked(&r, &a.to_string()).is_err(), "{field}");
    }
    let text = valid.to_string();
    let duplicate = text.replacen('{', "{\"state\":\"met\",", 1);
    assert!(parse_checked(&r, &duplicate).is_err());
    let mut long = valid.clone();
    long["reason"] = json!("🧪".repeat(512));
    assert!(parse_checked(&r, &long.to_string()).is_ok());
    assert!(parse_checked(&r, &" ".repeat(8193)).is_err());
}

#[test]
#[cfg(feature = "memory-prototype")]
fn checked_generation_preserves_budget_and_local_only_no_tools() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let budget = crate::memory_factual_support::GenerationBudget::new(
        1024,
        std::time::Duration::from_secs(30),
    )
    .unwrap();
    let request = r
        .checked_generation_request("synthetic-local", budget, NativeEvidenceFormat::Typed)
        .unwrap();
    assert_eq!(request.max_tokens, 1024);
    assert!(request.tools.is_empty());
    assert_eq!(request.think, Some(false));
    let schema = request.response_schema.unwrap();
    assert_eq!(
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .next_back()
            .unwrap(),
        "state"
    );
    let system = request.system.unwrap();
    assert!(system.contains("CURRENT confirmed criterion"));
    assert!(system.contains("observation from causal identification"));
}
