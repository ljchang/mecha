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

use crate::agent::{Agent, RunContext};
use crate::message::Message;
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

        Subagent { profile, agent, capabilities }
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
        // is the context-isolation half of why subagents are useful.
        let mut messages = vec![Message::user(task)];

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
            // subagent running would be a lie.
            cancel: self.agent.context().cancel.clone(),
            // Steering is addressed to the parent. The child was given a
            // self-contained task and has no conversation to redirect.
            queued_input: None,
        };

        let outcome = match self.agent.run_in(&cx, &mut messages, None).await {
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
            content.push_str(
                "\n\n[note: the subagent ran out of turns, so this may be incomplete]",
            );
        }
        if outcome.blocked_sends > 0 {
            content.push_str(
                "\n\n[note: the subagent attempted an outbound call that was blocked]",
            );
        }

        let output = ToolOutput::ok(content);
        // Marking the answer as external is what keeps the parent's interlock
        // honest — see the module docs on why a summary is not laundering.
        Ok(if self.capabilities.untrusted_input { output.from_outside() } else { output })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_defaults_are_conservative() {
        let p = SubagentProfile::default();
        assert!(p.tools.is_empty(), "a profile grants no tools unless it says so");
        assert!(!p.trusted_output, "child output is untrusted unless opted out");
    }
}
