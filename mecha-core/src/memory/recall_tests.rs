use super::{
    transcript_tests::{access, log, Fixture},
    *,
};

#[test]
fn future_and_ineligible_records_cannot_change_the_ranked_evidence() {
    let mut f = Fixture::new();
    f.db.import_session_jsonl(
        log("base", &["alpha", "beta"]).as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    let mut q = query(&f.db, "alpha beta");
    q.account = Some("a".into());
    q.session_id = Some("base".into());
    q.limit = 1;
    q.neighbors = 0;
    let before = f.db.recall(&q, &access()).unwrap();
    f.db.import_session_jsonl(
        log("future", &["beta"; 30]).as_bytes(),
        "a",
        ["secret".into()].into(),
    )
    .unwrap();
    let after = f.db.recall(&q, &access()).unwrap();
    assert_eq!(before[0].anchor.reference, after[0].anchor.reference);
    assert_eq!(before[0].bm25, after[0].bm25);
    q.known_at = f.db.snapshot().unwrap();
    let latest = f.db.recall(&q, &access()).unwrap();
    assert_eq!(before[0].anchor.reference, latest[0].anchor.reference);
    assert_eq!(before[0].bm25, latest[0].bm25);
}

pub(super) fn query(db: &Memory, text: &str) -> RecallQuery {
    RecallQuery {
        text: text.into(),
        known_at: db.snapshot().unwrap(),
        account: None,
        session_id: None,
        occurred_after: None,
        occurred_before: None,
        limit: 5,
        neighbors: 1,
        max_bytes: 20_000,
    }
}

#[test]
fn lexical_recall_has_exact_source_fields_and_scoped_ordered_context() {
    let mut f = Fixture::new();
    let raw = log(
        "s1",
        &[
            "The launch needs a permit.",
            "The permit is delayed.",
            "Ask for an alternative.",
        ],
    );
    f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
        .unwrap();
    f.db.import_session_jsonl(
        log("s1", &["delayed hidden neighbor"]).as_bytes(),
        "b",
        ["secret".into()].into(),
    )
    .unwrap();
    let mut q = query(&f.db, "delayed");
    q.limit = 1;
    let hits = f.db.recall(&q, &access()).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].anchor.private_data && hits[0].anchor.untrusted_input);
    let Record::Evidence(e) = &hits[0].anchor.record else {
        panic!()
    };
    assert_eq!(e.source.account, "a");
    assert_eq!(e.transcript.as_ref().unwrap().line, 3);
    assert_eq!(
        e.transcript.as_ref().unwrap().fields[0].text,
        "The permit is delayed."
    );
    let lines: Vec<_> = hits[0]
        .context
        .iter()
        .map(|h| {
            let Record::Evidence(e) = &h.record else {
                panic!()
            };
            assert_eq!(e.source.account, "a");
            e.transcript.as_ref().unwrap().line
        })
        .collect();
    assert_eq!(lines, vec![2, 4]);
    let value: serde_json::Value = serde_json::from_str(&e.text).unwrap();
    let field = &e.transcript.as_ref().unwrap().fields[0];
    assert_eq!(
        value.pointer(&field.pointer).unwrap().as_str(),
        Some(field.text.as_str())
    );
}

#[test]
fn eligibility_precedes_top_k_and_expansion_checks_each_neighbor() {
    let mut f = Fixture::new();
    let report =
        f.db.import_session_jsonl(
            log("s1", &["rareterm", "private nearby"]).as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap();
    f.db.conn
        .execute(
            "INSERT INTO scope VALUES (?1,'secret')",
            [&report.records[2].revision],
        )
        .unwrap();
    for n in 0..8 {
        f.db.import_session_jsonl(
            log(&format!("hidden{n}"), &["rareterm"]).as_bytes(),
            "a",
            ["secret".into()].into(),
        )
        .unwrap();
    }
    let mut q = query(&f.db, "rareterm");
    q.limit = 1;
    let hits = f.db.recall(&q, &access()).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].anchor.reference, report.records[1]);
    assert!(hits[0].context.is_empty());
    f.db.forget(&transcript_source("a", "s1", 2)).unwrap();
    assert!(f.db.recall(&q, &access()).unwrap().is_empty());
}

#[test]
fn occurrence_filters_do_not_treat_session_start_as_message_time() {
    let mut f = Fixture::new();
    let raw = format!(
        "{}{}\n",
        log("s1", &["deadline unknown time"]),
        r#"{"record":"message","role":"user","timestamp":"2026-02-01T12:00:00-05:00","content":[{"type":"text","text":"deadline known time"}]}"#
    );
    let report =
        f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
            .unwrap();
    let mut q = query(&f.db, "deadline");
    q.occurred_after = Some("2026-02-01T17:00:00Z".into());
    q.occurred_before = Some("2026-02-02T00:00:00Z".into());
    let hits = f.db.recall(&q, &access()).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].anchor.reference, report.records[2]);
    assert!(hits[0].context.is_empty());
    q.occurred_before = q.occurred_after.clone();
    assert!(f.db.recall(&q, &access()).is_err());
    q.occurred_before = Some("2026-02-03T00:00:00Z".into());
    q.account = Some("other".into());
    assert!(f.db.recall(&q, &access()).unwrap().is_empty());
}

#[test]
fn ranking_uses_lexical_relevance_and_budget_never_cuts_source_records() {
    let mut f = Fixture::new();
    let r =
        f.db.import_session_jsonl(
            log("s1", &["café café café"]).as_bytes(),
            "a",
            access().scopes,
        )
        .unwrap();
    let long = format!("café {}", "unrelated ".repeat(100));
    f.db.import_session_jsonl(log("s2", &[&long]).as_bytes(), "a", access().scopes)
        .unwrap();
    let mut q = query(&f.db, "café");
    q.neighbors = 0;
    let hits = f.db.recall(&q, &access()).unwrap();
    assert_eq!(hits[0].anchor.reference, r.records[1]);
    q.max_bytes = serde_json::to_vec(&hits[0].anchor).unwrap().len();
    assert_eq!(f.db.recall(&q, &access()).unwrap().len(), 1);
    q.max_bytes -= 1;
    assert!(f.db.recall(&q, &access()).unwrap().is_empty());
    q.text = "!!!".into();
    assert!(f.db.recall(&q, &access()).is_err());
}

#[test]
fn old_snapshot_expansion_excludes_later_appends_and_obeys_current_restrictions() {
    let mut f = Fixture::new();
    let raw = log("s1", &["anchor", "first neighbor"]);
    f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
        .unwrap();
    let mut q = query(&f.db, "anchor");
    q.neighbors = 3;
    f.db.import_session_jsonl(
        log("s1", &["anchor", "first neighbor", "future neighbor"]).as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    assert_eq!(f.db.recall(&q, &access()).unwrap()[0].context.len(), 1);
    f.db.import_session_jsonl(
        log("s1", &["anchor", "first neighbor", "future neighbor"]).as_bytes(),
        "a",
        ["secret".into()].into(),
    )
    .unwrap();
    assert!(f.db.recall(&q, &access()).unwrap().is_empty());
}

#[test]
fn required_mention_filters_before_top_k_without_stemming_names() {
    let mut f = Fixture::new();
    f.db.import_session_jsonl(
        log(
            "names",
            &[
                "The Bank Room seats six and is quiet.",
                "The Banks Room seats twelve.",
                "Bank has another room.",
                "Room capacity capacity capacity capacity.",
            ],
        )
        .as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    let mut q = query(&f.db, "room capacity");
    q.limit = 1;
    q.neighbors = 0;
    let hits = f.db.recall_mention(&q, "Bank Room", &access()).unwrap();
    let Record::Evidence(e) = &hits[0].anchor.record else {
        panic!()
    };
    assert_eq!(e.transcript.as_ref().unwrap().line, 2);
    assert!(hits[0].anchor.private_data && hits[0].anchor.untrusted_input);
    // Literal syntax cannot turn a required name into an OR expression.
    assert!(f
        .db
        .recall_mention(&q, "Bank OR Banks", &access())
        .unwrap()
        .is_empty());
    assert!(f
        .db
        .recall_mention(&q, "Absent", &access())
        .unwrap()
        .is_empty());
    for bad in [
        "".to_owned(),
        "!!!".into(),
        "x".repeat(257),
        "x ".repeat(33),
    ] {
        assert!(f.db.recall_mention(&q, &bad, &access()).is_err());
    }
    q.max_bytes = 1;
    assert!(f
        .db
        .recall_mention(&q, "Bank Room", &access())
        .unwrap()
        .is_empty());
}

#[test]
fn mention_gate_preserves_historical_scope_source_and_time_filters() {
    let mut f = Fixture::new();
    let raw = log("s1", &["Vireo seats six", "Nearby filler"]);
    f.db.import_session_jsonl(raw.as_bytes(), "a", access().scopes)
        .unwrap();
    let mut q = query(&f.db, "Vireo capacity");
    q.account = Some("a".into());
    q.session_id = Some("s1".into());
    let before = f.db.recall_mention(&q, "Vireo", &access()).unwrap();
    assert_eq!(before.len(), 1);
    f.db.import_session_jsonl(
        log("s1", &["Vireo capacity"]).as_bytes(),
        "b",
        access().scopes,
    )
    .unwrap();
    f.db.import_session_jsonl(
        log("s2", &["Vireo capacity"]).as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    let after = f.db.recall_mention(&q, "Vireo", &access()).unwrap();
    assert_eq!(before[0].anchor.reference, after[0].anchor.reference);
    assert_eq!(before[0].bm25, after[0].bm25);
    q.occurred_after = Some("2100-01-01T00:00:00Z".into());
    assert!(f
        .db
        .recall_mention(&q, "Vireo", &access())
        .unwrap()
        .is_empty());
    q.occurred_after = None;
    f.db.import_session_jsonl(raw.as_bytes(), "a", ["secret".into()].into())
        .unwrap();
    assert!(f
        .db
        .recall_mention(&q, "Vireo", &access())
        .unwrap()
        .is_empty());
}

#[test]
fn required_mention_preserves_combining_marks_and_quotes_literal_phrases() {
    let mut f = Fixture::new();
    f.db.import_session_jsonl(
        log(
            "unicode",
            &[
                "Sa\u{0303}o Bento seats six.",
                "São Bento seats twelve.",
                "North \"Annex\" seats eight.",
            ],
        )
        .as_bytes(),
        "a",
        access().scopes,
    )
    .unwrap();
    let mut q = query(&f.db, "capacity");
    q.neighbors = 0;
    for name in ["Sa\u{0303}o Bento", "São Bento"] {
        assert_eq!(f.db.recall_mention(&q, name, &access()).unwrap().len(), 2);
    }
    let hits =
        f.db.recall_mention(&q, "North \"Annex\"", &access())
            .unwrap();
    assert_eq!(hits.len(), 1);
    let Record::Evidence(e) = &hits[0].anchor.record else {
        panic!()
    };
    assert_eq!(e.transcript.as_ref().unwrap().line, 4);
}
