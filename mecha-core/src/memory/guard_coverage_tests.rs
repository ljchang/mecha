use super::guard_coverage::*;
use serde_json::{json, Value};

fn contract() -> Contract {
    serde_json::from_value(json!({
        "conditions":[{"id":"approval","description":"Jun approved publication","observed":"unknown"},
                      {"id":"timeout","description":"A day has passed","observed":"met"}],
        "actions":[{"id":"ask","description":"Ask Jun for approval","requirements":[]},
                   {"id":"publish","description":"Publish the report","requirements":[
                     {"condition":"approval","applicability":"applicable"}]}]
    })).unwrap()
}
fn action(id: &str) -> Value {
    json!({"kind":"action","action":id})
}
fn branch(condition: &str, met: Vec<Value>, unmet: Vec<Value>, unknown: Vec<Value>) -> Value {
    json!({"kind":"branch","condition":condition,"when_met":met,"when_unmet":unmet,"when_unknown":unknown})
}
fn plan(nodes: Vec<Value>) -> Plan {
    serde_json::from_value(json!({"steps":nodes})).unwrap()
}
fn report(nodes: Vec<Value>) -> CoverageReport {
    check(&contract(), &plan(nodes)).unwrap()
}

#[test]
fn guard_covers_only_its_branch_not_a_later_unconditional_action() {
    let r = report(vec![
        action("ask"),
        branch("approval", vec![action("publish")], vec![], vec![]),
        action("publish"),
    ]);
    assert_eq!(r.status, Coverage::Uncovered);
    assert_eq!(
        r.occurrences.iter().map(|x| x.status).collect::<Vec<_>>(),
        vec![Coverage::Covered, Coverage::Covered, Coverage::Uncovered]
    );
    assert_eq!(r.occurrences[2].issues[0].condition, "approval");
    assert!(r.occurrences[2].path.is_empty());
}

#[test]
fn false_unknown_and_timeout_branches_cannot_supply_approval() {
    for node in [
        branch("approval", vec![], vec![action("publish")], vec![]),
        branch("approval", vec![], vec![], vec![action("publish")]),
        branch("timeout", vec![action("publish")], vec![], vec![]),
    ] {
        assert_eq!(report(vec![node]).status, Coverage::Uncovered);
    }
}

#[test]
fn valid_request_then_guarded_publication_is_covered_without_guaranteeing_success() {
    let r = report(vec![
        action("ask"),
        branch("approval", vec![action("publish")], vec![], vec![]),
    ]);
    assert_eq!(r.status, Coverage::Covered);
    assert!(r.occurrences[0].matches_observation);
    assert!(!r.occurrences[1].matches_observation);
}

#[test]
fn observed_met_is_not_a_guard_for_a_future_unconditional_action() {
    let mut c = contract();
    c.conditions[0].observed = State::Met;
    assert_eq!(
        check(&c, &plan(vec![action("publish")])).unwrap().status,
        Coverage::Uncovered
    );
    let r = check(
        &c,
        &plan(vec![branch(
            "approval",
            vec![],
            vec![action("publish")],
            vec![],
        )]),
    )
    .unwrap();
    assert_eq!(r.status, Coverage::Uncovered);
    assert!(!r.occurrences[0].matches_observation);
}

#[test]
fn all_applicable_conditions_must_guard_the_action() {
    let mut c = contract();
    c.actions[1].requirements.push(Requirement {
        condition: "timeout".into(),
        applicability: Applicability::Applicable,
    });
    let single = plan(vec![branch(
        "approval",
        vec![action("publish")],
        vec![],
        vec![],
    )]);
    assert_eq!(check(&c, &single).unwrap().status, Coverage::Uncovered);
    let both = plan(vec![branch(
        "approval",
        vec![branch("timeout", vec![action("publish")], vec![], vec![])],
        vec![],
        vec![],
    )]);
    assert_eq!(check(&c, &both).unwrap().status, Coverage::Covered);
}

#[test]
fn applicability_is_separate_from_condition_satisfaction() {
    let mut c = contract();
    c.actions[1].requirements[0].applicability = Applicability::Unknown;
    let p = plan(vec![branch(
        "approval",
        vec![action("publish")],
        vec![],
        vec![],
    )]);
    assert_eq!(check(&c, &p).unwrap().status, Coverage::Unresolved);
    c.actions[1].requirements[0].applicability = Applicability::NotApplicable;
    assert_eq!(
        check(&c, &plan(vec![action("publish")])).unwrap().status,
        Coverage::Covered
    );
}

#[test]
fn contradictory_nested_arms_and_empty_plans_are_not_successes() {
    let r = report(vec![branch(
        "approval",
        vec![branch("approval", vec![], vec![action("publish")], vec![])],
        vec![],
        vec![],
    )]);
    assert_eq!(r.status, Coverage::NoReachableActions);
    assert_eq!(r.occurrences[0].status, Coverage::Unreachable);
    assert!(!r.occurrences[0].matches_observation);
    assert_eq!(report(vec![]).status, Coverage::NoReachableActions);
}

#[test]
fn malformed_references_and_duplicate_contract_bindings_fail_even_in_dead_arms() {
    let p = plan(vec![branch(
        "approval",
        vec![branch("approval", vec![], vec![action("foreign")], vec![])],
        vec![],
        vec![],
    )]);
    assert!(check(&contract(), &p).is_err());
    let mut c = contract();
    let duplicate = c.actions[1].requirements[0].clone();
    c.actions[1].requirements.push(duplicate);
    assert!(check(&c, &plan(vec![])).is_err());
    let mut c = contract();
    c.conditions.push(c.conditions[0].clone());
    assert!(check(&c, &plan(vec![])).is_err());
    let mut c = contract();
    c.actions.push(c.actions[0].clone());
    assert!(check(&c, &plan(vec![])).is_err());
    assert!(check(
        &contract(),
        &plan(vec![branch("foreign", vec![], vec![], vec![])])
    )
    .is_err());
    let mut c = contract();
    c.actions[1].requirements[0].condition = "foreign".into();
    assert!(check(&c, &plan(vec![])).is_err());
}

#[test]
fn rejects_oversized_trees_and_unknown_input_fields() {
    assert!(check(&contract(), &plan(vec![action("ask"); 129])).is_err());
    let mut node = action("ask");
    for _ in 0..9 {
        node = branch("approval", vec![node], vec![], vec![]);
    }
    assert!(check(&contract(), &plan(vec![node])).is_err());
    assert!(serde_json::from_value::<Plan>(
        json!({"steps":[{"kind":"action","action":"publish","requirements":[]}]})
    )
    .is_err());
    let mut c = contract();
    c.conditions[0].id = "x".repeat(65);
    assert!(check(&c, &plan(vec![])).is_err());
    assert!(check_json(&" ".repeat(65_537)).is_err());
}
