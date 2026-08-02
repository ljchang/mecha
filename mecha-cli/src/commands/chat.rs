//! `mecha chat` — an interactive session in the terminal.

use crate::{render, setup, GlobalOpts};
use anyhow::Result;
use mecha_core::message::{Message, Usage};
use mecha_core::session::{Record, Session, SessionMeta};
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

    let mut messages = Vec::new();
    let mut session = None;

    if let Some(id) = &args.resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        println!("resumed {} ({} messages)", meta.id, prior.len());
        messages = prior;
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

        if let Some(command) = input.strip_prefix('/') {
            match handle_command(command, &prepared, &mut messages, &total, session.as_ref()) {
                Flow::Continue => continue,
                Flow::Quit => break,
            }
        }

        let user = Message::user(input);
        messages.push(user.clone());
        if let Some(s) = &session {
            s.append(&Record::Message(user))?;
        }
        let history_len = messages.len();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let renderer =
            render::spawn(rx, render::RenderOpts { verbose: global.verbose, quiet: false });

        let result = prepared.agent.run(&mut messages, Some(tx.clone())).await;
        drop(tx);
        let _ = renderer.await;
        println!();

        match result {
            Ok(outcome) => {
                total.add(&outcome.usage);
                if let Some(s) = &session {
                    s.append_messages(&messages[history_len..])?;
                }
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                // Drop the turn so a failed request doesn't leave a dangling
                // user message that the next request would resend.
                messages.truncate(history_len - 1);
            }
        }
    }

    if let Some(s) = &session {
        println!("\nsession {} · {}", s.meta.id, render::format_usage(&total));
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
    messages: &mut Vec<Message>,
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
            messages.clear();
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
