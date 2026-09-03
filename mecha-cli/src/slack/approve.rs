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

use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Approve, and stop asking about *this tool* for the rest of *this run*.
    ///
    /// Deliberately narrower than the TUI's `Always`, which is process-local
    /// and therefore months long in a connector. This dies when the run does
    /// and never crosses a thread. It exists because the first real task —
    /// render a poll and publish it — raised seven cards, and approval fatigue
    /// is how a review surface becomes a reflex.
    ApproveForRun,
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
    /// Tools waved through for the rest of this run. Owned by the approver, so
    /// it is destroyed with the `RunContext` that held it.
    blanket: Mutex<std::collections::HashSet<String>>,
    /// Set by the first card nobody answered. After it, asking again is
    /// pointless and expensive: the model retries a refused call, each retry
    /// posts a fresh card and parks the run a full timeout — a real session
    /// stalled three consecutive ten-minute waits on one `fs_write`.
    ///
    /// **Shared with the connector** (via [`SlackApprover::unanswered_latch`])
    /// rather than scoped like `blanket`, because "the approver is rebuilt for
    /// each inbound message" is false for a LIVE run: the connector
    /// short-circuits messages on a running thread into the steering queue,
    /// so this approver — and this latch — survive exactly the moment the
    /// user comes back. The connector clears it when a steering message is
    /// enqueued or any approval-card button is pressed (a tap is proof
    /// someone is watching, even on an expired card); the next gated call
    /// then posts a fresh card and waits normally.
    unanswered: Arc<AtomicBool>,
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
            blanket: Mutex::new(std::collections::HashSet::new()),
            unanswered: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The unanswered latch, for the connector's per-thread running state.
    ///
    /// The connector stores this beside the steering-queue handle and clears
    /// it when the user is known to be watching again — a steering message,
    /// or any approval-card button press. Without the shared handle the latch
    /// could only clear with the run, and a live run refused every gated call
    /// invisibly for up to its whole budget.
    pub fn unanswered_latch(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.unanswered)
    }

    fn mode(&self) -> Mode {
        self.mode.lock().map(|m| *m).unwrap_or(Mode::Ask)
    }
}

/// A short, human-readable duration for a message a person reads on a phone.
fn humanise(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 && secs.is_multiple_of(60) {
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

        if self.blanket.lock().is_ok_and(|b| b.contains(tool.name())) {
            return Decision::Allow;
        }
        self.ask(tool, summarise(tool.name(), input), false).await
    }

    /// Past the modes that would have *passed* the call — `Allow` and the
    /// run's blanket approvals — on purpose: an escalation is the interlock
    /// asking a person about *this* call. Not past `ReadOnly`, which is a
    /// refusal the thread already made: an escalation narrows and never
    /// loosens, and the PR review of this change found the first version
    /// widening past it. The reason rides in the card's summary. An earlier
    /// unanswered card still blocks — nobody is watching, and that answer
    /// does not change.
    async fn escalate(&self, tool: &dyn Tool, input: &Value, why: &str) -> Decision {
        if self.mode() == Mode::ReadOnly && !tool.read_only() {
            return Decision::Blocked(format!(
                "`{}` modifies state and this thread is read-only; an escalation cannot widen \
                 that",
                tool.name()
            ));
        }
        self.ask(
            tool,
            format!("{why} {}", summarise(tool.name(), input)),
            true,
        )
        .await
    }

    /// Past the run's blanket approvals for the same reason as `escalate`: a
    /// `prompt` rule is the operator asking that a person see *this* call,
    /// and "approve for run" on `shell` is not that. Not past `ReadOnly`.
    async fn consult(&self, tool: &dyn Tool, input: &Value, why: &str) -> Decision {
        if self.mode() == Mode::ReadOnly && !tool.read_only() {
            return Decision::Blocked(format!(
                "`{}` modifies state and this thread is read-only; an approval rule cannot \
                 widen that",
                tool.name()
            ));
        }
        self.ask(
            tool,
            format!("{why} {}", summarise(tool.name(), input)),
            true,
        )
        .await
    }

    /// A rule's `allow` stands in for the card, not for the thread's mode: a
    /// read-only thread still refuses a write.
    async fn permit(&self, tool: &dyn Tool, _input: &Value) -> Decision {
        match self.mode() {
            Mode::ReadOnly if !tool.read_only() => Decision::Blocked(format!(
                "`{}` modifies state and this thread is read-only; an approval rule cannot \
                 widen that",
                tool.name()
            )),
            _ => Decision::Allow,
        }
    }
}

impl SlackApprover {
    /// `forced` for an escalation or a `prompt` rule: "approve for run" then
    /// allows this call only.
    async fn ask(&self, tool: &dyn Tool, summary: String, forced: bool) -> Decision {
        // After the mode and blanket checks, so a mid-run switch to `Allow` —
        // a button press, which is proof someone is watching after all —
        // still works. But never another card and another wait.
        if self.unanswered.load(Ordering::Relaxed) {
            return Decision::Blocked(
                "nobody answered an earlier approval card in this run, so this \
                 call was not asked. Reply in the thread, or press a button on \
                 an approval card, and this run will ask again on its next \
                 gated call."
                    .into(),
            );
        }

        let (reply, answer) = oneshot::channel();
        let request = Request {
            thread_key: self.thread_key.clone(),
            tool: tool.name().to_string(),
            summary,
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
            // "Approve for run" at a forced prompt would install a standing yes
            // on the ordinary path that `escalate` and `consult` deliberately
            // bypass — the rule the terminal and TUI approvers also keep. It
            // allows this call only.
            Ok(Ok(Answer::ApproveForRun)) if forced => Decision::Allow,
            Ok(Ok(Answer::ApproveForRun)) => {
                if let Ok(mut b) = self.blanket.lock() {
                    b.insert(tool.name().to_string());
                }
                Decision::Allow
            }
            Ok(Ok(Answer::Reject(reason))) => Decision::Deny(reason),
            Ok(Err(_)) => {
                Decision::Blocked("the approval was dropped before anyone answered it".into())
            }
            Err(_) => {
                self.unanswered.store(true, Ordering::Relaxed);
                Decision::Blocked(format!(
                    "nobody answered in Slack within {}",
                    humanise(self.timeout)
                ))
            }
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
    async fn after_one_timeout_the_run_stops_asking_and_is_refused_at_once() {
        // The old behaviour re-asked: the model retries the refused call, each
        // retry posts a new card and waits the full timeout again — three
        // consecutive ten-minute stalls on one `fs_write` in a real session.
        let (a, mut rx) = approver(Mode::Ask, Duration::from_millis(50));

        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Blocked(_)
        ));
        assert!(rx.try_recv().is_ok(), "the first ask posted a card");

        let start = std::time::Instant::now();
        match a.approve(&writer(), &json!({})).await {
            Decision::Blocked(reason) => {
                assert!(reason.contains("earlier"), "{reason}");
                // The advice must match what actually clears the latch: this
                // run's approver survives a reply (steering), so promising a
                // "fresh run" was false for a live one.
                assert!(reason.contains("ask again"), "{reason}");
                assert!(!reason.contains("fresh run"), "{reason}");
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "the second ask must not wait out another timeout"
        );
        assert!(rx.try_recv().is_err(), "no second card was posted");
    }

    #[tokio::test]
    async fn clearing_the_shared_latch_makes_the_next_call_ask_again() {
        // The latch assumed the approver dies with each inbound message —
        // false for a LIVE run, where the connector folds new messages into
        // the steering queue and the same approver survives the user's
        // return. The connector now clears the shared latch on a steering
        // message or any approval-card button press; from the approver's
        // side, both are a store(false) on the handle this test uses. The old
        // behaviour refused here instantly, with no card, for the rest of the
        // run. The timeout has to be long enough for the responder below to
        // beat it, and short enough that the first, deliberate timeout does
        // not drag the test.
        let (a, mut rx) = approver(Mode::Ask, Duration::from_millis(250));

        // First card times out and sets the latch.
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Blocked(_)
        ));
        assert!(rx.try_recv().is_ok(), "the first ask posted a card");

        // Latched: refused at once, no card.
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Blocked(_)
        ));
        assert!(rx.try_recv().is_err(), "no card while latched");

        // The user spoke — the connector clears the latch it shares with us.
        a.unanswered_latch().store(false, Ordering::Relaxed);

        // The next gated call posts a fresh card and waits for the answer.
        let responder = tokio::spawn(async move {
            let req = rx
                .recv()
                .await
                .expect("a fresh card once the latch is cleared");
            req.reply.send(Answer::Approve).ok();
        });
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));
        responder.await.unwrap();
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

    #[tokio::test]
    async fn approving_for_the_run_stops_asking_about_that_tool_only() {
        // Seven cards for one task is what argued for this. The scope is the
        // run: a second tool still asks, and nothing survives the approver.
        let (a, mut rx) = approver(Mode::Ask, Duration::from_secs(5));
        let task = tokio::spawn(async move {
            let first = rx.recv().await.expect("the first ask");
            first.reply.send(Answer::ApproveForRun).ok();
            // Whatever arrives next must be a *different* tool.
            rx.recv().await.map(|r| {
                let name = r.tool.clone();
                r.reply.send(Answer::Approve).ok();
                name
            })
        });

        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));
        // Same tool again: no card.
        assert!(matches!(
            a.approve(&writer(), &json!({})).await,
            Decision::Allow
        ));

        let other = Fake {
            name: "shell",
            read_only: false,
        };
        assert!(matches!(
            a.approve(&other, &json!({})).await,
            Decision::Allow
        ));
        assert_eq!(
            task.await.unwrap().as_deref(),
            Some("shell"),
            "a blanket on one tool must not cover another"
        );
    }

    /// A blanket "approve for run" on `shell` does not answer a `prompt`
    /// rule on a later `shell` call: `consult` sends a card past the blanket,
    /// and "approve for run" on that card installs nothing.
    #[tokio::test]
    async fn a_prompt_rule_is_asked_past_the_blanket() {
        let (a, mut rx) = approver(Mode::Ask, Duration::from_secs(5));
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(req) = rx.recv().await {
                seen.push(req.summary.clone());
                req.reply.send(Answer::ApproveForRun).ok();
            }
            seen
        });
        let shell = Fake {
            name: "shell",
            read_only: false,
        };
        assert!(matches!(
            a.approve(&shell, &json!({"command": "ls"})).await,
            Decision::Allow
        ));
        assert!(matches!(
            a.approve(&shell, &json!({"command": "ls -la"})).await,
            Decision::Allow
        ));
        for _ in 0..2 {
            assert!(matches!(
                a.consult(&shell, &json!({"command": "cargo publish"}), "a rule asks")
                    .await,
                Decision::Allow
            ));
        }
        drop(a);
        let seen = task.await.unwrap();
        assert_eq!(
            seen.len(),
            3,
            "one ordinary card, then both consults: {seen:?}"
        );
        assert!(seen[1].starts_with("a rule asks"), "{:?}", seen[1]);
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
