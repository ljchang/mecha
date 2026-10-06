//! Background jobs: a slow side effect that outlives the turn
//! (`docs/BACKGROUND-JOBS-DESIGN.md`, PERSONA-CONTEXT-DESIGN §5.4, R3).
//!
//! A tool may answer at once with what it has started and hand the slow half
//! over as a [`DeferredJob`] (`ToolOutput::deferred`). Who runs it is the
//! run's business, never the tool's:
//!
//! - **A chat host** gives the run a [`JobSink`] (`RunContext::jobs`) — a
//!   [`JobQueue`] keyed by the conversation — and the job runs there, past
//!   the run that started it. Talking cancels the run, never the job; the
//!   owner's Stop cancels both (§2.4).
//! - **Every other path** (the CLI, a subagent, an eval, a batch) has no
//!   sink, and the loop awaits the job inline, linked to the run's own
//!   cancellation: it behaves exactly as the tool did before it deferred,
//!   Stop included (§2.1).
//!
//! The job carries its own [`CancellationToken`], created by the tool and
//! watched by the job, because a boxed future can be dropped but not handed
//! a token afterwards, and the tool cannot know whether the run has a sink.
//!
//! One job per conversation at a time (§2.2): a second is refused with the
//! tool's own words ([`DeferredJob::busy`]), never picture prose in this
//! generic queue.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

use crate::tool::ToolOutput;

/// The slow half of a deferred tool call: run once, by whoever holds it.
pub struct DeferredJob {
    job: Mutex<Option<BoxFuture<'static, ToolOutput>>>,
    cancel: CancellationToken,
    busy: String,
}

impl DeferredJob {
    /// `job` must watch `cancel` and end with an output saying it stopped
    /// when it fires; `busy` is the refusal a second job of the same
    /// conversation gets while this one runs — the tool's words.
    pub fn new(
        job: impl Future<Output = ToolOutput> + Send + 'static,
        cancel: CancellationToken,
        busy: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(DeferredJob {
            job: Mutex::new(Some(Box::pin(job))),
            cancel,
            busy: busy.into(),
        })
    }

    /// The future, once: whoever takes it runs it.
    pub(crate) fn take(&self) -> Option<BoxFuture<'static, ToolOutput>> {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The token the job watches.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// What a second job of this conversation is told while this one runs.
    pub fn busy(&self) -> &str {
        &self.busy
    }
}

impl std::fmt::Debug for DeferredJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeferredJob")
            .field("busy", &self.busy)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// A job refused because the conversation already has one running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// Where a run hands a deferred job: a chat host's queue, for one
/// conversation (`RunContext::jobs`).
pub trait JobSink: Send + Sync {
    /// Take the job and run it, or refuse it: one at a time.
    fn submit(&self, call_id: &str, tool: &str, job: Arc<DeferredJob>) -> Result<(), Busy>;
}

/// A finished job, handed to the host's delivery: the call it answers, and
/// what it came to.
#[derive(Debug, Clone)]
pub struct Delivered {
    pub key: String,
    pub call_id: String,
    pub tool: String,
    pub output: ToolOutput,
}

type Deliver = dyn Fn(Delivered) + Send + Sync;

/// One job in flight per key — per conversation, by the host's session key
/// (`agent::Conversation` has no identity of its own) — run on the tokio
/// runtime, past the run that submitted it, and handed to `deliver` when it
/// ends: finished, failed or cancelled alike.
pub struct JobQueue {
    running: Mutex<HashMap<String, (String, CancellationToken)>>,
    deliver: Arc<Deliver>,
}

impl JobQueue {
    pub fn new(deliver: impl Fn(Delivered) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(JobQueue {
            running: Mutex::new(HashMap::new()),
            deliver: Arc::new(deliver),
        })
    }

    /// Run `job` for `key`, or refuse it if `key` already has one.
    pub fn submit(
        self: &Arc<Self>,
        key: &str,
        call_id: &str,
        tool: &str,
        job: Arc<DeferredJob>,
    ) -> Result<(), Busy> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains_key(key) {
            return Err(Busy);
        }
        let Some(fut) = job.take() else {
            // Already run elsewhere: nothing to start, and nothing to claim.
            return Err(Busy);
        };
        running.insert(
            key.to_string(),
            (call_id.to_string(), job.cancel_token().clone()),
        );
        drop(running);
        let queue = Arc::clone(self);
        let (key, call_id, tool) = (key.to_string(), call_id.to_string(), tool.to_string());
        tokio::spawn(async move {
            let output = fut.await;
            {
                let mut running = queue.running.lock().unwrap_or_else(|e| e.into_inner());
                if running.get(&key).is_some_and(|(id, _)| *id == call_id) {
                    running.remove(&key);
                }
            }
            (queue.deliver)(Delivered {
                key,
                call_id,
                tool,
                output,
            });
        });
        Ok(())
    }

    /// Cancel `key`'s job, if it has one — the owner's Stop (§2.4). The job
    /// ends by its own cancelled output, which is delivered like any other.
    pub fn cancel(&self, key: &str) -> bool {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        match running.get(key) {
            Some((_, token)) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// The call `key`'s running job answers, if any.
    pub fn pending(&self, key: &str) -> Option<String> {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        running.get(key).map(|(id, _)| id.clone())
    }

    /// This queue as one conversation's sink.
    pub fn sink(self: &Arc<Self>, key: impl Into<String>) -> Arc<dyn JobSink> {
        Arc::new(KeyedSink {
            queue: Arc::clone(self),
            key: key.into(),
        })
    }
}

struct KeyedSink {
    queue: Arc<JobQueue>,
    key: String,
}

impl JobSink for KeyedSink {
    fn submit(&self, call_id: &str, tool: &str, job: Arc<DeferredJob>) -> Result<(), Busy> {
        self.queue.submit(&self.key, call_id, tool, job)
    }
}

/// Run a deferred job inline, as the call would have run before it
/// deferred: the run's cancellation reaches the job's own token, so a Stop
/// still stops it (§2.1). The loop's path when the run has no sink.
pub(crate) async fn run_inline(
    job: &DeferredJob,
    run_cancel: Option<&CancellationToken>,
) -> ToolOutput {
    let Some(fut) = job.take() else {
        return ToolOutput::err("this work was already started elsewhere");
    };
    let Some(run_cancel) = run_cancel.cloned() else {
        return fut.await;
    };
    let job_cancel = job.cancel_token().clone();
    let link = tokio::spawn(async move {
        run_cancel.cancelled().await;
        job_cancel.cancel();
    });
    let output = fut.await;
    link.abort();
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A job that waits to be released or cancelled, and says which.
    fn gated(go: Arc<tokio::sync::Notify>, cancel: CancellationToken) -> Arc<DeferredJob> {
        let token = cancel.clone();
        DeferredJob::new(
            async move {
                tokio::select! {
                    _ = go.notified() => ToolOutput::ok("made"),
                    _ = token.cancelled() => ToolOutput::err("stopped, nothing made"),
                }
            },
            cancel,
            "not made: one is already being made",
        )
    }

    fn collecting() -> (Arc<JobQueue>, Arc<Mutex<Vec<Delivered>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&got);
        (JobQueue::new(move |d| sink.lock().unwrap().push(d)), got)
    }

    async fn delivered(got: &Arc<Mutex<Vec<Delivered>>>, n: usize) {
        for _ in 0..200 {
            if got.lock().unwrap().len() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("nothing was delivered");
    }

    #[tokio::test]
    async fn a_job_runs_past_its_submit_and_is_delivered_when_it_ends() {
        let (queue, got) = collecting();
        let go = Arc::new(tokio::sync::Notify::new());
        let job = gated(Arc::clone(&go), CancellationToken::new());
        queue.submit("chat", "c1", "draw", job).unwrap();
        assert_eq!(queue.pending("chat").as_deref(), Some("c1"));
        assert!(got.lock().unwrap().is_empty(), "delivered before it ended");
        go.notify_one();
        delivered(&got, 1).await;
        let d = &got.lock().unwrap()[0];
        assert_eq!(
            (
                d.key.as_str(),
                d.call_id.as_str(),
                d.output.content.as_str()
            ),
            ("chat", "c1", "made")
        );
        assert_eq!(
            queue.pending("chat"),
            None,
            "the slot frees when the job ends"
        );
    }

    /// One job per conversation: a second is refused while the first runs,
    /// and another conversation's is not.
    #[tokio::test]
    async fn one_job_per_conversation() {
        let (queue, _) = collecting();
        let go = Arc::new(tokio::sync::Notify::new());
        queue
            .submit(
                "chat",
                "c1",
                "draw",
                gated(Arc::clone(&go), CancellationToken::new()),
            )
            .unwrap();
        let second = gated(Arc::clone(&go), CancellationToken::new());
        assert_eq!(queue.submit("chat", "c2", "draw", second), Err(Busy));
        let elsewhere = gated(Arc::clone(&go), CancellationToken::new());
        assert_eq!(queue.submit("other", "c3", "draw", elsewhere), Ok(()));
    }

    /// The owner's Stop cancels the conversation's job, which ends by its own
    /// cancelled output, delivered like any other.
    #[tokio::test]
    async fn stop_cancels_the_conversations_job() {
        let (queue, got) = collecting();
        let job = gated(
            Arc::new(tokio::sync::Notify::new()),
            CancellationToken::new(),
        );
        queue.submit("chat", "c1", "draw", job).unwrap();
        assert!(queue.cancel("chat"));
        delivered(&got, 1).await;
        assert_eq!(
            got.lock().unwrap()[0].output.content,
            "stopped, nothing made"
        );
        assert!(!queue.cancel("chat"), "nothing left to stop");
    }

    /// Inline, the run's cancellation reaches the job's own token: a Stop on
    /// a path with no host still stops the work, as before it deferred.
    #[tokio::test]
    async fn inline_the_runs_stop_reaches_the_job() {
        let run = CancellationToken::new();
        let job = gated(
            Arc::new(tokio::sync::Notify::new()),
            CancellationToken::new(),
        );
        let stopper = run.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.cancel();
        });
        let out = run_inline(&job, Some(&run)).await;
        assert_eq!(out.content, "stopped, nothing made");
    }
}
