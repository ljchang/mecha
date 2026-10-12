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

use anyhow::{bail, Context, Result};
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
            let store = docs::store_path(&cli.account)?;
            let (id, secret) = resolve_client(&store, client_id, client_secret, client_json)?;
            consent(&cli.account, id, secret, false, &capture).await
        }
        Some(Command::Pick { capture }) => {
            let stored = token::load(&docs::store_path(&cli.account)?).with_context(|| {
                format!(
                    "no grant for account {:?} — run `mecha-docs auth` first",
                    cli.account
                )
            })?;
            // The shipped client picks with this build's secret, as it signs
            // in and refreshes (`builtin::secret_for`; review of #663).
            let secret = mecha_mail::builtin::secret_for(&stored.client_id)
                .map(str::to_string)
                .unwrap_or_else(|| stored.client_secret.clone());
            consent(
                &cli.account,
                stored.client_id.clone(),
                secret,
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

/// Resolve client credentials: a downloaded client JSON, then explicit flags,
/// then whatever the stored grant already used — so re-authenticating after a
/// revoked grant needs no flags, the rule the mail binaries already follow —
/// and last mecha's own Drive client, so a first sign-in needs nothing.
///
/// `path` is the account's grant (`docs::store_path`), passed in so every arm
/// is testable against a scratch directory.
fn resolve_client(
    path: &std::path::Path,
    client_id: Option<String>,
    client_secret: Option<String>,
    client_json: Option<std::path::PathBuf>,
) -> Result<(String, String)> {
    if let Some(path) = client_json {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        return mecha_mail::wizard::read_google_client_json(&raw)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()));
    }
    // An explicit id, then the stored grant's own client (a re-sign-in keeps
    // its registration), then mecha's own.
    let stored = || {
        token::load(path).with_context(|| {
            format!(
                "{} is there but does not load, so whose client it was cannot be said — and \
                 a grant under another client would drop every picked document out of reach; \
                 move it aside, or pass --client-json",
                path.display()
            )
        })
    };
    if let Some(id) = client_id {
        // The halves of one client, never two: an id with no secret takes the
        // stored grant's secret only when the grant is that same client, and
        // otherwise is refused here rather than as `invalid_client` after the
        // whole browser leg (review of #663).
        return match client_secret {
            Some(secret) => Ok((id, secret)),
            None if path.exists() => {
                let s = stored()?;
                anyhow::ensure!(
                    s.client_id == id,
                    "--client-id needs its --client-secret (or use --client-json)"
                );
                // The shipped client's secret is this build's, here as on
                // every other path (`builtin::secret_for`; review of #663).
                let secret = mecha_mail::builtin::secret_for(&s.client_id)
                    .map(str::to_string)
                    .unwrap_or(s.client_secret);
                Ok((id, secret))
            }
            None => bail!("--client-id needs its --client-secret (or use --client-json)"),
        };
    }
    // A store that exists is never passed over: one that will not load is a
    // refusal, not a first sign-in (review of #663).
    if path.exists() {
        let s = stored()?;
        // The shipped client signs in again with this build's secret, so a
        // rotation reaches a re-auth too (`builtin::secret_for`).
        let secret = mecha_mail::builtin::secret_for(&s.client_id)
            .map(str::to_string)
            .unwrap_or(s.client_secret);
        return Ok((s.client_id, secret));
    }
    let builtin = mecha_mail::builtin::GOOGLE_DOCS;
    Ok((builtin.id.to_string(), builtin.secret.to_string()))
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
        // The URL above is the real interface; opening a browser is a
        // convenience, and on macOS it is `open`, not `xdg-open`.
        mecha_mail::builtin::open_browser(&url);
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
    let (url, pending) = mint(client_id, client_secret, picker, capture.port);
    docs::save_pending(account, &pending)?;
    Ok(url)
}

/// The authorization URL and the record that will finish it. PKCE, so a code
/// seen in transit is worth nothing without the verifier this attempt keeps:
/// the client's id and secret are public (review of #663). State pairs the
/// callback with the attempt besides, and is a separate draw.
fn mint(
    client_id: String,
    client_secret: String,
    picker: bool,
    port: u16,
) -> (String, docs::PendingConsent) {
    let pkce = mecha_mail::google::auth::generate_pkce();
    let state = mecha_mail::google::auth::generate_pkce().code_verifier;
    let url = docs::build_auth_url(&client_id, port, &state, picker, &pkce.code_challenge);
    (
        url,
        docs::PendingConsent {
            state,
            picker,
            client_id,
            client_secret,
            port,
            started_at: chrono::Utc::now().to_rfc3339(),
            code_verifier: pkce.code_verifier,
        },
    )
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
        &pending.code_verifier,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The verifier the record keeps is the one whose S256 is in the URL,
    /// and the state is a different draw — a swap here breaks every Docs
    /// sign-in at the exchange with nothing else failing (review of #663).
    #[test]
    fn the_kept_verifier_is_the_one_the_url_challenges() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use sha2::{Digest, Sha256};
        for picker in [false, true] {
            let (url, pending) = mint(
                "c.apps.googleusercontent.com".into(),
                "s".into(),
                picker,
                8765,
            );
            let challenge = url
                .split(['?', '&'])
                .find_map(|kv| kv.strip_prefix("code_challenge="))
                .expect("a challenge in the URL");
            let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pending.code_verifier.as_bytes()));
            assert_eq!(challenge, expected);
            assert_ne!(pending.state, pending.code_verifier);
            assert_eq!(pending.picker, picker);
        }
    }

    fn grant(dir: &std::path::Path, id: &str, secret: &str) -> std::path::PathBuf {
        let path = dir.join("oauth.json");
        let creds = token::StoredCredentials {
            client_id: id.into(),
            client_secret: secret.into(),
            tenant: None,
            access_token: String::new(),
            refresh_token: "r".into(),
            expires_at: 0,
            account: None,
            granted_scopes: None,
            granted_at: None,
        };
        std::fs::write(&path, serde_json::to_string(&creds).unwrap()).unwrap();
        path
    }

    /// Every arm of the ladder, against a scratch store: none goes to the
    /// shipped client over a grant that exists, a grant that will not load
    /// is refused, an id needs its secret unless the grant is that client,
    /// and the shipped client always signs in with this build's secret.
    #[test]
    fn the_client_ladder_fails_closed() {
        let dir = std::env::temp_dir().join(format!("mecha-docs-ladder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shipped = mecha_mail::builtin::GOOGLE_DOCS;

        // No store: the shipped client.
        let none = dir.join("absent.json");
        assert_eq!(
            resolve_client(&none, None, None, None).unwrap(),
            (shipped.id.to_string(), shipped.secret.to_string())
        );
        // An id without its secret and no store: refused up front.
        assert!(resolve_client(&none, Some("x".into()), None, None).is_err());

        // A grant under the user's own client is kept, never replaced.
        let own = grant(&dir, "own.apps.googleusercontent.com", "GOCSPX-own");
        assert_eq!(
            resolve_client(&own, None, None, None).unwrap(),
            ("own.apps.googleusercontent.com".into(), "GOCSPX-own".into())
        );
        // Its id alone takes its own stored secret; another id alone is refused.
        assert_eq!(
            resolve_client(
                &own,
                Some("own.apps.googleusercontent.com".into()),
                None,
                None
            )
            .unwrap()
            .1,
            "GOCSPX-own"
        );
        assert!(resolve_client(&own, Some("other".into()), None, None).is_err());

        // A grant under the shipped client signs in with this build's secret,
        // by either arm, however stale the stored copy.
        let old = grant(&dir, shipped.id, "GOCSPX-stale");
        assert_eq!(
            resolve_client(&old, None, None, None).unwrap().1,
            shipped.secret
        );
        assert_eq!(
            resolve_client(&old, Some(shipped.id.into()), None, None)
                .unwrap()
                .1,
            shipped.secret
        );

        // A grant that will not load is a refusal, not a first sign-in.
        std::fs::write(dir.join("oauth.json"), "{ torn").unwrap();
        let err = resolve_client(&dir.join("oauth.json"), None, None, None).unwrap_err();
        assert!(format!("{err:#}").contains("does not load"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
