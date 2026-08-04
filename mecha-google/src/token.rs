//! The token lifecycle — the piece flowmail never had in Rust (its storage,
//! refresh, and retry-on-401 all lived in the JS frontend). One JSON file,
//! mode 0600, and a manager that refreshes ahead of expiry and persists
//! rotated tokens atomically.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::auth::{self, OAuthConfig};

/// Everything needed to mint access tokens, in one place: the Desktop-client
/// credentials are non-secret by Google's own definition, and the refresh
/// token is exactly as sensitive as the file's 0600 says it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// The account this authenticated as, for display.
    #[serde(default)]
    pub account: Option<String>,
}

pub fn default_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("MECHA_GOOGLE_DIR") {
        return Ok(PathBuf::from(dir).join("oauth.json"));
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".mecha").join("google").join("oauth.json"))
}

pub fn load(path: &Path) -> Result<StoredCredentials> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!("reading {} — run `mecha-google auth` first", path.display())
    })?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Write atomically with owner-only permissions. The temp sibling gets the
/// mode *before* the contents, so no window exists where another user could
/// read a token.
pub fn save(path: &Path, creds: &StoredCredentials) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
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
    Ok(())
}

/// Hands out live access tokens, refreshing behind a lock so concurrent tool
/// calls cannot race two refreshes (Google rotates the token; the loser of
/// that race would persist a stale one).
pub struct TokenManager {
    path: PathBuf,
    creds: Mutex<StoredCredentials>,
    config: OAuthConfig,
}

/// Refresh this many seconds before nominal expiry — clock skew plus the
/// duration of the call the token is about to be used for.
const EXPIRY_MARGIN_SECS: i64 = 120;

impl TokenManager {
    pub fn load(path: PathBuf) -> Result<Self> {
        let creds = load(&path)?;
        let config = auth::google_oauth_config(
            creds.client_id.clone(),
            creds.client_secret.clone(),
            auth::DEFAULT_REDIRECT_PORT,
        );
        Ok(TokenManager { path, creds: Mutex::new(creds), config })
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

    async fn refresh_locked(&self, creds: &mut StoredCredentials) -> Result<String> {
        let tokens =
            auth::refresh_token(&self.config, &creds.refresh_token, &crate::http::client())
                .await
                .context("refreshing the Google access token")?;
        creds.access_token = tokens.access_token.clone();
        if let Some(rt) = tokens.refresh_token {
            creds.refresh_token = rt;
        }
        creds.expires_at =
            tokens.expires_at.unwrap_or_else(|| chrono::Utc::now().timestamp() + 3600);
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
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_at: 1_000,
            account: Some("me@example.edu".into()),
        }
    }

    #[test]
    fn credentials_round_trip_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir()
            .join(format!("mecha-google-token-test-{}", std::process::id()));
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
}
