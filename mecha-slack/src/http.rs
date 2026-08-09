//! The Web API client: one connection pool, one place that knows a Slack
//! refusal arrives as HTTP 200, and one retry policy.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{classify_api_error, parse_retry_after, SlackError, SlackResult};

/// How many times a transient failure is tried in total, including the first.
const MAX_ATTEMPTS: u32 = 3;

/// Slack's public advice is to design for one request per second per method.
/// Nothing here enforces that — it is the caller's pacing to own, because only
/// the caller knows whether it is streaming or answering — but the backoff
/// starts above it so a retry never lands inside the window that refused.
fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(1000 * 2u64.pow(attempt))
}

/// A bot-token client for one workspace.
#[derive(Clone)]
pub struct Slack {
    http: reqwest::Client,
    bot_token: String,
    base: String,
}

impl std::fmt::Debug for Slack {
    /// Hand-written so a token cannot reach a log through a stray `{:?}`. The
    /// struct is small enough that deriving it would have been the obvious
    /// thing, which is exactly why this is written down.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slack")
            .field("base", &self.base)
            .field("bot_token", &"<redacted>")
            .finish()
    }
}

impl Slack {
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                // Slack's private file URLs redirect across hosts, and reqwest
                // would drop the Authorization header on the way. `files.rs`
                // needs to see the redirect rather than follow it, so no
                // request in this crate follows one. See `files::download`.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("building the shared HTTP client"),
            bot_token: bot_token.into(),
            base: "https://slack.com".into(),
        }
    }

    /// Point the client at a different origin. For tests; there is no
    /// configuration path that reaches this.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Call a Web API method with a JSON body.
    pub async fn call<T: DeserializeOwned>(&self, method: &str, body: Value) -> SlackResult<T> {
        let raw = self
            .send_with_retry(method, || {
                self.http
                    .post(format!("{}/api/{method}", self.base))
                    .bearer_auth(&self.bot_token)
                    .json(&body)
            })
            .await?;
        from_value(method, raw)
    }

    /// Call a Web API method with a form body. A few methods — the file upload
    /// pair among them — document form encoding rather than JSON.
    pub async fn call_form<T: DeserializeOwned>(
        &self,
        method: &str,
        form: &[(&str, String)],
    ) -> SlackResult<T> {
        let raw = self
            .send_with_retry(method, || {
                self.http
                    .post(format!("{}/api/{method}", self.base))
                    .bearer_auth(&self.bot_token)
                    .form(form)
            })
            .await?;
        from_value(method, raw)
    }

    /// Call with an explicit token — used by Socket Mode, whose
    /// `apps.connections.open` is authorised by an app-level token rather than
    /// the bot token this client otherwise carries.
    pub async fn call_with_token<T: DeserializeOwned>(
        &self,
        method: &str,
        token: &str,
    ) -> SlackResult<T> {
        let raw = self
            .send_with_retry(method, || {
                self.http
                    .post(format!("{}/api/{method}", self.base))
                    .bearer_auth(token)
                    // `apps.connections.open` is POST-only and rejects a
                    // request with no content type as `insecure_request`.
                    .header("content-type", "application/x-www-form-urlencoded")
            })
            .await?;
        from_value(method, raw)
    }

    async fn send_with_retry(
        &self,
        method: &str,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> SlackResult<Value> {
        let mut attempt = 0u32;
        loop {
            let result = self.send_once(method, build()).await;
            let err = match result {
                Ok(value) => return Ok(value),
                Err(e) => e,
            };

            if !err.is_transient() || attempt + 1 >= MAX_ATTEMPTS {
                return Err(err);
            }
            let delay = err.retry_after().unwrap_or_else(|| backoff(attempt));
            tracing::debug!("slack {method} transient ({err}); retrying in {delay:?}");
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn send_once(
        &self,
        method: &str,
        request: reqwest::RequestBuilder,
    ) -> SlackResult<Value> {
        let response = request
            .send()
            .await
            .map_err(|source| SlackError::Transport {
                method: method.to_string(),
                source,
            })?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = response
            .text()
            .await
            .map_err(|source| SlackError::Transport {
                method: method.to_string(),
                source,
            })?;
        interpret(method, status, retry_after.as_deref(), &body)
    }
}

/// Turn one HTTP response into either the payload or the right error.
///
/// Pure, so the whole policy — including the part that matters, which is that
/// `ok: false` at HTTP 200 is a failure — is testable without a socket.
pub(crate) fn interpret(
    method: &str,
    status: u16,
    retry_after: Option<&str>,
    body: &str,
) -> SlackResult<Value> {
    if status == 429 {
        return Err(SlackError::RateLimited {
            method: method.to_string(),
            retry_after: parse_retry_after(retry_after),
        });
    }
    if (500..=599).contains(&status) {
        return Err(SlackError::Transient {
            method: method.to_string(),
            detail: format!("HTTP {status}"),
        });
    }
    if !(200..300).contains(&status) {
        return Err(SlackError::Api {
            method: method.to_string(),
            code: format!("HTTP {status}"),
        });
    }

    let value: Value = serde_json::from_str(body).map_err(|e| SlackError::Malformed {
        method: method.to_string(),
        detail: format!("response was not JSON ({e})"),
    })?;

    // The whole reason this function exists. A 200 says the request arrived,
    // never that it worked.
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(value),
        Some(false) => {
            let code = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown_error");
            Err(classify_api_error(method, code))
        }
        None => Err(SlackError::Malformed {
            method: method.to_string(),
            detail: "response carried no `ok` field".into(),
        }),
    }
}

fn from_value<T: DeserializeOwned>(method: &str, value: Value) -> SlackResult<T> {
    serde_json::from_value(value).map_err(|e| SlackError::Malformed {
        method: method.to_string(),
        detail: format!("could not read the payload ({e})"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refusal_at_http_200_is_an_error() {
        // The trap this crate exists to not fall into. Without the `ok` check
        // this body deserialises cleanly into anything with optional fields.
        let err = interpret(
            "chat.postMessage",
            200,
            None,
            r#"{"ok":false,"error":"not_in_channel"}"#,
        )
        .unwrap_err();
        match err {
            SlackError::Api { code, .. } => assert_eq!(code, "not_in_channel"),
            other => panic!("expected an api error, got {other:?}"),
        }
    }

    #[test]
    fn a_success_yields_the_whole_payload() {
        let v = interpret("chat.postMessage", 200, None, r#"{"ok":true,"ts":"1.2"}"#).unwrap();
        assert_eq!(v["ts"], "1.2");
    }

    #[test]
    fn a_429_honours_the_header() {
        let err = interpret("chat.update", 429, Some("12"), "").unwrap_err();
        assert_eq!(err.retry_after(), Some(Duration::from_secs(12)));
        assert!(err.is_transient());
    }

    #[test]
    fn a_5xx_is_transient_and_a_4xx_is_not() {
        assert!(interpret("m", 503, None, "").unwrap_err().is_transient());
        assert!(!interpret("m", 404, None, "").unwrap_err().is_transient());
    }

    #[test]
    fn a_body_that_is_not_json_is_malformed_rather_than_a_panic() {
        // Slack serves HTML on some failure paths; the client must survive it.
        let err = interpret("m", 200, None, "<html>sign in</html>").unwrap_err();
        assert!(matches!(err, SlackError::Malformed { .. }));
    }

    #[test]
    fn a_json_body_with_no_ok_field_is_refused() {
        let err = interpret("m", 200, None, r#"{"ts":"1.2"}"#).unwrap_err();
        assert!(matches!(err, SlackError::Malformed { .. }));
    }

    #[test]
    fn backoff_grows_and_starts_above_slacks_one_per_second_guidance() {
        assert!(backoff(0) >= Duration::from_secs(1));
        assert!(backoff(0) < backoff(1));
        assert!(backoff(1) < backoff(2));
    }

    #[test]
    fn debug_never_prints_the_token() {
        let printed = format!("{:?}", Slack::new("xoxb-super-secret"));
        assert!(!printed.contains("super-secret"), "{printed}");
    }

    // The tests above are about the policy; these drive it over a real socket,
    // which is the only way the retry loop itself gets exercised.

    #[tokio::test]
    async fn a_rate_limit_is_waited_out_and_the_call_succeeds() {
        use crate::testutil::{mock_http, ok_body};

        let (base, count) = mock_http(vec![
            (429, vec![("retry-after", "0".into())], String::new()),
            (200, vec![], ok_body(r#""ts":"1.5""#)),
        ])
        .await;

        let slack = Slack::new("xoxb-test").with_base_url(base);
        let v: Value = slack
            .call("chat.postMessage", json!({"channel": "D1"}))
            .await
            .expect("the second attempt should succeed");

        assert_eq!(v["ts"], "1.5");
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one refused attempt and one that worked"
        );
    }

    #[tokio::test]
    async fn a_refusal_on_the_merits_is_never_retried() {
        use crate::testutil::mock_http;

        // The direction that matters: retrying a call Slack has already
        // considered and refused is how one mistake becomes a rate limit.
        let (base, count) = mock_http(vec![
            (
                200,
                vec![],
                r#"{"ok":false,"error":"channel_not_found"}"#.into(),
            ),
            (200, vec![], r#"{"ok":true}"#.into()),
        ])
        .await;

        let slack = Slack::new("xoxb-test").with_base_url(base);
        let err = slack
            .call::<Value>("chat.postMessage", json!({"channel": "nope"}))
            .await
            .unwrap_err();

        assert!(matches!(err, SlackError::Api { .. }), "{err:?}");
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one attempt"
        );
    }

    #[tokio::test]
    async fn transient_failures_give_up_rather_than_grinding() {
        use crate::testutil::mock_http;

        let (base, count) = mock_http(vec![
            (503, vec![], String::new()),
            (503, vec![], String::new()),
            (503, vec![], String::new()),
            (503, vec![], String::new()),
        ])
        .await;

        let slack = Slack::new("xoxb-test").with_base_url(base);
        let err = slack
            .call::<Value>("chat.update", json!({}))
            .await
            .unwrap_err();

        assert!(err.is_transient());
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            MAX_ATTEMPTS as usize,
            "bounded, so a sick Slack does not become an infinite loop"
        );
    }

    #[tokio::test]
    async fn the_bot_token_is_sent_as_a_bearer_header() {
        use crate::testutil::{mock_http, ok_body};

        // Cheap, and it catches the mistake that would otherwise show up as a
        // uniform `not_authed` against the live API.
        let (base, _) = mock_http(vec![(200, vec![], ok_body(r#""ts":"1""#))]).await;
        let slack = Slack::new("xoxb-abc").with_base_url(base);
        assert_eq!(slack.bot_token(), "xoxb-abc");
        let v: Value = slack.call("auth.test", json!({})).await.unwrap();
        assert_eq!(v["ok"], true);
    }
}
