//! Explicit, invocation-local lexical/dense comparison over synthetic evidence.
use super::{
    context::{self, ContextPacket, ContextPolicy, ContextRequest, Situation},
    recall::{self, RankedRow, RecallBatch},
    sql::*,
    *,
};
use anyhow::{ensure, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RECORDS: usize = 256;
const MAX_BYTES: usize = 262_144;
const MAX_PASSAGES: usize = 512;
const PASSAGE_SCALARS: usize = 2_000;
const RRF_K: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridMode {
    Lexical,
    Dense,
    Hybrid,
}

#[derive(Clone, Debug)]
pub struct HybridOptions {
    pub profile_id: String,
    pub dimensions: usize,
    pub required_mention: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridFallback {
    CorpusBounds,
    EmbeddingsUnavailable,
    CorpusChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridReadiness {
    Ready,
    Empty,
    LexicalFallback(HybridFallback),
}

/// Exact original field and UTF-8 byte span in a revision.
pub struct HybridPassage {
    pub reference: RevisionRef,
    pub field: String,
    pub start: usize,
    pub end: usize,
    pub content_digest: String,
    pub text: String,
    sequence: i64,
}

pub struct HybridVectors {
    pub profile_id: String,
    pub query: Vec<f32>,
    pub passages: Vec<Vec<f32>>,
}

/// Opaque receipt. No connection, transaction, or authorization survives the read.
pub struct PreparedHybrid {
    context: HybridContext,
    options: HybridOptions,
    passages: Vec<HybridPassage>,
    fingerprint: Option<[u8; 32]>,
    readiness: HybridReadiness,
    counters: HybridCounters,
}

impl PreparedHybrid {
    pub fn query(&self) -> &str {
        &self.context.query.text
    }
    pub fn passages(&self) -> &[HybridPassage] {
        &self.passages
    }
    pub fn readiness(&self) -> HybridReadiness {
        self.readiness
    }
    pub fn counters(&self) -> HybridCounters {
        self.counters
    }
}

#[derive(Debug, Serialize)]
pub struct RankedAnchor {
    pub reference: RevisionRef,
    /// Position among selected anchors before final packet budgeting.
    pub rank: usize,
    pub bm25: Option<f64>,
    pub cosine: Option<f64>,
    pub rrf: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HybridCounters {
    /// Full eligible lexical corpus, capped at the applicable bound plus one.
    pub eligible_records: usize,
    pub original_bytes: usize,
    /// Candidates with original fields after mention filtering. Zero when the
    /// record/byte bound prevented passage construction.
    pub dense_records: usize,
    /// Dense passage count, capped at the passage bound plus one.
    pub passages: usize,
}

#[derive(Debug, Serialize)]
pub struct HybridResult {
    pub packet: ContextPacket,
    pub requested_mode: HybridMode,
    pub effective_mode: HybridMode,
    pub fallback: Option<HybridFallback>,
    /// Selected anchors still present in the final, authorized packet.
    pub ranked_anchors: Vec<RankedAnchor>,
    /// Finalization's semantic scan, before any separately checked fallback.
    /// None for an independent lexical invocation.
    pub counters: Option<HybridCounters>,
}

struct HybridContext {
    query: RecallQuery,
    access: Access,
    valid_at: Option<String>,
    situation: Situation,
    mention: Option<String>,
}

impl HybridContext {
    fn new(
        request: &ContextRequest,
        policy: &ContextPolicy,
        situation: &Situation,
        required_mention: Option<&str>,
    ) -> Result<Self> {
        ensure!(
            request.bindings.is_empty(),
            "hybrid context does not accept source selections"
        );
        let (query, valid_at) = context::context_parameters(request, policy, situation)?;
        Ok(Self {
            query,
            access: policy.access.clone(),
            valid_at,
            situation: situation.clone(),
            mention: required_mention
                .map(recall::mention_expression)
                .transpose()?,
        })
    }

    fn packet(&self, budget_omitted: bool) -> ContextPacket {
        context::empty_packet(
            &self.query,
            self.valid_at.as_deref(),
            &self.situation,
            budget_omitted,
        )
    }
}

impl Memory {
    pub fn prepare_hybrid_context(
        &self,
        request: &ContextRequest,
        policy: &ContextPolicy,
        situation: &Situation,
        options: &HybridOptions,
    ) -> Result<PreparedHybrid> {
        ensure!(
            !options.profile_id.trim().is_empty() && options.profile_id.len() <= 8192,
            "invalid hybrid encoding profile"
        );
        ensure!(
            (1..=8192).contains(&options.dimensions),
            "invalid hybrid vector dimensions"
        );
        let context = HybridContext::new(
            request,
            policy,
            situation,
            options.required_mention.as_deref(),
        )?;
        let tx = self.conn.unchecked_transaction()?;
        let (query, _) = self.check_recall_in(
            &context.query,
            context.mention.as_deref(),
            &context.access,
            &tx,
        )?;
        context::check_envelope(&context.packet(false), query.max_bytes)?;
        let selection = SourceChecks::new(self.source_guard.as_deref());
        let corpus = Corpus::scan(
            &tx,
            &query,
            &context.access,
            context.mention.as_deref(),
            &selection,
        )?;
        tx.commit()?;
        Ok(PreparedHybrid {
            context,
            options: options.clone(),
            readiness: corpus.readiness,
            fingerprint: corpus.fingerprint,
            passages: corpus.passages,
            counters: corpus.counters,
        })
    }

    /// Run the ordinary lexical selection and assembly without preparing vectors.
    pub fn lexical_hybrid_context(
        &self,
        request: &ContextRequest,
        policy: &ContextPolicy,
        situation: &Situation,
        required_mention: Option<&str>,
    ) -> Result<HybridResult> {
        let context = HybridContext::new(request, policy, situation, required_mention)?;
        let tx = self.conn.unchecked_transaction()?;
        let result =
            HybridRead::new(self, &tx, &context).lexical(HybridMode::Lexical, None, None)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn finish_hybrid_context(
        &self,
        prepared: &PreparedHybrid,
        vectors: Option<&HybridVectors>,
        mode: HybridMode,
    ) -> Result<HybridResult> {
        let tx = self.conn.unchecked_transaction()?;
        let context = &prepared.context;
        let read = HybridRead::new(self, &tx, context);
        let vectors = vectors
            .map(|v| CheckedVectors::new(prepared, v))
            .transpose()?;
        if mode == HybridMode::Lexical {
            let result = read.lexical(mode, None, None)?;
            tx.commit()?;
            return Ok(result);
        }

        let mention = context.mention.as_deref();
        let (query, expression) =
            self.check_recall_in(&context.query, mention, &context.access, &tx)?;
        let selection = SourceChecks::new(self.source_guard.as_deref());
        let corpus = Corpus::scan(&tx, &query, &context.access, mention, &selection)?;
        let counters = Some(corpus.counters);
        let fallback = match (prepared.readiness, corpus.readiness) {
            (HybridReadiness::LexicalFallback(reason), _) => Some(reason),
            (before, after) if before != after || prepared.fingerprint != corpus.fingerprint => {
                Some(HybridFallback::CorpusChanged)
            }
            (HybridReadiness::Ready, _) if vectors.is_none() => {
                Some(HybridFallback::EmbeddingsUnavailable)
            }
            _ => None,
        };
        let result = if let Some(reason) = fallback {
            read.lexical(mode, Some(reason), counters)?
        } else if corpus.readiness == HybridReadiness::Empty {
            let packet = context.packet(false);
            context::check_envelope(&packet, query.max_bytes)?;
            HybridResult {
                packet,
                requested_mode: mode,
                effective_mode: mode,
                fallback: None,
                ranked_anchors: vec![],
                counters,
            }
        } else {
            let mut rows = dense_rows(
                &corpus.passages,
                vectors.expect("ready semantic mode has checked vectors"),
            );
            if mode == HybridMode::Hybrid {
                let mut statement = recall::lexical_statement(&corpus.scorer, mention)?;
                rows = fused_rows(
                    rows,
                    recall::lexical_rows(&mut statement, &expression, mention)?,
                )?;
            }
            let batch = recall::budgeted_recall(
                &query,
                &context.access,
                &tx,
                Some(&selection),
                rows.into_iter().map(Ok),
            )?;
            let (packet, ranked_anchors, invalidated) = read.assemble(&batch, &selection)?;
            if invalidated {
                // A binding changed after the eligible-view pass. Recompute
                // lexical statistics with fresh checks before returning anything.
                read.lexical(mode, Some(HybridFallback::CorpusChanged), counters)?
            } else {
                HybridResult {
                    packet,
                    requested_mode: mode,
                    effective_mode: mode,
                    fallback: None,
                    ranked_anchors,
                    counters,
                }
            }
        };
        tx.commit()?;
        Ok(result)
    }
}

struct HybridRead<'a> {
    db: &'a Memory,
    conn: &'a Connection,
    context: &'a HybridContext,
}

impl<'a> HybridRead<'a> {
    fn new(db: &'a Memory, conn: &'a Connection, context: &'a HybridContext) -> Self {
        Self { db, conn, context }
    }

    fn lexical(
        &self,
        requested_mode: HybridMode,
        fallback: Option<HybridFallback>,
        counters: Option<HybridCounters>,
    ) -> Result<HybridResult> {
        let ctx = self.context;
        let selection = SourceChecks::new(self.db.source_guard.as_deref());
        let batch = self.db.recall_in(
            &ctx.query,
            ctx.mention.as_deref(),
            &ctx.access,
            self.conn,
            Some(&selection),
        )?;
        let (packet, ranked_anchors, _) = self.assemble(&batch, &selection)?;
        Ok(HybridResult {
            packet,
            requested_mode,
            effective_mode: HybridMode::Lexical,
            fallback,
            ranked_anchors,
            counters,
        })
    }

    fn assemble<S: Copy + Into<Scores>>(
        &self,
        batch: &RecallBatch<S>,
        selection: &SourceChecks<'_>,
    ) -> Result<(ContextPacket, Vec<RankedAnchor>, bool)> {
        let ctx = self.context;
        let mut packet = ctx.packet(batch.budget_omitted);
        if ctx.mention.is_some() && batch.hits.is_empty() {
            context::check_envelope(&packet, ctx.query.max_bytes)?;
        } else {
            packet = context::assemble(
                self.conn,
                &ctx.query,
                &ctx.access,
                packet,
                &batch.candidates(),
                Some(selection),
            )?;
        }
        let had_records = !packet.records.is_empty();
        let packet = context::final_packet(
            self.conn,
            packet,
            &ctx.access,
            self.db.source_guard.as_deref(),
        )?;
        let invalidated = had_records && packet.records.is_empty();
        let ranked_anchors = batch
            .hits
            .iter()
            .enumerate()
            .filter(|(_, hit)| {
                packet
                    .records
                    .iter()
                    .any(|returned| returned.reference == hit.anchor.reference)
            })
            .map(|(index, hit)| {
                let scores = hit.score.into();
                RankedAnchor {
                    reference: hit.anchor.reference.clone(),
                    rank: index + 1,
                    bm25: scores.bm25,
                    cosine: scores.cosine,
                    rrf: scores.rrf,
                }
            })
            .collect();
        Ok((packet, ranked_anchors, invalidated))
    }
}

struct Corpus {
    scorer: Connection,
    readiness: HybridReadiness,
    fingerprint: Option<[u8; 32]>,
    passages: Vec<HybridPassage>,
    counters: HybridCounters,
}

struct OriginalFields {
    reference: RevisionRef,
    sequence: i64,
    fields: Vec<TextField>,
    fingerprint: [u8; 32],
}

impl Corpus {
    fn scan(
        conn: &Connection,
        query: &RecallQuery,
        access: &Access,
        mention: Option<&str>,
        selection: &SourceChecks<'_>,
    ) -> Result<Self> {
        let mut originals = Vec::new();
        let mut counters = HybridCounters::default();
        let has_bindings = host_capture::has_bindings(conn)?;
        let scorer = recall::eligible_index(
            conn,
            query,
            access,
            mention.is_some(),
            Some(selection),
            |hit, payload, indexed| {
                let Record::Evidence(evidence) = &hit.record else {
                    unreachable!()
                };
                counters.eligible_records = (counters.eligible_records + 1).min(MAX_RECORDS + 1);
                let fields = original_fields(evidence);
                counters.original_bytes = (counters.original_bytes
                    + fields.clone().map(|(_, text)| text.len()).sum::<usize>())
                .min(MAX_BYTES + 1);
                if counters.eligible_records <= MAX_RECORDS && counters.original_bytes <= MAX_BYTES
                {
                    let mut hasher = Sha256::new();
                    hash_bytes(&mut hasher, payload.as_bytes());
                    hash_bytes(&mut hasher, indexed.as_bytes());
                    hash_eligibility(&mut hasher, conn, hit, has_bindings)?;
                    originals.push(OriginalFields {
                        reference: hit.reference.clone(),
                        sequence: hit.sequence,
                        fingerprint: hasher.finalize().into(),
                        fields: fields
                            .map(|(pointer, text)| TextField {
                                pointer: pointer.into(),
                                text: text.into(),
                            })
                            .collect(),
                    });
                }
                Ok(())
            },
        )?;
        let too_large =
            counters.eligible_records > MAX_RECORDS || counters.original_bytes > MAX_BYTES;
        let fingerprint = (!too_large).then(|| {
            // Stabilize the bounded semantic view without adding a sort to
            // the ordinary BM25 scan or relying on SQLite row order.
            originals.sort_by(|a, b| {
                (a.sequence, &a.reference.revision, a.fingerprint).cmp(&(
                    b.sequence,
                    &b.reference.revision,
                    b.fingerprint,
                ))
            });
            let mut hasher = Sha256::new();
            for original in &originals {
                hasher.update(original.fingerprint);
            }
            hasher.finalize().into()
        });
        let mut readiness = HybridReadiness::Empty;
        let allowed = if let Some(expression) = mention {
            let mut statement =
                scorer.prepare("SELECT revision FROM mentions WHERE mentions MATCH ?1")?;
            let mut rows = statement.query_map([expression], |r| r.get::<_, String>(0))?;
            // The lexical index can be larger than the semantic bound. Avoid
            // materializing its mention matches when only emptiness is needed.
            if too_large {
                if rows.next().transpose()?.is_some() {
                    readiness = HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds);
                }
                None
            } else {
                Some(rows.collect::<rusqlite::Result<BTreeSet<_>>>()?)
            }
        } else {
            if too_large {
                readiness = HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds);
            }
            None
        };
        let mut corpus = Self {
            scorer,
            readiness,
            fingerprint,
            passages: vec![],
            counters,
        };
        if too_large {
            return Ok(corpus);
        }
        for original in originals {
            if original.fields.is_empty()
                || allowed
                    .as_ref()
                    .is_some_and(|ids| !ids.contains(&original.reference.revision))
            {
                continue;
            }
            corpus.counters.dense_records += 1;
            for field in original.fields {
                let mut start = 0;
                for end in field
                    .text
                    .char_indices()
                    .step_by(PASSAGE_SCALARS)
                    .skip(1)
                    .map(|(index, _)| index)
                    .chain(std::iter::once(field.text.len()))
                {
                    corpus.counters.passages = (corpus.counters.passages + 1).min(MAX_PASSAGES + 1);
                    if corpus.counters.passages <= MAX_PASSAGES {
                        let text = &field.text[start..end];
                        corpus.passages.push(HybridPassage {
                            reference: original.reference.clone(),
                            field: field.pointer.clone(),
                            start,
                            end,
                            content_digest: Sha256::digest(text.as_bytes())
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect(),
                            text: text.into(),
                            sequence: original.sequence,
                        });
                    }
                    start = end;
                }
            }
        }
        if corpus.counters.passages > MAX_PASSAGES {
            corpus.passages.clear();
            corpus.readiness = HybridReadiness::LexicalFallback(HybridFallback::CorpusBounds);
        } else if !corpus.passages.is_empty() {
            corpus.readiness = HybridReadiness::Ready;
        }
        Ok(corpus)
    }
}

fn original_fields(evidence: &Capture) -> impl Iterator<Item = (&str, &str)> + Clone {
    let projection = matches!(
        evidence.source.connector.as_str(),
        "host-capture-projection-v1" | "host-capture-projection-v2"
    );
    let plain = evidence
        .transcript
        .is_none()
        .then_some(("", evidence.text.as_str()));
    plain
        .into_iter()
        .chain(evidence.transcript.iter().flat_map(|meta| {
            meta.fields
                .iter()
                .map(|f| (f.pointer.as_str(), f.text.as_str()))
        }))
        .filter(move |(_, text)| !projection && !text.is_empty())
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn hash_eligibility(
    hasher: &mut Sha256,
    conn: &Connection,
    hit: &Hit,
    has_bindings: bool,
) -> Result<()> {
    let mut inputs = closure(conn, &hit.reference)?;
    inputs.sort_by(|a, b| a.revision.cmp(&b.revision));
    hash_bytes(
        hasher,
        &serde_json::to_vec(&(&hit.reference, hit.sequence, &inputs))?,
    );
    for input in inputs {
        let now: String = conn.query_row(
            "SELECT id FROM revision WHERE object_id=?1 ORDER BY seq DESC LIMIT 1",
            [&input.object],
            |r| r.get(0),
        )?;
        let binding =
            if has_bindings {
                conn.query_row(
                "SELECT authority,locator,binding_revision FROM external_binding WHERE revision=?1",
                [&input.revision],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
            ).optional()?
            } else {
                None
            };
        // Bind current scopes, not the current head ID. A later ordinary edit
        // must leave an otherwise eligible historical snapshot usable.
        hash_bytes(
            hasher,
            &serde_json::to_vec(&(
                required_scopes(conn, &input.revision)?,
                required_scopes(conn, &now)?,
                binding,
            ))?,
        );
    }
    Ok(())
}

struct CheckedVectors<'a> {
    data: &'a HybridVectors,
    query_norm: f64,
    passage_norms: Vec<f64>,
}

impl<'a> CheckedVectors<'a> {
    fn new(prepared: &PreparedHybrid, data: &'a HybridVectors) -> Result<Self> {
        ensure!(
            prepared.readiness == HybridReadiness::Ready,
            "prepared hybrid corpus does not accept vectors"
        );
        ensure!(
            data.profile_id == prepared.options.profile_id,
            "hybrid encoding profile mismatch"
        );
        ensure!(
            data.passages.len() == prepared.passages.len(),
            "hybrid passage vector count mismatch"
        );
        let norm = |vector: &[f32]| -> Result<f64> {
            ensure!(
                vector.len() == prepared.options.dimensions && vector.iter().all(|c| c.is_finite()),
                "invalid hybrid vector dimensions or coordinates"
            );
            let squared = vector.iter().map(|&c| f64::from(c).powi(2)).sum::<f64>();
            ensure!(squared > 0., "hybrid vector has zero norm");
            Ok(squared.sqrt())
        };
        let query_norm = norm(&data.query)?;
        let passage_norms = data
            .passages
            .iter()
            .map(|v| norm(v))
            .collect::<Result<_>>()?;
        Ok(Self {
            data,
            query_norm,
            passage_norms,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Scores {
    bm25: Option<f64>,
    cosine: Option<f64>,
    rrf: Option<f64>,
}

impl From<f64> for Scores {
    fn from(bm25: f64) -> Self {
        Self {
            bm25: Some(bm25),
            ..Self::default()
        }
    }
}

fn dense_rows(passages: &[HybridPassage], vectors: CheckedVectors<'_>) -> Vec<RankedRow<Scores>> {
    let mut by_revision = BTreeMap::<_, RankedRow<Scores>>::new();
    for ((passage, vector), norm) in passages
        .iter()
        .zip(&vectors.data.passages)
        .zip(vectors.passage_norms)
    {
        let dot = vectors
            .data
            .query
            .iter()
            .zip(vector)
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum::<f64>();
        let cosine = dot / (vectors.query_norm * norm);
        let row = by_revision
            .entry(&passage.reference.revision)
            .or_insert_with(|| RankedRow {
                reference: passage.reference.clone(),
                sequence: passage.sequence,
                score: Scores {
                    cosine: Some(cosine),
                    ..Scores::default()
                },
            });
        row.score.cosine = row.score.cosine.map(|previous| previous.max(cosine));
    }
    let mut rows: Vec<_> = by_revision.into_values().collect();
    sort_descending(&mut rows, |score| score.cosine.unwrap());
    rows
}

fn fused_rows(
    dense: Vec<RankedRow<Scores>>,
    lexical: impl Iterator<Item = Result<RankedRow<f64>>>,
) -> Result<Vec<RankedRow<Scores>>> {
    let weight = |index: usize| 1. / (RRF_K + index + 1) as f64;
    let mut rows = BTreeMap::new();
    for (index, mut row) in dense.into_iter().enumerate() {
        row.score.rrf = Some(weight(index));
        rows.insert(row.reference.revision.clone(), row);
    }
    for (index, row) in lexical.enumerate() {
        let RankedRow {
            reference,
            sequence,
            score,
        } = row?;
        let entry = rows
            .entry(reference.revision.clone())
            .or_insert_with(|| RankedRow {
                reference,
                sequence,
                score: Scores::default(),
            });
        entry.score.bm25 = Some(score);
        entry.score.rrf = Some(entry.score.rrf.unwrap_or_default() + weight(index));
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    sort_descending(&mut rows, |score| score.rrf.unwrap());
    Ok(rows)
}

fn sort_descending(rows: &mut [RankedRow<Scores>], score: impl Fn(Scores) -> f64) {
    rows.sort_by(|a, b| {
        score(b.score)
            .partial_cmp(&score(a.score))
            .expect("ranking scores are finite")
            .then_with(|| b.sequence.cmp(&a.sequence))
            .then_with(|| a.reference.revision.cmp(&b.reference.revision))
    });
}

#[cfg(test)]
#[path = "hybrid_tests.rs"]
mod tests;
