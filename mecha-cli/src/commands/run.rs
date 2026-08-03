//! `mecha run` — one task, one answer.

use crate::{render, setup, GlobalOpts};
use anyhow::{Context, Result};
use mecha_core::message::{Message, StopReason};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use std::io::{IsTerminal, Read};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// The task. Omit it, or pass `-`, to read from stdin.
    pub prompt: Option<String>,

    /// Emit a single JSON object instead of prose. Implies --quiet.
    #[arg(long)]
    pub json: bool,

    /// Print only the answer — no tool narration.
    #[arg(long)]
    pub quiet: bool,

    /// Wait for the whole answer instead of streaming it.
    #[arg(long)]
    pub no_stream: bool,

    /// Continue a saved session by id or unique prefix.
    #[arg(long)]
    pub resume: Option<String>,

    /// Don't write a transcript.
    #[arg(long)]
    pub no_session: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let prompt = read_prompt(args.prompt.as_deref())?;
    anyhow::ensure!(!prompt.trim().is_empty(), "no prompt given");

    // Nothing can answer an approval prompt when output is being piped or
    // parsed, so those runs use the configured permission mode instead.
    let interactive = std::io::stdin().is_terminal() && !args.json;
    let prepared = setup::prepare(global, interactive).await?;

    let session_dir = Session::default_dir()?;
    let mut convo = mecha_core::agent::Conversation::new();
    let mut session = None;

    if let Some(id) = &args.resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        convo = prior;
        session = Some(Session { meta, path });
    } else if !args.no_session {
        session = Some(Session::create(
            &session_dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: prepared.provider_name.clone(),
                model: prepared.model.clone(),
                workspace: prepared.workspace.clone(),
                title: Some(first_words(&prompt)),
            },
        )?);
    }

    // Written on create *and* on resume: a session picked up under different
    // flags is exactly the case this record exists to catch.
    if let Some(s) = &session {
        s.append(&Record::Config(RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
        )))?;
    }

    let user = Message::user(&prompt);
    convo.push(user.clone());
    if let Some(s) = &session {
        s.append(&Record::Message(user))?;
    }
    let history_len = convo.len();

    let quiet = args.quiet || args.json;
    let events = if args.no_stream || args.json {
        None
    } else {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = render::spawn(rx, render::RenderOpts { verbose: global.verbose, quiet });
        Some((tx, handle))
    };

    let outcome = crate::interrupt::run_interruptible(
        &prepared.agent,
        prepared.agent.context(),
        &mut convo,
        events.as_ref().map(|(tx, _)| tx.clone()),
    )
    .await?;

    // Closing the sender ends the render task; wait so its output lands before
    // anything we print below.
    if let Some((tx, handle)) = events {
        drop(tx);
        let _ = handle.await;
    }

    if let Some(s) = &session {
        s.append_messages(&convo.messages[history_len..])?;
        // Taint too, and for the same reason `chat` and `tui` record it: it
        // cannot be recovered by reading the transcript back, because it keys
        // off *provenance* and the transcript stores only content. `run`
        // supports `--resume`, so without this, resuming a one-shot that had
        // read a hostile page hands the model that page with the interlock
        // disarmed — the exact hole that was closed for the other two
        // front-ends and left open here.
        s.append(&Record::Taint(convo.taint))?;
        s.append(&Record::Summary { usage: outcome.usage.clone(), turns: outcome.turns })?;
    }

    if args.json {
        let value = serde_json::json!({
            "text": outcome.text,
            "stop_reason": outcome.stop_reason,
            "turns": outcome.turns,
            "exhausted": outcome.exhausted,
            "stop_cause": outcome.stop_cause,
            "cost_usd": outcome.cost_usd,
            "refusal": outcome.refusal,
            "usage": outcome.usage,
            "model": prepared.model,
            "provider": prepared.provider_name,
            "session": session.as_ref().map(|s| s.meta.id.clone()),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if args.no_stream {
        println!("{}", outcome.text);
    }

    if !quiet && !args.json {
        if let Some(s) = &session {
            eprintln!("\nsession {}", s.meta.id);
        }
    }

    // Distinct codes so a script can tell "the model refused" from "it ran out
    // of turns" from "everything worked".
    match outcome.stop_reason {
        StopReason::Refusal => std::process::exit(2),
        _ if outcome.exhausted => std::process::exit(3),
        _ => Ok(()),
    }
}

fn read_prompt(arg: Option<&str>) -> Result<String> {
    match arg {
        Some("-") | None => {
            if std::io::stdin().is_terminal() && arg.is_none() {
                anyhow::bail!("no prompt given (pass one as an argument, or pipe it on stdin)");
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf).context("reading prompt from stdin")?;
            Ok(buf)
        }
        Some(text) => Ok(text.to_string()),
    }
}

/// A short label for `sessions list`.
fn first_words(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 60 {
        format!("{}…", flat.chars().take(60).collect::<String>())
    } else {
        flat
    }
}
