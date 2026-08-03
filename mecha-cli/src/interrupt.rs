//! Making a run interruptible from the terminal.
//!
//! Ctrl-C during a run means "stop this", not "kill mecha". The distinction
//! matters: killing the process loses the session, the partial answer, and any
//! work the agent had already done, and the user pressing Ctrl-C almost never
//! wants that — they want to redirect.

use mecha_core::agent::{Agent, AgentEvent, Conversation, RunContext, RunOutcome};

use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

/// Run the agent with Ctrl-C wired to cancellation.
///
/// The first Ctrl-C cancels: the loop stops at the next safe point and keeps
/// what it has. A second one is left to the default handler, so a wedged run is
/// still killable — an uninterruptible process is worse than a lost session.
pub async fn run_interruptible(
    agent: &Agent,
    cx: &RunContext,
    convo: &mut Conversation,
    events: Option<UnboundedSender<AgentEvent>>,
) -> anyhow::Result<RunOutcome> {
    let token = CancellationToken::new();
    let cx = cx.clone().with_cancel(token.clone());

    // Watch in a task rather than selecting on the run itself: selecting would
    // drop the run future on Ctrl-C, which throws away the very partial answer
    // cancellation exists to preserve. Cancel, then let the run wind itself up.
    let watcher = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    if signal.is_ok() {
                        eprintln!("\n^C — stopping after the current step. Ctrl-C again to force.");
                        token.cancel();
                    }
                }
                // The run finished on its own; stop listening so the next one
                // gets a fresh handler.
                _ = token.cancelled() => {}
            }
        })
    };

    let result = agent.run_in(&cx, convo, events).await;

    // Ends the watcher whichever way the run went, and releases the signal
    // handler so a later Ctrl-C at the prompt behaves normally.
    token.cancel();
    let _ = watcher.await;

    result
}

// The other half of interruption — steering, via `RunContext::queued_input` —
// has no consumer here on purpose. Reading stdin *while* a run streams needs a
// second reader on the same file descriptor, and whichever one is blocked when
// the run ends steals the user's next prompt line. The fix is a single owner of
// input with a persistent input area, which is a TUI. See docs/HANDOFF.md.
