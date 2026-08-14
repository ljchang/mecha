//! Google OAuth for a desktop client: PKCE, a loopback listener for the
//! redirect, code exchange, and refresh.
//!
//! Scoped to Gmail: the Microsoft/Outlook config and its AADSTS error
//! translation live under [`crate::microsoft`] instead. Two deliberate
//! choices: the loopback port is a parameter rather than a hardcoded
//! constant, so another desktop OAuth client on the same machine can hold
//! its own port and both flows can run; and the scope list drops
//! `gmail.modify` — nothing here modifies messages, and least-privilege
//! beats saving a future consent click.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::MailError;

/// The loopback port mecha-google listens on for the OAuth redirect. Google
/// Desktop-type clients accept any loopback port; this one just needs to be
/// unlikely to collide with another desktop OAuth client's listener on the
/// same machine.
pub const DEFAULT_REDIRECT_PORT: u16 = 8924;

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
}

/// Generate a PKCE code_verifier and code_challenge pair.
pub fn generate_pkce() -> PkceChallenge {
    // `rand::rng()` is the thread-local CSPRNG — rand 0.9 renamed it from
    // `thread_rng` and dropped `gen` for `random`. Filled in one call rather
    // than byte at a time, which is what the old `(0..32).map(...)` was doing
    // the long way round.
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let digest = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);

    PkceChallenge {
        code_verifier,
        code_challenge,
    }
}

/// The Google OAuth configuration for this client. Scopes cover Gmail
/// read/send and Calendar — one consent covers both surfaces.
pub fn google_oauth_config(client_id: String, client_secret: String, port: u16) -> OAuthConfig {
    OAuthConfig {
        client_id,
        client_secret,
        redirect_uri: format!("http://localhost:{port}/callback"),
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        scopes: vec![
            "https://www.googleapis.com/auth/gmail.readonly".to_string(),
            "https://www.googleapis.com/auth/gmail.send".to_string(),
            "https://www.googleapis.com/auth/calendar".to_string(),
            "https://www.googleapis.com/auth/calendar.events".to_string(),
        ],
    }
}

/// Build the full authorization URL for the user to visit.
///
/// `access_type=offline&prompt=consent` — Google needs both to reliably
/// return a `refresh_token` on every sign-in, not only the first.
pub fn build_auth_url(config: &OAuthConfig, pkce: &PkceChallenge, state: &str) -> String {
    let scopes = config.scopes.join(" ");
    format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        config.auth_url,
        urlencoding(&config.client_id),
        urlencoding(&config.redirect_uri),
        urlencoding(&scopes),
        urlencoding(&pkce.code_challenge),
        urlencoding(state),
    )
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    code_verifier: &str,
    client: &reqwest::Client,
) -> Result<OAuthTokens, MailError> {
    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &config.redirect_uri),
        ("client_id", &config.client_id),
        ("code_verifier", code_verifier),
    ];
    append_optional_secret(&mut params, &config.client_secret);

    let resp = client.post(&config.token_url).form(&params).send().await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(MailError::ApiError {
            status,
            message: extract_token_error_description(&body),
        });
    }

    Ok(parse_token_response(resp.json().await?, None))
}

/// Refresh an expired access token using a refresh token.
pub async fn refresh_token(
    config: &OAuthConfig,
    refresh_tok: &str,
    client: &reqwest::Client,
) -> Result<OAuthTokens, MailError> {
    let mut params: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_tok),
        ("client_id", &config.client_id),
    ];
    append_optional_secret(&mut params, &config.client_secret);

    let resp = client.post(&config.token_url).form(&params).send().await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_refresh_failure(status, &body));
    }

    // Google usually omits the refresh token on refresh; carry the old one
    // forward so the caller never loses it.
    Ok(parse_token_response(resp.json().await?, Some(refresh_tok)))
}

pub fn parse_token_response(json: serde_json::Value, prior_refresh: Option<&str>) -> OAuthTokens {
    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    OAuthTokens {
        access_token: json["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| prior_refresh.map(|s| s.to_string())),
        expires_at: Some(chrono::Utc::now().timestamp() + expires_in),
        token_type: json["token_type"].as_str().unwrap_or("Bearer").to_string(),
    }
}

/// Append `client_secret` only when one is configured. Google's Desktop-app
/// client type expects its pseudo-secret; a public client omits it entirely.
fn append_optional_secret<'a>(params: &mut Vec<(&'a str, &'a str)>, client_secret: &'a str) {
    if !client_secret.is_empty() {
        params.push(("client_secret", client_secret));
    }
}

/// Sort a failed refresh into permanent versus everything else.
///
/// `invalid_grant` is the OAuth server saying the refresh token itself is dead
/// — revoked, expired, or the consent withdrawn. That never recovers on retry,
/// so it becomes [`MailError::AuthRevoked`] rather than a generic `ApiError`
/// indistinguishable from a transient 5xx; the token manager keys off the
/// variant to leave a marker and name the re-auth command.
pub(crate) fn classify_refresh_failure(status: u16, body: &str) -> MailError {
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|c| c.as_str()).map(str::to_string));
    if code.as_deref() == Some("invalid_grant") {
        return MailError::AuthRevoked(extract_token_error_description(body));
    }
    MailError::ApiError {
        status,
        message: extract_token_error_description(body),
    }
}

/// Return the OAuth error as "error_code: description" when both fields are
/// present, so callers can match on the code (e.g. `invalid_grant`)
/// regardless of wording. Falls back to whatever is available.
fn extract_token_error_description(body: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_string();
    };
    let code = v.get("error").and_then(|c| c.as_str());
    let desc = v.get("error_description").and_then(|d| d.as_str());
    match (code, desc) {
        (Some(c), Some(d)) => format!("{c}: {d}"),
        (Some(c), None) => c.to_string(),
        (None, Some(d)) => d.to_string(),
        (None, None) => body.to_string(),
    }
}

/// Start a temporary localhost HTTP server to capture the OAuth redirect.
/// Returns the authorization code and state from the callback query params.
/// Times out after 120 seconds — a human is reading a consent screen, and a
/// 30s window regularly loses the race to a careful reader.
pub async fn wait_for_oauth_redirect(port: u16) -> Result<(String, String), MailError> {
    let addr = format!("127.0.0.1:{port}");
    let server = tiny_http::Server::http(&addr).map_err(|e| MailError::AuthError(e.to_string()))?;

    let request = server
        .recv_timeout(std::time::Duration::from_secs(120))
        .map_err(|e| MailError::AuthError(format!("Failed to receive redirect: {e}")))?
        .ok_or_else(|| {
            MailError::AuthError(
                "OAuth timed out — no redirect within 120 seconds. Run `auth` again.".to_string(),
            )
        })?;

    let url = request.url().to_string();
    let query = url
        .split('?')
        .nth(1)
        .ok_or_else(|| MailError::AuthError("No query parameters in redirect".to_string()))?;

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;

    for param in query.split('&') {
        let mut parts = param.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();
        match key {
            "code" => code = Some(value.to_string()),
            "state" => state = Some(value.to_string()),
            "error" => error = Some(value.to_string()),
            "error_description" => error_description = Some(value.replace('+', " ")),
            _ => {}
        }
    }

    let html = if error.is_some() {
        "<html><body><h1>Authentication failed</h1><p>You can close this window and run `mecha-google auth` again.</p></body></html>"
    } else {
        "<html><body><h1>Authentication successful</h1><p>You can close this window and return to the terminal.</p></body></html>"
    };
    let response = tiny_http::Response::from_string(html).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap(),
    );
    let _ = request.respond(response);

    if let Some(err) = error {
        let msg = error_description.unwrap_or(err);
        return Err(MailError::AuthError(format!("OAuth error: {msg}")));
    }

    let code = code.ok_or_else(|| MailError::AuthError("No code in redirect".to_string()))?;
    Ok((code, state.unwrap_or_default()))
}

/// The complete interactive sign-in: PKCE, browser hand-off, loopback
/// redirect, code exchange, and a profile read that both records whose
/// mailbox this is and proves the scopes work before anything is saved.
/// Lives in the library so `mecha-google auth` and the unified
/// `mecha-mail auth` are the same flow rather than two drifting copies.
pub async fn interactive_flow(
    client_id: String,
    client_secret: String,
    port: u16,
) -> anyhow::Result<crate::token::StoredCredentials> {
    use anyhow::Context;

    let config = google_oauth_config(client_id.clone(), client_secret.clone(), port);
    let pkce = generate_pkce();
    // The PKCE verifier already proves the callback pairs with this attempt;
    // state adds CSRF protection for the browser leg.
    let state = generate_pkce().code_verifier;
    let url = build_auth_url(&config, &pkce, &state);

    eprintln!("Open this URL to authorize (listening on 127.0.0.1:{port}):\n\n{url}\n");
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();

    let (code, returned_state) = wait_for_oauth_redirect(port).await?;
    anyhow::ensure!(returned_state == state, "OAuth state mismatch — try again");

    let tokens = exchange_code(&config, &code, &pkce.code_verifier, &crate::http::client()).await?;
    let refresh_token = tokens.refresh_token.clone().context(
        "Google returned no refresh token; remove the app's access at myaccount.google.com/permissions and re-run",
    )?;

    // Whose mailbox did we just get? Also the first authenticated call, so a
    // scope or consent problem surfaces here rather than at first use.
    let account = crate::google::gmail::GmailProvider::new(tokens.access_token.clone())
        .profile_address()
        .await?;

    Ok(crate::token::StoredCredentials {
        client_id,
        client_secret,
        tenant: None,
        access_token: tokens.access_token,
        refresh_token,
        expires_at: tokens.expires_at.unwrap_or_default(),
        account: Some(account),
    })
}

/// Simple percent-encoding for URL parameters.
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => result.push_str(&format!("%{byte:02X}")),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_pkce() -> PkceChallenge {
        PkceChallenge {
            code_verifier: "verifier".to_string(),
            code_challenge: "challenge".to_string(),
        }
    }

    #[test]
    fn the_auth_url_includes_consent_and_offline() {
        let config = google_oauth_config("id".into(), "secret".into(), DEFAULT_REDIRECT_PORT);
        let url = build_auth_url(&config, &fixed_pkce(), "state-xyz");
        assert!(
            url.contains("&prompt=consent") && url.contains("&access_type=offline"),
            "both are required for Google to return a refresh token every sign-in: {url}"
        );
        assert!(
            url.contains("localhost%3A8924"),
            "the port must ride in the redirect: {url}"
        );
    }

    #[test]
    fn scopes_cover_mail_and_calendar_but_not_modify() {
        let config = google_oauth_config("id".into(), "s".into(), DEFAULT_REDIRECT_PORT);
        let joined = config.scopes.join(" ");
        assert!(joined.contains("gmail.readonly") && joined.contains("gmail.send"));
        assert!(joined.contains("auth/calendar"));
        // Nothing in this crate modifies messages; least privilege wins.
        assert!(!joined.contains("gmail.modify"));
    }

    #[test]
    fn extract_token_error_description_includes_code_and_description() {
        let body = r#"{"error":"invalid_grant","error_description":"Token has been revoked."}"#;
        assert_eq!(
            extract_token_error_description(body),
            "invalid_grant: Token has been revoked."
        );
    }

    #[test]
    fn extract_token_error_description_handles_code_only() {
        assert_eq!(
            extract_token_error_description(r#"{"error":"invalid_grant"}"#),
            "invalid_grant"
        );
    }

    #[test]
    fn extract_token_error_description_handles_description_only() {
        let body = r#"{"error_description":"Token has been expired or revoked."}"#;
        assert_eq!(
            extract_token_error_description(body),
            "Token has been expired or revoked."
        );
    }

    #[test]
    fn extract_token_error_description_falls_back_to_body() {
        assert_eq!(extract_token_error_description("not json"), "not json");
    }

    /// The incident this guards: a revoked refresh token surfaced as a generic
    /// auth error, so a scheduled sweep retried it every two minutes for three
    /// days. `invalid_grant` is permanent and must be its own class.
    #[test]
    fn an_invalid_grant_refresh_is_classified_permanent() {
        let body = r#"{"error":"invalid_grant","error_description":"Token has been revoked."}"#;
        let err = classify_refresh_failure(400, body);
        assert!(matches!(err, MailError::AuthRevoked(_)), "{err}");
        let text = err.to_string();
        assert!(
            text.starts_with(crate::types::AUTH_REVOKED),
            "the sentinel is what string-only surfaces key on: {text}"
        );
        assert!(text.contains("Token has been revoked"), "{text}");
    }

    /// Everything that is not `invalid_grant` stays transient-shaped: a 500,
    /// or a 400 for a different error code, may recover on retry and must not
    /// tell the user to re-authenticate.
    #[test]
    fn other_refresh_failures_stay_generic() {
        assert!(matches!(
            classify_refresh_failure(500, "gateway timeout"),
            MailError::ApiError { status: 500, .. }
        ));
        assert!(matches!(
            classify_refresh_failure(400, r#"{"error":"invalid_client"}"#),
            MailError::ApiError { status: 400, .. }
        ));
    }

    #[test]
    fn append_optional_secret_skips_when_empty() {
        let mut params: Vec<(&str, &str)> = vec![("grant_type", "authorization_code")];
        append_optional_secret(&mut params, "");
        assert!(params.iter().all(|(k, _)| *k != "client_secret"));
    }

    #[test]
    fn append_optional_secret_includes_when_present() {
        let mut params: Vec<(&str, &str)> = vec![("grant_type", "authorization_code")];
        append_optional_secret(&mut params, "shhh");
        assert!(params
            .iter()
            .any(|(k, v)| *k == "client_secret" && *v == "shhh"));
    }

    /// The PKCE pair had no test at all, which was noticed while migrating it
    /// off `rand::thread_rng` — a security-relevant function whose random
    /// source changed with nothing asserting it still worked.
    ///
    /// The challenge check is the real one: it is exactly what the
    /// authorisation server recomputes, so a wrong transform fails every
    /// sign-in with an error that names neither end.
    #[test]
    fn a_pkce_challenge_is_the_url_safe_sha256_of_its_verifier() {
        let pkce = generate_pkce();

        // 32 random bytes, base64url with no padding.
        assert_eq!(pkce.code_verifier.len(), 43, "{}", pkce.code_verifier);
        assert!(
            pkce.code_verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "not URL-safe: {}",
            pkce.code_verifier
        );

        let mut hasher = Sha256::new();
        hasher.update(pkce.code_verifier.as_bytes());
        assert_eq!(
            pkce.code_challenge,
            URL_SAFE_NO_PAD.encode(hasher.finalize()),
            "the challenge is not the digest of the verifier"
        );
    }

    /// A generator that stopped being random would still pass every shape
    /// check above, and would hand every sign-in the same verifier.
    #[test]
    fn two_pkce_pairs_are_not_the_same_pair() {
        let (a, b) = (generate_pkce(), generate_pkce());
        assert_ne!(a.code_verifier, b.code_verifier);
        assert_ne!(a.code_challenge, b.code_challenge);
    }

    #[test]
    fn a_refresh_reply_without_a_refresh_token_keeps_the_old_one() {
        let json: serde_json::Value =
            serde_json::json!({"access_token": "new", "expires_in": 100, "token_type": "Bearer"});
        let tokens = parse_token_response(json, Some("old-refresh"));
        assert_eq!(tokens.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(tokens.access_token, "new");
    }
}
