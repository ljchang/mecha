//! Microsoft Entra OAuth for a CLI.
//!
//! **Device code, not loopback.** The user's org has already approved one app
//! registration, and its only redirect URI is flowmail's
//! `http://localhost:8923/callback`. Device code needs *no* redirect URI, so
//! this reuses the approved registration without touching it — and it needs
//! no port forwarding when you are working over SSH, which the loopback flow
//! does. The tradeoff is that some tenants block device code by Conditional
//! Access; [`super::auth`] keeps the loopback path available for that case.
//!
//! **No client secret, ever.** flowmail's Outlook flow is a public client:
//! Entra binds the refresh credential to the auth method that minted it, so
//! sending a `client_secret` after a PKCE- or device-code-minted token is
//! rejected with `AADSTS7000215` even when the secret is correct.

use std::time::Duration;

use serde::Deserialize;

use crate::google::auth::OAuthTokens;
use crate::types::MailError;

/// Graph scopes. `offline_access` is what yields a refresh token; the rest
/// mirror flowmail's, minus `Mail.ReadWrite` — nothing here modifies a
/// message in place, and least privilege beats a future consent click.
pub const SCOPES: &[&str] = &[
    "https://graph.microsoft.com/Mail.Read",
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
pub async fn refresh_token(
    tenant: &str,
    client_id: &str,
    refresh_tok: &str,
    client: &reqwest::Client,
) -> Result<OAuthTokens, MailError> {
    let scope = SCOPES.join(" ");
    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_tok),
        ("scope", scope.as_str()),
    ];

    let resp = client.post(token_url(tenant)).form(&params).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(MailError::ApiError {
            status,
            message: humanize_aadsts(&body),
        });
    }
    Ok(crate::google::auth::parse_token_response(
        resp.json().await?,
        Some(refresh_tok),
    ))
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
    })
}

/// Translate the AADSTS codes that actually block people into instructions,
/// keeping the raw text so nothing is lost. Ported from flowmail, with the
/// codes a CLI hits added.
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

    #[test]
    fn scopes_request_offline_access_and_no_readwrite() {
        let joined = SCOPES.join(" ");
        assert!(
            joined.contains("offline_access"),
            "no refresh token without it"
        );
        assert!(joined.contains("Mail.Read") && joined.contains("Mail.Send"));
        assert!(joined.contains("Calendars.ReadWrite"));
        assert!(
            !joined.contains("Mail.ReadWrite"),
            "nothing here modifies a message"
        );
    }

    #[test]
    fn the_endpoints_carry_the_tenant() {
        let t = "995b0936-48d6-40e5-a31e-bf689ec9446f";
        assert!(devicecode_url(t).contains(t) && devicecode_url(t).ends_with("/devicecode"));
        assert!(token_url(t).contains(t) && token_url(t).ends_with("/token"));
    }
}
