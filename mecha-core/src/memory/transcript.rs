//! Native Mecha JSONL capture. Reading a session is not replaying its messages:
//! retain every physical record, index only known original message text fields.
use super::{sql::*, store::capture_in, *};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::{collections::BTreeSet, io::Read};

const MAX_FILE: u64 = 32 * 1024 * 1024;

pub fn transcript_source(account: &str, session: &str, line: usize) -> SourceKey {
    SourceKey {
        connector: "mecha-session-jsonl".into(),
        account: account.into(),
        id: serde_json::json!([session, line]).to_string(),
    }
}

impl Memory {
    /// Explicit operator-selected, synthetic input only. Atomic append/retry;
    /// rejects edited/truncated prefixes. Raw records include their line ending.
    pub fn import_session_jsonl(
        &mut self,
        reader: impl Read,
        account: &str,
        scopes: BTreeSet<String>,
    ) -> Result<ImportReport> {
        ensure!(
            !account.trim().is_empty() && account.len() <= 256,
            "invalid account"
        );
        validate_scopes(&scopes)?;
        let mut raw = String::new();
        reader.take(MAX_FILE + 1).read_to_string(&mut raw)?;
        ensure!(raw.len() as u64 <= MAX_FILE, "session exceeds 32 MiB");
        ensure!(
            raw.ends_with('\n'),
            "session must end in a complete newline-terminated record"
        );
        let mut inputs = Vec::new();
        let mut session = String::new();
        for (index, line) in raw.split_inclusive('\n').enumerate() {
            ensure!(
                index < 100_000 && line.len() <= 1_000_000,
                "session record limit exceeded"
            );
            // Do not include source text in errors (including serde's snippets).
            let v: Value = serde_json::from_str(line)
                .map_err(|_| anyhow::anyhow!("invalid JSON at record {}", index + 1))?;
            let kind = v
                .get("record")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .with_context(|| format!("missing record kind at record {}", index + 1))?;
            if index == 0 {
                ensure!(kind == "meta", "first record must be native session meta");
                session = v
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty() && s.len() <= 256)
                    .context("invalid session ID")?
                    .to_owned();
            } else {
                ensure!(kind != "meta", "multiple session headers are not supported");
            }
            let occurred_at = if kind == "meta" {
                Some(normalize_time(
                    v.get("created_at")
                        .and_then(Value::as_str)
                        .context("missing session creation timestamp")?,
                )?)
            } else if kind == "message" {
                // Native messages have no timestamp today. Accept an explicit
                // timestamp when supplied, never inherit the session's clock.
                v.get("timestamp")
                    .map(|t| normalize_time(t.as_str().context("invalid record timestamp")?))
                    .transpose()?
            } else {
                // Future record kinds may use another timestamp representation.
                // Retain their bytes without assigning semantics to unknown fields.
                None
            };
            let speaker = if kind == "message" {
                Some(
                    v.get("role")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty() && s.len() <= 128)
                        .context("message has no speaker")?
                        .to_owned(),
                )
            } else {
                None
            };
            let mut fields = Vec::new();
            if kind == "message" {
                let blocks = v
                    .get("content")
                    .and_then(Value::as_array)
                    .context("message content must be an array")?;
                for (i, block) in blocks.iter().enumerate() {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .context("text block must contain a string")?;
                        fields.push(TextField {
                            pointer: format!("/content/{i}/text"),
                            text: text.into(),
                        });
                    }
                }
            }
            inputs.push(Capture {
                source: transcript_source(account, &session, index + 1),
                text: line.into(),
                occurred_at,
                scopes: scopes.clone(),
                transcript: Some(TranscriptMeta {
                    session_id: session.clone(),
                    line: index + 1,
                    record_type: kind.into(),
                    speaker,
                    fields,
                }),
            });
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous_lines: i64 = tx.query_row(
            "SELECT coalesce(max(json_extract(r.payload,'$.data.transcript.line')),0)
             FROM source_item s JOIN revision r ON r.object_id=s.object_id
             WHERE json_extract(r.payload,'$.data.source.connector')='mecha-session-jsonl'
             AND json_extract(r.payload,'$.data.source.account')=?1
             AND json_extract(r.payload,'$.data.transcript.session_id')=?2",
            params![account, session],
            |r| r.get(0),
        )?;
        ensure!(
            inputs.len() as i64 >= previous_lines,
            "session prefix was truncated"
        );
        let mut report = ImportReport {
            session_id: session,
            inserted: 0,
            unchanged: 0,
            updated_scopes: 0,
            records: Vec::new(),
        };
        for input in inputs {
            let digest = source_digest(&self.instance, &input.source)?;
            let object: Option<String> = tx
                .query_row(
                    "SELECT object_id FROM source_item WHERE digest=?1",
                    [&digest],
                    |r| r.get(0),
                )
                .optional()?;
            let previous = object
                .map(|id| head(&tx, &id, i64::MAX))
                .transpose()?
                .flatten();
            if let Some((_, Record::Evidence(ref old), _)) = previous {
                ensure!(
                    old.text == input.text,
                    "session prefix changed; use a distinct source session for a rewritten export"
                );
            }
            let reference = capture_in(&tx, &self.instance, input)?;
            match previous {
                None => report.inserted += 1,
                Some((ref old, _, _)) if *old == reference => report.unchanged += 1,
                Some(_) => report.updated_scopes += 1,
            }
            report.records.push(reference);
        }
        tx.commit()?;
        Ok(report)
    }
}
