use super::*;
use std::{cell::RefCell, fs, future::ready, path::PathBuf, time::Duration};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hybrid-workflow-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn db(&self) -> PathBuf {
        self.0.join("workflow.memory-prototype.sqlite")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn options() -> EvaluationOptions {
    EvaluationOptions {
        endpoint: "http://127.0.0.1:1".into(),
        expected_alias: "authored-encoder".into(),
        artifact_revision: "authored-revision".into(),
        dimensions: 2,
        pooling: "last".into(),
        normalization: "l2".into(),
    }
}

#[test]
fn profile_and_fixture_match_the_pre_model_freeze() {
    let profile = EvaluationOptions {
        expected_alias: "harrier-oss-v1-0.6b".into(),
        artifact_revision:
            "sha256:f3af313d8f58b59f282bdf7299a6ad738efdcb0644e7a98ee6deb106336e717a".into(),
        dimensions: 1024,
        ..options()
    }
    .profile()
    .unwrap();
    assert_eq!(
        profile.id(),
        "a73d53ee86ac813f5adcdd91af6deb47efb8b33cc4a6f41a92a22ae47991b04e"
    );
    assert_eq!(
        fixture_sha256(),
        "43cd0d770c07829583af7be4a39a6bbc969f7de86d188185eb73ecd99a006d10"
    );
}

#[tokio::test]
async fn shares_checked_inputs_and_reports_the_frozen_cohort() {
    let fixture = Fixture::new();
    let options = options();
    let profile = options.profile().unwrap();
    let submitted = RefCell::new(Vec::new());
    let report = evaluate(&fixture.db(), &options, &profile, |inputs| {
        let vectors = inputs
            .iter()
            .map(|input| {
                if input.task == Task::Query || input.text.starts_with("A rubber gasket") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect();
        submitted.borrow_mut().push(inputs);
        ready(Ok(EmbeddedBatch {
            profile_id: profile.id().to_owned(),
            server_model: Some("authored-encoder".into()),
            vectors,
            attempts: 3,
            request_bytes: 100,
            response_bytes: 200,
        }))
    })
    .await
    .unwrap();

    assert_eq!(report.cases.len(), 18);
    for summary in &report.modes {
        assert_eq!(summary.planned, 18);
        assert_eq!(summary.completed, 18);
        assert_eq!(summary.planned_nonempty_gold, 16);
        assert_eq!(summary.planned_empty_gold, 2);
        assert_eq!(summary.unauthorized_returns, 0);
        assert_eq!(summary.literal_violations, 0);
    }
    let submitted = submitted.into_inner();
    assert_eq!(submitted.len(), 17, "one mixed operation per nonempty pool");
    let inputs_for = |query: &str| {
        submitted
            .iter()
            .find(|inputs| inputs[0].text == query)
            .unwrap()
    };
    let first = inputs_for("How should outdoor electronics stay dry through downpours?");
    assert_eq!(first[0].task, Task::Query);
    assert_eq!(first.len(), 38);
    assert!(first
        .iter()
        .skip(1)
        .all(|input| input.task == Task::Passage));
    assert!(first[1..].iter().any(|input| {
        input.text == "A rubber gasket seats inside the enclosure lid and blocks rainwater ingress."
    }));
    assert!(!first[1..]
        .iter()
        .any(|input| input.text.contains("compartment 17")));
    assert_eq!(inputs_for("Q7F4 approved mounting fastener").len(), 3);
    assert_eq!(
        inputs_for("What does the October acceptance entry give as the L6 vibration limit?").len(),
        37
    );
    assert_eq!(
        inputs_for("What Q1 relay trip setting was known at the earlier snapshot?").len(),
        37
    );

    let first_case = &report.cases[0];
    assert_eq!(first_case.id, "P01");
    assert_eq!(first_case.shared.embedding.inputs, Some(38));
    assert_eq!(first_case.shared.embedding.attempts, Some(3));
    assert_eq!(first_case.treatments[0].coverage, Some(0.0));
    for semantic in &first_case.treatments[1..] {
        assert_eq!(semantic.coverage, Some(1.0));
        assert!(semantic.derived_wall_micros.is_some());
    }
    assert!(first_case.treatments[0].derived_wall_micros.is_none());
    let literal = report.cases.iter().find(|case| case.id == "X01").unwrap();
    assert!(literal
        .treatments
        .iter()
        .all(|arm| arm.ranked_anchors.len() == 1));
    assert!(report
        .cases
        .iter()
        .flat_map(|case| &case.treatments)
        .all(|arm| { arm.packet_bytes.is_some_and(|bytes| bytes <= 8192) }));
    let empty = report.cases.last().unwrap();
    assert_eq!(empty.id, "E02");
    assert_eq!(empty.shared.embedding.attempts, Some(0));
    for treatment in &empty.treatments {
        assert_eq!(treatment.coverage, None);
        assert_eq!(treatment.complete, None);
        assert_eq!(treatment.reciprocal_rank, None);
        assert_eq!(treatment.returned_records, Some(0));
    }
}

fn capture(memory: &mut Memory, id: &str, text: &str) -> RevisionRef {
    memory
        .capture(Capture {
            source: fixture::source_key(id),
            text: text.into(),
            occurred_at: None,
            scopes: fixture::access().scopes,
            transcript: None,
        })
        .unwrap()
}

fn selection(memory: &Memory) -> Selection {
    Selection {
        request: ContextRequest {
            query: "red housing".into(),
            bindings: vec![],
        },
        policy: ContextPolicy {
            access: fixture::access(),
            recall: RecallQuery {
                text: String::new(),
                known_at: memory.snapshot().unwrap(),
                account: None,
                session_id: None,
                occurred_after: None,
                occurred_before: None,
                limit: 4,
                neighbors: 0,
                max_bytes: 8192,
            },
            valid_at: None,
        },
        required_mention: None,
    }
}

fn situation() -> Situation {
    Situation {
        event: "Review test records".into(),
        goal: "Select evidence".into(),
        needs: None,
    }
}

#[tokio::test]
async fn timeout_returns_fresh_fallback_without_an_open_sqlite_transaction() {
    let fixture = Fixture::new();
    let db = fixture.db();
    let mut memory = Memory::create(&db, "synthetic-demo").unwrap();
    let gold = capture(&mut memory, "gold", "The red housing surrounds the pump.");
    let before = fs::read(&db).unwrap();
    let reader = rusqlite::Connection::open(&db).unwrap();
    let other = rusqlite::Connection::open(&db).unwrap();
    other.execute_batch("PRAGMA busy_timeout=0").unwrap();
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM revision", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let held = other.execute_batch("BEGIN EXCLUSIVE").unwrap_err();
    assert_eq!(
        held.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    reader.execute_batch("ROLLBACK").unwrap();

    let profile = options().profile().unwrap();
    let comparison = compare(
        &memory,
        &selection(&memory),
        &situation(),
        &profile,
        |_| async {
            other.execute_batch("BEGIN EXCLUSIVE; ROLLBACK").unwrap();
            assert_eq!(fs::read(&db).unwrap(), before);
            tokio::time::timeout(Duration::from_millis(5), std::future::pending::<()>()).await?;
            unreachable!()
        },
    )
    .await;
    assert_eq!(fs::read(&db).unwrap(), before);
    assert_eq!(comparison.shared.embedding.attempts, None);
    assert_eq!(comparison.shared.embedding.request_bytes, None);
    assert_eq!(comparison.shared.embedding.response_bytes, None);
    assert_eq!(comparison.shared.embedding.inputs, Some(2));
    let (_, treatments) = comparison.grade(&[gold], &[], None).unwrap();
    assert_eq!(treatments[0].coverage, Some(1.0));
    for arm in &treatments[1..] {
        assert_eq!(arm.status, TreatmentStatus::Fallback);
        assert_eq!(arm.fallback, Some(HybridFallback::EmbeddingsUnavailable));
        assert_eq!(arm.effective_mode, Some(HybridMode::Lexical));
        assert_eq!(arm.coverage, Some(0.0));
        assert_eq!(arm.complete, Some(false));
        assert_eq!(arm.reciprocal_rank, Some(0.0));
        assert_eq!(arm.returned_records, Some(1));
    }
}

#[tokio::test]
async fn corpus_bound_fallback_never_invokes_the_embedding_operation() {
    let fixture = Fixture::new();
    let mut memory = Memory::create(&fixture.db(), "synthetic-demo").unwrap();
    let gold = capture(&mut memory, "gold", "Red housing parts");
    for row in 0..256 {
        capture(&mut memory, &format!("unrelated-{row}"), "Blue gaskets");
    }
    let comparison = compare(
        &memory,
        &selection(&memory),
        &situation(),
        &options().profile().unwrap(),
        |_| -> std::future::Ready<Result<EmbeddedBatch>> {
            panic!("oversized corpus must never call an embedder");
        },
    )
    .await;
    assert_eq!(comparison.shared.embedding.inputs, Some(0));
    assert_eq!(comparison.shared.embedding.attempts, Some(0));
    let (_, treatments) = comparison.grade(&[gold], &[], None).unwrap();
    assert_eq!(treatments[0].coverage, Some(1.0));
    for arm in &treatments[1..] {
        assert_eq!(arm.status, TreatmentStatus::Fallback);
        assert_eq!(arm.fallback, Some(HybridFallback::CorpusBounds));
        assert_eq!(arm.coverage, Some(0.0));
        assert_eq!(arm.returned_records, Some(1));
    }
}

#[test]
fn scoring_uses_delivered_anchors_for_rank_and_all_evidence_for_coverage() {
    let fixture = Fixture::new();
    let mut memory = Memory::create(&fixture.db(), "synthetic-demo").unwrap();
    let neighbor = capture(&mut memory, "neighbor", "Red housing material");
    let gold = capture(&mut memory, "gold", "Red housing gasket");
    let selection = selection(&memory);
    let mut result = memory
        .lexical_hybrid_context(&selection.request, &selection.policy, &situation(), None)
        .unwrap();
    assert_eq!(result.packet.records.len(), 2);
    result.ranked_anchors.retain(|a| a.reference == gold);
    result.ranked_anchors[0].rank = 3;
    result.ranked_anchors.insert(
        0,
        RankedAnchor {
            reference: RevisionRef {
                object: "withheld".into(),
                revision: "withheld".into(),
            },
            rank: 1,
            bm25: None,
            cosine: None,
            rrf: None,
        },
    );
    let packet = serde_json::to_value(&result.packet).unwrap();
    let evidence_bytes: usize = packet["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| serde_json::to_vec(record).unwrap().len())
        .sum();
    let arm = score(
        TimedTreatment {
            mode: HybridMode::Lexical,
            measured_micros: Some(5),
            result: Ok(result),
        },
        0,
        &[gold, neighbor.clone()],
        &[neighbor],
        Some("missing123"),
    )
    .unwrap();
    assert_eq!(arm.reciprocal_rank, Some(1.0));
    assert_eq!(arm.coverage, Some(1.0));
    assert_eq!(arm.complete, Some(true));
    assert_eq!(arm.ranked_anchors.len(), 1);
    assert_eq!(arm.unauthorized_returns, Some(1));
    assert_eq!(arm.literal_violations, Some(1));
    assert_eq!(arm.evidence_bytes, Some(evidence_bytes));
}

#[tokio::test]
async fn fallback_and_errors_remain_in_the_planned_nonempty_denominator() {
    let fixture = Fixture::new();
    let options = options();
    let profile = options.profile().unwrap();
    let report = evaluate(&fixture.db(), &options, &profile, |inputs| {
        let query = &inputs[0].text;
        if query.starts_with("How can a fragile vial") {
            return ready(Err(anyhow::anyhow!("authored transport failure")));
        }
        let profile_id = if query.starts_with("How should outdoor electronics") {
            profile.id()
        } else {
            "wrong-profile"
        };
        let vectors = inputs
            .iter()
            .map(|input| {
                if input.task == Task::Query || input.text.starts_with("A rubber gasket") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect();
        ready(Ok(EmbeddedBatch {
            profile_id: profile_id.into(),
            server_model: Some("authored-encoder".into()),
            vectors,
            attempts: 3,
            request_bytes: 100,
            response_bytes: 200,
        }))
    })
    .await
    .unwrap();
    for semantic in &report.modes[1..] {
        assert_eq!(semantic.planned, 18);
        assert_eq!(semantic.completed, 2);
        assert_eq!(semantic.fallback, 1);
        assert_eq!(semantic.error, 15);
        assert_eq!(semantic.planned_nonempty_gold, 16);
        assert_eq!(semantic.complete, 1);
        assert_eq!(semantic.mean_coverage, Some(1.0 / 16.0));
        assert_eq!(semantic.mean_reciprocal_rank, Some(1.0 / 16.0));
    }
    for treatment in &report.cases[2].treatments[1..] {
        assert_eq!(treatment.status, TreatmentStatus::Error);
        assert_eq!(treatment.coverage, Some(0.0));
        assert_eq!(treatment.effective_mode, None);
        assert_eq!(treatment.returned_records, None);
    }
}
