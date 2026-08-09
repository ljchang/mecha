//! `mecha chat` — an interactive session in the terminal.

use crate::{render, setup, GlobalOpts};
use anyhow::Result;
use mecha_core::agent::Conversation;
use mecha_core::message::{Message, Usage};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Continue a saved session by id or unique prefix.
    #[arg(long)]
    pub resume: Option<String>,

    /// Don't write a transcript.
    #[arg(long)]
    pub no_session: bool,
}

pub async fn execute(global: &GlobalOpts, args: Args) -> Result<()> {
    let prepared = setup::prepare(global, true).await?;
    let session_dir = Session::default_dir()?;

    // One conversation for the whole session: the taint travels with it, so a
    // hostile page read on turn one still arms the interlock on turn five.
    let mut convo = Conversation::new();
    let mut session = None;

    if let Some(id) = &args.resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        println!("resumed {} ({} messages)", meta.id, prior.messages.len());
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
                title: None,
            },
        )?);
    }

    // On create and on resume both: a session picked up under different flags
    // is exactly what this record exists to catch.
    if let Some(s) = &session {
        s.append(&Record::Config(RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
        )))?;
        // Staged outbox items point back at the session that drafted them.
        if let Some(route) = &prepared.agent.context().outbox {
            route.set_session_id(&s.meta.id);
        }
        // The interactive surface is one producer, `chat`, whichever session
        // is live — that is what lets an overnight trigger address it without
        // knowing which session tomorrow brings. A `--no-session` chat stays
        // anonymous: no identity, no mailbox, no return address.
        if let Some(mb) = &prepared.mailbox {
            mb.set_identity("chat", &s.meta.id);
            if let Err(e) = mb.store.announce("chat", &s.meta.id) {
                tracing::warn!("could not announce chat session: {e:#}");
            }
            // Attended surfaces default to `hold`: say what is waiting
            // rather than folding it in unasked.
            if !mb.delivers() {
                match mb.store.pending_for("chat") {
                    Ok(pending) if !pending.is_empty() => println!(
                        "{} message(s) waiting — `mecha msg list` to read them",
                        pending.len()
                    ),
                    _ => {}
                }
            }
        }
    }

    println!(
        "mecha {} · {} ({}) · {} tools · {}",
        mecha_core::VERSION,
        prepared.model,
        prepared.provider_name,
        prepared.agent.registry().len(),
        prepared.workspace.display()
    );
    println!("/help for commands, Ctrl-D to exit.\n");

    let mut editor = DefaultEditor::new()?;
    // History survives the process, and it lives beside the transcripts on
    // purpose: the sessions directory is owner-only, and a typed prompt
    // deserves the same protection as the transcript that records it.
    // Best-effort throughout — a first run has no file, and losing history
    // must never lose the chat.
    let history_path = {
        let _ = mecha_core::create_private_dir(&session_dir);
        session_dir.join("chat_history")
    };
    let _ = editor.load_history(&history_path);
    let mut total = Usage::default();

    loop {
        let line = match editor.readline("› ") {
            Ok(line) => line,
            // Ctrl-C abandons the line; Ctrl-D ends the session.
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        let _ = editor.add_history_entry(input);
        // Saved per line rather than at exit, so a killed process keeps what
        // was typed before it died.
        if history_path.exists() {
            let _ = editor.append_history(&history_path);
        } else {
            let _ = editor.save_history(&history_path);
        }

        if let Some(command) = input.strip_prefix('/') {
            match handle_command(command, &prepared, &mut convo, &total, session.as_ref()) {
                Flow::Continue => continue,
                Flow::Quit => break,
            }
        }

        let user = Message::user(input);
        convo.push(user.clone());
        if let Some(s) = &session {
            s.append(&Record::Message(user))?;
        }
        let history_len = convo.len();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let renderer = render::spawn(
            rx,
            render::RenderOpts {
                verbose: global.verbose,
                quiet: false,
            },
        );

        let result = crate::interrupt::run_interruptible(
            &prepared.agent,
            prepared.agent.context(),
            &mut convo,
            Some(tx.clone()),
        )
        .await;
        drop(tx);
        let _ = renderer.await;
        println!();

        match result {
            Ok(outcome) => {
                total.add(&outcome.usage);
                if let Some(s) = &session {
                    s.append_messages(&convo.messages[history_len..])?;
                    // Persist what the conversation now knows, or resuming it
                    // launders the taint exactly as a turn boundary used to.
                    s.append(&Record::Taint(convo.taint))?;
                }
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                // Drop the turn so a failed request doesn't leave a dangling
                // user message that the next request would resend.
                convo.messages.truncate(history_len - 1);
            }
        }
    }

    if let Some(s) = &session {
        println!("\nsession {} · {}", s.meta.id, render::format_usage(&total));
        if let Some(mb) = &prepared.mailbox {
            mb.store.depart(&s.meta.id);
        }
        let cx = prepared.agent.context();
        cx.hooks
            .session_end(&s.meta.id, &s.path, &cx.tools.workspace)
            .await;
    }
    Ok(())
}

enum Flow {
    Continue,
    Quit,
}

fn handle_command(
    command: &str,
    prepared: &setup::Prepared,
    convo: &mut Conversation,
    total: &Usage,
    session: Option<&Session>,
) -> Flow {
    let (name, _rest) = command.split_once(' ').unwrap_or((command, ""));
    match name {
        "exit" | "quit" | "q" => return Flow::Quit,

        "help" | "h" => {
            println!(
                "  /tools          list available tools\n\
                 \x20 /model          show the active model and provider\n\
                 \x20 /usage          tokens used this session\n\
                 \x20 /clear          forget the conversation so far\n\
                 \x20 /session        show the transcript path\n\
                 \x20 /exit           quit"
            );
        }

        "tools" => {
            for tool in prepared.agent.registry().iter() {
                println!(
                    "  {:<28} {}",
                    tool.name(),
                    tool.description().lines().next().unwrap_or("")
                );
            }
        }

        "model" => println!("  {} ({})", prepared.model, prepared.provider_name),

        "usage" => println!("  {}", render::format_usage(total)),

        "clear" => {
            // A new conversation, taint included: nothing the old one read is
            // in context any more, so nothing it read should still apply.
            *convo = Conversation::new();
            println!("  conversation cleared");
        }

        "session" => match session {
            Some(s) => println!("  {} — {}", s.meta.id, s.path.display()),
            None => println!("  not recording (--no-session)"),
        },

        other => println!("  unknown command /{other} — try /help"),
    }
    Flow::Continue
}
