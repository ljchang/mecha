//! Synthetic authored checks of reference integrity, never semantic reliability.
use super::*;
use crate::memory::support_tests::{access, prepared_supported, supported_fixture};
use std::sync::{atomic::Ordering, Arc};

struct Fixture {
    path: std::path::PathBuf,
    db: Memory,
    live: Arc<std::sync::atomic::AtomicBool>,
    access: Access,
    template: HostCapture,
    context: InterpretationContext,
    concept: RevisionRef,
    receipt: CaptureReceipt,
    uncited: RevisionRef,
    opposition: RevisionRef,
}

impl Fixture {
    fn new() -> Self {
        let (path, mut db, live) = supported_fixture();
        let prepared = prepared_supported(&mut db);
        let Some(Record::Evidence(projection)) =
            lookup(&db.conn, &prepared.receipt.projection).unwrap()
        else {
            panic!()
        };
        let template = projection_input(&db.conn, &projection).unwrap();
        let mut receipts = vec![prepared.receipt.clone()];
        for name in ["second", "uncited", "opposition"] {
            let capture = Self::input(&template, name, vec![]);
            receipts.push(
                db.admit_host_capture_with_evidence(capture, &access())
                    .unwrap(),
            );
        }
        let selected: Vec<_> = receipts.iter().map(|r| r.originals[0].clone()).collect();
        let request = db
            .prepare_native_concept(&selected, &db.snapshot().unwrap(), &access())
            .unwrap();
        let selection = |r: &RevisionRef| json!({"source_id": r.revision, "field": "/tool/call1", "quote": "convert synthetic CSV to JSON"});
        let draft = json!({"name":"Synthetic conversion hypothesis", "principle":"Check the independently observed output.", "applicability":"Synthetic conversion tasks.", "support":[selection(&selected[0]),selection(&selected[1])], "counterexamples":[selection(&selected[3])]});
        let concept = db
            .retain_native_concept(request, &draft.to_string(), &access(), None)
            .unwrap();
        let receipt = db
            .admit_host_capture_with_evidence(
                Self::input(&template, "recapture", vec![concept.clone()]),
                &access(),
            )
            .unwrap();
        Self {
            path,
            db,
            live,
            access: access(),
            template,
            context: prepared.context,
            concept,
            receipt,
            uncited: selected[2].clone(),
            opposition: selected[3].clone(),
        }
    }

    fn input(template: &HostCapture, name: &str, consulted: Vec<RevisionRef>) -> HostCapture {
        let mut input = template.clone();
        input.identity.session_id = name.into();
        for original in &mut input.originals {
            original.source.id = format!("{name}-{}", original.source.id);
        }
        input.consulted = consulted;
        input
    }

    fn request(&self) -> NativeConceptEvidenceRequest {
        self.db
            .prepare_native_concept_evidence(
                &self.receipt,
                &[],
                &self.context,
                &self.db.snapshot().unwrap(),
                &self.access,
            )
            .unwrap()
    }
}

fn response(r: &NativeConceptEvidenceRequest) -> Value {
    let artifact = r
        .view()
        .facts
        .iter()
        .find(|f| {
            f.metadata.role == EvidenceRole::ArtifactObservation
                && f.metadata
                    .captures
                    .iter()
                    .any(|c| c.relation == CaptureRelation::SelectedCapture)
        })
        .unwrap();
    json!({"appraisal":{"request_key":r.view().request_key,"artifact_content":"complete","artifact_facts":[artifact.id],"comparison":"contradicts","because":[artifact.id],"reason":"The observed synthetic array is empty.","questions":[],"state":"not_met"}, "hypotheses":r.view().hypotheses.iter().map(|h| json!({"hypothesis_id":h.id,"applicability":"applicable","because":[artifact.id],"reason":"The current criterion concerns conversion output.","questions":[]})).collect::<Vec<_>>()})
}

#[test]
fn explicit_native_concepts_are_separate_from_facts_and_do_not_enable_legacy_preparation() {
    let f = Fixture::new();
    assert!(f
        .db
        .prepare_interpretation(
            &f.receipt,
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    let r = f.request();
    assert_eq!(r.view().protocol, "situated-native-concept-appraisal-v1");
    assert_eq!(r.view().hypotheses.len(), 1);
    let h = &r.view().hypotheses[0];
    assert_eq!(h.source, f.concept);
    assert!(h.id.starts_with("hypothesis:"));
    assert_eq!(h.concept.support.len(), 2);
    assert_eq!(h.concept.counterexamples.len(), 1);
    assert!(h.concept.consulted.contains(&f.uncited));
    assert!(h.concept.consulted.contains(&f.opposition));
    assert!(r
        .view()
        .facts
        .iter()
        .all(|fact| fact.source != f.concept && fact.id.starts_with("fact:")));
    assert!(r
        .view()
        .facts
        .iter()
        .any(|fact| fact.metadata.interpretation_influenced
            && fact.source_kind == SourceKind::InterpretationInfluenced));
    let before = sequence(&f.db.conn).unwrap();
    let finished =
        f.db.finish_native_concept_appraisal(&r, &response(&r).to_string(), &f.access)
            .unwrap();
    assert_eq!(finished.appraisal.state, NativeCriterionState::NotMet);
    assert_eq!(
        finished.hypotheses[0].applicability,
        ConceptApplicability::Applicable
    );
    assert_eq!(sequence(&f.db.conn).unwrap(), before);
    assert!(f
        .db
        .prepare_native_concept(
            &[f.receipt.originals[0].clone(), f.uncited.clone()],
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
}

#[test]
fn hypothesis_ids_cannot_establish_facts_or_artifact_observations() {
    let f = Fixture::new();
    let r = f.request();
    for field in ["because", "artifact_facts"] {
        let mut bad = response(&r);
        bad["appraisal"][field] = json!([r.view().hypotheses[0].id]);
        assert!(
            f.db.finish_native_concept_appraisal(&r, &bad.to_string(), &f.access)
                .is_err(),
            "{field}"
        );
    }
    let mut bad = response(&r);
    bad["hypotheses"][0]["because"] = json!([r.view().hypotheses[0].id]);
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &bad.to_string(), &f.access)
        .is_err());
    let mut bad = response(&r);
    bad["appraisal"]["because"] = json!([]);
    bad["appraisal"]["artifact_facts"] = json!([]);
    bad["appraisal"]["artifact_content"] = json!("not_assessed");
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &bad.to_string(), &f.access)
        .is_err());
}

#[test]
fn assessments_require_exact_hypothesis_coverage_and_unknown_questions() {
    let f = Fixture::new();
    let r = f.request();
    for mutation in [
        "missing",
        "extra",
        "duplicate",
        "unknown_without_question",
        "foreign_reason",
        "comparison",
    ] {
        let mut bad = response(&r);
        match mutation {
            "missing" => bad["hypotheses"] = json!([]),
            "extra" => bad["hypotheses"][0]["hypothesis_id"] = json!("hypothesis:foreign"),
            "duplicate" => {
                let duplicate = bad["hypotheses"][0].clone();
                bad["hypotheses"].as_array_mut().unwrap().push(duplicate);
            }
            "unknown_without_question" => bad["hypotheses"][0]["applicability"] = json!("unknown"),
            "foreign_reason" => bad["hypotheses"][0]["because"] = json!(["fact:foreign"]),
            "comparison" => bad["appraisal"]["comparison"] = json!("supports"),
            _ => unreachable!(),
        }
        assert!(
            f.db.finish_native_concept_appraisal(&r, &bad.to_string(), &f.access)
                .is_err(),
            "{mutation}"
        );
    }
    let mut unknown = response(&r);
    unknown["hypotheses"][0]["applicability"] = json!("unknown");
    unknown["hypotheses"][0]["because"] = json!([]);
    unknown["hypotheses"][0]["questions"] = json!(["Which conversion constraint applies?"]);
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &unknown.to_string(), &f.access)
        .is_ok());
}

#[test]
fn changed_criterion_binds_new_identity_and_permits_criterion_only_inapplicability() {
    let f = Fixture::new();
    let r = f.request();
    let mut criterion = f.context.criterion().clone();
    criterion.criterion_text =
        "Confirm that an array exists; nonempty rows are unnecessary.".into();
    criterion.criterion_revision = "9".repeat(64);
    let context = InterpretationContext::new(criterion, "changed-context".into()).unwrap();
    let changed =
        f.db.prepare_native_concept_evidence(
            &f.receipt,
            &[],
            &context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    assert_ne!(r.view().request_key, changed.view().request_key);
    assert!(f
        .db
        .finish_native_concept_appraisal(&changed, &response(&r).to_string(), &f.access)
        .is_err());
    let mut reply = response(&changed);
    reply["hypotheses"][0]["applicability"] = json!("inapplicable");
    reply["hypotheses"][0]["because"] = json!([]);
    reply["hypotheses"][0]["reason"] = json!("The current criterion only requires an array.");
    reply["appraisal"]["comparison"] = json!("supports");
    reply["appraisal"]["state"] = json!("met");
    assert!(f
        .db
        .finish_native_concept_appraisal(&changed, &reply.to_string(), &f.access)
        .is_ok());
}

#[test]
fn duplicate_json_keys_and_foreign_protocol_keys_are_rejected_without_repairs() {
    let f = Fixture::new();
    let r = f.request();
    let good = response(&r).to_string();
    let repeated = good.replacen(
        "\"state\":\"not_met\"",
        "\"state\":\"met\",\"state\":\"not_met\"",
        1,
    );
    assert_ne!(repeated, good);
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &repeated, &f.access)
        .is_err());
    let p =
        f.db.prepare_interpretation(
            &f.template_receipt(),
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    let old = f.db.prepare_native_evidence(&p, &f.access).unwrap();
    for key in [
        old.view().request_key.clone(),
        old.checked_request_key().unwrap(),
    ] {
        let mut bad = response(&r);
        bad["appraisal"]["request_key"] = json!(key);
        assert!(f
            .db
            .finish_native_concept_appraisal(&r, &bad.to_string(), &f.access)
            .is_err());
    }
    assert!(f
        .db
        .finish_native_checked_appraisal(&old, &response(&r)["appraisal"].to_string(), &f.access)
        .is_err());
}

impl Fixture {
    fn template_receipt(&self) -> CaptureReceipt {
        let row: (String, String, String, String) = self.db.conn.query_row("SELECT receipt_id,projection_object,projection_revision,originals FROM host_capture_receipt WHERE state='active' ORDER BY rowid LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        CaptureReceipt {
            id: row.0,
            projection: RevisionRef {
                object: row.1,
                revision: row.2,
            },
            originals: serde_json::from_str(&row.3).unwrap(),
        }
    }
}

#[test]
fn every_uncited_opposing_and_concept_source_is_rechecked_with_exact_scopes() {
    for mutation in [
        "guard",
        "uncited",
        "opposition",
        "concept_scope",
        "uncited_scope",
        "replacement",
        "canonical_quote",
    ] {
        let mut f = Fixture::new();
        f.access.scopes.insert("extra".into());
        let r = f.request();
        let reply = response(&r).to_string();
        match mutation {
            "guard" => f.live.store(false, Ordering::SeqCst),
            "uncited" | "opposition" => {
                let reference = if mutation == "uncited" {
                    &f.uncited
                } else {
                    &f.opposition
                };
                let Some(Record::Evidence(e)) = lookup(&f.db.conn, reference).unwrap() else {
                    panic!()
                };
                f.db.forget(&e.source).unwrap();
            }
            "concept_scope" | "uncited_scope" => {
                let reference = if mutation == "concept_scope" {
                    &f.concept
                } else {
                    &f.uncited
                };
                f.db.conn
                    .execute(
                        "INSERT INTO scope VALUES (?1,'extra')",
                        [&reference.revision],
                    )
                    .unwrap();
            }
            "replacement" => {
                std::fs::rename(&f.path, f.path.with_extension("old.sqlite")).unwrap();
                drop(Memory::create_supported_situated(&f.path, "test").unwrap());
            }
            "canonical_quote" => {
                let Some(Record::Concept(mut c)) = lookup(&f.db.conn, &f.concept).unwrap() else {
                    panic!()
                };
                c.support[0].quote = "fabricated source quote".into();
                f.db.conn
                    .execute(
                        "UPDATE revision SET payload=?1 WHERE id=?2",
                        rusqlite::params![
                            serde_json::to_string(&Record::Concept(c)).unwrap(),
                            f.concept.revision
                        ],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            f.db.recheck_native_concept_evidence(&r, &f.access).is_err(),
            "recheck {mutation}"
        );
        assert!(
            f.db.finish_native_concept_appraisal(&r, &reply, &f.access)
                .is_err(),
            "finish {mutation}"
        );
    }
}

#[test]
fn sealed_views_receipts_access_and_historical_snapshots_cannot_be_substituted() {
    let f = Fixture::new();
    let r = f.request();
    let mut changed = r.clone();
    changed.view.facts[0].value = NativeValue::Text("fabricated observation".into());
    assert!(f
        .db
        .recheck_native_concept_evidence(&changed, &f.access)
        .is_err());
    let mut changed = r.clone();
    changed.view.hypotheses[0].concept.applicability = "every situation".into();
    assert!(f
        .db
        .recheck_native_concept_evidence(&changed, &f.access)
        .is_err());
    let mut changed = r.clone();
    changed.receipt.originals.reverse();
    assert!(f
        .db
        .recheck_native_concept_evidence(&changed, &f.access)
        .is_err());
    let mut changed = f.access.clone();
    changed.scopes.insert("newly-authorized".into());
    assert!(f.db.recheck_native_concept_evidence(&r, &changed).is_err());
    let mut snapshot = f.db.snapshot().unwrap();
    snapshot.sequence = 0;
    assert!(f
        .db
        .prepare_native_concept_evidence(&f.receipt, &[], &f.context, &snapshot, &f.access)
        .is_err());
    assert!(Memory::open(&f.path)
        .unwrap()
        .prepare_native_concept_evidence(
            &f.receipt,
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
}

#[test]
fn omitted_values_are_not_citable_and_remain_authorization_dependencies() {
    let mut f = Fixture::new();
    let mut input = Fixture::input(&f.template, "long", vec![]);
    let text = "\0".repeat(2048);
    let digest = Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    input.artifact.artifact_digest = Some(digest.clone());
    let passage = input
        .originals
        .iter_mut()
        .find(|p| p.kind == PassageKind::ArtifactObservation)
        .unwrap();
    passage.text = serde_json::to_string(&ArtifactObservation {
        content_status: ContentStatus::Complete,
        artifact_text: Some(text),
        artifact_digest: Some(digest),
        verdict: CheckVerdict::Failed,
    })
    .unwrap();
    let original = passage.source.clone();
    let long =
        f.db.admit_host_capture_with_evidence(input, &f.access)
            .unwrap();
    let r =
        f.db.prepare_native_concept_evidence(
            &f.receipt,
            &[long.projection],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    let omitted = r
        .view()
        .facts
        .iter()
        .find(|fact| matches!(fact.value, NativeValue::Omitted { .. }))
        .unwrap();
    let mut reply = response(&r);
    reply["hypotheses"][0]["because"] = json!([omitted.id]);
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &reply.to_string(), &f.access)
        .is_err());
    let mut reply = response(&r);
    reply["appraisal"]["because"] = json!([omitted.id]);
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &reply.to_string(), &f.access)
        .is_err());
    let reply = response(&r).to_string();
    f.db.forget(&original).unwrap();
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &reply, &f.access)
        .is_err());
}

struct Until {
    count: std::sync::atomic::AtomicUsize,
    allowed: usize,
}
impl ExternalSourceGuard for Until {
    fn check(&self, _: &ExternalBinding) -> SourceStatus {
        if self.count.fetch_add(1, Ordering::SeqCst) < self.allowed {
            SourceStatus::Checked
        } else {
            SourceStatus::Withheld
        }
    }
}

#[test]
fn preparation_and_finish_have_independent_fresh_final_authorization() {
    let mut f = Fixture::new();
    let r = f.request();
    f.db.source_guard = Some(Arc::new(Until {
        count: 0.into(),
        allowed: 1,
    }));
    assert!(f
        .db
        .prepare_native_concept_evidence(
            &f.receipt,
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
    f.db.source_guard = Some(Arc::new(Until {
        count: 0.into(),
        allowed: 2,
    }));
    assert!(f
        .db
        .finish_native_concept_appraisal(&r, &response(&r).to_string(), &f.access)
        .is_err());
}

#[test]
fn stale_revision_and_legacy_concept_roots_are_refused() {
    let mut f = Fixture::new();
    let r = f.request();
    let Some(Record::Evidence(original)) = lookup(&f.db.conn, &f.uncited).unwrap() else {
        panic!()
    };
    let mut input = Fixture::input(&f.template, "uncited", vec![]);
    input.identity.prefix_digest = "7".repeat(64);
    input
        .originals
        .iter_mut()
        .find(|p| p.source.id == original.source.id)
        .unwrap()
        .text = "revised synthetic observation".into();
    f.db.admit_host_capture_with_evidence(input, &f.access)
        .unwrap();
    assert!(f.db.recheck_native_concept_evidence(&r, &f.access).is_err());
    let mut f = Fixture::new();
    let inputs: Vec<_> = ["left", "right"]
        .into_iter()
        .map(|name| {
            f.db.capture(Capture {
                source: SourceKey {
                    connector: "synthetic".into(),
                    account: "test".into(),
                    id: name.into(),
                },
                text: "permit pending".into(),
                occurred_at: None,
                scopes: BTreeSet::new(),
                transcript: None,
            })
            .unwrap()
        })
        .collect();
    let prepared =
        f.db.prepare_concept(&inputs, &f.db.snapshot().unwrap(), &f.access)
            .unwrap();
    let draft = json!({"name":"Legacy","principle":"Check","applicability":"Current","support":inputs.iter().map(|r| json!({"source_id":r.revision,"field":"","quote":"permit"})).collect::<Vec<_>>(),"counterexamples":[]});
    let legacy =
        f.db.retain_concept(prepared, &draft.to_string(), &f.access, None)
            .unwrap();
    assert!(f
        .db
        .prepare_native_concept_evidence(
            &f.receipt,
            &[legacy],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access
        )
        .is_err());
}

#[test]
fn complete_closure_accepts_64_records_and_refuses_65_without_truncation() {
    let mut f = Fixture::new();
    let original = f.template_receipt();
    let second =
        f.db.admit_host_capture_with_evidence(
            Fixture::input(&f.template, "bound-second", vec![]),
            &f.access,
        )
        .unwrap();
    let refs = [original.originals[0].clone(), second.originals[0].clone()];
    let mut concepts = Vec::new();
    for index in 0..53 {
        let p =
            f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
                .unwrap();
        let draft = json!({"name":format!("H{index}"),"principle":"Check","applicability":"Current","support":refs.iter().map(|r| json!({"source_id":r.revision,"field":"/tool/call1","quote":"convert"})).collect::<Vec<_>>(),"counterexamples":[]});
        concepts.push(
            f.db.retain_native_concept(p, &draft.to_string(), &f.access, None)
                .unwrap(),
        );
    }
    let receipt =
        f.db.admit_host_capture_with_evidence(
            Fixture::input(&f.template, "bound-current", concepts[..52].to_vec()),
            &f.access,
        )
        .unwrap();
    assert_eq!(closure(&f.db.conn, &receipt.projection).unwrap().len(), 64);
    let request =
        f.db.prepare_native_concept_evidence(
            &receipt,
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap();
    assert_eq!(request.view().hypotheses.len(), 52);
    assert!(serde_json::to_vec(request.view()).unwrap().len() <= 64 * 1024);
    let mut reply = response(&request);
    reply["hypotheses"] = json!(request
        .view()
        .hypotheses
        .iter()
        .map(|h| json!({
            "hypothesis_id":h.id,"applicability":"unknown","because":[],
            "reason":"Unresolved","questions":["Which scope applies?"]
        }))
        .collect::<Vec<_>>());
    let reply = reply.to_string();
    assert!(
        reply.len() > 8192 && reply.len() <= 16384,
        "reply bytes: {}",
        reply.len()
    );
    assert_eq!(
        f.db.finish_native_concept_appraisal(&request, &reply, &f.access)
            .unwrap()
            .hypotheses
            .len(),
        52
    );
    #[cfg(feature = "memory-prototype")]
    {
        let budget = crate::memory_factual_support::GenerationBudget::new(
            8192,
            std::time::Duration::from_secs(60),
        )
        .unwrap();
        let generated = request.generation_request("authored", budget).unwrap();
        assert_eq!(generated.max_tokens, 8192);
        assert!(generated.tools.is_empty());
        assert_eq!(
            generated.response_schema.as_ref().unwrap()["properties"]["hypotheses"]["minItems"],
            52
        );
    }

    let err =
        f.db.prepare_native_concept_evidence(
            &receipt,
            &concepts[52..],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap_err();
    assert!(err.to_string().contains("exceeds 64"), "{err:#}");
}

#[test]
fn mandatory_hypotheses_exceeding_the_combined_view_are_refused_not_truncated() {
    let mut f = Fixture::new();
    let refs = [f.template_receipt().originals[0].clone(), f.uncited.clone()];
    let mut concepts = Vec::new();
    for index in 0..13 {
        let p =
            f.db.prepare_native_concept(&refs, &f.db.snapshot().unwrap(), &f.access)
                .unwrap();
        let draft = json!({"name":format!("Large{index}"),"principle":"p".repeat(3000),"applicability":"a".repeat(2000),"support":refs.iter().map(|r| json!({"source_id":r.revision,"field":"/tool/call1","quote":"convert"})).collect::<Vec<_>>(),"counterexamples":[]});
        concepts.push(
            f.db.retain_native_concept(p, &draft.to_string(), &f.access, None)
                .unwrap(),
        );
    }
    let receipt =
        f.db.admit_host_capture_with_evidence(
            Fixture::input(&f.template, "large-hypotheses", concepts),
            &f.access,
        )
        .unwrap();
    let error =
        f.db.prepare_native_concept_evidence(
            &receipt,
            &[],
            &f.context,
            &f.db.snapshot().unwrap(),
            &f.access,
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("hypotheses exceed view budget"),
        "{error:#}"
    );
}
