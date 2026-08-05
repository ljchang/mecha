//! `mecha-google` — Gmail and Google Calendar as MCP tools. Default mode
//! serves MCP over stdio; `auth` runs the interactive OAuth flow (loopback
//! redirect) and stores credentials.

use anyhow::{Context, Result};
use clap::Parser;
use mecha_mail::google::{auth, server::GoogleTools};
use mecha_mail::{mcp, token};

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
    mecha_mail::init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Auth { client_id, client_secret, port }) => {
            authenticate(client_id, client_secret, port).await
        }
        Some(Command::Serve) | None => {
            let manager = token::TokenManager::load(token::default_path()?)?;
            mcp::serve(GoogleTools { manager }).await
        }
    }
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

    let creds = auth::interactive_flow(client_id, client_secret, port).await?;
    let account = creds.account.clone().unwrap_or_default();
    token::save(&path, &creds)?;
    eprintln!("authenticated as {account}; credentials in {}", path.display());
    Ok(())
}
