//! `mecha-google` — the binary. Default mode serves MCP over stdio; `auth`
//! runs the interactive OAuth flow and stores credentials.

use anyhow::{Context, Result};
use clap::Parser;
use mecha_google::{auth, gmail::GmailProvider, server, token};

#[derive(Parser, Debug)]
#[command(name = "mecha-google", about = "Gmail and Google Calendar as MCP tools")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Run the interactive OAuth flow and store credentials.
    Auth {
        /// Google OAuth client id (Desktop-app type).
        #[arg(long, env = "GMAIL_CLIENT_ID")]
        client_id: Option<String>,
        /// The Desktop-app client's pseudo-secret.
        #[arg(long, env = "GMAIL_CLIENT_SECRET")]
        client_secret: Option<String>,
        /// Loopback port for the OAuth redirect.
        #[arg(long, default_value_t = auth::DEFAULT_REDIRECT_PORT)]
        port: u16,
    },
    /// Serve MCP over stdio (the default when no subcommand is given).
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Auth { client_id, client_secret, port }) => {
            authenticate(client_id, client_secret, port).await
        }
        Some(Command::Serve) | None => {
            let manager = token::TokenManager::load(token::default_path()?)?;
            server::serve(manager).await
        }
    }
}

/// stderr only — stdout belongs to the MCP transport.
fn tracing_subscriber() {
    use tracing_subscriber::EnvFilter;
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("MECHA_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}

async fn authenticate(
    client_id: Option<String>,
    client_secret: Option<String>,
    port: u16,
) -> Result<()> {
    let path = token::default_path()?;

    // Fall back to the stored client credentials, so re-auth after a revoked
    // or expired grant needs no flags.
    let existing = token::load(&path).ok();
    let client_id = client_id
        .or_else(|| existing.as_ref().map(|c| c.client_id.clone()))
        .context("no client id: pass --client-id or set GMAIL_CLIENT_ID")?;
    let client_secret = client_secret
        .or_else(|| existing.as_ref().map(|c| c.client_secret.clone()))
        .unwrap_or_default();

    let config = auth::google_oauth_config(client_id.clone(), client_secret.clone(), port);
    let pkce = auth::generate_pkce();
    // The PKCE verifier already proves the callback pairs with this attempt;
    // state adds CSRF protection for the browser leg.
    let state = auth::generate_pkce().code_verifier;
    let url = auth::build_auth_url(&config, &pkce, &state);

    eprintln!("Open this URL to authorize (listening on 127.0.0.1:{port}):\n\n{url}\n");
    // Best effort; headless machines just use the printed URL.
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();

    let (code, returned_state) = auth::wait_for_oauth_redirect(port).await?;
    anyhow::ensure!(returned_state == state, "OAuth state mismatch — try again");

    let tokens = auth::exchange_code(&config, &code, &pkce.code_verifier, &crate::client()).await?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .context("Google returned no refresh token; remove the app's access at myaccount.google.com/permissions and re-run")?;

    // Whose mailbox did we just get? Also the first authenticated call, so
    // a scope or consent problem surfaces here rather than at first use.
    let account = GmailProvider::new(tokens.access_token.clone()).profile_address().await?;

    token::save(
        &path,
        &token::StoredCredentials {
            client_id,
            client_secret,
            access_token: tokens.access_token,
            refresh_token,
            expires_at: tokens.expires_at.unwrap_or_default(),
            account: Some(account.clone()),
        },
    )?;
    eprintln!("authenticated as {account}; credentials in {}", path.display());
    Ok(())
}

fn client() -> reqwest::Client {
    mecha_google::http::client()
}
