use super::plan_extraction::*;
use serde_json::{json, Value};

fn input() -> Input {
    serde_json::from_value(json!({"evidence":[{"source_id":"episode:revision-1","text":"Jun must approve publication. No answer has been recorded."}],"plan":["Ask Jun. If Jun approves, publish. Then publish regardless."]})).unwrap()
}
fn pointer_draft() -> Value {
    json!({"contract":{"conditions":[{"id":"approval","description":"Jun approves publication","observed":"unknown"}],"actions":[{"id":"ask","description":"Ask Jun","requirements":[]},{"id":"publish","description":"publish","requirements":[{"condition":"approval","applicability":"applicable"}]}]},"plan":{"steps":[{"kind":"action","action":"ask"},{"kind":"branch","condition":"approval","when_met":[{"kind":"action","action":"publish"}],"when_unmet":[],"when_unknown":[]},{"kind":"action","action":"publish"}]},"links":[
        {"target":"/contract/conditions/0/description","passages":["e1"]},
        {"target":"/contract/conditions/0/observed","passages":[]},
        {"target":"/contract/actions/0/description","passages":["p1"]},
        {"target":"/contract/actions/1/description","passages":["p1"]},
        {"target":"/contract/actions/1/requirements/0","passages":["e1"]},
        {"target":"/plan/steps/0","passages":["p1"]},
        {"target":"/plan/steps/1","passages":["p1"]},
        {"target":"/plan/steps/1/when_met/0","passages":["p1"]},
        {"target":"/plan/steps/2","passages":["p1"]}
    ],"unresolved":[]})
}
fn draft() -> Value {
    let mut d = pointer_draft();
    let links = d.as_object_mut().unwrap().remove("links").unwrap();
    for link in links.as_array().unwrap() {
        let target = link["target"].as_str().unwrap();
        let (container, field) = if let Some(parent) = target.strip_suffix("/description") {
            (parent, "citations")
        } else if let Some(parent) = target.strip_suffix("/observed") {
            (parent, "observed_citations")
        } else {
            (target, "citations")
        };
        d.pointer_mut(container)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(field.into(), link["passages"].clone());
    }
    d
}
#[test]
fn host_links_original_bytes_and_preserves_the_unsafe_tail() {
    let prepared = prepare(input()).unwrap();
    let report = finish(&prepared, &draft().to_string()).unwrap();
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["coverage"]["status"], "uncovered");
    assert_eq!(
        value["coverage"]["occurrences"].as_array().unwrap().len(),
        3
    );
    let citation = &value["links"][0]["citations"][0];
    assert_eq!(citation["text"], input().evidence[0].text);
    assert_eq!(citation["source_id"], "episode:revision-1");
    assert_eq!(citation["sha256"].as_str().unwrap().len(), 64);
    assert!(!prepared.request().prompt.contains("episode:revision-1"));
    assert!(prepared
        .request()
        .prompt
        .contains("Then publish regardless"));
}
#[test]
fn missing_duplicate_foreign_and_wrong_kind_links_fail() {
    let prepared = prepare(input()).unwrap();
    for change in 0..6 {
        let mut d = draft();
        match change {
            0 => {
                d["plan"]["steps"][2]
                    .as_object_mut()
                    .unwrap()
                    .remove("citations");
            }
            1 => d["contract"]["conditions"][0]["citations"] = json!(["e1", "e1"]),
            2 => d["links"] = json!([]),
            3 => d["contract"]["actions"][1]["requirements"][0]["citations"] = json!(["p1"]),
            4 => d["plan"]["steps"][0]["citations"] = json!(["e1"]),
            _ => d["contract"]["conditions"][0]["citations"] = json!(["e99"]),
        }
        assert!(
            finish(&prepared, &d.to_string()).is_err(),
            "change {change}"
        );
    }
}
#[test]
fn known_observation_requires_evidence_and_unknown_is_not_permission() {
    let prepared = prepare(input()).unwrap();
    let mut d = draft();
    d["contract"]["conditions"][0]["observed"] = json!("met");
    assert!(finish(&prepared, &d.to_string()).is_err());
    d["contract"]["conditions"][0]["observed_citations"] = json!(["e1"]);
    // Membership can pass for a semantically false assertion: explicitly not entailment.
    assert!(finish(&prepared, &d.to_string()).is_ok());
    d["contract"]["conditions"][0]["observed_citations"] = json!(["p1"]);
    assert!(finish(&prepared, &d.to_string()).is_err());
}
#[test]
fn source_receipt_does_not_accept_reply_supplied_evidence_or_unknown_fields() {
    let prepared = prepare(input()).unwrap();
    let mut d = draft();
    d["evidence"] = json!([]);
    assert!(finish(&prepared, &d.to_string()).is_err());
    let mut d = draft();
    d["contract"]["conditions"][0]["quote"] = json!("invented");
    assert!(finish(&prepared, &d.to_string()).is_err());
    let mut d = draft();
    d["plan"]["steps"][0]["action"] = json!("invented");
    assert!(finish(&prepared, &d.to_string()).is_err());
}
#[test]
fn budgets_and_duplicate_passages_fail_closed() {
    let mut i = input();
    i.evidence.push(i.evidence[0].clone());
    assert!(prepare(i).is_err());
    let mut i = input();
    i.evidence[0].text = "é".repeat(1001);
    assert!(prepare(i).is_err());
    let mut i = input();
    i.plan.clear();
    assert!(prepare(i).is_err());
    let prepared = prepare(input()).unwrap();
    assert!(finish(&prepared, &" ".repeat(65537)).is_err());
    let mut d = draft();
    d["contract"]["conditions"][0]["citations"] = json!(["e1", "e1"]);
    assert!(finish(&prepared, &d.to_string()).is_err());
    let mut d = draft();
    d["unresolved"] = json!(["x".repeat(501)]);
    assert!(finish(&prepared, &d.to_string()).is_err());
}

#[test]
fn passage_order_preserves_double_digit_plan_paragraphs() {
    let mut i = input();
    i.plan = (1..=12).map(|n| format!("Paragraph number {n}.")).collect();
    let prompt = prepare(i).unwrap().request().prompt;
    assert!(
        prompt.find("Paragraph number 2.").unwrap() < prompt.find("Paragraph number 10.").unwrap()
    );
}

#[test]
fn duplicate_json_members_cannot_erase_plan_or_provenance() {
    let prepared = prepare(input()).unwrap();
    let raw = draft().to_string();
    for (field, first) in [
        ("plan", r#"{"steps":[]}"#),
        ("requirements", "[]"),
        ("kind", r#""branch""#),
        ("citations", "[]"),
    ] {
        let key = format!("\"{field}\":");
        let duplicate = raw.replacen(&key, &format!("{key}{first},{key}"), 1);
        assert!(finish(&prepared, &duplicate).is_err(), "duplicate {field}");
    }
}
#[test]
fn depth_limit_counts_nodes_not_empty_branch_arms() {
    let prepared = prepare(input()).unwrap();
    let mut node = json!({"kind":"branch","condition":"approval","when_met":[],"when_unmet":[],"when_unknown":[],"citations":["p1"]});
    for _ in 0..8 {
        node = json!({"kind":"branch","condition":"approval","when_met":[node],"when_unmet":[],"when_unknown":[],"citations":["p1"]});
    }
    let mut d = draft();
    d["plan"]["steps"] = json!([node.clone()]);
    assert!(finish(&prepared, &d.to_string()).is_ok());
    d["plan"]["steps"] = json!([{"kind":"branch","condition":"approval","when_met":[node],"when_unmet":[],"when_unknown":[],"citations":["p1"]}]);
    assert!(finish(&prepared, &d.to_string()).is_err());
}

#[test]
fn direct_judgment_binds_sources_but_does_not_certify_semantics() {
    let p = prepare(input()).unwrap();
    let req = p.direct_request();
    assert!(!req.prompt.contains("episode:revision-1"));
    assert_eq!(
        req.prompt.split("PASSAGES:\n").last(),
        p.request().prompt.split("PASSAGES:\n").last()
    );
    let report = finish_direct(&p, &json!({"status":"covered","reason":"An intentionally wrong judgment.","evidence_citations":["e1"],"plan_citations":["p1"]}).to_string()).unwrap();
    let v = serde_json::to_value(report).unwrap();
    assert_eq!(v["protocol"], "direct-coverage-v1");
    assert_eq!(v["evidence"][0]["source_id"], "episode:revision-1");
    assert_eq!(v["plan"][0]["text"], input().plan[0]);
    assert_eq!(v["status"], "covered"); // Host does not grade model semantics.
}
#[test]
fn direct_judgment_rejects_invalid_sources_and_unbounded_fields() {
    let p = prepare(input()).unwrap();
    let original = json!({"status":"uncovered","reason":"Unconditional tail.","evidence_citations":["e1"],"plan_citations":["p1"]});
    for (key, value) in [
        ("status", json!("approved")),
        ("reason", json!("")),
        ("reason", json!("a".repeat(2001))),
        ("evidence_citations", json!([])),
        ("evidence_citations", json!(["p1"])),
        ("plan_citations", json!(["e1"])),
        ("plan_citations", json!(["p99"])),
        ("plan_citations", json!(["p1", "p1"])),
        ("authority", json!(true)),
    ] {
        let mut v = original.clone();
        v[key] = value;
        assert!(finish_direct(&p, &v.to_string()).is_err(), "{key}: {v}");
    }
    assert!(finish_direct(&p, r#"{"status":"covered","status":"uncovered","reason":"x","evidence_citations":["e1"],"plan_citations":["p1"]}"#).is_err());
    assert!(finish_direct(&p, &" ".repeat(65537)).is_err());
}
