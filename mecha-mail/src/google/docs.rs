//! Google Docs, Sheets and Slides — create, edit and trash, under the one
//! scope in the family that costs nothing.
//!
//! **`drive.file` is the whole design.** It is Google's only *non-sensitive*
//! scope here: no verification, no annual CASA assessment, and — once the
//! Cloud project is published — no seven-day refresh-token expiry. What it
//! buys structurally is better than what it saves: the grant covers only
//! files this app *created* or that the user *handed it*, so mecha cannot
//! reach a document nobody gave it. That is a path jail for Drive, provable
//! by reading a scope string rather than by reviewing every future diff.
//! `docs/DOCS-RESEARCH.md` has the measurements behind each claim.
//!
//! Two flows, and the split is not cosmetic:
//!
//! - [`device_code_flow`] mints the grant with **no redirect at all** —
//!   `drive.file` is one of only six scopes Google's limited-input flow
//!   permits. A headless box signs in over SSH with a code typed on a phone,
//!   exactly as the Microsoft mail account already does. This covers the
//!   common case completely, because **every document mecha creates is in
//!   scope forever with no picking**.
//! - [`pick_flow`] adopts a document that predates mecha, and structurally
//!   *requires* a reachable loopback: the file ids come back on the redirect
//!   (`picked_file_ids`), and a device flow has no redirect to carry them.
//!   So this one needs a browser or an `ssh -L` tunnel, permanently, and no
//!   amount of design removes that.
//!
//! The credential is a [`crate::token::StoredCredentials`] like any other,
//! but under **its own root** (`~/.mecha/docs/<account>/oauth.json`) rather
//! than beside the mail grant. That is deliberate: `mecha doctor` globs
//! `~/.mecha/mail/*/` and reads the `oauth.json` in each account directory
//! *as the mail grant*, asserting it covers that provider's triage scope. A
//! `drive.file` grant sitting there would fail that assertion and be reported
//! as a broken mail account — a finding naming the wrong subsystem, which is
//! worse than no finding. Share the type, never the namespace.

use serde::Deserialize;

use crate::types::MailError;

/// The only scope this surface ever requests. Non-sensitive, and the
/// negative half is load-bearing: `drive` and `drive.readonly` are
/// *restricted* (annual paid assessment), and `documents`/`spreadsheets`/
/// `presentations` are *sensitive* (review, and no publishing until it
/// passes). Widening this is not a config change; it is a different project
/// with a different verification story.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const DRIVE_API: &str = "https://www.googleapis.com/drive/v3";

/// Loopback port for the picker redirect. Desktop-app clients accept any
/// loopback port without registering it, which is what lets this be a
/// default rather than a console setting.
pub const DEFAULT_PICK_PORT: u16 = 8765;

/// `~/.mecha/docs/`, the root this surface owns.
pub fn docs_home() -> anyhow::Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    Ok(home.join(".mecha").join("docs"))
}

/// Where one account's grant lives.
pub fn store_path(account: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(docs_home()?.join(account).join("oauth.json"))
}

/// Every account with a stored grant, sorted. The directory listing *is* the
/// account list — there is no registry file, because a registry that can
/// disagree with the filesystem is a second source of truth.
pub fn accounts() -> anyhow::Result<Vec<String>> {
    let home = docs_home()?;
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&home) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        if entry.path().join("oauth.json").is_file() {
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Why there is no device-code flow here
// ---------------------------------------------------------------------------
//
// `drive.file` *is* one of the six scopes Google's limited-input device flow
// permits, so this looked available and was designed for. It is not, and the
// reason is worth keeping because it is not discoverable from the docs:
//
//   1. `trigger_onepick` is accepted only for a **Desktop-app** client.
//   2. The device flow **refuses** a Desktop-app client outright —
//      `401 invalid_client: "Invalid client type."` (measured 2026-08-18).
//   3. Two client ids do not resolve it. A `drive.file` grant is per
//      *(user, client)*, so files picked under the Desktop client are
//      invisible to a TV client's token. The two would hold disjoint scopes
//      and `pick` would extend a grant that `auth` could never read.
//
// One client must therefore do both, and that client must be Desktop-app, so
// the browser leg is unavoidable. What *is* avoidable is needing the loopback
// to be reachable: see `parse_redirect_url`, which takes the redirect the
// browser already displays in its address bar.

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Picker: adopting a document that predates mecha
// ---------------------------------------------------------------------------

/// A token endpoint response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Build the authorization URL that opens Google's real file chooser.
///
/// `trigger_onepick=true` is the whole trick, and it **cannot be combined
/// with any other scope** — Google rejects the request. That is why this is
/// its own flow rather than an argument to the ordinary one, and part of why
/// the documents grant wants its own Cloud project.
pub fn build_auth_url(client_id: &str, port: u16, state: &str, picker: bool) -> String {
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let mut params = vec![
        ("client_id", client_id),
        ("response_type", "code"),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("redirect_uri", &redirect),
        ("scope", SCOPE),
        ("state", state),
    ];
    if picker {
        params.extend([
            ("trigger_onepick", "true"),
            ("allow_multiple", "true"),
            ("allow_folder_selection", "true"),
        ]);
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{AUTH_URL}?{query}")
}

/// Read the redirect out of a URL the user pasted back.
///
/// **This is the whole no-tunnel story.** A headless box cannot receive the
/// redirect, and device code — which would need none — is unavailable here:
/// Google refuses `trigger_onepick` for any client type but Desktop-app, and
/// refuses Desktop-app clients for the device flow (`invalid_client: Invalid
/// client type`). Two client ids would not fix it either, because a
/// `drive.file` grant is per *(user, client)*: files picked under one client
/// are invisible to the other, so the two would hold disjoint scopes. One
/// client must do both, so the browser leg is unavoidable — but *receiving*
/// it is not. The browser shows the whole redirect in its address bar even
/// when nothing is listening, which is exactly how this was measured.
pub fn parse_redirect_url(url: &str) -> Result<PickerRedirect, MailError> {
    let query = url.split_once('?').map(|(_, q)| q).ok_or_else(|| {
        MailError::AuthError(
            "that does not look like a redirect URL — paste the whole \
             127.0.0.1:… address, including everything after the `?`"
                .into(),
        )
    })?;
    parse_picker_redirect(query.trim())
}

/// What the picker's redirect carries back.
#[derive(Debug, Default, PartialEq)]
pub struct PickerRedirect {
    pub code: String,
    pub state: String,
    /// Ids of everything the user chose. Empty is a legitimate answer — the
    /// user opened the chooser and picked nothing.
    pub picked: Vec<String>,
}

/// Parse the picker's callback query. Pure, because every interesting case
/// here is a string case: a cancel, an empty pick, a multi-pick.
pub fn parse_picker_redirect(query: &str) -> Result<PickerRedirect, MailError> {
    let mut out = PickerRedirect::default();
    let mut error = None;
    let mut description = None;

    for param in query.split('&') {
        let mut parts = param.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = percent_decode(parts.next().unwrap_or_default());
        match key {
            "code" => out.code = value,
            "state" => out.state = value,
            "picked_file_ids" => {
                out.picked = value
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            "error" => error = Some(value),
            "error_description" => description = Some(value),
            _ => {}
        }
    }

    if let Some(err) = error {
        // `access_denied` here is usually not a cancel: it is the Testing
        // publishing status refusing an account that is not a test user.
        // Saying so beats making someone find that out twice.
        let hint = if err == "access_denied" {
            " — if the Cloud project is in Testing, add this account under \
             Audience -> Test users, or publish the app"
        } else {
            ""
        };
        let desc = description.unwrap_or_else(|| err.clone());
        return Err(MailError::AuthError(format!(
            "picker refused: {desc}{hint}"
        )));
    }
    if out.code.is_empty() {
        return Err(MailError::AuthError(
            "no authorization code in the picker redirect".into(),
        ));
    }
    Ok(out)
}

/// Serve one loopback request and read the picker's answer off it.
///
/// Deliberately *not* sharing `auth::wait_for_oauth_redirect`: that one
/// returns `(code, state)` and knows nothing of `picked_file_ids`, and the
/// picker's failure modes want their own wording. Consolidating the two is a
/// reasonable later cleanup; forking the mail flow's listener to do it now
/// is not.
pub async fn wait_for_picker_redirect(
    port: u16,
    timeout_secs: u64,
) -> Result<PickerRedirect, MailError> {
    let addr = format!("127.0.0.1:{port}");
    let server = tiny_http::Server::http(&addr).map_err(|e| {
        MailError::AuthError(format!(
            "cannot listen on {addr}: {e}. Another process may hold the port."
        ))
    })?;

    let request = server
        .recv_timeout(std::time::Duration::from_secs(timeout_secs))
        .map_err(|e| MailError::AuthError(format!("failed to receive redirect: {e}")))?
        .ok_or_else(|| {
            MailError::AuthError(format!(
                "no redirect within {timeout_secs}s. If you are over SSH, the tunnel \
                 must forward {port}: ssh -L {port}:127.0.0.1:{port} <host>"
            ))
        })?;

    let url = request.url().to_string();
    let query = url.split('?').nth(1).unwrap_or_default().to_string();
    let parsed = parse_picker_redirect(&query);

    let html = match &parsed {
        Ok(p) => format!(
            "<html><body><h1>Picked {} file(s)</h1><p>You can close this window \
             and return to the terminal.</p></body></html>",
            p.picked.len()
        ),
        Err(_) => "<html><body><h1>Nothing was picked</h1><p>Return to the terminal \
                   for the reason.</p></body></html>"
            .to_string(),
    };
    let response = tiny_http::Response::from_string(html).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap(),
    );
    let _ = request.respond(response);

    parsed
}

/// Exchange an authorization code for a token.
pub async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    port: u16,
    client: &reqwest::Client,
) -> Result<TokenResponse, MailError> {
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(MailError::AuthError(format!(
            "code exchange failed ({status}): {body}"
        )));
    }
    serde_json::from_str(&body).map_err(|e| MailError::ParseError(e.to_string()))
}

/// Build the credential to store from a token response.
///
/// `granted_at` is stamped here and never on a refresh, matching the mail
/// grant's rule. `grant_lifetime_days` has no analogue to set: a published
/// `drive.file` project's grant does not expire, and that must record as
/// *absence* rather than as some large number, or "does not expire" stops
/// being distinguishable from "nobody measured".
pub fn credentials_from(
    client_id: String,
    client_secret: String,
    tokens: TokenResponse,
    account: Option<String>,
) -> anyhow::Result<crate::token::StoredCredentials> {
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        anyhow::anyhow!(
            "Google returned no refresh token; remove this app's access at \
             myaccount.google.com/permissions and run `auth` again"
        )
    })?;
    let expires_at = chrono::Utc::now().timestamp() + tokens.expires_in.unwrap_or(3600);
    Ok(crate::token::StoredCredentials {
        client_id,
        client_secret,
        tenant: None,
        access_token: tokens.access_token,
        refresh_token,
        expires_at,
        account,
        granted_scopes: tokens.scope,
        granted_at: Some(chrono::Utc::now().to_rfc3339()),
    })
}

// ---------------------------------------------------------------------------
// The Drive/Docs client
// ---------------------------------------------------------------------------

/// One file in scope.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DriveFile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "mimeType", default)]
    pub mime_type: String,
    #[serde(rename = "modifiedTime", default)]
    pub modified_time: Option<String>,
}

/// The document kinds this surface understands, and their Drive MIME types.
pub const MIME_DOC: &str = "application/vnd.google-apps.document";
pub const MIME_SHEET: &str = "application/vnd.google-apps.spreadsheet";
pub const MIME_SLIDES: &str = "application/vnd.google-apps.presentation";
pub const MIME_FOLDER: &str = "application/vnd.google-apps.folder";

/// A short human label for a Drive MIME type.
pub fn kind_of(mime: &str) -> &'static str {
    match mime {
        MIME_DOC => "doc",
        MIME_SHEET => "sheet",
        MIME_SLIDES => "slides",
        MIME_FOLDER => "folder",
        _ => "file",
    }
}

pub struct DocsClient {
    manager: crate::token::TokenManager,
}

impl DocsClient {
    pub fn new(manager: crate::token::TokenManager) -> Self {
        Self { manager }
    }

    async fn get_json(&self, url: &str) -> Result<serde_json::Value, MailError> {
        let token = self
            .manager
            .access_token()
            .await
            .map_err(|e| MailError::AuthError(e.to_string()))?;
        let resp = crate::http::send_with_retry(crate::http::client().get(url).bearer_auth(&token))
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        if status >= 400 {
            return Err(MailError::ApiError {
                status,
                message: body,
            });
        }
        serde_json::from_str(&body).map_err(|e| MailError::ParseError(e.to_string()))
    }

    /// Everything the grant currently reaches.
    ///
    /// Under `drive.file` a listing returns *only in-scope files*, so this is
    /// the honest answer to "what can mecha touch" with no local index to
    /// drift out of date. That is why no `picked.json` exists: the grant is
    /// the record, and a second copy could disagree with it.
    pub async fn list_scope(&self) -> Result<Vec<DriveFile>, MailError> {
        let url = format!(
            "{DRIVE_API}/files?pageSize=200&orderBy=modifiedTime desc\
             &fields=files(id,name,mimeType,modifiedTime)\
             &supportsAllDrives=true&includeItemsFromAllDrives=true"
        );
        let json = self.get_json(&url.replace(' ', "%20")).await?;
        let files = json["files"].clone();
        serde_json::from_value(files).map_err(|e| MailError::ParseError(e.to_string()))
    }

    /// Whose Drive this grant belongs to.
    ///
    /// The mail flow answers this from a profile read, which needs a scope
    /// this surface deliberately does not hold. Drive's own `about` reports
    /// the signed-in user under `drive.file`, so the account can be recorded
    /// without widening anything — and it must be recorded, because an
    /// account label is how a human tells two grants apart and how any
    /// future doctor check names the one that broke.
    pub async fn account_email(&self) -> Result<String, MailError> {
        let json = self
            .get_json(&format!("{DRIVE_API}/about?fields=user(emailAddress)"))
            .await?;
        json["user"]["emailAddress"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| MailError::ParseError("no user in drive about response".into()))
    }

    /// One file's metadata, which is also the cheapest proof that an id is in
    /// scope at all.
    pub async fn file(&self, id: &str) -> Result<DriveFile, MailError> {
        let url = format!(
            "{DRIVE_API}/files/{id}?fields=id,name,mimeType,modifiedTime&supportsAllDrives=true"
        );
        let json = self.get_json(&url).await?;
        serde_json::from_value(json).map_err(|e| MailError::ParseError(e.to_string()))
    }
}

// ---------------------------------------------------------------------------

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The positive half is that the picker is switched on; the **negative
    /// half is load-bearing** and is why this asserts an absence. Google
    /// rejects `trigger_onepick` combined with any other scope, so a second
    /// scope creeping in here would not degrade the feature — it would break
    /// the flow outright, at the one moment a human is watching a browser.
    #[test]
    fn picker_url_requests_the_picker_and_exactly_one_scope() {
        let url = build_auth_url("cid.apps.googleusercontent.com", 8765, "st4te", true);
        assert!(url.contains("trigger_onepick=true"));
        assert!(url.contains("allow_folder_selection=true"));
        assert!(url.contains("drive.file"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback"));
        // No sensitive or restricted scope, ever.
        for forbidden in [
            "auth/drive%20",
            "documents",
            "spreadsheets",
            "presentations",
            "gmail",
        ] {
            assert!(
                !url.contains(forbidden),
                "picker url must not carry {forbidden}"
            );
        }
    }

    /// Consent without picking is the same flow minus the chooser, and it
    /// must stay on the same client id — a `drive.file` grant is per
    /// (user, client), so a second client would hold a disjoint set of files
    /// and `pick` would be extending a scope this token cannot see.
    #[test]
    fn consent_without_the_picker_omits_the_trigger() {
        let url = build_auth_url("cid", 8765, "st", false);
        assert!(!url.contains("trigger_onepick"));
        assert!(!url.contains("allow_folder_selection"));
        assert!(url.contains("drive.file"));
        assert!(url.contains("access_type=offline"));
    }

    /// The no-tunnel path: the browser shows the full redirect even when
    /// nothing is listening, so a pasted address carries everything.
    #[test]
    fn a_pasted_redirect_url_carries_the_same_answer() {
        let pasted = "http://127.0.0.1:8765/callback?state=st&code=4%2Fabc\
                      &picked_file_ids=aaa%2Cbbb";
        let r = parse_redirect_url(pasted).expect("parses");
        assert_eq!(r.code, "4/abc");
        assert_eq!(r.picked, vec!["aaa", "bbb"]);
        // Trailing whitespace from a terminal paste must not corrupt the id.
        let r2 =
            parse_redirect_url("http://127.0.0.1:8765/callback?code=xyz&picked_file_ids=one  \n")
                .expect("parses");
        assert_eq!(r2.picked, vec!["one"]);
    }

    #[test]
    fn something_that_is_not_a_redirect_url_says_so() {
        let err = parse_redirect_url("I pressed cancel").unwrap_err();
        assert!(err.to_string().contains("redirect URL"), "got: {err}");
    }

    #[test]
    fn picked_ids_are_comma_separated_and_percent_encoded() {
        let r = parse_picker_redirect("state=abc&code=4%2Fxyz&picked_file_ids=one%2Ctwo%2Cthree")
            .expect("valid redirect");
        assert_eq!(r.state, "abc");
        assert_eq!(r.code, "4/xyz");
        assert_eq!(r.picked, vec!["one", "two", "three"]);
    }

    /// Opening the chooser and picking nothing is a legitimate outcome, not
    /// an error: the grant still mints. Treating it as failure would make a
    /// change of mind look like a broken flow.
    #[test]
    fn an_empty_pick_is_not_an_error() {
        let r = parse_picker_redirect("code=abc&picked_file_ids=").expect("valid");
        assert!(r.picked.is_empty());
        assert_eq!(r.code, "abc");
    }

    /// The measured failure mode: `access_denied` is usually the Testing
    /// publishing status refusing a non-test-user, not a cancel. The remedy
    /// belongs in the message because otherwise it is discovered twice.
    #[test]
    fn access_denied_names_the_test_user_remedy() {
        let err = parse_picker_redirect("error=access_denied&state=x").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Test users"), "got: {msg}");
        assert!(msg.contains("publish"), "got: {msg}");
    }

    #[test]
    fn a_redirect_without_a_code_is_refused() {
        assert!(parse_picker_redirect("state=only").is_err());
    }

    /// A published `drive.file` project's grant does not expire, and the
    /// credential must say so by *omission* — `granted_at` is stamped, and
    /// nothing invents a lifetime. Unknown must never masquerade as measured.
    #[test]
    fn credentials_stamp_consent_and_claim_no_lifetime() {
        let creds = credentials_from(
            "cid".into(),
            "secret".into(),
            TokenResponse {
                access_token: "at".into(),
                refresh_token: Some("rt".into()),
                expires_in: Some(3600),
                scope: Some(SCOPE.into()),
            },
            Some("someone@example.com".into()),
        )
        .expect("complete response");
        assert!(creds.granted_at.is_some());
        assert_eq!(creds.granted_scopes.as_deref(), Some(SCOPE));
        assert!(creds.expires_at > chrono::Utc::now().timestamp());
        assert!(creds.tenant.is_none());
    }

    /// A response with no refresh token is a dead end that must fail loudly
    /// at consent, not silently store a credential that cannot be renewed.
    #[test]
    fn a_response_without_a_refresh_token_is_refused() {
        let err = credentials_from(
            "cid".into(),
            String::new(),
            TokenResponse {
                access_token: "at".into(),
                refresh_token: None,
                expires_in: None,
                scope: None,
            },
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("permissions"), "got: {err}");
    }

    /// The store must never land under `~/.mecha/mail/`: doctor's mail checks
    /// glob that directory and read each `oauth.json` as a mail grant, so a
    /// `drive.file` credential there is reported as a broken mail account.
    #[test]
    fn the_store_has_its_own_root() {
        let path = store_path("personal").expect("home dir");
        let s = path.to_string_lossy();
        assert!(s.ends_with(".mecha/docs/personal/oauth.json"), "got: {s}");
        assert!(
            !s.contains(".mecha/mail/"),
            "must not share the mail root: {s}"
        );
    }

    #[test]
    fn mime_types_map_to_labels() {
        assert_eq!(kind_of(MIME_DOC), "doc");
        assert_eq!(kind_of(MIME_SHEET), "sheet");
        assert_eq!(kind_of(MIME_SLIDES), "slides");
        assert_eq!(kind_of(MIME_FOLDER), "folder");
        assert_eq!(kind_of("application/pdf"), "file");
    }
}
