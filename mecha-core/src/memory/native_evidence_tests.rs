use super::*;
use crate::memory::support_tests::{access, prepared_supported, supported_fixture};
use serde_json::json;

#[test]
fn native_values_are_complete_and_authenticated_json_text_stays_text() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let artifact = r
        .view()
        .facts
        .iter()
        .find(|f| f.metadata.role == EvidenceRole::ArtifactObservation)
        .unwrap();
    let NativeValue::ArtifactObservation(value) = &artifact.value else {
        panic!("native artifact envelope was not decoded")
    };
    assert_eq!(value.artifact_text.as_deref(), Some("[]"));
    assert_eq!(value.content_status, ContentStatus::Complete);
    assert_eq!(artifact.selector, NativeSelector::WholeField);
    assert_eq!(
        artifact.metadata,
        *p.evidence_metadata
            .iter()
            .find(|m| m.source_id == artifact.source_id)
            .unwrap()
    );
    let mut ordinary = p.sources[0].clone();
    ordinary.text = serde_json::to_string(value).unwrap();
    let mut metadata = artifact.metadata.clone();
    metadata.role = EvidenceRole::AssistantStatement;
    assert!(
        matches!(decode_native(&ordinary, &metadata).unwrap(), NativeValue::Text(t) if t == ordinary.text)
    );
}

fn reply(r: &NativeEvidenceRequest) -> String {
    json!({"request_key":r.view().request_key,"state":"met","because":[r.view().facts[0].id],"reason":"Synthetic provisional assessment","questions":[]}).to_string()
}

#[test]
fn appraisal_is_read_only_bound_and_rejects_foreign_duplicate_or_missing_premises() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let before = db.snapshot().unwrap();
    let valid = reply(&r);
    assert_eq!(
        db.finish_native_appraisal(&r, &valid, &access())
            .unwrap()
            .state,
        NativeCriterionState::Met
    );
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(db.snapshot().unwrap()).unwrap()
    );
    for ids in [
        json!([]),
        json!(["foreign"]),
        json!([r.view().facts[0].id, r.view().facts[0].id]),
    ] {
        let mut bad: serde_json::Value = serde_json::from_str(&valid).unwrap();
        bad["because"] = ids;
        assert!(db
            .finish_native_appraisal(&r, &bad.to_string(), &access())
            .is_err());
    }
    let mut other = p.clone();
    other.context =
        InterpretationContext::new(p.context.criterion.clone(), "different-host-context".into())
            .unwrap();
    let r2 = NativeEvidenceRequest::new(other).unwrap();
    assert!(parse_appraisal(&r2, &valid).is_err());
    let mut bad: serde_json::Value = serde_json::from_str(&valid).unwrap();
    bad["state"] = json!("unknown");
    assert!(db
        .finish_native_appraisal(&r, &bad.to_string(), &access())
        .is_err());
    bad["questions"] = json!(["Which required observation is missing?"]);
    assert!(db
        .finish_native_appraisal(&r, &bad.to_string(), &access())
        .is_ok());
}

fn captured(db: &Memory, p: &PreparedInterpretation) -> HostCapture {
    let Some(Record::Evidence(projection)) = lookup(&db.conn, &p.receipt.projection).unwrap()
    else {
        panic!("missing capture")
    };
    projection_input(&db.conn, &projection).unwrap()
}

#[test]
fn complete_long_empty_and_omitted_artifact_states_remain_distinct() {
    for (status, text, verdict) in [
        (
            ContentStatus::Complete,
            Some("".to_owned()),
            CheckVerdict::Failed,
        ),
        (
            ContentStatus::Complete,
            Some("complete native value with escaped quote \" and line\n".repeat(30)),
            CheckVerdict::Failed,
        ),
        (ContentStatus::Missing, None, CheckVerdict::Unavailable),
        (ContentStatus::Unavailable, None, CheckVerdict::Unavailable),
        (ContentStatus::Refused, None, CheckVerdict::Refused),
        (ContentStatus::NonUtf8, None, CheckVerdict::Failed),
        (ContentStatus::Oversized, None, CheckVerdict::Failed),
    ] {
        let (_, mut db, _) = supported_fixture();
        let original = prepared_supported(&mut db);
        let mut input = captured(&db, &original);
        input.identity.prefix_digest = "2".repeat(64);
        let artifact_digest = if let Some(t) = &text {
            Some(
                Sha256::digest(t.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
            )
        } else if matches!(status, ContentStatus::NonUtf8 | ContentStatus::Oversized) {
            Some("9".repeat(64))
        } else {
            None
        };
        input.artifact = ArtifactCheck {
            artifact_digest: artifact_digest.clone(),
            verdict: verdict.clone(),
        };
        let envelope = ArtifactObservation {
            content_status: status.clone(),
            artifact_text: text.clone(),
            artifact_digest,
            verdict,
        };
        input
            .originals
            .iter_mut()
            .find(|p| p.kind == PassageKind::ArtifactObservation)
            .unwrap()
            .text = serde_json::to_string(&envelope).unwrap();
        let receipt = db
            .admit_host_capture_with_evidence(input, &access())
            .unwrap();
        let p = db
            .prepare_interpretation(
                &receipt,
                &[],
                original.context(),
                &db.snapshot().unwrap(),
                &access(),
            )
            .unwrap();
        let r = db.prepare_native_evidence(&p, &access()).unwrap();
        let f = r
            .view()
            .facts
            .iter()
            .find(|f| f.metadata.role == EvidenceRole::ArtifactObservation)
            .unwrap();
        let NativeValue::ArtifactObservation(observed) = &f.value else {
            panic!("missing native artifact")
        };
        assert_eq!(observed, &envelope);
        assert_eq!(observed.artifact_text, text);
        assert_eq!(observed.content_status, status);
    }
}

#[test]
fn malformed_authenticated_envelopes_fail_closed_without_decoding_ordinary_json() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let fact = r
        .view()
        .facts
        .iter()
        .find(|f| f.metadata.role == EvidenceRole::ArtifactObservation)
        .unwrap();
    let mut source = p
        .sources
        .iter()
        .find(|s| s.id == fact.source_id)
        .unwrap()
        .clone();
    for invalid in [
        "{}",
        "not json",
        r#"{"content_status":"Complete","content_status":"Missing","artifact_text":null,"artifact_digest":null,"verdict":"Failed"}"#,
    ] {
        source.text = invalid.into();
        assert!(decode_native(&source, &fact.metadata).is_err());
        let mut ordinary = fact.metadata.clone();
        ordinary.role = EvidenceRole::ToolResult;
        assert!(
            matches!(decode_native(&source,&ordinary).unwrap(),NativeValue::Text(t) if t==invalid)
        );
    }
    source.field = "/user_supplied_artifact".into();
    assert!(decode_native(&source, &fact.metadata).is_err());
}

#[test]
fn every_uncited_source_remains_a_dependency_at_prepare_dispatch_and_finish() {
    for mutation in 0..4 {
        let (_, mut db, guard) = supported_fixture();
        let p = prepared_supported(&mut db);
        let r = db.prepare_native_evidence(&p, &access()).unwrap();
        let mut response: serde_json::Value = serde_json::from_str(&reply(&r)).unwrap();
        let cited = r
            .view()
            .facts
            .iter()
            .find(|f| f.metadata.role == EvidenceRole::ArtifactObservation)
            .unwrap();
        response["because"] = json!([cited.id]);
        let uncited = p
            .sources
            .iter()
            .find(|s| {
                p.evidence_metadata
                    .iter()
                    .any(|m| m.source_id == s.id && m.role == EvidenceRole::AssistantStatement)
            })
            .unwrap();
        let Some(Record::Evidence(original)) = lookup(&db.conn, &uncited.source).unwrap() else {
            panic!("missing original")
        };
        match mutation {
            0 => guard.store(false, std::sync::atomic::Ordering::SeqCst),
            1 => {
                db.forget(&original.source).unwrap();
            }
            2 => {
                let mut updated = captured(&db, &p);
                updated.identity.prefix_digest = "2".repeat(64);
                updated
                    .originals
                    .iter_mut()
                    .find(|o| o.source.id == original.source.id)
                    .unwrap()
                    .text = "new unrelated synthetic source revision".into();
                db.admit_host_capture_with_evidence(updated, &access())
                    .unwrap();
            }
            _ => {
                db.conn
                    .execute(
                        "INSERT INTO scope VALUES (?1,'restricted')",
                        [&uncited.source.revision],
                    )
                    .unwrap();
            }
        }
        assert!(
            db.prepare_native_evidence(&p, &access()).is_err(),
            "prepare mutation {mutation}"
        );
        assert!(
            db.recheck_native_evidence(&r, &access()).is_err(),
            "dispatch mutation {mutation}"
        );
        assert!(
            db.finish_native_appraisal(&r, &response.to_string(), &access())
                .is_err(),
            "finish mutation {mutation}"
        );
    }
}

#[test]
fn raw_and_typed_formats_share_provenance_ids_schema_criterion_and_complete_values() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let (raw, raw_schema) = r.input(NativeEvidenceFormat::Raw).unwrap();
    let (typed, typed_schema) = r.input(NativeEvidenceFormat::Typed).unwrap();
    assert_eq!(raw_schema, typed_schema);
    let raw: Value = serde_json::from_str(&raw).unwrap();
    let typed: Value = serde_json::from_str(&typed).unwrap();
    assert_eq!(raw["criterion"], typed["criterion"]);
    assert_eq!(raw["request_key"], typed["request_key"]);
    for ((a, b), s) in raw["facts"]
        .as_array()
        .unwrap()
        .iter()
        .zip(typed["facts"].as_array().unwrap())
        .zip(p.sources())
    {
        assert_eq!(a["value"]["value"], s.text);
        for field in ["id", "source_id", "source", "field", "selector", "metadata"] {
            assert_eq!(a[field], b[field]);
        }
    }
    for branch in typed_schema["oneOf"].as_array().unwrap() {
        assert_eq!(branch["type"], "object");
        assert_eq!(branch["additionalProperties"], false);
        assert_eq!(branch["required"].as_array().unwrap().len(), 5);
        assert_eq!(branch["properties"].as_object().unwrap().len(), 5);
        assert_eq!(
            branch["properties"]["request_key"]["enum"],
            json!([r.view().request_key])
        );
    }
    let r2 = db.prepare_native_evidence(&p, &access()).unwrap();
    assert_eq!(
        serde_json::to_value(r.view()).unwrap(),
        serde_json::to_value(r2.view()).unwrap()
    );
    let mut changed = p.clone();
    changed.evidence_metadata[0].captures[0].passage_time = Some("2026-01-01T00:00:00Z".into());
    let r3 = NativeEvidenceRequest::new(changed).unwrap();
    assert_ne!(r.view().facts[0].id, r3.view().facts[0].id);
    assert_ne!(r.view().request_key, r3.view().request_key);
}

#[test]
fn oversized_whole_values_are_explicit_omissions_and_never_issued_premises() {
    let (_, mut db, _) = supported_fixture();
    let mut p = prepared_supported(&mut db);
    let index = p
        .evidence_metadata
        .iter()
        .position(|m| m.role == EvidenceRole::AssistantStatement)
        .unwrap();
    p.sources[index].text = "x".repeat(VALUE_BYTES + 1);
    let r = NativeEvidenceRequest::new(p).unwrap();
    let f = &r.view().facts[index];
    assert!(
        matches!(f.value,NativeValue::Omitted {reason:NativeOmissionReason::ValueTooLarge,original_bytes,..} if original_bytes==VALUE_BYTES+1)
    );
    let response = json!({"request_key":r.view().request_key,"state":"met","because":[f.id],"reason":"Unusable omitted premise","questions":[]});
    assert!(parse_appraisal(&r, &response.to_string()).is_err());
    let (raw, _) = r.input(NativeEvidenceFormat::Raw).unwrap();
    assert!(!raw.contains(&"x".repeat(100)));
}

#[test]
#[cfg(feature = "memory-prototype")]
fn matched_provider_requests_use_identical_explicit_budget_and_no_tools() {
    let (_, mut db, _) = supported_fixture();
    let p = prepared_supported(&mut db);
    let r = db.prepare_native_evidence(&p, &access()).unwrap();
    let budget = crate::memory_factual_support::GenerationBudget::new(
        1024,
        std::time::Duration::from_secs(30),
    )
    .unwrap();
    let raw = r
        .generation_request("synthetic-local", budget, NativeEvidenceFormat::Raw)
        .unwrap();
    let typed = r
        .generation_request("synthetic-local", budget, NativeEvidenceFormat::Typed)
        .unwrap();
    assert_eq!(raw.max_tokens, typed.max_tokens);
    assert_eq!(raw.max_tokens, 1024);
    assert_eq!(raw.response_schema, typed.response_schema);
    assert_eq!(raw.think, Some(false));
    assert_eq!(raw.think, typed.think);
    assert!(raw.tools.is_empty());
    assert!(typed.tools.is_empty());
    assert_eq!(raw.system, typed.system);
}
