//! Posting, updating, and streaming.
//!
//! The streaming trio (`chat.startStream` / `appendStream` / `stopStream`) is
//! what makes a long run watchable without a `chat.update` loop fighting the
//! one-message-per-second-per-channel posting rule. Its rate limits say the
//! same thing: start and stop are Tier 2 while append is Tier 4, which is
//! Slack expecting many appends per stream.
//!
//! **Unfurling is off on everything this module posts, and there is no
//! parameter to turn it on.** Slack's own security guidance names unfurling as
//! the step that issues "the immediate, unauthorized HTTP request that would
//! complete the data exfiltration" — a model-emitted URL becomes an outbound
//! GET that no tool call ever made and no interlock ever saw. It is the same
//! reasoning that makes `http_fetch` a send sink despite being read-only, and
//! making it a function of the transport rather than an argument is what stops
//! it being forgotten at one call site.

use serde_json::{json, Value};

use crate::blocks;
use crate::error::{SlackError, SlackResult};
use crate::http::Slack;

/// A message's timestamp, which is also its id within a channel.
pub type Ts = String;

/// One unit of a live stream.
#[derive(Debug, Clone)]
pub enum Chunk {
    /// Model prose.
    Markdown(String),
    /// One step of the run's progress — a tool call, a compaction, a denial.
    ///
    /// `id` is what makes this a *transition* rather than a new line: Slack
    /// keys the task card on it, so the same id sent again with a new status
    /// updates the card in place. A fresh id per update would render a call's
    /// lifecycle as three unrelated entries.
    Task {
        id: String,
        title: String,
        status: TaskStatus,
    },
    /// Terminal blocks, sent with `chat.stopStream`.
    Blocks(Vec<Value>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Complete,
    Error,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Complete => "complete",
            TaskStatus::Error => "error",
        }
    }
}

impl Chunk {
    /// The wire form, with each field held to its own documented cap.
    pub fn to_json(&self) -> Value {
        match self {
            // Inside a `chunks` array the text field is `text`; the
            // `markdown_text` spelling is the *top-level* argument. They are
            // not interchangeable.
            Chunk::Markdown(text) => json!({
                "type": "markdown_text",
                "text": blocks::truncate(text, blocks::limits::STREAM_MARKDOWN),
            }),
            // Flat, not nested under a `task_update` key. Nesting it is
            // `invalid_arguments`, which is what shipping a guessed shape
            // bought — the prose streamed fine and every task update was
            // refused, so fixtures had nothing to disagree with.
            Chunk::Task { id, title, status } => json!({
                "type": "task_update",
                "id": id,
                "title": blocks::truncate(title, blocks::limits::TASK_TEXT),
                "status": status.as_str(),
            }),
            Chunk::Blocks(b) => json!({
                "type": "blocks",
                "blocks": blocks::cap_blocks(b.clone()),
            }),
        }
    }
}

/// Post a message. Always in a thread when `thread_ts` is given, and never
/// unfurling.
pub async fn post_message(
    slack: &Slack,
    channel: &str,
    thread_ts: Option<&str>,
    text: &str,
    blocks: Option<Vec<Value>>,
) -> SlackResult<Ts> {
    let mut body = json!({
        "channel": channel,
        "text": text,
        "unfurl_links": false,
        "unfurl_media": false,
    });
    if let Some(thread_ts) = thread_ts {
        body["thread_ts"] = json!(thread_ts);
    }
    if let Some(b) = blocks {
        body["blocks"] = json!(blocks::cap_blocks(b));
    }
    let v: Value = slack.call("chat.postMessage", body).await?;
    read_ts("chat.postMessage", &v)
}

/// Rewrite a message in place. This is how an approval card becomes a terminal
/// "approved by @x" record that cannot be clicked again.
pub async fn update(
    slack: &Slack,
    channel: &str,
    ts: &str,
    text: &str,
    blocks_in: Option<Vec<Value>>,
) -> SlackResult<()> {
    let mut body = json!({
        "channel": channel,
        "ts": ts,
        "text": text,
        "unfurl_links": false,
        "unfurl_media": false,
    });
    if let Some(b) = blocks_in {
        body["blocks"] = json!(blocks::cap_blocks(b));
    }
    let _: Value = slack.call("chat.update", body).await?;
    Ok(())
}

/// Open a stream as a reply to the message that asked for it.
pub async fn start_stream(slack: &Slack, channel: &str, thread_ts: &str) -> SlackResult<Ts> {
    let v: Value = slack
        .call(
            "chat.startStream",
            json!({ "channel": channel, "thread_ts": thread_ts }),
        )
        .await?;
    read_ts("chat.startStream", &v)
}

pub async fn append_stream(
    slack: &Slack,
    channel: &str,
    ts: &str,
    chunk: &Chunk,
) -> SlackResult<()> {
    // **Everything goes through `chunks`, including prose.** A stream has one
    // mode: send a `task_update` chunk and then a top-level `markdown_text`
    // and Slack answers `streaming_mode_mismatch`. That was live here, and it
    // hid until the task chunks started working — while they were being
    // rejected the stream stayed in markdown mode and the prose was fine, so
    // fixing one bug is what exposed the other.
    let body = json!({ "channel": channel, "ts": ts, "chunks": [chunk.to_json()] });
    let _: Value = slack.call("chat.appendStream", body).await?;
    Ok(())
}

/// Close a stream, optionally with final blocks.
///
/// **Always call this.** A stream left open is indistinguishable from a run
/// still working, which is the wedged-versus-waiting confusion the whole design
/// is trying to avoid.
pub async fn stop_stream(
    slack: &Slack,
    channel: &str,
    ts: &str,
    blocks_in: Option<Vec<Value>>,
) -> SlackResult<()> {
    let mut body = json!({ "channel": channel, "ts": ts });
    if let Some(b) = blocks_in {
        body["blocks"] = json!(blocks::cap_blocks(b));
    }
    let _: Value = slack.call("chat.stopStream", body).await?;
    Ok(())
}

/// The "…is thinking" line, for the gap before the first token.
///
/// Not a send sink: it writes only to the requesting user's own view, sends no
/// message, and **clears itself after two minutes**. That expiry is a feature —
/// a connector that dies mid-run cannot leave a spinner going forever.
pub async fn set_status(
    slack: &Slack,
    channel: &str,
    thread_ts: &str,
    status: &str,
) -> SlackResult<()> {
    let _: Value = slack
        .call(
            "assistant.threads.setStatus",
            json!({ "channel_id": channel, "thread_ts": thread_ts, "status": status }),
        )
        .await?;
    Ok(())
}

fn read_ts(method: &str, v: &Value) -> SlackResult<Ts> {
    v.get("ts")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| SlackError::Malformed {
            method: method.to_string(),
            detail: "no `ts` on an ok response".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_chunk_is_flat_and_carries_its_id() {
        // Both halves were wrong when this was guessed rather than read, and
        // Slack answered `invalid_arguments` for the nesting while saying
        // nothing about the missing id — which would have rendered every call
        // as three unrelated lines instead of one card changing state.
        let c = Chunk::Task {
            id: "toolu_01".into(),
            title: "shell: cargo test".into(),
            status: TaskStatus::InProgress,
        };
        let v = c.to_json();
        assert_eq!(v["type"], "task_update");
        assert_eq!(v["status"], "in_progress", "status is flat on the chunk");
        assert_eq!(v["id"], "toolu_01", "the id is what makes it a transition");
        assert!(
            v.get("task_update").is_none(),
            "no nested object: that spelling is invalid_arguments"
        );
    }

    #[test]
    fn a_markdown_chunk_uses_the_chunk_spelling_of_its_text_field() {
        let v = Chunk::Markdown("hi".into()).to_json();
        assert_eq!(v["text"], "hi");
        assert!(
            v.get("markdown_text").is_none(),
            "`markdown_text` is the top-level argument, not the chunk field"
        );
    }

    #[test]
    fn a_long_task_title_is_cut_to_slacks_limit() {
        let c = Chunk::Task {
            id: "t".into(),
            title: "x".repeat(500),
            status: TaskStatus::Complete,
        };
        let title = c.to_json()["title"].as_str().unwrap().to_string();
        assert!(title.chars().count() <= blocks::limits::TASK_TEXT);
    }

    #[test]
    fn a_markdown_chunk_is_held_to_the_per_call_cap() {
        let c = Chunk::Markdown("y".repeat(20_000));
        let text = c.to_json()["text"].as_str().unwrap().to_string();
        assert!(text.chars().count() <= blocks::limits::STREAM_MARKDOWN);
    }

    #[test]
    fn a_blocks_chunk_is_capped_like_a_message() {
        let many: Vec<Value> = (0..60).map(|i| blocks::section(&i.to_string())).collect();
        let v = Chunk::Blocks(many).to_json();
        assert_eq!(
            v["blocks"].as_array().unwrap().len(),
            blocks::limits::BLOCKS_PER_MESSAGE
        );
    }

    #[test]
    fn every_chunk_kind_serialises_into_the_same_mode() {
        // The mode is a property of the stream, not of the call: mixing a
        // `chunks` array with a top-level `markdown_text` is
        // `streaming_mode_mismatch`. Nothing in this crate may send the
        // top-level form.
        for chunk in [
            Chunk::Markdown("hi".into()),
            Chunk::Task {
                id: "t".into(),
                title: "x".into(),
                status: TaskStatus::Complete,
            },
            Chunk::Blocks(vec![blocks::section("b")]),
        ] {
            let body = json!({ "channel": "C", "ts": "1", "chunks": [chunk.to_json()] });
            assert!(
                body.get("markdown_text").is_none(),
                "prose must ride in a chunk like everything else"
            );
            assert!(body["chunks"].as_array().is_some_and(|c| c.len() == 1));
        }
    }

    #[test]
    fn read_ts_refuses_a_response_without_one() {
        assert!(read_ts("chat.postMessage", &json!({"ok": true})).is_err());
        assert_eq!(
            read_ts("chat.postMessage", &json!({"ok": true, "ts": "1.2"})).unwrap(),
            "1.2"
        );
    }
}
