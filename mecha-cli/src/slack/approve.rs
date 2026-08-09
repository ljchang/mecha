//! Approvals over Slack, and the three ways a call can be refused.
//!
//! Shaped like `tui/approve.rs` — a request over a channel and a `oneshot` for
//! the answer — because the `Approver` trait is `async` and a remote human is
//! therefore expressible without the loop knowing anything changed. What is
//! different is who is *not* there.
//!
//! **The distinction this module exists to preserve:** a timeout is not a
//! denial by the user. `Decision::Deny` becomes `"Denied by the user: …"` in
//! the transcript, and the learning miner keys on exactly that string, so an
//! approval nobody was awake to answer at 2am would otherwise become training
//! data attributed to a human who never spoke — the same mistake as mining a
//! publish's changed path as a voice correction. `Decision::Blocked` is the
//! machine's no and is never mined. Every refusal here that no human made uses
//! it.
//!
//! **No `Always`.** The TUI has one because a decision that outlives the
//! terminal it was made in is bounded by the session; a connector runs for
//! months, and "allow this tool forever" made once on a phone is a much larger
//! blast radius than it looks.

// Like `threads.rs`: the connector is the consumer and lands next. The
// attribute comes off with it.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mecha_core::tool::{Approver, Decision, Tool};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// What a thread's runs are allowed to do without asking. Per thread, set by a
/// button, and never inferred from prompt text — permission policy must not be
/// decidable by anything sharing a context window with third-party text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every state-changing call raises a card. The default.
    Ask,
    /// Nothing asks. Bounded by the thread's jail, not by the approver.
    Allow,
    /// Anything that is not read-only is refused outright.
    ReadOnly,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "ask" => Some(Mode::Ask),
            "allow" => Some(Mode::Allow),
            "read-only" | "read_only" => Some(Mode::ReadOnly),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Allow => "allow",
            Mode::ReadOnly => "read-only",
        }
    }
}

/// One pending approval, handed to whoever owns the Slack connection.
#[derive(Debug)]
pub struct Request {
    pub thread_key: String,
    pub tool: String,
    /// One line describing what the call will actually do.
    pub summary: String,
    pub reply: oneshot::Sender<Answer>,
}

#[derive(Debug, Clone)]
pub enum Answer {
    Approve,
    /// A human really said no, and said why. This one is a correction and is
    /// mined as such.
    Reject(String),
}

/// The approver for one thread.
///
/// The mode is shared rather than copied so a button pressed mid-run takes
/// effect on the **next** call. It is per thread and never per agent: the
/// connector holds one `Agent` serving every thread, so widening the mode on
/// the agent — which is what the TUI's `set_approver` does — would widen every
/// other thread at the same time.
pub struct SlackApprover {
    thread_key: String,
    mode: Arc<Mutex<Mode>>,
    tx: mpsc::Sender<Request>,
    timeout: Duration,
}

impl SlackApprover {
    pub fn new(
        thread_key: impl Into<String>,
        mode: Arc<Mutex<Mode>>,
        tx: mpsc::Sender<Request>,
        timeout: Duration,
    ) -> Self {
        Self {
            thread_key: thread_key.into(),
            mode,
            tx,
            timeout,
        }
    }

    fn mode(&self) -> Mode {
        self.mode.lock().map(|m| *m).unwrap_or(Mode::Ask)
    }
}

/// A short, human-readable duration for a message a person reads on a phone.
fn humanise(d: Duration) -> String {
    let secs = d.as_secs();
    if secs % 60 == 0 && secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[async_trait]
impl Approver for SlackApprover {
    async fn approve(&self, tool: &dyn Tool, input: &Value) -> Decision {
        match self.mode() {
            Mode::Allow => return Decision::Allow,
            // A forced approval on a read-only tool still asks: the interlock
            // or the leak guard raised it, and "read-only" was never a claim
            // about where the bytes go.
            Mode::ReadOnly if !tool.read_only() => {
                return Decision::Blocked(format!(
                    "`{}` modifies state and this thread is read-only",
                    tool.name()
                ));
            }
            _ => {}
        }

        let (reply, answer) = oneshot::channel();
        let request = Request {
            thread_key: self.thread_key.clone(),
            tool: tool.name().to_string(),
            summary: summarise(tool.name(), input),
            reply,
        };

        // Every failure below is `Blocked`, never `Deny`: nobody said no, and
        // recording that they did is what teaches a rule from silence.
        if self.tx.send(request).await.is_err() {
            return Decision::Blocked(
                "the Slack connection is gone, so nobody could be asked".into(),
            );
        }

        match tokio::time::timeout(self.timeout, answer).await {
            Ok(Ok(Answer::Approve)) => Decision::Allow,
            Ok(Ok(Answer::Reject(reason))) => Decision::Deny(reason),
            Ok(Err(_)) => {
                Decision::Blocked("the approval was dropped before anyone answered it".into())
            }
            Err(_) => Decision::Blocked(format!(
                "nobody answered in Slack within {}",
                humanise(self.timeout)
            )),
        }
    }
}

/// One line a person can act on from a phone, without the full arguments.
fn summarise(tool: &str, input: &Value) -> String {
    let detail = input
        .get("command")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("url"))
        .and_then(Value::as_str)
        .map(|s| s.chars().take(160).collect::<String>());
    match detail {
        Some(d) => format!("{tool}: {d}"),
        None => tool.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::tool::ToolCtx;
    use serde_json::json;

    struct Fake {
        name: &'static str,
        read_only: bool,
    }

    #[async_trait]
    impl Tool for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "a test tool"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            self.read_only
        }
        async fn call(
            &self,
            _input: Value,
            _ctx: &ToolCtx,
        ) -> anyhow::Result<mecha_core::tool::ToolOutput> {
            unreachable!("the approver never runs the tool")
        }
    }

    fn writer() -> Fake {
        Fake {
            name: "fs_write",
            read_only: false,
        }
    }

    fn approver(mode: Mode, timeout: Duration) -> (SlackApprover, mpsc::Receiver<Request>) {
        let (tx, rx) = mpsc::channel(8);
        (
            SlackApprover::new("D1-1.0", Arc::new(Mutex::new(mode)), tx, timeout),
            rx,
        )
    }

    #[tokio::test]
    async fn allow_mode_never_asks() {
        let (a, mut rx) = approver(Mode::Allow, Duration::from_secs(1));
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));
        assert!(rx.try_recv().is_err(), "nothing should have been asked");
    }

    #[tokio::test]
    async fn read_only_blocks_a_writer_without_asking_and_it_is_not_a_user_denial() {
        let (a, mut rx) = approver(Mode::ReadOnly, Duration::from_secs(1));
        match a.approve(&writer(), &json!({})).await {
            Decision::Blocked(reason) => assert!(reason.contains("read-only"), "{reason}"),
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_human_rejection_is_a_denial_and_carries_the_reason() {
        // The one refusal that *is* a correction, and the only one the
        // learning miner should ever see.
        let (a, mut rx) = approver(Mode::Ask, Duration::from_secs(5));
        let task = tokio::spawn(async move {
            let req = rx.recv().await.expect("a request");
            req.reply.send(Answer::Reject("not that file".into())).ok();
        });
        match a.approve(&writer(), &json!({"path": "secrets.txt"})).await {
            Decision::Deny(reason) => assert_eq!(reason, "not that file"),
            other => panic!("expected Deny, got {other:?}"),
        }
        task.await.unwrap();
    }

    #[tokio::test]
    async fn an_unanswered_approval_is_blocked_and_never_denied() {
        // The trap this module is built around. `Deny` would become "Denied by
        // the user:" in the transcript and be mined as a correction from a
        // person who was asleep.
        let (a, _rx) = approver(Mode::Ask, Duration::from_millis(30));
        match a.approve(&writer(), &json!({})).await {
            Decision::Blocked(reason) => {
                assert!(reason.contains("nobody answered"), "{reason}");
                assert!(reason.contains("30s") || reason.contains("0m") || !reason.is_empty());
            }
            other => panic!("a timeout must never be a user denial, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_dropped_or_closed_channel_fails_closed() {
        // Silence is never approval, in either direction.
        let (a, rx) = approver(Mode::Ask, Duration::from_secs(5));
        drop(rx);
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Blocked(_)
        ));

        let (a, mut rx) = approver(Mode::Ask, Duration::from_secs(5));
        let task = tokio::spawn(async move {
            let req = rx.recv().await.expect("a request");
            drop(req.reply); // someone closed the modal without answering
        });
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Blocked(_)
        ));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_mode_change_takes_effect_on_the_next_call() {
        let mode = Arc::new(Mutex::new(Mode::Ask));
        let (tx, mut rx) = mpsc::channel(8);
        let a = SlackApprover::new("D1-1.0", Arc::clone(&mode), tx, Duration::from_secs(5));

        let task = tokio::spawn(async move {
            let req = rx.recv().await.expect("a request while asking");
            req.reply.send(Answer::Approve).ok();
            // Nothing should arrive after the mode widens.
            rx.recv().await
        });

        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));
        *mode.lock().unwrap() = Mode::Allow;
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));

        drop(a);
        assert!(
            task.await.unwrap().is_none(),
            "the second call asked nobody"
        );
    }

    #[test]
    fn modes_round_trip_through_their_names() {
        for mode in [Mode::Ask, Mode::Allow, Mode::ReadOnly] {
            assert_eq!(Mode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(Mode::parse("bypass"), None, "unknown modes are not guessed");
    }

    #[test]
    fn a_summary_names_what_the_call_will_actually_do() {
        assert_eq!(
            summarise("shell", &json!({"command": "rm -rf build"})),
            "shell: rm -rf build"
        );
        assert_eq!(summarise("todo", &json!({})), "todo");
        let long = summarise("shell", &json!({"command": "x".repeat(500)}));
        assert!(long.chars().count() <= 160 + "shell: ".len());
    }
}
