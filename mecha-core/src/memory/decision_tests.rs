use super::decision::*;
use super::RevisionRef;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn contract() -> Contract {
    Contract {
        workload: "Fixture".into(),
        objective: Objective::MinimizeMinutes,
        energy_budget: 8,
        minimum_quality: 7,
    }
}
fn content() -> String {
    json!({"kind":"host_authored_synthetic_measurements_v1","workload":"Fixture","observations":[
        {"choice":"Nori","observed_minutes":4,"energy":8,"quality":8},
        {"choice":"Arden","observed_minutes":9,"energy":2,"quality":9},
        {"choice":"Quill","observed_minutes":6,"energy":4,"quality":8}],"complete":true})
    .to_string()
}
fn binding(text: &str) -> ReadBinding {
    ReadBinding {
        task: RevisionRef {
            object: "task".into(),
            revision: "t1".into(),
        },
        observation: RevisionRef {
            object: "observation".into(),
            revision: "o1".into(),
        },
        read_sha256: Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
fn prepared(view: EvidenceView) -> Prepared {
    let text = content();
    prepare(&contract(), binding(&text), &text, view).unwrap()
}
fn reply(choice: &str, citations: Value) -> String {
    json!({"choice":choice,"citations":citations,"reason":"Compare the supplied observations."})
        .to_string()
}

#[test]
fn source_projection_keeps_original_binding_and_distinct_hashes() {
    let p = prepared(EvidenceView::Observed);
    assert_eq!(p.request().passages.len(), 3);
    let d =
        serde_json::to_value(p.finish(&reply("Nori", json!(["e1", "e2", "e3"]))).unwrap()).unwrap();
    assert_eq!(d["disposition"], "recommend");
    assert_eq!(d["binding"]["observation"]["revision"], "o1");
    assert_eq!(d["binding"]["read_sha256"], binding(&content()).read_sha256);
    assert_eq!(d["request_sha256"], p.request_sha256());
    assert_eq!(
        p.request().passages[0].sha256,
        binding(&content()).read_sha256
    );
    assert_eq!(p.request().passages[0].text, content());
    assert_ne!(
        p.request().passages[0].projection.as_ref().unwrap().sha256,
        binding(&content()).read_sha256
    );
    assert_eq!(p.request().passages[0].source.object, "observation");
}

#[test]
fn no_memory_validates_source_but_has_no_measurement_passages() {
    let p = prepared(EvidenceView::None);
    assert!(p.request().passages.is_empty());
    assert!(!p.request().prompt.contains("observed_minutes"));
    let abstain = p.finish(&reply("abstain", json!([]))).unwrap();
    assert_eq!(
        serde_json::to_value(abstain).unwrap()["disposition"],
        "abstain"
    );
    let chosen = p.finish(&reply("Nori", json!([]))).unwrap();
    assert_eq!(
        serde_json::to_value(chosen).unwrap()["disposition"],
        "withheld"
    );
    assert_ne!(
        p.request_sha256(),
        prepared(EvidenceView::Observed).request_sha256()
    );
}

#[test]
fn malformed_duplicate_unknown_and_mismatched_source_reject_both_views() {
    let original = content();
    let mut cases = vec![
        String::new(),
        original[..original.len() - 1].into(),
        format!("[truncated] {original}"),
        format!("{original} {{}}"),
        original.replace("\"complete\":true", "\"complete\":false"),
        original.replace("\"complete\":true", "\"complete\":true,\"complete\":true"),
        original.replace("\"energy\":8", "\"energy\":8,\"energy\":8"),
        original.replace("\"quality\":8", "\"quality\":-1"),
        original.replace("\"quality\":8", "\"quality\":1.5"),
        original.replace("\"quality\":8", "\"quality\":4294967296"),
    ];
    let base: Value = serde_json::from_str(&original).unwrap();
    for (pointer, value) in [
        ("/kind", json!("unknown")),
        ("/workload", json!("Other")),
        ("/observations/0/choice", json!("Other")),
    ] {
        let mut v = base.clone();
        *v.pointer_mut(pointer).unwrap() = value;
        cases.push(v.to_string());
    }
    let mut unknown = base.clone();
    unknown["extra"] = json!(true);
    cases.push(unknown.to_string());
    let mut unknown = base.clone();
    unknown["observations"][0]["extra"] = json!(true);
    cases.push(unknown.to_string());
    let mut duplicate = base.clone();
    duplicate["observations"][1] = duplicate["observations"][0].clone();
    cases.push(duplicate.to_string());
    for text in cases {
        for view in [EvidenceView::None, EvidenceView::Observed] {
            assert!(
                prepare(&contract(), binding(&text), &text, view).is_err(),
                "{text}"
            );
        }
    }
}

#[test]
fn read_hash_and_contract_workload_are_validated() {
    let text = content();
    let mut b = binding(&text);
    b.read_sha256 = "0".repeat(64);
    assert!(prepare(&contract(), b, &text, EvidenceView::Observed).is_err());
    for workload in ["".to_string(), " ".into(), "x".repeat(129)] {
        let mut c = contract();
        c.workload = workload;
        assert!(prepare(&c, binding(&text), &text, EvidenceView::None).is_err());
    }
}

#[test]
fn missing_alternatives_withhold_a_choice_without_inventing_evidence() {
    let mut v: Value = serde_json::from_str(&content()).unwrap();
    v["observations"].as_array_mut().unwrap().pop();
    let text = v.to_string();
    let p = prepare(&contract(), binding(&text), &text, EvidenceView::Observed).unwrap();
    let d = serde_json::to_value(p.finish(&reply("Nori", json!(["e1", "e2"]))).unwrap()).unwrap();
    assert_eq!(d["disposition"], "withheld");
    assert_eq!(d["assessment"]["missing"], json!(["Quill"]));
    assert!(p
        .reconsideration(&reply("Nori", json!(["e1", "e2"])))
        .unwrap()
        .is_some());
    assert!(p
        .reconsideration(&reply("abstain", json!([])))
        .unwrap()
        .is_none());
}

#[test]
fn objective_feasibility_ties_and_suboptimal_advice_use_only_supplied_values() {
    let text = content();
    let mut c = contract();
    c.objective = Objective::MinimizeEnergy;
    let p = prepare(&c, binding(&text), &text, EvidenceView::Observed).unwrap();
    let n = reply("Nori", json!(["e1", "e2", "e3"]));
    assert!(p.assess(&n).unwrap().admissible);
    assert!(p.reconsideration(&n).unwrap().is_some());
    assert!(p
        .reconsideration(&reply("Arden", json!(["e1", "e2", "e3"])))
        .unwrap()
        .is_none());
    c.energy_budget = 4;
    let p = prepare(&c, binding(&text), &text, EvidenceView::Observed).unwrap();
    assert!(!p.assess(&n).unwrap().admissible);
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["observations"][2]["observed_minutes"] = json!(4);
    let text = v.to_string();
    let p = prepare(&contract(), binding(&text), &text, EvidenceView::Observed).unwrap();
    assert!(p
        .reconsideration(&reply("Quill", json!(["e1", "e2", "e3"])))
        .unwrap()
        .is_none());
}

#[test]
fn malformed_model_responses_are_terminal_errors_without_retry_requests() {
    let p = prepared(EvidenceView::Observed);
    for raw in [
        "{}".to_string(),
        "refused".into(),
        reply("Other", json!([])),
        reply("Nori", json!(["foreign"])),
        reply("Nori", json!(["e1", "e1"])),
        reply("Nori", json!([])).replace(
            "\"choice\":\"Nori\"",
            "\"choice\":\"Nori\",\"choice\":\"Arden\"",
        ),
    ] {
        assert!(p.assess(&raw).is_err());
        assert!(p.reconsideration(&raw).is_err());
        assert!(p.finish(&raw).is_err());
    }
}

#[test]
fn revision_preserves_evidence_and_schema_and_does_not_mutate_initial_request() {
    let p = prepared(EvidenceView::Observed);
    let before = p.request().prompt.clone();
    let hash = p.request_sha256().to_string();
    let retry = p
        .reconsideration(&reply("abstain", json!([])))
        .unwrap()
        .unwrap();
    assert!(retry.prompt.starts_with(&before));
    assert_eq!(retry.response_schema, p.request().response_schema);
    assert_eq!(retry.passages.len(), 3);
    assert_eq!(p.request().prompt, before);
    assert_eq!(p.request_sha256(), hash);
}
