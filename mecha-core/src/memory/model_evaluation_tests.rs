use super::{model_evaluation::*, *};
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<T>(f: impl Future<Output = T>) -> T {
    match pin!(f).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(x) => x,
        Poll::Pending => panic!("test callback must be ready"),
    }
}

#[test]
fn query_plans_are_bounded_and_cannot_select_scopes() {
    assert_eq!(
        parse_queries(r#"{"queries":["delivery speed", "freight cost"]}"#)
            .unwrap()
            .len(),
        2
    );
    for bad in [
        r#"{"queries":["a","b","c"]}"#,
        r#"{"queries":[""]}"#,
        r#"{"queries":["a"],"account":"secret"}"#,
        "not json",
    ] {
        assert!(parse_queries(bad).is_err(), "{bad}");
    }
    assert!(parse_queries(&format!(r#"{{"queries":["{}"]}}"#, "x ".repeat(40))).is_err());
}

#[test]
fn grounding_rejects_guesses_wrong_citations_and_fake_abstention() {
    let d = dataset().unwrap();
    let c = &d.cases[8];
    let available = vec!["saffron:2".to_string()];
    assert!(
        grade(
            c,
            r#"{"choice":"r7","citations":["saffron:2"],"abstain":false}"#,
            &available
        )
        .unwrap()
        .grounded
    );
    for bad in [
        r#"{"choice":"r7","citations":[],"abstain":false}"#,
        r#"{"choice":"r7","citations":["saffron:3"],"abstain":false}"#,
        r#"{"choice":"r8","citations":["saffron:2"],"abstain":false}"#,
    ] {
        assert!(!grade(c, bad, &available).unwrap().grounded);
    }
    assert!(grade(c, "nonsense", &available).is_err());
    let absent = &d.cases[14];
    assert!(
        grade(
            absent,
            r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#,
            &[]
        )
        .unwrap()
        .grounded
    );
    assert!(
        !grade(
            absent,
            r#"{"choice":"insufficient_evidence","citations":[],"abstain":false}"#,
            &[]
        )
        .unwrap()
        .grounded
    );
}

#[test]
fn heldout_pairs_share_question_and_evidence_and_planner_never_sees_gold() {
    let d = dataset().unwrap();
    assert_eq!(d.cases.len(), 16);
    for pair in d.cases[..8].chunks(2) {
        assert_eq!(pair[0].session_id, pair[1].session_id);
        assert_eq!(pair[0].question, pair[1].question);
        assert_ne!(pair[0].expected_choice, pair[1].expected_choice);
        assert_ne!(pair[0].goal, pair[1].goal);
    }
    for c in &d.cases {
        let prompt = planner_prompt(c).unwrap();
        for forbidden in ["expected_choice", "expected_lines", "choices", "citations"] {
            assert!(!prompt.contains(forbidden));
        }
    }
}

#[test]
fn three_arm_run_preserves_limits_errors_and_oracle_separation() {
    let f = transcript_tests::Fixture::new();
    let path = f.dir.join("model.memory-prototype.sqlite");
    let mut count = 0;
    let report = ready(run(
        &path,
        "test-model",
        |prompt| {
            count += 1;
            let text = if prompt.contains("SEARCH_PLANNER") {
                r#"{"queries":["available", "correction"]}"#
            } else {
                r#"{"choice":"insufficient_evidence","citations":[],"abstain":true}"#
            };
            std::future::ready(Ok(ModelReply {
                text: text.into(),
                model: "test-model".into(),
                input_tokens: Some(10),
                output_tokens: Some(5),
                error: None,
            }))
        },
        |_| {},
    ))
    .unwrap();
    assert_eq!(report.rows.len(), 48);
    assert_eq!(count, 64);
    for r in &report.rows {
        assert!(r.evidence_bytes <= EVIDENCE_BUDGET);
        assert!(r.evidence.len() <= MAX_EVIDENCE);
        assert!(r.queries.len() <= 3);
        assert!(r.error.is_none(), "{:?}", r.error);
        for h in &r.evidence {
            assert!(h.hit.private_data && h.hit.untrusted_input);
            let Record::Evidence(capture) = &h.hit.record else {
                panic!()
            };
            assert_eq!(capture.source.account, "synthetic-lab");
            if r.case_id == "opal-early" {
                assert!(capture.occurred_at.as_deref().unwrap() < "2026-01-02T00:00:00Z");
            }
        }
        if r.arm == Arm::Oracle && !r.case_id.starts_with("umber") {
            assert_eq!(r.coverage, Some(1.0));
        }
    }
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|r| r.score.as_ref().is_some_and(|s| s.grounded))
            .count(),
        6
    );
    assert!(ready(run(
        &path,
        "test-model",
        |_| std::future::ready(Err(anyhow::anyhow!("must not run"))),
        |_| {}
    ))
    .is_err());
}

#[test]
fn provider_failures_and_wrong_models_are_counted_not_dropped() {
    let f = transcript_tests::Fixture::new();
    let report = ready(run(
        &f.dir.join("fail.memory-prototype.sqlite"),
        "expected",
        |_| std::future::ready(Err(anyhow::anyhow!("offline"))),
        |_| {},
    ))
    .unwrap();
    assert_eq!(report.rows.len(), 48);
    assert!(report
        .rows
        .iter()
        .all(|r| r.error.is_some() && r.score.is_none()));
    let report = ready(run(
        &f.dir.join("wrong.memory-prototype.sqlite"),
        "expected",
        |_| {
            std::future::ready(Ok(ModelReply {
                text: "{}".into(),
                model: "different".into(),
                input_tokens: Some(9),
                output_tokens: Some(3),
                error: None,
            }))
        },
        |_| {},
    ))
    .unwrap();
    assert!(report
        .rows
        .iter()
        .all(|r| r.error.as_deref().unwrap().contains("model mismatch")));
    assert!(report
        .rows
        .iter()
        .all(|r| r.calls[0].reply.as_ref().unwrap().input_tokens == Some(9)));
}

#[test]
fn heldout_gold_respects_battery_reserve_and_includes_repair_costs() {
    let d: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/model_comparison.json")).unwrap();
    let cases = d["cases"].as_array().unwrap();
    let topaz = cases.iter().find(|c| c["id"] == "topaz-neri").unwrap();
    let number = |text: &str| {
        text.split(|c: char| !c.is_ascii_digit())
            .find(|x| !x.is_empty())
            .unwrap()
            .parse::<u32>()
            .unwrap()
    };
    let battery = number(topaz["homeostatic_context"].as_str().unwrap());
    let reserve = number(topaz["goal"].as_str().unwrap());
    let source = d["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["jsonl"].as_str().unwrap().contains("Neri uses"))
        .unwrap()["jsonl"]
        .as_str()
        .unwrap();
    let source: serde_json::Value =
        serde_json::from_str(source.lines().find(|s| s.contains("Neri uses")).unwrap()).unwrap();
    let required = number(source["content"][0]["text"].as_str().unwrap());
    assert!(
        battery >= required + reserve,
        "gold action cannot preserve the required battery reserve"
    );
    for c in cases.iter().filter(|c| c["session_id"] == "marlin") {
        assert!(
            c["expected_lines"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(7)),
            "oracle must receive the cost/compatibility premise"
        );
    }
}

#[test]
fn a_correct_answer_with_an_irrelevant_citation_is_not_grounded() {
    let d = dataset().unwrap();
    assert!(
        !grade(
            &d.cases[8],
            r#"{"choice":"r7","citations":["saffron:2","saffron:3"],"abstain":false}"#,
            &["saffron:2".into(), "saffron:3".into()]
        )
        .unwrap()
        .grounded
    );
}
