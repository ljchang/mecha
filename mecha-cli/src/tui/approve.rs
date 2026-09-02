//! Approval, asked through the interface instead of through stdin.
//!
//! The terminal approver prints a prompt and blocks on `read_line`. Under a TUI
//! that is doubly wrong: stdin belongs to the event loop, and printing straight
//! to the screen would tear the frame. So the approver becomes a message: it
//! sends the pending call to the UI and waits on a one-shot for the answer.
//!
//! The approver is still what the agent loop calls, so the interlock ordering is
//! unchanged — the trifecta check runs *before* this, and a user cannot approve
//! their way past it.

use async_trait::async_trait;
use mecha_core::tool::{Approver, Decision, Tool};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

/// One call waiting on a human.
pub struct Request {
    pub tool: String,
    /// One line describing what the call will actually do.
    pub summary: String,
    pub reply: oneshot::Sender<Answer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Allow,
    /// Allow this tool for the rest of the process. Never persisted: a decision
    /// made in one session should not silently apply to the next.
    Always,
    Deny,
}

pub struct TuiApprover {
    tx: mpsc::UnboundedSender<Request>,
    always: Mutex<HashSet<String>>,
}

impl TuiApprover {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Request>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            TuiApprover {
                tx,
                always: Mutex::new(HashSet::new()),
            },
            rx,
        )
    }
}

#[async_trait]
impl Approver for TuiApprover {
    async fn approve(&self, tool: &dyn Tool, input: &Value) -> Decision {
        if self.always.lock().is_ok_and(|a| a.contains(tool.name())) {
            return Decision::Allow;
        }
        self.ask(tool, crate::approve::summarize(tool.name(), input), false)
            .await
    }

    /// Past the `always` list on purpose: an escalation is the interlock
    /// asking a person about *this* call, and a standing yes for the tool is
    /// not that. The reason rides in the summary the modal shows.
    async fn escalate(&self, tool: &dyn Tool, input: &Value, why: &str) -> Decision {
        let summary = format!("{why} {}", crate::approve::summarize(tool.name(), input));
        self.ask(tool, summary, true).await
    }

    /// Past the `always` list for the same reason: a `prompt` rule is the
    /// operator asking that a person see *this* call, whatever standing yes
    /// the tool has collected. The ruling's sentence rides in the summary.
    async fn consult(&self, tool: &dyn Tool, input: &Value, why: &str) -> Decision {
        let summary = format!("{why} {}", crate::approve::summarize(tool.name(), input));
        self.ask(tool, summary, true).await
    }

    /// A rule's `allow` is a yes written down in advance; this approver has
    /// no mode of its own to consult.
    async fn permit(&self, _tool: &dyn Tool, _input: &Value) -> Decision {
        Decision::Allow
    }
}

impl TuiApprover {
    /// `forced` for an escalation or a `prompt` rule: the answer "always" then
    /// allows this call only, because installing a standing yes from a prompt
    /// that exists to bypass standing yeses would defeat the next one.
    async fn ask(&self, tool: &dyn Tool, summary: String, forced: bool) -> Decision {
        let (reply, answer) = oneshot::channel();
        let request = Request {
            tool: tool.name().to_string(),
            summary,
            reply,
        };

        // The UI is gone, so nobody can consent — and nobody said no either.
        // `Blocked`, not `Deny`: a refusal no human made must not be mined as
        // a correction, the rule `Approver::escalate`'s default states.
        if self.tx.send(request).is_err() {
            return Decision::Blocked("the interface closed before this was approved".into());
        }

        match answer.await {
            Ok(Answer::Allow) => Decision::Allow,
            Ok(Answer::Always) if forced => Decision::Allow,
            Ok(Answer::Always) => {
                if let Ok(mut always) = self.always.lock() {
                    always.insert(tool.name().to_string());
                }
                Decision::Allow
            }
            Ok(Answer::Deny) => Decision::Deny("the user declined this call".into()),
            // Dropped without answering — the run was cancelled out from under
            // it, or the UI quit. Same reasoning as above: nobody spoke.
            Err(_) => Decision::Blocked("the request was dismissed without an answer".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::tool::{ToolCtx, ToolOutput};
    use serde_json::json;

    struct Shell;
    #[async_trait]
    impl Tool for Shell {
        fn name(&self) -> &str {
            "shell"
        }
        fn description(&self) -> &str {
            "a test tool"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            false
        }
        async fn call(&self, _input: Value, _ctx: &ToolCtx) -> anyhow::Result<ToolOutput> {
            unreachable!("the approver never runs the tool")
        }
    }

    /// One "always" to `shell: ls` must not cover a later `shell: cargo
    /// publish` the operator wrote a `prompt` rule for: `consult` asks past
    /// the standing yes, and answering "always" there installs nothing.
    #[tokio::test]
    async fn a_prompt_rule_is_asked_past_the_standing_yes() {
        let (a, mut rx) = TuiApprover::new();
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(req) = rx.recv().await {
                seen.push(req.summary.clone());
                req.reply.send(Answer::Always).ok();
            }
            seen
        });

        // "Always" on an ordinary approval installs the standing yes …
        assert!(matches!(
            a.approve(&Shell, &json!({"command": "ls"})).await,
            Decision::Allow
        ));
        // … which `approve` honours without asking …
        assert!(matches!(
            a.approve(&Shell, &json!({"command": "ls -la"})).await,
            Decision::Allow
        ));
        // … and `consult` asks past, twice: "always" at a forced prompt
        // allows one call only.
        for _ in 0..2 {
            assert!(matches!(
                a.consult(&Shell, &json!({"command": "cargo publish"}), "a rule asks")
                    .await,
                Decision::Allow
            ));
        }
        drop(a);
        let seen = task.await.unwrap();
        assert_eq!(
            seen.len(),
            3,
            "one ordinary ask, then both consults: {seen:?}"
        );
        assert!(seen[1].starts_with("a rule asks"), "{:?}", seen[1]);
        assert!(seen[2].starts_with("a rule asks"), "{:?}", seen[2]);
    }
}
