//! The TUI's answer to [`mecha_core::tool::ask::Asker`].
//!
//! Same shape as the approval modal, and for the same reason: the tool runs in
//! the agent task, the question has to be rendered by the event loop, and the
//! answer has to travel back. A channel out, a oneshot back.
//!
//! The event loop is what guarantees the "never blocks forever" half of the
//! `Asker` contract — dropping the reply sender on shutdown resolves the
//! oneshot as an error, which becomes `None`, which the tool turns into "carry
//! on with your best guess".

use async_trait::async_trait;
use mecha_core::tool::ask::Asker;
use tokio::sync::{mpsc, oneshot};

/// A question waiting on the event loop.
pub struct Question {
    pub question: String,
    /// Empty for an open question, which the input line answers instead of a
    /// list.
    pub options: Vec<String>,
    pub reply: oneshot::Sender<Option<String>>,
}

pub struct TuiAsker {
    tx: mpsc::UnboundedSender<Question>,
}

impl TuiAsker {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<Question>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (TuiAsker { tx }, rx)
    }
}

#[async_trait]
impl Asker for TuiAsker {
    async fn ask(&self, question: &str, options: &[String]) -> Option<String> {
        let (reply, answer) = oneshot::channel();
        let sent = self.tx.send(Question {
            question: question.to_string(),
            options: options.to_vec(),
            reply,
        });
        // The interface has gone away: answer nothing rather than wait on a
        // receiver nobody is holding.
        if sent.is_err() {
            return None;
        }
        answer.await.ok().flatten()
    }
}
