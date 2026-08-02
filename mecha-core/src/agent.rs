//! The agent loop.
//!
//! Ask the model, run whatever tools it asks for, feed the results back, repeat
//! until it stops asking. Everything interesting — which provider, which tools,
//! who approves side effects — is injected, so the same loop drives the REPL,
//! a one-shot run, and a batch worker.

use crate::config::AgentConfig;
use crate::message::*;
use crate::provider::{Provider, StreamEvent};
use crate::tool::{Approver, Decision, Registry, ToolCtx, ToolOutput};
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Everything the loop wants to tell an observer. The CLI renders these; a
/// batch runner ignores all but the last.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStart { turn: u32 },
    ThinkingDelta(String),
    TextDelta(String),
    /// The complete assistant text for this turn, after streaming finishes.
    AssistantText(String),
    ToolCall { id: String, name: String, input: Value },
    ToolDenied { name: String, reason: String },
    ToolResult { id: String, name: String, is_error: bool, content: String },
    TurnUsage(Usage),
    Done(Box<RunOutcome>),
}

/// One tool call as it actually happened. The trace is what you grade a model
/// on — final text alone can't tell a lucky guess from correct tool use.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallTrace {
    pub name: String,
    pub input: Value,
    /// The tool ran and reported failure.
    pub is_error: bool,
    /// Refused by the approver before it ran.
    pub denied: bool,
    /// The model named a tool that does not exist.
    pub unknown: bool,
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Text of the final assistant turn.
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub turns: u32,
    pub refusal: Option<Refusal>,
    /// True when the loop stopped because it hit `max_turns`, not because the
    /// model was finished. The answer is probably incomplete.
    pub exhausted: bool,
    /// Every tool call attempted, in order.
    pub tool_calls: Vec<ToolCallTrace>,
    /// Calls whose arguments did not parse as JSON.
    pub malformed_tool_args: u32,
}

pub struct Agent {
    provider: Box<dyn Provider>,
    registry: Registry,
    approver: Arc<dyn Approver>,
    ctx: ToolCtx,
    cfg: AgentConfig,
    model: String,
    system: Option<String>,
}

impl Agent {
    pub fn new(
        provider: Box<dyn Provider>,
        registry: Registry,
        approver: Arc<dyn Approver>,
        ctx: ToolCtx,
        cfg: AgentConfig,
        model: Option<String>,
    ) -> Result<Self> {
        let model = model.unwrap_or_else(|| provider.default_model().to_string());
        let system = cfg.resolve_system_prompt()?;
        Ok(Agent { provider, registry, approver, ctx, cfg, model, system })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Run until the model stops calling tools.
    ///
    /// `messages` is the live conversation: it is appended to in place, so a
    /// REPL can call this repeatedly and keep the history.
    pub async fn run(
        &self,
        messages: &mut Vec<Message>,
        events: Option<UnboundedSender<AgentEvent>>,
    ) -> Result<RunOutcome> {
        let mut usage = Usage::default();
        let mut turns = 0;
        let mut trace: Vec<ToolCallTrace> = Vec::new();
        let mut malformed = 0u32;

        loop {
            if turns >= self.cfg.max_turns {
                let outcome = RunOutcome {
                    text: messages.last().map(Message::text).unwrap_or_default(),
                    stop_reason: StopReason::Other,
                    usage,
                    turns,
                    refusal: None,
                    exhausted: true,
                    tool_calls: trace,
                    malformed_tool_args: malformed,
                };
                emit(&events, AgentEvent::Done(Box::new(outcome.clone())));
                return Ok(outcome);
            }
            turns += 1;
            emit(&events, AgentEvent::TurnStart { turn: turns });

            let request = CompletionRequest {
                model: self.model.clone(),
                system: self.system.clone(),
                messages: messages.clone(),
                tools: self.registry.specs(),
                max_tokens: self.cfg.max_tokens,
                effort: self.cfg.effort,
                thinking: self.cfg.thinking,
                cache_prompt: self.cfg.cache_prompt,
            };

            let response = self.complete(&request, &events).await?;
            usage.add(&response.usage);
            malformed += response.malformed_tool_args;
            emit(&events, AgentEvent::TurnUsage(response.usage.clone()));

            let text = response.message.text();
            if !text.is_empty() {
                emit(&events, AgentEvent::AssistantText(text.clone()));
            }
            messages.push(response.message.clone());

            match response.stop_reason {
                StopReason::ToolUse => {
                    let results = self.run_tools(&response.message, &events, &mut trace).await;
                    // The API rejects the next request unless every tool_use id
                    // has a matching tool_result, so this must never be empty
                    // when the model asked for tools.
                    if results.is_empty() {
                        let outcome = self.finish(text, &response, usage, turns, trace, malformed);
                        emit(&events, AgentEvent::Done(Box::new(outcome.clone())));
                        return Ok(outcome);
                    }
                    messages.push(Message::tool_results(results));
                }
                // A server-side tool loop paused mid-turn. Resending the
                // conversation as-is resumes it; no extra user message.
                StopReason::PauseTurn => continue,
                _ => {
                    let outcome = self.finish(text, &response, usage, turns, trace, malformed);
                    emit(&events, AgentEvent::Done(Box::new(outcome.clone())));
                    return Ok(outcome);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        text: String,
        response: &CompletionResponse,
        usage: Usage,
        turns: u32,
        tool_calls: Vec<ToolCallTrace>,
        malformed_tool_args: u32,
    ) -> RunOutcome {
        RunOutcome {
            text,
            stop_reason: response.stop_reason,
            usage,
            turns,
            refusal: response.refusal.clone(),
            exhausted: false,
            tool_calls,
            malformed_tool_args,
        }
    }

    /// Call the provider, bridging its stream events onto ours when someone is
    /// listening.
    async fn complete(
        &self,
        request: &CompletionRequest,
        events: &Option<UnboundedSender<AgentEvent>>,
    ) -> Result<CompletionResponse> {
        let Some(events) = events.clone() else {
            return self.provider.complete(request, None).await;
        };

        let (tx, mut rx) = unbounded_channel::<StreamEvent>();
        let forwarder = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let mapped = match ev {
                    StreamEvent::TextDelta(t) => AgentEvent::TextDelta(t),
                    StreamEvent::ThinkingDelta(t) => AgentEvent::ThinkingDelta(t),
                    // Surfaced through ToolCall once arguments are complete.
                    StreamEvent::ToolUseStart { .. } => continue,
                };
                let _ = events.send(mapped);
            }
        });

        let result = self.provider.complete(request, Some(&tx)).await;
        drop(tx);
        let _ = forwarder.await;
        result
    }

    /// Approve, then execute, every tool call in the assistant turn.
    ///
    /// Approval is sequential because it may block on a human. Execution is
    /// concurrent, because by then all the decisions are made.
    async fn run_tools(
        &self,
        assistant: &Message,
        events: &Option<UnboundedSender<AgentEvent>>,
        trace: &mut Vec<ToolCallTrace>,
    ) -> Vec<Block> {
        let calls: Vec<(String, String, Value)> = assistant
            .tool_uses()
            .into_iter()
            .map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone()))
            .collect();

        let mut approved = Vec::new();
        let mut results: Vec<Option<Block>> = vec![None; calls.len()];

        for (i, (id, name, input)) in calls.iter().enumerate() {
            emit(
                events,
                AgentEvent::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() },
            );

            let Some(tool) = self.registry.get(name) else {
                let content = format!(
                    "no tool named `{name}`. Available: {}",
                    self.registry
                        .iter()
                        .map(|t| t.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                emit(
                    events,
                    AgentEvent::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        is_error: true,
                        content: content.clone(),
                    },
                );
                results[i] = Some(Block::ToolResult {
                    tool_use_id: id.clone(),
                    content,
                    is_error: true,
                });
                trace.push(ToolCallTrace {
                    name: name.clone(),
                    input: input.clone(),
                    is_error: true,
                    denied: false,
                    unknown: true,
                });
                continue;
            };

            if !tool.read_only() {
                if let Decision::Deny(reason) = self.approver.approve(tool.as_ref(), input).await {
                    emit(
                        events,
                        AgentEvent::ToolDenied { name: name.clone(), reason: reason.clone() },
                    );
                    results[i] = Some(Block::ToolResult {
                        tool_use_id: id.clone(),
                        content: format!("Denied by the user: {reason}"),
                        is_error: true,
                    });
                    trace.push(ToolCallTrace {
                        name: name.clone(),
                        input: input.clone(),
                        is_error: true,
                        denied: true,
                        unknown: false,
                    });
                    continue;
                }
            }

            approved.push((i, Arc::clone(tool), id.clone(), name.clone(), input.clone()));
        }

        let executed = futures::future::join_all(approved.into_iter().map(
            |(i, tool, id, name, input)| async move {
                let out = match tool.call(input, &self.ctx).await {
                    Ok(out) => out,
                    // A tool that returns Err failed in a way it didn't
                    // anticipate; tell the model so it can try something else.
                    Err(e) => ToolOutput::err(format!("tool `{name}` failed: {e:#}")),
                };
                (i, id, name, out)
            },
        ))
        .await;

        for (i, id, name, out) in executed {
            trace.push(ToolCallTrace {
                name: name.clone(),
                input: calls[i].2.clone(),
                is_error: out.is_error,
                denied: false,
                unknown: false,
            });
            emit(
                events,
                AgentEvent::ToolResult {
                    id: id.clone(),
                    name,
                    is_error: out.is_error,
                    content: out.content.clone(),
                },
            );
            results[i] = Some(Block::ToolResult {
                tool_use_id: id,
                content: out.content,
                is_error: out.is_error,
            });
        }

        results.into_iter().flatten().collect()
    }
}

fn emit(events: &Option<UnboundedSender<AgentEvent>>, event: AgentEvent) {
    if let Some(tx) = events {
        let _ = tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::StreamSink;
    use crate::tool::{ModeApprover, Tool, ToolOutput};
    use crate::config::PermissionMode;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    /// Replays a fixed script of turns and records what it was asked.
    struct ScriptedProvider {
        turns: Mutex<Vec<CompletionResponse>>,
        seen: Mutex<Vec<CompletionRequest>>,
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn id(&self) -> &str { "scripted" }
        fn default_model(&self) -> &str { "scripted-1" }

        async fn complete(
            &self,
            req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            self.seen.lock().unwrap().push(req.clone());
            let mut turns = self.turns.lock().unwrap();
            anyhow::ensure!(!turns.is_empty(), "provider ran out of scripted turns");
            Ok(turns.remove(0))
        }
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "Echo the `value` argument back." }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {"value": {"type": "string"}}})
        }
        fn read_only(&self) -> bool { true }
        async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok(input.get("value").and_then(Value::as_str).unwrap_or("")))
        }
    }

    fn assistant(blocks: Vec<Block>, stop: StopReason) -> CompletionResponse {
        CompletionResponse {
            message: Message::assistant(blocks),
            stop_reason: stop,
            usage: Usage { input_tokens: 10, output_tokens: 5, ..Usage::default() },
            refusal: None,
            model: "scripted-1".into(),
            malformed_tool_args: 0,
        }
    }

    fn agent_with(turns: Vec<CompletionResponse>, mode: PermissionMode) -> (Agent, Arc<ScriptedProvider>) {
        let provider = Arc::new(ScriptedProvider {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
        });
        let mut registry = Registry::new();
        registry.insert(Arc::new(EchoTool));

        struct Shared(Arc<ScriptedProvider>);
        #[async_trait]
        impl Provider for Shared {
            fn id(&self) -> &str { self.0.id() }
            fn default_model(&self) -> &str { self.0.default_model() }
            async fn complete(
                &self,
                req: &CompletionRequest,
                sink: Option<&StreamSink>,
            ) -> Result<CompletionResponse> {
                self.0.complete(req, sink).await
            }
        }

        let agent = Agent::new(
            Box::new(Shared(Arc::clone(&provider))),
            registry,
            Arc::new(ModeApprover { mode }),
            ToolCtx {
                workspace: std::env::temp_dir(),
                shell_timeout: std::time::Duration::from_secs(1),
            },
            AgentConfig::default(),
            None,
        )
        .unwrap();
        (agent, provider)
    }

    #[tokio::test]
    async fn tool_call_result_is_fed_back_and_loop_terminates() {
        let (agent, provider) = agent_with(
            vec![
                assistant(
                    vec![Block::ToolUse {
                        id: "t1".into(),
                        name: "echo".into(),
                        input: json!({"value": "pong"}),
                    }],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("done")], StopReason::EndTurn),
            ],
            PermissionMode::Allow,
        );

        let mut messages = vec![Message::user("ping")];
        let outcome = agent.run(&mut messages, None).await.unwrap();

        assert_eq!(outcome.text, "done");
        assert_eq!(outcome.turns, 2);
        assert!(!outcome.exhausted);
        // Usage accumulates across turns rather than reporting only the last.
        assert_eq!(outcome.usage.output_tokens, 10);

        // user, assistant(tool_use), user(tool_result), assistant(text)
        assert_eq!(messages.len(), 4);
        match &messages[2].content[0] {
            Block::ToolResult { tool_use_id, content, is_error } => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(content, "pong");
                assert!(!is_error);
            }
            other => panic!("expected a tool result, got {other:?}"),
        }

        // The second request carried the whole history, including the result.
        let seen = provider.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].messages.len(), 3);
    }

    #[tokio::test]
    async fn unknown_tool_returns_an_error_result_rather_than_aborting() {
        let (agent, _) = agent_with(
            vec![
                assistant(
                    vec![Block::ToolUse {
                        id: "t1".into(),
                        name: "nonexistent".into(),
                        input: json!({}),
                    }],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("recovered")], StopReason::EndTurn),
            ],
            PermissionMode::Allow,
        );

        let mut messages = vec![Message::user("go")];
        let outcome = agent.run(&mut messages, None).await.unwrap();

        assert_eq!(outcome.text, "recovered");
        match &messages[2].content[0] {
            Block::ToolResult { is_error, content, .. } => {
                assert!(is_error);
                assert!(content.contains("no tool named"));
            }
            other => panic!("expected an error tool result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn max_turns_stops_a_model_that_never_finishes() {
        let looping = || {
            assistant(
                vec![Block::ToolUse {
                    id: "t".into(),
                    name: "echo".into(),
                    input: json!({"value": "again"}),
                }],
                StopReason::ToolUse,
            )
        };
        let (agent, _) = agent_with(
            (0..10).map(|_| looping()).collect(),
            PermissionMode::Allow,
        );

        let mut messages = vec![Message::user("loop forever")];
        // Shrink the budget rather than waiting for the default.
        let outcome = {
            let mut agent = agent;
            agent.cfg.max_turns = 3;
            agent.run(&mut messages, None).await.unwrap()
        };

        assert!(outcome.exhausted);
        assert_eq!(outcome.turns, 3);
    }

    #[tokio::test]
    async fn read_only_mode_denies_writing_tools_but_still_answers() {
        struct WriteTool;
        #[async_trait]
        impl Tool for WriteTool {
            fn name(&self) -> &str { "mutate" }
            fn description(&self) -> &str { "Changes something." }
            fn input_schema(&self) -> Value { json!({"type": "object"}) }
            async fn call(&self, _input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                panic!("a denied tool must never execute");
            }
        }

        let (mut agent, _) = agent_with(
            vec![
                assistant(
                    vec![Block::ToolUse {
                        id: "t1".into(),
                        name: "mutate".into(),
                        input: json!({}),
                    }],
                    StopReason::ToolUse,
                ),
                assistant(vec![Block::text("understood")], StopReason::EndTurn),
            ],
            PermissionMode::ReadOnly,
        );
        agent.registry.insert(Arc::new(WriteTool));

        let mut messages = vec![Message::user("change it")];
        let outcome = agent.run(&mut messages, None).await.unwrap();

        assert_eq!(outcome.text, "understood");
        match &messages[2].content[0] {
            Block::ToolResult { is_error, content, .. } => {
                assert!(is_error);
                assert!(content.contains("Denied"));
            }
            other => panic!("expected a denial, got {other:?}"),
        }
    }
}
