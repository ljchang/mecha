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
    let who: Value = slack
        .call("auth.test", json!({}))
        .await
        .context("the bot token was refused by Slack")?;

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
