//! Fixed synthetic evidence retrieval comparison. Gold, policy and source IDs
//! stay on the host. Only prepared original passages and the public query go
//! to the checked loopback embedder.
use crate::{
    embed::{EmbeddedBatch, Embedder, EmbeddingInput, EncodingProfile, EncodingProfileSpec, Task},
    memory::{
        context::{ContextPolicy, ContextRequest, Situation, SourceSelection},
        fixture,
        hybrid::{
            HybridCounters, HybridFallback, HybridMode, HybridOptions, HybridReadiness,
            HybridResult, HybridVectors, PreparedHybrid, RankedAnchor,
        },
        Access, Capture, Memory, RecallQuery, Record, RevisionRef, Snapshot,
    },
    provider::router::is_literal_loopback,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::Path,
    time::Instant,
};

const FIXTURE: &str = include_str!("memory/fixtures/hybrid_retrieval.json");
const FIXTURE_SHA256: &str = "43cd0d770c07829583af7be4a39a6bbc969f7de86d188185eb73ecd99a006d10";
const PROTOCOL_SHA256: &str = "182edc38963989b73d93c3aa515cbf4274841a92a3ba7f7d32612c67223f5e95";
const MODES: [HybridMode; 3] = [HybridMode::Lexical, HybridMode::Dense, HybridMode::Hybrid];

/// Explicit host declarations. The server's echoed alias does not attest weights.
pub struct EvaluationOptions {
    pub endpoint: String,
    pub expected_alias: String,
    pub artifact_revision: String,
    pub dimensions: usize,
    pub pooling: String,
    pub normalization: String,
}

impl EvaluationOptions {
    fn profile(&self) -> Result<EncodingProfile> {
        EncodingProfile::new(EncodingProfileSpec {
            expected_alias: self.expected_alias.clone(),
            artifact_revision: self.artifact_revision.clone(),
            dimensions: self.dimensions,
            pooling: self.pooling.clone(),
            normalization: self.normalization.clone(),
            query_instruction: crate::embed::QUERY_INSTRUCTION.into(),
            query_template: "Instruct: {instruction}\nQuery: {text}".into(),
            preprocessing_version: "verbatim-v1".into(),
            representation_version: "original-evidence-spans-v1".into(),
        })
    }
}

#[derive(Deserialize)]
struct Dataset {
    version: u32,
    fixture_only: bool,
    corpus: Corpus,
    retrieval: Retrieval,
    grading: Grading,
}

#[derive(Deserialize)]
struct Corpus {
    domain: String,
    connector: String,
    account: String,
    records: Vec<FixtureEvidence>,
    snapshot_after: BTreeMap<String, String>,
    forget_after_snapshots: Vec<String>,
}

#[derive(Deserialize)]
struct FixtureEvidence {
    source_id: String,
    text: String,
    occurred_at: Option<String>,
    scopes: BTreeSet<String>,
}

#[derive(Deserialize)]
struct Retrieval {
    policy_defaults: PolicyDefaults,
    situation: FixtureSituation,
    bindings: Vec<SourceSelection>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct FixtureSituation {
    event: String,
    goal: String,
    needs: Option<String>,
}

#[derive(Deserialize)]
struct PolicyDefaults {
    scopes: BTreeSet<String>,
    account: Option<String>,
    session_id: Option<String>,
    occurred_after: Option<String>,
    occurred_before: Option<String>,
    required_mention: Option<String>,
    valid_at: Option<String>,
    limit: usize,
    neighbors: usize,
    max_bytes: usize,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    query: String,
    policy: CasePolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CasePolicy {
    snapshot: String,
    occurred_after: Option<String>,
    occurred_before: Option<String>,
    required_mention: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct Grading {
    labels: BTreeMap<String, String>,
    gold: BTreeMap<String, BTreeSet<String>>,
    forbidden: Forbidden,
}

#[derive(Deserialize)]
struct Forbidden {
    all_cases: BTreeMap<String, String>,
    by_case: BTreeMap<String, BTreeMap<String, String>>,
}

struct Selection {
    request: ContextRequest,
    policy: ContextPolicy,
    required_mention: Option<String>,
}

impl PolicyDefaults {
    fn selection(&self, case: &Case, domain: &str, known_at: Snapshot) -> Selection {
        let policy = &case.policy;
        Selection {
            request: ContextRequest {
                query: case.query.clone(),
                bindings: Vec::new(),
            },
            policy: ContextPolicy {
                access: Access {
                    domain: domain.into(),
                    scopes: self.scopes.clone(),
                },
                recall: RecallQuery {
                    text: String::new(),
                    known_at,
                    account: self.account.clone(),
                    session_id: self.session_id.clone(),
                    occurred_after: policy
                        .occurred_after
                        .clone()
                        .or_else(|| self.occurred_after.clone()),
                    occurred_before: policy
                        .occurred_before
                        .clone()
                        .or_else(|| self.occurred_before.clone()),
                    limit: policy.limit.unwrap_or(self.limit),
                    neighbors: self.neighbors,
                    max_bytes: self.max_bytes,
                },
                valid_at: self.valid_at.clone(),
            },
            required_mention: policy
                .required_mention
                .clone()
                .or_else(|| self.required_mention.clone()),
        }
    }
}

#[derive(Serialize)]
pub struct EvaluationReport {
    fixture_only: bool,
    fixture_version: u32,
    fixture_sha256: String,
    frozen_protocol_sha256: &'static str,
    encoding: ProfileReport,
    snapshots: BTreeMap<String, Snapshot>,
    source_revisions: BTreeMap<String, RevisionRef>,
    wall_micros: u128,
    modes: [ModeSummary; 3],
    cases: Vec<CaseReport>,
}

#[derive(Serialize)]
struct ProfileReport {
    endpoint: String,
    profile_id: String,
    expected_alias: String,
    host_declared_artifact_revision: String,
    dimensions: usize,
    pooling: String,
    normalization: String,
    query_instruction: String,
    query_template: String,
    preprocessing_version: String,
    representation_version: String,
}

#[derive(Serialize)]
struct CaseReport {
    id: String,
    category: String,
    gold_revisions: Vec<RevisionRef>,
    forbidden_revisions: Vec<RevisionRef>,
    snapshot: String,
    anchor_limit: usize,
    neighbors: usize,
    max_bytes: usize,
    shared: SharedWork,
    treatments: [Treatment; 3],
}

#[derive(Serialize)]
struct SharedWork {
    preparation_micros: u128,
    readiness: Option<HybridReadiness>,
    preparation_error: Option<String>,
    counters: Option<HybridCounters>,
    embedding: EmbeddingCost,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum EmbeddingStatus {
    NotPrepared,
    Skipped,
    Completed,
    Error,
}

#[derive(Serialize)]
struct EmbeddingCost {
    status: EmbeddingStatus,
    wall_micros: u128,
    inputs: Option<usize>,
    attempts: Option<usize>,
    request_bytes: Option<usize>,
    response_bytes: Option<usize>,
    server_reported_alias: Option<String>,
    error: Option<String>,
}

impl EmbeddingCost {
    fn skipped(prepared: bool) -> Self {
        Self {
            status: if prepared {
                EmbeddingStatus::Skipped
            } else {
                EmbeddingStatus::NotPrepared
            },
            wall_micros: 0,
            inputs: prepared.then_some(0),
            attempts: Some(0),
            request_bytes: Some(0),
            response_bytes: Some(0),
            server_reported_alias: None,
            error: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TreatmentStatus {
    Completed,
    Fallback,
    Error,
}

#[derive(Serialize)]
struct Treatment {
    mode: HybridMode,
    status: TreatmentStatus,
    effective_mode: Option<HybridMode>,
    fallback: Option<HybridFallback>,
    error: Option<String>,
    /// Independently measured lexical work, or semantic finalization alone.
    measured_micros: Option<u128>,
    /// Semantic finalization plus the shared preparation and embedding.
    derived_wall_micros: Option<u128>,
    coverage: Option<f64>,
    complete: Option<bool>,
    reciprocal_rank: Option<f64>,
    returned_records: Option<usize>,
    evidence_bytes: Option<usize>,
    packet_bytes: Option<usize>,
    unauthorized_returns: Option<usize>,
    literal_violations: Option<usize>,
    ranked_anchors: Vec<RankedAnchor>,
    returned_revisions: Vec<RevisionRef>,
}

#[derive(Serialize)]
struct ModeSummary {
    mode: HybridMode,
    planned: usize,
    completed: usize,
    fallback: usize,
    error: usize,
    planned_nonempty_gold: usize,
    planned_empty_gold: usize,
    mean_coverage: Option<f64>,
    complete: usize,
    mean_reciprocal_rank: Option<f64>,
    empty_gold_returned_records: usize,
    evidence_bytes: usize,
    unauthorized_returns: usize,
    literal_violations: usize,
    measured_micros: u128,
    derived_wall_micros: Option<u128>,
}

/// Compare the built-in fixture in a new plaintext prototype database.
pub async fn run(path: &Path, options: &EvaluationOptions) -> Result<EvaluationReport> {
    ensure!(
        is_literal_loopback(&options.endpoint),
        "hybrid evaluation requires a literal loopback HTTP endpoint without credentials, query or fragment"
    );
    let profile = options.profile()?;
    let embedder = Embedder::new(&options.endpoint)?;
    evaluate(path, options, &profile, |inputs| {
        let (embedder, profile) = (&embedder, &profile);
        async move { embedder.embed_checked(&inputs, profile).await }
    })
    .await
}

fn fixture_sha256() -> String {
    Sha256::digest(FIXTURE.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn evaluate<F, Fut>(
    path: &Path,
    options: &EvaluationOptions,
    profile: &EncodingProfile,
    embed: F,
) -> Result<EvaluationReport>
where
    F: Fn(Vec<EmbeddingInput>) -> Fut,
    Fut: Future<Output = Result<EmbeddedBatch>>,
{
    let started = Instant::now();
    let fixture_sha256 = fixture_sha256();
    ensure!(
        fixture_sha256 == FIXTURE_SHA256,
        "synthetic hybrid fixture hash differs from the frozen protocol"
    );
    let Dataset {
        version,
        fixture_only,
        corpus,
        retrieval,
        grading,
    } = serde_json::from_str(FIXTURE)?;
    ensure!(
        fixture_only && version == 1,
        "invalid synthetic hybrid fixture"
    );
    ensure!(
        retrieval.bindings.is_empty(),
        "hybrid fixture cannot bind sources"
    );
    let source = fixture::source_key("");
    ensure!(
        corpus.connector == source.connector && corpus.account == source.account,
        "invalid synthetic source namespace"
    );
    let mut memory = Memory::create(path, &corpus.domain)?;
    let mut source_revisions = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    for record in corpus.records {
        let reference = memory.capture(Capture {
            source: fixture::source_key(&record.source_id),
            text: record.text,
            occurred_at: record.occurred_at,
            scopes: record.scopes,
            transcript: None,
        })?;
        ensure!(
            source_revisions
                .insert(record.source_id.clone(), reference)
                .is_none(),
            "duplicate hybrid fixture source"
        );
        for (name, after) in &corpus.snapshot_after {
            if *after == record.source_id {
                snapshots.insert(name.clone(), memory.snapshot()?);
            }
        }
    }
    ensure!(
        snapshots.len() == corpus.snapshot_after.len(),
        "unknown snapshot boundary"
    );
    for id in corpus.forget_after_snapshots {
        ensure!(
            source_revisions.contains_key(&id),
            "unknown source to forget"
        );
        memory.forget(&fixture::source_key(&id))?;
    }

    let situation = Situation {
        event: retrieval.situation.event,
        goal: retrieval.situation.goal,
        needs: retrieval.situation.needs,
    };
    let mut cases = Vec::with_capacity(retrieval.cases.len());
    for case in retrieval.cases {
        let snapshot = snapshots
            .get(&case.policy.snapshot)
            .context("unknown hybrid case snapshot")?;
        let selection =
            retrieval
                .policy_defaults
                .selection(&case, &corpus.domain, snapshot.clone());
        let gold = grading
            .gold
            .get(&case.id)
            .context("missing hybrid case gold")?;
        let forbidden = grading
            .forbidden
            .all_cases
            .keys()
            .chain(
                grading
                    .forbidden
                    .by_case
                    .get(&case.id)
                    .into_iter()
                    .flat_map(BTreeMap::keys),
            )
            .collect::<BTreeSet<_>>();
        let resolve = |id: &String| {
            source_revisions
                .get(id)
                .cloned()
                .context("unknown grading source")
        };
        let gold = gold.iter().map(resolve).collect::<Result<Vec<_>>>()?;
        let forbidden = forbidden
            .into_iter()
            .map(resolve)
            .collect::<Result<Vec<_>>>()?;
        let category = grading
            .labels
            .get(&case.id)
            .context("missing hybrid case category")?
            .clone();
        let comparison = compare(&memory, &selection, &situation, profile, &embed).await;
        let (shared, treatments) =
            comparison.grade(&gold, &forbidden, selection.required_mention.as_deref())?;
        cases.push(CaseReport {
            id: case.id,
            category,
            gold_revisions: gold,
            forbidden_revisions: forbidden,
            snapshot: case.policy.snapshot,
            anchor_limit: selection.policy.recall.limit,
            neighbors: selection.policy.recall.neighbors,
            max_bytes: selection.policy.recall.max_bytes,
            shared,
            treatments,
        });
    }

    Ok(EvaluationReport {
        fixture_only,
        fixture_version: version,
        fixture_sha256,
        frozen_protocol_sha256: PROTOCOL_SHA256,
        encoding: ProfileReport {
            endpoint: options.endpoint.clone(),
            profile_id: profile.id().into(),
            expected_alias: profile.expected_alias().into(),
            host_declared_artifact_revision: profile.artifact_revision().into(),
            dimensions: profile.dimensions(),
            pooling: profile.pooling().into(),
            normalization: profile.normalization().into(),
            query_instruction: profile.query_instruction().into(),
            query_template: profile.query_template().into(),
            preprocessing_version: profile.preprocessing_version().into(),
            representation_version: profile.representation_version().into(),
        },
        snapshots,
        source_revisions,
        wall_micros: started.elapsed().as_micros(),
        modes: [
            summarize(&cases, 0),
            summarize(&cases, 1),
            summarize(&cases, 2),
        ],
        cases,
    })
}

struct Comparison {
    shared: SharedWork,
    treatments: [TimedTreatment; 3],
}

struct TimedTreatment {
    mode: HybridMode,
    measured_micros: Option<u128>,
    result: Result<HybridResult>,
}

impl TimedTreatment {
    fn measure(mode: HybridMode, run: impl FnOnce() -> Result<HybridResult>) -> Self {
        let started = Instant::now();
        let result = run();
        Self {
            mode,
            measured_micros: Some(started.elapsed().as_micros()),
            result,
        }
    }
}

async fn compare<F, Fut>(
    memory: &Memory,
    selection: &Selection,
    situation: &Situation,
    profile: &EncodingProfile,
    embed: F,
) -> Comparison
where
    F: FnOnce(Vec<EmbeddingInput>) -> Fut,
    Fut: Future<Output = Result<EmbeddedBatch>>,
{
    let Selection {
        request,
        policy,
        required_mention,
    } = selection;
    let lexical = TimedTreatment::measure(HybridMode::Lexical, || {
        memory.lexical_hybrid_context(request, policy, situation, required_mention.as_deref())
    });
    let started = Instant::now();
    let prepared = memory.prepare_hybrid_context(
        request,
        policy,
        situation,
        &HybridOptions {
            profile_id: profile.id().into(),
            dimensions: profile.dimensions(),
            required_mention: required_mention.clone(),
        },
    );
    let preparation_micros = started.elapsed().as_micros();
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            let unavailable = |mode| TimedTreatment {
                mode,
                measured_micros: None,
                result: Err(anyhow::anyhow!("semantic preparation failed")),
            };
            return Comparison {
                shared: SharedWork {
                    preparation_micros,
                    readiness: None,
                    counters: None,
                    preparation_error: Some(error.to_string()),
                    embedding: EmbeddingCost::skipped(false),
                },
                treatments: [
                    lexical,
                    unavailable(HybridMode::Dense),
                    unavailable(HybridMode::Hybrid),
                ],
            };
        }
    };
    let readiness = prepared.readiness();
    let (vectors, embedding) = if readiness == HybridReadiness::Ready {
        embed_ready(&prepared, embed).await
    } else {
        (None, EmbeddingCost::skipped(true))
    };
    let finish = |mode| {
        TimedTreatment::measure(mode, || {
            memory.finish_hybrid_context(&prepared, vectors.as_ref(), mode)
        })
    };
    Comparison {
        shared: SharedWork {
            preparation_micros,
            readiness: Some(readiness),
            counters: Some(prepared.counters()),
            preparation_error: None,
            embedding,
        },
        treatments: [
            lexical,
            finish(HybridMode::Dense),
            finish(HybridMode::Hybrid),
        ],
    }
}

async fn embed_ready<F, Fut>(
    prepared: &PreparedHybrid,
    embed: F,
) -> (Option<HybridVectors>, EmbeddingCost)
where
    F: FnOnce(Vec<EmbeddingInput>) -> Fut,
    Fut: Future<Output = Result<EmbeddedBatch>>,
{
    let started = Instant::now();
    let inputs: Vec<_> = std::iter::once(EmbeddingInput {
        task: Task::Query,
        text: prepared.query().into(),
    })
    .chain(prepared.passages().iter().map(|passage| EmbeddingInput {
        task: Task::Passage,
        text: passage.text.clone(),
    }))
    .collect();
    let count = inputs.len();
    let result = embed(inputs).await;
    let wall_micros = started.elapsed().as_micros();
    match result {
        Ok(batch) => {
            let mut vectors = batch.vectors.into_iter();
            let query = vectors.next().unwrap_or_default();
            (
                Some(HybridVectors {
                    profile_id: batch.profile_id,
                    query,
                    passages: vectors.collect(),
                }),
                EmbeddingCost {
                    status: EmbeddingStatus::Completed,
                    wall_micros,
                    inputs: Some(count),
                    attempts: Some(batch.attempts),
                    request_bytes: Some(batch.request_bytes),
                    response_bytes: Some(batch.response_bytes),
                    server_reported_alias: batch.server_model,
                    error: None,
                },
            )
        }
        Err(error) => (
            None,
            EmbeddingCost {
                status: EmbeddingStatus::Error,
                wall_micros,
                inputs: Some(count),
                attempts: None,
                request_bytes: None,
                response_bytes: None,
                server_reported_alias: None,
                error: Some(error.to_string()),
            },
        ),
    }
}

impl Comparison {
    fn grade(
        self,
        gold: &[RevisionRef],
        forbidden: &[RevisionRef],
        required_mention: Option<&str>,
    ) -> Result<(SharedWork, [Treatment; 3])> {
        let shared_micros = self.shared.preparation_micros + self.shared.embedding.wall_micros;
        let [lexical, dense, hybrid] = self.treatments;
        let grade = |timed| score(timed, shared_micros, gold, forbidden, required_mention);
        Ok((
            self.shared,
            [grade(lexical)?, grade(dense)?, grade(hybrid)?],
        ))
    }
}

fn score(
    timed: TimedTreatment,
    shared_micros: u128,
    gold: &[RevisionRef],
    forbidden: &[RevisionRef],
    required_mention: Option<&str>,
) -> Result<Treatment> {
    let nonempty = !gold.is_empty();
    let mut treatment = Treatment {
        mode: timed.mode,
        status: TreatmentStatus::Error,
        effective_mode: None,
        fallback: None,
        error: None,
        measured_micros: timed.measured_micros,
        derived_wall_micros: (timed.mode != HybridMode::Lexical)
            .then_some(shared_micros + timed.measured_micros.unwrap_or(0)),
        coverage: nonempty.then_some(0.0),
        complete: nonempty.then_some(false),
        reciprocal_rank: nonempty.then_some(0.0),
        returned_records: None,
        evidence_bytes: None,
        packet_bytes: None,
        unauthorized_returns: None,
        literal_violations: None,
        ranked_anchors: Vec::new(),
        returned_revisions: Vec::new(),
    };
    let result = match timed.result {
        Ok(result) => result,
        Err(error) => {
            treatment.error = Some(error.to_string());
            return Ok(treatment);
        }
    };
    let delivered: Vec<_> = result
        .packet
        .records
        .iter()
        .map(|hit| hit.reference.clone())
        .collect();
    let anchors: Vec<_> = result
        .ranked_anchors
        .into_iter()
        .filter(|anchor| delivered.contains(&anchor.reference))
        .collect();
    let packet = serde_json::to_vec(&result.packet)?;
    let wire: serde_json::Value = serde_json::from_slice(&packet)?;
    let wire_records = wire["records"]
        .as_array()
        .context("missing context records")?;
    let evidence_bytes = wire_records
        .iter()
        .zip(&result.packet.records)
        .filter(|(_, hit)| matches!(&hit.record, Record::Evidence(_)))
        .try_fold(0, |sum, (record, _)| {
            Ok::<_, serde_json::Error>(sum + serde_json::to_vec(record)?.len())
        })?;
    let literal_violations = required_mention.map_or(0, |mention| {
        anchors
            .iter()
            .filter(|anchor| {
                !result.packet.records.iter().any(|hit| {
                    hit.reference == anchor.reference
                        && matches!(&hit.record, Record::Evidence(capture)
                    if capture.text.split(|c: char| !c.is_alphanumeric())
                        .any(|word| word.eq_ignore_ascii_case(mention)))
                })
            })
            .count()
    });
    treatment.status = if result.fallback.is_some() {
        TreatmentStatus::Fallback
    } else {
        TreatmentStatus::Completed
    };
    treatment.effective_mode = Some(result.effective_mode);
    treatment.fallback = result.fallback;
    treatment.returned_records = Some(delivered.len());
    treatment.evidence_bytes = Some(evidence_bytes);
    treatment.packet_bytes = Some(packet.len());
    treatment.unauthorized_returns = Some(
        delivered
            .iter()
            .filter(|reference| forbidden.contains(reference))
            .count(),
    );
    treatment.literal_violations = Some(literal_violations);
    if nonempty && treatment.status == TreatmentStatus::Completed {
        let found = gold
            .iter()
            .filter(|reference| delivered.contains(reference))
            .count();
        treatment.coverage = Some(found as f64 / gold.len() as f64);
        treatment.complete = Some(found == gold.len());
        treatment.reciprocal_rank = Some(
            anchors
                .iter()
                .position(|anchor| gold.contains(&anchor.reference))
                .map_or(0.0, |position| 1.0 / (position + 1) as f64),
        );
    }
    treatment.ranked_anchors = anchors;
    treatment.returned_revisions = delivered;
    Ok(treatment)
}

fn summarize(cases: &[CaseReport], arm: usize) -> ModeSummary {
    let mode = MODES[arm];
    let mut summary = ModeSummary {
        mode,
        planned: cases.len(),
        completed: 0,
        fallback: 0,
        error: 0,
        planned_nonempty_gold: 0,
        planned_empty_gold: 0,
        mean_coverage: None,
        complete: 0,
        mean_reciprocal_rank: None,
        empty_gold_returned_records: 0,
        evidence_bytes: 0,
        unauthorized_returns: 0,
        literal_violations: 0,
        measured_micros: 0,
        derived_wall_micros: (mode != HybridMode::Lexical).then_some(0),
    };
    let mut coverage = 0.0;
    let mut reciprocal_rank = 0.0;
    for case in cases {
        let result = &case.treatments[arm];
        match result.status {
            TreatmentStatus::Completed => summary.completed += 1,
            TreatmentStatus::Fallback => summary.fallback += 1,
            TreatmentStatus::Error => summary.error += 1,
        }
        if case.gold_revisions.is_empty() {
            summary.planned_empty_gold += 1;
            summary.empty_gold_returned_records += result.returned_records.unwrap_or(0);
        } else {
            summary.planned_nonempty_gold += 1;
            coverage += result.coverage.unwrap_or(0.0);
            reciprocal_rank += result.reciprocal_rank.unwrap_or(0.0);
            summary.complete += usize::from(result.complete == Some(true));
        }
        summary.evidence_bytes += result.evidence_bytes.unwrap_or(0);
        summary.unauthorized_returns += result.unauthorized_returns.unwrap_or(0);
        summary.literal_violations += result.literal_violations.unwrap_or(0);
        summary.measured_micros += result.measured_micros.unwrap_or(0);
        if let Some(derived) = &mut summary.derived_wall_micros {
            *derived += result.derived_wall_micros.unwrap_or(0);
        }
    }
    if summary.planned_nonempty_gold != 0 {
        let denominator = summary.planned_nonempty_gold as f64;
        summary.mean_coverage = Some(coverage / denominator);
        summary.mean_reciprocal_rank = Some(reciprocal_rank / denominator);
    }
    summary
}

#[cfg(test)]
#[path = "memory_hybrid_workflow_tests.rs"]
mod tests;
