//! `mecha-outlook` — Outlook mail and calendar as MCP tools. Default mode
//! serves MCP over stdio; `auth` runs the **device-code** flow, which needs
//! no redirect URI and no forwarded port — so it works over SSH and against
//! an app registration you must not modify.

use anyhow::{Context, Result};
use clap::Parser;
use mecha_mail::microsoft::{auth, server::OutlookTools};
use mecha_mail::{mcp, token};

#[derive(Parser, Debug)]
#[command(
    name = "mecha-outlook",
    about = "Outlook mail and calendar as MCP tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Sign in with the device-code flow and store credentials.
    Auth {
        /// Entra application (client) id — Overview page of the app registration.
        #[arg(long, env = "OUTLOOK_CLIENT_ID")]
        client_id: Option<String>,
        /// Directory (tenant) id — same page.
        #[arg(long, env = "OUTLOOK_TENANT_ID")]
        tenant: Option<String>,
    },
    /// Serve MCP over stdio (the default when no subcommand is given).
    Serve,
}

fn store_path() -> Result<std::path::PathBuf> {
    token::provider_path("outlook", "MECHA_OUTLOOK_DIR")
}

#[tokio::main]
async fn main() -> Result<()> {
    mecha_mail::init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Auth { client_id, tenant }) => authenticate(client_id, tenant).await,
        Some(Command::Serve) | None => {
            let manager = token::TokenManager::load_microsoft(store_path()?)?;
            mcp::serve(OutlookTools { manager }).await
        }
    }
}

async fn authenticate(client_id: Option<String>, tenant: Option<String>) -> Result<()> {
    let path = store_path()?;
    // Re-auth after an expired grant needs no flags: the ids are remembered.
    let existing = token::load(&path).ok();
    let client_id = client_id
        .or_else(|| existing.as_ref().map(|c| c.client_id.clone()))
        .context("no client id: pass --client-id or set OUTLOOK_CLIENT_ID")?;
    let tenant = tenant
        .or_else(|| existing.as_ref().and_then(|c| c.tenant.clone()))
        .context("no tenant: pass --tenant or set OUTLOOK_TENANT_ID")?;

    let creds = auth::device_flow(client_id, tenant).await?;
    let account = creds.account.clone();
    token::save(&path, &creds)?;
    eprintln!(
        "\n✓ authenticated{}\n  credentials saved to {}",
        account.map(|a| format!(" as {a}")).unwrap_or_default(),
        path.display()
    );
    Ok(())
}
