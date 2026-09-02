//! The present human: live approvals and `ask_user`, routed to the page.
//!
//! The Slack connector proved the outbox right for an *absent* human; a page
//! open in a hand is a present one, and this module is what that buys. Both
//! shapes are one mechanism — a question posted to the session's event
//! stream, answered over one endpoint, or expiring into the honest refusal:
//!
//! - **Approval** (mode `ask`): the model's tool call parks in the approver
//!   until the page answers or two minutes pass. A tap is a real
//!   `Decision::Allow`/`Deny` — the loop prefixes a deny with "Denied by the
//!   user:", so the learning miner sees a genuine correction. A timeout is
//!   `Blocked` — machine policy, never mined as a person's no.
//! - **Ask** (`ask_user`): the option-card sheet. Ten minutes, then the tool's
//!   own measured decline wording — never an invitation to guess.
//!
//! Routing rides the run's jail: one agent serves every session, but each
//! run's workspace is `…/work/web/<key>`, so the workspace's file name *is*
//! the session key ([`mecha_core::tool::ask::Asker::ask_in`] exists for
//! exactly this). The Slack cancel lesson is kept: cancelling a run also
//! drains its pending questions, because a run parked in `approve()` never
//! sees the cancel token.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use mecha_core::config::PermissionMode;
use mecha_core::tool::ask::Asker;
use mecha_core::tool::{Approver, Decision, ModeApprover, Tool, ToolCtx};

use super::chat::WireEvent;

pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
pub const ASK_TIMEOUT: Duration = Duration::from_secs(600);

/// What the page sent back for one question.
#[derive(Debug)]
pub enum Answer {
    Approve,
    Deny(String),
    Text(String),
    Decline,
}

/// One session's outstanding questions. Cheap to clone; the map is the state.
#[derive(Clone, Default)]
pub struct Questions {
    next: Arc<AtomicU64>,
    pending: Arc<StdMutex<HashMap<u64, PendingQuestion>>>,
}

struct PendingQuestion {
    tx: oneshot::Sender<Answer>,
    /// The card as it went over the event stream — kept so a page that
    /// reloads mid-question (a locked phone kills the stream) gets the card
    /// back from the transcript read instead of a run silently parked on a
    /// question nobody can see.
    card: Option<WireEvent>,
}

impl Questions {
    fn open(&self) -> (u64, oneshot::Receiver<Answer>) {
        let qid = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(qid, PendingQuestion { tx, card: None });
        }
        (qid, rx)
    }

    /// Keep the card beside its channel, for the reload path.
    fn remember(&self, qid: u64, card: WireEvent) {
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(q) = pending.get_mut(&qid) {
                q.card = Some(card);
            }
        }
    }

    /// Every card still waiting — what the transcript read returns.
    pub fn cards(&self) -> Vec<WireEvent> {
        self.pending
            .lock()
            .map(|p| p.values().filter_map(|q| q.card.clone()).collect())
            .unwrap_or_default()
    }

    fn close(&self, qid: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&qid);
        }
    }

    /// Answer one question. False when it is unknown — already answered from
    /// another device, expired, or invented.
    pub fn answer(&self, qid: u64, answer: Answer) -> bool {
        let sender = self.pending.lock().ok().and_then(|mut p| p.remove(&qid));
        match sender {
            Some(q) => q.tx.send(answer).is_ok(),
            None => false,
        }
    }

    /// Drop every outstanding question — the cancel path. A parked
    /// `approve()` resolves as `Blocked`, an `ask` as a decline; neither
    /// waits out its timeout against a run that is already stopping.
    pub fn drain(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }
}

fn clip_args(input: &serde_json::Value) -> String {
    let pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
    if pretty.chars().count() > 2000 {
        let cut: String = pretty.chars().take(2000).collect();
        format!("{cut}\n… (truncated — the full call is in the session record)")
    } else {
        pretty
    }
}

/// The per-run approver: delegates to [`ModeApprover`] except in `ask` mode,
/// where the question goes to the page.
pub struct WebApprover {
    pub mode: Arc<StdMutex<PermissionMode>>,
    pub questions: Questions,
    pub events: tokio::sync::broadcast::Sender<WireEvent>,
    pub timeout: Duration,
}

#[async_trait]
impl Approver for WebApprover {
    async fn approve(&self, tool: &dyn Tool, input: &serde_json::Value) -> Decision {
        let mode = self
            .mode
            .lock()
            .map(|m| *m)
            .unwrap_or(PermissionMode::ReadOnly);
        if mode != PermissionMode::Ask {
            // Read-only and allow are ModeApprover's exact semantics,
            // wording included — two spellings of a refusal would drift.
            return ModeApprover { mode }.approve(tool, input).await;
        }
        if tool.read_only() {
            return Decision::Allow;
        }
        self.ask(tool, input, None).await
    }

    /// Past the modes that would have *passed* the call — `Allow`, the
    /// read-only shortcut — on purpose: an escalation is the interlock asking
    /// a person about *this* call. Not past `ReadOnly`, which is a refusal
    /// the surface already made: an escalation narrows and never loosens,
    /// and the PR review of this change found the first version widening
    /// past it. The reason rides on the card as its question.
    async fn escalate(&self, tool: &dyn Tool, input: &serde_json::Value, why: &str) -> Decision {
        let mode = self
            .mode
            .lock()
            .map(|m| *m)
            .unwrap_or(PermissionMode::ReadOnly);
        if mode == PermissionMode::ReadOnly && !tool.read_only() {
            return ModeApprover { mode }.approve(tool, input).await;
        }
        self.ask(tool, input, Some(why.to_string())).await
    }

    /// A rule's `allow` stands in for the card, not for the mode: a
    /// read-only surface still refuses a write.
    async fn permit(&self, tool: &dyn Tool, input: &serde_json::Value) -> Decision {
        let mode = self
            .mode
            .lock()
            .map(|m| *m)
            .unwrap_or(PermissionMode::ReadOnly);
        ModeApprover { mode }.permit(tool, input).await
    }
}

impl WebApprover {
    async fn ask(
        &self,
        tool: &dyn Tool,
        input: &serde_json::Value,
        question: Option<String>,
    ) -> Decision {
        let (qid, rx) = self.questions.open();
        let card = WireEvent::Question {
            qid,
            kind: "approval".into(),
            tool: Some(tool.name().to_string()),
            args: Some(clip_args(input)),
            // The essentials, with the whole call still beside them. A
            // reviewer who cannot read what they are approving approves it
            // anyway — which is the failure this card exists to prevent,
            // not a cosmetic complaint about JSON.
            draft: super::chat::WireDraft::of(input),
            question,
            options: Vec::new(),
            timeout_secs: self.timeout.as_secs(),
        };
        self.questions.remember(qid, card.clone());
        let _ = self.events.send(card);

        let decision = match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(Answer::Approve)) => Decision::Allow,
            Ok(Ok(Answer::Deny(reason))) => Decision::Deny(if reason.trim().is_empty() {
                "denied from the web surface".into()
            } else {
                reason
            }),
            // Decline/Text on an approval card should not happen; treat as
            // the machine's no rather than inventing a user's.
            Ok(Ok(_)) => Decision::Blocked("the approval was dismissed unanswered".into()),
            // Sender dropped: the run was cancelled out from under the card.
            Ok(Err(_)) => Decision::Blocked("the run was cancelled before anyone answered".into()),
            Err(_) => Decision::Blocked(format!(
                "nobody answered the approval request for {} within {}s — nothing is \
                 watching this run",
                tool.name(),
                self.timeout.as_secs()
            )),
        };
        self.questions.close(qid);
        let _ = self.events.send(WireEvent::QuestionDone { qid });
        decision
    }
}

/// Handles to one session's question plumbing, looked up at ask time.
pub type SessionLookup = Arc<
    dyn Fn(
            &str,
        ) -> Option<(
            Questions,
            tokio::sync::broadcast::Sender<WireEvent>,
            Option<Arc<mecha_core::questions::ParkingAsker>>,
        )> + Send
        + Sync,
>;

/// The shared `ask_user` back-end: one instance on the shared agent, routing
/// each ask to the session that owns the calling run's jail.
pub struct WebAsker {
    /// Per-session plumbing, resolved at ask time: the card channel, the
    /// event stream, and — for a delegation — where a question goes when
    /// there is nobody there to read the card.
    ///
    /// **That fallback is why there is no mode switch.** "Interactive while
    /// the page is open, autonomous when it is closed" is the right behaviour
    /// and the wrong *implementation*: a backgrounded phone keeps its stream
    /// open, so a switch keyed on being connected gets the one case that
    /// matters wrong — page attached, nobody attending — and shows a card
    /// that expires into a refusal.
    ///
    /// So the card is offered whenever anyone might see it, and both ways of
    /// going unanswered end the same way: the question is stored and the run
    /// **ends**, holding no slot and no cached prefix, until an answer
    /// resumes the conversation. Waiting indefinitely costs nothing because
    /// nothing is left waiting.
    ///
    /// Absent for an ordinary chat and for voice, where a turn has a person's
    /// attention and a decline is the honest answer — and where the voice
    /// facade sends turns without subscribing to this stream at all, so an
    /// unwatched-means-park rule would park every spoken question.
    pub lookup: SessionLookup,
}

#[async_trait]
impl Asker for WebAsker {
    async fn ask(&self, _question: &str, _options: &[String]) -> Option<String> {
        // No context means no session to route to; the tool renders `None`
        // as its measured decline. Reachable only through a caller that
        // bypassed `ask_in`, which nothing in-tree does.
        None
    }

    async fn ask_in(&self, ctx: &ToolCtx, question: &str, options: &[String]) -> Option<String> {
        let key = ctx.workspace.file_name()?.to_str()?.to_string();
        let (questions, events, park) = (self.lookup)(&key)?;

        // Nobody is subscribed, so a card would be shown to an empty room for
        // ten minutes and then resolve as a decline. Park it now instead: the
        // owner gets the question where they will actually find it, and the
        // run stops rather than carrying on having invented an answer.
        if let Some(park) = &park {
            if events.receiver_count() == 0 {
                return park.ask_in(ctx, question, options).await;
            }
        }

        let (qid, rx) = questions.open();
        let card = WireEvent::Question {
            qid,
            kind: "ask".into(),
            tool: None,
            args: None,
            // An `ask_user` card is prose already; shaping it would be
            // inventing headers for a sentence.
            draft: None,
            question: Some(question.to_string()),
            options: options.to_vec(),
            timeout_secs: ASK_TIMEOUT.as_secs(),
        };
        questions.remember(qid, card.clone());
        let _ = events.send(card);

        let answer = match tokio::time::timeout(ASK_TIMEOUT, rx).await {
            Ok(Ok(Answer::Text(text))) => Some(text),
            // An approval-shaped answer to an ask card is a UI bug, not a
            // user's words; decline rather than fabricate.
            Ok(Ok(Answer::Approve)) | Ok(Ok(Answer::Deny(_))) | Ok(Ok(Answer::Decline)) => None,
            Ok(Err(_)) | Err(_) => None,
        };
        questions.close(qid);
        let _ = events.send(WireEvent::QuestionDone { qid });
        match (answer, &park) {
            (Some(text), _) => Some(text),
            // Shown, and nobody answered — the owner walked away mid-question,
            // which is indistinguishable from never having been there. Same
            // ending, so the run does not have to guess which happened.
            (None, Some(park)) => park.ask_in(ctx, question, options).await,
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use mecha_core::tool::{Capabilities, ToolOutput};

    struct FakeTool {
        read_only: bool,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "fake_write"
        }
        fn description(&self) -> &str {
            "test double"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn read_only(&self) -> bool {
            self.read_only
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        async fn call(&self, _input: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
            Ok(ToolOutput::ok("unused"))
        }
    }

    fn approver(mode: PermissionMode, timeout: Duration) -> (WebApprover, Questions) {
        let questions = Questions::default();
        let (events, _) = tokio::sync::broadcast::channel(16);
        (
            WebApprover {
                mode: Arc::new(StdMutex::new(mode)),
                questions: questions.clone(),
                events,
                timeout,
            },
            questions,
        )
    }

    #[tokio::test]
    async fn read_only_mode_delegates_to_mode_approver_and_blocks() {
        let (approver, _) = approver(PermissionMode::ReadOnly, APPROVAL_TIMEOUT);
        let decision = approver
            .approve(&FakeTool { read_only: false }, &serde_json::json!({}))
            .await;
        assert!(
            matches!(decision, Decision::Blocked(_)),
            "a read-only run's refusal is machine policy, never a user's no"
        );
    }

    #[tokio::test]
    async fn an_unanswered_approval_expires_as_blocked_not_denied() {
        let (approver, _) = approver(PermissionMode::Ask, Duration::from_millis(30));
        let decision = approver
            .approve(&FakeTool { read_only: false }, &serde_json::json!({}))
            .await;
        match decision {
            Decision::Blocked(reason) => assert!(reason.contains("nobody answered")),
            other => panic!("timeout must be Blocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_answered_approval_allows_and_a_deny_carries_the_reason() {
        let (approver, questions) = approver(PermissionMode::Ask, Duration::from_secs(5));
        let mut rx = approver.events.subscribe();
        let q2 = questions.clone();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if let WireEvent::Question { qid, .. } = event {
                    q2.answer(qid, Answer::Deny("wrong file".into()));
                    break;
                }
            }
        });
        let decision = approver
            .approve(&FakeTool { read_only: false }, &serde_json::json!({}))
            .await;
        match decision {
            Decision::Deny(reason) => assert_eq!(reason, "wrong file"),
            other => panic!("expected the user's deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_only_tools_never_generate_an_approval_card() {
        let (approver, questions) = approver(PermissionMode::Ask, Duration::from_millis(50));
        let decision = approver
            .approve(&FakeTool { read_only: true }, &serde_json::json!({}))
            .await;
        assert!(matches!(decision, Decision::Allow));
        assert!(questions.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_pending_card_survives_for_the_reload_path_and_leaves_when_closed() {
        let (approver, questions) = approver(PermissionMode::Ask, Duration::from_millis(80));
        let handle = tokio::spawn(async move {
            approver
                .approve(&FakeTool { read_only: false }, &serde_json::json!({}))
                .await
        });
        // The card appears while the question waits…
        let mut waited = 0;
        while questions.cards().is_empty() && waited < 50 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            waited += 1;
        }
        assert_eq!(
            questions.cards().len(),
            1,
            "the transcript must see the card"
        );
        let _ = handle.await;
        // …and is gone once it resolves (here: by timeout).
        assert!(
            questions.cards().is_empty(),
            "a resolved card must not linger"
        );
    }

    #[tokio::test]
    async fn a_drained_question_resolves_as_blocked_not_a_timeout_wait() {
        let (approver, questions) = approver(PermissionMode::Ask, Duration::from_secs(30));
        let q2 = questions.clone();
        let mut rx = approver.events.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                if matches!(event, WireEvent::Question { .. }) {
                    q2.drain(); // the cancel path
                    break;
                }
            }
        });
        let started = std::time::Instant::now();
        let decision = approver
            .approve(&FakeTool { read_only: false }, &serde_json::json!({}))
            .await;
        assert!(matches!(decision, Decision::Blocked(_)));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a drained approval must not wait out its timeout"
        );
    }
}
