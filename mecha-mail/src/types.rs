//! Shared types, deliberately narrow: the `Email` struct keeps the fields
//! that describe a mail message and holds no product-level fields
//! (`ai_priority`, `card_id`, …) — those describe an application's opinions
//! about a message, not the message, and belong to whatever holds them.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    /// Deterministic local id: `gmail-{provider_id}`.
    pub id: String,
    pub provider: String,
    pub provider_id: String,
    pub thread_id: Option<String>,
    /// The RFC 5322 Message-ID, for reply threading.
    pub message_id: Option<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    pub bcc_addresses: Vec<String>,
    /// RFC 3339, UTC.
    pub date_received: String,
    /// Whether `date_received` came from a usable provider/header date.
    /// Some(false) means the display date was synthesized; None covers old
    /// records whose date certainty was not recorded. Only Some(true) ranks.
    #[serde(default)]
    pub date_known: Option<bool>,
    pub body_text: String,
    pub body_html: String,
    pub snippet: String,
    pub labels: Vec<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub list_unsubscribe: Option<String>,
    #[serde(default)]
    pub auto_submitted: Option<String>,
    #[serde(default)]
    pub precedence: Option<String>,
    #[serde(default)]
    pub list_id: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Every instance in the provider's array order; that this is received
    /// order is unverified (MAIL-RULES-DESIGN.md §5.4 counts claims rather
    /// than relying on order). None means absent from the returned
    /// collection, or not read; consult `headers_complete` before trusting
    /// absence or uniqueness. Never a provider-authentication verdict.
    #[serde(default)]
    pub authentication_results: Option<Vec<AuthenticationResults>>,
    /// Some(true) only when the headers were read whole; Some(false) for
    /// known truncation, None when completeness cannot be established.
    #[serde(default)]
    pub headers_complete: Option<bool>,
    /// Drafts are not owner participation. None means the provider did not
    /// return the information needed to distinguish a draft from sent mail.
    #[serde(default)]
    pub is_draft: Option<bool>,
}

/// One Authentication-Results field, without a trust decision. Instances must
/// stay in provider-returned order; an authserv-id is a claim, not proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationResults {
    pub value: String,
    /// None for absent or malformed ids, including EOP's `spf=...` shape.
    pub authserv_id: Option<String>,
}

impl AuthenticationResults {
    pub fn parse(value: &str) -> Self {
        Self {
            value: value.to_owned(),
            authserv_id: parse_authserv_id(value),
        }
    }
}

/// Parse only RFC 8601's leading authserv-id and optional version, never a
/// method result. In particular, EOP's leading `spf=pass` is not an id.
fn parse_authserv_id(value: &str) -> Option<String> {
    fn cfws(mut s: &str) -> Option<&str> {
        loop {
            s = s.trim_start_matches([' ', '\t', '\r', '\n']);
            if !s.starts_with('(') {
                return Some(s);
            }
            let mut depth = 0usize;
            let mut escaped = false;
            let mut end = None;
            for (i, c) in s.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            s = &s[end?..];
        }
    }

    let s = cfws(value)?;
    let (id, rest) = if let Some(quoted) = s.strip_prefix('"') {
        let mut id = String::new();
        let mut escaped = false;
        let mut end = None;
        for (i, c) in quoted.char_indices() {
            if escaped {
                id.push(c);
                escaped = false;
            } else {
                match c {
                    '\\' => escaped = true,
                    '"' => {
                        end = Some(i + 1);
                        break;
                    }
                    c if c.is_control() => return None,
                    c => id.push(c),
                }
            }
        }
        (id, &quoted[end?..])
    } else {
        // RFC 2045 token (the unquoted form of RFC 8601's `value`).
        let end = s
            .find(|c: char| {
                !c.is_ascii() || c.is_ascii_control() || c == ' ' || "()<>@,;:\\\"/[]?=".contains(c)
            })
            .unwrap_or(s.len());
        (s[..end].to_owned(), &s[end..])
    };
    if id.is_empty() {
        return None;
    }
    let mut rest = cfws(rest)?;
    if rest.starts_with(|c: char| c.is_ascii_digit()) {
        rest = cfws(rest.trim_start_matches(|c: char| c.is_ascii_digit()))?;
    }
    rest.starts_with(';').then_some(id)
}

/// All instances, including malformed values, in array order. No matching
/// field means absent, not a successful check with an empty result list.
pub(crate) fn authentication_results(
    headers: Option<&[serde_json::Value]>,
) -> Option<Vec<AuthenticationResults>> {
    let instances: Vec<_> = headers?
        .iter()
        .filter(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("authentication-results"))
        })
        .map(|h| AuthenticationResults::parse(h["value"].as_str().unwrap_or_default()))
        .collect();
    (!instances.is_empty()).then_some(instances)
}

/// Pick the first matching instance in provider array order, deliberately
/// first-wins unlike Gmail's legacy last-wins loop. Duplicates of Auto-Submitted,
/// Precedence, List-Id and Reply-To are not counted: the sender controls every copy.
pub(crate) fn first_header(headers: Option<&[serde_json::Value]>, name: &str) -> Option<String> {
    headers?
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .map(|h| h["value"].as_str().unwrap_or_default().to_owned())
}

/// Account-relative facts over a complete thread read. Unknown addresses or
/// missing sender/draft information must not become evidence of no owner reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMetadata {
    pub account_address: Option<String>,
    pub owner_participated: Option<bool>,
    /// Provider id, never the sender-written RFC Message-ID. None when no
    /// inbound message is known, direction cannot be determined, or any
    /// inbound message has an unknown date.
    pub newest_inbound_provider_id: Option<String>,
}

impl ThreadMetadata {
    /// `emails` must cover the whole thread; input order does not matter.
    /// Only the account's own address is used, not guessed send-as aliases.
    pub fn from_emails(emails: &[Email], account_address: Option<&str>) -> Self {
        let address = account_address.map(str::trim).filter(|a| !a.is_empty());
        let mut result = Self {
            account_address: address.map(str::to_owned),
            owner_participated: None,
            newest_inbound_provider_id: None,
        };
        let Some(address) = address else {
            return result;
        };
        let sent = emails
            .iter()
            .any(|e| e.from_address.eq_ignore_ascii_case(address) && e.is_draft == Some(false));
        let unknown = emails
            .iter()
            .any(|e| e.from_address.is_empty() || e.is_draft.is_none());
        result.owner_participated = if sent {
            Some(true)
        } else if unknown {
            None
        } else {
            Some(false)
        };
        let inbound = emails
            .iter()
            .filter(|e| e.is_draft == Some(false) && !e.from_address.eq_ignore_ascii_case(address));
        if !unknown && inbound.clone().all(|e| e.date_known == Some(true)) {
            result.newest_inbound_provider_id = inbound
                .max_by(|a, b| a.date_received.cmp(&b.date_received))
                .map(|e| e.provider_id.clone());
        }
        result
    }
}

/// The phrase every permanently-dead-credential error carries, so surfaces
/// that only see flattened strings (the unified fan-out's failure notes, the
/// CLI's exit-code decision, a systemd journal) can still recognise the class.
/// [`MailError::AuthRevoked`]'s display starts with it; a test pins that.
pub const AUTH_REVOKED: &str = "refresh token expired or revoked";

#[derive(Debug, Error)]
pub enum MailError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    ApiError { status: u16, message: String },

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    /// The refresh credential itself is dead — `invalid_grant` from either
    /// provider's token endpoint. **Permanent**: no retry ever recovers it,
    /// only a human re-authenticating does, which is why it is not another
    /// `AuthError` — a scheduled sweep that cannot tell the two apart retries
    /// a revoked token every two minutes for days, silently.
    #[error("refresh token expired or revoked: {0}")]
    AuthRevoked(String),
}

impl MailError {
    /// True when a forced token refresh and one retry is the right response.
    pub fn is_auth_expiry(&self) -> bool {
        matches!(self, MailError::ApiError { status: 401, .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// String-only surfaces (the fan-out's failure notes, the CLI's exit-code
    /// decision) key on the sentinel, so the variant's display and the const
    /// must never drift apart.
    #[test]
    fn a_revoked_credential_error_carries_the_sentinel() {
        let err = MailError::AuthRevoked("invalid_grant: Token has been revoked.".into());
        assert!(err.to_string().starts_with(AUTH_REVOKED), "{err}");
    }
    #[test]
    fn authserv_id_parsing_preserves_the_claim_without_authenticating_it() {
        for (value, expected) in [
            (
                "mx.google.com; dmarc=pass header.from=example.edu",
                Some("mx.google.com"),
            ),
            (
                " (first (nested)) \r\n\tmx.google.com (comment) 1; none",
                Some("mx.google.com"),
            ),
            ("\"mx.google.com\"; dmarc=pass", Some("mx.google.com")),
            ("relay.example.com; dmarc=pass", Some("relay.example.com")),
            (
                "mx.google.com.example.com; dmarc=pass",
                Some("mx.google.com.example.com"),
            ),
            (
                "spf=pass; dmarc=pass header.from=example.edu; compauth=pass reason=100",
                None,
            ),
            ("; dmarc=pass", None),
            ("", None),
            ("mx.google.com", None),
            ("mx.google.com extra; dmarc=pass", None),
            ("(unterminated mx.google.com; dmarc=pass", None),
            ("\"unterminated; dmarc=pass", None),
        ] {
            let parsed = AuthenticationResults::parse(value);
            assert_eq!(parsed.value, value);
            assert_eq!(parsed.authserv_id.as_deref(), expected, "{value}");
        }
    }

    #[test]
    fn malformed_authentication_results_are_not_silently_dropped() {
        let headers = serde_json::json!([
            {"name":"Authentication-Results", "value":null},
            {"name":"Authentication-Results", "value":"mx.google.com; none"}
        ]);
        let results = authentication_results(headers.as_array().map(Vec::as_slice)).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].authserv_id, None);
        assert_eq!(results[1].authserv_id.as_deref(), Some("mx.google.com"));
    }

    #[test]
    fn older_email_records_leave_new_facts_unknown() {
        let email = crate::google::gmail::parse_gmail_message(&serde_json::json!({"id":"m1"}));
        let mut value = serde_json::to_value(email).unwrap();
        for name in [
            "date_known",
            "auto_submitted",
            "precedence",
            "list_id",
            "reply_to",
            "authentication_results",
            "headers_complete",
            "is_draft",
        ] {
            value.as_object_mut().unwrap().remove(name);
        }
        let loaded: Email = serde_json::from_value(value).unwrap();
        assert!(loaded.authentication_results.is_none());
        assert!(loaded.headers_complete.is_none());
        assert!(loaded.is_draft.is_none());
        assert!(loaded.date_known.is_none());
        assert!(loaded.auto_submitted.is_none());
        assert!(loaded.precedence.is_none());
        assert!(loaded.list_id.is_none());
        assert!(loaded.reply_to.is_none());
    }
}
