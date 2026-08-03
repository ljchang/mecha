//! Run the same agent over many inputs.
//!
//! This is the eval/sweep shape: N independent prompts, bounded concurrency,
//! failures recorded rather than fatal, results keyed so they can be joined
//! back to their inputs in any order.

use crate::agent::{Agent, Conversation, RunContext, ToolCallTrace};
use crate::message::{StopReason, Usage};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItem {
    /// Caller-supplied key. Results are matched on this, never on position.
    pub id: String,
    pub prompt: String,
    /// Carried through to the result untouched — useful for gold answers,
    /// subject ids, or whatever the caller is joining against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub id: String,
    pub ok: bool,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub turns: u32,
    pub usage: Usage,
    pub stop_reason: Option<StopReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub elapsed_ms: u64,
    /// What the model actually did. Grading tool use needs this, not `text`.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallTrace>,
    #[serde(default)]
    pub malformed_tool_args: u32,
}

/// Run every item, at most `concurrency` at a time.
///
/// `on_result` fires as each item finishes, so a caller can stream progress
/// instead of waiting for the whole batch.
pub async fn run<F>(
    agent: &Agent,
    items: Vec<BatchItem>,
    concurrency: usize,
    on_result: F,
) -> Vec<BatchResult>
where
    F: FnMut(&BatchResult),
{
    run_with(agent, items, concurrency, |_| None, on_result).await
}

/// As [`run`], but each item may be given its own [`RunContext`].
///
/// Returning `None` from `context_for` uses the agent's own. This is what makes
/// a batch of *mutating* items possible: hand each one a private workspace and
/// the permission to write to it, and they stop being able to see each other's
/// side effects.
pub async fn run_with<C, F>(
    agent: &Agent,
    items: Vec<BatchItem>,
    concurrency: usize,
    context_for: C,
    mut on_result: F,
) -> Vec<BatchResult>
where
    C: Fn(&BatchItem) -> Option<Arc<RunContext>> + Sync,
    F: FnMut(&BatchResult),
{
    let concurrency = concurrency.max(1);
    let context_for = &context_for;

    let mut stream = futures::stream::iter(items.into_iter().map(|item| async move {
        let started = std::time::Instant::now();
        // Each item gets a fresh conversation — batch items are independent by
        // definition, and sharing history would leak one into the next. That
        // now covers the taint as well: one item reading a hostile page must
        // not arm the interlock for the next, which is a different
        // conversation that never saw it.
        let mut convo = Conversation::user(&item.prompt);

        let cx = context_for(&item).unwrap_or_else(|| Arc::clone(agent.context()));

        match agent.run_in(&cx, &mut convo, None).await {
            Ok(outcome) => BatchResult {
                id: item.id,
                // An exhausted run technically returned, but the answer is
                // truncated; callers shouldn't count it as a success.
                ok: !outcome.exhausted
                    && outcome.stop_reason != StopReason::Refusal
                    && outcome.malformed_tool_args == 0,
                text: outcome.text,
                error: outcome.refusal.map(|r| {
                    format!(
                        "refused ({}): {}",
                        r.category.unwrap_or_else(|| "unspecified".into()),
                        r.explanation.unwrap_or_default()
                    )
                }),
                turns: outcome.turns,
                usage: outcome.usage,
                stop_reason: Some(outcome.stop_reason),
                meta: item.meta,
                elapsed_ms: started.elapsed().as_millis() as u64,
                tool_calls: outcome.tool_calls,
                malformed_tool_args: outcome.malformed_tool_args,
            },
            Err(e) => BatchResult {
                id: item.id,
                ok: false,
                text: String::new(),
                error: Some(format!("{e:#}")),
                turns: 0,
                usage: Usage::default(),
                stop_reason: None,
                meta: item.meta,
                elapsed_ms: started.elapsed().as_millis() as u64,
                tool_calls: Vec::new(),
                malformed_tool_args: 0,
            },
        }
    }))
    .buffer_unordered(concurrency);

    let mut results = Vec::new();
    while let Some(result) = stream.next().await {
        on_result(&result);
        results.push(result);
    }
    results
}

/// Totals for a finished batch.
#[derive(Debug, Clone, Serialize)]
pub struct BatchSummary {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub usage: Usage,
    pub elapsed_ms: u64,
}

impl BatchSummary {
    pub fn of(results: &[BatchResult], elapsed_ms: u64) -> Self {
        let mut usage = Usage::default();
        for r in results {
            usage.add(&r.usage);
        }
        let succeeded = results.iter().filter(|r| r.ok).count();
        BatchSummary {
            total: results.len(),
            succeeded,
            failed: results.len() - succeeded,
            usage,
            elapsed_ms,
        }
    }
}
