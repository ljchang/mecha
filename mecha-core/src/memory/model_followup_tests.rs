use super::*;
use std::{
    pin::pin,
    task::{Context as TaskContext, Poll, Waker},
};
fn ready<T>(f: impl Future<Output = T>) -> T {
    match pin!(f).poll(&mut TaskContext::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("test must be ready"),
    }
}
fn reply(text: &str) -> ModelReply {
    ModelReply {
        text: text.into(),
        model: "test".into(),
        input_tokens: Some(10),
        output_tokens: Some(5),
        error: None,
    }
}

#[test]
fn citation_schema_lists_visible_ids_only_and_empty_evidence_forces_empty_citations() {
    let d = load(Split::Development).unwrap();
    let c = &d.cases[8];
    let f = super::super::super::transcript_tests::Fixture::new();
    let (memory, refs) = prepare(&f.dir.join("schema.memory-prototype.sqlite"), &d).unwrap();
    let ev = oracle(&memory, c, &refs).unwrap();
    let req = answer_request(c, &ev).unwrap();
    assert_eq!(
        req.response_schema["properties"]["citations"]["items"]["enum"],
        json!(["saffron:2"])
    );
    assert_eq!(
        req.response_schema["properties"]["choice"]["enum"]
            .as_array()
            .unwrap()
            .len(),
        c.choices.len()
    );
    assert!(req.prompt.contains("allowed_citation_ids"));
    assert!(!req.prompt.contains("episode:line"));
    let empty = answer_request(c, &[]).unwrap();
    assert_eq!(
        empty.response_schema["properties"]["citations"]["maxItems"],
        0
    );
    assert!(
        !grade(
            c,
            r#"{"choice":"r7","citations":["episode:2"],"abstain":false}"#,
            &["saffron:2".into()]
        )
        .unwrap()
        .grounded
    );
}

#[test]
fn planner_receives_only_authorized_initial_evidence_never_answer_choices_or_gold() {
    let d = load(Split::Development).unwrap();
    let c = &d.cases[0];
    let f = super::super::super::transcript_tests::Fixture::new();
    let (memory, _) = prepare(&f.dir.join("planner.memory-prototype.sqlite"), &d).unwrap();
    let ev = retrieve_anchors(
        &memory,
        c,
        std::slice::from_ref(&c.question),
        &memory.snapshot().unwrap(),
    )
    .unwrap();
    for shown in [&[][..], ev.as_slice()] {
        let req = search_request(c, shown).unwrap();
        let data: serde_json::Value =
            serde_json::from_str(req.prompt.rsplit_once('\n').unwrap().1).unwrap();
        assert_eq!(
            data["initial_evidence"].as_array().unwrap().len(),
            shown.len()
        );
        assert!(data.get("choices").is_none());
        assert!(data.get("expected_choice").is_none());
        assert!(data.get("expected_lines").is_none());
        assert_eq!(req.response_schema["properties"]["queries"]["maxItems"], 2);
    }
}

#[test]
fn matched_search_calls_keep_shared_filters_budgets_and_initial_evidence() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let mut n = 0;
    let r = ready(run(
        &f.dir.join("hop.memory-prototype.sqlite"),
        "test",
        Split::Holdout,
        |req| {
            n += 1;
            std::future::ready(Ok(reply(if req.prompt.starts_with("SEARCH_PLANNER") {
                r#"{"queries":["available","counter"]}"#
            } else {
                r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#
            })))
        },
        |_| {},
    ))
    .unwrap();
    assert_eq!(r.rows.len(), 36);
    assert_eq!(n, 60);
    for chunk in r.rows.chunks(3) {
        let control = chunk.iter().find(|r| r.row.arm == Arm::GoalAware).unwrap();
        let treatment = chunk
            .iter()
            .find(|r| r.row.arm == Arm::EvidenceAware)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&control.initial_evidence).unwrap(),
            serde_json::to_value(&treatment.initial_evidence).unwrap()
        );
    }
    for r in &r.rows {
        assert!(r.row.error.is_none(), "{:?}", r.row.error);
        assert!(serde_json::to_vec(&r.initial_evidence).unwrap().len() <= EVIDENCE_BUDGET);
        assert!(r.row.evidence_bytes <= EVIDENCE_BUDGET && r.row.evidence.len() <= MAX_EVIDENCE);
        assert_eq!(
            r.row.calls.len(),
            if r.row.arm == Arm::Oracle { 1 } else { 2 }
        );
        assert!(r.row.calls.iter().all(|c| c.response_schema.is_some()));
        for e in r.initial_evidence.iter().chain(&r.row.evidence) {
            let Record::Evidence(c) = &e.hit.record else {
                panic!()
            };
            assert_eq!(c.source.account, "synthetic-hop");
            assert!(e.hit.private_data && e.hit.untrusted_input);
            if r.row.case_id == "basalt-early" {
                assert!(c.occurred_at.as_deref().unwrap() < "2026-02-02T00:00:00Z");
            }
        }
    }
}

#[test]
fn fresh_fixture_has_distinct_sessions_feasible_choices_and_resolvable_oracles() {
    let dev = load(Split::Development).unwrap();
    let held = load(Split::Holdout).unwrap();
    let ids: Vec<_> = dev.cases.iter().map(|c| &c.session_id).collect();
    assert_eq!(held.cases.len(), 12);
    assert!(held
        .cases
        .iter()
        .all(|c| !ids.contains(&&c.session_id) && c.choices.contains_key(&c.expected_choice)));
    for pair in held.cases[..6].chunks(2) {
        assert_eq!(pair[0].question, pair[1].question);
        assert_eq!(pair[0].session_id, pair[1].session_id);
        assert_ne!(pair[0].expected_choice, pair[1].expected_choice);
        assert!(
            pair[0].goal != pair[1].goal
                || pair[0].homeostatic_context != pair[1].homeostatic_context
        );
    }
    let f = super::super::super::transcript_tests::Fixture::new();
    let (memory, refs) = prepare(&f.dir.join("oracle.memory-prototype.sqlite"), &held).unwrap();
    for c in &held.cases {
        assert_eq!(
            oracle(&memory, c, &refs).unwrap().len(),
            c.expected_lines.len()
        );
    }
}

#[test]
fn query_anchors_outrank_nearby_filler() {
    let d = load(Split::Development).unwrap();
    let c = &d.cases[0];
    let f = super::super::super::transcript_tests::Fixture::new();
    let (memory, _) = prepare(&f.dir.join("anchors.memory-prototype.sqlite"), &d).unwrap();
    let got = retrieve_anchors(
        &memory,
        c,
        &[
            c.question.clone(),
            "Kestrel delivery hours cost".into(),
            "Morrow freight days cost".into(),
        ],
        &memory.snapshot().unwrap(),
    )
    .unwrap();
    let ids: Vec<_> = got.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"juniper:4") && ids.contains(&"juniper:6"),
        "{ids:?}"
    );
}

#[test]
fn followup_never_uses_other_accounts_or_currently_restricted_evidence() {
    let d = load(Split::Development).unwrap();
    let c = &d.cases[0];
    let f = super::super::super::transcript_tests::Fixture::new();
    let (mut memory, _) = prepare(&f.dir.join("scope.memory-prototype.sqlite"), &d).unwrap();
    let queries = vec!["Kestrel delivery".into(), "Morrow freight".into()];
    let before = memory.snapshot().unwrap();
    // Same session/text in another account must not affect the selected records.
    memory
        .import_session_jsonl(
            d.sessions[0].jsonl.as_bytes(),
            "foreign",
            fixture::access().scopes,
        )
        .unwrap();
    let hits = retrieve_anchors(&memory, c, &queries, &memory.snapshot().unwrap()).unwrap();
    assert!(!hits.is_empty());
    assert!(hits
        .iter()
        .all(|e| matches!(&e.hit.record,Record::Evidence(x) if x.source.account==c.account)));
    // A scope tightening also applies to the old knowledge snapshot.
    memory
        .import_session_jsonl(
            d.sessions[0].jsonl.as_bytes(),
            &c.account,
            BTreeSet::from(["restricted".into()]),
        )
        .unwrap();
    assert!(retrieve_anchors(&memory, c, &queries, &before)
        .unwrap()
        .is_empty());
}

#[test]
fn duplicate_valid_citations_cannot_pass_host_grading() {
    let d = load(Split::Development).unwrap();
    let c = &d.cases[8];
    let score = grade(
        c,
        r#"{"choice":"r7","citations":["saffron:2","saffron:2"],"abstain":false}"#,
        &["saffron:2".into()],
    )
    .unwrap();
    assert!(score.choice_correct && score.required_support_cited);
    assert!(!score.citations_valid && !score.grounded);
}

#[test]
fn mention_replay_recovers_alternative_facts_under_same_budgets() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let r = mention_evaluation::run(&f.dir.join("replay.memory-prototype.sqlite")).unwrap();
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    for pair in rows.chunks(2) {
        assert_eq!(pair[0]["initial_ids"], pair[1]["initial_ids"]);
        assert_eq!(pair[0]["search_count"], pair[1]["search_count"]);
        for row in pair {
            assert!(row["evidence_bytes"].as_u64().unwrap() <= 8000);
            assert!(row["evidence_ids"].as_array().unwrap().len() <= 4);
            assert!(row["search_count"].as_u64().unwrap() <= 3);
        }
        if pair[0]["case_id"] == "lumen-quiet" || pair[0]["case_id"] == "lyra-local" {
            assert_eq!(pair[0]["coverage"], 0.0);
            assert_eq!(pair[1]["coverage"], 1.0);
        }
        if pair[0]["category"] != "changed_goal" {
            assert_eq!(pair[0]["evidence_ids"], pair[1]["evidence_ids"]);
        }
    }
}

#[test]
fn automatic_bindings_share_one_plan_and_preserve_oracle_on_invalid_binding() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let mut calls = 0;
    let r = ready(binding_evaluation::run(&f.dir.join("binding.memory-prototype.sqlite"), "test", Split::Holdout, |req| {
        calls += 1;
        std::future::ready(Ok(reply(if req.prompt.starts_with("BINDING_PLANNER") {
            r#"{"searches":[{"query":"Invented capacity","mention":"Invented","source_id":"missing:2"}]}"#
        } else { r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"# })))
    }, |_| {})).unwrap();
    assert_eq!(calls, 28); //14 planner +14 oracle; invalid plan never broadens search.
    for c in r["cases"].as_array().unwrap() {
        assert!(c["plan_error"].is_string());
        assert_eq!(
            c.get("grounded_binding_count"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(c["planner_calls"].as_array().unwrap().len(), 1);
        for row in c["rows"].as_array().unwrap() {
            assert_eq!(row["error"].is_null(), row["arm"] == "oracle");
        }
    }
}

#[test]
fn automatic_bindings_count_shared_planner_once_and_keep_budget_time_and_taint() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let mut calls = 0;
    let r = ready(binding_evaluation::run(
        &f.dir.join("valid.memory-prototype.sqlite"),
        "test",
        Split::Holdout,
        |req| {
            calls += 1;
            std::future::ready(Ok(reply(if req.prompt.starts_with("BINDING_PLANNER") {
                r#"{"searches":[]}"#
            } else {
                r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#
            })))
        },
        |_| {},
    ))
    .unwrap();
    assert_eq!(calls, 56);
    for c in r["cases"].as_array().unwrap() {
        assert!(c["plan_error"].is_null());
        assert_eq!(c["grounded_binding_count"], 0);
        assert_eq!(c["planner_calls"].as_array().unwrap().len(), 1);
        let rows = c["rows"].as_array().unwrap();
        let a = rows.iter().find(|x| x["arm"] == "unbound").unwrap();
        let b = rows.iter().find(|x| x["arm"] == "bound").unwrap();
        assert_eq!(a["queries"], b["queries"]);
        assert_eq!(a["evidence"], b["evidence"]);
        for row in rows {
            assert_eq!(row["calls"].as_array().unwrap().len(), 1);
            assert!(row["evidence_bytes"].as_u64().unwrap() <= 8000);
            assert!(row["evidence"].as_array().unwrap().len() <= 4);
            for e in row["evidence"].as_array().unwrap() {
                assert_eq!(e["hit"]["private_data"], true);
                assert_eq!(e["hit"]["untrusted_input"], true);
                assert_eq!(
                    e["hit"]["record"]["data"]["source"]["account"],
                    "synthetic-bindings"
                );
                if c["case_id"] == "ochre-early" {
                    assert!(
                        e["hit"]["record"]["data"]["occurred_at"].as_str().unwrap()
                            < "2026-03-02T00:00:00Z"
                    );
                }
            }
        }
    }
}

#[test]
fn automatic_non_null_binding_changes_the_executed_retrieval_arm() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let report = ready(binding_evaluation::run(
        &f.dir.join("non-null.memory-prototype.sqlite"), "test", Split::Holdout,
        |req| {
            let text = if req.prompt.starts_with("BINDING_PLANNER") {
                let data: serde_json::Value = serde_json::from_str(req.prompt.rsplit_once('\n').unwrap().1).unwrap();
                if data["question"].as_str().unwrap().contains("Palisade") {
                    r#"{"searches":[{"query":"São Vale Palisade needs transport selection","mention":"São Vale","source_id":"palisade:2"}]}"#
                } else { r#"{"searches":[]}"# }
            } else { r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"# };
            std::future::ready(Ok(reply(text)))
        }, |_| {},
    )).unwrap();
    let case = &report["cases"][0];
    assert!(case["plan_error"].is_null(), "{case}");
    assert_eq!(case["plan"]["searches"][0]["mention"], "São Vale");
    assert_eq!(case["grounded_binding_count"], 1);
    let rows = case["rows"].as_array().unwrap();
    let unbound = rows.iter().find(|r| r["arm"] == "unbound").unwrap();
    let bound = rows.iter().find(|r| r["arm"] == "bound").unwrap();
    let has_fact = |r: &serde_json::Value| {
        r["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "palisade:4")
    };
    assert!(
        !has_fact(unbound),
        "unbound must miss the named fact for this regression to be discriminating"
    );
    assert!(
        has_fact(bound),
        "valid non-null binding must route to required-mention retrieval"
    );
    assert_eq!(unbound["queries"], bound["queries"]);
}

#[test]
fn automatic_binding_count_excludes_explicitly_unbound_searches() {
    let f = super::super::super::transcript_tests::Fixture::new();
    let report = ready(binding_evaluation::run(
        &f.dir.join("unbound-count.memory-prototype.sqlite"),
        "test",
        Split::Holdout,
        |req| {
            std::future::ready(Ok(reply(if req.prompt.starts_with("BINDING_PLANNER") {
                r#"{"searches":[{"query":"capacity","mention":null,"source_id":null}]}"#
            } else {
                r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#
            })))
        },
        |_| {},
    ))
    .unwrap();
    for case in report["cases"].as_array().unwrap() {
        assert!(case["plan_error"].is_null());
        assert_eq!(case["plan"]["searches"].as_array().unwrap().len(), 1);
        assert_eq!(case["grounded_binding_count"], 0);
    }
}
