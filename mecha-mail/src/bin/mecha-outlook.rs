//! `mecha-outlook` — Outlook mail and calendar as MCP tools. Default mode
//! serves MCP over stdio; `auth` runs the **device-code** flow, which needs
//! no redirect URI and no forwarded port — so it works over SSH and against
//! an app registration you must not modify.

use anyhow::{Context, Result};
use clap::Parser;
use mecha_mail::microsoft::{auth, graph_mail::OutlookProvider, server::OutlookTools};
use mecha_mail::{mcp, token};

#[derive(Parser, Debug)]
#[command(name = "mecha-outlook", about = "Outlook mail and calendar as MCP tools")]
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

    let client = mecha_mail::http::client();
    let device = auth::request_device_code(&tenant, &client_id, &client).await?;

    // The whole point of device code: the human signs in wherever they have a
    // browser, which need not be this machine.
    eprintln!(
        "\nTo sign in, open {} on any device\nand enter this code:\n\n    {}\n",
        device.verification_uri, device.user_code
    );

    let mut last_line = 0i64;
    let tokens = auth::poll_for_token(&tenant, &client_id, &device, &client, |remaining| {
        // One line per half-minute, so a long sign-in does not scroll.
        if last_line == 0 || last_line - remaining >= 30 {
            eprintln!("waiting for sign-in… ({}:{:02} left)", remaining / 60, remaining % 60);
            last_line = remaining;
        }
    })
    .await?;

    let refresh_token = tokens
        .refresh_token
        .clone()
        .context("Entra returned no refresh token — check that `offline_access` is consented")?;

    // Which account signed in — nice to record, but **never fatal**. Losing a
    // completed sign-in because a cosmetic lookup failed would make the user
    // authenticate twice for nothing; the tokens are the point.
    let account = match OutlookProvider::new(tokens.access_token.clone()).profile_address().await
    {
        Ok(addr) => Some(addr),
        Err(e) => {
            eprintln!("(signed in, but could not read the account address: {e})");
            None
        }
    };

    token::save(
        &path,
        &token::StoredCredentials {
            client_id,
            client_secret: String::new(), // public client: never a secret
            tenant: Some(tenant),
            access_token: tokens.access_token,
            refresh_token,
            expires_at: tokens.expires_at.unwrap_or_default(),
            account: account.clone(),
        },
    )?;
    eprintln!(
        "\n✓ authenticated{}\n  credentials saved to {}",
        account.map(|a| format!(" as {a}")).unwrap_or_default(),
        path.display()
    );
    Ok(())
}
