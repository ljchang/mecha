use super::{
    concept::*,
    transcript_tests::{access, log, Fixture},
    *,
};
use serde_json::json;

fn sources(f: &mut Fixture) -> Vec<RevisionRef> {
    [
        "When a permit is pending, reserve an alternate route.",
        "A missing inspection blocked departure; keeping a fallback route helped.",
        "An emergency waiver allowed departure without the usual inspection.",
    ]
    .iter()
    .enumerate()
    .map(|(i, text)| {
        f.db.import_session_jsonl(
            log(&format!("episode-{i}"), &[text]).as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap()
        .records[1]
            .clone()
    })
    .collect()
}
fn draft(refs: &[RevisionRef]) -> String {
    json!({"name":"Approval dependency", "principle":"A plan depending on an unfinished approval benefits from a feasible alternative.", "applicability":"Plans with an external approval prerequisite; emergency waivers can be exceptions.",
        "support":[{"source_id":refs[0].revision,"field":"/content/0/text","quote":"permit is pending"},{"source_id":refs[1].revision,"field":"/content/0/text","quote":"missing inspection blocked departure"}],
        "counterexamples":[{"source_id":refs[2].revision,"field":"/content/0/text","quote":"emergency waiver allowed departure"}]}).to_string()
}
#[test]
fn concept_generation_is_grounded_provisional_and_inherits_all_consulted_sources() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let request =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    assert!(request.generation().prompt.contains("untrusted"));
    assert_eq!(
        request.generation().response_schema["properties"]["support"]["items"]["properties"]
            ["field"]["enum"],
        json!(["/content/0/text"])
    );
    let r =
        f.db.retain_concept(request, &draft(&refs), &access(), None)
            .unwrap();
    let Some(Record::Concept(c)) = f.db.read(&r, &access()).unwrap() else {
        panic!()
    };
    assert!(matches!(c.status, ConceptStatus::Provisional));
    assert_eq!(c.consulted.len(), 3);
    assert_eq!(c.support.len(), 2);
    assert_eq!(c.counterexamples.len(), 1);
    let hits =
        f.db.search(
            &Query {
                text: "Approval".into(),
                known_at: f.db.snapshot().unwrap(),
                valid_at: None,
                include_possible: false,
                limit: 10,
            },
            &access(),
        )
        .unwrap();
    assert!(hits
        .iter()
        .any(|h| h.reference == r && h.private_data && h.untrusted_input));
    assert_eq!(
        f.db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    Memory::open_read_only(&f.dir.join("test.memory-prototype.sqlite")).unwrap();
    // Even an uncited consulted source contributes permissions and forgetting.
    let mut without_counter = serde_json::from_str::<serde_json::Value>(&draft(&refs)).unwrap();
    without_counter["counterexamples"] = json!([]);
    let request =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    let revision =
        f.db.retain_concept(request, &without_counter.to_string(), &access(), Some(&r))
            .unwrap();
    f.db.conn
        .execute(
            "INSERT INTO scope VALUES (?1, 'secret')",
            [&refs[2].revision],
        )
        .unwrap();
    assert!(f.db.read(&revision, &access()).unwrap().is_none());
    f.db.forget(&transcript_source("a", "episode-2", 2))
        .unwrap();
    assert!(f.db.read(&r, &access()).unwrap().is_none());
    assert!(f.db.read(&revision, &access()).unwrap().is_none());
}
#[test]
fn concept_rejects_invented_quotes_same_episode_support_and_stale_generation() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let mut bad: serde_json::Value = serde_json::from_str(&draft(&refs)).unwrap();
    bad["support"][0]["quote"] = json!("permit approved");
    let request =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    assert!(f
        .db
        .retain_concept(request, &bad.to_string(), &access(), None)
        .is_err());
    assert_eq!(
        f.db.conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let same =
        f.db.import_session_jsonl(
            log(
                "one",
                &["permit is pending", "missing inspection blocked departure"],
            )
            .as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap()
        .records;
    let same_refs = vec![same[1].clone(), same[2].clone(), refs[2].clone()];
    let request =
        f.db.prepare_concept(&same_refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    assert!(f
        .db
        .retain_concept(request, &draft(&same_refs), &access(), None)
        .is_err());
    let request =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    f.db.conn
        .execute(
            "INSERT INTO scope VALUES (?1,'secret')",
            [&refs[2].revision],
        )
        .unwrap();
    assert!(f
        .db
        .retain_concept(request, &draft(&refs), &access(), None)
        .is_err());
}
#[test]
fn concept_retention_rechecks_source_revision_after_generation() {
    let mut f = Fixture::new();
    let mk = |id: &str, text: &str| Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: "a".into(),
            id: id.into(),
        },
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    };
    let a = f.db.capture(mk("a", "permit is pending")).unwrap();
    let b =
        f.db.capture(mk("b", "missing inspection blocked departure"))
            .unwrap();
    let refs = vec![a, b];
    let request =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    let text=json!({"name":"Approval dependency","principle":"Pending prerequisites constrain plans.","applicability":"Pending approval cases","support":[{"source_id":refs[0].revision,"field":"","quote":"permit is pending"},{"source_id":refs[1].revision,"field":"","quote":"missing inspection blocked departure"}],"counterexamples":[]}).to_string();
    f.db.capture(mk("a", "permit granted")).unwrap();
    assert!(f
        .db
        .retain_concept(request, &text, &access(), None)
        .is_err());
}

#[test]
fn concept_preparation_refuses_a_single_episode_before_calling_a_model() {
    let mut f = Fixture::new();
    let records =
        f.db.import_session_jsonl(
            log("one-episode", &["permit pending", "inspection pending"]).as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap()
        .records;
    assert!(f
        .db
        .prepare_concept(&records[1..], &f.db.snapshot().unwrap(), &access())
        .is_err());
}

#[test]
fn concept_learning_calls_generation_once_without_granting_authority() {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let snapshot = f.db.snapshot().unwrap();
    let access = access();
    let reply = draft(&refs);
    let mut calls = 0;
    let result = {
        let future =
            f.db.learn_concept(&refs, &snapshot, &access, None, |generation| {
                calls += 1;
                assert!(
                    generation.response_schema["properties"]["support"]["items"]["properties"]
                        ["source_id"]["enum"]
                        .as_array()
                        .unwrap()
                        .len()
                        == 3
                );
                std::future::ready(Ok(reply))
            });
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result.unwrap(),
            Poll::Pending => panic!("scripted provider must be ready"),
        }
    };
    assert_eq!(calls, 1);
    assert!(matches!(
        f.db.read(&result, &access).unwrap(),
        Some(Record::Concept(Concept {
            status: ConceptStatus::Provisional,
            ..
        }))
    ));
}

#[test]
fn concept_rejects_scope_changes_even_when_caller_remains_authorized() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let mut broad = access();
    broad.scopes.insert("extra".into());
    let receipt =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &broad)
            .unwrap();
    f.db.conn
        .execute("INSERT INTO scope VALUES (?1,'extra')", [&refs[2].revision])
        .unwrap();
    assert!(f
        .db
        .retain_concept(receipt, &draft(&refs), &broad, None)
        .is_err());
}

#[test]
fn concept_rejects_model_selected_authority_and_missing_provenance_then_can_withdraw() {
    let mut f = Fixture::new();
    let refs = sources(&mut f);
    let original: serde_json::Value = serde_json::from_str(&draft(&refs)).unwrap();
    for case in [
        "authority",
        "unknown_source",
        "missing_counterexamples",
        "wrong_field",
    ] {
        let mut reply = original.clone();
        match case {
            "authority" => reply["status"] = json!("trusted"),
            "unknown_source" => reply["support"][0]["source_id"] = json!("model-invented"),
            "missing_counterexamples" => {
                reply.as_object_mut().unwrap().remove("counterexamples");
            }
            "wrong_field" => reply["support"][0]["field"] = json!("/summary"),
            _ => unreachable!(),
        }
        let receipt =
            f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
                .unwrap();
        assert!(
            f.db.retain_concept(receipt, &reply.to_string(), &access(), None)
                .is_err(),
            "{case}"
        );
    }
    let receipt =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    let reference =
        f.db.retain_concept(receipt, &draft(&refs), &access(), None)
            .unwrap();
    f.db.withdraw(&reference).unwrap();
    let hits =
        f.db.search(
            &Query {
                text: "Approval".into(),
                known_at: f.db.snapshot().unwrap(),
                valid_at: None,
                include_possible: false,
                limit: 10,
            },
            &access(),
        )
        .unwrap();
    assert!(!hits.iter().any(|h| h.reference.object == reference.object));
}
