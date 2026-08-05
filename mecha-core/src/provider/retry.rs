//! Provider failure classification, and the retry policy over it.
//!
//! Any non-2xx used to bail straight out of both providers, which meant a
//! single transient 429, a 529 overload, or a stale pooled connection killed
//! the run — and in `batch` or `eval`, killed it in the middle of a fan-out
//! that had already spent real time. Observed live, reproducibly: llama-server
//! closes idle keep-alive connections, reqwest reuses one, and the write dies
//! with "connection closed before message completed" on a request that would
//! have succeeded one retry later.
//!
//! The load-bearing invariant: **a retry must never duplicate work.** Retrying
//! the HTTP request is safe exactly when nothing of the attempt has been acted
//! on — no tool has run, no delta has reached the front-end. So retries live
//! at the request level, before the response body is consumed; once a
//! streaming body is being read, a failure is not retried at all. Mid-stream
//! errors therefore carry no [`ProviderError`] in their chain, which is also
//! what tells the failover wrapper it must not re-issue them.

use std::time::Duration;

/// Why a provider call failed, coarsely enough to decide policy per class.
///
/// Classification is by status *and* by message text, because the text is
/// sometimes the only signal — no backend gives context overflow a usable
/// code, which is the lesson `is_context_overflow` already encodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// HTTP 429 — `Retry-After` is honoured when it is sane; a provider can
    /// name a wait long enough that the process is simply asleep.
    RateLimit { retry_after: Option<Duration> },
    /// The provider says it is drowning (529, or a 503 that says so).
    Overloaded,
    /// Any other 5xx.
    ServerError,
    /// 401/403 — terminal; the same key fails the same way every time.
    Auth,
    /// Credit exhausted — terminal, and retrying it spends nothing but time.
    Billing,
    /// The prompt does not fit. Never retried here: the compaction path in
    /// the loop owns this one, and a retry with the same payload cannot fit
    /// any better.
    ContextOverflow,
    /// Any other 4xx — the same payload fails the same way.
    Invalid(String),
    /// Connect failures, timeouts, aborted writes. The pooled-connection race
    /// lands here.
    Transport,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::RateLimit {
                retry_after: Some(d),
            } => {
                write!(f, "rate limited (retry after {}s)", d.as_secs())
            }
            ProviderError::RateLimit { retry_after: None } => write!(f, "rate limited"),
            ProviderError::Overloaded => write!(f, "provider overloaded"),
            ProviderError::ServerError => write!(f, "provider server error"),
            ProviderError::Auth => write!(f, "authentication failed"),
            ProviderError::Billing => write!(f, "billing/credit failure"),
            ProviderError::ContextOverflow => write!(f, "prompt exceeds the context window"),
            ProviderError::Invalid(detail) => write!(f, "invalid request: {detail}"),
            ProviderError::Transport => write!(f, "transport failure"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    /// Whether another *identical request* could plausibly succeed. This is
    /// also what the failover wrapper keys on: an error without this property
    /// fails the same way on every provider (`Invalid`, `ContextOverflow`) or
    /// must not be retried at all.
    pub fn transient(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimit { .. }
                | ProviderError::Overloaded
                | ProviderError::ServerError
                | ProviderError::Transport
        )
    }
}

/// Does this error text say the prompt did not fit? Shared with the loop's
/// `is_context_overflow`, because no backend gives it a usable code:
/// llama-server says `exceed_context_size_error`, vLLM says "maximum context
/// length", Anthropic says "prompt is too long".
pub fn overflow_text(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("exceed_context_size")
        || t.contains("context_length_exceeded")
        || t.contains("context length")
        || t.contains("context size")
        || t.contains("prompt is too long")
        || t.contains("too many tokens")
        || t.contains("maximum context")
}

/// Classify a non-2xx response.
pub fn classify_http(status: u16, body: &str, retry_after: Option<Duration>) -> ProviderError {
    let lower = body.to_ascii_lowercase();
    match status {
        401 | 403 => ProviderError::Auth,
        402 => ProviderError::Billing,
        429 => ProviderError::RateLimit { retry_after },
        529 => ProviderError::Overloaded,
        503 if lower.contains("overload") => ProviderError::Overloaded,
        // Before the 5xx arm: llama-server reports overflow as a *500* saying
        // "Context size has been exceeded" (observed live). Classified as
        // ServerError it would be retried with the same payload three times
        // and then never reach the loop's compact-and-retry recovery.
        _ if overflow_text(body) => ProviderError::ContextOverflow,
        s if s >= 500 => ProviderError::ServerError,
        _ if lower.contains("credit balance") || lower.contains("billing") => {
            ProviderError::Billing
        }
        _ => ProviderError::Invalid(body.chars().take(200).collect()),
    }
}

/// Per-request retry policy. Lives on the provider, built from its config.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Retries after the first attempt. 0 disables retrying entirely.
    pub max_retries: u32,
    /// A `Retry-After` above this is surfaced as a failure instead of slept
    /// through — control has to return to a layer that could fall back.
    pub retry_after_cap: Duration,
    /// First backoff delay; doubles per attempt, capped at [`Self::MAX_DELAY`].
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_retries: 3,
            retry_after_cap: Duration::from_secs(60),
            base_delay: Duration::from_millis(2_500),
        }
    }
}

impl RetryPolicy {
    pub const MAX_DELAY: Duration = Duration::from_secs(30);

    pub fn from_config(cfg: &crate::config::ProviderConfig) -> Self {
        let d = RetryPolicy::default();
        RetryPolicy {
            max_retries: cfg.max_retries.unwrap_or(d.max_retries),
            retry_after_cap: cfg
                .retry_after_cap_secs
                .map(Duration::from_secs)
                .unwrap_or(d.retry_after_cap),
            base_delay: d.base_delay,
        }
    }

    /// How long to wait before retry number `attempt` (1-based), or `None`
    /// for "do not retry" — exhausted, terminal class, or a `Retry-After`
    /// past the cap.
    pub fn delay_for(&self, error: &ProviderError, attempt: u32) -> Option<Duration> {
        if attempt > self.max_retries || !error.transient() {
            return None;
        }
        match error {
            ProviderError::RateLimit {
                retry_after: Some(after),
            } => {
                // Above the cap is a failure, not a nap: sleeping an hour on
                // a header's say-so takes the process hostage.
                (*after <= self.retry_after_cap).then_some(*after)
            }
            _ => {
                let exp = self
                    .base_delay
                    .saturating_mul(1u32 << (attempt - 1).min(16));
                Some(exp.min(Self::MAX_DELAY))
            }
        }
    }
}

/// What a request died of, once the policy gave up on it.
///
/// The caller formats the user-facing message — each provider keeps its
/// existing error shape, which the loop's overflow detection greps — and
/// attaches [`RequestFailure::class`] underneath it so policy layers
/// (failover, the loop) can match on the class instead of the prose.
#[derive(Debug)]
pub struct RequestFailure {
    pub class: ProviderError,
    /// HTTP status, when the failure got that far. `None` is transport.
    pub status: Option<u16>,
    /// The provider's error body, or the transport error text.
    pub detail: String,
}

/// Send a request until it succeeds, the policy gives up, or the class is
/// terminal. Retries cover the send and the status line only — the response
/// body is never consumed here, so nothing of a retried attempt can have
/// been shown or acted on, which is the invariant that makes the retry safe.
pub async fn send_with_retry(
    make_request: impl Fn() -> reqwest::RequestBuilder,
    policy: &RetryPolicy,
) -> Result<reqwest::Response, RequestFailure> {
    let mut attempt = 0u32;
    loop {
        let failure = match make_request().send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let retry_after = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                let body = resp.text().await.unwrap_or_default();
                RequestFailure {
                    class: classify_http(status, &body, retry_after),
                    status: Some(status),
                    detail: body,
                }
            }
            Err(e) => RequestFailure {
                class: ProviderError::Transport,
                status: None,
                detail: e.to_string(),
            },
        };

        attempt += 1;
        match policy.delay_for(&failure.class, attempt) {
            Some(delay) => {
                tracing::warn!(
                    error = %failure.class,
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    "provider request failed; retrying"
                );
                tokio::time::sleep(delay).await;
            }
            None => return Err(failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_class_gets_its_policy() {
        let p = RetryPolicy {
            base_delay: Duration::from_millis(10),
            ..Default::default()
        };

        // Transient classes back off, doubling, capped.
        for err in [
            ProviderError::Overloaded,
            ProviderError::ServerError,
            ProviderError::Transport,
        ] {
            assert_eq!(p.delay_for(&err, 1), Some(Duration::from_millis(10)));
            assert_eq!(p.delay_for(&err, 2), Some(Duration::from_millis(20)));
            assert_eq!(p.delay_for(&err, 4), None, "exhausted past max_retries");
        }

        // Terminal classes never retry: the same payload fails the same way,
        // and a retried 401 is a lockout risk, not a recovery.
        for err in [
            ProviderError::Auth,
            ProviderError::Billing,
            ProviderError::Invalid("x".into()),
            ProviderError::ContextOverflow,
        ] {
            assert_eq!(p.delay_for(&err, 1), None);
        }
    }

    #[test]
    fn retry_after_is_honoured_when_sane_and_a_failure_when_hostile() {
        let p = RetryPolicy::default();
        let soon = ProviderError::RateLimit {
            retry_after: Some(Duration::from_secs(3)),
        };
        assert_eq!(p.delay_for(&soon, 1), Some(Duration::from_secs(3)));

        // An hour-long Retry-After would put the process to sleep past every
        // budget; control must return to a layer that can decide.
        let hostile = ProviderError::RateLimit {
            retry_after: Some(Duration::from_secs(3_600)),
        };
        assert_eq!(p.delay_for(&hostile, 1), None);

        let unstated = ProviderError::RateLimit { retry_after: None };
        assert_eq!(p.delay_for(&unstated, 1), Some(p.base_delay));
    }

    #[test]
    fn zero_max_retries_disables_retrying() {
        let p = RetryPolicy {
            max_retries: 0,
            ..Default::default()
        };
        assert_eq!(p.delay_for(&ProviderError::Transport, 1), None);
    }

    #[test]
    fn the_backoff_never_exceeds_the_ceiling() {
        let p = RetryPolicy {
            max_retries: 40,
            ..Default::default()
        };
        assert_eq!(
            p.delay_for(&ProviderError::Transport, 39),
            Some(RetryPolicy::MAX_DELAY)
        );
    }

    #[test]
    fn classification_reads_status_and_text() {
        use ProviderError::*;
        assert_eq!(classify_http(401, "", None), Auth);
        assert_eq!(classify_http(403, "", None), Auth);
        assert_eq!(
            classify_http(429, "", Some(Duration::from_secs(2))),
            RateLimit {
                retry_after: Some(Duration::from_secs(2))
            }
        );
        assert_eq!(classify_http(529, "", None), Overloaded);
        assert_eq!(
            classify_http(503, "The server is overloaded", None),
            Overloaded
        );
        assert_eq!(classify_http(500, "", None), ServerError);
        assert_eq!(classify_http(503, "", None), ServerError);

        // The text is sometimes the only signal.
        assert_eq!(
            classify_http(400, r#"{"type":"exceed_context_size_error"}"#, None),
            ContextOverflow
        );
        // ...and it outranks the status class: llama-server reports overflow
        // as a 500 (observed live). As ServerError it would be retried with
        // the same payload and never reach compaction recovery.
        assert_eq!(
            classify_http(500, "Context size has been exceeded.", None),
            ContextOverflow
        );
        assert_eq!(
            classify_http(400, "Your credit balance is too low", None),
            Billing
        );
        assert!(matches!(classify_http(400, "bad json", None), Invalid(_)));
    }
}
