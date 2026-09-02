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

    /// A rule's `allow` is a yes written down in advance; this approver has
    /// no mode of its own to consult.
    async fn permit(&self, _tool: &dyn Tool, _input: &Value) -> Decision {
        Decision::Allow
    }
}

impl TuiApprover {
    async fn ask(&self, tool: &dyn Tool, summary: String, escalated: bool) -> Decision {
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
            // "Always" at an escalation would install a standing yes on the
            // ordinary path that `escalate` deliberately bypasses; it allows
            // this call only.
            Ok(Answer::Always) if escalated => Decision::Allow,
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
