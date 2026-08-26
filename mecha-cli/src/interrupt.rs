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
    run_interruptible_watching(agent, cx, convo, events, None, None).await
}

/// [`run_interruptible`], plus a second thing that can ask the run to stop.
///
/// A run started from the web has no terminal to Ctrl-C, so "stop" arrives as
/// a file the run watches for (`mecha_core::runmarker`). Both routes cancel
/// the **same** token, which is the point: a sentinel that killed the process
/// instead would discard the partial turn that cancellation exists to keep,
/// and a detached run you can only kill is worse than the TUI's case rather
/// than equivalent to it.
///
/// Polled rather than pushed, at two seconds — the same cadence the trigger
/// runner uses, and slow enough that a run doing real work is not paying for
/// the ability to be stopped.
pub async fn run_interruptible_watching(
    agent: &Agent,
    cx: &RunContext,
    convo: &mut Conversation,
    events: Option<UnboundedSender<AgentEvent>>,
    stop: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    pump: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
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

    // The out-of-band stop, when the caller has one. Ends itself the moment
    // the token is cancelled, however that happened, so a finished run leaves
    // no poller behind.
    let watcher2 = stop.map(|stop| {
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                        if stop() {
                            eprintln!("mecha: stop requested — finishing the current step");
                            token.cancel();
                            return;
                        }
                    }
                }
            }
        })
    });

    // **Something to do on every watch tick**, and deliberately nothing more
    // specific than that. Its one caller drains a file of queued instructions
    // into this run's steering queue, so a detached run can be redirected
    // from another process — but the loop never learns that a steer can come
    // from a file, exactly as it never learns where a tool came from. Ends
    // with the token, like the stop poller beside it, so a finished run
    // leaves no poller behind.
    let watcher3 = pump.map(|pump| {
        let token = token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => pump(),
                }
            }
        })
    });

    let result = agent.run_in(&cx, convo, events).await;

    // Ends the watcher whichever way the run went, and releases the signal
    // handler so a later Ctrl-C at the prompt behaves normally.
    token.cancel();
    let _ = watcher.await;
    if let Some(w) = watcher2 {
        let _ = w.await;
    }
    if let Some(w) = watcher3 {
        let _ = w.await;
    }

    result
}

// The other half of interruption — steering, via `RunContext::queued_input` —
// has no consumer here on purpose. Reading stdin *while* a run streams needs a
// second reader on the same file descriptor, and whichever one is blocked when
// the run ends steals the user's next prompt line. The fix is a single owner of
// input with a persistent input area, which is a TUI. See CLAUDE.md,
// "Interruption and steering".
