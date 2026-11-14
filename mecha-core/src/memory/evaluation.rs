//! Fixed synthetic retrieval experiment. Gold evidence never enters recall's
//! input. Oracle coverage is a diagnostic, not an answer or reasoning score.
use super::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Instant,
};

#[derive(Deserialize)]
struct Dataset {
    version: u32,
    sessions: Vec<Session>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Session {
    account: String,
    jsonl: String,
}
#[derive(Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub question: String,
    pub query: String,
    pub account: String,
    pub session_id: String,
    pub goal: Option<String>,
    pub homeostatic_context: Option<String>,
    pub occurred_after: Option<String>,
    pub occurred_before: Option<String>,
    pub expected_lines: Vec<usize>,
}
#[derive(Serialize)]
pub struct CaseResult {
    pub case: Case,
    pub baseline: Vec<RecallHit>,
    pub oracle: Vec<Hit>,
    pub coverage: f64,
    pub complete: bool,
    pub retrieval_micros: u128,
    pub evidence_bytes: usize,
}
#[derive(Serialize)]
pub struct EvaluationReport {
    pub dataset_version: u32,
    pub fixture_only: bool,
    pub reasoning_evaluated: bool,
    pub baseline_complete: usize,
    pub oracle_complete: usize,
    pub snapshot: Snapshot,
    pub cases: Vec<CaseResult>,
    pub limitations: Vec<&'static str>,
}

pub fn run(path: &Path) -> Result<EvaluationReport> {
    let dataset: Dataset = serde_json::from_str(include_str!("fixtures/retrieval.json"))?;
    let mut memory = Memory::create(path, "synthetic-demo")?;
    let access = fixture::access();
    let mut references = BTreeMap::new();
    for session in &dataset.sessions {
        let report = memory.import_session_jsonl(
            session.jsonl.as_bytes(),
            &session.account,
            access.scopes.clone(),
        )?;
        for (i, reference) in report.records.into_iter().enumerate() {
            references.insert(
                (session.account.clone(), report.session_id.clone(), i + 1),
                reference,
            );
        }
    }
    let snapshot = memory.snapshot()?;
    let mut cases = Vec::new();
    for case in dataset.cases {
        ensure!(
            !case.expected_lines.is_empty(),
            "evaluation case has no gold evidence"
        );
        let started = Instant::now();
        let baseline = memory.recall(
            &RecallQuery {
                text: case.query.clone(),
                known_at: snapshot.clone(),
                account: Some(case.account.clone()),
                session_id: Some(case.session_id.clone()),
                occurred_after: case.occurred_after.clone(),
                occurred_before: case.occurred_before.clone(),
                limit: 2,
                neighbors: 1,
                max_bytes: 8_000,
            },
            &access,
        )?;
        let retrieval_micros = started.elapsed().as_micros();
        // Deliberately do not route from goal/needs yet. Their omission is the
        // measured baseline failure a future situated-retrieval arm can address.
        let found: BTreeSet<_> = baseline
            .iter()
            .flat_map(|h| std::iter::once(&h.anchor).chain(&h.context))
            .map(|h| h.reference.revision.clone())
            .collect();
        let evidence_bytes = baseline
            .iter()
            .flat_map(|h| std::iter::once(&h.anchor).chain(&h.context))
            .map(|h| serde_json::to_vec(h).map(|v| v.len()))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let mut oracle = Vec::new();
        let mut matched = 0;
        for line in &case.expected_lines {
            let reference = references
                .get(&(case.account.clone(), case.session_id.clone(), *line))
                .context("gold citation does not resolve")?;
            let record = memory
                .read(reference, &access)?
                .context("gold evidence unavailable")?;
            matched += usize::from(found.contains(&reference.revision));
            let sequence = memory.conn.query_row(
                "SELECT seq FROM revision WHERE id=?1",
                [&reference.revision],
                |r| r.get(0),
            )?;
            oracle.push(Hit {
                reference: reference.clone(),
                sequence,
                record,
                possible_time: false,
                required_scopes: access.scopes.clone(),
                private_data: true,
                untrusted_input: true,
                external: None,
                origin: None,
            });
        }
        let complete = matched == case.expected_lines.len();
        let coverage = matched as f64 / case.expected_lines.len() as f64;
        cases.push(CaseResult {
            case,
            baseline,
            oracle,
            coverage,
            complete,
            retrieval_micros,
            evidence_bytes,
        });
    }
    Ok(EvaluationReport { dataset_version: dataset.version, fixture_only: true,
        reasoning_evaluated: false, baseline_complete: cases.iter().filter(|c| c.complete).count(),
        oracle_complete: cases.iter().filter(|c| c.oracle.len() == c.case.expected_lines.len()).count(),
        snapshot, cases,
        limitations: vec!["32 synthetic cases across eight scenarios; not independent benchmark validation",
            "evidence coverage only; no answer model, concept induction or planning outcome measured",
            "baseline uses explicit query/source/time filters; goals and needs are recorded but not routed",
            "timings are local diagnostics, not a controlled performance benchmark"],
    })
}
