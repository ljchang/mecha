//! One process-wide HTTP client (flowmail's pattern — the connection pool is
//! shared instead of a fresh TLS handshake per operation), plus the
//! retry-with-backoff flowmail never had: a 429 or a 5xx from Google is
//! usually a moment's congestion, and surfacing it as a hard tool error made
//! the model give up on transient weather.

use std::sync::OnceLock;
use std::time::Duration;

use crate::types::MailError;

pub fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("building the shared HTTP client")
        })
        .clone()
}

/// Whether this attempt should be retried, and after how long. Pure, so the
/// policy is testable without a network.
pub(crate) fn retry_after(status: Option<u16>, attempt: u32) -> Option<Duration> {
    const MAX_ATTEMPTS: u32 = 3;
    if attempt + 1 >= MAX_ATTEMPTS {
        return None;
    }
    match status {
        // 429 and 5xx are Google's weather; everything else is our request.
        Some(429) | Some(500..=599) | None => {
            Some(Duration::from_millis(500 * 2u64.pow(attempt)))
        }
        Some(_) => None,
    }
}

/// Send a request, retrying transient failures. The builder is cloned per
/// attempt; a non-cloneable (streaming) request gets exactly one try.
pub async fn send_with_retry(
    builder: reqwest::RequestBuilder,
) -> Result<reqwest::Response, MailError> {
    let mut attempt = 0u32;
    loop {
        let this_try = match builder.try_clone() {
            Some(b) => b,
            None => return Ok(builder.send().await?),
        };
        match this_try.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match retry_after(Some(status), attempt) {
                    Some(delay) if !resp.status().is_success() => {
                        tracing::debug!("google api {status}, retrying in {delay:?}");
                        tokio::time::sleep(delay).await;
                    }
                    _ => return Ok(resp),
                }
            }
            Err(e) => match retry_after(None, attempt) {
                Some(delay) => {
                    tracing::debug!("google api transport error ({e}), retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                }
                None => return Err(e.into()),
            },
        }
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_statuses_retry_with_growing_delays_then_stop() {
        assert!(retry_after(Some(429), 0).is_some());
        assert!(retry_after(Some(503), 0).is_some());
        assert!(retry_after(None, 0).is_some(), "transport errors are transient too");
        assert!(retry_after(Some(429), 0) < retry_after(Some(429), 1));
        assert_eq!(retry_after(Some(429), 2), None, "three attempts total");
    }

    #[test]
    fn client_errors_never_retry() {
        for status in [400, 401, 403, 404] {
            assert_eq!(retry_after(Some(status), 0), None, "{status} is our fault");
        }
    }
}
