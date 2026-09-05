//! Turning a run into a Slack thread: one stream, one controls message.
//!
//! The mapping is nearly one-to-one because Slack's streaming API was built
//! for this shape — `task_update` chunks are a tool call's lifecycle with the
//! names already chosen. What is *not* mechanical is what gets left out.
//!
//! - **Thinking is dropped.** A thread is a medium other people can read, and
//!   thinking blocks are the least reviewed text the model produces. The TUI
//!   shows them because a terminal has one reader.
//! - **A denial names the layer that refused it.** Interlock, hook, policy or
//!   a person — because a policy refusal the human never sees reads as a
//!   mysterious failure, and the thread is where the human already is.
//! - **`stop_stream` always runs**, on every exit path. A stream left open is
//!   indistinguishable from a run still working, which is the whole confusion
//!   this design exists to prevent.

use std::time::{Duration, Instant};

use mecha_core::agent::{AgentEvent, RunOutcome};
use mecha_slack::chat::{self, Chunk, TaskStatus};
use mecha_slack::{blocks, Slack};
use tokio::sync::mpsc::UnboundedReceiver;

pub struct PumpConfig {
    pub flush_chars: usize,
    pub flush_ms: u64,
}

/// Consume a run's events until the channel closes, rendering them into one
/// streamed message. Returns the stream's `ts` so a caller can refer to it.
pub async fn pump(
    slack: &Slack,
    channel: &str,
    thread_ts: &str,
    mut events: UnboundedReceiver<AgentEvent>,
    cfg: &PumpConfig,
) -> Option<String> {
    let stream_ts = match chat::start_stream(slack, channel, thread_ts).await {
        Ok(ts) => ts,
        Err(e) => {
            tracing::warn!("could not open a Slack stream: {e}");
            // Without a stream there is nowhere to render, but the run is
            // already going: drain so the sender never blocks.
            while events.recv().await.is_some() {}
            return None;
        }
    };

    let mut buffer = String::new();
    let mut last_flush = Instant::now();
    // Task titles by tool_use id, so a result renders under the same wording
    // the call did. Notices have no natural id, so they get a counter — every
    // task card needs a distinct one or they overwrite each other.
    let mut titles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut notice = 0u32;
    let mut outcome: Option<Box<RunOutcome>> = None;

    while let Some(event) = events.recv().await {
        match event {
            // Prose accumulates; everything else flushes it first so the
            // ordering a reader sees matches the ordering that happened.
            AgentEvent::TextDelta(text) => {
                buffer.push_str(&text);
                if buffer.chars().count() >= cfg.flush_chars
                    || last_flush.elapsed() >= Duration::from_millis(cfg.flush_ms)
                {
                    flush(slack, channel, &stream_ts, &mut buffer, &mut last_flush).await;
                }
            }
            AgentEvent::ThinkingDelta(_) | AgentEvent::AssistantText(_) => {}
            // The call and its result share the tool_use id, which is what
            // makes Slack update one task card rather than append a second
            // line. `titles` keeps the wording stable across the transition: a
            // card that renames itself mid-flight reads as a different task.
            AgentEvent::ToolCall { id, name, input } => {
                flush(slack, channel, &stream_ts, &mut buffer, &mut last_flush).await;
                let title = describe(&name, &input);
                titles.insert(id.clone(), title.clone());
                task(
                    slack,
                    channel,
                    &stream_ts,
                    &id,
                    &title,
                    TaskStatus::InProgress,
                )
                .await;
            }
            AgentEvent::ToolResult {
                id, name, is_error, ..
            } => {
                let status = if is_error {
                    TaskStatus::Error
                } else {
                    TaskStatus::Complete
                };
                let title = titles.remove(&id).unwrap_or(name);
                task(slack, channel, &stream_ts, &id, &title, status).await;
            }
            AgentEvent::ToolDenied { name, reason } => {
                flush(slack, channel, &stream_ts, &mut buffer, &mut last_flush).await;
                notice += 1;
                task(
                    slack,
                    channel,
                    &stream_ts,
                    &format!("notice-{notice}"),
                    &format!("{name} refused — {reason}"),
                    TaskStatus::Error,
                )
                .await;
            }
            AgentEvent::Compacted { .. } => {
                notice += 1;
                task(
                    slack,
                    channel,
                    &stream_ts,
                    &format!("notice-{notice}"),
                    "the transcript was summarised to fit the context window",
                    TaskStatus::Complete,
                )
                .await;
            }
            AgentEvent::QueuedInput(text) => {
                notice += 1;
                task(
                    slack,
                    channel,
                    &stream_ts,
                    &format!("notice-{notice}"),
                    &format!("steering: {text}"),
                    TaskStatus::Complete,
                )
                .await;
            }
            AgentEvent::Done(done) => outcome = Some(done),
            _ => {}
        }
    }

    flush(slack, channel, &stream_ts, &mut buffer, &mut last_flush).await;

    let footer = outcome.as_deref().map(footer_blocks);
    if let Err(e) = chat::stop_stream(slack, channel, &stream_ts, footer).await {
        tracing::warn!("could not close the Slack stream: {e}");
    }
    Some(stream_ts)
}

async fn flush(
    slack: &Slack,
    channel: &str,
    ts: &str,
    buffer: &mut String,
    last_flush: &mut Instant,
) {
    if buffer.is_empty() {
        return;
    }
    let chunk = Chunk::Markdown(std::mem::take(buffer));
    if let Err(e) = chat::append_stream(slack, channel, ts, &chunk).await {
        tracing::warn!("could not append to the Slack stream: {e}");
    }
    *last_flush = Instant::now();
}

async fn task(slack: &Slack, channel: &str, ts: &str, id: &str, title: &str, status: TaskStatus) {
    let chunk = Chunk::Task {
        id: id.to_string(),
        title: title.to_string(),
        status,
    };
    if let Err(e) = chat::append_stream(slack, channel, ts, &chunk).await {
        tracing::warn!("could not append a task update: {e}");
    }
}

/// What a call will actually do, in one line. The same shape the approval card
/// uses, so a person sees the same words in both places.
fn describe(name: &str, input: &serde_json::Value) -> String {
    let detail = input
        .get("command")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(120).collect::<String>());
    match detail {
        Some(d) => format!("{name}: {d}"),
        None => name.to_string(),
    }
}

/// What a run cost and how it ended — the part that makes a thread auditable
/// rather than just readable.
fn footer_blocks(outcome: &RunOutcome) -> Vec<serde_json::Value> {
    vec![blocks::context(&footer_parts(outcome).join(" · "))]
}

/// Pure, so the parts that must never be silent are testable without a run.
fn footer_parts(outcome: &RunOutcome) -> Vec<String> {
    let mut parts = vec![format!("{} turns", outcome.turns)];
    parts.push(format!(
        "{} in / {} out{}",
        outcome.usage.input_tokens,
        outcome.usage.output_tokens,
        // A cancelled run keeps its input tokens and loses the output tokens of
        // the cut turn. Reporting the shortfall as a measurement would be a
        // quiet lie in the field a budget reads.
        if outcome.usage_complete {
            ""
        } else {
            " (partial)"
        }
    ));
    if let Some(cost) = outcome.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    parts.push(format!("{:?}", outcome.stop_cause).to_lowercase());
    if outcome.compactions > 0 {
        parts.push(format!("{} compaction(s)", outcome.compactions));
    }
    if outcome.blocked_sends > 0 {
        parts.push(format!(
            "{} send(s) refused by the interlock",
            outcome.blocked_sends
        ));
    }
    if outcome.exhausted {
        parts.push("*hit the turn limit — the answer is probably incomplete*".into());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_call_is_described_by_what_it_will_do() {
        assert_eq!(
            describe("shell", &json!({"command": "cargo test"})),
            "shell: cargo test"
        );
        assert_eq!(describe("todo", &json!({})), "todo");
    }

    #[test]
    fn a_long_command_is_cut_before_it_reaches_slacks_task_limit() {
        let d = describe("shell", &json!({"command": "x".repeat(1000)}));
        assert!(d.chars().count() <= 120 + "shell: ".len());
    }

    fn outcome() -> RunOutcome {
        RunOutcome {
            duration_secs: None,
            context_overflows: 0,
            boredom_notices: 0,
            step_escalations_attempted: 0,
            step_escalations_revised: 0,
            step_nulls: 0,
            step_reopens: 0,
            step_completions: 0,
            homeostat: None,
            text: String::new(),
            stop_reason: mecha_core::message::StopReason::EndTurn,
            usage: mecha_core::Usage::default(),
            turns: 3,
            refusal: None,
            exhausted: false,
            ended_on_failed_call: false,
            tool_calls: Vec::new(),
            malformed_tool_args: 0,
            blocked_sends: 0,
            taint: Default::default(),
            stop_cause: mecha_core::agent::StopCause::Completed,
            cost_usd: None,
            compactions: 0,
            usage_complete: true,
        }
    }

    #[test]
    fn the_footer_says_how_it_ended_and_what_it_cost() {
        let mut o = outcome();
        o.usage.input_tokens = 100;
        o.usage.output_tokens = 20;
        o.blocked_sends = 2;
        o.exhausted = true;
        o.compactions = 1;

        let parts = footer_parts(&o).join(" · ");
        assert!(parts.contains("3 turns"), "{parts}");
        assert!(parts.contains("100 in / 20 out"), "{parts}");
        assert!(parts.contains("1 compaction"), "{parts}");
        // The two that matter for trust: a refused send and a truncated answer
        // must never be silent.
        assert!(parts.contains("refused by the interlock"), "{parts}");
        assert!(parts.contains("turn limit"), "{parts}");
    }

    #[test]
    fn partial_usage_says_so_rather_than_reading_as_a_measurement() {
        // A cancelled run loses the cut turn's output tokens. Presenting the
        // shortfall as a number is a quiet lie in the field a budget reads.
        let mut o = outcome();
        o.usage_complete = false;
        assert!(footer_parts(&o).join(" ").contains("(partial)"));
        assert!(!footer_parts(&outcome()).join(" ").contains("(partial)"));
    }
}
