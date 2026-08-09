//! Files, both directions.
//!
//! Out: the three-step external upload, because `files.upload` is retired.
//! The pattern worth using is **upload naming no channel** — the file stays
//! private — and then referencing it from an image block
//! ([`crate::blocks::image_from_file`]), so a rendered chart appears inline in
//! a thread without ever becoming a public URL.
//!
//! In: [`download`], which is where the care is. A private file URL requires a
//! bearer token, and when that token is missing, under-privileged, or stripped
//! by a redirect, **Slack answers HTTP 200 with an HTML sign-in page** — not a
//! 401, not JSON. Nothing about that failure is loud, and the consequence is
//! that a login page reaches the model labelled as the user's screenshot. Four
//! guards, all of them necessary:
//!
//! 1. Send the `Authorization` header explicitly.
//! 2. Never follow redirects — `files.slack.com` redirects to
//!    `<team>.slack.com`, and an HTTP client drops `Authorization` across
//!    hosts, which reproduces the failure reliably. The shared client is built
//!    with `redirect::Policy::none()` for this reason.
//! 3. Reject `text/html` even at HTTP 200.
//! 4. Cross-check the byte count against the size Slack reported.

use serde_json::{json, Value};

use crate::envelope::FileRef;
use crate::error::{SlackError, SlackResult};
use crate::http::Slack;

/// Where an upload should land, if anywhere.
#[derive(Debug, Clone, Default)]
pub struct Share<'a> {
    /// Omit to keep the file private — the recommended path.
    pub channel_id: Option<&'a str>,
    pub thread_ts: Option<&'a str>,
    pub initial_comment: Option<&'a str>,
    pub title: Option<&'a str>,
}

/// Upload bytes and return the new file's id.
///
/// Each step retries independently; the sequence does not. A retried
/// `getUploadURLExternal` costs a wasted URL, a retried byte-POST is
/// idempotent against the same URL, and a retried `completeUploadExternal` is
/// the only one that could double-post — which is why it is the step whose
/// failure is returned rather than re-driven from the top.
pub async fn upload(
    slack: &Slack,
    filename: &str,
    bytes: &[u8],
    share: &Share<'_>,
) -> SlackResult<String> {
    let ticket: Value = slack
        .call_form(
            "files.getUploadURLExternal",
            &[
                ("filename", filename.to_string()),
                ("length", bytes.len().to_string()),
            ],
        )
        .await?;

    let upload_url = ticket
        .get("upload_url")
        .and_then(Value::as_str)
        .ok_or_else(|| SlackError::Malformed {
            method: "files.getUploadURLExternal".into(),
            detail: "no upload_url".into(),
        })?;
    let file_id = ticket
        .get("file_id")
        .and_then(Value::as_str)
        .ok_or_else(|| SlackError::Malformed {
            method: "files.getUploadURLExternal".into(),
            detail: "no file_id".into(),
        })?
        .to_string();

    let put = slack
        .client()
        .post(upload_url)
        .header("content-type", "application/octet-stream")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|source| SlackError::Transport {
            method: "files.upload(bytes)".into(),
            source,
        })?;
    if !put.status().is_success() {
        return Err(SlackError::Api {
            method: "files.upload(bytes)".into(),
            code: format!("HTTP {}", put.status().as_u16()),
        });
    }

    let mut entry = json!({ "id": file_id });
    if let Some(title) = share.title {
        entry["title"] = json!(title);
    }
    let mut body = json!({ "files": [entry] });
    if let Some(channel) = share.channel_id {
        body["channel_id"] = json!(channel);
    }
    if let Some(thread_ts) = share.thread_ts {
        body["thread_ts"] = json!(thread_ts);
    }
    if let Some(comment) = share.initial_comment {
        body["initial_comment"] = json!(comment);
    }
    let _: Value = slack.call("files.completeUploadExternal", body).await?;

    Ok(file_id)
}

/// Fetch a file a user shared, refusing anything that is not the file.
pub async fn download(slack: &Slack, file: &FileRef, max_bytes: u64) -> SlackResult<Vec<u8>> {
    let url = file
        .url_private
        .as_deref()
        .ok_or_else(|| SlackError::NotAFile {
            file_id: file.id.clone(),
            detail: "the event carried no url_private".into(),
        })?;

    if let Some(size) = file.size {
        if size > max_bytes {
            return Err(SlackError::NotAFile {
                file_id: file.id.clone(),
                detail: format!("{size} bytes exceeds the {max_bytes} byte limit"),
            });
        }
    }

    let response = slack
        .client()
        .get(url)
        // Guard 1. `bearer_auth` rather than a query parameter: a token in a
        // URL reaches every log between here and Slack.
        .bearer_auth(slack.bot_token())
        .send()
        .await
        .map_err(|source| SlackError::Transport {
            method: "files.download".into(),
            source,
        })?;

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    guard(&file.id, status, &content_type)?;

    let bytes = response
        .bytes()
        .await
        .map_err(|source| SlackError::Transport {
            method: "files.download".into(),
            source,
        })?;

    // Guard 4.
    if let Some(size) = file.size {
        if bytes.len() as u64 != size {
            return Err(SlackError::NotAFile {
                file_id: file.id.clone(),
                detail: format!("expected {size} bytes, read {}", bytes.len()),
            });
        }
    }
    if bytes.len() as u64 > max_bytes {
        return Err(SlackError::NotAFile {
            file_id: file.id.clone(),
            detail: format!("read {} bytes, over the limit", bytes.len()),
        });
    }

    Ok(bytes.to_vec())
}

/// Guards 2 and 3, pure so the silent failure is testable without Slack.
pub(crate) fn guard(file_id: &str, status: u16, content_type: &str) -> SlackResult<()> {
    // Guard 2. The client follows no redirects, so a 3xx arrives here rather
    // than becoming an unauthenticated request to another host.
    if (300..400).contains(&status) {
        return Err(SlackError::NotAFile {
            file_id: file_id.to_string(),
            detail: format!("HTTP {status}: a redirect, which would drop the Authorization header"),
        });
    }
    if !(200..300).contains(&status) {
        return Err(SlackError::NotAFile {
            file_id: file_id.to_string(),
            detail: format!("HTTP {status}"),
        });
    }
    // Guard 3. The whole point: a 200 is not evidence of a file.
    let kind = content_type.split(';').next().unwrap_or("").trim();
    if kind.eq_ignore_ascii_case("text/html") {
        return Err(SlackError::NotAFile {
            file_id: file_id.to_string(),
            detail: "HTML at HTTP 200 — this is a sign-in page, not the file".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_at_http_200_is_refused() {
        // The failure this module exists to catch. Slack serves a sign-in page
        // with a success status when the token did not arrive.
        let err = guard("F1", 200, "text/html; charset=utf-8").unwrap_err();
        match err {
            SlackError::NotAFile { detail, .. } => assert!(detail.contains("sign-in page")),
            other => panic!("expected NotAFile, got {other:?}"),
        }
        // And the negative, so the test is not vacuous: a real file passes.
        assert!(guard("F1", 200, "image/png").is_ok());
    }

    #[test]
    fn a_redirect_is_refused_rather_than_followed() {
        for status in [301, 302, 307, 308] {
            let err = guard("F1", status, "").unwrap_err();
            assert!(
                matches!(err, SlackError::NotAFile { .. }),
                "HTTP {status} must not become an unauthenticated fetch"
            );
        }
    }

    #[test]
    fn content_type_matching_ignores_parameters_and_case() {
        assert!(guard("F1", 200, "TEXT/HTML").is_err());
        assert!(guard("F1", 200, "text/html;charset=utf-8").is_err());
        assert!(
            guard("F1", 200, "text/plain").is_ok(),
            "a text snippet is a real file"
        );
    }

    #[test]
    fn a_non_success_status_is_still_not_a_file() {
        assert!(guard("F1", 404, "application/json").is_err());
        assert!(guard("F1", 401, "").is_err());
    }
}
