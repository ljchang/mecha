//! Subagents.
//!
//! A subagent is an [`Agent`] wrapped in a [`Tool`]. That is the whole design:
//! the parent loop never learns that delegation exists, it just calls a tool
//! that happens to take a while and return prose.
//!
//! What makes them worth having is **capability restriction**. The child gets a
//! rebuilt tool registry — an allowlist, not an inheritance — so you can hand
//! it exactly one dangerous capability and nothing to pair it with. A child
//! that can fetch web pages but cannot send anything is unable to exfiltrate no
//! matter what the page tells it.
//!
//! ## What subagents do not do
//!
//! They do not launder untrusted content into trusted content. If a child reads
//! a web page and hands its parent a summary, that summary is still derived
//! from attacker-influenced text and can still carry instructions. So by default
//! a child whose tools can reach untrusted sources produces **untrusted
//! output**, and the parent's trifecta interlock still applies.
//!
//! What you actually gain is threefold: the raw content never enters the
//! parent's context, the child cannot send, and the two halves of the trifecta
//! can be kept in separate agents entirely.

use crate::agent::{Agent, Conversation, RunContext};
use crate::tool::{Capabilities, Tool, ToolCtx, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubagentProfile {
    /// Tool name the parent sees. Keep it a verb the model will reach for.
    pub name: String,
    /// Shown to the parent model. This is what decides whether delegation
    /// happens at all, so say when to use it, not just what it is.
    pub description: String,
    /// Allowlist of tools the child may use. Empty means no tools, which is
    /// occasionally what you want — a pure summarizer.
    pub tools: Vec<String>,
    pub system_prompt: Option<String>,
    pub max_turns: u32,
    /// Run this child on a different model. A narrow task with two tools does
    /// not need the model the parent is using, and a small fast one keeps
    /// delegation cheap enough to be worth doing.
    pub model: Option<String>,
    /// Run this child against a different provider entry — a second
    /// llama-server on another port, or a hosted model for one hard step.
    pub provider: Option<String>,
    /// Force the child's answer to be treated as trustworthy even though its
    /// tools can reach untrusted sources.
    ///
    /// Off by default, and turning it on is a real risk decision: it lets
    /// attacker-influenced text through to the parent with the trifecta
    /// interlock disarmed. Reasonable when the child returns something
    /// structurally harmless — a number, a yes/no — and not otherwise.
    pub trusted_output: bool,
}

impl Default for SubagentProfile {
    fn default() -> Self {
        SubagentProfile {
            name: "subagent".into(),
            description: "Delegate a self-contained task.".into(),
            tools: Vec::new(),
            system_prompt: None,
            max_turns: 12,
            model: None,
            provider: None,
            trusted_output: false,
        }
    }
}

/// A configured subagent, exposed to the parent as one tool.
pub struct Subagent {
    profile: SubagentProfile,
    agent: Arc<Agent>,
    /// Derived from the child's tools at construction, so the parent's taint
    /// tracking stays correct without anyone having to remember to declare it.
    capabilities: Capabilities,
}

impl Subagent {
    pub fn new(profile: SubagentProfile, agent: Arc<Agent>) -> Self {
        // A child's answer is only as trustworthy as the least trustworthy
        // thing it can read. Private data is *not* propagated: the point of a
        // subagent is that what it saw stays with it, and only its answer —
        // which the parent is about to read anyway — comes back.
        let child_reads_untrusted = agent
            .registry()
            .iter()
            .any(|t| t.capabilities().untrusted_input);

        let capabilities = Capabilities {
            untrusted_input: child_reads_untrusted && !profile.trusted_output,
            ..Capabilities::default()
        };

        Subagent {
            profile,
            agent,
            capabilities,
        }
    }

    /// The tools this child was actually given, for `mecha tools` and for
    /// checking that a profile's allowlist matched anything at all.
    pub fn tool_names(&self) -> Vec<&str> {
        self.agent.registry().iter().map(|t| t.name()).collect()
    }
}

#[async_trait]
impl Tool for Subagent {
    fn name(&self) -> &str {
        &self.profile.name
    }

    fn description(&self) -> &str {
        &self.profile.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The complete task, written for someone with no \
                                    memory of this conversation. State the goal, any \
                                    context they need, and what to return."
                }
            },
            "required": ["task"]
        })
    }

    fn read_only(&self) -> bool {
        // The child enforces its own permissions over its own tools; gating the
        // spawn itself would ask the user to approve twice.
        true
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let Some(task) = input.get("task").and_then(Value::as_str) else {
            return Ok(ToolOutput::err("missing required string argument `task`"));
        };

        // A fresh conversation every time. The child inherits no history, which
        // is the context-isolation half of why subagents are useful — and no
        // taint either, because it has not read any of what the parent read.
        // What comes back is marked untrusted on its own merits, below.
        let mut convo = Conversation::user(task);

        // The child works in the *caller's* workspace, not the one that existed
        // when it was built — otherwise a parent running against a per-run
        // sandbox delegates to a child still pointed at the original directory,
        // which is both wrong and a hole in the jail. Permissions stay the
        // child's own: the allowlist is the point of a subagent.
        let cx = RunContext {
            tools: Arc::new(ctx.clone()),
            approver: Arc::clone(&self.agent.context().approver),
            // The child's own `max_turns` comes from its profile, via its
            // config — a parent's remaining budget is not the child's business.
            budget: Default::default(),
            // Cancelling the parent cancels the child with it: the child is
            // one of the parent's tool calls, and a Ctrl-C that left a
            // subagent running would be a lie. From the *caller's* context —
            // the agent's own default has no token, which is exactly how this
            // used to wait out the whole child run.
            cancel: ctx.cancel.clone(),
            // The child has its own transcript, and its own config decides
            // when to summarise it.
            compact_at_tokens: None,
            // A subagent inherits the caller's phase: delegating from a
            // planning run must not be the way to get a write executed. Also
            // from the caller's context, for the same reason as `cancel` —
            // the agent's own default is always `Execute`.
            phase: ctx.phase,
            // The child agent's own hooks — the front-end that installs hooks
            // on the parent must install them on each child too (setup does),
            // or delegating becomes the way around a pre_tool policy.
            hooks: Arc::clone(&self.agent.context().hooks),
            // Steering is addressed to the parent. The child was given a
            // self-contained task and has no conversation to redirect.
            queued_input: None,
            // Same rule as hooks: setup installs the parent's outbox route on
            // each child, or delegating becomes the way to send unstaged.
            outbox: self.agent.context().outbox.clone(),
        };

        // If somebody is watching the parent run, forward the child's events
        // wrapped in `Nested`, so a delegation stops being a tool call that
        // goes dark for minutes. A grandchild's events arrive here already
        // wrapped once and get wrapped again — depth for free.
        let (child_events, forwarder) = match &ctx.events {
            Some(parent) => {
                let parent = parent.clone();
                let name = self.profile.name.clone();
                // The dispatch stamped the parent's tool_use id for this very
                // call; carrying it on every wrapped event is what lets a
                // renderer keep two parallel delegations apart.
                let call_id = ctx.call_id.clone();
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let task = tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        let _ = parent.send(crate::agent::AgentEvent::Nested {
                            tool: name.clone(),
                            id: call_id.clone(),
                            event: Box::new(event),
                        });
                    }
                });
                (Some(tx), Some(task))
            }
            None => (None, None),
        };

        let result = self.agent.run_in(&cx, &mut convo, child_events).await;

        // Drain the forwarder before building the result, on both paths.
        // `run_in` dropped its sender on return, so this terminates — and it
        // is what guarantees every `Nested` event lands *between* the parent's
        // `ToolCall` and `ToolResult` rather than racing past the latter.
        if let Some(task) = forwarder {
            let _ = task.await;
        }

        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                return Ok(ToolOutput::err(format!(
                    "subagent `{}` failed: {e:#}",
                    self.profile.name
                )))
            }
        };

        let mut content = outcome.text;
        if content.trim().is_empty() {
            content = format!(
                "The `{}` subagent finished without producing an answer after {} turns.",
                self.profile.name, outcome.turns
            );
        }
        if outcome.exhausted {
            content
                .push_str("\n\n[note: the subagent ran out of turns, so this may be incomplete]");
        }
        if outcome.blocked_sends > 0 {
            content
                .push_str("\n\n[note: the subagent attempted an outbound call that was blocked]");
        }

        let output = ToolOutput::ok(content);
        // Marking the answer as external is what keeps the parent's interlock
        // honest — see the module docs on why a summary is not laundering.
        Ok(if self.capabilities.untrusted_input {
            output.from_outside()
        } else {
            output
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_defaults_are_conservative() {
        let p = SubagentProfile::default();
        assert!(
            p.tools.is_empty(),
            "a profile grants no tools unless it says so"
        );
        assert!(
            !p.trusted_output,
            "child output is untrusted unless opted out"
        );
    }
}
