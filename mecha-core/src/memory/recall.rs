//! Evidence-only BM25 baseline with source-revision receipts and bounded windows.
use super::{sql::*, *};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct RecallQuery {
    pub text: String,
    pub known_at: Snapshot,
    pub account: Option<String>,
    pub session_id: Option<String>,
    /// Half-open occurrence interval; unknown timestamps are excluded if set.
    pub occurred_after: Option<String>,
    pub occurred_before: Option<String>,
    pub limit: usize,
    pub neighbors: usize,
    /// Total serialized Hit bytes (anchors + context); whole records only.
    pub max_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct RecallHit {
    /// Lower BM25 values rank earlier. This is not a confidence probability.
    pub bm25: f64,
    pub anchor: Hit,
    pub context: Vec<Hit>,
}

/// Internal diagnostic; public recall callers retain their original result shape.
pub(super) struct RecallBatch {
    pub hits: Vec<RecallHit>,
    pub budget_omitted: bool,
}

impl Memory {
    pub fn recall(&self, query: &RecallQuery, access: &Access) -> Result<Vec<RecallHit>> {
        self.recall_filtered(query, None, access)
    }

    /// Require a literal name/phrase in each anchor before top-k selection.
    /// unicode61 normalizes case, diacritics and punctuation, without stemming.
    /// This is mention matching, not entity/alias resolution. Neighbors need not match.
    pub fn recall_mention(
        &self,
        query: &RecallQuery,
        mention: &str,
        access: &Access,
    ) -> Result<Vec<RecallHit>> {
        let expression = mention_expression(mention)?;
        self.recall_filtered(query, Some(&expression), access)
    }

    fn recall_filtered(
        &self,
        query: &RecallQuery,
        mention: Option<&str>,
        access: &Access,
    ) -> Result<Vec<RecallHit>> {
        let tx = self.conn.unchecked_transaction()?;
        let hits = self.recall_in(query, mention, access, &tx)?;
        tx.commit()?;
        Ok(hits.hits)
    }

    /// Shared reader for a complete context assembled in one read transaction.
    pub(super) fn recall_in(
        &self,
        query: &RecallQuery,
        mention: Option<&str>,
        access: &Access,
        tx: &Connection,
    ) -> Result<RecallBatch> {
        self.check_access(access)?;
        ensure!(
            query.limit > 0 && query.limit <= 100,
            "limit must be 1..=100"
        );
        ensure!(query.neighbors <= 8, "neighbors must be 0..=8");
        ensure!(
            query.max_bytes > 0 && query.max_bytes <= 1_000_000,
            "evidence budget must be 1..=1000000 bytes"
        );
        ensure!(
            !query.text.trim().is_empty() && query.text.len() <= 4096,
            "invalid recall text"
        );
        ensure!(
            query.session_id.is_none() || query.account.is_some(),
            "session filter requires account"
        );
        let mut query = query.clone();
        query.occurred_after = query
            .occurred_after
            .as_deref()
            .map(normalize_time)
            .transpose()?;
        query.occurred_before = query
            .occurred_before
            .as_deref()
            .map(normalize_time)
            .transpose()?;
        if let (Some(a), Some(b)) = (&query.occurred_after, &query.occurred_before) {
            ensure!(a < b, "occurrence interval must be nonempty");
        }
        // Literal terms only; OR admits partial overlap for a ranked baseline.
        let mut expression = fts_expression(&query.text)?.replace(" AND ", " OR ");
        if let Some(name) = mention {
            // Descriptive terms rank results but cannot exclude a named record.
            expression = format!("({expression}) OR {name}");
        }
        ensure!(
            query.known_at.instance == self.instance
                && query.known_at.sequence >= 0
                && query.known_at.sequence <= sequence(tx)?,
            "invalid or foreign snapshot"
        );
        // FTS5's built-in rank uses collection-wide statistics. A WHERE
        // clause on the persistent index cannot isolate historical/scope
        // statistics, so score a disposable eligible-corpus index in RAM.
        let scorer = eligible_index(tx, &query, access, mention.is_some())?;
        let sql = if mention.is_some() {
            "SELECT revision,object_id,seq,bm25(evidence) FROM evidence
             WHERE evidence MATCH ?1 AND revision IN
               (SELECT revision FROM mentions WHERE mentions MATCH ?2)
             ORDER BY bm25(evidence),CAST(seq AS INTEGER) DESC,revision"
        } else {
            "SELECT revision,object_id,seq,bm25(evidence) FROM evidence
             WHERE evidence MATCH ?1 AND ?2 IS NULL
             ORDER BY bm25(evidence),CAST(seq AS INTEGER) DESC,revision"
        };
        let mut statement = scorer.prepare(sql)?;
        let rows = statement.query_map(params![expression, mention], |r| {
            Ok((
                RevisionRef {
                    revision: r.get(0)?,
                    object: r.get(1)?,
                },
                r.get::<_, i64>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?;
        let mut hits = Vec::new();
        let mut budget_omitted = false;
        let mut remaining = query.max_bytes;
        for row in rows {
            let (reference, seq, score) = row?;
            let Some(record) = lookup(tx, &reference)? else {
                continue;
            };
            let Some(anchor) = eligible(tx, reference, seq, record, &query, access)? else {
                continue;
            };
            let size = serde_json::to_vec(&anchor)?.len();
            if size > remaining {
                budget_omitted = true;
                continue;
            }
            remaining -= size;
            hits.push(RecallHit {
                bm25: score,
                anchor,
                context: Vec::new(),
            });
            if hits.len() == query.limit {
                break;
            }
        }
        // Reserve the budget for anchors before spending it on context.
        for hit in &mut hits {
            let Record::Evidence(e) = &hit.anchor.record else {
                unreachable!()
            };
            let Some(meta) = &e.transcript else { continue };
            if query.neighbors == 0 {
                continue;
            }
            let low = meta.line.saturating_sub(query.neighbors);
            let high = meta.line.saturating_add(query.neighbors);
            let mut neighbors = tx.prepare(
                "SELECT r.id,r.object_id,r.seq,r.payload FROM revision r
                 WHERE r.seq<=?1 AND r.lifecycle='active'
                 AND json_extract(r.payload,'$.data.source.connector')='mecha-session-jsonl'
                 AND json_extract(r.payload,'$.data.source.account')=?2
                 AND json_extract(r.payload,'$.data.transcript.session_id')=?3
                 AND json_extract(r.payload,'$.data.transcript.line') BETWEEN ?4 AND ?5
                 AND r.id<>?6 AND NOT EXISTS (SELECT 1 FROM revision newer
                   WHERE newer.object_id=r.object_id AND newer.seq>r.seq AND newer.seq<=?1)
                 ORDER BY abs(json_extract(r.payload,'$.data.transcript.line')-?7),r.seq",
            )?;
            let rows = neighbors.query_map(
                params![
                    query.known_at.sequence,
                    e.source.account,
                    meta.session_id,
                    low as i64,
                    high as i64,
                    hit.anchor.reference.revision,
                    meta.line as i64
                ],
                |r| {
                    Ok((
                        RevisionRef {
                            revision: r.get(0)?,
                            object: r.get(1)?,
                        },
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )?;
            for row in rows {
                let (reference, seq, payload) = row?;
                let record = serde_json::from_str(&payload)?;
                let Some(neighbor) = eligible(tx, reference, seq, record, &query, access)? else {
                    continue;
                };
                let size = serde_json::to_vec(&neighbor)?.len();
                if size <= remaining {
                    remaining -= size;
                    hit.context.push(neighbor);
                } else {
                    budget_omitted = true;
                }
            }
            hit.context.sort_by_key(|h| match &h.record {
                Record::Evidence(e) => e.transcript.as_ref().map(|m| m.line),
                _ => None,
            });
        }
        drop(statement);
        Ok(RecallBatch {
            hits,
            budget_omitted,
        })
    }
}

pub(super) fn eligible(
    conn: &Connection,
    reference: RevisionRef,
    seq: i64,
    record: Record,
    query: &RecallQuery,
    access: &Access,
) -> Result<Option<Hit>> {
    let Record::Evidence(ref e) = record else {
        return Ok(None);
    };
    if e.transcript.as_ref().is_some_and(|m| m.fields.is_empty())
        || query
            .account
            .as_ref()
            .is_some_and(|a| *a != e.source.account)
        || query
            .session_id
            .as_ref()
            .is_some_and(|s| e.transcript.as_ref().is_none_or(|m| m.session_id != *s))
    {
        return Ok(None);
    }
    if query.occurred_after.is_some() || query.occurred_before.is_some() {
        let Some(at) = &e.occurred_at else {
            return Ok(None);
        };
        if query.occurred_after.as_ref().is_some_and(|a| at < a)
            || query.occurred_before.as_ref().is_some_and(|b| at >= b)
        {
            return Ok(None);
        }
    }
    if !authorized(conn, &reference, access)? || !fresh(conn, &reference, query.known_at.sequence)?
    {
        return Ok(None);
    }
    Ok(Some(Hit {
        required_scopes: required_scopes(conn, &reference.revision)?,
        reference,
        sequence: seq,
        record,
        possible_time: false,
        private_data: true,
        untrusted_input: true,
    }))
}

/// Correctness-first prototype: scan indexed evidence and index the eligible
/// corpus per query. No disk scratch or cross-scope/future ranking statistics.
fn eligible_index(
    conn: &Connection,
    query: &RecallQuery,
    access: &Access,
    with_mentions: bool,
) -> Result<Connection> {
    let mut scorer = Connection::open_in_memory()?;
    scorer.execute_batch("PRAGMA temp_store=MEMORY;
        CREATE VIRTUAL TABLE evidence USING fts5(revision UNINDEXED,object_id UNINDEXED,seq UNINDEXED,text,tokenize='porter unicode61');")?;
    if with_mentions {
        scorer.execute_batch("CREATE VIRTUAL TABLE mentions USING fts5(revision UNINDEXED,text,tokenize='unicode61');")?;
    }
    let mut statement = conn.prepare(
        "SELECT r.id,r.object_id,r.seq,r.payload,memory_fts.text
         FROM memory_fts JOIN revision r ON r.id=memory_fts.revision
         JOIN object o ON o.id=r.object_id
         WHERE o.kind='evidence' AND r.lifecycle='active' AND r.seq<=?1
         AND NOT EXISTS (SELECT 1 FROM revision newer
             WHERE newer.object_id=r.object_id AND newer.seq>r.seq AND newer.seq<=?1)",
    )?;
    let rows = statement.query_map([query.known_at.sequence], |r| {
        Ok((
            RevisionRef {
                revision: r.get(0)?,
                object: r.get(1)?,
            },
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let tx = scorer.transaction()?;
    {
        let mut insert = tx.prepare("INSERT INTO evidence VALUES (?1,?2,?3,?4)")?;
        for row in rows {
            let (reference, seq, payload, text) = row?;
            let record = serde_json::from_str(&payload)?;
            if let Some(hit) = eligible(conn, reference, seq, record, query, access)? {
                if with_mentions {
                    tx.execute(
                        "INSERT INTO mentions VALUES (?1,?2)",
                        params![hit.reference.revision, text],
                    )?;
                }
                insert.execute(params![
                    hit.reference.revision,
                    hit.reference.object,
                    seq,
                    text
                ])?;
            }
        }
    }
    tx.commit()?;
    Ok(scorer)
}

// Use the same literal syntax and unicode61 semantics for source validation and recall.
pub(super) fn mention_expression(mention: &str) -> Result<String> {
    ensure!(
        !mention.trim().is_empty() && mention.len() <= 256,
        "invalid required mention"
    );
    fts_expression(mention)?;
    Ok(format!("\"{}\"", mention.replace('"', "\"\"")))
}
pub(super) fn text_has_mention(text: &str, mention: &str) -> Result<bool> {
    let expression = mention_expression(mention)?;
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; CREATE VIRTUAL TABLE visible USING fts5(text, tokenize='unicode61');")?;
    conn.execute("INSERT INTO visible VALUES (?1)", [text])?;
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM visible WHERE visible MATCH ?1)",
        [expression],
        |r| r.get(0),
    )?)
}
