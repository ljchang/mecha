use super::*;
use crate::memory::{
    recall_tests,
    transcript_tests::{access, Fixture},
};
use rusqlite::params;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn source(id: &str) -> SourceKey {
    SourceKey {
        connector: "fixture".into(),
        account: "lab".into(),
        id: id.into(),
    }
}

fn capture(db: &mut Memory, id: &str, text: &str) -> RevisionRef {
    db.capture(Capture {
        source: source(id),
        text: text.into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    })
    .unwrap()
}

fn request(text: &str) -> ContextRequest {
    ContextRequest {
        query: text.into(),
        bindings: vec![],
    }
}

fn policy(db: &Memory) -> ContextPolicy {
    let mut recall = recall_tests::query(db, "replaced by request");
    recall.neighbors = 0;
    ContextPolicy {
        access: access(),
        recall,
        valid_at: None,
    }
}

fn situation() -> Situation {
    Situation {
        event: "Select a test fixture".into(),
        goal: "task:synthetic".into(),
        needs: Some("never an embedding input".into()),
    }
}

fn prepare(
    db: &Memory,
    text: &str,
    policy: &ContextPolicy,
    mention: Option<&str>,
) -> PreparedHybrid {
    db.prepare_hybrid_context(
        &request(text),
        policy,
        &situation(),
        &HybridOptions {
            profile_id: "fixture-profile".into(),
            dimensions: 2,
            required_mention: mention.map(str::to_owned),
        },
    )
    .unwrap()
}

fn vectors(
    prepared: &PreparedHybrid,
    vector: impl Fn(&HybridPassage) -> Vec<f32>,
) -> HybridVectors {
    HybridVectors {
        profile_id: "fixture-profile".into(),
        query: vec![1., 0.],
        passages: prepared.passages().iter().map(vector).collect(),
    }
}

fn refs(result: &HybridResult) -> Vec<&RevisionRef> {
    result.packet.records.iter().map(|h| &h.reference).collect()
}

#[test]
fn dense_finds_a_semantic_only_episode_and_hybrid_fuses_independent_ranks() {
    let mut f = Fixture::new();
    let lexical_only = capture(&mut f.db, "lexical", "aperture aperture aperture");
    let both = capture(&mut f.db, "both", "aperture opening fixture");
    let semantic_only = capture(&mut f.db, "semantic", "diaphragm opening");
    let mut p = policy(&f.db);
    p.recall.limit = 2;
    let prepared = prepare(&f.db, "aperture", &p, None);
    let vectors = vectors(&prepared, |passage| {
        if passage.reference == semantic_only {
            vec![1., 0.]
        } else if passage.reference == both {
            vec![1., 1.]
        } else {
            vec![-1., 0.]
        }
    });
    let dense =
        f.db.finish_hybrid_context(&prepared, Some(&vectors), HybridMode::Dense)
            .unwrap();
    assert_eq!(refs(&dense), [&semantic_only, &both]);
    assert_eq!(dense.ranked_anchors[0].cosine, Some(1.));
    assert_eq!(dense.ranked_anchors[0].bm25, None);
    assert_eq!(dense.ranked_anchors[0].rrf, None);
    let hybrid =
        f.db.finish_hybrid_context(&prepared, Some(&vectors), HybridMode::Hybrid)
            .unwrap();
    assert_eq!(refs(&hybrid), [&lexical_only, &both]);
    let ranks = &hybrid.ranked_anchors;
    assert_eq!(ranks[0].rrf, Some(1. / 61. + 1. / 63.));
    assert!(ranks[1].bm25.unwrap() < 0.);
    assert_eq!(ranks[1].cosine, Some(1. / 2_f64.sqrt()));
    assert_eq!(ranks[1].rrf, Some(2. / 62.));
    let lexical =
        f.db.lexical_hybrid_context(&request("aperture"), &p, &situation(), None)
            .unwrap();
    assert_eq!(refs(&lexical), [&lexical_only, &both]);
    assert!(lexical
        .ranked_anchors
        .iter()
        .all(|a| a.bm25.is_some() && a.cosine.is_none() && a.rrf.is_none()));
    assert_eq!(
        serde_json::to_vec(&lexical.packet).unwrap(),
        serde_json::to_vec(
            &f.db
                .context(&request("aperture"), &p, &situation())
                .unwrap()
        )
        .unwrap()
    );
    for result in [lexical, dense, hybrid] {
        assert!(serde_json::to_vec(&result.packet).unwrap().len() <= p.recall.max_bytes);
    }
}

#[test]
fn passages_cover_exact_unicode_spans_and_all_original_fields_with_one_vote_per_revision() {
    let mut f = Fixture::new();
    let first = format!("{}late", "🧪".repeat(2000));
    let log = format!(
        "{}\n{}\n",
        serde_json::json!({"record":"meta","id":"unicode","created_at":"2026-01-01T00:00:00Z"}),
        serde_json::json!({"record":"message","role":"user","content":[{"type":"text","text":first},{"type":"text","text":"é second"}],"unused":"never embedded"})
    );
    let imported =
        f.db.import_session_jsonl(log.as_bytes(), "lab", access().scopes)
            .unwrap();
    let newer = capture(&mut f.db, "other", "untied candidate");
    let p = policy(&f.db);
    let prepared = prepare(&f.db, "missing", &p, None);
    let passages = prepared
        .passages()
        .iter()
        .filter(|p| p.reference == imported.records[1])
        .collect::<Vec<_>>();
    assert_eq!(passages.len(), 3);
    assert_eq!(
        (
            passages[0].field.as_str(),
            passages[0].start,
            passages[0].end
        ),
        ("/content/0/text", 0, 8000)
    );
    assert_eq!(passages[0].text.chars().count(), 2000);
    assert_eq!(
        (
            passages[1].field.as_str(),
            passages[1].start,
            passages[1].end,
            passages[1].text.as_str()
        ),
        ("/content/0/text", 8000, 8004, "late")
    );
    assert_eq!(
        (
            passages[2].field.as_str(),
            passages[2].start,
            passages[2].end,
            passages[2].text.as_str()
        ),
        ("/content/1/text", 0, 9, "é second")
    );
    assert!(prepared
        .passages()
        .iter()
        .all(|p| !p.text.contains("never embedded")));
    let vecs = vectors(&prepared, |p| {
        if p.text == "late" || p.text == "é second" {
            vec![f32::MAX, 0.]
        } else {
            vec![0., 1.]
        }
    });
    let result =
        f.db.finish_hybrid_context(&prepared, Some(&vecs), HybridMode::Hybrid)
            .unwrap();
    assert_eq!(refs(&result), [&imported.records[1], &newer]);
    assert_eq!(result.ranked_anchors[0].rrf, Some(1. / 61.));
    assert_eq!(result.ranked_anchors[1].rrf, Some(1. / 62.));
}

#[test]
fn equal_cosines_order_by_descending_sequence_then_revision() {
    let mut f = Fixture::new();
    let older = capture(&mut f.db, "old", "older fixture");
    let newer = capture(&mut f.db, "new", "newer fixture");
    let prepared = prepare(&f.db, "absent", &policy(&f.db), None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    let result =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(refs(&result), [&newer, &older]);
    f.db.conn
        .execute(
            "UPDATE revision SET seq=(SELECT seq FROM revision WHERE id=?1) WHERE id=?2",
            params![older.revision, newer.revision],
        )
        .unwrap();
    let prepared = prepare(&f.db, "absent", &policy(&f.db), None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    let result =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    let mut expected = [&older, &newer];
    expected.sort_by_key(|r| &r.revision);
    assert_eq!(refs(&result), expected);
}

#[test]
fn required_mention_precedes_dense_top_k_and_all_lexical_fallbacks() {
    let mut f = Fixture::new();
    let named = capture(&mut f.db, "named", "Café Meridian paperwork");
    capture(&mut f.db, "other", "approval approval Cafés Meridian");
    let mut p = policy(&f.db);
    p.recall.limit = 1;
    let prepared = prepare(&f.db, "approval", &p, Some("cafe meridian"));
    assert_eq!(prepared.passages().len(), 1);
    assert_eq!(prepared.passages()[0].reference, named);
    let v = vectors(&prepared, |_| vec![0., 1.]);
    let dense =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(refs(&dense), [&named]);
    let unavailable =
        f.db.finish_hybrid_context(&prepared, None, HybridMode::Hybrid)
            .unwrap();
    assert_eq!(
        unavailable.fallback,
        Some(HybridFallback::EmbeddingsUnavailable)
    );
    assert_eq!(refs(&unavailable), [&named]);
    // The nonmatching row contributes to the ranking corpus; changing it must
    // invalidate even though neither its text nor vector was sent to a model.
    f.db.conn
        .execute(
            "UPDATE memory_fts SET text='approval altered' WHERE revision<>?1",
            [&named.revision],
        )
        .unwrap();
    let changed =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
            .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    assert_eq!(refs(&changed), [&named]);
    capture(&mut f.db, "too-large", &"x".repeat(262_145));
    p.recall.known_at = f.db.snapshot().unwrap();
    let bounded = prepare(&f.db, "approval", &p, Some("cafe meridian"));
    assert_eq!(
        bounded.readiness(),
        HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds)
    );
    assert!(bounded.passages().is_empty());
    let bounded =
        f.db.finish_hybrid_context(&bounded, None, HybridMode::Dense)
            .unwrap();
    assert_eq!(bounded.fallback, Some(HybridFallback::CorpusBounds));
    assert_eq!(refs(&bounded), [&named]);
    assert_eq!(bounded.effective_mode, HybridMode::Lexical);
}

#[test]
fn inaccessible_and_future_rows_do_not_use_eligible_bounds() {
    let mut f = Fixture::new();
    let visible = capture(&mut f.db, "visible", "local test source");
    f.db.capture(Capture {
        source: source("hidden"),
        text: "X".repeat(262_145),
        occurred_at: None,
        scopes: ["hidden".into()].into(),
        transcript: None,
    })
    .unwrap();
    let p = policy(&f.db);
    capture(&mut f.db, "future", &"x".repeat(262_145));
    let prepared = prepare(&f.db, "test", &p, None);
    assert_eq!(prepared.readiness(), HybridReadiness::Ready);
    assert_eq!(prepared.passages().len(), 1);
    let result =
        f.db.finish_hybrid_context(
            &prepared,
            Some(&vectors(&prepared, |_| vec![1., 0.])),
            HybridMode::Hybrid,
        )
        .unwrap();
    assert_eq!(refs(&result), [&visible]);
    assert_eq!(result.counters.unwrap().eligible_records, 1);
    assert_eq!(result.counters.unwrap().original_bytes, 17);
}

#[test]
fn record_and_passage_overflow_withhold_all_embedding_inputs() {
    let mut f = Fixture::new();
    for i in 0..257 {
        capture(&mut f.db, &format!("record-{i}"), "small");
    }
    let prepared = prepare(&f.db, "small", &policy(&f.db), None);
    assert_eq!(prepared.counters().eligible_records, 257);
    assert_eq!(
        prepared.readiness(),
        HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds)
    );
    assert!(prepared.passages().is_empty());

    let mut f = Fixture::new();
    let content = vec![serde_json::json!({"type":"text","text":"x"}); 513];
    let log = format!(
        "{}\n{}\n",
        serde_json::json!({"record":"meta","id":"passages","created_at":"2026-01-01T00:00:00Z"}),
        serde_json::json!({"record":"message","role":"user","content":content})
    );
    f.db.import_session_jsonl(log.as_bytes(), "lab", access().scopes)
        .unwrap();
    let prepared = prepare(&f.db, "small", &policy(&f.db), None);
    assert_eq!(prepared.counters().eligible_records, 1);
    assert_eq!(prepared.counters().passages, 513);
    assert_eq!(
        prepared.readiness(),
        HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds)
    );
    assert!(prepared.passages().is_empty());
}

#[test]
fn request_cannot_supply_followup_bindings_or_move_to_another_store() {
    use crate::memory::context::SourceSelection;
    let mut f = Fixture::new();
    let source = capture(&mut f.db, "selected", "named fixture");
    let p = policy(&f.db);
    let prepared = prepare(&f.db, "named", &p, None);
    let other = Fixture::new();
    assert!(other
        .db
        .finish_hybrid_context(&prepared, None, HybridMode::Dense)
        .is_err());
    let mut req = request("named");
    req.bindings.push(SourceSelection {
        source,
        field: "".into(),
        quote: "named".into(),
        query: "fixture".into(),
    });
    assert!(f
        .db
        .prepare_hybrid_context(
            &req,
            &p,
            &situation(),
            &HybridOptions {
                profile_id: "fixture-profile".into(),
                dimensions: 2,
                required_mention: None,
            }
        )
        .is_err());
}

#[test]
fn empty_and_unmatched_mention_are_successful_empty_semantic_results() {
    let mut f = Fixture::new();
    for populated in [false, true] {
        if populated {
            capture(&mut f.db, "other", "approval elsewhere");
        }
        let prepared = prepare(
            &f.db,
            "approval",
            &policy(&f.db),
            populated.then_some("No Such Name"),
        );
        assert_eq!(prepared.readiness(), HybridReadiness::Empty);
        assert!(prepared.passages().is_empty());
        let result =
            f.db.finish_hybrid_context(&prepared, None, HybridMode::Dense)
                .unwrap();
        assert_eq!(result.effective_mode, HybridMode::Dense);
        assert_eq!(result.fallback, None);
        assert!(result.packet.records.is_empty());
    }
}

#[test]
fn vectors_reject_wrong_profile_shape_nonfinite_and_zero_norm() {
    let mut f = Fixture::new();
    capture(&mut f.db, "one", "original fixture");
    let prepared = prepare(&f.db, "original", &policy(&f.db), None);
    let mut v = vectors(&prepared, |_| vec![1., 0.]);
    v.profile_id = "different-same-width".into();
    assert!(f
        .db
        .finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
        .is_err());
    v.profile_id = "fixture-profile".into();
    for malformed in [
        vec![],
        vec![0.],
        vec![0., 0.],
        vec![f32::NAN, 1.],
        vec![1., f32::INFINITY],
    ] {
        v.query = malformed;
        assert!(f
            .db
            .finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
            .is_err());
    }
    v.query = vec![1., 0.];
    v.passages.clear();
    assert!(f
        .db
        .finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
        .is_err());
}

#[test]
fn finish_revalidates_payload_index_and_newly_authorized_corpus_members() {
    let mut f = Fixture::new();
    let visible = capture(&mut f.db, "visible", "blue cloth");
    let hidden =
        f.db.capture(Capture {
            source: source("hidden"),
            text: "some other material".into(),
            occurred_at: None,
            scopes: ["hidden".into()].into(),
            transcript: None,
        })
        .unwrap();
    let p = policy(&f.db);
    let prepared = prepare(&f.db, "blue", &p, None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    let conn = Memory::open(&f.dir.join("test.memory-prototype.sqlite"))
        .unwrap()
        .conn;
    assert!(f.db.conn.is_autocommit());
    conn.execute(
        "UPDATE revision SET payload=replace(payload,'blue cloth','blue silks') WHERE id=?1",
        [&visible.revision],
    )
    .unwrap();
    let changed =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
            .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    let Record::Evidence(e) = &changed.packet.records[0].record else {
        panic!()
    };
    assert_eq!(e.text, "blue silks");
    let prepared = prepare(&f.db, "blue", &p, None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    conn.execute(
        "UPDATE memory_fts SET text='blue changed' WHERE revision=?1",
        [&visible.revision],
    )
    .unwrap();
    assert_eq!(
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap()
            .fallback,
        Some(HybridFallback::CorpusChanged)
    );
    let prepared = prepare(&f.db, "blue", &p, None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    conn.execute("DELETE FROM scope WHERE revision=?1", [&hidden.revision])
        .unwrap();
    let changed =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    assert_eq!(refs(&changed), [&visible]);
}

#[test]
fn historical_revisions_survive_ordinary_edits_but_current_scope_and_forgetting_override() {
    let mut f = Fixture::new();
    let historical = capture(&mut f.db, "sample", "old mineral");
    let p = policy(&f.db);
    let prepared = prepare(&f.db, "mineral", &p, None);
    let v = vectors(&prepared, |_| vec![1., 0.]);
    capture(&mut f.db, "sample", "current mineral");
    let result =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(result.fallback, None);
    assert_eq!(refs(&result), [&historical]);
    f.db.capture(Capture {
        source: source("sample"),
        text: "current mineral".into(),
        occurred_at: None,
        scopes: ["new restricted scope".into()].into(),
        transcript: None,
    })
    .unwrap();
    let changed =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    assert!(changed.packet.records.is_empty());
    f.db.forget(&source("sample")).unwrap();
    let changed =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
            .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    assert!(changed.packet.records.is_empty() && changed.ranked_anchors.is_empty());
}

#[test]
fn semantic_anchor_returns_supported_relation_with_whole_dependencies_and_budget() {
    let mut f = Fixture::new();
    let source = capture(&mut f.db, "support", "A ramp allows the passage.");
    let other = capture(&mut f.db, "consulted", "Slope was verified.");
    let entity = f.db.entity("Synthetic ramp", Default::default()).unwrap();
    let relation =
        f.db.relate(RelationInput {
            predicate: "allows".into(),
            subject: entity.clone(),
            object: Target::Literal("passage".into()),
            support: Span {
                evidence: source.clone(),
                start: 0,
                end: 25,
            },
            consulted: vec![other.clone()],
            text: "Allows the passage.".into(),
            valid: Interval {
                from: Bound::Unbounded,
                to: Bound::Unbounded,
            },
            polarity: 1,
        })
        .unwrap();
    let mut p = policy(&f.db);
    p.recall.limit = 1;
    let prepared = prepare(&f.db, "accessibility", &p, None);
    let v = vectors(&prepared, |passage| {
        if passage.reference == source {
            vec![1., 0.]
        } else {
            vec![-1., 0.]
        }
    });
    let complete =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    for reference in [&source, &other, &relation, &entity] {
        assert!(refs(&complete).contains(&reference));
    }
    assert!(complete
        .packet
        .records
        .iter()
        .all(|hit| hit.private_data && hit.untrusted_input));
    p.recall.max_bytes = serde_json::to_vec(&complete.packet).unwrap().len() - 1;
    let prepared = prepare(&f.db, "accessibility", &p, None);
    let partial =
        f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
            .unwrap();
    assert_eq!(refs(&partial), [&source]);
    assert!(partial.packet.budget_omitted);
    assert!(serde_json::to_vec(&partial.packet).unwrap().len() <= p.recall.max_bytes);
}

#[test]
fn semantic_anchor_delivers_v8_concept_only_with_all_consulted_evidence() {
    let f = Fixture::new();
    let mut db =
        Memory::create_supported_situated(&f.dir.join("concept.memory-prototype.sqlite"), "test")
            .unwrap();
    let anchor = capture(&mut db, "gate", "Gate certification was pending.");
    let second = capture(&mut db, "track", "A track inspection was delayed.");
    let consulted_text = "A temporary waiver let the ordinary route proceed.";
    let consulted = capture(&mut db, "waiver", consulted_text);
    let concept_request = db
        .prepare_concept(
            &[anchor.clone(), second.clone(), consulted.clone()],
            &db.snapshot().unwrap(),
            &access(),
        )
        .unwrap();
    let draft = serde_json::json!({
        "name": "Redundancy",
        "principle": "Keep an alternate route when access is pending.",
        "applicability": "Routes gated by an independent check.",
        "support": [
            {"source_id": anchor.revision, "field": "", "quote": "certification was pending"},
            {"source_id": second.revision, "field": "", "quote": "inspection was delayed"}
        ],
        "counterexamples": []
    });
    let concept = db
        .retain_concept(concept_request, &draft.to_string(), &access(), None)
        .unwrap();

    let mut p = policy(&db);
    p.recall.limit = 1;
    let query = "redundancy";
    assert!(db
        .recall(
            &RecallQuery {
                text: query.into(),
                ..p.recall.clone()
            },
            &p.access
        )
        .unwrap()
        .is_empty());
    let prepared = prepare(&db, query, &p, None);
    assert_eq!(prepared.passages().len(), 3);
    let v = vectors(&prepared, |passage| {
        if passage.reference == anchor {
            vec![1., 0.]
        } else {
            vec![-1., 0.]
        }
    });
    let complete = db
        .finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
        .unwrap();
    assert_eq!(complete.effective_mode, HybridMode::Hybrid);
    assert_eq!(complete.ranked_anchors.len(), 1);
    let diagnostic = &complete.ranked_anchors[0];
    assert_eq!(diagnostic.reference, anchor);
    assert_eq!(diagnostic.rank, 1);
    assert_eq!(diagnostic.bm25, None);
    assert_eq!(diagnostic.cosine, Some(1.));
    assert_eq!(diagnostic.rrf, Some(1. / 61.));
    assert_eq!(complete.packet.records.len(), 4);
    for reference in [&anchor, &second, &consulted, &concept] {
        assert!(refs(&complete).contains(&reference));
    }

    let mut tight = p.clone();
    tight.recall.max_bytes = serde_json::to_vec(&complete.packet).unwrap().len() - 1;
    let limited = prepare(&db, query, &tight, None);
    let partial = db
        .finish_hybrid_context(&limited, Some(&v), HybridMode::Hybrid)
        .unwrap();
    assert_eq!(partial.effective_mode, HybridMode::Hybrid);
    assert_eq!(refs(&partial), [&anchor]);
    assert!(partial.packet.budget_omitted);
    assert_eq!(partial.ranked_anchors.len(), 1);
    assert_eq!(partial.ranked_anchors[0].reference, anchor);
    assert!(serde_json::to_vec(&partial.packet).unwrap().len() <= tight.recall.max_bytes);

    // Narrow the current source without deleting the original consulted
    // revision or the concept. The original snapshot cannot bypass the change.
    db.capture(Capture {
        source: source("waiver"),
        text: consulted_text.into(),
        occurred_at: None,
        scopes: ["restricted".into()].into(),
        transcript: None,
    })
    .unwrap();
    assert!(matches!(
        lookup(&db.conn, &concept).unwrap(),
        Some(Record::Concept(_))
    ));
    let changed = db
        .finish_hybrid_context(&prepared, Some(&v), HybridMode::Hybrid)
        .unwrap();
    assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
    assert_eq!(changed.effective_mode, HybridMode::Lexical);
    assert!(changed.packet.records.is_empty());
    assert!(changed.ranked_anchors.is_empty());

    let fresh = prepare(&db, query, &p, None);
    assert_eq!(fresh.passages().len(), 2);
    let fresh_vectors = vectors(&fresh, |passage| {
        if passage.reference == anchor {
            vec![1., 0.]
        } else {
            vec![-1., 0.]
        }
    });
    let withheld = db
        .finish_hybrid_context(&fresh, Some(&fresh_vectors), HybridMode::Dense)
        .unwrap();
    assert_eq!(withheld.effective_mode, HybridMode::Dense);
    assert_eq!(withheld.fallback, None);
    assert_eq!(refs(&withheld), [&anchor]);
    assert_eq!(withheld.ranked_anchors.len(), 1);
    assert_eq!(withheld.ranked_anchors[0].reference, anchor);
}

struct Guard {
    calls: AtomicUsize,
    withdraw_at: usize,
}
impl host_capture::ExternalSourceGuard for Guard {
    fn check(&self, _: &host_capture::ExternalBinding) -> host_capture::SourceStatus {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.withdraw_at {
            host_capture::SourceStatus::Checked
        } else {
            host_capture::SourceStatus::Withheld
        }
    }
}

#[test]
fn changed_external_authority_rebuilds_corpus_and_final_check_uses_a_fresh_guard() {
    for withdraw_at in [1, 2] {
        let mut f = Fixture::new();
        let guarded = capture(&mut f.db, "guarded", "literal guarded fixture");
        let lexical = capture(&mut f.db, "independent", "literal ordinary fixture");
        f.db.conn.execute_batch("CREATE TABLE external_binding(revision TEXT PRIMARY KEY,authority TEXT,locator TEXT,binding_revision TEXT);").unwrap();
        f.db.conn
            .execute(
                "INSERT INTO external_binding VALUES (?1,'host-v1',?2,?2)",
                params![guarded.revision, "a".repeat(64)],
            )
            .unwrap();
        let guard = Arc::new(Guard {
            calls: AtomicUsize::new(0),
            withdraw_at,
        });
        f.db.source_guard = Some(guard.clone());
        let mut p = policy(&f.db);
        p.recall.limit = 1;
        let prepared = prepare(&f.db, "literal", &p, None);
        assert_eq!(prepared.passages().len(), 2);
        let v = vectors(&prepared, |passage| {
            if passage.reference == guarded {
                vec![1., 0.]
            } else {
                vec![-1., 0.]
            }
        });
        let changed =
            f.db.finish_hybrid_context(&prepared, Some(&v), HybridMode::Dense)
                .unwrap();
        assert_eq!(changed.fallback, Some(HybridFallback::CorpusChanged));
        assert_eq!(refs(&changed), [&lexical]);
        assert!(changed
            .ranked_anchors
            .iter()
            .all(|a| a.reference != guarded));
    }
}
