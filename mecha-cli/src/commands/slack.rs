//! `mecha slack` — the credential, the binding, and who may drive.
//!
//! This is step 2 of `docs/SLACK-DESIGN.md`: everything up to and including the
//! security boundary, and nothing that runs an agent. A bound app that answers
//! *"I heard you, and you are the owner"* is worth landing on its own, because
//! it is the part that has to be right before anything else is safe to build.
//!
//! The verbs split by what they touch:
//!
//! - `auth` stores the two tokens, having proved them against Slack first.
//! - `link` mints a nonce, prints it **here**, and binds whoever types it into
//!   Slack. Typing a code that was printed on this machine proves shell access
//!   to the machine the agent runs on; an email address proves only what the
//!   workspace claims about it.
//! - `status` says what is bound and whether the credential still works.
//! - `unlink` removes the binding and keeps the tokens, because losing the
//!   binding is the recoverable half.
//!
//! The tokens come from the environment rather than from flags on purpose: a
//! flag lands in shell history and in `ps` output, and a Slack bot token is a
//! credential that reaches the whole workspace.

use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use mecha_slack::binding::{Binding, Credentials, PendingLink, SlackStore};
use mecha_slack::envelope::Inbound;
use mecha_slack::{chat, Slack, SocketMode, SocketOptions};

use crate::slack::threads::{ThreadState, ThreadStore};
use crate::GlobalOpts;
use serde_json::{json, Value};

const BOT_TOKEN_ENV: &str = "MECHA_SLACK_BOT_TOKEN";
const APP_TOKEN_ENV: &str = "MECHA_SLACK_APP_TOKEN";

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What is bound, and whether the credential still works (default).
    Status,
    /// Store the bot and app-level tokens, after proving them against Slack.
    ///
    /// Reads them from `MECHA_SLACK_BOT_TOKEN` and `MECHA_SLACK_APP_TOKEN`
    /// rather than from flags, so neither reaches shell history or `ps`.
    Auth,
    /// Print a one-time code and bind whoever sends it to the app in Slack.
    Link {
        /// Give up after this many minutes.
        #[arg(long, default_value_t = 10)]
        timeout: i64,
        /// Bind even if this install is already bound to another workspace.
        #[arg(long)]
        force: bool,
    },
    /// What state each Slack thread is in, and what would resolve it.
    Threads {
        /// Only threads in this state: `idle`, `running`, `awaiting_input`,
        /// `cancelled`, `staged`, `done`, `failed`, `orphaned`.
        #[arg(long)]
        state: Option<String>,
    },
    /// Run the connector: hold the Slack socket open and drive runs from
    /// threads. This is what the systemd unit runs.
    Connect,
    /// Mark threads whose run did not survive a restart, so none is left
    /// showing "working…" forever. The connector does this on startup; this is
    /// the same pass, by hand.
    Sweep,
    /// Read stdin and send it to the owner as a DM.
    ///
    /// What a trigger's `notify` calls: it already runs `sh -c` with the run's
    /// answer on stdin, so `notify = "mecha slack notify"` puts the morning
    /// briefing on a phone with no new trigger concept at all.
    Notify {
        /// A line above the message, for saying which trigger sent it.
        #[arg(long)]
        title: Option<String>,
    },
    /// Send a file to the owner's DM.
    ///
    /// The way something a headless box made gets looked at: a chart, a log, a
    /// screenshot. Rung 1 of `docs/REMOTE-CONTROL-DESIGN.md`.
    Send {
        /// The file to send.
        path: std::path::PathBuf,
        /// A line above it in the DM.
        #[arg(long)]
        comment: Option<String>,
    },
    /// Named threads a TUI session has been mirrored into, and their state.
    ///
    /// The `threads` verb's counterpart for the other kind of thread: those
    /// are driven from Slack, these mirror a terminal session.
    Remote {
        /// Mark cold every attachment whose session is gone, so none is left
        /// reading `live` for a process that has died.
        #[arg(long)]
        sweep: bool,
    },
    /// Forget the binding. The tokens stay, so `link` can be run again.
    Unlink,
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    let store = open_store()?;
    match args.cmd.unwrap_or(Cmd::Status) {
        Cmd::Status => status(&store).await,
        Cmd::Auth => auth(&store).await,
        Cmd::Link { timeout, force } => link(&store, timeout, force).await,
        Cmd::Threads { state } => threads(state.as_deref()),
        Cmd::Sweep => sweep(),
        Cmd::Notify { title } => notify(&store, title.as_deref()).await,
        Cmd::Send { path, comment } => send(&path, comment.as_deref()).await,
        Cmd::Remote { sweep } => remote(sweep),
        Cmd::Connect => crate::slack::connector::run(global).await,
        Cmd::Unlink => {
            store.clear_binding()?;
            store.clear_pending_link()?;
            println!("Unbound. The tokens are still stored; `mecha slack link` re-binds.");
            Ok(())
        }
    }
}

fn open_store() -> Result<SlackStore> {
    let root = mecha_core::work::mecha_home()?.join("slack");
    Ok(SlackStore::open(root)?)
}

fn credentials(store: &SlackStore) -> Result<Credentials> {
    store
        .credentials()?
        .context("no Slack tokens stored — run `mecha slack auth` first")
}

async fn auth(store: &SlackStore) -> Result<()> {
    let bot_token = std::env::var(BOT_TOKEN_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .with_context(|| format!("set {BOT_TOKEN_ENV} (the `xoxb-` bot token)"))?;
    let app_token = std::env::var(APP_TOKEN_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .with_context(|| format!("set {APP_TOKEN_ENV} (the `xapp-` app-level token)"))?;

    // Shape check before the network call, because `not_authed` on a token
    // pasted into the wrong variable is a confusing way to learn that.
    if !bot_token.starts_with("xoxb-") {
        bail!("{BOT_TOKEN_ENV} does not look like a bot token (expected it to start with `xoxb-`)");
    }
    if !app_token.starts_with("xapp-") {
        bail!("{APP_TOKEN_ENV} does not look like an app-level token (expected `xapp-`)");
    }

    // Prove the credential before storing it. Writing an unverified token means
    // discovering it is wrong at the first real run, hours later.
    let slack = Slack::new(bot_token.trim());
    let who: Value = match slack.call::<Value>("auth.test", json!({})).await {
        Ok(who) => who,
        Err(e) => bail!("{}", explain(&e)),
    };

    store.save_credentials(&Credentials {
        bot_token: bot_token.trim().to_string(),
        app_token: app_token.trim().to_string(),
    })?;

    println!(
        "Stored. Bot `{}` in workspace `{}` ({}).",
        who["user"].as_str().unwrap_or("?"),
        who["team"].as_str().unwrap_or("?"),
        who["team_id"].as_str().unwrap_or("?")
    );
    println!("Next: `mecha slack link` to say who may drive this agent.");
    Ok(())
}

/// Turn a Slack error code into something a person can act on.
///
/// Written the first time one was hit for real: `account_inactive` is
/// correctly classified, correctly terminal, and tells the reader nothing
/// about what to do — which makes the classification worth exactly as much as
/// the message beside it.
fn explain(e: &mecha_slack::SlackError) -> String {
    let hint = match e {
        mecha_slack::SlackError::Auth { code, .. } => match code.as_str() {
            "account_inactive" => Some(
                "the app no longer exists in that workspace — it was deleted, or its \
                 installation was removed. Check https://api.slack.com/apps, reinstall, \
                 and copy the fresh Bot User OAuth Token",
            ),
            "invalid_auth" | "not_authed" => Some(
                "the token was not accepted at all. Check it was copied whole, and that \
                 it is the Bot User OAuth Token (`xoxb-`) rather than an app-level token",
            ),
            "token_revoked" => Some("the token was revoked — generate a new one and reinstall"),
            "missing_scope" => Some(
                "the app is installed but lacks a scope this call needs. Add it under \
                 OAuth & Permissions, then reinstall — scope changes need a reinstall",
            ),
            _ => None,
        },
        _ => None,
    };
    match hint {
        Some(hint) => format!("{e}\n\n  {hint}."),
        None => e.to_string(),
    }
}

/// Every thread and where it stands.
///
/// Prints what resolves each state beside it, because the reason the state
/// machine carries that string is so a person looking at a stuck thread can see
/// the way out without reading the source.
fn threads(filter: Option<&str>) -> Result<()> {
    // A filter naming no state matches nothing and looks identical to "there
    // are none", so it is refused rather than applied.
    if let Some(f) = filter {
        if !ThreadState::ALL.iter().any(|s| s.as_str() == f) {
            let valid: Vec<_> = ThreadState::ALL.iter().map(|s| s.as_str()).collect();
            bail!("no such state `{f}`. Valid: {}", valid.join(", "));
        }
    }

    let store = ThreadStore::open(thread_root()?)?;
    let all = store.all()?;
    let shown: Vec<_> = all
        .iter()
        .filter(|r| filter.is_none_or(|f| r.state.as_str() == f))
        .collect();

    if shown.is_empty() {
        println!(
            "No threads{}.",
            filter.map(|f| format!(" in {f}")).unwrap_or_default()
        );
        println!("store    {}", store.root().display());
        return Ok(());
    }

    for record in shown {
        println!(
            "{}  {}  mode={}  {}",
            record.key,
            record.state.as_str(),
            record.mode,
            record.updated_at.format("%Y-%m-%d %H:%M UTC")
        );
        println!("    {}", record.state.describe());
        println!("    resolved by: {}", record.state.resolved_by());
        if let Some(session) = &record.session_id {
            println!("    session {session}");
        }
    }
    Ok(())
}

/// Send whatever arrives on stdin to the first bound owner.
///
/// Deliberately a DM and never a channel: this is mecha talking to its owner,
/// and a DM's recipient is the principal, so it is not a send sink in the way
/// a channel post would be.
async fn notify(store: &SlackStore, title: Option<&str>) -> Result<()> {
    // The credential is checked before stdin is read and the DM is opened
    // after — the order matters, and it is why `owner_client` and `open_dm`
    // are separate. A trigger that produced no answer must cost no round trip,
    // and one with a broken binding must still say so.
    let (slack, owner) = crate::slack::send::owner_client(store)?;

    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)
        .context("reading the message from stdin")?;
    let body = body.trim();
    if body.is_empty() {
        // Nothing to say is not a failure; a trigger that produced no answer
        // should not look like one that broke.
        return Ok(());
    }

    let channel = crate::slack::send::open_dm(&slack, &owner).await?;
    let text = match title {
        Some(t) => format!("*{t}*\n{body}"),
        None => body.to_string(),
    };
    chat::post_message(&slack, &channel, None, &text, None).await?;
    Ok(())
}

/// `mecha slack send` — the CLI half of the TUI's `/send`.
///
/// The path is taken as typed, where the TUI's `/send` puts it through the
/// run's path jail. That difference is deliberate rather than an oversight:
/// this verb runs in the user's own shell, which is already the boundary, and
/// a jail here would refuse to send a file the person is standing next to.
async fn send(path: &std::path::Path, comment: Option<&str>) -> Result<()> {
    let sent = crate::slack::send::send_file(path, comment).await?;
    println!(
        "sent {} ({}) to your Slack DM",
        sent.filename,
        crate::slack::send::human(sent.bytes)
    );
    Ok(())
}

/// What is mirrored where, and the by-hand cold pass.
///
/// A store read and a pid check: no network, no model, no token. Being able to
/// answer "what is this machine mirroring" without talking to Slack is what
/// makes it usable when Slack is the thing that is wrong.
fn remote(sweep: bool) -> Result<()> {
    let store = crate::slack::remote::RemoteStore::open_default()?;
    if sweep {
        let cooled = store.sweep()?;
        if cooled.is_empty() {
            println!("Nothing to sweep — no attachment names a process that has gone.");
        } else {
            for rec in &cooled {
                println!("{}  cooled  (was session {})", rec.name, rec.session_id);
            }
        }
        return Ok(());
    }

    let records = store.list()?;
    if records.is_empty() {
        println!("No named threads yet — `/remote-control <name>` in the TUI makes one.");
        return Ok(());
    }
    for rec in &records {
        // Liveness is re-checked on read rather than trusted from the file: a
        // record says what was true when it was written, and the process it
        // names may have gone since without anything getting the chance to
        // record it.
        let state = if rec.is_live() { "live" } else { "cold" };
        println!(
            "{:<16} {:<5} {}  {}",
            rec.name,
            state,
            rec.session_id,
            rec.workspace.display()
        );
        if let Some(reason) = &rec.ended_reason {
            println!("{:<16} {:<5} {reason}", "", "");
        }
    }
    Ok(())
}

/// Orphan whatever was mid-flight when the process holding it died.
fn sweep() -> Result<()> {
    let store = ThreadStore::open(thread_root()?)?;
    let orphaned = store.sweep()?;
    if orphaned.is_empty() {
        println!("Nothing to sweep — no thread is mid-flight without a live run.");
        return Ok(());
    }
    for record in &orphaned {
        println!("{}  orphaned  ({})", record.key, record.state.describe());
    }
    println!(
        "\n{} thread(s) marked. The connector announces these in Slack; \
         until it runs, they are visible here.",
        orphaned.len()
    );
    Ok(())
}

fn thread_root() -> Result<std::path::PathBuf> {
    Ok(mecha_core::work::mecha_home()?
        .join("slack")
        .join("threads"))
}

async fn status(store: &SlackStore) -> Result<()> {
    println!("store    {}", store.root().display());

    match store.credentials()? {
        None => println!("tokens   none — run `mecha slack auth`"),
        Some(creds) => {
            let slack = Slack::new(&creds.bot_token);
            match slack.call::<Value>("auth.test", json!({})).await {
                Ok(who) => println!(
                    "tokens   ok — bot `{}` in `{}`",
                    who["user"].as_str().unwrap_or("?"),
                    who["team"].as_str().unwrap_or("?")
                ),
                Err(e) => println!("tokens   stored, but Slack refused them: {e}"),
            }
        }
    }

    match store.binding()? {
        None => println!("binding  none — run `mecha slack link`"),
        Some(b) => {
            println!(
                "binding  workspace {} · bound {}",
                b.team_id,
                b.bound_at.format("%Y-%m-%d %H:%M UTC")
            );
            for owner in &b.owners {
                println!("owner    {owner}");
            }
        }
    }

    if let Some(pending) = store.pending_link()? {
        if pending.is_live(Utc::now()) {
            println!("pending  a link code is live and waiting");
        }
    }
    Ok(())
}

async fn link(store: &SlackStore, timeout_minutes: i64, force: bool) -> Result<()> {
    let creds = credentials(store)?;
    let existing = store.binding()?;

    let pending = PendingLink::mint(Duration::minutes(timeout_minutes));
    store.save_pending_link(&pending)?;

    println!("Send this code to the app in a Slack DM:\n");
    println!("    {}\n", pending.nonce);
    println!(
        "It is good for {timeout_minutes} minutes and can be used once. \
         Waiting… (Ctrl-C to stop)"
    );

    let slack = Slack::new(&creds.bot_token);
    let socket = SocketMode::new(
        slack.clone(),
        SocketOptions {
            app_token: creds.app_token.clone(),
            debug_reconnects: false,
        },
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let driver = tokio::spawn(async move { socket.run(tx, || false).await });

    let deadline = std::time::Duration::from_secs((timeout_minutes.max(1) * 60) as u64);
    let outcome = tokio::time::timeout(deadline, async {
        while let Some(inbound) = rx.recv().await {
            let Inbound::Event { event, .. } = inbound else {
                continue;
            };
            // Only a direct message, and only from a person. A code posted in a
            // channel would bind whoever repeated it there.
            if event.kind != "message"
                || event.channel_type.as_deref() != Some("im")
                || !event.is_from_a_human()
            {
                continue;
            }
            let Some(text) = event.text.as_deref() else {
                continue;
            };
            if !pending.matches(text) {
                continue;
            }
            if !pending.is_live(Utc::now()) {
                return Err(anyhow::anyhow!("that code had already expired"));
            }
            let (Some(user), Some(team)) = (event.user.clone(), event.team_id.clone()) else {
                continue;
            };
            return Ok((user, team, event.channel.clone()));
        }
        Err(anyhow::anyhow!("the Slack connection closed"))
    })
    .await;

    driver.abort();

    let (user, team, channel) = match outcome {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => {
            store.clear_pending_link()?;
            return Err(e);
        }
        Err(_) => {
            store.clear_pending_link()?;
            bail!("nobody sent the code within {timeout_minutes} minutes");
        }
    };

    let binding = match existing {
        Some(mut b) if b.team_id == team => {
            // Idempotent, and the way a second device or a second account for
            // the same person is added.
            if !b.owners.contains(&user) {
                b.owners.push(user.clone());
            }
            b
        }
        Some(b) if !force => {
            store.clear_pending_link()?;
            bail!(
                "this install is bound to workspace {} and the code came from {team}. \
                 Re-run with --force to rebind, which drops the existing owners.",
                b.team_id
            );
        }
        _ => Binding {
            team_id: team.clone(),
            enterprise_id: None,
            owners: vec![user.clone()],
            bound_at: Utc::now(),
        },
    };

    store.save_binding(&binding)?;
    store.clear_pending_link()?;

    println!("\nBound: {user} in workspace {team}.");

    // Say so in Slack too. The boundary working is only convincing from the
    // side the human is standing on.
    if let Some(channel) = channel {
        let _ = chat::post_message(
            &slack,
            &channel,
            None,
            "Linked. I heard you, and you are the owner of this agent.",
            None,
        )
        .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::explain;
    use mecha_slack::SlackError;

    fn auth(code: &str) -> SlackError {
        SlackError::Auth {
            method: "auth.test".into(),
            code: code.into(),
        }
    }

    #[test]
    fn the_codes_a_person_actually_hits_say_what_to_do() {
        // `account_inactive` is the one that was hit for real, and the one a
        // reader cannot possibly act on from its name alone.
        let text = explain(&auth("account_inactive"));
        assert!(
            text.contains("account_inactive"),
            "keep Slack's own code: {text}"
        );
        assert!(text.contains("reinstall"), "and say what to do: {text}");

        for code in [
            "invalid_auth",
            "not_authed",
            "token_revoked",
            "missing_scope",
        ] {
            assert!(
                explain(&auth(code)).lines().count() > 1,
                "{code} has no hint"
            );
        }
    }

    #[test]
    fn an_unrecognised_code_is_passed_through_rather_than_guessed_at() {
        // Inventing advice for a code we have not seen is worse than none.
        let text = explain(&auth("some_future_code"));
        assert!(text.contains("some_future_code"));
        assert_eq!(text.lines().count(), 1, "no invented hint: {text}");
    }
}
