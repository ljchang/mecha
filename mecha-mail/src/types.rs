//! Shared types, deliberately narrow: the `Email` struct keeps the fields
//! that describe a mail message and holds no product-level fields
//! (`ai_priority`, `card_id`, …) — those describe an application's opinions
//! about a message, not the message, and belong to whatever holds them.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Facts returned by the provider, never inferred from a sender header alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionEvidence {
    Sent,
    Draft,
    /// Gmail returned a complete label collection without SENT or DRAFT.
    NotSent,
    /// Graph returned isDraft=false; owner addresses must still be checked.
    NotDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailDirection {
    Inbound,
    Sent,
    Draft,
}

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
    /// Gmail preserves its raw Subject; Graph supplies already decoded text.
    #[serde(default)]
    pub subject_is_raw: bool,
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
    /// Provider arrival timestamp only. Header Date remains display data.
    #[serde(default)]
    pub provider_arrival_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Unknown when the provider omitted attachment metadata or MIME structure.
    #[serde(default)]
    pub attachment_presence: Option<bool>,
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
    /// A complete dedicated subject or a complete source header collection.
    #[serde(default)]
    pub subject_complete: Option<bool>,
    /// All To/Cc/Bcc collections were returned and parsed completely.
    #[serde(default)]
    pub recipients_complete: Option<bool>,
    /// Drafts are not owner participation. None means the provider did not
    /// return the information needed to distinguish a draft from sent mail.
    #[serde(default)]
    pub is_draft: Option<bool>,
    #[serde(default)]
    pub direction_evidence: Option<DirectionEvidence>,
    /// Raw header instances for deterministic header rules; no trust decision.
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, Vec<String>>,
}

impl Email {
    /// An owner address (including aliases) cannot be an inbound sender.
    /// Unknown provider direction or sender data never becomes incoming mail.
    pub fn direction(&self, owners: &[String]) -> Option<MailDirection> {
        match self.direction_evidence? {
            DirectionEvidence::Draft => Some(MailDirection::Draft),
            DirectionEvidence::Sent => Some(MailDirection::Sent),
            DirectionEvidence::NotSent | DirectionEvidence::NotDraft => {
                if self.from_address.is_empty() || owners.is_empty() {
                    return None;
                }
                Some(
                    if owners
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(&self.from_address))
                    {
                        MailDirection::Sent
                    } else {
                        MailDirection::Inbound
                    },
                )
            }
        }
    }

    /// Variable-width fractional seconds and timezone offsets must be compared
    /// as instants, never lexicographically or at whole-second precision.
    pub(crate) fn received_instant(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::parse_from_rfc3339(&self.date_received)
            .ok()
            .map(|at| at.with_timezone(&chrono::Utc))
    }
}

pub(crate) fn header_values(
    headers: Option<&[serde_json::Value]>,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut values: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for header in headers.into_iter().flatten() {
        if let (Some(name), Some(value)) = (header["name"].as_str(), header["value"].as_str()) {
            values
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(value.into());
        }
    }
    values
}

/// One Authentication-Results field, without a trust decision. Instances must
/// stay in provider-returned order; an authserv-id is a claim, not proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationResults {
    #[serde(default)]
    pub value: String,
    /// None for absent or malformed ids, including EOP's `spf=...` shape.
    #[serde(default)]
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
    /// inbound message has an unknown date or an empty provider id.
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
        let owners = [address.to_owned()];
        let sent = emails
            .iter()
            .any(|e| e.direction(&owners) == Some(MailDirection::Sent));
        let unknown = emails.iter().any(|e| e.direction(&owners).is_none());
        result.owner_participated = if sent {
            Some(true)
        } else if unknown {
            None
        } else {
            Some(false)
        };
        let inbound = emails
            .iter()
            .filter(|e| e.direction(&owners) == Some(MailDirection::Inbound));
        if !unknown
            && inbound.clone().all(|e| {
                e.date_known == Some(true)
                    && !e.provider_id.is_empty()
                    && chrono::DateTime::parse_from_rfc3339(&e.date_received).is_ok()
            })
        {
            result.newest_inbound_provider_id = inbound
                .max_by_key(|e| chrono::DateTime::parse_from_rfc3339(&e.date_received).ok())
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

    /// A Gmail 403 whose structured reasons are all documented quota failures.
    /// Other providers' 403 responses never acquire this exemption.
    #[error("Gmail quota exceeded (403, {reasons:?}): {message}")]
    GmailQuota {
        reasons: Vec<String>,
        message: String,
    },

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
    pub(crate) fn gmail_api_error(status: u16, message: String) -> Self {
        // https://developers.google.com/workspace/gmail/api/guides/handle-errors#403_errors
        // Neither status alone nor a substring in human-readable prose proves
        // a quota failure. Mixed/unknown/malformed reason sets remain actions.
        if status == 403 {
            let reasons = serde_json::from_str::<serde_json::Value>(&message)
                .ok()
                .and_then(|body| {
                    body["error"]["errors"]
                        .as_array()?
                        .iter()
                        .map(|error| error["reason"].as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                });
            if let Some(reasons) = reasons.filter(|reasons| {
                !reasons.is_empty()
                    && reasons.iter().all(|reason| {
                        matches!(
                            reason.as_str(),
                            "rateLimitExceeded" | "userRateLimitExceeded" | "dailyLimitExceeded"
                        )
                    })
            }) {
                return Self::GmailQuota { reasons, message };
            }
        }
        Self::ApiError { status, message }
    }

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
    fn an_empty_inbound_provider_id_makes_newest_unknown_for_both_providers() {
        use crate::google::gmail::{parse_gmail_message, HeaderRead};
        use crate::microsoft::graph_mail::parse_outlook_message;
        use serde_json::json;

        let fixture = json!({
            "id":"known", "internalDate":"1704067200000", "labelIds":["INBOX"],
            "payload":{"headers":[{"name":"From", "value":"sender@example.edu"}]},
            "isDraft":false, "receivedDateTime":"2024-01-01T00:00:00Z",
            "from":{"emailAddress":{"address":"sender@example.edu"}}
        });
        for id in [None, Some(json!(""))] {
            let mut missing = fixture.clone();
            missing.as_object_mut().unwrap().remove("id");
            if let Some(id) = id {
                missing["id"] = id;
            }
            for (known, missing) in [
                (
                    parse_gmail_message(HeaderRead::Full, &fixture),
                    parse_gmail_message(HeaderRead::Full, &missing),
                ),
                (
                    parse_outlook_message(&fixture),
                    parse_outlook_message(&missing),
                ),
            ] {
                assert!(missing.provider_id.is_empty());
                assert_eq!(missing.date_known, Some(true));
                // Even an apparently older unidentified inbound blocks selection.
                for date in ["2023-12-01T00:00:00Z", "2024-02-01T00:00:00Z"] {
                    let mut missing = missing.clone();
                    missing.date_received = date.into();
                    let metadata = ThreadMetadata::from_emails(
                        &[known.clone(), missing.clone()],
                        Some("owner@example.com"),
                    );
                    assert_eq!(metadata.newest_inbound_provider_id, None);
                    assert_eq!(metadata.owner_participated, Some(false));
                    missing.from_address = "owner@example.com".into();
                    let metadata = ThreadMetadata::from_emails(
                        &[known.clone(), missing],
                        Some("owner@example.com"),
                    );
                    assert_eq!(
                        metadata.newest_inbound_provider_id.as_deref(),
                        Some("known")
                    );
                }
            }
        }
    }

    #[test]
    fn authentication_results_missing_fields_load_with_safe_defaults() {
        let empty: AuthenticationResults = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.value.is_empty());
        assert_eq!(empty.authserv_id, None);
        let old: AuthenticationResults = serde_json::from_value(serde_json::json!({
            "value":"relay.example.edu; dmarc=pass header.from=example.edu"
        }))
        .unwrap();
        assert_eq!(
            old.value,
            "relay.example.edu; dmarc=pass header.from=example.edu"
        );
        assert_eq!(old.authserv_id, None);
    }

    #[test]
    fn older_email_records_leave_new_facts_unknown() {
        let email = crate::google::gmail::parse_gmail_message(
            crate::google::gmail::HeaderRead::Partial,
            &serde_json::json!({"id":"m1"}),
        );
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
