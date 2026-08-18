//! Microsoft Entra OAuth for a CLI.
//!
//! **Device code, not loopback.** The user's org has already approved one app
//! registration, whose only redirect URI is another desktop client's loopback
//! callback (`http://localhost:8923/callback`). Device code needs *no*
//! redirect URI, so
//! this reuses the approved registration without touching it — and it needs
//! no port forwarding when you are working over SSH, which the loopback flow
//! does. The tradeoff is that some tenants block device code by Conditional
//! Access; [`super::auth`] keeps the loopback path available for that case.
//!
//! **No client secret, ever.** This Outlook flow is a public client:
//! Entra binds the refresh credential to the auth method that minted it, so
//! sending a `client_secret` after a PKCE- or device-code-minted token is
//! rejected with `AADSTS7000215` even when the secret is correct.

use std::time::Duration;

use serde::Deserialize;

use crate::google::auth::OAuthTokens;
use crate::types::MailError;

/// Graph scopes. `offline_access` is what yields a refresh token; the rest
/// are the least that read, triage, send and calendar work need.
///
/// **`Mail.ReadWrite` replaced `Mail.Read` on 2026-08-18, and it is not a
/// free change.** It subsumes `Mail.Read`, so the surface is one rung wider,
/// not a different tier — but Microsoft classes it high-impact and its
/// recommended user-consent policy **blocks it from end-user consent**. In a
/// managed tenant a non-admin does not see a consent screen at all; they see
/// *"Need admin approval"*. So on a tenant with that policy this scope needs
/// an administrator to grant it to the app registration once, and until they
/// do, `mecha-mail auth` fails at consent rather than at first use.
///
/// That cost is real and was weighed: the alternative is triage that works on
/// Gmail and silently does nothing on Outlook, which is the
/// silently-degrading-sandbox shape this repository refuses everywhere else.
/// A verb that cannot work must fail loudly. `mecha doctor` reports an
/// account whose grant does not cover the triage verbs, so "why did archive
/// not work on this account" is answerable without reading source.
///
/// `User.Read` stays absent for the original reason — `GET /me` is not worth
/// a consent prompt when Sent Items answers the same question.
///
/// Note for later: from 2026-12-31 Microsoft moves modification of *sensitive*
/// mail properties behind a further `Mail-Advanced.ReadWrite`. Nothing here
/// touches those properties today, so this list stands — but the ledge moves.
pub const SCOPES: &[&str] = &[
    "https://graph.microsoft.com/Mail.ReadWrite",
    "https://graph.microsoft.com/Mail.Send",
    "https://graph.microsoft.com/Calendars.ReadWrite",
    "offline_access",
];

fn devicecode_url(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/devicecode")
}

pub fn token_url(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token")
}

/// What Entra hands back to start a device-code flow: a code the human types
/// on any device, and the terms for polling.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    /// Seconds between polls, per the authorization server.
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub message: Option<String>,
}

fn default_interval() -> u64 {
    5
}

/// Begin the flow: ask Entra for a code to show the user.
pub async fn request_device_code(
    tenant: &str,
    client_id: &str,
    client: &reqwest::Client,
) -> Result<DeviceCode, MailError> {
    let scope = SCOPES.join(" ");
    let params = [("client_id", client_id), ("scope", scope.as_str())];

    let resp = client
        .post(devicecode_url(tenant))
        .form(&params)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(MailError::ApiError {
            status,
            message: humanize_aadsts(&body),
        });
    }
    resp.json::<DeviceCode>().await.map_err(MailError::from)
}

/// What one poll of the token endpoint means.
#[derive(Debug)]
pub enum PollOutcome {
    /// The user has not finished signing in; keep polling at the same rate.
    Pending,
    /// We are polling too fast; add 5 seconds, per the OAuth device-flow spec.
    SlowDown,
    /// Terminal: the code expired, the user declined, or the tenant refused.
    Failed(String),
    Success(Box<OAuthTokens>),
}

/// Interpret one token-endpoint response. Pure, so every branch of the
/// device-code contract is testable without a network.
pub fn interpret_poll(status: u16, body: &str) -> PollOutcome {
    let json: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return PollOutcome::Failed(body.to_string()),
    };

    if status == 200 {
        return PollOutcome::Success(Box::new(crate::google::auth::parse_token_response(
            json, None,
        )));
    }

    match json
        .get("error")
        .and_then(|e| e.as_str())
        .unwrap_or_default()
    {
        "authorization_pending" => PollOutcome::Pending,
        "slow_down" => PollOutcome::SlowDown,
        other => {
            let desc = json
                .get("error_description")
                .and_then(|d| d.as_str())
                .unwrap_or(other);
            PollOutcome::Failed(humanize_aadsts(desc))
        }
    }
}

/// Poll until the user finishes signing in, the code expires, or Entra
/// refuses. `on_wait` is called once per poll so a CLI can show progress.
pub async fn poll_for_token(
    tenant: &str,
    client_id: &str,
    device: &DeviceCode,
    client: &reqwest::Client,
    mut on_wait: impl FnMut(i64),
) -> Result<OAuthTokens, MailError> {
    let deadline = chrono::Utc::now().timestamp() + device.expires_in;
    let mut interval = Duration::from_secs(device.interval.max(1));

    loop {
        let remaining = deadline - chrono::Utc::now().timestamp();
        if remaining <= 0 {
            return Err(MailError::AuthError(
                "the device code expired before sign-in completed — run `auth` again".into(),
            ));
        }
        on_wait(remaining);
        tokio::time::sleep(interval).await;

        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", device.device_code.as_str()),
        ];
        let resp = client.post(token_url(tenant)).form(&params).send().await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();

        match interpret_poll(status, &body) {
            PollOutcome::Success(tokens) => return Ok(*tokens),
            PollOutcome::Pending => {}
            // The spec's remedy, and the reason `interval` is not a constant.
            PollOutcome::SlowDown => interval += Duration::from_secs(5),
            PollOutcome::Failed(message) => return Err(MailError::AuthError(message)),
        }
    }
}

/// Refresh a Microsoft access token. Public-client shaped: no secret.
/// The form body of a refresh request. Pure, so the absence of `scope` is a
/// property a test can assert rather than a line someone has to keep noticing.
fn refresh_form_params<'a>(
    client_id: &'a str,
    refresh_tok: &'a str,
) -> [(&'static str, &'a str); 3] {
    [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_tok),
    ]
}

pub async fn refresh_token(
    tenant: &str,
    client_id: &str,
    refresh_tok: &str,
    client: &reqwest::Client,
) -> Result<OAuthTokens, MailError> {
    // **No `scope` on a refresh.** RFC 6749 §6 makes it optional and defaults
    // it to the original grant, and Entra is explicit that a refresh's scopes
    // must be "equivalent to or a subset of" what was originally consented.
    // Sending `SCOPES` here sends a *superset* the moment this list grows —
    // which is exactly what happened on 2026-08-18 when `Mail.ReadWrite`
    // replaced `Mail.Read`: every already-working account would have had its
    // next refresh refused with `invalid_grant`, which
    // `classify_refresh_failure` correctly reads as permanent and reports as a
    // dead login. A scope widening would have taken down the accounts that had
    // not re-consented yet, an hour later, looking exactly like a revocation.
    //
    // Omitting it makes refresh scope-agnostic: an old grant keeps working at
    // its old privileges until the user re-consents, and a new grant refreshes
    // at the new ones. It also matches the Google path, which never sent
    // scope here. All scopes in `SCOPES` target Graph, so there is no
    // multi-resource ambiguity for Entra to resolve.
    let params = refresh_form_params(client_id, refresh_tok);

    let resp = client.post(token_url(tenant)).form(&params).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_refresh_failure(status, &body));
    }
    Ok(crate::google::auth::parse_token_response(
        resp.json().await?,
        Some(refresh_tok),
    ))
}

/// Entra's twin of the Google classifier: `invalid_grant` is the code Entra
/// puts on a dead refresh credential — AADSTS700082 (refresh token expired),
/// AADSTS50173 (revoked by a password change or an admin), AADSTS70000 — and
/// none of them ever recover on retry. Permanent gets its own class; the
/// AADSTS translation still runs so the message stays actionable.
pub(crate) fn classify_refresh_failure(status: u16, body: &str) -> MailError {
    let json = serde_json::from_str::<serde_json::Value>(body).ok();
    let code = json
        .as_ref()
        .and_then(|v| v.get("error").and_then(|c| c.as_str()));
    if code == Some("invalid_grant") {
        let detail = json
            .as_ref()
            .and_then(|v| v.get("error_description").and_then(|d| d.as_str()))
            .unwrap_or(body);
        return MailError::AuthRevoked(humanize_aadsts(detail));
    }
    MailError::ApiError {
        status,
        message: humanize_aadsts(body),
    }
}

/// The complete device-code sign-in: request a code, tell the human where to
/// type it, poll until they do. In the library so `mecha-outlook auth` and
/// the unified `mecha-mail auth` share one flow. The account lookup is
/// deliberately **never fatal** — losing a completed sign-in over a cosmetic
/// detail makes the user authenticate twice; the tokens are the point.
pub async fn device_flow(
    client_id: String,
    tenant: String,
) -> anyhow::Result<crate::token::StoredCredentials> {
    use anyhow::Context;

    let client = crate::http::client();
    let device = request_device_code(&tenant, &client_id, &client).await?;

    // The whole point of device code: the human signs in wherever they have a
    // browser, which need not be this machine.
    eprintln!(
        "\nTo sign in, open {} on any device\nand enter this code:\n\n    {}\n",
        device.verification_uri, device.user_code
    );

    let mut last_line = 0i64;
    let tokens = poll_for_token(&tenant, &client_id, &device, &client, |remaining| {
        // One line per half-minute, so a long sign-in does not scroll.
        if last_line == 0 || last_line - remaining >= 30 {
            eprintln!(
                "waiting for sign-in… ({}:{:02} left)",
                remaining / 60,
                remaining % 60
            );
            last_line = remaining;
        }
    })
    .await?;

    let refresh_token = tokens
        .refresh_token
        .clone()
        .context("Entra returned no refresh token — check that `offline_access` is consented")?;

    let account =
        match crate::microsoft::graph_mail::OutlookProvider::new(tokens.access_token.clone())
            .profile_address()
            .await
        {
            Ok(addr) => Some(addr),
            Err(e) => {
                eprintln!("(signed in, but could not read the account address: {e})");
                None
            }
        };

    Ok(crate::token::StoredCredentials {
        client_id,
        client_secret: String::new(), // public client: never a secret
        tenant: Some(tenant),
        access_token: tokens.access_token,
        refresh_token,
        expires_at: tokens.expires_at.unwrap_or_default(),
        account,
        granted_scopes: tokens.scope,
        granted_at: Some(chrono::Utc::now().to_rfc3339()),
    })
}

/// Translate the AADSTS codes that actually block people into instructions,
/// keeping the raw text so nothing is lost. Covers the codes a CLI hits.
pub fn humanize_aadsts(description: &str) -> String {
    let code = description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|s| s.starts_with("AADSTS"))
        .unwrap_or("");
    let friendly = match code {
        "AADSTS65001" | "AADSTS90094" => Some(
            "Admin consent has not been granted for this app in this tenant. \
             Ask IT to approve it in Entra → Enterprise Applications.",
        ),
        "AADSTS700016" => Some(
            "This app is not registered in this tenant — check the client id \
             and tenant id.",
        ),
        "AADSTS50020" => Some(
            "This account belongs to a different organization than the tenant \
             this app is registered in.",
        ),
        "AADSTS7000218" => Some(
            "The app registration does not allow public client flows, which \
             device code requires. In Entra → App registrations → your app → \
             Authentication → Settings, set 'Allow public client flows' to Yes.",
        ),
        "AADSTS7000215" => Some(
            "A client secret was sent for a public client. This is a bug: the \
             Microsoft flow must never send one.",
        ),
        "AADSTS50059" | "AADSTS900023" => {
            Some("The tenant id was not recognized — check it in Entra → Overview.")
        }
        _ => None,
    };
    match friendly {
        Some(msg) => format!("{msg} ({description})"),
        None => description.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_poll_keeps_waiting_and_slow_down_backs_off() {
        let pending = r#"{"error":"authorization_pending","error_description":"waiting"}"#;
        assert!(matches!(interpret_poll(400, pending), PollOutcome::Pending));
        let slow = r#"{"error":"slow_down"}"#;
        assert!(matches!(interpret_poll(400, slow), PollOutcome::SlowDown));
    }

    #[test]
    fn a_successful_poll_yields_tokens() {
        let body =
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3599,"token_type":"Bearer"}"#;
        match interpret_poll(200, body) {
            PollOutcome::Success(t) => {
                assert_eq!(t.access_token, "at");
                assert_eq!(t.refresh_token.as_deref(), Some("rt"));
                assert!(t.expires_at.unwrap() > chrono::Utc::now().timestamp());
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn terminal_errors_stop_the_loop_with_an_actionable_message() {
        let expired = r#"{"error":"expired_token","error_description":"code expired"}"#;
        assert!(matches!(
            interpret_poll(400, expired),
            PollOutcome::Failed(_)
        ));

        let declined = r#"{"error":"access_denied","error_description":"user declined"}"#;
        assert!(matches!(
            interpret_poll(400, declined),
            PollOutcome::Failed(_)
        ));

        // The one a CLI is most likely to hit on a fresh registration.
        let public = r#"{"error":"invalid_client","error_description":"AADSTS7000218: The request body must contain client_assertion or client_secret."}"#;
        match interpret_poll(400, public) {
            PollOutcome::Failed(msg) => {
                assert!(msg.contains("Allow public client flows"), "{msg}");
                assert!(
                    msg.contains("AADSTS7000218"),
                    "raw text must survive: {msg}"
                );
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn a_non_json_body_is_a_failure_not_a_panic() {
        assert!(matches!(
            interpret_poll(500, "<html>gateway</html>"),
            PollOutcome::Failed(_)
        ));
    }

    /// The triage verbs need `Mail.ReadWrite`, which subsumes `Mail.Read`.
    /// `User.Read` stays out: `GET /me` is not worth a consent prompt when
    /// Sent Items answers the same question, and in a managed tenant every
    /// extra scope is another thing an administrator has to agree to.
    #[test]
    fn scopes_request_offline_access_and_readwrite_but_not_the_directory() {
        let joined = SCOPES.join(" ");
        assert!(
            joined.contains("offline_access"),
            "no refresh token without it"
        );
        assert!(
            joined.contains("Mail.ReadWrite"),
            "triage modifies messages in place: {joined}"
        );
        assert!(joined.contains("Mail.Send"));
        assert!(joined.contains("Calendars.ReadWrite"));
        assert!(!joined.contains("User.Read"), "{joined}");
    }

    /// A refresh must never request scopes the grant may not have.
    ///
    /// Fails on the behaviour shipped until 2026-08-18, where the refresh body
    /// carried `SCOPES`. Widening that list then asked Entra for a superset of
    /// what the stored grant consented to, which it refuses with
    /// `invalid_grant` — indistinguishable from a revocation, and classified
    /// as permanent. The consequence was an outage on every account that had
    /// not re-consented, roughly an hour after the new binary was installed.
    #[test]
    fn a_refresh_requests_no_scopes_at_all() {
        let body = refresh_form_params("client", "refresh-tok");
        let keys: Vec<&str> = body.iter().map(|(k, _)| *k).collect();
        assert!(
            !keys.contains(&"scope"),
            "a refresh inherits the grant's scopes; asking widens it: {keys:?}"
        );
        assert!(keys.contains(&"grant_type"));
        assert!(keys.contains(&"refresh_token"));
        assert!(keys.contains(&"client_id"));
    }

    /// Entra reports a dead refresh credential as `invalid_grant` with an
    /// AADSTS code. Permanent — a sweep that retries it forever is the
    /// recorded three-day silent failure.
    #[test]
    fn an_entra_invalid_grant_refresh_is_classified_permanent() {
        let body = r#"{"error":"invalid_grant","error_description":"AADSTS700082: The refresh token has expired due to inactivity."}"#;
        let err = classify_refresh_failure(400, body);
        assert!(matches!(err, MailError::AuthRevoked(_)), "{err}");
        let text = err.to_string();
        assert!(text.starts_with(crate::types::AUTH_REVOKED), "{text}");
        assert!(
            text.contains("AADSTS700082"),
            "raw code must survive: {text}"
        );

        // A throttle or an outage stays transient-shaped.
        assert!(matches!(
            classify_refresh_failure(503, "service unavailable"),
            MailError::ApiError { status: 503, .. }
        ));
    }

    #[test]
    fn the_endpoints_carry_the_tenant() {
        let t = "995b0936-48d6-40e5-a31e-bf689ec9446f";
        assert!(devicecode_url(t).contains(t) && devicecode_url(t).ends_with("/devicecode"));
        assert!(token_url(t).contains(t) && token_url(t).ends_with("/token"));
    }
}
