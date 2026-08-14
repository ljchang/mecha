//! The token lifecycle, kept in the library rather than in whatever calls it:
//! storage, refresh, and retry-on-401 all live here. One JSON file, mode
//! 0600, and a manager that refreshes ahead of expiry and persists rotated
//! tokens atomically.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::google::auth::OAuthConfig;

/// Create the store directory owner-only. The files inside already enforce
/// 0600 on themselves; the directory holding tokens and account addresses
/// deserves the matching rule, and this tightens a pre-existing one too.
pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Everything needed to mint access tokens, in one place. A Desktop-client
/// secret is non-secret by Google's own definition and absent entirely for
/// Microsoft public clients; the refresh token is exactly as sensitive as
/// the file's 0600 says it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredentials {
    pub client_id: String,
    /// Empty for public clients (Microsoft): sending a secret after a
    /// PKCE-minted token is rejected with AADSTS7000215.
    #[serde(default)]
    pub client_secret: String,
    /// The Entra tenant, for Microsoft. Absent for Google.
    #[serde(default)]
    pub tenant: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The account this authenticated as, for display.
    #[serde(default)]
    pub account: Option<String>,
}

/// Where one provider's credentials live: `~/.mecha/<provider>/oauth.json`,
/// or `$<ENV>` when set. Separate stores per provider on purpose — one
/// provider's re-auth must never disturb the other's tokens.
pub fn provider_path(provider: &str, env_var: &str) -> Result<PathBuf> {
    if let Ok(dir) = std::env::var(env_var) {
        return Ok(PathBuf::from(dir).join("oauth.json"));
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".mecha").join(provider).join("oauth.json"))
}

pub fn default_path() -> Result<PathBuf> {
    provider_path("google", "MECHA_GOOGLE_DIR")
}

pub fn load(path: &Path) -> Result<StoredCredentials> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {} — run `mecha-google auth` first", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Write atomically with owner-only permissions. The temp sibling gets the
/// mode *before* the contents, so no window exists where another user could
/// read a token.
///
/// Also clears the account's [`AuthErrorMarker`]: every successful credential
/// write — a refresh that rotated the token, a fresh `auth` — is proof the
/// login works again, and a marker that outlives the recovery would keep
/// every surface reporting a dead account that is alive.
pub fn save(path: &Path, creds: &StoredCredentials) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        create_private_dir(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(creds)?.as_bytes())?;
    }
    std::fs::rename(&tmp, path)?;
    clear_auth_error(path);
    Ok(())
}

/// The sidecar a **permanent** refresh failure leaves beside `oauth.json`, so
/// every surface — the CLI, an agent, a script on a timer — can notice a dead
/// login without spending a network call or a token. Written when a refresh
/// comes back `invalid_grant`, removed by [`save`] on the next successful
/// refresh or re-auth. Absence means the last refresh worked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthErrorMarker {
    /// RFC 3339, when the permanent failure was first observed.
    pub at: String,
    /// The full remediation message, naming the exact re-auth command.
    pub message: String,
}

/// `auth_error.json`, beside the given credential store.
pub fn auth_error_path(credentials: &Path) -> PathBuf {
    credentials.with_file_name("auth_error.json")
}

/// The marker for this credential store, if its login is known dead.
pub fn load_auth_error(credentials: &Path) -> Option<AuthErrorMarker> {
    let text = std::fs::read_to_string(auth_error_path(credentials)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Best-effort by design: the marker is a courtesy signal beside the real
/// error, and failing the refresh error over a marker write would bury the
/// message that names the fix. Owner-only like everything else in the store.
fn record_auth_error(credentials: &Path, message: &str) {
    use std::os::unix::fs::OpenOptionsExt;
    let marker = AuthErrorMarker {
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        message: message.to_string(),
    };
    let Ok(text) = serde_json::to_string_pretty(&marker) else {
        return;
    };
    let path = auth_error_path(credentials);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(text.as_bytes())
        });
}

fn clear_auth_error(credentials: &Path) {
    let _ = std::fs::remove_file(auth_error_path(credentials));
}

/// Which provider's refresh endpoint to talk to. The stores are separate
/// files; this is the one behavioural difference between them.
enum Refresher {
    Google(Box<OAuthConfig>),
    /// Public client: tenant + client id, never a secret.
    Microsoft {
        tenant: String,
        client_id: String,
    },
}

/// Hands out live access tokens, refreshing behind a lock so concurrent tool
/// calls cannot race two refreshes (both providers rotate the token; the
/// loser of that race would persist a stale one).
pub struct TokenManager {
    path: PathBuf,
    creds: Mutex<StoredCredentials>,
    refresher: Refresher,
}

/// Refresh this many seconds before nominal expiry — clock skew plus the
/// duration of the call the token is about to be used for.
const EXPIRY_MARGIN_SECS: i64 = 120;

impl TokenManager {
    /// Load a Google credential store.
    pub fn load(path: PathBuf) -> Result<Self> {
        let creds = load(&path)?;
        Ok(Self::with_credentials(path, creds))
    }

    /// Wrap credentials the caller already read — the unified server loads
    /// them once for the account address and must not read the file twice
    /// (a re-auth between two reads would pair one login's address with
    /// another's tokens).
    pub fn with_credentials(path: PathBuf, creds: StoredCredentials) -> Self {
        let config = crate::google::auth::google_oauth_config(
            creds.client_id.clone(),
            creds.client_secret.clone(),
            crate::google::auth::DEFAULT_REDIRECT_PORT,
        );
        TokenManager {
            path,
            creds: Mutex::new(creds),
            refresher: Refresher::Google(Box::new(config)),
        }
    }

    /// Load a Microsoft credential store. The tenant is recorded at auth
    /// time; without it there is no endpoint to refresh against.
    pub fn load_microsoft(path: PathBuf) -> Result<Self> {
        let creds = load(&path)?;
        Self::with_credentials_microsoft(path, creds).context("run `mecha-outlook auth` again")
    }

    /// The Microsoft twin of [`Self::with_credentials`]. Errors when the
    /// store has no tenant; the caller owns naming the right re-auth
    /// command for its surface.
    pub fn with_credentials_microsoft(path: PathBuf, creds: StoredCredentials) -> Result<Self> {
        let tenant = creds
            .tenant
            .clone()
            .context("stored credentials have no tenant")?;
        let client_id = creds.client_id.clone();
        Ok(TokenManager {
            path,
            creds: Mutex::new(creds),
            refresher: Refresher::Microsoft { tenant, client_id },
        })
    }

    pub async fn account(&self) -> Option<String> {
        self.creds.lock().await.account.clone()
    }

    /// A currently-valid access token, refreshed if near expiry.
    pub async fn access_token(&self) -> Result<String> {
        let mut creds = self.creds.lock().await;
        let now = chrono::Utc::now().timestamp();
        if creds.expires_at - now > EXPIRY_MARGIN_SECS {
            return Ok(creds.access_token.clone());
        }
        self.refresh_locked(&mut creds).await
    }

    /// Refresh regardless of expiry — the 401 path, where the server said the
    /// token is dead whatever the clock thinks.
    pub async fn force_refresh(&self) -> Result<String> {
        let mut creds = self.creds.lock().await;
        self.refresh_locked(&mut creds).await
    }

    /// The account name and provider this store answers to, for naming the
    /// re-auth command. The account is the store's directory (`~/.mecha/mail/
    /// <name>/oauth.json` — the registry's own layout); a legacy per-provider
    /// store yields its provider's name, which still reads as a usable
    /// placeholder in the command.
    fn identity(&self) -> (String, &'static str) {
        let account = self
            .path
            .parent()
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<account>".into());
        let provider = match &self.refresher {
            Refresher::Google(_) => "google",
            Refresher::Microsoft { .. } => "outlook",
        };
        (account, provider)
    }

    async fn refresh_locked(&self, creds: &mut StoredCredentials) -> Result<String> {
        let client = crate::http::client();
        let result = match &self.refresher {
            Refresher::Google(config) => {
                crate::google::auth::refresh_token(config, &creds.refresh_token, &client)
                    .await
                    .context("refreshing the Google access token")
            }
            Refresher::Microsoft { tenant, client_id } => crate::microsoft::auth::refresh_token(
                tenant,
                client_id,
                &creds.refresh_token,
                &client,
            )
            .await
            .context("refreshing the Microsoft access token"),
        };
        let tokens = match result {
            Ok(t) => t,
            Err(e) => {
                // Permanent, so say so once and leave a marker every surface
                // can read for free — the alternative is the recorded failure
                // mode: the same generic auth error every two minutes for
                // three days, and a stranded booking nobody heard about.
                if let Some(crate::types::MailError::AuthRevoked(detail)) =
                    e.downcast_ref::<crate::types::MailError>()
                {
                    let (account, provider) = self.identity();
                    let message = format!(
                        "account `{account}`: {} — run `mecha-mail auth {account} \
                         --provider {provider}` ({detail})",
                        crate::types::AUTH_REVOKED
                    );
                    record_auth_error(&self.path, &message);
                    return Err(anyhow::anyhow!(message));
                }
                return Err(e);
            }
        };
        creds.access_token = tokens.access_token.clone();
        if let Some(rt) = tokens.refresh_token {
            creds.refresh_token = rt;
        }
        creds.expires_at = tokens
            .expires_at
            .unwrap_or_else(|| chrono::Utc::now().timestamp() + 3600);
        save(&self.path, creds)?;
        Ok(tokens.access_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> StoredCredentials {
        StoredCredentials {
            client_id: "id".into(),
            client_secret: "secret".into(),
            tenant: None,
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 1_000,
            account: Some("me@example.edu".into()),
        }
    }

    #[test]
    fn credentials_round_trip_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("mecha-google-token-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("oauth.json");

        save(&path, &creds()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "tokens must be owner-only");

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.refresh_token, "rt");
        assert_eq!(loaded.account.as_deref(), Some("me@example.edu"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_store_names_the_fix() {
        let err = load(Path::new("/nonexistent/oauth.json")).unwrap_err();
        assert!(err.to_string().contains("mecha-google auth"), "{err}");
    }

    /// A scripted token endpoint: each queued `(status, body)` answers one
    /// request, so a test can fail a refresh permanently and then let the
    /// next one succeed.
    fn scripted_token_endpoint(responses: Vec<(u16, &'static str)>) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap().to_string();
        std::thread::spawn(move || {
            for (request, (status, body)) in server.incoming_requests().zip(responses) {
                let response =
                    tiny_http::Response::from_string(body).with_status_code(status);
                let _ = request.respond(response);
            }
        });
        format!("http://{addr}/token")
    }

    fn manager_against(dir: &Path, token_url: String) -> TokenManager {
        let path = dir.join("personal").join("oauth.json");
        save(&path, &creds()).unwrap();
        let mut config =
            crate::google::auth::google_oauth_config("id".into(), "secret".into(), 1);
        config.token_url = token_url;
        TokenManager {
            path: path.clone(),
            creds: Mutex::new(load(&path).unwrap()),
            refresher: Refresher::Google(Box::new(config)),
        }
    }

    /// The 2026-08-11 incident, as a test: a revoked refresh token surfaced
    /// as a generic auth error, indistinguishable from a transient one, and a
    /// scheduled sweep retried it silently for three days. A permanent
    /// failure must name the re-auth command, leave a marker beside the
    /// store, and have the marker cleared by the recovery.
    #[tokio::test]
    async fn a_revoked_refresh_names_the_fix_leaves_a_marker_and_recovery_clears_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let url = scripted_token_endpoint(vec![
            (
                400,
                r#"{"error":"invalid_grant","error_description":"Token has been revoked."}"#,
            ),
            (
                200,
                r#"{"access_token":"new-at","expires_in":3600,"token_type":"Bearer"}"#,
            ),
        ]);
        let manager = manager_against(dir.path(), url);

        let err = manager.force_refresh().await.unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains(crate::types::AUTH_REVOKED), "{text}");
        assert!(
            text.contains("run `mecha-mail auth personal --provider google`"),
            "the error must name the exact re-auth command: {text}"
        );

        let marker = load_auth_error(&manager.path).expect("a marker beside oauth.json");
        assert!(marker.message.contains(crate::types::AUTH_REVOKED));
        assert!(marker.message.contains("mecha-mail auth personal"));
        let mode = std::fs::metadata(auth_error_path(&manager.path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the marker is owner-only like the store");

        // The next successful refresh is the recovery, and it must clear the
        // marker — a signal that outlives the outage reports a dead account
        // that is alive.
        let token = manager.force_refresh().await.unwrap();
        assert_eq!(token, "new-at");
        assert!(
            load_auth_error(&manager.path).is_none(),
            "a successful refresh must clear the marker"
        );
    }

    /// A transient failure keeps its transient shape: no marker, no
    /// instruction to re-authenticate — telling someone to redo a sign-in
    /// over an outage teaches them to ignore the message that matters.
    #[tokio::test]
    async fn a_transient_refresh_failure_leaves_no_marker() {
        let dir = tempfile::tempdir().unwrap();
        let url = scripted_token_endpoint(vec![(503, "service unavailable")]);
        let manager = manager_against(dir.path(), url);

        let err = manager.force_refresh().await.unwrap_err();
        let text = format!("{err:#}");
        assert!(!text.contains(crate::types::AUTH_REVOKED), "{text}");
        assert!(load_auth_error(&manager.path).is_none());
    }
}
