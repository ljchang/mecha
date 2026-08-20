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
use mecha_mail::google::{docs, docs_server};
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
    command: Option<Command>,
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
    List {
        /// Machine-readable output, for a caller that is not a person.
        #[arg(long)]
        json: bool,
    },
    /// Serve MCP over stdio (the default when no subcommand is given).
    Serve,
}

/// How the redirect gets back here.
///
/// Four shapes, one browser leg. The last two are the same as `--paste` with
/// the waiting taken out: `--url` starts an attempt and stops, `--redirect`
/// finishes the one that was started. That split is what lets a front-end
/// drive the flow — a full-screen TUI cannot hand its keyboard to a child
/// that blocks on stdin, and neither can a script.
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
    #[arg(long, conflicts_with_all = ["url", "redirect"])]
    paste: bool,
    /// Print the authorization URL and stop, recording the attempt. Finish it
    /// afterwards — from anywhere, at any time — with `--redirect`.
    #[arg(long, conflicts_with = "redirect")]
    url: bool,
    /// Finish an attempt started with `--url`, from the address the browser
    /// landed on.
    #[arg(long, value_name = "URL")]
    redirect: Option<String>,
    /// Machine-readable output, for a caller that is not a person.
    #[arg(long)]
    json: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    mecha_mail::init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Auth {
            client_id,
            client_secret,
            client_json,
            capture,
        }) => {
            let (id, secret) = resolve_client(&cli.account, client_id, client_secret, client_json)?;
            consent(&cli.account, id, secret, false, &capture).await
        }
        Some(Command::Pick { capture }) => {
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
        Some(Command::List { json }) => list(&cli.account, json).await,
        Some(Command::Serve) | None => {
            let path = docs::store_path(&cli.account)?;
            anyhow::ensure!(
                path.exists(),
                "no grant for account {:?} — run `mecha-docs auth` first",
                cli.account
            );
            let manager = token::TokenManager::load(path)?;
            mecha_mail::mcp::serve(docs_server::DocsTools::new(manager)).await
        }
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
///
/// Three ways to receive the redirect and **one** way to start and one to
/// finish: `begin` mints the URL and records the attempt, `finish` verifies
/// and exchanges. Every mode below is those two with something different in
/// between, which is what stops `--paste` and `--url`/`--redirect` becoming
/// two flows that have to be kept in step.
async fn consent(
    account: &str,
    client_id: String,
    client_secret: String,
    picker: bool,
    capture: &Capture,
) -> Result<()> {
    // Finishing an attempt started earlier: nothing to mint, and the client
    // credentials come from the record rather than from these arguments —
    // which is also why `--redirect` needs no flags.
    if let Some(pasted) = &capture.redirect {
        let pending = docs::load_pending(account)?;
        let redirect = docs::parse_redirect_url(pasted.trim())?;
        return finish(account, &pending, redirect, capture.json).await;
    }

    let url = begin(account, client_id, client_secret, picker, capture)?;

    if capture.url {
        // Nothing else on stdout: a caller parsing this is a program.
        if capture.json {
            println!(
                "{}",
                serde_json::json!({ "url": url, "picker": picker, "account": account })
            );
        } else {
            println!("{url}");
        }
        eprintln!(
            "\nOpen that in any browser, on any machine. It finishes on a \n\
             127.0.0.1 address that fails to load — that is expected. Then:\n\n  \
             mecha-docs --account {account} {} --redirect '<that address>'\n",
            if picker { "pick" } else { "auth" }
        );
        return Ok(());
    }

    let pending = docs::load_pending(account)?;

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

    finish(account, &pending, redirect, capture.json).await
}

/// Mint the authorization URL and record the attempt. Returns the URL.
fn begin(
    account: &str,
    client_id: String,
    client_secret: String,
    picker: bool,
    capture: &Capture,
) -> Result<String> {
    // The picker leg carries no PKCE of its own, so state is the only thing
    // pairing the callback with this attempt.
    let state = mecha_mail::google::auth::generate_pkce().code_verifier;
    let url = docs::build_auth_url(&client_id, capture.port, &state, picker);
    docs::save_pending(
        account,
        &docs::PendingConsent {
            state,
            picker,
            client_id,
            client_secret,
            port: capture.port,
            started_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;
    Ok(url)
}

/// Verify the pairing, exchange the code, save the grant, and report what the
/// chooser put in scope.
async fn finish(
    account: &str,
    pending: &docs::PendingConsent,
    redirect: docs::PickerRedirect,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(
        redirect.state == pending.state,
        "state mismatch on the redirect — that address belongs to a different \
         attempt. Start again with --url."
    );

    let tokens = docs::exchange_code(
        &pending.client_id,
        &pending.client_secret,
        &redirect.code,
        pending.port,
        &mecha_mail::http::client(),
    )
    .await?;

    let path = docs::store_path(account)?;
    let previous_account = token::load(&path).ok().and_then(|c| c.account);
    let mut creds = docs::credentials_from(
        pending.client_id.clone(),
        pending.client_secret.clone(),
        tokens,
        previous_account,
    )?;

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
    // The attempt is spent. Only now: a record cleared before the exchange
    // would strand a redirect that arrives a second late with nothing to
    // verify against.
    docs::clear_pending(account);

    // Read the picked items back before printing anything, so the JSON and the
    // prose are built from the same answer.
    let mut picked: Vec<serde_json::Value> = Vec::new();
    let mut folders = 0;
    if pending.picker && !redirect.picked.is_empty() {
        let client = docs::DocsClient::new(token::TokenManager::load(path.clone())?);
        for id in &redirect.picked {
            match client.file(id).await {
                Ok(f) => {
                    if f.mime_type == docs::MIME_FOLDER {
                        folders += 1;
                    }
                    picked.push(serde_json::json!({
                        "id": f.id,
                        "name": f.name,
                        "kind": docs::kind_of(&f.mime_type),
                    }));
                }
                Err(e) => picked.push(serde_json::json!({
                    "id": id,
                    "name": format!("(could not read back: {e})"),
                    "kind": "file",
                })),
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "account": creds.account,
                "picker": pending.picker,
                "picked": picked,
                "folders": folders,
                "granted_scope": creds.granted_scopes,
            })
        );
        return Ok(());
    }

    if let Some(who) = &creds.account {
        eprintln!("authorized as {who}");
    }
    eprintln!("credentials in {}", path.display());
    eprintln!(
        "granted scope: {}",
        creds.granted_scopes.as_deref().unwrap_or("(none reported)")
    );

    if !pending.picker {
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
    for f in &picked {
        eprintln!(
            "  {:7} {}",
            f["kind"].as_str().unwrap_or("file"),
            f["name"].as_str().unwrap_or("")
        );
    }
    if folders > 0 {
        eprintln!(
            "\nNote: a picked folder is in scope as an object — the documents\n\
             inside it are not. Pick those individually."
        );
    }
    Ok(())
}

async fn list(account: &str, json: bool) -> Result<()> {
    let path = docs::store_path(account)?;
    anyhow::ensure!(
        path.exists(),
        "no grant for account {account:?} — run `mecha-docs auth` first"
    );
    let client = docs::DocsClient::new(token::TokenManager::load(path)?);
    let files = client.list_scope().await?;
    if json {
        // Empty is a legitimate answer and prints as one: a caller that gets
        // `[]` knows the grant works and holds nothing, which is a different
        // fact from an error.
        println!("{}", serde_json::to_string(&files)?);
        return Ok(());
    }
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
