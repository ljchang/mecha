use super::{
    context::*,
    transcript_tests::{access, log, Fixture},
    *,
};

fn policy(db: &Memory) -> ContextPolicy {
    let mut recall = super::recall_tests::query(db, "launch");
    recall.account = Some("a".into());
    recall.neighbors = 0;
    recall.limit = 1;
    ContextPolicy {
        access: access(),
        recall,
        valid_at: Some("2026-06-01T00:00:00Z".into()),
    }
}
fn situation() -> Situation {
    Situation {
        event: "Choose a launch route".into(),
        goal: "task:launch".into(),
        needs: None,
    }
}
fn request() -> ContextRequest {
    ContextRequest {
        query: "launch".into(),
        bindings: vec![],
    }
}

#[test]
fn context_exact_quote_routes_followup_without_composing_a_name() {
    let mut f = Fixture::new();
    let imported =
        f.db.import_session_jsonl(
            log(
                "s",
                &[
                    "launch: use Ilex or Quill pipeline",
                    "Ilex budget is 7 credits",
                ],
            )
            .as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap();
    let p = policy(&f.db);
    let initial = f.db.context(&request(), &p, &situation()).unwrap();
    assert_eq!(initial.records.len(), 1);
    let source = imported.records[1].clone();
    let mut req = request();
    req.bindings.push(SourceSelection {
        source: source.clone(),
        field: "/content/0/text".into(),
        quote: "Ilex".into(),
        query: "budget".into(),
    });
    let packet = f.db.context(&req, &p, &situation()).unwrap();
    assert!(packet
        .records
        .iter()
        .any(|h| h.reference == imported.records[2]));
    assert_eq!(packet.bindings[0].mention, "Ilex");
    assert_eq!(packet.bindings[0].start, 12);
    assert_eq!(packet.bindings[0].end, 16);
    assert!(packet
        .records
        .iter()
        .all(|h| h.private_data && h.untrusted_input));
    req.bindings[0].quote = "Ilex pipeline".into();
    assert!(f.db.context(&req, &p, &situation()).is_err());
    req.bindings[0].quote = "Ilex".into();
    req.bindings[0].field = "/source/account".into();
    assert!(f.db.context(&req, &p, &situation()).is_err());
    req.bindings[0].field = "/content/0/text".into();
    f.db.forget(&transcript_source("a", "s", 2)).unwrap();
    assert!(f.db.context(&req, &p, &situation()).is_err());
}

#[test]
fn context_budget_includes_envelope_and_read_only_handle_cannot_write() {
    let mut f = Fixture::new();
    f.db.import_session_jsonl(
        log("s", &[&format!("launch {}", "big ".repeat(500))]).as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    let mut p = policy(&f.db);
    p.recall.max_bytes = 1200;
    let mut reader = Memory::open_read_only(&f.dir.join("test.memory-prototype.sqlite")).unwrap();
    assert!(reader.entity("must not write", Default::default()).is_err());
    let packet = reader.context(&request(), &p, &situation()).unwrap();
    assert!(serde_json::to_vec(&packet).unwrap().len() <= 1200);
    assert!(packet.records.is_empty());
    assert!(packet.incomplete);
    assert!(
        packet.budget_omitted,
        "oversized eligible recall was omitted before assembly"
    );
    assert!(packet.needs_unavailable);
    p.recall.max_bytes = 5;
    assert!(reader.context(&request(), &p, &situation()).is_err());
}

#[test]
fn context_relations_keep_provenance_time_and_all_dependency_scopes() {
    let mut f = Fixture::new();
    let capture = |account: &str, id: &str| Capture {
        source: SourceKey {
            connector: "fixture".into(),
            account: account.into(),
            id: id.into(),
        },
        text: "launch choice is Ilex".into(),
        occurred_at: None,
        scopes: access().scopes,
        transcript: None,
    };
    let source = f.db.capture(capture("a", "source")).unwrap();
    let hidden = f.db.capture(capture("b", "hidden")).unwrap();
    let entity = f.db.entity("Ilex", Default::default()).unwrap();
    let mut relation = RelationInput {
        predicate: "choice".into(),
        subject: entity,
        object: Target::Literal("Ilex".into()),
        support: Span {
            evidence: source.clone(),
            start: 0,
            end: 20,
        },
        consulted: vec![],
        text: "choice is Ilex".into(),
        valid: Interval {
            from: Bound::Unknown,
            to: Bound::Known("2026-07-01T00:00:00Z".into()),
        },
        polarity: 1,
    };
    let visible = f.db.relate(relation.clone()).unwrap();
    relation.consulted.push(hidden);
    let cross_account = f.db.relate(relation).unwrap();
    let mut p = policy(&f.db);
    let packet = f.db.context(&request(), &p, &situation()).unwrap();
    assert!(packet
        .records
        .iter()
        .any(|h| h.reference == visible && h.possible_time));
    assert!(!packet.records.iter().any(|h| h.reference == cross_account));
    assert!(packet.records.iter().any(|h| h.reference == source));
    p.valid_at = Some("2026-07-01T00:00:00Z".into());
    assert!(!f
        .db
        .context(&request(), &p, &situation())
        .unwrap()
        .records
        .iter()
        .any(|h| h.reference == visible));
    p.valid_at = None;
    p.recall.occurred_after = Some("2026-01-01T00:00:00Z".into());
    assert!(f
        .db
        .context(&request(), &p, &situation())
        .unwrap()
        .records
        .is_empty());
}

#[test]
fn context_source_reference_rejects_unknown_fields_without_changing_record_decoder() {
    let reference = serde_json::json!({"object":"o","revision":"r","unexpected":true});
    assert!(
        serde_json::from_value::<RevisionRef>(reference.clone()).is_ok(),
        "historical record decoder stays compatible"
    );
    let request = serde_json::json!({"query":"launch","bindings":[{"source":reference,"field":"","quote":"Ilex","query":"cost"}]});
    assert!(
        serde_json::from_value::<ContextRequest>(request).is_err(),
        "new context boundary must enforce its schema"
    );
}

#[test]
fn context_retrieves_provisional_concept_for_new_words_with_complete_scoped_provenance() {
    let mut f = Fixture::new();
    let texts = [
        "A permit was pending so a backup route was reserved.",
        "An inspection was missing so a different departure path was kept.",
        "An emergency waiver allowed the original path.",
    ];
    let refs: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            f.db.import_session_jsonl(
                log(&format!("concept-{i}"), &[text]).as_bytes(),
                "a",
                access().scopes,
            )
            .unwrap()
            .records[1]
                .clone()
        })
        .collect();
    let before = f.db.snapshot().unwrap();
    let proposal = serde_json::json!({
        "name":"Approval bottleneck", "principle":"An unfinished prerequisite may call for a feasible backup plan.",
        "applicability":"Plans blocked by an external prerequisite; an explicit waiver can be an exception.",
        "support":[{"source_id":refs[0].revision,"field":"/content/0/text","quote":"permit was pending"},{"source_id":refs[1].revision,"field":"/content/0/text","quote":"inspection was missing"}],
        "counterexamples":[{"source_id":refs[2].revision,"field":"/content/0/text","quote":"emergency waiver"}]
    });
    let receipt = f.db.prepare_concept(&refs, &before, &access()).unwrap();
    let concept =
        f.db.retain_concept(receipt, &proposal.to_string(), &access(), None)
            .unwrap();
    let req = ContextRequest {
        query: "bottleneck".into(),
        bindings: vec![],
    };
    let mut p = policy(&f.db);
    let packet = f.db.context(&req, &p, &situation()).unwrap();
    assert!(
        packet
            .records
            .iter()
            .any(|h| h.reference == concept && matches!(h.record, Record::Concept(_))),
        "concept words should retrieve knowledge even when no original episode matches them"
    );
    for source in &refs {
        assert!(packet.records.iter().any(|h| &h.reference == source));
    }
    assert!(packet
        .records
        .iter()
        .all(|h| h.private_data && h.untrusted_input));
    assert!(serde_json::to_vec(&packet).unwrap().len() <= p.recall.max_bytes);
    p.recall.known_at = before;
    assert!(
        f.db.context(&req, &p, &situation())
            .unwrap()
            .records
            .is_empty(),
        "no future concept in historical context"
    );
    p.recall.known_at = f.db.snapshot().unwrap();
    p.recall.session_id = Some("concept-0".into());
    assert!(
        f.db.context(&req, &p, &situation())
            .unwrap()
            .records
            .is_empty(),
        "one eligible support cannot admit cross-session dependencies"
    );
    p.recall.session_id = None;
    p.recall.max_bytes = 1200;
    let small = f.db.context(&req, &p, &situation()).unwrap();
    assert!(
        small.records.is_empty() && small.budget_omitted,
        "omit entire concept bundle when it does not fit"
    );
    p.recall.max_bytes = 20_000;
    f.db.conn
        .execute(
            "INSERT INTO scope VALUES (?1, 'restricted')",
            [&refs[2].revision],
        )
        .unwrap();
    assert!(
        f.db.context(&req, &p, &situation())
            .unwrap()
            .records
            .is_empty(),
        "counterexample permissions govern the whole concept"
    );
    f.db.forget(&transcript_source("a", "concept-2", 2))
        .unwrap();
    assert!(
        f.db.context(&req, &p, &situation())
            .unwrap()
            .records
            .is_empty(),
        "forget overrides the historical snapshot"
    );
}
