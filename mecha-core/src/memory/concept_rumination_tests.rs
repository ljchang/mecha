use super::*;
use crate::memory::transcript_tests::{access, Fixture};
use serde_json::{json, Value};

fn capture(f: &mut Fixture, id: &str, text: &str) -> RevisionRef {
    f.db.capture(Capture {
        source: SourceKey {
            connector: "synthetic".into(),
            account: "a".into(),
            id: id.into(),
        },
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    })
    .unwrap()
}
fn selection(r: &RevisionRef, quote: &str) -> Value {
    json!({"source_id": r.revision, "field":"", "quote":quote})
}
fn draft(refs: &[RevisionRef]) -> Value {
    json!({"name":"Approval dependency", "principle":"Pending approval needs an alternate route.", "applicability":"Ordinary approval cases.",
        "support":[selection(&refs[0], "permit pending"), selection(&refs[1], "inspection pending")], "counterexamples":[]})
}
fn fixture() -> (Fixture, Vec<RevisionRef>, RevisionRef) {
    let mut f = Fixture::new();
    let refs = vec![
        capture(&mut f, "one", "permit pending"),
        capture(&mut f, "two", "inspection pending"),
        capture(&mut f, "uncited", "weather is calm"),
    ];
    let req =
        f.db.prepare_concept(&refs, &f.db.snapshot().unwrap(), &access())
            .unwrap();
    let prior =
        f.db.retain_concept(req, &draft(&refs).to_string(), &access(), None)
            .unwrap();
    (f, refs, prior)
}
fn counter(f: &mut Fixture) -> ConceptExcerpt {
    ConceptExcerpt {
        evidence: capture(f, "exception", "emergency waiver"),
        field: "".into(),
        start: 0,
        end: 16,
        quote: "emergency waiver".into(),
    }
}
fn revised(refs: &[RevisionRef], exceptions: &[ConceptExcerpt]) -> String {
    let mut d = draft(refs);
    d["applicability"] = json!("Ordinary cases; emergency waivers are exceptions.");
    d["counterexamples"] = Value::Array(
        exceptions
            .iter()
            .map(|e| selection(&e.evidence, &e.quote))
            .collect(),
    );
    json!({"outcome":"revised_provisional", "draft":d}).to_string()
}

#[test]
fn rumination_preserves_full_evidence_across_revision_reopen_and_recall() {
    let (mut f, refs, prior) = fixture();
    let exception = counter(&mut f);
    let request =
        f.db.prepare_concept_rumination(
            &prior,
            std::slice::from_ref(&exception),
            &f.db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    assert!(request.generation().prompt.contains("untrusted hypothesis"));
    let result =
        f.db.finish_concept_rumination(request, &revised(&refs, &[exception]), &access())
            .unwrap();
    let RuminationOutcome::RevisedProvisional { reference } = result else {
        panic!("expected revision")
    };
    assert_eq!(reference.object, prior.object);
    assert_ne!(reference.revision, prior.revision);
    let reopened = Memory::open(&f.dir.join("test.memory-prototype.sqlite")).unwrap();
    let Some(Record::Concept(c)) = reopened.read(&reference, &access()).unwrap() else {
        panic!()
    };
    assert_eq!(c.consulted.len(), 4);
    assert_eq!(c.counterexamples[0].quote, "emergency waiver");
    assert!(matches!(c.status, ConceptStatus::Provisional));
    let packet = reopened
        .context(
            &context::ContextRequest {
                query: "Approval".into(),
                bindings: vec![],
            },
            &context::ContextPolicy {
                access: access(),
                recall: RecallQuery {
                    text: "Approval".into(),
                    known_at: reopened.snapshot().unwrap(),
                    account: None,
                    session_id: None,
                    occurred_after: None,
                    occurred_before: None,
                    limit: 10,
                    neighbors: 0,
                    max_bytes: 50_000,
                },
                valid_at: None,
            },
            &context::Situation {
                event: "route planning".into(),
                goal: "plan departure".into(),
                needs: None,
            },
        )
        .unwrap();
    assert!(packet.records.iter().any(|h| h.reference == reference));
    for source in c.consulted {
        assert!(packet.records.iter().any(|h| h.reference == source));
    }
}

#[test]
fn rumination_rejects_dropped_counterexamples_without_a_write() {
    let (mut f, refs, prior) = fixture();
    let exception = counter(&mut f);
    let request =
        f.db.prepare_concept_rumination(&prior, &[exception], &f.db.snapshot().unwrap(), &access())
            .unwrap();
    let before = f.db.snapshot().unwrap().sequence;
    assert!(f
        .db
        .finish_concept_rumination(request, &revised(&refs, &[]), &access())
        .is_err());
    assert_eq!(f.db.snapshot().unwrap().sequence, before);
}

#[test]
fn rumination_unchanged_and_abstained_do_not_persist_new_evidence() {
    for outcome in ["unchanged", "abstained"] {
        let (mut f, _, prior) = fixture();
        let exception = counter(&mut f);
        let request = f
            .db
            .prepare_concept_rumination(&prior, &[exception], &f.db.snapshot().unwrap(), &access())
            .unwrap();
        let before = f.db.snapshot().unwrap().sequence;
        let result =
            f.db.finish_concept_rumination(
                request,
                &json!({"outcome":outcome,"reason":"Insufficient basis for revision."}).to_string(),
                &access(),
            )
            .unwrap();
        assert!(matches!(
            result,
            RuminationOutcome::Unchanged { .. } | RuminationOutcome::Abstained { .. }
        ));
        assert_eq!(f.db.snapshot().unwrap().sequence, before);
        let Some(Record::Concept(c)) = f.db.read(&prior, &access()).unwrap() else {
            panic!()
        };
        assert_eq!(c.consulted.len(), 3);
        assert!(c.counterexamples.is_empty());
    }
}

#[test]
fn rumination_rechecks_every_outcome_including_uncited_and_previous_scopes() {
    for outcome in ["revised_provisional", "unchanged", "abstained"] {
        for change in [
            "source_revision",
            "source_scope",
            "new_source_revision",
            "new_source_scope",
            "new_source_forget",
            "native_binding",
            "previous_scope",
            "forget",
            "previous_revision",
            "permission",
        ] {
            let (mut f, refs, prior) = fixture();
            let exception = counter(&mut f);
            let mut broad = access();
            broad.scopes.insert("extra".into());
            let request =
                f.db.prepare_concept_rumination(
                    &prior,
                    std::slice::from_ref(&exception),
                    &f.db.snapshot().unwrap(),
                    &broad,
                )
                .unwrap();
            match change {
                "source_revision" => {
                    capture(&mut f, "uncited", "weather changed");
                }
                "source_scope" => {
                    f.db.conn
                        .execute("INSERT INTO scope VALUES (?1,'extra')", [&refs[2].revision])
                        .unwrap();
                }
                "new_source_revision" => {
                    capture(&mut f, "exception", "waiver expired");
                }
                "new_source_scope" => {
                    f.db.conn
                        .execute(
                            "INSERT INTO scope VALUES (?1,'extra')",
                            [&exception.evidence.revision],
                        )
                        .unwrap();
                }
                "new_source_forget" => {
                    f.db.forget(&SourceKey {
                        connector: "synthetic".into(),
                        account: "a".into(),
                        id: "exception".into(),
                    })
                    .unwrap();
                }
                "native_binding" => {
                    f.db.conn.execute_batch("CREATE TABLE external_binding(revision TEXT PRIMARY KEY,authority TEXT,locator TEXT,binding_revision TEXT);").unwrap();
                    f.db.conn
                        .execute(
                            "INSERT INTO external_binding VALUES (?1,'host-v1',?2,?2)",
                            rusqlite::params![refs[2].revision, "a".repeat(64)],
                        )
                        .unwrap();
                }
                "previous_scope" => {
                    f.db.conn
                        .execute("INSERT INTO scope VALUES (?1,'extra')", [&prior.revision])
                        .unwrap();
                }
                "forget" => {
                    f.db.forget(&SourceKey {
                        connector: "synthetic".into(),
                        account: "a".into(),
                        id: "uncited".into(),
                    })
                    .unwrap();
                }
                "previous_revision" => {
                    f.db.withdraw(&prior).unwrap();
                }
                "permission" => {
                    broad.scopes.clear();
                }
                _ => unreachable!(),
            }
            let before = f.db.snapshot().unwrap().sequence;
            let reply = if outcome == "revised_provisional" {
                revised(&refs, &[exception])
            } else {
                json!({"outcome":outcome,"reason":"No revision."}).to_string()
            };
            assert!(
                f.db.finish_concept_rumination(request, &reply, &broad)
                    .is_err(),
                "{outcome}: {change}"
            );
            assert_eq!(
                f.db.snapshot().unwrap().sequence,
                before,
                "{outcome}: {change}"
            );
        }
    }
}

#[test]
fn rumination_refuses_inexact_new_excerpts_before_inference_and_preserves_old_exceptions() {
    let (mut f, refs, prior) = fixture();
    let exception = counter(&mut f);
    for bad in [
        ConceptExcerpt {
            start: 1,
            ..exception.clone()
        },
        ConceptExcerpt {
            quote: "invented".into(),
            ..exception.clone()
        },
        ConceptExcerpt {
            field: "/summary".into(),
            ..exception.clone()
        },
        ConceptExcerpt {
            evidence: RevisionRef {
                object: "foreign-object".into(),
                revision: exception.evidence.revision.clone(),
            },
            ..exception.clone()
        },
    ] {
        assert!(f
            .db
            .prepare_concept_rumination(&prior, &[bad], &f.db.snapshot().unwrap(), &access())
            .is_err());
    }
    let req =
        f.db.prepare_concept_rumination(
            &prior,
            std::slice::from_ref(&exception),
            &f.db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let RuminationOutcome::RevisedProvisional { reference } =
        f.db.finish_concept_rumination(
            req,
            &revised(&refs, std::slice::from_ref(&exception)),
            &access(),
        )
        .unwrap()
    else {
        panic!()
    };
    let req =
        f.db.prepare_concept_rumination(&reference, &[], &f.db.snapshot().unwrap(), &access())
            .unwrap();
    assert!(f
        .db
        .finish_concept_rumination(req, &revised(&refs, &[]), &access())
        .is_err());
    // Duplicating an already consulted source does not count as another episode.
    let req =
        f.db.prepare_concept_rumination(
            &reference,
            &[exception.clone(), exception.clone()],
            &f.db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let RuminationOutcome::RevisedProvisional { reference } =
        f.db.finish_concept_rumination(req, &revised(&refs, &[exception]), &access())
            .unwrap()
    else {
        panic!()
    };
    let Some(Record::Concept(c)) = f.db.read(&reference, &access()).unwrap() else {
        panic!()
    };
    assert_eq!(c.consulted.len(), 4);
    assert_eq!(c.counterexamples.len(), 1);
}

#[tokio::test]
async fn rumination_calls_provider_once_and_errors_never_write() {
    let (mut f, _, prior) = fixture();
    let exception = counter(&mut f);
    let snapshot = f.db.snapshot().unwrap();
    let before = snapshot.sequence;
    let mut calls = 0;
    let result =
        f.db.ruminate_concept(&prior, &[exception], &snapshot, &access(), |_| {
            calls += 1;
            std::future::ready(Ok(
                "{\"outcome\":\"abstained\",\"reason\":\"Insufficient evidence.\"}".into(),
            ))
        })
        .await
        .unwrap();
    assert!(matches!(result, RuminationOutcome::Abstained { .. }));
    assert_eq!(calls, 1);
    assert_eq!(f.db.snapshot().unwrap().sequence, before);
    let mut calls = 0;
    assert!(f
        .db
        .ruminate_concept(&prior, &[], &snapshot, &access(), |_| {
            calls += 1;
            std::future::ready(Err(anyhow::anyhow!("provider failed")))
        })
        .await
        .is_err());
    assert_eq!(calls, 1);
    assert_eq!(f.db.snapshot().unwrap().sequence, before);
}

#[test]
fn rumination_preparation_bounds_union_and_refuses_stale_expected_concepts() {
    let (mut f, _, prior) = fixture();
    let mut exceptions = Vec::new();
    for i in 0..6 {
        exceptions.push(ConceptExcerpt {
            evidence: capture(&mut f, &format!("extra-{i}"), "exception"),
            field: "".into(),
            start: 0,
            end: 9,
            quote: "exception".into(),
        });
    }
    let before = f.db.snapshot().unwrap().sequence;
    assert!(f
        .db
        .prepare_concept_rumination(&prior, &exceptions, &f.db.snapshot().unwrap(), &access())
        .is_err());
    assert_eq!(f.db.snapshot().unwrap().sequence, before);
    f.db.withdraw(&prior).unwrap();
    assert!(f
        .db
        .prepare_concept_rumination(&prior, &[], &f.db.snapshot().unwrap(), &access())
        .is_err());
}

#[test]
fn rumination_refuses_invalid_or_authoritative_outputs_without_writes() {
    for reply in [
        "not json",
        "{\"outcome\":\"trusted\"}",
        "{\"outcome\":\"unchanged\",\"reason\":\"\"}",
        "{\"outcome\":\"abstained\",\"reason\":\"Unknown\",\"authority\":true}",
    ] {
        let (mut f, _, prior) = fixture();
        let req =
            f.db.prepare_concept_rumination(&prior, &[], &f.db.snapshot().unwrap(), &access())
                .unwrap();
        let before = f.db.snapshot().unwrap().sequence;
        assert!(f
            .db
            .finish_concept_rumination(req, reply, &access())
            .is_err());
        assert_eq!(f.db.snapshot().unwrap().sequence, before);
    }
}

#[test]
fn rumination_preserves_the_previous_concepts_own_scope_restrictions() {
    let (mut f, refs, prior) = fixture();
    f.db.conn
        .execute("INSERT INTO scope VALUES (?1,'extra')", [&prior.revision])
        .unwrap();
    let mut broad = access();
    broad.scopes.insert("extra".into());
    let request =
        f.db.prepare_concept_rumination(&prior, &[], &f.db.snapshot().unwrap(), &broad)
            .unwrap();
    let RuminationOutcome::RevisedProvisional { reference } =
        f.db.finish_concept_rumination(request, &revised(&refs, &[]), &broad)
            .unwrap()
    else {
        panic!()
    };
    assert!(f.db.read(&reference, &access()).unwrap().is_none());
    assert!(f.db.read(&reference, &broad).unwrap().is_some());
}

#[test]
fn rumination_rejects_duplicate_json_fields_at_every_depth() {
    for key in ["name", "quote"] {
        let (mut f, refs, prior) = fixture();
        let request =
            f.db.prepare_concept_rumination(&prior, &[], &f.db.snapshot().unwrap(), &access())
                .unwrap();
        let reply = revised(&refs, &[]).replacen(
            &format!("\"{key}\":"),
            &format!("\"{key}\":\"discarded\",\"{key}\":"),
            1,
        );
        let before = f.db.snapshot().unwrap().sequence;
        assert!(
            f.db.finish_concept_rumination(request, &reply, &access())
                .is_err(),
            "duplicate {key}"
        );
        assert_eq!(f.db.snapshot().unwrap().sequence, before);
    }
}
