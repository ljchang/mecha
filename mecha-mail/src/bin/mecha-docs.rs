//! `mecha-docs` — Google Docs, Sheets and Slides under the `drive.file`
//! scope.
//!
//! Two verbs, one grant:
//!
//! - `auth` consents once. Everything mecha **creates** from then on is in
//!   scope permanently, with nothing to pick.
//! - `pick` opens Google's real file chooser to adopt a document that
//!   already existed. Per-document: picking a folder puts the folder in
//!   scope, not its contents.
//!
//! Both need a browser *somewhere*, and it does not have to be this machine.
//! There is deliberately no device-code option — see `google::docs` for the
//! measured reason. What replaces it is `--paste`: run the URL in any
//! browser, then paste the address it lands on back here. The redirect is
//! fully visible in the address bar even when nothing is listening, so a
//! headless box needs no tunnel and no forwarded port.

use anyhow::{Context, Result};
use clap::Parser;
use mecha_mail::google::docs;
use mecha_mail::token;

#[derive(Parser, Debug)]
#[command(
    name = "mecha-docs",
    about = "Google Docs, Sheets and Slides as MCP tools (drive.file scope)"
)]
struct Cli {
    /// Which stored grant to use. Accounts live in ~/.mecha/docs/<name>/.
    #[arg(long, global = true, default_value = "personal")]
    account: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Consent once. Covers every document mecha creates from then on.
    Auth {
        /// OAuth client id (Desktop-app type).
        #[arg(long, env = "MECHA_DOCS_CLIENT_ID")]
        client_id: Option<String>,
        /// The Desktop-app client's pseudo-secret.
        #[arg(long, env = "MECHA_DOCS_CLIENT_SECRET")]
        client_secret: Option<String>,
        /// Read the client id and secret from a downloaded client JSON.
        #[arg(long)]
        client_json: Option<std::path::PathBuf>,
        #[command(flatten)]
        capture: Capture,
    },
    /// Open Google's file chooser to put existing documents in scope.
    Pick {
        #[command(flatten)]
        capture: Capture,
    },
    /// List every file this grant can reach.
    List,
}

/// How the redirect gets back here.
#[derive(clap::Args, Debug, Clone)]
struct Capture {
    /// Loopback port to listen on. Must match any SSH tunnel.
    #[arg(long, default_value_t = docs::DEFAULT_PICK_PORT)]
    port: u16,
    /// How long to wait for the redirect, in seconds.
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    /// Do not listen: print the URL and read the resulting address back from
    /// stdin. The path for a headless machine with no tunnel.
    #[arg(long)]
    paste: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    mecha_mail::init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Auth {
            client_id,
            client_secret,
            client_json,
            capture,
        } => {
            let (id, secret) = resolve_client(&cli.account, client_id, client_secret, client_json)?;
            consent(&cli.account, id, secret, false, &capture).await
        }
        Command::Pick { capture } => {
            let stored = token::load(&docs::store_path(&cli.account)?).with_context(|| {
                format!(
                    "no grant for account {:?} — run `mecha-docs auth` first",
                    cli.account
                )
            })?;
            consent(
                &cli.account,
                stored.client_id.clone(),
                stored.client_secret.clone(),
                true,
                &capture,
            )
            .await
        }
        Command::List => list(&cli.account).await,
    }
}

/// Resolve client credentials: explicit flags, then a downloaded client JSON,
/// then whatever the stored grant already used — so re-authenticating after a
/// revoked grant needs no flags, the rule the mail binaries already follow.
fn resolve_client(
    account: &str,
    client_id: Option<String>,
    client_secret: Option<String>,
    client_json: Option<std::path::PathBuf>,
) -> Result<(String, String)> {
    if let Some(path) = client_json {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let json: serde_json::Value = serde_json::from_str(&raw)?;
        anyhow::ensure!(
            json.get("web").is_none(),
            "that is a Web-application client. The picker and the loopback \
             redirect both require a Desktop-app client."
        );
        let cfg = json.get("installed").unwrap_or(&json);
        let id = cfg["client_id"]
            .as_str()
            .context("no client_id in the client JSON")?;
        return Ok((
            id.to_string(),
            cfg["client_secret"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ));
    }
    let stored = token::load(&docs::store_path(account)?).ok();
    let id = client_id
        .or_else(|| stored.as_ref().map(|c| c.client_id.clone()))
        .context("no client id: pass --client-id, --client-json, or set MECHA_DOCS_CLIENT_ID")?;
    let secret = client_secret
        .or_else(|| stored.as_ref().map(|c| c.client_secret.clone()))
        .unwrap_or_default();
    Ok((id, secret))
}

/// The one browser leg, shared by `auth` and `pick` because they differ only
/// in whether the chooser opens. Keeping it one function is what stops the
/// two drifting into two grants.
async fn consent(
    account: &str,
    client_id: String,
    client_secret: String,
    picker: bool,
    capture: &Capture,
) -> Result<()> {
    // The picker leg carries no PKCE of its own, so state is the only thing
    // pairing the callback with this attempt.
    let state = mecha_mail::google::auth::generate_pkce().code_verifier;
    let url = docs::build_auth_url(&client_id, capture.port, &state, picker);

    let redirect = if capture.paste {
        eprintln!("\nOpen this in any browser, on any machine:\n\n{url}\n");
        eprintln!(
            "It will finish on a 127.0.0.1 address that fails to load — that is\n\
             expected. Copy the whole address from the bar and paste it here:\n"
        );
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("reading the pasted redirect URL")?;
        docs::parse_redirect_url(line.trim())?
    } else {
        eprintln!(
            "\nOpen this to authorize (listening on 127.0.0.1:{}):\n\n{url}\n",
            capture.port
        );
        eprintln!(
            "Over SSH? Either forward the port:\n  \
             ssh -L {p}:127.0.0.1:{p} <this-host>\n\
             or re-run with --paste and no tunnel is needed.\n",
            p = capture.port
        );
        // Headless boxes have no DISPLAY, and xdg-open says so on stderr —
        // which reads as this command failing when it has not. The URL above
        // is the real interface; opening a browser is a convenience.
        let _ = std::process::Command::new("xdg-open")
            .arg(&url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        docs::wait_for_picker_redirect(capture.port, capture.timeout).await?
    };

    anyhow::ensure!(
        redirect.state == state,
        "state mismatch on the redirect — run the command again"
    );

    let tokens = docs::exchange_code(
        &client_id,
        &client_secret,
        &redirect.code,
        capture.port,
        &mecha_mail::http::client(),
    )
    .await?;

    let path = docs::store_path(account)?;
    let previous_account = token::load(&path).ok().and_then(|c| c.account);
    let mut creds = docs::credentials_from(client_id, client_secret, tokens, previous_account)?;

    // Name the account from Drive's own `about`, which answers under
    // `drive.file`. Best-effort on purpose: losing a completed sign-in over a
    // cosmetic label would make someone authorize twice, which is the rule
    // the Microsoft flow already learned.
    if creds.account.is_none() {
        let probe = token::TokenManager::with_credentials(path.clone(), creds.clone());
        match docs::DocsClient::new(probe).account_email().await {
            Ok(email) => creds.account = Some(email),
            Err(e) => tracing::debug!("could not read the account name: {e}"),
        }
    }
    token::save(&path, &creds)?;
    if let Some(who) = &creds.account {
        eprintln!("authorized as {who}");
    }

    eprintln!("credentials in {}", path.display());
    eprintln!(
        "granted scope: {}",
        creds.granted_scopes.as_deref().unwrap_or("(none reported)")
    );

    if !picker {
        eprintln!(
            "\nEverything mecha creates from now on is in scope automatically.\n\
             To let it edit a document that already exists: mecha-docs pick"
        );
        return Ok(());
    }

    if redirect.picked.is_empty() {
        eprintln!("\nNothing was picked. The grant was renewed; scope is unchanged.");
        return Ok(());
    }

    eprintln!("\nput {} item(s) in scope:", redirect.picked.len());
    let client = docs::DocsClient::new(token::TokenManager::load(path)?);
    let mut folders = 0;
    for id in &redirect.picked {
        match client.file(id).await {
            Ok(f) => {
                if f.mime_type == docs::MIME_FOLDER {
                    folders += 1;
                }
                eprintln!("  {:7} {}", docs::kind_of(&f.mime_type), f.name);
            }
            Err(e) => eprintln!("  {id}  (could not read back: {e})"),
        }
    }
    if folders > 0 {
        eprintln!(
            "\nNote: a picked folder is in scope as an object — the documents\n\
             inside it are not. Pick those individually."
        );
    }
    Ok(())
}

async fn list(account: &str) -> Result<()> {
    let path = docs::store_path(account)?;
    anyhow::ensure!(
        path.exists(),
        "no grant for account {account:?} — run `mecha-docs auth` first"
    );
    let client = docs::DocsClient::new(token::TokenManager::load(path)?);
    let files = client.list_scope().await?;
    if files.is_empty() {
        eprintln!(
            "nothing in scope yet. Documents mecha creates land here \
             automatically; `mecha-docs pick` adds existing ones."
        );
        return Ok(());
    }
    for f in &files {
        println!(
            "{:7}  {:44}  {}",
            docs::kind_of(&f.mime_type),
            f.name,
            f.modified_time.as_deref().unwrap_or("")
        );
    }
    // Read the owner live rather than backfilling the stored credential: a
    // read command that quietly writes is the "examination that heals what it
    // was about to report" shape doctor.rs is careful to avoid.
    let who = client.account_email().await.ok();
    match who {
        Some(email) => eprintln!(
            "\n{} file(s) in scope for account {account:?} ({email})",
            files.len()
        ),
        None => eprintln!("\n{} file(s) in scope for account {account:?}", files.len()),
    }
    Ok(())
}
