//! Optional operational readiness observations; no domain or provider knowledge.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionReadiness {
    Ready,
    Incomplete,
    Unavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionArtifactIssue {
    Missing,
    Malformed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionFeedback {
    pub required_read_missing: bool,
    pub artifact: Option<CompletionArtifactIssue>,
    pub do_not_repeat_read: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionObservation {
    pub readiness: CompletionReadiness,
    pub feedback: Option<CompletionFeedback>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionReport {
    pub first: Option<CompletionObservation>,
    pub latest: Option<CompletionObservation>,
    /// The notice was appended; this does not prove a provider received it.
    pub feedback_delivered: bool,
    /// Rejected checks never replace the first/latest accepted observations.
    pub timed_out: u32,
    pub interrupted: u32,
}
/// An opt-in check of operations, independent of providers and tool domains.
///
/// `observe` must be cancellation-safe and return data without publishing a
/// report or mutating execution authority. Its future can be dropped at any
/// time. Convert inspection failures to `Unavailable`, without dynamic prose.
/// The loop calls `record` synchronously for its accepted report snapshots;
/// implementations should only copy the report to a task-local cell and return.
/// Reusing an observer across runs makes that reporting cell the host's concern.
#[async_trait::async_trait]
pub trait CompletionObserver: Send + Sync {
    async fn observe(&self) -> CompletionObservation;
    fn record(&self, _report: &CompletionReport) {}
}

// Every variant is fixed harness language. Exact recognition deliberately rejects
// a genuine owner message that merely starts with a familiar harness prefix.
const NOTICES: [&str; 7] = [
    "(From the harness: required operations are incomplete. Perform the required read before finishing. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. The required artifact is missing. Save it in the requested format. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. The required artifact is malformed. Save it in the requested format. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. Perform the required read before finishing. The required artifact is missing. Save it in the requested format. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. Perform the required read before finishing. The required artifact is malformed. Save it in the requested format. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. The required artifact is missing. Save it in the requested format. Do not repeat the required read. Continue within the existing permissions and budget.)",
    "(From the harness: required operations are incomplete. The required artifact is malformed. Save it in the requested format. Do not repeat the required read. Continue within the existing permissions and budget.)",
];

impl CompletionFeedback {
    fn notice(&self) -> Option<&'static str> {
        use CompletionArtifactIssue::{Malformed, Missing};
        match (
            self.required_read_missing,
            self.artifact,
            self.do_not_repeat_read,
        ) {
            (true, None, false) => Some(NOTICES[0]),
            (false, Some(Missing), false) => Some(NOTICES[1]),
            (false, Some(Malformed), false) => Some(NOTICES[2]),
            (true, Some(Missing), false) => Some(NOTICES[3]),
            (true, Some(Malformed), false) => Some(NOTICES[4]),
            (false, Some(Missing), true) => Some(NOTICES[5]),
            (false, Some(Malformed), true) => Some(NOTICES[6]),
            _ => None,
        }
    }
}

pub(super) fn is_notice(text: &str) -> bool {
    NOTICES.contains(&text)
}

impl CompletionReport {
    /// One bounded check. A dropped/late future cannot publish an observation:
    /// only this synchronous acceptance point calls the host's report callback.
    pub(super) async fn observe(
        &mut self,
        cx: &super::RunContext,
        can_continue: bool,
        messages: &mut Vec<crate::message::Message>,
    ) -> bool {
        let Some(observer) = cx.completion_observer.as_ref() else {
            return false;
        };
        let now = tokio::time::Instant::now();
        let deadline = (now + std::time::Duration::from_secs(2)).min(
            cx.completion_deadline
                .unwrap_or(now + std::time::Duration::from_secs(2)),
        );
        if cx.cancelled() {
            self.interrupted += 1;
            observer.record(self);
            return false;
        }
        if now >= deadline {
            self.timed_out += 1;
            observer.record(self);
            return false;
        }
        let cancelled = async {
            match &cx.cancel {
                Some(token) => token.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        let observed = tokio::select! {
            biased;
            _ = cancelled => None,
            result = tokio::time::timeout_at(deadline, observer.observe()) => result.ok(),
        };
        // timeout_at cannot preempt a future whose poll blocks. Check after
        // polling as well, before accepting even an already-ready result.
        if cx.cancelled() {
            self.interrupted += 1;
            observer.record(self);
            return false;
        }
        if tokio::time::Instant::now() >= deadline || observed.is_none() {
            self.timed_out += 1;
            observer.record(self);
            return false;
        }
        let mut observed = observed.expect("checked above");
        let notice = observed
            .feedback
            .as_ref()
            .and_then(CompletionFeedback::notice);
        if observed.feedback.is_some()
            && (observed.readiness != CompletionReadiness::Incomplete || notice.is_none())
        {
            observed = CompletionObservation {
                readiness: CompletionReadiness::Unavailable,
                feedback: None,
            };
        }
        if self.first.is_none() {
            self.first = Some(observed.clone());
        }
        self.latest = Some(observed.clone());
        observer.record(self);
        // Recheck after the host callback too: it must be synchronous and cheap,
        // but cannot authorize a late append if it crosses the host deadline.
        if !can_continue
            || self.feedback_delivered
            || cx.cancelled()
            || tokio::time::Instant::now() >= deadline
            || observed.readiness != CompletionReadiness::Incomplete
        {
            return false;
        }
        let Some(notice) = observed
            .feedback
            .as_ref()
            .and_then(CompletionFeedback::notice)
        else {
            return false;
        };
        let mut message = crate::message::Message::user(notice);
        message.harness = true;
        messages.push(message);
        self.feedback_delivered = true;
        observer.record(self);
        true
    }
}
