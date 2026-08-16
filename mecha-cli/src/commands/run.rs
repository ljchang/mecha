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
    let mut prepared = setup::prepare(global, interactive).await?;

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
        // Only a resumed run: a fresh one-shot's record is empty until the
        // run ends, so recall would be a dead spec in its prompt. Before the
        // config record, which captures the tool list for replay.
        if args.resume.is_some() {
            setup::register_recall(&mut prepared.agent, s);
        }
        s.append(&Record::Config(RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
        )))?;
        // Staged outbox items point back at the session that drafted them.
        if let Some(route) = &prepared.agent.context().outbox {
            route.set_session_id(&s.meta.id);
        }
        // One-shots are their own producer, `run` — addressable, though
        // rarely addressed. `run` does not set `global_config_only`, so its
        // resolved inbound default is Hold whether stdin is a terminal or a
        // pipe: a scripted `run --json` must not fold a stray message into a
        // task it has nothing to do with. Only the trigger runner accepts.
        if let Some(mb) = &prepared.mailbox {
            mb.attach("run", &s.meta.id);
        }
    }

    let user = Message::user(&prompt);
    convo.push(user.clone());
    if let Some(s) = &session {
        s.append(&Record::Message(user))?;
    }
    // Exactly what the file holds now, for the post-run reconcile: a run
    // that only appended gets its tail appended, one that rewrote history
    // (compaction) gets a rewrite record.
    let recorded = convo.messages.clone();

    let quiet = args.quiet || args.json;
    let events = if args.no_stream || args.json {
        None
    } else {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = render::spawn(
            rx,
            render::RenderOpts {
                verbose: global.verbose,
                quiet,
            },
        );
        Some((tx, handle))
    };

    let result = crate::interrupt::run_interruptible(
        &prepared.agent,
        prepared.agent.context(),
        &mut convo,
        events.as_ref().map(|(tx, _)| tx.clone()),
    )
    .await;

    // Closing the sender ends the render task; wait so its output lands before
    // anything we print below.
    if let Some((tx, handle)) = events {
        drop(tx);
        let _ = handle.await;
    }

    if let Some(s) = &session {
        // Before the result is inspected, so a run that died mid-flight still
        // leaves its turns on disk. `?` first is how a fatal 400 used to leave
        // a three-line transcript of a three-minute benchmark trial — and the
        // failed runs are precisely the transcripts that get read.
        s.record_run(&recorded, &convo)?;
        // Taint too, and for the same reason `chat` and `tui` record it: it
        // cannot be recovered by reading the transcript back, because it keys
        // off *provenance* and the transcript stores only content. `run`
        // supports `--resume`, so without this, resuming a one-shot that had
        // read a hostile page hands the model that page with the interlock
        // disarmed — the exact hole that was closed for the other two
        // front-ends and left open here.
        s.append(&Record::Taint(convo.taint))?;
    }

    let outcome = result?;
    if let Some(s) = &session {
        s.append(&Record::Summary {
            usage: outcome.usage.clone(),
            turns: outcome.turns,
        })?;
        if let Some(mb) = &prepared.mailbox {
            mb.detach(&s.meta.id);
        }
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

    // Before the exit-code match below: those paths leave the process.
    if let Some(s) = &session {
        let cx = prepared.agent.context();
        cx.hooks
            .session_end(&s.meta.id, &s.path, &cx.tools.workspace)
            .await;
    }

    // Distinct codes so a script can tell "the model refused" from "it
    // produced nothing" from "everything worked". Exhaustion alone is *not*
    // a failure code: a run that hit its turn or token ceiling still answered
    // and still left its work on disk, and callers that grade the artifact
    // treat non-zero as "the agent crashed" — on the 2026-08-07
    // Terminal-Bench subset, a MaxTurns trial was recorded as an agent error
    // while its verifier scored the work 1.0. A script that cares which
    // ceiling stopped the run has `--json`'s `stop_cause`; the exit code
    // answers the only question a caller can't get elsewhere: is there an
    // answer at all.
    match outcome.stop_reason {
        StopReason::Refusal => std::process::exit(2),
        _ if outcome.stop_cause == mecha_core::agent::StopCause::NoOutput => std::process::exit(3),
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
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("reading prompt from stdin")?;
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
