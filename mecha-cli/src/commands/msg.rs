//! `mecha msg` — the human surface over the inter-agent mailbox.
//!
//! Reading and sending both work whether or not `[messages] enabled` has
//! wired the agents up: the store is the user's own, and "what did the
//! overnight run tell me" must not depend on a feature flag. A message sent
//! from here is stamped `from: user` (or `--from`) with a clean taint —
//! typed by the person at the keyboard, the one sender whose words *are*
//! trusted input.

use anyhow::Result;
use mecha_core::agent::Taint;
use mecha_core::mailbox::{MailboxStore, SendOutcome};
use std::io::IsTerminal;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// Leave a message for an agent, by producer name.
    Send {
        /// Recipient: `chat`, a trigger's name, `run`.
        to: String,
        /// The message text.
        body: String,
        /// Sender name recorded on the message.
        #[arg(long, default_value = "user")]
        from: String,
        /// Id of the message this answers.
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Messages, pending first. Default recipient: all of them.
    List {
        /// Only this recipient's mailbox.
        #[arg(long)]
        to: Option<String>,
        /// Include delivered messages, not just pending.
        #[arg(long)]
        all: bool,
    },
    /// One message in full.
    Show { id: String },
    /// Set pending messages aside unread. A full mailbox refuses new sends,
    /// so a backlog nobody is coming to claim needs this, not `rm`.
    Dismiss {
        /// Message ids (or unique prefixes).
        #[arg(required_unless_present = "all")]
        ids: Vec<String>,
        /// Every pending message instead.
        #[arg(long, conflicts_with = "ids")]
        all: bool,
        /// With --all: only this recipient's mailbox.
        #[arg(long, requires = "all")]
        to: Option<String>,
    },
    /// Which agents are live right now, per the session markers.
    Agents,
}

pub async fn execute(args: Args) -> Result<()> {
    let store = open_store()?;
    match args.cmd {
        Cmd::Send {
            to,
            body,
            from,
            reply_to,
        } => send(&store, &to, &body, &from, reply_to),
        Cmd::List { to, all } => list(&store, to.as_deref(), all),
        Cmd::Show { id } => show(&store, &id),
        Cmd::Dismiss { ids, all, to } => dismiss(&store, &ids, all, to.as_deref()),
        Cmd::Agents => agents(&store),
    }
}

fn dismiss(store: &MailboxStore, ids: &[String], all: bool, to: Option<&str>) -> Result<()> {
    let ids: Vec<String> = if all {
        let recipients = match to {
            Some(r) => vec![r.to_string()],
            None => store.recipients()?,
        };
        recipients
            .iter()
            .map(|r| store.pending_for(r))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .map(|m| m.id)
            .collect()
    } else {
        ids.to_vec()
    };
    if ids.is_empty() {
        println!("nothing pending to dismiss");
        return Ok(());
    }
    for id in &ids {
        let m = store.dismiss(id)?;
        println!("dismissed {} ({} → {})", m.id, m.from, m.to);
    }
    Ok(())
}

/// The agents' store is configured in `[messages] dir`; this surface must
/// open the same one. Global config only, like the section itself.
pub(crate) fn open_store() -> Result<MailboxStore> {
    let cfg = mecha_core::config::Config::load_global()?;
    let root = match cfg.messages.dir {
        Some(dir) => dir,
        None => MailboxStore::default_root()?,
    };
    Ok(MailboxStore::open(root)?
        .with_limits(cfg.messages.pending_cap, cfg.messages.max_body_bytes))
}

fn send(
    store: &MailboxStore,
    to: &str,
    body: &str,
    from: &str,
    reply_to: Option<String>,
) -> Result<()> {
    // The one place a message's taint is *not* stamped by the loop, so it
    // fails closed instead. A person typing at a terminal is the one sender
    // whose words are trusted input — clean. Anything else (a pipe, a script,
    // or an agent's `shell` calling `mecha msg send` to route around the
    // harness stamp with `sandbox = "none"`) is untrusted: the receiver gets
    // the wrapper and the taint merge, exactly as message_send would give it.
    let taint = if std::io::stdin().is_terminal() {
        Taint::default()
    } else {
        Taint {
            private: true,
            untrusted: true,
        }
    };
    match store.send(to, from, None, body, reply_to, taint)? {
        SendOutcome::Sent(id) => {
            println!("sent {id} to `{to}`");
            // A live marker means a session exists, not that it will fold the
            // message — the recipient's inbound policy decides that, and this
            // side cannot read it. So report presence without claiming
            // delivery: a session that accepts folds it at its next turn, one
            // that holds surfaces it via `mecha msg list`.
            let live = store.agents()?.into_iter().any(|a| a.producer == to);
            if live {
                println!("`{to}` is running — it will see this at its next turn if it accepts inbound mail, otherwise on `mecha msg list`");
            } else {
                println!("`{to}` is not running — it waits for the next run");
            }
        }
        SendOutcome::Duplicate(id) => {
            println!("an identical message is already pending as {id}; nothing sent");
        }
    }
    Ok(())
}

fn list(store: &MailboxStore, to: Option<&str>, all: bool) -> Result<()> {
    let recipients = match to {
        Some(r) => vec![r.to_string()],
        None => store.recipients()?,
    };
    let mut any = false;
    for recipient in recipients {
        let msgs = store.messages_for(&recipient)?;
        for m in msgs {
            if !all && m.status != "pending" {
                continue;
            }
            any = true;
            let mark = if m.effective_taint().untrusted {
                " [untrusted]"
            } else {
                ""
            };
            println!(
                "{}  {:<9}  {} → {}{}  {}",
                m.id,
                m.status,
                m.from,
                m.to,
                mark,
                first_line(&m.body)
            );
        }
    }
    if !any {
        println!("no messages{}", if all { "" } else { " pending" });
    }
    Ok(())
}

fn show(store: &MailboxStore, id: &str) -> Result<()> {
    let m = store.message(id)?;
    println!("id        {}", m.id);
    println!("status    {}", m.status);
    println!(
        "from      {}{}",
        m.from,
        m.from_session
            .as_deref()
            .map(|s| format!(" (session {s})"))
            .unwrap_or_default()
    );
    println!("to        {}", m.to);
    println!("created   {}", m.created_at);
    if let Some(r) = &m.reply_to {
        println!("reply_to  {r}");
    }
    if let Some(d) = &m.dismissed_at {
        println!("dismissed {d}");
    }
    if let Some(d) = &m.delivered_at {
        println!(
            "delivered {d}{}",
            m.delivered_to
                .as_deref()
                .map(|s| format!(" to session {s}"))
                .unwrap_or_default()
        );
    }
    let t = m.effective_taint();
    if t.untrusted || t.private {
        println!(
            "taint     {}{}{}",
            if t.private { "private" } else { "" },
            if t.private && t.untrusted { " + " } else { "" },
            if t.untrusted {
                "untrusted — the sender had read third-party content; \
                 weigh the text accordingly"
            } else {
                ""
            }
        );
    }
    println!("\n{}", m.body);
    Ok(())
}

fn agents(store: &MailboxStore) -> Result<()> {
    let live = store.agents()?;
    if live.is_empty() {
        println!("no agents running");
        return Ok(());
    }
    for a in live {
        println!(
            "{:<20}  session {}  pid {}  since {}",
            a.producer, a.session_id, a.pid, a.started_at
        );
    }
    Ok(())
}

fn first_line(body: &str) -> String {
    let line = body.lines().next().unwrap_or("");
    if line.chars().count() > 60 {
        let head: String = line.chars().take(60).collect();
        format!("{head}…")
    } else {
        line.to_string()
    }
}
