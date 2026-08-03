//! Run the same agent over many inputs.
//!
//! This is the eval/sweep shape: N independent prompts, bounded concurrency,
//! failures recorded rather than fatal, results keyed so they can be joined
//! back to their inputs in any order.

use crate::agent::{Agent, Conversation, RunContext, StopCause, Taint, ToolCallTrace};
use crate::message::{Message, StopReason, Usage};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What to say to the agent: one turn, or several in one conversation.
///
/// Untagged so `"prompt": "..."` and `"prompt": ["...", "..."]` both parse, and
/// no existing case or batch file has to change. Several turns share one
/// `Conversation`, which is the whole point — a turn boundary is not a security
/// boundary, and anything that only goes wrong across turns (taint
/// accumulating, a transcript growing past the compaction threshold) cannot be
/// expressed by a single prompt at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prompt {
    One(String),
    Many(Vec<String>),
}

impl Prompt {
    pub fn turns(&self) -> &[String] {
        match self {
            Prompt::One(s) => std::slice::from_ref(s),
            Prompt::Many(v) => v,
        }
    }

    /// The opening turn, for logs and titles.
    pub fn first(&self) -> &str {
        self.turns().first().map(String::as_str).unwrap_or_default()
    }

    /// Every turn as one readable block, for a judge or a log.
    ///
    /// A judge shown only the last turn of a multi-turn case would grade the
    /// answer against half the question.
    pub fn render(&self) -> String {
        match self {
            Prompt::One(s) => s.clone(),
            Prompt::Many(v) => v
                .iter()
                .enumerate()
                .map(|(i, t)| format!("[turn {}] {t}", i + 1))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

impl From<String> for Prompt {
    fn from(s: String) -> Self {
        Prompt::One(s)
    }
}

impl From<&str> for Prompt {
    fn from(s: &str) -> Self {
        Prompt::One(s.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItem {
    /// Caller-supplied key. Results are matched on this, never on position.
    pub id: String,
    pub prompt: Prompt,
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
    /// Why the loop stopped, as distinct from why the model stopped talking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_cause: Option<StopCause>,
    /// What had entered the conversation by the end.
    #[serde(default)]
    pub taint: Taint,
    /// Outbound calls the interlock refused.
    #[serde(default)]
    pub blocked_sends: u32,
    /// How many times the transcript was summarised.
    #[serde(default)]
    pub compactions: u32,
    /// False when `usage` is a lower bound — see [`crate::agent::RunOutcome`].
    #[serde(default)]
    pub usage_complete: bool,
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
        let mut convo = Conversation::new();
        let cx = context_for(&item).unwrap_or_else(|| Arc::clone(agent.context()));

        // Every turn runs on the *same* conversation, so taint accumulates and
        // the transcript grows exactly as it would in a real session. Totals
        // are summed across turns; the last turn's answer is the answer.
        let mut totals = Totals::default();
        let mut failure = None;
        let mut last: Option<crate::agent::RunOutcome> = None;

        for turn in item.prompt.turns() {
            convo.push(Message::user(turn.clone()));
            match agent.run_in(&cx, &mut convo, None).await {
                Ok(outcome) => {
                    totals.absorb(&outcome);
                    last = Some(outcome);
                }
                Err(e) => {
                    // Stop here: later turns were written to follow this one,
                    // and running them against a conversation missing a reply
                    // would measure something nobody asked for.
                    failure = Some(format!("{e:#}"));
                    break;
                }
            }
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;

        match (last, failure) {
            (Some(outcome), None) => BatchResult {
                id: item.id,
                // An exhausted run technically returned, but the answer is
                // truncated; callers shouldn't count it as a success.
                ok: !outcome.exhausted
                    && outcome.stop_reason != StopReason::Refusal
                    && totals.malformed_tool_args == 0,
                text: outcome.text,
                error: outcome.refusal.map(|r| {
                    format!(
                        "refused ({}): {}",
                        r.category.unwrap_or_else(|| "unspecified".into()),
                        r.explanation.unwrap_or_default()
                    )
                }),
                turns: totals.turns,
                usage: totals.usage,
                stop_reason: Some(outcome.stop_reason),
                meta: item.meta,
                elapsed_ms,
                tool_calls: totals.tool_calls,
                malformed_tool_args: totals.malformed_tool_args,
                stop_cause: Some(outcome.stop_cause),
                // Taint lives on the conversation, so it is already cumulative.
                taint: convo.taint,
                blocked_sends: totals.blocked_sends,
                compactions: totals.compactions,
                usage_complete: totals.usage_complete,
            },
            // A failure keeps whatever the earlier turns cost: they ran, and a
            // sweep that under-reports its own spend is worse than one that
            // reports a failure.
            (_, error) => BatchResult {
                id: item.id,
                ok: false,
                text: String::new(),
                error: error.or_else(|| Some("the item had no prompts".into())),
                turns: totals.turns,
                usage: totals.usage,
                stop_reason: None,
                meta: item.meta,
                elapsed_ms,
                tool_calls: totals.tool_calls,
                malformed_tool_args: totals.malformed_tool_args,
                stop_cause: None,
                taint: convo.taint,
                blocked_sends: totals.blocked_sends,
                compactions: totals.compactions,
                usage_complete: totals.usage_complete,
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

/// Running totals across the turns of one item.
///
/// A multi-turn item is still *one* result, so everything countable is summed
/// rather than overwritten. Reporting only the last turn would make a two-turn
/// case look cheaper than a one-turn case that did the same work.
struct Totals {
    usage: Usage,
    turns: u32,
    tool_calls: Vec<ToolCallTrace>,
    malformed_tool_args: u32,
    blocked_sends: u32,
    compactions: u32,
    usage_complete: bool,
}

impl Default for Totals {
    fn default() -> Self {
        Totals {
            usage: Usage::default(),
            turns: 0,
            tool_calls: Vec::new(),
            malformed_tool_args: 0,
            blocked_sends: 0,
            compactions: 0,
            // True until a turn says otherwise: one incomplete count makes the
            // whole item's total a lower bound.
            usage_complete: true,
        }
    }
}

impl Totals {
    fn absorb(&mut self, outcome: &crate::agent::RunOutcome) {
        self.usage.add(&outcome.usage);
        self.turns += outcome.turns;
        self.tool_calls.extend(outcome.tool_calls.iter().cloned());
        self.malformed_tool_args += outcome.malformed_tool_args;
        self.blocked_sends += outcome.blocked_sends;
        self.compactions += outcome.compactions;
        self.usage_complete &= outcome.usage_complete;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, PermissionMode};
    use crate::message::{Block, CompletionRequest, CompletionResponse, Message};
    use crate::provider::{Provider, StreamSink};
    use crate::tool::{ModeApprover, Registry, ToolCtx};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Answers from the *request* rather than from a queue.
    ///
    /// A scripted list of turns is useless here: `buffer_unordered` means the
    /// order items reach the provider is not the order they were submitted, so
    /// a shared queue would hand item B the answer meant for item A and the
    /// test would be measuring its own fixture.
    #[derive(Default)]
    struct EchoProvider {
        /// How many messages each request carried. One per item, if each item
        /// really does get its own conversation.
        history_lengths: Mutex<Vec<usize>>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
        delay: std::time::Duration,
    }

    #[async_trait]
    impl Provider for EchoProvider {
        fn id(&self) -> &str {
            "echo"
        }
        fn default_model(&self) -> &str {
            "echo-1"
        }

        async fn complete(
            &self,
            req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);

            self.history_lengths.lock().unwrap().push(req.messages.len());
            let prompt = req.messages.last().map(|m| m.text()).unwrap_or_default();

            // One item is allowed to blow up, so the "recorded, not fatal"
            // behaviour has something to record.
            anyhow::ensure!(!prompt.contains("boom"), "the provider exploded");

            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::text(format!("answered: {prompt}"))]),
                stop_reason: StopReason::EndTurn,
                usage: Usage { input_tokens: 10, output_tokens: 5, ..Usage::default() },
                refusal: None,
                model: "echo-1".into(),
                malformed_tool_args: 0,
            })
        }
    }


    fn agent_with(provider: Arc<EchoProvider>) -> Agent {
        struct Shared(Arc<EchoProvider>);
        #[async_trait]
        impl Provider for Shared {
            fn id(&self) -> &str {
                self.0.id()
            }
            fn default_model(&self) -> &str {
                self.0.default_model()
            }
            async fn complete(
                &self,
                req: &CompletionRequest,
                sink: Option<&StreamSink>,
            ) -> Result<CompletionResponse> {
                self.0.complete(req, sink).await
            }
        }

        Agent::new(
            Box::new(Shared(provider)),
            Registry::new(),
            Arc::new(ModeApprover { mode: PermissionMode::Allow }),
            ToolCtx { workspace: std::env::temp_dir(), ..Default::default() },
            AgentConfig::default(),
            None,
        )
        .unwrap()
    }

    fn items(prompts: &[&str]) -> Vec<BatchItem> {
        prompts
            .iter()
            .enumerate()
            .map(|(i, p)| BatchItem {
                id: format!("item-{i}"),
                prompt: (*p).to_string().into(),
                meta: Some(serde_json::json!({"index": i})),
            })
            .collect()
    }

    #[tokio::test]
    async fn results_are_matched_by_id_not_by_position() {
        let provider = Arc::new(EchoProvider {
            delay: std::time::Duration::from_millis(20),
            ..Default::default()
        });
        let agent = agent_with(Arc::clone(&provider));

        let results = run(&agent, items(&["alpha", "beta", "gamma", "delta"]), 4, |_| {}).await;

        // Completion order under concurrency is not submission order, so every
        // result has to carry its own key and metadata home with it.
        assert_eq!(results.len(), 4);
        for r in &results {
            let index = r.meta.as_ref().unwrap()["index"].as_u64().unwrap();
            assert_eq!(r.id, format!("item-{index}"));
            let expected = ["alpha", "beta", "gamma", "delta"][index as usize];
            assert_eq!(r.text, format!("answered: {expected}"), "{} got another item's answer", r.id);
        }
    }

    #[tokio::test]
    async fn every_item_gets_its_own_conversation() {
        let provider = Arc::new(EchoProvider::default());
        let agent = agent_with(Arc::clone(&provider));

        run(&agent, items(&["one", "two", "three"]), 1, |_| {}).await;

        // Batch items are independent by definition. A shared conversation
        // would grow, and — since taint travels with the messages — one item
        // reading a hostile page would arm the interlock for every item after
        // it.
        let lengths = provider.history_lengths.lock().unwrap();
        assert_eq!(*lengths, vec![1, 1, 1], "history leaked between items");
    }

    #[tokio::test]
    async fn a_failing_item_is_recorded_rather_than_sinking_the_batch() {
        let provider = Arc::new(EchoProvider::default());
        let agent = agent_with(Arc::clone(&provider));

        let results = run(&agent, items(&["fine", "boom", "also fine"]), 1, |_| {}).await;

        assert_eq!(results.len(), 3, "a failure took other items down with it");
        let failed: Vec<_> = results.iter().filter(|r| !r.ok).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].id, "item-1");
        assert!(failed[0].error.as_ref().unwrap().contains("exploded"));
        // A failure still reports its key and metadata, or it cannot be joined
        // back to the input that caused it.
        assert!(failed[0].meta.is_some());

        assert!(results.iter().filter(|r| r.ok).count() == 2);
    }

    #[tokio::test]
    async fn concurrency_is_bounded_by_what_was_asked_for() {
        let provider = Arc::new(EchoProvider {
            delay: std::time::Duration::from_millis(50),
            ..Default::default()
        });
        let agent = agent_with(Arc::clone(&provider));

        run(&agent, items(&["a", "b", "c", "d", "e", "f"]), 2, |_| {}).await;

        let peak = provider.max_in_flight.load(Ordering::SeqCst);
        assert!(peak <= 2, "{peak} items ran at once against a limit of 2");
        assert_eq!(peak, 2, "the limit was never actually reached; the test proves nothing");
    }

    #[tokio::test]
    async fn a_concurrency_of_zero_still_makes_progress() {
        let provider = Arc::new(EchoProvider::default());
        let agent = agent_with(Arc::clone(&provider));

        // `concurrency.max(1)`: a zero would otherwise mean a stream that never
        // polls anything and a batch that hangs forever.
        let results = run(&agent, items(&["only"]), 0, |_| {}).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].ok);
    }

    #[tokio::test]
    async fn each_result_is_announced_as_it_lands() {
        let provider = Arc::new(EchoProvider::default());
        let agent = agent_with(Arc::clone(&provider));

        // The callback is what lets a long eval print progress instead of
        // going quiet for ten minutes.
        let mut announced = Vec::new();
        let results = run(&agent, items(&["a", "b", "c"]), 1, |r| announced.push(r.id.clone())).await;

        assert_eq!(announced.len(), 3);
        assert_eq!(announced, results.iter().map(|r| r.id.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn a_summary_totals_usage_and_counts_both_outcomes() {
        let result = |id: &str, ok: bool| BatchResult {
            id: id.into(),
            ok,
            text: String::new(),
            error: None,
            turns: 1,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Usage::default() },
            stop_reason: Some(StopReason::EndTurn),
            meta: None,
            elapsed_ms: 1,
            tool_calls: Vec::new(),
            malformed_tool_args: 0,
            stop_cause: None,
            taint: Taint::default(),
            blocked_sends: 0,
            compactions: 0,
            usage_complete: true,
        };

        let summary = BatchSummary::of(&[result("a", true), result("b", false), result("c", true)], 99);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 1);
        // Failed items still burned tokens, and a summary that hid them would
        // under-report what a sweep cost.
        assert_eq!(summary.usage.input_tokens, 30);
        assert_eq!(summary.usage.output_tokens, 15);
        assert_eq!(summary.elapsed_ms, 99);
    }
}
