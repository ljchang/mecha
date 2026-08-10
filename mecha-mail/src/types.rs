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
    pub body_text: String,
    pub body_html: String,
    pub snippet: String,
    pub labels: Vec<String>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub list_unsubscribe: Option<String>,
}

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
}

impl MailError {
    /// True when a forced token refresh and one retry is the right response.
    pub fn is_auth_expiry(&self) -> bool {
        matches!(self, MailError::ApiError { status: 401, .. })
    }
}
