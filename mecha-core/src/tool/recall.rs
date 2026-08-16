//! Recall: search this conversation's full recorded history.
//!
//! Compaction trades the middle of a transcript for a summary, and a summary
//! preserves what the summariser thought mattered. When the run later needs a
//! detail the summary dropped — the exact figure a tool returned twenty turns
//! ago, the wording of an instruction — its options today are to re-run the
//! tool or re-live the whole stretch, which is the loop the post-compaction
//! guard exists to stop. Recall gives that moment somewhere to go: the session
//! transcript already holds everything the conversation ever contained, so the
//! model can look the detail up instead of reconstructing it.
//!
//! Two properties make this safe to hand to the model:
//!
//! - **It is taint-neutral by construction.** Everything in the transcript
//!   entered *this* conversation once already, and taint is a property of the
//!   conversation — recorded when the content arrived, merged back on resume,
//!   never un-armed by compaction. Re-surfacing recorded content therefore
//!   cannot change what the interlock knows, which is why the tool declares no
//!   capabilities and its output is never marked `from_outside`: the bytes may
//!   include third-party text, but this result came from our own store, and
//!   the arrival that mattered was already accounted.
//! - **The transcript path is the operator's, never the model's.** It is fixed
//!   at registration to the session this conversation is recorded in, so there
//!   is no path argument to resolve and no way to point the tool at another
//!   session — recall over a *different* conversation's transcript would
//!   re-surface content whose taint lives on a conversation that is not this
//!   one, which is exactly the laundering the fixed path forecloses. Register
//!   it only on the conversation the transcript records.
//!
//! Coverage: everything from earlier runs — every prior chat turn, every
//! previous firing — which for long-lived sessions is precisely what
//! compaction removes. Turns a *mid-run* compaction replaced reach the file
//! too: the loop keeps each pre-rewrite state on the conversation
//! ([`Conversation::rewritten`]) and `Session::record_run` walks them at run
//! end. The one thing the corpus lags on is the current run itself —
//! recording happens when it finishes — and those turns are the ones still
//! in context, so the lag costs recall nothing it was for.
//!
//! [`Conversation::rewritten`]: crate::agent::Conversation

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::message::{Block, Message, Role};
use crate::session::Record;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;

/// Matches returned per call unless the model asks for fewer. Enough to be
/// useful, small enough that the interesting case — one needle — stays
/// readable; the turn's output budget (and the spill behind it) still caps
/// the pathological query.
const DEFAULT_MAX_MATCHES: usize = 20;

/// Lines of context on each side of a matching line.
const CONTEXT_LINES: usize = 2;

pub struct Recall {
    transcript: PathBuf,
}

impl Recall {
    pub fn new(transcript: PathBuf) -> Self {
        Recall { transcript }
    }
}

#[async_trait]
impl Tool for Recall {
    fn name(&self) -> &str {
        "recall"
    }

    fn description(&self) -> &str {
        "Search this conversation's full recorded history — including turns that were \
         summarized away by compaction — for a case-insensitive literal string. Use it when \
         an earlier detail (a value a tool returned, an instruction's exact wording) is no \
         longer in context: searching the record is cheaper and more faithful than re-running \
         the tool or reconstructing from memory. Returns matching lines with surrounding \
         context, oldest first."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Case-insensitive literal text to search for. Not a regex."
                },
                "max_matches": {
                    "type": "integer",
                    "description": "Maximum matching blocks to return (default 20)."
                }
            },
            "required": ["query"]
        })
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Capabilities {
        // Deliberately none — see the module docs. The transcript's content
        // already entered this conversation, and its taint entered with it.
        Capabilities::default()
    }

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let query = match input.get("query").and_then(Value::as_str) {
            Some(q) if !q.trim().is_empty() => q.to_string(),
            _ => {
                return Ok(ToolOutput::err(
                    "missing or empty required argument `query`",
                ))
            }
        };
        let max_matches = input
            .get("max_matches")
            .and_then(Value::as_u64)
            .map(|n| n.max(1) as usize)
            .unwrap_or(DEFAULT_MAX_MATCHES);

        let text = match tokio::fs::read_to_string(&self.transcript).await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "cannot read the session transcript ({e}); this conversation may not \
                     be recording, in which case there is no history beyond what is in \
                     context"
                )))
            }
        };

        let messages = every_message_ever(&text);
        let (rendered, matched, capped) = search(&messages, &query, max_matches);

        if matched == 0 {
            return Ok(ToolOutput::ok(format!(
                "no matches for {query:?} in {} recorded messages. The record covers \
                 completed runs of this session; the current run's turns are still in \
                 context rather than in the record.",
                messages.len()
            )));
        }

        let mut out = format!(
            "{matched} matching block(s) for {query:?} across {} recorded messages, \
             oldest first:\n\n{rendered}",
            messages.len()
        );
        if capped > 0 {
            out.push_str(&format!(
                "\n[{capped} more matching block(s) not shown — narrow the query, or \
                 raise max_matches]"
            ));
        }
        Ok(ToolOutput::ok(out))
    }
}

/// Every message the conversation ever contained, in first-seen order.
///
/// `Message` records are the append-only common case. A `Rewrite` record is a
/// compaction (or eviction, or thinning) replacing the list in place — for
/// *loading* a session the replacement is the truth, but for recall the whole
/// point is what the replacement dropped, so its messages are unioned in
/// rather than substituted: anything new (the summary, an edited result)
/// joins the corpus, anything already seen is skipped. Malformed lines are
/// skipped exactly as [`crate::session::Session::load`] skips them — a
/// truncated final line is the normal residue of a killed process.
fn every_message_ever(transcript: &str) -> Vec<Message> {
    let mut seen = HashSet::new();
    let mut all = Vec::new();
    let mut admit = |m: Message, all: &mut Vec<Message>| {
        // Equality via the serialized form: `Message` is `PartialEq` but not
        // `Hash`, and the serialization is already the file's own currency.
        if let Ok(key) = serde_json::to_string(&m) {
            if seen.insert(key) {
                all.push(m);
            }
        }
    };
    for line in transcript.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Record>(line) {
            Ok(Record::Message(m)) => admit(m, &mut all),
            Ok(Record::Rewrite { messages }) => {
                for m in messages {
                    admit(m, &mut all);
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "recall: skipping malformed transcript line"),
        }
    }
    all
}

/// The searchable text of a block, with a label saying what kind of thing
/// matched — a value found in a tool result and the same value found in the
/// model's own thinking carry different weight, and the label is what lets
/// the model tell them apart.
fn block_text(block: &Block) -> (&'static str, String) {
    match block {
        Block::Text { text } => ("text", text.clone()),
        Block::Thinking { text, .. } => ("thinking", text.clone()),
        Block::ToolUse { name, input, .. } => ("tool_use", format!("{name} {input}")),
        Block::ToolResult { content, .. } => ("tool_result", content.clone()),
    }
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Search the corpus. Returns (rendered matches, matched-block count shown,
/// matching blocks beyond the cap).
fn search(messages: &[Message], query: &str, max_matches: usize) -> (String, usize, usize) {
    let needle = query.to_lowercase();
    let mut rendered = Vec::new();
    let mut shown = 0usize;
    let mut beyond = 0usize;

    for (idx, message) in messages.iter().enumerate() {
        for block in &message.content {
            let (kind, text) = block_text(block);
            let windows = matching_windows(&text, &needle);
            if windows.is_empty() {
                continue;
            }
            if shown >= max_matches {
                beyond += 1;
                continue;
            }
            shown += 1;
            let lines: Vec<&str> = text.lines().collect();
            let mut body = String::new();
            for (start, end) in &windows {
                if !body.is_empty() {
                    body.push_str("  ⋮\n");
                }
                for line in &lines[*start..*end] {
                    body.push_str("  ");
                    body.push_str(line);
                    body.push('\n');
                }
            }
            rendered.push(format!(
                "[message {idx} · {} · {kind}]\n{body}",
                role_name(&message.role)
            ));
        }
    }
    (rendered.join("\n"), shown, beyond)
}

/// Half-open line ranges around each matching line, overlapping ranges
/// merged so a cluster of hits reads as one excerpt instead of repeating
/// itself.
fn matching_windows(text: &str, lowercase_needle: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.to_lowercase().contains(lowercase_needle) {
            continue;
        }
        let start = i.saturating_sub(CONTEXT_LINES);
        let end = (i + CONTEXT_LINES + 1).min(lines.len());
        match windows.last_mut() {
            Some((_, prev_end)) if start <= *prev_end => *prev_end = end,
            _ => windows.push((start, end)),
        }
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;
    use crate::session::{Record, SessionMeta};
    use crate::tool::ToolCtx;

    fn write_transcript(records: &[Record]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("mecha-recall-{}.jsonl", uuid::Uuid::new_v4()));
        let body: String = records
            .iter()
            .map(|r| serde_json::to_string(r).unwrap() + "\n")
            .collect();
        std::fs::write(&path, body).unwrap();
        path
    }

    fn meta() -> Record {
        Record::Meta(SessionMeta {
            id: "recall-test".into(),
            created_at: chrono::Utc::now(),
            provider: "scripted".into(),
            model: "none".into(),
            workspace: std::env::temp_dir(),
            title: None,
        })
    }

    fn ctx() -> ToolCtx {
        ToolCtx::default().with_workspace(std::env::temp_dir())
    }

    async fn run(tool: &Recall, input: Value) -> ToolOutput {
        tool.call(input, &ctx()).await.unwrap()
    }

    /// The reason the tool exists: content a compaction rewrite dropped is
    /// still found, because the corpus is the union of everything ever
    /// recorded, not the post-rewrite state a `load` would return.
    #[tokio::test]
    async fn finds_content_a_rewrite_dropped() {
        let dropped = Message::assistant(vec![Block::text("the magic number is 74656")]);
        let path = write_transcript(&[
            meta(),
            Record::Message(Message::user("compute the magic number")),
            Record::Message(dropped),
            Record::Rewrite {
                messages: vec![Message::user("[summary: a number was computed]")],
            },
        ]);
        let tool = Recall::new(path);

        let out = run(&tool, json!({"query": "74656"})).await;
        assert!(!out.is_error);
        assert!(
            out.content.contains("74656"),
            "dropped content not found: {}",
            out.content
        );
        assert!(
            out.content.contains("assistant"),
            "match not attributed: {}",
            out.content
        );

        // The rewrite's own additions are searchable too.
        let out = run(&tool, json!({"query": "summary:"})).await;
        assert!(out.content.contains("[summary:"));
    }

    /// A message recorded once and repeated verbatim inside a rewrite is one
    /// corpus entry, not two — otherwise every compaction would double every
    /// surviving message's matches.
    #[tokio::test]
    async fn a_rewritten_duplicate_matches_once() {
        let kept = Message::user("the anchor phrase");
        let path = write_transcript(&[
            meta(),
            Record::Message(kept.clone()),
            Record::Rewrite {
                messages: vec![kept],
            },
        ]);
        let out = run(&Recall::new(path), json!({"query": "anchor phrase"})).await;
        assert!(
            out.content.starts_with("1 matching block(s)"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn matching_is_case_insensitive_and_labelled_by_block_kind() {
        let path = write_transcript(&[
            meta(),
            Record::Message(Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "Quarterly Total: $12,345".into(),
                is_error: false,
            }])),
        ]);
        let out = run(&Recall::new(path), json!({"query": "quarterly total"})).await;
        assert!(!out.is_error);
        assert!(out.content.contains("tool_result"), "{}", out.content);
        assert!(out.content.contains("$12,345"));
    }

    #[tokio::test]
    async fn zero_matches_reports_the_corpus_size_not_an_error() {
        let path = write_transcript(&[meta(), Record::Message(Message::user("hello"))]);
        let out = run(&Recall::new(path), json!({"query": "absent"})).await;
        assert!(!out.is_error);
        assert!(out.content.contains("no matches"));
        assert!(out.content.contains("1 recorded messages"));
    }

    #[tokio::test]
    async fn a_missing_transcript_is_an_expected_failure() {
        let tool = Recall::new(std::env::temp_dir().join("mecha-recall-nonexistent.jsonl"));
        let out = run(&tool, json!({"query": "anything"})).await;
        assert!(out.is_error);
        assert!(out.content.contains("not be recording"));
    }

    #[tokio::test]
    async fn an_empty_query_is_refused() {
        let path = write_transcript(&[meta()]);
        let out = run(&Recall::new(path), json!({"query": "  "})).await;
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn the_match_cap_reports_what_it_hid() {
        let records: Vec<Record> = std::iter::once(meta())
            .chain((0..5).map(|i| Record::Message(Message::user(format!("needle row {i}")))))
            .collect();
        let path = write_transcript(&records);
        let out = run(
            &Recall::new(path),
            json!({"query": "needle", "max_matches": 2}),
        )
        .await;
        assert!(
            out.content.contains("2 matching block(s)"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("3 more matching block(s)"),
            "{}",
            out.content
        );
    }
}
