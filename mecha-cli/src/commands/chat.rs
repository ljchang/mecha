//! `mecha chat` — an interactive session in the terminal.

use crate::{render, setup, GlobalOpts};
use anyhow::Result;
use mecha_core::agent::Conversation;
use mecha_core::goal_context::{OwnerInput, OwnerSource};
use mecha_core::message::{Message, Usage};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::io::IsTerminal;

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Continue a saved session by id or unique prefix.
    #[arg(long)]
    pub resume: Option<String>,

    /// Don't write a transcript.
    #[arg(long)]
    pub no_session: bool,

    /// Explicit encrypted episodic memory manifest (local recorded dynamic chat).
    #[cfg(feature = "memory-prototype")]
    #[arg(long, value_name = "REQUEST_JSON", conflicts_with = "no_session")]
    pub episodic_memory: Option<std::path::PathBuf>,

    /// Admit selected read-only tools from this owner-configured graph server.
    #[cfg(feature = "memory-prototype")]
    #[arg(
        long,
        value_name = "NAME",
        requires_all = ["episodic_memory", "dynamic_goals"],
        conflicts_with = "no_mcp"
    )]
    pub memory_graph_server: Option<String>,
    /// Create one durable Task from this strict owner-authored JSON file.
    #[cfg(feature = "memory-prototype")]
    #[arg(long, value_name = "TASK_JSON", requires_all = ["episodic_memory", "dynamic_goals"], conflicts_with_all = ["resume", "memory_task_resume", "memory_graph_server"])]
    pub memory_task: Option<std::path::PathBuf>,
    /// Resume this session's exact durable Task and Plan from encrypted storage.
    #[cfg(feature = "memory-prototype")]
    #[arg(long, requires_all = ["episodic_memory", "dynamic_goals", "resume"], conflicts_with_all = ["memory_task", "memory_graph_server"])]
    pub memory_task_resume: bool,
}

#[cfg(feature = "memory-prototype")]
pub(crate) struct MemoryPreparation {
    state: MemoryState,
    graph: Option<PreparedMemoryGraph>,
    task_draft: Option<mecha_core::memory::TaskDraft>,
    task_conversation: Option<(Session, Conversation)>,
}
#[cfg(feature = "memory-prototype")]
enum MemoryState {
    Fresh(Box<mecha_core::memory_session::SessionMemory>),
    Resumed(Box<mecha_core::memory_session::chat::ChatMemory>),
}
#[cfg(feature = "memory-prototype")]
struct PreparedMemoryGraph {
    tools: Box<setup::PreparedTools>,
    surface: mecha_core::memory_session::surface::ChatGraphSurface,
}
/// Runs in main before router observation or provider setup.
#[cfg(feature = "memory-prototype")]
pub(crate) async fn preflight_memory(
    global: &mut GlobalOpts,
    args: &Args,
) -> Result<Option<MemoryPreparation>> {
    let Some(path) = &args.episodic_memory else {
        return Ok(None);
    };
    anyhow::ensure!(
        global.dynamic_goals && std::io::stdin().is_terminal() && !args.no_session,
        "chat memory requires --dynamic-goals, a local terminal, and recording"
    );
    anyhow::ensure!(
        global.goal.is_none(),
        "chat memory uses owner context, not --goal"
    );
    let profile = super::memory_profile::open_chat_profile(
        global,
        path,
        args.memory_graph_server.as_deref(),
        args.memory_task.is_some() || args.memory_task_resume,
    )?;
    let task_draft = args
        .memory_task
        .as_ref()
        .map(|path| {
            // Match the canonical effective workspace used by the profile.
            let config = mecha_core::config::Config::load_global()?;
            let workspace = global
                .workspace
                .as_ref()
                .or(config.tools.workspace.as_ref())
                .cloned()
                .unwrap_or(std::env::current_dir()?);
            mecha_core::memory_session::SessionMemory::load_task_draft(
                &std::path::absolute(path)?,
                &workspace.canonicalize()?,
            )
        })
        .transpose()?;
    let mut task_conversation = None;
    let state = if let Some(id) = &args.resume {
        let path = Session::find(&Session::default_dir()?, id)?;
        let t = Session::read(&path)?;
        let session = Session { meta: t.meta, path };
        let mode = mecha_core::memory_session::chat::ChatAttachMode::Resume;
        let chat = if args.memory_task_resume {
            profile
                .memory
                .attach_task_chat(&session, None, &global.tools)?
        } else if profile.graph.is_some() {
            profile
                .memory
                .attach_chat_with_graph(&session, mode, None)?
        } else {
            profile.memory.attach_chat(&session, mode)?
        };
        if args.memory_task_resume {
            task_conversation = Some(chat.task_conversation()?);
        }
        MemoryState::Resumed(Box::new(chat))
    } else {
        MemoryState::Fresh(Box::new(profile.memory))
    };
    let graph = if let Some(selection) = profile.graph {
        let opts = GlobalOpts {
            surface: Some(mecha_core::session::SessionKind::Chat),
            ..global.clone()
        };
        let staged = setup::prepare_tools_for_memory_graph(
            &opts,
            true,
            &selection.server,
            &selection.remote_names,
        )
        .await?;
        super::memory_profile::validate_memory_config_with_graph(
            &opts,
            &staged.tools.config,
            &selection.local_names,
        )?;
        let local_names: Vec<_> = opts
            .tools
            .iter()
            .filter(|name| !selection.local_names.contains(name))
            .cloned()
            .collect();
        match &state {
            MemoryState::Fresh(memory) => {
                memory.validate_surface(&local_names, &staged.tools.workspace, &[])?;
            }
            MemoryState::Resumed(memory) => {
                // Authenticate saved authority before spawning even the selected child.
                let expected = memory
                    .graph_surface()?
                    .ok_or_else(|| anyhow::anyhow!("chat has no bound graph surface"))?;
                let mut names = opts.tools.clone();
                names.sort();
                anyhow::ensure!(
                    expected.server_name == selection.server.name
                        && expected.launch_digest == staged.launch.launch_digest()
                        && expected.tool_names() == names,
                    "chat graph launch or selected tools changed"
                );
                memory.validate_surface(&names, &staged.tools.workspace, &[])?;
            }
        }
        let (tools, inventory) = staged.connect().await?;
        let surface = graph_surface(&selection.server.name, &local_names, &inventory)?;
        surface.validate(&tools.workspace)?;
        if let MemoryState::Resumed(memory) = &state {
            memory.validate_graph_surface(&surface)?;
        }
        Some(PreparedMemoryGraph {
            tools: Box::new(tools),
            surface,
        })
    } else {
        None
    };
    Ok(Some(MemoryPreparation {
        state,
        graph,
        task_draft,
        task_conversation,
    }))
}

#[cfg(feature = "memory-prototype")]
fn graph_surface(
    server: &str,
    local_names: &[String],
    inventory: &mecha_core::mcp::SelectedGraphTools,
) -> Result<mecha_core::memory_session::surface::ChatGraphSurface> {
    use mecha_core::memory_session::surface::{
        ChatGraphSurface, GraphToolBinding, CHAT_GRAPH_SURFACE_VERSION,
    };
    let mut local_tools = local_names.to_vec();
    local_tools.sort();
    let mut graph_tools = Vec::new();
    for listed in inventory.tools() {
        let descriptor = listed.descriptor();
        anyhow::ensure!(
            descriptor.server_name == server,
            "graph inventory origin changed"
        );
        graph_tools.push(GraphToolBinding {
            local_name: descriptor.local_name.clone(),
            remote_name: descriptor.remote_name.clone(),
            schema_digest: descriptor.schema_digest.clone(),
            description_digest: descriptor.description_digest.clone(),
            read_only: descriptor.read_only,
            private_data: descriptor.capabilities.private_data,
            untrusted_input: descriptor.capabilities.untrusted_input,
            egress: descriptor.capabilities.egress.as_str().into(),
            destructive: descriptor.capabilities.destructive,
            effect: descriptor
                .effect
                .ok_or_else(|| anyhow::anyhow!("graph effect missing"))?
                .as_str()
                .into(),
            deferred: descriptor.deferred,
            fixed_workspace: descriptor.fixed_workspace.clone(),
        });
    }
    graph_tools.sort_by(|a, b| a.local_name.cmp(&b.local_name));
    Ok(ChatGraphSurface {
        version: CHAT_GRAPH_SURFACE_VERSION.into(),
        server_name: server.into(),
        launch_digest: inventory.launch_digest().into(),
        local_tools,
        graph_tools,
    })
}
#[cfg(feature = "memory-prototype")]
fn report_memory(report: &mecha_core::memory_session::chat::ChatMemoryReport) {
    eprintln!(
        "mecha: episodic chat memory {}",
        serde_json::to_string(report).expect("memory report serializes")
    );
}
pub async fn execute(
    global: &GlobalOpts,
    args: Args,
    #[cfg(feature = "memory-prototype")] memory_preparation: Option<MemoryPreparation>,
) -> Result<()> {
    let opts = GlobalOpts {
        surface: Some(mecha_core::session::SessionKind::Chat),
        ..global.clone()
    };
    // A person is at the approver only when stdin is a terminal: a piped
    // `mecha chat` would read `y` from the pipe (review of #294), as `run`
    // already knew.
    let interactive = std::io::stdin().is_terminal();
    anyhow::ensure!(
        !opts.dynamic_goals || interactive,
        "--dynamic-goals requires a local chat terminal"
    );
    #[cfg(feature = "memory-prototype")]
    let (memory_state, prepared_graph, task_draft, mut task_conversation) = match memory_preparation
    {
        Some(memory) => (
            Some(memory.state),
            memory.graph,
            memory.task_draft,
            memory.task_conversation,
        ),
        None => (None, None, None, None),
    };
    #[cfg(feature = "memory-prototype")]
    let carried_todo = match &memory_state {
        Some(MemoryState::Resumed(chat)) if chat.is_task_chat() => Some(chat.task_todo()?),
        _ => None,
    };
    #[cfg(feature = "memory-prototype")]
    let (mut prepared, graph_surface) = match prepared_graph {
        Some(graph) => (
            setup::finish_prepared_tools(*graph.tools, &opts).await?,
            Some(graph.surface),
        ),
        None => (
            setup::prepare_carrying(&opts, interactive, carried_todo).await?,
            None,
        ),
    };
    #[cfg(not(feature = "memory-prototype"))]
    let mut prepared = setup::prepare(&opts, interactive).await?;
    let session_dir = Session::default_dir()?;

    // One conversation for the whole session: the taint travels with it, so a
    // hostile page read on turn one still arms the interlock on turn five.
    let mut convo = Conversation::new();
    let mut session = None;
    #[cfg(feature = "memory-prototype")]
    let (mut chat_memory, mut fresh_memory) = match memory_state {
        Some(MemoryState::Fresh(memory)) => (None, Some(*memory)),
        Some(MemoryState::Resumed(chat)) => (Some(*chat), None),
        None => (None, None),
    };
    #[cfg(feature = "memory-prototype")]
    let memory_enabled = chat_memory.is_some() || fresh_memory.is_some();
    #[cfg(not(feature = "memory-prototype"))]
    let memory_enabled = false;

    #[cfg(feature = "memory-prototype")]
    if let Some((saved, prior)) = task_conversation.take() {
        println!(
            "resumed {} ({} messages)",
            saved.meta.id,
            prior.messages.len()
        );
        convo = prior;
        session = Some(saved);
    }
    if session.is_some() {
        // Authenticated task resume was carried from preflight under the same lease.
    } else if let Some(id) = &args.resume {
        let path = Session::find(&session_dir, id)?;
        // **The other door onto one transcript.** A delegated run is a
        // detached child, so nothing in *this* process can see that its
        // conversation is live; picking the file up anyway would give it two
        // writers, the child appending its turns and this session appending
        // the owner's. The web's `resume` learned to ask the run markers and
        // this one had not, which would have left the rule enforced on the
        // surface that happens to have a card for it and nowhere else — a
        // guard that is really a UI condition.
        if let Some(task) = crate::commands::tasks::markers()
            .ok()
            .and_then(|m| m.live_writer_of(id))
        {
            anyhow::bail!(
                "a run is working {task} in this transcript — `mecha tasks stop {task}` ends \
                 it and keeps what it has, or `mecha tasks steer {task} \"...\"` redirects it \
                 where it is"
            );
        }
        #[cfg(feature = "memory-prototype")]
        let (meta, prior) = if chat_memory.is_some() {
            let t = Session::read(&path)?;
            (t.meta, t.convo)
        } else {
            Session::load(&path)?
        };
        #[cfg(not(feature = "memory-prototype"))]
        let (meta, prior) = Session::load(&path)?;
        anyhow::ensure!(
            !prior.has_dynamic_goal_context() || opts.dynamic_goals,
            "resume this chat with --dynamic-goals"
        );
        println!("resumed {} ({} messages)", meta.id, prior.messages.len());
        // D15: restore the plan from the transcript the model is about to
        // re-read it from, so the two agree from the first turn.
        if let Some(todo) = &prepared.todo {
            let plan = mecha_core::tool::todo::PlanKey::of(&prepared.agent.context().tools);
            if let Some(n) = todo.rehydrate(&plan, &prior.messages) {
                println!("restored a plan of {n} item(s)");
            }
        }
        convo = prior;
        mecha_core::archive::reopened(&path, &meta.id);
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
                kind: Some(mecha_core::session::SessionKind::Chat),
            },
        )?);
    }

    #[cfg(feature = "memory-prototype")]
    if let Some(memory) = fresh_memory.take() {
        let session = session.as_ref().expect("recording required");
        let mode = mecha_core::memory_session::chat::ChatAttachMode::Create;
        chat_memory = Some(if let Some(draft) = task_draft {
            eprintln!(
                "mecha: task startup session {} (retry-task-start uses this identity)",
                session.meta.id
            );
            memory.attach_task_chat(session, Some(draft), &opts.tools)?
        } else if let Some(surface) = &graph_surface {
            memory.attach_chat_with_graph(session, mode, Some(surface.clone()))?
        } else {
            memory.attach_chat(session, mode)?
        });
    }
    #[cfg(feature = "memory-prototype")]
    if let Some(memory) = &chat_memory {
        if memory.is_task_chat() {
            let todo = memory.task_todo()?;
            prepared.agent.registry_mut().insert(todo.clone());
            prepared.todo = Some(todo.clone());
            let registered: std::sync::Arc<dyn mecha_core::tool::Tool> = todo.clone();
            anyhow::ensure!(
                std::sync::Arc::ptr_eq(prepared.todo.as_ref().expect("installed todo"), &todo)
                    && std::sync::Arc::ptr_eq(
                        prepared
                            .agent
                            .registry()
                            .get("todo")
                            .expect("installed todo"),
                        &registered
                    ),
                "task todo handle differs from registry"
            );
            let task = memory.task_diagnostics()?;
            println!("task: {} ({:?})", task.value.title, task.value.status);
            if let Some(plan) = memory.task_plan_summary()? {
                println!("committed plan: {plan}");
            }
        }
        let graph_names: Vec<_> = graph_surface.as_ref().map_or_else(Vec::new, |surface| {
            surface
                .graph_tools
                .iter()
                .map(|tool| tool.local_name.clone())
                .collect()
        });
        super::memory_profile::validate_memory_runtime(&opts, &prepared, memory, &graph_names)?;
        if let Some(surface) = &graph_surface {
            memory.validate_graph_surface(surface)?;
        }
        prepared.agent.registry_mut().insert(memory.context_tool()?);
    }

    // On create and on resume both: a session picked up under different flags
    // is exactly what this record exists to catch.
    if let Some(s) = &session {
        // Before the config record, which captures the tool list for replay.
        if !memory_enabled {
            setup::register_recall(&mut prepared.agent, s);
        }
        let config = RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
            &prepared.levers_off,
            Some(&prepared.rules),
        );
        #[cfg(feature = "memory-prototype")]
        if let Some(memory) = chat_memory.as_ref().filter(|m| m.is_task_chat()) {
            memory.task_record_config(config)?;
        } else {
            s.append(&Record::Config(config))?;
        }
        #[cfg(not(feature = "memory-prototype"))]
        s.append(&Record::Config(config))?;
        // Staged outbox items point back at the session that drafted them.
        if let Some(route) = &prepared.agent.context().outbox {
            route.set_session_id(&s.meta.id);
        }
        // The interactive surface is one producer, `chat`, whichever session
        // is live — that is what lets an overnight trigger address it without
        // knowing which session tomorrow brings. A `--no-session` chat stays
        // anonymous: no identity, no mailbox, no return address.
        if let Some(mb) = &prepared.mailbox {
            mb.attach("chat", &s.meta.id);
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
    let history_path = if !opts.dynamic_goals || session.is_some() {
        let _ = mecha_core::create_private_dir(&session_dir);
        Some(session_dir.join("chat_history"))
    } else {
        None
    };
    if let Some(path) = &history_path {
        let _ = editor.load_history(path);
    }
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
        if let Some(path) = &history_path {
            if path.exists() {
                let _ = editor.append_history(path);
            } else {
                let _ = editor.save_history(path);
            }
        }

        if let Some(command) = input.strip_prefix('/') {
            match handle_command(
                command,
                &prepared,
                &mut convo,
                &total,
                session.as_ref(),
                memory_enabled,
            ) {
                Flow::Continue => continue,
                Flow::Quit => break,
            }
        }

        // Folded, not pushed, when the tail is already a user message — a
        // Ctrl-C mid-tool-turn keeps the partial turn (cancel's contract),
        // so the conversation can end on the user message carrying tool
        // results, and pushing there makes two user messages in a row. The
        // fold is recorded at submit (one direct `Rewrite` — `record_run`
        // between runs would replay the previous run's still-uncleared
        // `rewritten` states) and fails closed like the push branch's own
        // append, so either way `recorded` is what the file already holds.
        let owner_input = if opts.dynamic_goals {
            match OwnerInput::new(OwnerSource::CliChat, input.to_string()) {
                Ok(input) => Some(input),
                Err(err) => {
                    eprintln!("error: {err:#}");
                    continue;
                }
            }
        } else {
            None
        };
        #[cfg(feature = "memory-prototype")]
        let memory_turn = if let Some(memory) = chat_memory.as_mut() {
            let turn = memory.begin_turn(
                session.as_ref().expect("recorded chat"),
                owner_input.as_ref().expect("dynamic owner input"),
            )?;
            let tool = memory.goal_memory(&turn)?;
            let old = prepared
                .agent
                .registry()
                .get("goal_context")
                .expect("profile tool");
            anyhow::ensure!(
                old.input_schema() == tool.input_schema()
                    && old.description() == tool.description(),
                "chat memory schema changed between turns"
            );
            prepared.agent.registry_mut().insert(tool);
            Some(turn)
        } else {
            None
        };
        #[cfg(feature = "memory-prototype")]
        if chat_memory.as_ref().is_some_and(|m| m.is_task_chat()) {
            anyhow::ensure!(
                !convo
                    .messages
                    .last()
                    .is_some_and(|m| m.role == mecha_core::message::Role::User),
                "task cannot rewrite an incomplete owner tail"
            );
        }
        let recorded = if convo
            .messages
            .last()
            .is_some_and(|m| m.role == mecha_core::message::Role::User)
        {
            mecha_core::agent::append_user_text(&mut convo.messages, input.to_string());
            if let Some(s) = &session {
                s.append(&Record::Rewrite {
                    messages: convo.messages.clone(),
                })?;
            }
            convo.messages.clone()
        } else {
            let user = Message::user(input);
            convo.push(user.clone());
            if let Some(s) = &session {
                #[cfg(feature = "memory-prototype")]
                if let Some(memory) = chat_memory.as_ref().filter(|m| m.is_task_chat()) {
                    memory.task_record_owner(
                        memory_turn.as_ref().expect("turn"),
                        owner_input.as_ref().expect("owner"),
                    )?;
                } else {
                    s.append(&Record::Message(user))?;
                }
                #[cfg(not(feature = "memory-prototype"))]
                s.append(&Record::Message(user))?;
            }
            convo.messages.clone()
        };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let renderer = render::spawn(
            rx,
            render::RenderOpts {
                verbose: global.verbose,
                quiet: false,
            },
        );

        let mut cx = prepared.agent.context().as_ref().clone();
        #[cfg(feature = "memory-prototype")]
        let mut task_active = None;
        if let Some(owner_input) = owner_input {
            #[cfg(feature = "memory-prototype")]
            if let Some(memory) = chat_memory.as_ref().filter(|m| m.is_task_chat()) {
                let (context, active) =
                    memory.task_context(memory_turn.as_ref().expect("turn"), owner_input, cx)?;
                cx = context;
                task_active = Some(active);
            } else {
                cx = cx
                    .with_goal_ingress(vec![owner_input], session.clone().map(std::sync::Arc::new));
            }
            #[cfg(not(feature = "memory-prototype"))]
            {
                cx = cx
                    .with_goal_ingress(vec![owner_input], session.clone().map(std::sync::Arc::new));
            }
        }
        let result =
            crate::interrupt::run_interruptible(&prepared.agent, &cx, &mut convo, Some(tx.clone()))
                .await;
        drop(tx);
        let _ = renderer.await;
        println!();

        match result {
            Ok(outcome) => {
                total.add(&outcome.usage);
                #[cfg(feature = "memory-prototype")]
                if let (Some(memory), Some(turn), Some(s)) =
                    (chat_memory.as_mut(), memory_turn.as_ref(), session.as_ref())
                {
                    let recording = (|| -> Result<()> {
                        if let Some(active) = task_active.take() {
                            return memory.task_record_final(
                                turn,
                                active,
                                &recorded,
                                &convo,
                                Some(&outcome),
                            );
                        }
                        s.record_run(&recorded, &convo)?;
                        s.record_outcome(&outcome)?;
                        s.append(&Record::Taint(convo.taint))?;
                        Ok(())
                    })();
                    if let Err(error) = recording {
                        eprintln!("chat recording failed: {error:#}");
                        break;
                    }
                    let report = memory.finish_turn(turn);
                    report_memory(&report);
                    if report.status != mecha_core::memory_session::SessionMemoryStatus::Retained {
                        break;
                    }
                    continue;
                }
                if let Some(s) = &session {
                    s.record_run(&recorded, &convo)?;
                    // How the run went, beside what it said. An interactive
                    // run used to record less than a trigger did.
                    s.record_outcome(&outcome)?;
                    // Persist what the conversation now knows, or resuming it
                    // launders the taint exactly as a turn boundary used to.
                    s.append(&Record::Taint(convo.taint))?;
                }
            }
            Err(e) => {
                eprintln!("error: {e:#}");
                #[cfg(feature = "memory-prototype")]
                if let (Some(memory), Some(turn), Some(s)) =
                    (chat_memory.as_mut(), memory_turn.as_ref(), session.as_ref())
                {
                    if let Some(active) = task_active.take() {
                        if let Err(error) =
                            memory.task_record_final(turn, active, &recorded, &convo, None)
                        {
                            eprintln!("task recording failed: {error:#}");
                        }
                        break;
                    }
                    if let Err(error) = s.record_run(&recorded, &convo) {
                        eprintln!("chat recording failed: {error:#}");
                    }
                    let _ = s.append(&Record::Taint(convo.taint));
                    report_memory(&memory.finish_turn(turn));
                    break;
                }
                // Drop the turn so a failed request doesn't leave a dangling
                // user message that the next request would resend — see
                // `Conversation::roll_back_failed_turn` for why restore-then-
                // pop, and why a bare pop was wrong twice over.
                convo.roll_back_failed_turn(recorded.clone());
                // And the transcript must agree, or the failure survives a
                // resume (serve/chat's review finding, which holds here
                // verbatim): the triggering user message was appended at
                // submit, so without this the file ends on a dangling user
                // turn and `--resume`'s next line puts two user messages in
                // a row. The rolled-back list is not an extension of
                // `recorded`, so `record_run` writes the `Rewrite` that
                // makes a resume load exactly what memory holds. Taint is
                // still persisted — a failed turn that read a hostile page
                // still read it.
                if let Some(s) = &session {
                    // The one write whose failure reproduces the resume-time
                    // 400 this arm exists to prevent — never silent.
                    if let Err(e) = s.record_run(&recorded, &convo) {
                        eprintln!(
                            "warning: the rollback was not recorded — resuming \
                             this session will replay the failed turn: {e:#}"
                        );
                    }
                    let _ = s.append(&Record::Taint(convo.taint));
                }
            }
        }
    }

    if let Some(s) = &session {
        println!("\nsession {} · {}", s.meta.id, render::format_usage(&total));
        if let Some(mb) = &prepared.mailbox {
            mb.detach(&s.meta.id);
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

/// An owner's grant change, written to the transcript at once — the TUI's
/// `recorded`, for the same reason: a revoke the file forgot comes back on
/// resume.
fn recorded(
    said: String,
    session: Option<&Session>,
    grants: &[mecha_core::grant::Grant],
) -> String {
    match session.map(|s| s.record_grants(grants)) {
        Some(Err(e)) => format!("{said} (not written to the transcript: {e:#})"),
        _ => said,
    }
}

fn handle_command(
    command: &str,
    prepared: &setup::Prepared,
    convo: &mut Conversation,
    total: &Usage,
    session: Option<&Session>,
    memory_enabled: bool,
) -> Flow {
    let (name, rest) = command.split_once(' ').unwrap_or((command, ""));
    if memory_enabled
        && (name == "clear"
            || matches!(name, "grant" | "grants" | "revoke") && !rest.trim().is_empty())
    {
        eprintln!("chat memory refuses /{name}; start a fresh session for a new context or grants");
        return Flow::Continue;
    }
    match name {
        "exit" | "quit" | "q" => return Flow::Quit,

        "help" | "h" => {
            println!(
                "  /tools          list available tools\n\
                 \x20 /model          show the active model and provider\n\
                 \x20 /usage          tokens used this session\n\
                 \x20 /clear          forget the conversation so far\n\
                 \x20 /session        show the transcript path\n\
                 \x20 /grant [path [write]]  list, or let this conversation reach a directory\n\
                 \x20 /revoke <path>  end this conversation's grants at or under a path\n\
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
            if let Err(err) = convo.clear_with_recording(session) {
                eprintln!("error clearing conversation: {err:#}");
                return Flow::Continue;
            }
            // And whatever the tools held for it, as the TUI's `/clear` does:
            // a picture or a near-copy strike from the old conversation is
            // not this one's (review of #543).
            prepared.agent.registry().forget_conversation_state();
            println!("  conversation cleared");
        }

        // The TUI's `/grant`, through the same function. No mid-run case:
        // this loop reads the next line only after the run returns.
        "grant" | "grants" => {
            let standing = &prepared.agent.context().tools.grants;
            if rest.trim().is_empty() {
                let lines = mecha_core::grant::listing(standing, &convo.grants);
                if lines.is_empty() {
                    println!("  no grants: the run reaches its workspace only");
                }
                for line in lines {
                    println!("  {line}");
                }
            } else {
                match mecha_core::grant::grant_by_owner(rest, standing, &mut convo.grants) {
                    Ok(said) => println!("  {}", recorded(said, session, &convo.grants)),
                    Err(e) => println!("  /grant {}: {e:#}", rest.trim()),
                }
            }
        }

        "revoke" => {
            let standing = &prepared.agent.context().tools.grants;
            match mecha_core::grant::revoke_by_owner(rest, standing, &mut convo.grants) {
                Ok(said) => println!("  {}", recorded(said, session, &convo.grants)),
                Err(e) => println!("  /revoke {}: {e:#}", rest.trim()),
            }
        }

        "session" => match session {
            Some(s) => println!("  {} — {}", s.meta.id, s.path.display()),
            None => println!("  not recording (--no-session)"),
        },

        other => println!("  unknown command /{other} — try /help"),
    }
    Flow::Continue
}

#[cfg(all(test, feature = "memory-prototype"))]
mod graph_memory_cli_tests {
    use clap::Parser;

    #[test]
    fn graph_memory_selection_is_explicit_chat_only_and_requires_both_opt_ins() {
        let valid = [
            "mecha",
            "chat",
            "--dynamic-goals",
            "--episodic-memory",
            "authored-request.json",
            "--memory-graph-server",
            "graph-fixture",
        ];
        assert!(crate::Cli::try_parse_from(valid).is_ok());
        for invalid in [
            vec!["mecha", "chat", "--memory-graph-server", "graph-fixture"],
            vec![
                "mecha",
                "chat",
                "--episodic-memory",
                "authored-request.json",
                "--memory-graph-server",
                "graph-fixture",
            ],
            vec![
                "mecha",
                "chat",
                "--dynamic-goals",
                "--memory-graph-server",
                "graph-fixture",
            ],
            valid.into_iter().chain(["--no-mcp"]).collect(),
            valid.into_iter().chain(["--no-session"]).collect(),
            vec![
                "mecha",
                "run",
                "--episodic-memory",
                "authored-request.json",
                "--memory-graph-server",
                "graph-fixture",
                "authored task",
            ],
        ] {
            assert!(
                crate::Cli::try_parse_from(&invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }
}
