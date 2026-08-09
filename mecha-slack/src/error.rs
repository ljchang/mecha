//! What can go wrong, classified by whether trying again could help.
//!
//! The taxonomy mirrors `mecha-core`'s `provider/retry.rs` deliberately: the
//! transient classes back off and retry, the terminal ones never do, and the
//! rule that carries it is the same — **a retry must never duplicate work.**
//! Every method in this crate is a single request whose effect is visible only
//! on success, so a retried attempt cannot have posted a message twice. The one
//! exception is called out where it lives: [`crate::files`]'s three-step upload
//! retries each step, not the sequence.

use std::time::Duration;

/// How long a `Retry-After` may ask us to wait before waiting stops being the
/// right answer. Above this a rate limit is a failure to report, not a nap to
/// take — a connector that silently sleeps for ten minutes looks identical to
/// one that has wedged, which is the distinction the whole design is trying to
/// preserve.
pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);

pub type SlackResult<T> = Result<T, SlackError>;

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    /// Slack asked us to slow down. `retry_after` is its own number, not ours.
    #[error("slack rate limited `{method}`; retry after {}s", retry_after.as_secs())]
    RateLimited {
        method: String,
        retry_after: Duration,
    },

    /// Congestion or a dropped connection. Trying again is reasonable.
    #[error("slack transport failure calling `{method}`: {source}")]
    Transport {
        method: String,
        #[source]
        source: reqwest::Error,
    },

    /// A 5xx, or one of Slack's own transient error codes.
    #[error("slack is unwell calling `{method}`: {detail}")]
    Transient { method: String, detail: String },

    /// The credential is wrong, revoked, or the app was uninstalled. Retrying
    /// this is how an app gets its tokens disabled.
    #[error("slack rejected the credential calling `{method}`: {code}")]
    Auth { method: String, code: String },

    /// Slack understood the call and refused it. `code` is Slack's own string
    /// (`not_in_channel`, `channel_not_found`, …), kept verbatim because it is
    /// the only thing worth showing a human who has to fix it.
    #[error("slack refused `{method}`: {code}")]
    Api { method: String, code: String },

    /// The response was well-formed HTTP and not what the API documents.
    #[error("slack sent something unexpected for `{method}`: {detail}")]
    Malformed { method: String, detail: String },

    /// A file download returned something that is not the file. Its own variant
    /// because the failure is silent otherwise: Slack serves a sign-in page
    /// with a 200 and an HTML content type when the token is missing or was
    /// stripped across a redirect, and the bytes would otherwise reach a model
    /// labelled as the user's attachment.
    #[error("slack served a page instead of the file `{file_id}`: {detail}")]
    NotAFile { file_id: String, detail: String },

    /// The Socket Mode connection ended. Reconnecting is the caller's job.
    #[error("socket mode disconnected: {0}")]
    Disconnected(String),
}

impl SlackError {
    /// Whether another attempt could plausibly succeed. Terminal by default:
    /// anything not explicitly transient is treated as final, so a new error
    /// class added later fails closed rather than retrying forever.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            SlackError::RateLimited { .. }
                | SlackError::Transport { .. }
                | SlackError::Transient { .. }
        )
    }

    /// The delay this error asks for, if it asks for one.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            SlackError::RateLimited { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }
}

/// Slack error codes that mean "the credential is the problem". Terminal, and
/// separated from the rest because they are the ones worth surfacing to a human
/// immediately — every subsequent call will fail the same way, so a connector
/// seeing one should stop rather than grind.
const AUTH_CODES: &[&str] = &[
    "invalid_auth",
    "not_authed",
    "account_inactive",
    "token_revoked",
    "token_expired",
    "no_permission",
    "missing_scope",
];

/// Slack error codes that arrive with `ok: false` but mean "later". The list is
/// short on purpose: an unknown code is terminal, because retrying a call Slack
/// has already refused on its merits is how a loop becomes a rate limit.
const TRANSIENT_CODES: &[&str] = &[
    "service_unavailable",
    "internal_error",
    "fatal_error",
    "request_timeout",
];

/// Turn an `ok: false` payload into the right variant.
pub(crate) fn classify_api_error(method: &str, code: &str) -> SlackError {
    if code == "ratelimited" || code == "rate_limited" {
        // An `ok: false` rate limit carries no header, so we pick the floor
        // Slack's own guidance implies rather than inventing something larger.
        return SlackError::RateLimited {
            method: method.to_string(),
            retry_after: Duration::from_secs(1),
        };
    }
    if AUTH_CODES.contains(&code) {
        return SlackError::Auth {
            method: method.to_string(),
            code: code.to_string(),
        };
    }
    if TRANSIENT_CODES.contains(&code) {
        return SlackError::Transient {
            method: method.to_string(),
            detail: code.to_string(),
        };
    }
    SlackError::Api {
        method: method.to_string(),
        code: code.to_string(),
    }
}

/// Parse a `Retry-After` header, clamping to [`RETRY_AFTER_CAP`]. Slack sends
/// whole seconds; anything unparseable is treated as one second rather than as
/// an error, because the 429 itself is the signal and the header is advice.
pub(crate) fn parse_retry_after(raw: Option<&str>) -> Duration {
    let secs = raw.and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(1);
    Duration::from_secs(secs).min(RETRY_AFTER_CAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_error_code_is_terminal() {
        // The direction that matters: a code nobody has seen before must not
        // be retried, or one refusal becomes a rate limit.
        let e = classify_api_error("chat.postMessage", "some_future_code");
        assert!(matches!(e, SlackError::Api { .. }));
        assert!(!e.is_transient());
    }

    #[test]
    fn credential_failures_are_never_transient() {
        for code in AUTH_CODES {
            let e = classify_api_error("chat.postMessage", code);
            assert!(matches!(e, SlackError::Auth { .. }), "{code}");
            assert!(!e.is_transient(), "{code} must never be retried");
        }
    }

    #[test]
    fn slacks_own_transient_codes_retry() {
        for code in TRANSIENT_CODES {
            assert!(
                classify_api_error("chat.update", code).is_transient(),
                "{code}"
            );
        }
    }

    #[test]
    fn a_rate_limit_without_a_header_still_asks_for_a_wait() {
        let e = classify_api_error("chat.postMessage", "ratelimited");
        assert_eq!(e.retry_after(), Some(Duration::from_secs(1)));
        assert!(e.is_transient());
    }

    #[test]
    fn retry_after_is_honoured_but_capped() {
        assert_eq!(parse_retry_after(Some("30")), Duration::from_secs(30));
        assert_eq!(
            parse_retry_after(Some("3600")),
            RETRY_AFTER_CAP,
            "an hour-long nap is a failure to report, not a wait to take"
        );
        assert_eq!(parse_retry_after(None), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("garbage")), Duration::from_secs(1));
    }
}
