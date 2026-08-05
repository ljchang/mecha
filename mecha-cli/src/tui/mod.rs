//! A terminal interface where the input line stays live while the agent works.
//!
//! That is the whole reason this exists rather than another REPL. A readline
//! prompt owns stdin only between runs, so anything typed *during* a run either
//! sits in the tty buffer until the run ends or gets stolen by whichever reader
//! is blocked when it does. Here a single event loop owns the terminal for the
//! session, the agent runs in a task beside it, and a line submitted mid-run
//! goes into the run's steering queue — reaching the model inside the turn it is
//! already working on.
//!
//! Ctrl-C is the other half: it cancels the run rather than killing the process,
//! keeping the partial answer and the session.

mod approve;
mod ask;
mod command;
mod tools;
mod transcript;
mod triggers;

use crate::{setup, GlobalOpts};
use anyhow::{Context, Result};
use command::mode_name;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use mecha_core::agent::{Agent, AgentEvent, Conversation, Phase, RunOutcome};
use mecha_core::config::PermissionMode;
use mecha_core::message::{Message, Usage};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use mecha_core::tool::{Approver, ModeApprover};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use transcript::{Entry, Transcript};

type RunResult = (Result<RunOutcome>, Conversation);

/// What the agent is doing, and everything needed to steer or stop it.
struct Running {
    handle: JoinHandle<RunResult>,
    cancel: CancellationToken,
    /// Where a mid-run line goes. Shared with the [`RunContext`] the task holds.
    queue: Arc<Mutex<VecDeque<String>>>,
    started: std::time::Instant,
    /// Set once Ctrl-C has been pressed, so the status line can say so and a
    /// second press can mean something stronger.
    cancelling: bool,
    /// How many messages were already on disk when the run began.
    ///
    /// Carried here because the run *owns* the conversation while it is in
    /// flight — `App::messages` is empty — so there is nothing to measure
    /// against when it comes back. Getting this wrong rewrites the whole
    /// history into the transcript on every turn.
    persisted: usize,
}

/// Everything a provider or MCP change replaces at once.
///
/// Bundled because they have to move together: a new agent comes with a new
/// model name, a new provider name, and a new set of MCP child processes, and
/// leaving any of them behind would show one thing in the status bar while
/// another answered.
struct Live {
    agent: Arc<Agent>,
    model: String,
    provider: String,
    /// The options this agent was built from — the *current* ones, not the ones
    /// the process started with. Switches compose off this: without it,
    /// `/mcp off` followed by `/model x` would quietly turn MCP back on,
    /// because the rebuild would start from the original flags again.
    opts: GlobalOpts,
    /// The todo tool the agent is actually using, polled each frame for the
    /// live pane. Riding here, not on `App`, so a `/model` switch — which
    /// rebuilds the agent and its tools wholesale — refreshes it for free; a
    /// handle cached anywhere else would go stale and watch a dead list.
    todo: Option<Arc<mecha_core::tool::todo::TodoTool>>,
    /// Held for the lifetime of the session: dropping a client kills its
    /// server, so the *old* set must outlive the switch that replaced it only
    /// until the new one is up.
    _mcp: Vec<Arc<mecha_core::mcp::McpClient>>,
}

impl Live {
    fn new(p: setup::Prepared, opts: GlobalOpts) -> Self {
        Live {
            agent: Arc::new(p.agent),
            model: p.model,
            provider: p.provider_name,
            opts,
            todo: p.todo,
            _mcp: p._mcp,
        }
    }
}

/// A modal list of things to switch to.
///
/// Built rather than typed because the useful question is "what can I switch
/// to", and a command that only accepts an exact string cannot answer it. The
/// choices come from the configured providers: those are the entries that
/// actually exist, each already carrying the model it serves.
struct Picker {
    title: String,
    /// Label and the command choosing it runs. Commands rather than switches so
    /// a menu can open another menu — `/help` lists the commands, and picking
    /// `/model` from it opens the model list.
    items: Vec<(String, command::Command)>,
    selected: usize,
}

impl Picker {
    fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as isize;
        // Wraps, because a list this short is faster to cycle than to bound.
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
    }
}

/// A change that cannot be made from a key handler, because it is async and
/// because it must not happen while a run is in flight.
#[derive(Debug, Clone)]
enum Switch {
    Model(String),
    Provider(String),
    Mode(PermissionMode),
    Mcp(bool),
    McpServer(String, bool),
}

struct App {
    transcript: Transcript,
    input: String,
    /// Byte offset into `input`. Bytes, not chars, so it can index directly;
    /// every move steps by whole characters to keep it on a boundary.
    cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    convo: Conversation,
    running: Option<Running>,
    pending: Option<approve::Request>,
    usage: Usage,
    /// What the provider said the last prompt cost. Shown because context
    /// pressure is invisible until it is fatal, and a user who can watch it
    /// climb can decide to /clear or set --compact-at before it bites.
    prompt_tokens: u64,
    /// The model's window, when the provider config says. Turns the number
    /// above into a fraction with a colour, which is the difference between
    /// data and a warning.
    context_window: Option<u64>,
    should_quit: bool,
    /// Ctrl-C at an idle prompt: once to warn, twice to leave.
    quit_armed: bool,
    /// Requested by a slash command, applied by the event loop once it is safe.
    pending_switch: Option<Switch>,
    /// What the approver is currently doing, for `/mode` to report.
    mode: PermissionMode,
    /// Whether MCP servers are connected at all, for `/mcp` to report.
    mcp_on: bool,
    /// Every configured server and whether it is currently connected.
    mcp_servers: Vec<(String, bool)>,
    /// Which tools the next run may see. Toggled with shift+tab.
    phase: Phase,
    /// A question the model is waiting on. Takes every key while it is up, the
    /// same as an approval — and for the same reason, since the run is blocked
    /// on it either way.
    asking: Option<ask::Question>,
    /// Open modal list, if any. Takes every key while it is up.
    picker: Option<Picker>,
    /// The help overlay is up. It exists to be glanced at and dismissed, so
    /// any key closes it.
    help: bool,
    /// The /tools modal, when open. Takes every key while it is up.
    tools: Option<tools::ToolsModal>,
    /// The /triggers modal, when open. Takes every key while it is up.
    scheduled: Option<triggers::TriggersModal>,
    /// A trigger file to open in $EDITOR, deferred to the event loop for the
    /// same reason `pending_editor` is: suspending the TUI needs the terminal.
    pending_trigger_edit: Option<String>,
    /// Where a finished `!command` posts its output. The receiver lives in
    /// the event loop; running the command on a task keeps the input line
    /// live while it does.
    shell_tx: mpsc::UnboundedSender<Entry>,
    /// What `shell` actually is, computed once — the sandbox is config-driven
    /// and a provider switch rebuilds it identically.
    sandbox_line: String,
    /// The workspace root, for `@path` completion. Fixed for the session.
    workspace: std::path::PathBuf,
    /// Whether the todo pane may appear at all. `/todo` flips it; the pane
    /// additionally requires a non-empty list, so this is a veto, not a
    /// summons.
    todo_visible: bool,
    /// ^G was pressed: open $EDITOR on the input. Deferred to the event loop
    /// like `pending_switch`, because suspending the TUI needs the terminal,
    /// which a key handler does not hold.
    pending_editor: bool,
    /// Every provider entry in config, as (name, model). Fixed for the session.
    providers: Vec<(String, String)>,
    /// Whether the terminal speaks the kitty keyboard protocol, which is what
    /// makes Shift+Enter distinguishable from Enter. Alt+Enter works either
    /// way; help text offers Shift+Enter only where it can actually arrive.
    kitty_keyboard: bool,
}

impl App {
    fn status(&self, model: &str, provider: &str, tools: usize) -> Line<'static> {
        let mut spans = vec![
            Span::styled(
                format!(" {model} "),
                Style::new().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::styled(
                format!(" {provider} · {tools} tools "),
                Style::new().fg(Color::DarkGray),
            ),
        ];

        // Only shown while planning: a badge that is always there stops being
        // read, and execute is the state people expect to be in.
        if self.phase == Phase::Plan {
            spans.push(Span::styled(
                " plan ",
                Style::new().fg(Color::Black).bg(Color::Magenta),
            ));
        }

        match &self.running {
            Some(run) => {
                let secs = run.started.elapsed().as_secs();
                spans.push(Span::styled(
                    if run.cancelling {
                        format!(" stopping… {secs}s ")
                    } else {
                        format!(" working {secs}s ")
                    },
                    Style::new().fg(Color::Yellow),
                ));
                spans.push(Span::styled(
                    "· type to steer · ^C to stop ",
                    Style::new().fg(Color::DarkGray),
                ));
            }
            None => {
                spans.push(Span::styled(
                    format!(
                        " {} in / {} out ",
                        self.usage.total_input(),
                        self.usage.output_tokens
                    ),
                    Style::new().fg(Color::DarkGray),
                ));
                if self.prompt_tokens > 0 {
                    // With the window known this is a fuel gauge, not a
                    // curiosity: it turns "the run died at 38869 tokens" into
                    // something visible while there is still room to act.
                    let (text, colour) = match self.context_window {
                        Some(window) if window > 0 => {
                            let pct = (self.prompt_tokens * 100 / window).min(999);
                            let colour = match pct {
                                0..=74 => Color::DarkGray,
                                75..=89 => Color::Yellow,
                                _ => Color::Red,
                            };
                            (
                                format!(
                                    "· context {}/{} ({pct}%) ",
                                    human_tokens(self.prompt_tokens),
                                    human_tokens(window)
                                ),
                                colour,
                            )
                        }
                        _ => (
                            format!("· context {} ", human_tokens(self.prompt_tokens)),
                            Color::DarkGray,
                        ),
                    };
                    spans.push(Span::styled(text, Style::new().fg(colour)));
                }
            }
        }

        if !self.transcript.follow {
            spans.push(Span::styled("· scrolled ", Style::new().fg(Color::Yellow)));
        }
        Line::from(spans)
    }
}

pub async fn execute(global: &GlobalOpts, resume: Option<String>, no_session: bool) -> Result<()> {
    // The approver has to exist before the agent is built, since the agent
    // takes ownership of it.
    let (tui_approver, mut approvals) = approve::TuiApprover::new();
    // Only the TUI registers this: a batch worker or an eval case has nobody to
    // answer, and a tool that blocks forever is worse than one that is absent.
    let (asker, mut questions) = ask::TuiAsker::new();
    let asker: Arc<dyn mecha_core::tool::ask::Asker> = Arc::new(asker);
    // Retained: switching back to `ask` mode has to reinstate *this* approver,
    // the one wired to the event loop, not a fresh terminal one that would
    // fight the interface for stdin.
    let approver: Arc<dyn Approver> = Arc::new(tui_approver);
    let mut prepared = setup::prepare_with_approver(global, Arc::clone(&approver)).await?;
    prepared
        .agent
        .registry_mut()
        .insert(Arc::new(mecha_core::tool::ask::AskUserTool::new(
            Arc::clone(&asker),
        )));

    let session_dir = Session::default_dir()?;
    // One conversation for the session, so the taint accumulates across turns
    // the way the model's context does.
    let mut convo = Conversation::new();
    let mut session = None;

    if let Some(id) = &resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        convo = prior;
        session = Some(Session { meta, path });
    } else if !no_session {
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
    }

    let (shell_tx, mut shell_rx) = mpsc::unbounded_channel::<Entry>();
    let mut app = App {
        transcript: Transcript::new(global.verbose),
        input: String::new(),
        cursor: 0,
        history: Vec::new(),
        history_pos: None,
        convo,
        running: None,
        pending: None,
        usage: Usage::default(),
        prompt_tokens: 0,
        context_window: prepared.agent.context_window(),
        should_quit: false,
        quit_armed: false,
        pending_switch: None,
        mode: prepared.config.tools.permission_mode,
        mcp_on: !global.no_mcp && !prepared.config.mcp.is_empty(),
        mcp_servers: prepared
            .config
            .mcp
            .iter()
            .map(|m| {
                let off = m.disabled
                    || global.no_mcp
                    || global.no_mcp_servers.iter().any(|n| n == &m.name);
                (m.name.clone(), !off)
            })
            .collect(),
        phase: Phase::default(),
        asking: None,
        picker: None,
        help: false,
        tools: None,
        sandbox_line: setup::sandbox_line(&prepared.sandbox),
        workspace: prepared.workspace.clone(),
        todo_visible: true,
        pending_editor: false,
        scheduled: None,
        pending_trigger_edit: None,
        shell_tx,
        providers: prepared
            .config
            .providers
            .iter()
            .map(|(name, cfg)| (name.clone(), cfg.model.clone().unwrap_or_default()))
            .collect(),
        kitty_keyboard: false,
    };

    if !app.convo.is_empty() {
        // Say what was resumed *into*, not just how much. Either half of the
        // taint changes what the next turn is allowed to do, so a user who
        // reopens a conversation should not have to guess why an outbound call
        // is suddenly refused.
        let carried = match (app.convo.taint.private, app.convo.taint.untrusted) {
            (true, true) => " · already holds private data and third-party content, so outbound calls will be refused",
            (true, false) => " · already holds private data",
            (false, true) => " · already holds third-party content",
            (false, false) => "",
        };
        app.transcript.push(Entry::Notice(format!(
            "resumed {} messages{carried}",
            app.convo.len()
        )));
    }

    let mut live = Live::new(prepared, global.clone());
    let (mut terminal, kitty) = enter()?;
    app.kitty_keyboard = kitty;
    set_title(&format!("mecha · {}", workspace_name(&app)));
    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut live,
        &mut approvals,
        &mut questions,
        &mut shell_rx,
        session.as_ref(),
        &approver,
    )
    .await;
    leave(&mut terminal)?;

    if let Some(s) = &session {
        println!(
            "session {} · {}",
            s.meta.id,
            crate::render::format_usage(&app.usage)
        );
        let cx = live.agent.context();
        cx.hooks
            .session_end(&s.meta.id, &s.path, &cx.tools.workspace)
            .await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    terminal: &mut Terminal<impl Backend>,
    app: &mut App,
    live: &mut Live,
    approvals: &mut mpsc::UnboundedReceiver<approve::Request>,
    questions: &mut mpsc::UnboundedReceiver<ask::Question>,
    shell_results: &mut mpsc::UnboundedReceiver<Entry>,
    session: Option<&Session>,
    approver: &Arc<dyn Approver>,
) -> Result<()> {
    let mut keys = EventStream::new();
    // Agent events arrive on a channel that is replaced per run. Holding a
    // sender here keeps the receiver alive between runs so `select!` has
    // something to poll rather than a closed branch.
    let (mut events_tx, mut events_rx) = mpsc::unbounded_channel::<AgentEvent>();

    loop {
        // Recomputed each frame: a `/provider` or `/mcp` switch changes the
        // tool list underneath us.
        let (model, provider, tools) = (
            live.model.clone(),
            live.provider.clone(),
            live.agent.registry().len(),
        );
        // Polled per frame rather than event-driven: the list lives behind a
        // Mutex the tool writes to, and a lock-and-clone at frame rate is
        // cheaper than being clever.
        let todo_items = live.todo.as_ref().map(|t| t.items());
        // CSI 2026: the terminal buffers everything between the pair and
        // presents it as one repaint. Follow-mode streaming scrolls the whole
        // transcript region every token, and over SSH that write arrives in
        // arbitrary packet-sized pieces — without this, visibly torn.
        // Terminals that do not know the mode ignore it by spec, so there is
        // nothing to probe.
        crossterm::queue!(
            std::io::stdout(),
            crossterm::terminal::BeginSynchronizedUpdate
        )?;
        terminal.draw(|frame| draw(frame, app, &model, &provider, tools, todo_items.as_deref()))?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EndSynchronizedUpdate
        )?;

        // Applied here rather than in the key handler: rebuilding is async, and
        // a run in flight must finish under the settings it started with.
        if let Some(switch) = app.pending_switch.take() {
            apply_switch(switch, app, live, approver, session).await?;
            continue;
        }

        // ^G, deferred here for the same reason: handing the terminal to
        // $EDITOR needs the terminal.
        if app.pending_editor {
            app.pending_editor = false;
            suspend_and_edit(terminal, app)?;
            continue;
        }

        // Editing a trigger file, same deferral and the same suspend dance.
        if let Some(name) = app.pending_trigger_edit.take() {
            suspend_and_edit_trigger(terminal, app, &name)?;
            continue;
        }

        if app.should_quit {
            return Ok(());
        }

        // A run in flight redraws on a timer so the elapsed clock ticks even
        // when nothing else is happening.
        let tick = tokio::time::sleep(std::time::Duration::from_millis(if app.running.is_some() {
            200
        } else {
            60_000
        }));

        tokio::select! {
            Some(Ok(event)) = keys.next() => on_terminal_event(app, event, &mut events_tx, &mut events_rx, &live.agent, session)?,

            Some(event) = events_rx.recv() => {
                match &event {
                    AgentEvent::TurnUsage(u) => {
                        app.usage.add(u);
                        app.prompt_tokens = u.total_input();
                    }
                    AgentEvent::Compacted { messages_before, messages_after, .. } => {
                        app.transcript.push(Entry::Notice(format!(
                            "compacted {messages_before} messages into {messages_after} to fit the context"
                        )));
                    }
                    _ => {}
                }
                app.transcript.absorb(&event);
            }

            Some(request) = approvals.recv() => app.pending = Some(request),
            Some(question) = questions.recv() => app.asking = Some(question),
            // A `!command` finished; its output enters the transcript and
            // nothing else — the model never sees it.
            Some(entry) = shell_results.recv() => app.transcript.push(entry),

            // A finished run: collect the outcome and take the conversation back.
            outcome = wait_for_run(&mut app.running), if app.running.is_some() => {
                let persisted = app.running.as_ref().map_or(0, |r| r.persisted);
                finish_run(app, outcome, persisted, session)?;
            }

            _ = tick => {}
        }
    }
}

/// Await the in-flight run without holding a borrow across the `select!`.
async fn wait_for_run(running: &mut Option<Running>) -> RunResult {
    match running {
        Some(run) => match (&mut run.handle).await {
            Ok(result) => result,
            // The task owns the conversation, so a panic takes it with it and
            // there is nothing to hand back. The transcript on disk still has
            // everything up to this turn — say so, rather than letting the
            // screen quietly empty.
            Err(e) => (
                Err(anyhow::anyhow!(
                    "the run task failed: {e}. The conversation in memory is lost; \
                     reopen it with --resume."
                )),
                Conversation::new(),
            ),
        },
        // Never selected: the branch is guarded on `is_some`.
        None => std::future::pending().await,
    }
}

fn finish_run(
    app: &mut App,
    outcome: RunResult,
    persisted: usize,
    session: Option<&Session>,
) -> Result<()> {
    let (result, convo) = outcome;
    app.convo = convo;

    match result {
        Ok(outcome) => {
            app.usage = Usage::default();
            app.usage.add(&outcome.usage);
            if outcome.stop_cause.is_early() {
                app.transcript.push(Entry::Notice(format!(
                    "{} after {}",
                    outcome.stop_cause.describe(),
                    mecha_core::agent::turns_phrase(outcome.turns)
                )));
            }
            if let Some(s) = session {
                // Everything the run added; the opening user message was
                // written when it was submitted.
                s.append_messages(&app.convo.messages[persisted.min(app.convo.len())..])?;
                s.append(&Record::Taint(app.convo.taint))?;
            }
        }
        Err(e) => {
            app.transcript.push(Entry::Error(format!("error: {e:#}")));
            // Drop the dangling user turn so the next request doesn't resend it.
            app.convo.messages.truncate(persisted.saturating_sub(1));
        }
    }

    app.running = None;
    set_title(&format!("mecha · {}", workspace_name(app)));
    Ok(())
}

fn on_terminal_event(
    app: &mut App,
    event: Event,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    agent: &Arc<Agent>,
    session: Option<&Session>,
) -> Result<()> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            on_key(app, key, events_tx, events_rx, agent, session)
        }
        // Inserted whole, never submitted. Dropping a file onto the terminal
        // arrives here too: terminals send the *path* as pasted text, so a drop
        // is a paste as far as this is concerned.
        Event::Paste(text) => {
            app.quit_armed = false;
            app.input.insert_str(app.cursor, &text);
            app.cursor += text.len();
            Ok(())
        }
        Event::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::ScrollUp => app.transcript.scroll_up(3),
                MouseEventKind::ScrollDown => app.transcript.scroll_down(3),
                _ => {}
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn on_key(
    app: &mut App,
    key: KeyEvent,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    agent: &Arc<Agent>,
    session: Option<&Session>,
) -> Result<()> {
    // An approval modal takes every key: nothing else should be reachable while
    // a tool call is waiting on a decision.
    if let Some(request) = app.pending.take() {
        use approve::Answer;
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Some(Answer::Allow),
            KeyCode::Char('a') => Some(Answer::Always),
            KeyCode::Char('n') | KeyCode::Esc => Some(Answer::Deny),
            _ => None,
        };
        match answer {
            Some(answer) => {
                app.transcript.push(Entry::Notice(match answer {
                    Answer::Allow => format!("allowed {}", request.tool),
                    Answer::Always => format!("allowing {} for this session", request.tool),
                    Answer::Deny => format!("declined {}", request.tool),
                }));
                let _ = request.reply.send(answer);
            }
            // Unrecognised key: put it back and keep waiting.
            None => app.pending = Some(request),
        }
        return Ok(());
    }

    // A question owns the keyboard, but only for the keys that answer it:
    // an open question is answered by typing, so ordinary editing has to fall
    // through to the input line below.
    if app.asking.is_some() {
        let has_options = app.asking.as_ref().is_some_and(|q| !q.options.is_empty());
        match key.code {
            KeyCode::Esc => {
                // Declining is a legitimate answer; the tool tells the model to
                // proceed with its best interpretation and say which it chose.
                if let Some(q) = app.asking.take() {
                    let _ = q.reply.send(None);
                    app.transcript
                        .push(Entry::Notice("left it to the model".into()));
                }
                return Ok(());
            }
            // Only while nothing has been typed. Otherwise an answer that
            // begins with a digit — "3 files, not 2" — selects option 3 before
            // its second character arrives, and the typed route is only
            // available to answers that happen not to start with a number.
            KeyCode::Char(c) if has_options && c.is_ascii_digit() && app.input.is_empty() => {
                let choice = c.to_digit(10).unwrap_or(0) as usize;
                if choice >= 1 {
                    if let Some(q) = app.asking.take() {
                        match q.options.get(choice - 1) {
                            Some(answer) => {
                                app.transcript.push(Entry::User(answer.clone()));
                                let _ = q.reply.send(Some(answer.clone()));
                            }
                            None => app.asking = Some(q),
                        }
                    }
                }
                return Ok(());
            }
            // Modified Enter falls through to the editor below and becomes a
            // newline — an answer is allowed to have paragraphs.
            KeyCode::Enter
                if !app.input.trim().is_empty()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                let answer = app.input.trim().to_string();
                app.input.clear();
                app.cursor = 0;
                if let Some(q) = app.asking.take() {
                    app.transcript.push(Entry::User(answer.clone()));
                    let _ = q.reply.send(Some(answer));
                }
                return Ok(());
            }
            // Anything else edits the answer being typed.
            _ => {}
        }
    }

    // The tools modal owns the keyboard while it is up, like the picker below.
    if let Some(modal) = &mut app.tools {
        match key.code {
            KeyCode::Up if !modal.detail => modal.move_by(-1),
            KeyCode::Down if !modal.detail => modal.move_by(1),
            // Enter opens the detail; from the detail it steps back out, so
            // enter-enter-enter walks in and out rather than dead-ending.
            KeyCode::Enter => modal.detail = !modal.detail,
            KeyCode::Esc | KeyCode::Char('q') => {
                if modal.detail {
                    modal.detail = false;
                } else {
                    app.tools = None;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // The triggers modal, same rule as /tools: it owns the keyboard.
    if app.scheduled.is_some() {
        return handle_triggers_key(app, key);
    }

    // A modal list owns the keyboard while it is up, for the same reason the
    // approval modal does: a keystroke meant for the list must not also reach
    // the input line behind it.
    if let Some(picker) = &mut app.picker {
        match key.code {
            KeyCode::Up => picker.move_by(-1),
            KeyCode::Down => picker.move_by(1),
            KeyCode::Esc | KeyCode::Char('q') => {
                app.picker = None;
            }
            KeyCode::Enter => {
                if let Some(picker) = app.picker.take() {
                    let chosen = picker.selected;
                    if let Some((_, cmd)) = picker.items.into_iter().nth(chosen) {
                        return run_command(app, cmd, agent, session);
                    }
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // The help overlay closes on the next key — but a printable key was
    // meant for the input, not the overlay, so it falls through: someone
    // typing "?why" opens help on the ? and must not lose the w. A second ?
    // just closes (or the overlay would reopen and the key would toggle
    // nothing). Checked after the real modals — an approval or a question
    // arriving while help is up still gets its answer.
    if app.help {
        app.help = false;
        match key.code {
            KeyCode::Char(c) if c != '?' => {}
            KeyCode::Backspace => {}
            _ => return Ok(()),
        }
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char('c') if ctrl => match &mut app.running {
            // Stop the run, not the process. The partial answer survives.
            Some(run) => {
                run.cancel.cancel();
                run.cancelling = true;
            }
            None if app.quit_armed => app.should_quit = true,
            None => {
                app.quit_armed = true;
                app.transcript
                    .push(Entry::Notice("^C again to quit".into()));
            }
        },

        KeyCode::Char('d') if ctrl && app.input.is_empty() => app.should_quit = true,

        // Compose in $EDITOR. Deferred to the event loop; what comes back
        // lands in the input box, not on the wire — sending is still Enter.
        KeyCode::Char('g') if ctrl => app.pending_editor = true,

        // A live version of --verbose. The transcript records everything and
        // filters at render, so turning this on mid-run reveals the tool
        // output that already happened — which is exactly when you want it.
        KeyCode::Char('o') if ctrl => {
            app.transcript.verbose = !app.transcript.verbose;
            app.transcript
                .push(Entry::Notice(if app.transcript.verbose {
                    "showing thinking and tool output — ^O to hide".into()
                } else {
                    "hiding thinking and tool output — ^O to show".into()
                }));
        }

        // Fill in as much as every candidate agrees on. Repeated presses
        // converge rather than cycling through guesses — and on a lone
        // directory candidate the fill ends in `/`, so the next press
        // descends.
        KeyCode::Tab => {
            if let Some((start, partial)) = command::at_token(&app.input, app.cursor) {
                let candidates = command::path_candidates(partial, &app.workspace);
                let filled = command::common_prefix(&candidates);
                if filled.len() > partial.len() {
                    app.input.replace_range(start..app.cursor, &filled);
                    app.cursor = start + filled.len();
                }
            } else {
                let candidates = command::completions(&app.input);
                let filled = command::common_prefix(&candidates);
                if !filled.is_empty() {
                    app.input = format!("/{filled}");
                    app.cursor = app.input.len();
                }
            }
        }

        // Shift+Tab. Toggling rather than a command because it is the one
        // setting worth changing without breaking stride.
        KeyCode::BackTab => {
            app.phase = match app.phase {
                Phase::Execute => Phase::Plan,
                Phase::Plan => Phase::Execute,
            };
            app.transcript.push(Entry::Notice(match app.phase {
                Phase::Plan => "planning — writing tools are not offered".into(),
                Phase::Execute => "executing — every tool is available".into(),
            }));
        }

        // A newline, not a submission. Shift+Enter needs the kitty keyboard
        // protocol to be distinguishable at all; Alt+Enter arrives distinctly
        // on almost every terminal, so it is the fallback spelling.
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.quit_armed = false;
            app.input.insert(app.cursor, '\n');
            app.cursor += 1;
        }

        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            if !text.is_empty() {
                app.input.clear();
                app.cursor = 0;
                app.history.push(text.clone());
                app.history_pos = None;
                submit(app, text, events_tx, events_rx, agent, session)?;
            }
        }

        // Only on an empty line: with anything typed, `?` is a character in a
        // question the user is writing.
        KeyCode::Char('?') if app.input.is_empty() => app.help = true,

        KeyCode::Char(c) => {
            app.quit_armed = false;
            app.input.insert(app.cursor, c);
            app.cursor += c.len_utf8();
        }

        KeyCode::Backspace => {
            if let Some(prev) = prev_boundary(&app.input, app.cursor) {
                app.input.remove(prev);
                app.cursor = prev;
            }
        }
        KeyCode::Delete => {
            if app.cursor < app.input.len() {
                app.input.remove(app.cursor);
            }
        }

        KeyCode::Left => app.cursor = prev_boundary(&app.input, app.cursor).unwrap_or(0),
        KeyCode::Right => app.cursor = next_boundary(&app.input, app.cursor),
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.input.len(),

        KeyCode::Up => recall(app, -1),
        KeyCode::Down => recall(app, 1),

        KeyCode::PageUp => app.transcript.scroll_up(10),
        KeyCode::PageDown => app.transcript.scroll_down(10),
        KeyCode::Esc => app.transcript.jump_to_bottom(),

        _ => {}
    }
    Ok(())
}

/// Apply a queued switch. Called from the event loop, never from a key handler:
/// rebuilding is async, and none of it is safe with a run in flight.
///
/// Three things every switch here has to respect, each learned somewhere else
/// in this codebase:
///
///   * **The tool list is the front of the cached prefix.** Changing it — which
///     `/provider` and `/mcp` both do — invalidates the prompt cache, so the
///     next turn re-pays for the whole prefix. Said out loud rather than
///     absorbed silently, because on a metered provider it is money.
///   * **A switch is a configuration change, so it gets a `Record::Config`.**
///     Without one the transcript claims the whole session ran under the
///     settings it started with, and a replay of it would be diffing against a
///     recording that never happened.
///   * **Taint does not un-happen.** Dropping the servers that fetched
///     something hostile does not unread it, and the interlock stays armed.
///     `/clear` is the only thing that resets it, because that drops the
///     context too.
async fn apply_switch(
    switch: Switch,
    app: &mut App,
    live: &mut Live,
    approver: &Arc<dyn Approver>,
    session: Option<&Session>,
) -> Result<()> {
    if app.running.is_some() {
        app.transcript
            .push(Entry::Notice("busy — stop the run first (^C)".into()));
        return Ok(());
    }

    // Permission mode needs no rebuild: the approver is behind an `Arc` in the
    // run context, and swapping it is copy-on-write.
    if let Switch::Mode(mode) = switch {
        let Some(agent) = Arc::get_mut(&mut live.agent) else {
            app.transcript.push(Entry::Notice(
                "cannot change mode while the agent is shared".into(),
            ));
            return Ok(());
        };
        let next: Arc<dyn Approver> = match mode {
            PermissionMode::Ask => Arc::clone(approver),
            other => Arc::new(ModeApprover { mode: other }),
        };
        agent.set_approver(next);
        app.mode = mode;
        app.transcript
            .push(Entry::Notice(format!("mode {}", mode_name(mode))));
        record_config(session, live, app.mode)?;
        return Ok(());
    }

    // Everything else means building a new agent, starting from what is
    // running now rather than from what the process was launched with.
    let mut opts = live.opts.clone();
    let what = match &switch {
        Switch::Model(id) => {
            opts.model = Some(id.clone());
            format!("model {id}")
        }
        Switch::Provider(name) => {
            opts.provider = Some(name.clone());
            // The new provider brings its own default model; carrying the old
            // one across would ask for a model the new backend has never heard
            // of.
            opts.model = None;
            format!("provider {name}")
        }
        Switch::Mcp(on) => {
            opts.no_mcp = !on;
            // Turning them all on clears the individual exclusions too, or
            // "all on" would silently leave one off.
            if *on {
                opts.no_mcp_servers.clear();
            }
            if *on {
                "MCP on".to_string()
            } else {
                "MCP off".to_string()
            }
        }
        Switch::McpServer(name, on) => {
            opts.no_mcp_servers.retain(|n| n != name);
            if !on {
                opts.no_mcp_servers.push(name.clone());
            } else {
                // Naming one server to turn on has to lift the blanket switch,
                // or nothing happens and the reason is invisible.
                opts.no_mcp = false;
            }
            format!("{name} {}", if *on { "on" } else { "off" })
        }
        Switch::Mode(_) => unreachable!("handled above"),
    };

    app.transcript
        .push(Entry::Notice(format!("switching to {what}…")));

    let prepared = match setup::prepare_with_approver(&opts, Arc::clone(approver)).await {
        Ok(p) => p,
        // Keep the working agent. A failed switch that also broke the session
        // would punish a typo far out of proportion.
        Err(e) => {
            app.transcript.push(Entry::Error(format!(
                "could not switch: {e:#} — staying on {}",
                live.model
            )));
            return Ok(());
        }
    };

    let tools_changed = prepared.agent.registry().len() != live.agent.registry().len();
    *live = Live::new(prepared, opts);
    app.mcp_on = !live.opts.no_mcp;
    for (name, on) in &mut app.mcp_servers {
        *on = !live.opts.no_mcp && !live.opts.no_mcp_servers.iter().any(|n| n == name);
    }

    app.transcript.push(Entry::Notice(format!(
        "now {} ({}) · {} tools{}",
        live.model,
        live.provider,
        live.agent.registry().len(),
        if tools_changed {
            " · prompt cache reset"
        } else {
            ""
        }
    )));

    record_config(session, live, app.mode)?;
    Ok(())
}

/// Append the configuration a run will now use, so the transcript does not
/// claim the whole session ran under whatever it started with.
fn record_config(session: Option<&Session>, live: &Live, mode: PermissionMode) -> Result<()> {
    let Some(s) = session else { return Ok(()) };
    let cfg = mecha_core::config::Config::load(
        live.opts
            .workspace
            .as_deref()
            .unwrap_or(std::path::Path::new(".")),
    )?;
    let mut record = RunConfig::of(&live.agent, &cfg, &live.provider);
    // The file cannot know about a `/mode` switch, and a replay that read the
    // file's mode would be reproducing permissions this session never ran under.
    record.permission_mode = mode;
    s.append(&Record::Config(record))
}

/// Carry out a slash command. Everything here is local to the session — none of
/// it reaches the model.
fn run_command(
    app: &mut App,
    cmd: command::Command,
    agent: &Arc<Agent>,
    session: Option<&Session>,
) -> Result<()> {
    use command::Command;

    let mut say = |text: String| app.transcript.push(Entry::Notice(text));

    match cmd {
        Command::Help => app.help = true,

        Command::Tools => {
            let outbox = agent.context().outbox.clone();
            let rows = agent
                .registry()
                .iter()
                .map(|t| tools::ToolRow {
                    name: t.name().to_string(),
                    read_only: t.read_only(),
                    outbox: outbox.as_ref().is_some_and(|o| o.routes(t.name())),
                    caps: t.capabilities(),
                    description: t.description().to_string(),
                })
                .collect();
            app.tools = Some(tools::ToolsModal {
                rows,
                selected: 0,
                detail: false,
                sandbox_line: app.sandbox_line.clone(),
            });
        }

        Command::Triggers => match triggers::load(5) {
            Ok(rows) => app.scheduled = Some(triggers::TriggersModal::new(rows)),
            Err(e) => say(format!("triggers: {e:#}")),
        },

        Command::Usage => say(format!(
            "{} · {} in the last prompt",
            crate::render::format_usage(&app.usage),
            app.prompt_tokens
        )),

        Command::Session => say(match session {
            Some(s) => format!("{}", s.path.display()),
            None => "not recording a transcript (--no-session)".to_string(),
        }),

        Command::Clear => {
            // A whole new conversation, not just an emptied message list: taint
            // is a property of the conversation, so dropping the context has to
            // drop what entered it. Keeping the taint here would leave the
            // interlock armed by a page nothing in context has read any more.
            app.convo = Conversation::new();
            app.usage = Usage::default();
            app.prompt_tokens = 0;
            app.transcript.push(Entry::Notice(
                "cleared — new conversation, and the taint went with it".into(),
            ));
        }

        Command::Todo => {
            app.todo_visible = !app.todo_visible;
            say(if app.todo_visible {
                "todo pane shown — it appears whenever the list is non-empty".into()
            } else {
                "todo pane hidden".into()
            });
        }

        Command::Quit => app.should_quit = true,

        Command::Model(None) | Command::Provider(None) => {
            let current = agent.provider_id();
            let items: Vec<(String, Command)> = app
                .providers
                .iter()
                .map(|(name, model)| {
                    let here = if name == current { "  ← current" } else { "" };
                    (
                        format!("{name:<10} {model}{here}"),
                        Command::Provider(Some(name.clone())),
                    )
                })
                .collect();

            if items.is_empty() {
                say("no providers configured — see `mecha config path`".into());
            } else {
                let selected = app
                    .providers
                    .iter()
                    .position(|(n, _)| n == current)
                    .unwrap_or(0);
                app.picker = Some(Picker {
                    title: " switch model · ↑↓ then enter, esc to cancel ".into(),
                    items,
                    selected,
                });
            }
        }
        Command::Mode(None) => {
            let modes = [
                PermissionMode::Ask,
                PermissionMode::Allow,
                PermissionMode::ReadOnly,
            ];
            let describe = |m: PermissionMode| match m {
                PermissionMode::Ask => "ask        approve each write or command",
                PermissionMode::Allow => "allow      run everything without asking",
                PermissionMode::ReadOnly => "read-only  refuse anything that writes",
            };
            app.picker = Some(Picker {
                title: " permission mode · ↑↓ then enter ".into(),
                items: modes
                    .iter()
                    .map(|m| {
                        let here = if *m == app.mode { "  ← current" } else { "" };
                        (format!("{}{here}", describe(*m)), Command::Mode(Some(*m)))
                    })
                    .collect(),
                selected: modes.iter().position(|m| *m == app.mode).unwrap_or(0),
            });
        }
        Command::Mcp(None) => {
            if app.mcp_servers.is_empty() {
                say("no MCP servers configured — see `mecha config path`".into());
            } else {
                let mut items = vec![
                    ("all on".to_string(), Command::Mcp(Some(true))),
                    ("all off".to_string(), Command::Mcp(Some(false))),
                ];
                // Each server flips individually: with more than one of them,
                // "all" is rarely the granularity you want.
                for (name, on) in &app.mcp_servers {
                    items.push((
                        format!("{:<14} {}", name, if *on { "on" } else { "off" }),
                        Command::McpServer(name.clone(), Some(!on)),
                    ));
                }
                app.picker = Some(Picker {
                    title: " MCP servers · enter flips the one you pick ".into(),
                    items,
                    selected: 2,
                });
            }
        }

        Command::McpServer(name, want) => match app.mcp_servers.iter().find(|(n, _)| *n == name) {
            Some((_, on)) => {
                let target = want.unwrap_or(!on);
                if target == *on {
                    say(format!(
                        "{name} is already {}",
                        if target { "on" } else { "off" }
                    ));
                } else {
                    app.pending_switch = Some(Switch::McpServer(name, target));
                }
            }
            None => say(format!(
                "no MCP server named {name:?} — configured: {}",
                app.mcp_servers
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        },

        // Everything that changes the agent goes through the event loop: these
        // are async, and none of them may happen with a run in flight.
        Command::Model(Some(id)) => app.pending_switch = Some(Switch::Model(id)),
        Command::Provider(Some(name)) => app.pending_switch = Some(Switch::Provider(name)),
        Command::Mode(Some(m)) => app.pending_switch = Some(Switch::Mode(m)),
        Command::Mcp(Some(on)) => app.pending_switch = Some(Switch::Mcp(on)),

        Command::BadToggle(word) => say(format!("say on or off, not {word:?}")),

        Command::BadMode(word) => say(format!("no such mode {word:?} (ask | allow | read-only)")),
        Command::Unknown(name) => say(format!("no such command /{name}\n{}", command::HELP)),
    }
    Ok(())
}

/// Either start a run or steer the one already going — the same key, and from
/// the user's side the same gesture.
fn submit(
    app: &mut App,
    text: String,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    agent: &Arc<Agent>,
    session: Option<&Session>,
) -> Result<()> {
    // The shell escape is handled before steering, like slash commands: a
    // `!git status` typed mid-run is the user checking on the world, not an
    // instruction meant for the model.
    if let Some(cmd) = command::shell_escape(&text) {
        run_shell_escape(app, agent, cmd.to_string());
        return Ok(());
    }

    // Commands are handled before steering: a `/clear` typed mid-run is far
    // more likely to be a mistake than an instruction meant for the model, and
    // sending it as steering would put a slash command into the transcript.
    if let Some(cmd) = command::parse(&text) {
        return run_command(app, cmd, agent, session);
    }

    if let Some(run) = &app.running {
        // Steering. The loop picks this up at the top of its next turn and
        // folds it in beside the tool results, so the model reads it without
        // the run being stopped and restarted.
        if let Ok(mut queue) = run.queue.lock() {
            queue.push_back(text);
        }
        return Ok(());
    }

    let user = Message::user(&text);
    app.convo.push(user.clone());
    if let Some(s) = session {
        s.append(&Record::Message(user))?;
    }
    app.transcript.push(Entry::User(text));

    set_title(&format!(
        "mecha ▶ {} · {}",
        workspace_name(app),
        agent.model()
    ));

    // A fresh channel per run, so a late event from a cancelled run cannot
    // bleed into the next one.
    let (tx, rx) = mpsc::unbounded_channel();
    *events_tx = tx.clone();
    *events_rx = rx;

    let cancel = CancellationToken::new();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let cx = agent
        .context()
        .as_ref()
        .clone()
        .with_cancel(cancel.clone())
        .with_phase(app.phase)
        .with_queued_input(Arc::clone(&queue));

    let agent = Arc::clone(agent);
    // Everything up to and including the message just submitted is on disk.
    let persisted = app.convo.len();
    let mut convo = std::mem::take(&mut app.convo);
    let handle = tokio::spawn(async move {
        let result = agent.run_in(&cx, &mut convo, Some(tx)).await;
        (result, convo)
    });

    app.running = Some(Running {
        handle,
        cancel,
        queue,
        started: std::time::Instant::now(),
        cancelling: false,
        persisted,
    });
    Ok(())
}

/// Keys for the /triggers modal.
///
/// Split out rather than inlined because it does more than move a cursor: each
/// action shells out and then reloads the rows, and that is worth reading in
/// one place. Every mutation goes through `mecha trigger ...`, so the modal can
/// do exactly what the command line can and no more.
fn handle_triggers_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.scheduled else {
        return Ok(());
    };

    // A pending confirmation swallows the keyboard: y does the thing, anything
    // else backs out. Deliberately not "any key confirms".
    if let Some(confirm) = modal.confirm.take() {
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            let outcome = trigger_cli(&["rm", &confirm.name]);
            modal.status = Some(match outcome {
                Ok(_) => format!("deleted `{}`", confirm.name),
                Err(e) => format!("could not delete `{}`: {e}", confirm.name),
            });
            reload_triggers(app);
        }
        return Ok(());
    }

    // Any keypress clears the last action's message: it has been read, and a
    // stale "started `morning`" over a later action is worse than no message.
    modal.status = None;

    match key.code {
        KeyCode::Up => {
            if modal.detail {
                modal.scroll_detail(-1)
            } else {
                modal.move_by(-1)
            }
        }
        KeyCode::Down => {
            if modal.detail {
                modal.scroll_detail(1)
            } else {
                modal.move_by(1)
            }
        }
        KeyCode::PageUp if modal.detail => modal.scroll_detail(-10),
        KeyCode::PageDown if modal.detail => modal.scroll_detail(10),
        KeyCode::Enter => {
            modal.detail = !modal.detail;
            modal.detail_scroll = 0;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            if modal.detail {
                modal.detail = false;
            } else {
                app.scheduled = None;
            }
        }
        // Editing suspends the whole TUI, so it cannot happen here — defer it
        // to the event loop, which owns the terminal.
        KeyCode::Char('e') => {
            if let Some(name) = modal.selected_name() {
                app.pending_trigger_edit = Some(name.to_string());
            }
        }
        KeyCode::Char(' ') => {
            if let Some(row) = modal.selected_row() {
                let (verb, name) = (
                    if row.enabled { "disable" } else { "enable" },
                    row.name.clone(),
                );
                let outcome = trigger_cli(&[verb, &name]);
                modal.status = Some(match outcome {
                    Ok(_) => format!("{verb}d `{name}`"),
                    Err(e) => format!("could not {verb} `{name}`: {e}"),
                });
                reload_triggers(app);
            }
        }
        // Run now: spawned detached, never awaited. A briefing takes half a
        // minute and a codegen trigger could take twenty, and the interface
        // has to stay live — the ledger and the session are where the result
        // lands, and the modal shows both on reload.
        KeyCode::Char('r') => {
            if let Some(name) = modal.selected_name().map(str::to_string) {
                modal.status = Some(match spawn_trigger_run(&name) {
                    Ok(_) => format!("started `{name}` — reopen /triggers to see how it went"),
                    Err(e) => format!("could not start `{name}`: {e}"),
                });
                reload_triggers(app);
            }
        }
        // Cancel the run in flight. Not a signal — see `TriggerStore::
        // request_cancel`; it stops at the next safe point and keeps its
        // partial answer.
        KeyCode::Char('c') => {
            if let Some(name) = modal.selected_name().map(str::to_string) {
                modal.status = Some(match trigger_cli(&["cancel", &name]) {
                    Ok(out) => out.trim().to_string(),
                    Err(e) => format!("could not cancel `{name}`: {e}"),
                });
                reload_triggers(app);
            }
        }
        // Deleting is the one thing here that cannot be undone by pressing the
        // same key again, so it is the one thing that asks.
        KeyCode::Char('x') => {
            if let Some(row) = modal.selected_row() {
                modal.confirm = Some(triggers::Confirm {
                    name: row.name.clone(),
                    prompt: format!(
                        "Delete trigger `{}`? Its file goes; its ledger rows stay as the record.",
                        row.name
                    ),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuild the modal's rows, keeping the cursor where it was.
fn reload_triggers(app: &mut App) {
    let (selected, detail, status) = match &app.scheduled {
        Some(m) => (m.selected, m.detail, m.status.clone()),
        None => return,
    };
    match triggers::load(5) {
        Ok(rows) => {
            let selected = selected.min(rows.len().saturating_sub(1));
            app.scheduled = Some(triggers::TriggersModal {
                selected,
                detail: detail && !rows.is_empty(),
                status,
                ..triggers::TriggersModal::new(rows)
            });
        }
        Err(e) => {
            app.scheduled = None;
            app.transcript
                .push(Entry::Error(format!("triggers: {e:#}")));
        }
    }
}

/// Run `mecha trigger ...` and return its output.
///
/// `current_exe` rather than a bare `mecha`, so a TUI started from
/// `target/debug` drives the build it is part of and not whatever is on PATH —
/// otherwise testing a change to triggers would silently exercise the
/// installed binary.
fn trigger_cli(args: &[&str]) -> Result<String> {
    let exe = std::env::current_exe().context("cannot find my own binary")?;
    let out = std::process::Command::new(exe)
        .arg("trigger")
        .args(args)
        .output()
        .context("running mecha trigger")?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("{}", err.trim().lines().next().unwrap_or("failed"))
    }
}

/// Start a run and do not wait for it. Output goes nowhere: the TUI owns the
/// screen, and the run's real record is its session and its ledger row.
fn spawn_trigger_run(name: &str) -> Result<()> {
    let exe = std::env::current_exe().context("cannot find my own binary")?;
    std::process::Command::new(exe)
        .args(["trigger", "run", name])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting the run")?;
    Ok(())
}

/// Hand the terminal to `$EDITOR` with the current input, and take both back.
///
/// The suspend mirrors `leave()` and the restore mirrors `enter()`, minus the
/// panic hook (still installed) and the probe (already answered — the kitty
/// flags are re-pushed if they were pushed before). The full `terminal.clear`
/// afterwards is load-bearing: the editor drew over everything, and a diff
/// against the pre-editor buffer would restore only what happened to change.
fn suspend_and_edit(terminal: &mut Terminal<impl Backend>, app: &mut App) -> Result<()> {
    disable_raw_mode()?;
    if kitty_pushed() {
        crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
    }
    crossterm::execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;

    let result = crate::editor::edit_text(
        &app.input,
        &format!("mecha-compose-{}.txt", std::process::id()),
    );

    enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    if kitty_pushed() {
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    terminal.clear()?;

    match result {
        // Into the input box, not onto the wire: sending is still Enter.
        Ok(text) => {
            app.input = text.trim_end().to_string();
            app.cursor = app.input.len();
        }
        // A failed editor keeps what was typed — quitting vim in anger must
        // not eat the draft.
        Err(e) => app.transcript.push(Entry::Error(format!(
            "editor: {e:#} — the input is unchanged"
        ))),
    }
    Ok(())
}

/// Edit a trigger's file in `$EDITOR`, then reload the modal.
///
/// The suspend is the same dance as `suspend_and_edit`; what differs is where
/// the text goes. Saving goes through `mecha trigger edit`'s own validation
/// path — a file that does not parse is refused and the old one kept — so a
/// mistyped schedule cannot silently disarm a trigger.
fn suspend_and_edit_trigger(
    terminal: &mut Terminal<impl Backend>,
    app: &mut App,
    name: &str,
) -> Result<()> {
    disable_raw_mode()?;
    if kitty_pushed() {
        crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
    }
    crossterm::execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;

    let result = trigger_cli(&["edit", name]);

    enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    if kitty_pushed() {
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    terminal.clear()?;

    if let Some(modal) = &mut app.scheduled {
        modal.status = Some(match &result {
            Ok(_) => format!("saved `{name}`"),
            Err(e) => format!("`{name}` not saved: {e}"),
        });
    }
    // Loud as well as in the title: an edit that was rejected must not be
    // something you only find out about at 07:00 tomorrow.
    if let Err(e) = result {
        app.transcript
            .push(Entry::Error(format!("trigger `{name}` was not saved: {e}")));
    }
    reload_triggers(app);
    Ok(())
}

/// The terminal/tab title: "is it still going" answered from a tab strip,
/// which matters over SSH where notifications do not reach.
fn set_title(title: &str) {
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(title));
}

fn workspace_name(app: &App) -> String {
    app.workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.workspace.display().to_string())
}

/// Run a `!command` in the workspace, on a task so the input line stays live.
///
/// Deliberately none of what a tool call gets: no approval (the user typed the
/// command themselves — approving your own keystrokes is theatre), no taint
/// (nothing reaches the model), no session record (it is the user's own
/// terminal, not part of the conversation). Useful precisely because of what
/// it is not.
fn run_shell_escape(app: &mut App, agent: &Arc<Agent>, cmd: String) {
    let workspace = agent.context().tools.workspace.clone();
    let tx = app.shell_tx.clone();
    app.transcript
        .push(Entry::Notice(format!("running !{cmd}")));
    tokio::spawn(async move {
        let result = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .current_dir(&workspace)
            // The TUI owns the terminal; a child that inherits stdin would
            // silently eat keystrokes meant for the input line.
            .stdin(std::process::Stdio::null())
            .output()
            .await;

        let entry = match result {
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                if !out.stderr.is_empty() {
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    text.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                Entry::Shell {
                    cmd,
                    output: clip_output(&text),
                    status: out.status.code(),
                }
            }
            Err(e) => Entry::Error(format!("!{cmd}: {e}")),
        };
        // The receiver only closes when the TUI is exiting; output arriving
        // after that has nowhere sensible to go anyway.
        let _ = tx.send(entry);
    });
}

/// Keep a local command's output readable in the transcript, which is a view
/// and not a pager. The full output was never captured for the model — this
/// is only about the screen.
///
/// Both axes, because they fail differently: many lines scroll the useful
/// part away, and one enormous line (`!cat` on a minified file) sits whole in
/// memory and wraps for thousands of rows.
fn clip_output(s: &str) -> String {
    const MAX_LINES: usize = 200;
    const MAX_BYTES: usize = 16_000;

    let total = s.lines().count();
    let mut out: String = if total <= MAX_LINES {
        s.trim_end().to_string()
    } else {
        let mut kept: String = s.lines().take(MAX_LINES).collect::<Vec<_>>().join("\n");
        kept.push_str(&format!("\n… ({} more lines)", total - MAX_LINES));
        kept
    };

    if out.len() > MAX_BYTES {
        let cut = (0..=MAX_BYTES)
            .rev()
            .find(|&i| out.is_char_boundary(i))
            .unwrap_or(0);
        let dropped = out.len() - cut;
        out.truncate(cut);
        out.push_str(&format!("\n… ({dropped} more bytes)"));
    }
    out
}

fn recall(app: &mut App, direction: i32) {
    if app.history.is_empty() {
        return;
    }
    let next = match (app.history_pos, direction) {
        (None, -1) => Some(app.history.len() - 1),
        (Some(i), -1) => Some(i.saturating_sub(1)),
        (Some(i), 1) if i + 1 < app.history.len() => Some(i + 1),
        // Past the newest entry is the empty line you were typing.
        (Some(_), 1) => None,
        (None, _) => None,
        _ => app.history_pos,
    };
    app.history_pos = next;
    app.input = next.map(|i| app.history[i].clone()).unwrap_or_default();
    app.cursor = app.input.len();
}

fn prev_boundary(s: &str, at: usize) -> Option<usize> {
    s[..at].char_indices().next_back().map(|(i, _)| i)
}

fn next_boundary(s: &str, at: usize) -> usize {
    s[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// Where the cursor sits in the wrapped input box, and how many rows the text
/// needs: `(column, row, rows)`.
///
/// Split out and made pure because pasted text can contain newlines, and the
/// arithmetic that assumed it could not put the cursor in the wrong place the
/// moment anyone pasted a snippet. A newline is a hard break; everything else
/// wraps at `width`.
fn input_layout(text: &str, cursor: usize, width: u16) -> (u16, u16, u16) {
    let width = width.max(1);
    let (mut col, mut row) = (0u16, 0u16);
    let (mut cursor_col, mut cursor_row) = (0u16, 0u16);

    for (offset, ch) in text.char_indices() {
        if offset == cursor {
            (cursor_col, cursor_row) = (col, row);
        }
        if ch == '\n' {
            col = 0;
            row += 1;
        } else {
            col += 1;
            if col >= width {
                col = 0;
                row += 1;
            }
        }
    }
    if cursor >= text.len() {
        (cursor_col, cursor_row) = (col, row);
    }

    (cursor_col, cursor_row, row + 1)
}

fn draw(
    frame: &mut Frame,
    app: &mut App,
    model: &str,
    provider: &str,
    tools: usize,
    todo: Option<&[mecha_core::tool::todo::TodoItem]>,
) {
    // The input box grows with what has been typed rather than scrolling
    // sideways, so a long steering instruction stays readable while writing it.
    let inner_width = frame.area().width.saturating_sub(2);
    let (cursor_col, cursor_row, rows) = input_layout(&app.input, app.cursor, inner_width);
    let input_height = rows.clamp(1, 6) + 2;

    // The pane exists only while there is a list: an empty bordered box would
    // be a badge that is always there, and those stop being read.
    let todo = todo.filter(|items| app.todo_visible && !items.is_empty());
    let todo_height = todo.map_or(0, |items| (items.len() as u16).min(8) + 2);

    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(todo_height),
        Constraint::Length(1),
        Constraint::Length(input_height),
    ])
    .split(frame.area());

    app.transcript.draw(frame, chunks[0]);
    if let Some(items) = todo {
        draw_todo(frame, chunks[1], items);
    }
    frame.render_widget(
        Paragraph::new(app.status(model, provider, tools)),
        chunks[2],
    );

    let (border, hint) = match &app.running {
        Some(run) if run.cancelling => (Color::Red, " stopping "),
        Some(_) => (Color::Yellow, " steer "),
        None => (Color::Cyan, " message "),
    };
    // Ghost completion: the rest of what every candidate agrees on, dim, after
    // the cursor. Shown rather than applied, so typing on never fights it.
    // Two candidate sources, one mechanism: an `@path` token at the cursor
    // completes against the workspace, anything else against command names.
    let (candidates, typed) = match command::at_token(&app.input, app.cursor) {
        Some((_, partial)) => (
            command::path_candidates(partial, &app.workspace),
            partial.to_string(),
        ),
        None => (
            command::completions(&app.input)
                .into_iter()
                .map(str::to_string)
                .collect(),
            app.input.trim_start_matches('/').to_string(),
        ),
    };
    let ghost = command::common_prefix(&candidates)
        .strip_prefix(&typed)
        .unwrap_or_default()
        .to_string();

    let body = if ghost.is_empty() {
        Line::from(app.input.as_str())
    } else {
        Line::from(vec![
            Span::raw(app.input.as_str()),
            Span::styled(ghost.clone(), Style::new().fg(Color::DarkGray)),
            Span::styled("  tab", Style::new().fg(Color::DarkGray)),
        ])
    };

    let input = Paragraph::new(body).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(hint),
    );
    frame.render_widget(input, chunks[3]);

    // Cursor position inside the bordered box, wrapping as the text does and
    // breaking where the text does.
    frame.set_cursor_position((
        chunks[3].x + 1 + cursor_col,
        chunks[3].y + 1 + cursor_row.min(rows.clamp(1, 6).saturating_sub(1)),
    ));

    // What else could still be meant, listed under the box. Only while the
    // name is being typed — once there is an argument the question is settled.
    if !candidates.is_empty() && candidates.len() > 1 {
        // One row: past a dozen entries the answer is a narrower partial,
        // not a longer menu.
        let shown = candidates.len().min(12);
        let mut hint = format!("  {}", candidates[..shown].join("  "));
        if candidates.len() > shown {
            hint.push_str(&format!("  … +{}", candidates.len() - shown));
        }
        let area = Rect {
            x: chunks[3].x,
            y: chunks[3].y.saturating_sub(1),
            width: chunks[3].width,
            height: 1,
        };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(Line::styled(hint, Style::new().fg(Color::DarkGray))),
            area,
        );
    }

    // Help first: a question or an approval arriving while it is up matters
    // more than the reference card, so they draw over it.
    if app.help {
        draw_help(frame, app.kitty_keyboard);
    }
    if let Some(modal) = &app.tools {
        modal.draw(frame);
    }
    if let Some(modal) = &app.scheduled {
        modal.draw(frame);
    }
    if let Some(question) = &app.asking {
        draw_question(frame, question);
    }
    if let Some(picker) = &app.picker {
        draw_picker(frame, picker);
    }
    if let Some(request) = &app.pending {
        draw_approval(frame, request);
    }
}

/// The agent's own task list, live. The model has no read path to this Mutex
/// beyond the echo in its last `todo` result — the pane is for the human, and
/// it is most of why the tool is worth having during a long run.
fn draw_todo(frame: &mut Frame, area: Rect, items: &[mecha_core::tool::todo::TodoItem]) {
    use mecha_core::tool::todo::Status;

    let done = items
        .iter()
        .filter(|i| i.status == Status::Completed)
        .count();
    let body: Vec<Line> = items
        .iter()
        .map(|item| {
            let (marker, style) = match item.status {
                Status::Completed => ("[x]", Style::new().fg(Color::DarkGray)),
                Status::InProgress => ("[~]", Style::new().fg(Color::Yellow)),
                Status::Pending => ("[ ]", Style::new().fg(Color::White)),
            };
            Line::styled(format!(" {marker} {}", item.content), style)
        })
        .collect();

    // When the list is taller than the pane, keep the working edge visible:
    // the finished head is the part nobody is waiting on.
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let first_active = items
        .iter()
        .position(|i| i.status != Status::Completed)
        .unwrap_or(0);
    let scroll = (first_active + 1).saturating_sub(visible) as u16;

    frame.render_widget(
        Paragraph::new(body).scroll((scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::DarkGray))
                .title(format!(" todo {done}/{} · /todo hides ", items.len())),
        ),
        area,
    );
}

/// The middle tier of progressive disclosure: the status line hints at 3–4
/// keys in the moment, this lists all of them, and the docs hold the rest.
fn draw_help(frame: &mut Frame, kitty: bool) {
    // Shift+Enter only where it can actually arrive — advertising it on a
    // terminal without the kitty protocol would teach a key that submits.
    let newline_keys = if kitty {
        "shift+enter · alt+enter"
    } else {
        "alt+enter"
    };
    let keys: Vec<(&str, String)> = vec![
        ("enter", "send · while running, steer the run".into()),
        (newline_keys, "insert a newline".into()),
        ("tab", "complete a /command or an @path".into()),
        ("shift+tab", "toggle planning (writing tools hidden)".into()),
        ("^o", "show or hide thinking and tool output".into()),
        ("^c", "stop the run · twice at idle to quit".into()),
        ("^d", "quit, when the input is empty".into()),
        ("esc", "jump back to the newest output".into()),
        ("pgup pgdn wheel", "scroll the transcript".into()),
        ("↑ ↓", "input history".into()),
        ("?", "this overlay, on an empty line".into()),
        (
            "!command",
            "run it locally — the model never sees it".into(),
        ),
        ("^g", "compose the input in $EDITOR".into()),
    ];

    let mut body: Vec<Line> = keys
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("  {key:<18}"), Style::new().fg(Color::Cyan)),
                Span::styled(what.clone(), Style::new().fg(Color::White)),
            ])
        })
        .collect();
    body.push(Line::raw(""));
    for line in command::HELP.lines() {
        body.push(Line::styled(
            line.to_string(),
            Style::new().fg(Color::DarkGray),
        ));
    }

    let area = centered(
        frame.area(),
        70,
        (body.len() as u16)
            .saturating_add(2)
            .min(frame.area().height),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan))
                .title(" help · any key to close "),
        ),
        area,
    );
}

fn draw_question(frame: &mut Frame, q: &ask::Question) {
    // question + blank + options + blank + hint, inside two borders — plus a
    // row for the question wrapping, which it does at any real width. Getting
    // this one short silently clips the line telling you how to answer.
    const WIDTH: u16 = 74;
    let question_rows = (q.question.len() as u16 / (WIDTH - 2).max(1)) + 1;
    let height = (q.options.len() as u16).clamp(0, 8) + question_rows + 5;
    let area = centered(frame.area(), WIDTH, height);
    frame.render_widget(Clear, area);

    let mut body = vec![
        Line::styled(q.question.as_str(), Style::new().fg(Color::White).bold()),
        Line::raw(""),
    ];
    for (i, option) in q.options.iter().enumerate() {
        body.push(Line::from(vec![
            Span::styled(
                format!(" {} ", i + 1),
                Style::new().fg(Color::Black).bg(Color::Green),
            ),
            Span::raw(" "),
            Span::styled(option.clone(), Style::new().fg(Color::White)),
        ]));
    }
    body.push(Line::raw(""));
    body.push(Line::styled(
        if q.options.is_empty() {
            "type an answer and press enter · esc to let it decide"
        } else {
            "press a number, or type an answer · esc to let it decide"
        },
        Style::new().fg(Color::DarkGray),
    ));

    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Green))
                .title(" the agent is asking "),
        ),
        area,
    );
}

fn draw_picker(frame: &mut Frame, picker: &Picker) {
    let height = (picker.items.len() as u16).clamp(1, 12) + 2;
    let area = centered(frame.area(), 64, height);
    frame.render_widget(Clear, area);

    let body: Vec<Line> = picker
        .items
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            if i == picker.selected {
                Line::styled(
                    format!("› {label}"),
                    Style::new().fg(Color::Black).bg(Color::Cyan),
                )
            } else {
                Line::styled(format!("  {label}"), Style::new().fg(Color::White))
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan))
                .title(picker.title.as_str()),
        ),
        area,
    );
}

fn draw_approval(frame: &mut Frame, request: &approve::Request) {
    let area = centered(frame.area(), 70, 9);
    frame.render_widget(Clear, area);

    let body = vec![
        Line::from(vec![Span::styled(
            request.tool.as_str(),
            Style::new().fg(Color::Magenta).bold(),
        )]),
        Line::raw(""),
        Line::styled(request.summary.as_str(), Style::new().fg(Color::White)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("[y]", Style::new().fg(Color::Green).bold()),
            Span::raw("es  "),
            Span::styled("[a]", Style::new().fg(Color::Green).bold()),
            Span::raw("lways  "),
            Span::styled("[n]", Style::new().fg(Color::Red).bold()),
            Span::raw("o"),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Yellow))
                .title(" allow this? "),
        ),
        area,
    );
}

/// 12400 -> "12.4k". A running token count is glanced at, not read.
fn human_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{:.1}k", n as f64 / 1000.0)
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Whether the kitty keyboard flags were pushed, for the teardown paths. A
/// static because the panic hook — installed before the probe has run — must
/// know whether there is anything to pop.
static KITTY_PUSHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn kitty_pushed() -> bool {
    KITTY_PUSHED.load(std::sync::atomic::Ordering::SeqCst)
}

/// Take over the terminal, and make sure a panic gives it back.
///
/// Without the hook, a panic in raw mode leaves the user with an unusable shell
/// and no visible message — the backtrace is drawn into the alternate screen
/// that never gets torn down.
///
/// The second return says whether the terminal speaks the kitty keyboard
/// protocol — probed here because the probe needs raw mode.
fn enter() -> Result<(Terminal<CrosstermBackend<std::io::Stdout>>, bool)> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        // A panic between Begin and End would otherwise leave the terminal
        // buffering until its own timeout; ending an update that was never
        // begun is harmless.
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EndSynchronizedUpdate
        );
        if kitty_pushed() {
            let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = crossterm::execute!(
            std::io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
        previous(info);
    }));

    enable_raw_mode().context("this needs a terminal")?;
    let mut stdout = std::io::stdout();
    // Without bracketed paste, a pasted newline arrives as `KeyCode::Enter` and
    // *submits*: paste three lines and you have fired off three half-written
    // prompts. It also makes a dragged-and-dropped file path arrive as one
    // event rather than a burst of keystrokes.
    crossterm::execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;

    // The kitty keyboard protocol is what makes Shift+Enter a different key
    // from Enter. Only where the terminal reports it: pushed blind, terminals
    // that half-implement it can start reporting keys this loop does not
    // expect. Pushed *after* entering the alternate screen, because kitty
    // keeps a separate flag stack per screen buffer — pushed before, the flags
    // would outlive the TUI on the main screen.
    let kitty = matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    );
    if kitty {
        crossterm::execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        KITTY_PUSHED.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    Ok((Terminal::new(CrosstermBackend::new(stdout))?, kitty))
}

fn leave(terminal: &mut Terminal<impl Backend>) -> Result<()> {
    // An emptied title lets the shell's own prompt hook reclaim it; leaving
    // "mecha ▶ …" on a tab that no longer runs mecha is a small lie forever.
    set_title("");
    disable_raw_mode()?;
    // Popped before leaving the alternate screen, mirroring the push order —
    // the flags belong to the alternate screen's stack.
    if kitty_pushed() {
        crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
        KITTY_PUSHED.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    crossterm::execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::input_layout;
    use super::*;
    use ratatui::backend::TestBackend;

    use super::Picker;

    /// An `App` with nothing going on, for frame tests. Fields that need a
    /// live agent stay inert: `running` is `None`, channels dangle unused.
    fn test_app() -> App {
        let (shell_tx, _shell_rx) = mpsc::unbounded_channel();
        // The receiver is dropped; frame tests never run a `!command`.
        std::mem::forget(_shell_rx);
        App {
            transcript: Transcript::new(false),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            convo: Conversation::new(),
            running: None,
            pending: None,
            usage: Usage::default(),
            prompt_tokens: 0,
            context_window: None,
            should_quit: false,
            quit_armed: false,
            pending_switch: None,
            mode: PermissionMode::Ask,
            mcp_on: false,
            mcp_servers: Vec::new(),
            phase: Phase::default(),
            asking: None,
            picker: None,
            help: false,
            tools: None,
            sandbox_line: "sandbox: none — commands run as you, with your credentials".into(),
            workspace: std::env::temp_dir(),
            todo_visible: true,
            pending_editor: false,
            scheduled: None,
            pending_trigger_edit: None,
            shell_tx,
            providers: Vec::new(),
            kitty_keyboard: false,
        }
    }

    /// Everything on the frame, one string, for substring assertions —
    /// deliberately not a snapshot, so cosmetic tweaks do not churn tests.
    fn frame_text(app: &mut App, width: u16, height: u16, todo: Option<&[TodoItem]>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, app, "test-model", "test-provider", 3, todo))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    use mecha_core::tool::todo::{Status, TodoItem};

    #[test]
    fn the_status_line_reads_idle_context_and_scrolled() {
        let mut app = test_app();
        let idle = frame_text(&mut app, 80, 12, None);
        assert!(idle.contains("test-model"), "{idle}");
        assert!(idle.contains("0 in / 0 out"), "{idle}");

        // With the window known, the count becomes a fuel gauge.
        app.prompt_tokens = 29_300;
        app.context_window = Some(32_800);
        let gauged = frame_text(&mut app, 80, 12, None);
        assert!(gauged.contains("context 29.3k/32.8k (89%)"), "{gauged}");

        // Scrolled back: the status says so, and only while actually back.
        // Wider frame — the badge sits at the end of the status line, and a
        // narrow terminal legitimately truncates it.
        for i in 0..40 {
            app.transcript.push(Entry::Notice(format!("line {i}")));
        }
        app.transcript.scroll_up(5);
        let scrolled = frame_text(&mut app, 110, 12, None);
        assert!(scrolled.contains("scrolled"), "{scrolled}");
        app.transcript.jump_to_bottom();
        let followed = frame_text(&mut app, 110, 12, None);
        assert!(!followed.contains("scrolled"), "{followed}");
    }

    #[tokio::test]
    async fn a_running_frame_shows_the_timer_and_the_steering_hint() {
        let mut app = test_app();
        app.running = Some(Running {
            handle: tokio::spawn(async { std::future::pending::<RunResult>().await }),
            cancel: CancellationToken::new(),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            started: std::time::Instant::now(),
            cancelling: false,
            persisted: 0,
        });
        let text = frame_text(&mut app, 80, 12, None);
        assert!(text.contains("working"), "{text}");
        assert!(text.contains("type to steer"), "{text}");
    }

    #[test]
    fn the_help_overlay_advertises_the_newline_key_only_where_it_exists() {
        // On a terminal without the kitty protocol, Shift+Enter *submits* —
        // help that teaches it as a newline is worse than no help.
        let mut app = test_app();
        app.help = true;
        let plain = frame_text(&mut app, 100, 30, None);
        assert!(plain.contains("alt+enter"), "{plain}");
        assert!(!plain.contains("shift+enter"), "{plain}");
        assert!(
            plain.contains("/clear"),
            "commands render from HELP: {plain}"
        );

        app.kitty_keyboard = true;
        let kitty = frame_text(&mut app, 100, 30, None);
        assert!(kitty.contains("shift+enter"), "{kitty}");
    }

    #[test]
    fn the_tools_modal_detail_spells_the_declared_surface_out() {
        let mut app = test_app();
        app.tools = Some(tools::ToolsModal {
            rows: vec![tools::ToolRow {
                name: "shell".into(),
                read_only: false,
                outbox: false,
                caps: mecha_core::tool::Capabilities {
                    private_data: true,
                    ..Default::default()
                },
                description: "Run a command.".into(),
            }],
            selected: 0,
            detail: true,
            sandbox_line: app.sandbox_line.clone(),
        });
        let text = frame_text(&mut app, 100, 30, None);
        assert!(
            text.contains("reads data the user considers private"),
            "{text}"
        );
        assert!(
            text.contains("sandbox: none"),
            "shell's detail names the sandbox: {text}"
        );
    }

    #[test]
    fn the_todo_pane_appears_with_content_clamps_and_can_be_vetoed() {
        let mut app = test_app();
        let items: Vec<TodoItem> = (0..12)
            .map(|i| TodoItem {
                content: format!("step {i}"),
                status: if i < 2 {
                    Status::Completed
                } else {
                    Status::Pending
                },
            })
            .collect();

        let text = frame_text(&mut app, 80, 24, Some(&items));
        assert!(text.contains("todo 2/12"), "{text}");
        // Clamped at eight rows of items: the pane is a glance, not a pager.
        let shown = (0..12)
            .filter(|i| text.contains(&format!("step {i}")))
            .count();
        assert!(
            shown <= 8,
            "expected at most 8 items on screen, saw {shown}:\n{text}"
        );

        // Empty list: no pane at all — an always-there box stops being read.
        let empty = frame_text(&mut app, 80, 24, Some(&[]));
        assert!(!empty.contains("todo"), "{empty}");

        // /todo vetoes it even with content.
        app.todo_visible = false;
        let vetoed = frame_text(&mut app, 80, 24, Some(&items));
        assert!(!vetoed.contains("todo 2/12"), "{vetoed}");
    }

    #[test]
    fn shell_output_is_clipped_on_both_axes() {
        // Many lines and one enormous line fail differently: the first
        // scrolls the useful part away, the second sits whole in memory and
        // wraps for thousands of rows.
        let many = (0..500)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let clipped = super::clip_output(&many);
        assert!(
            clipped.lines().count() <= 201,
            "kept {} lines",
            clipped.lines().count()
        );
        assert!(clipped.contains("more lines"), "{clipped}");

        let huge = "x".repeat(100_000);
        let clipped = super::clip_output(&huge);
        assert!(clipped.len() < 17_000, "kept {} bytes", clipped.len());
        assert!(clipped.contains("more bytes"), "says what was dropped");

        // A multi-byte char at the cut must not split.
        let unicode = "é".repeat(20_000);
        let clipped = super::clip_output(&unicode);
        assert!(clipped.len() < 17_000);
    }

    #[test]
    fn nested_subagent_calls_indent_under_their_parent() {
        let mut app = test_app();
        app.transcript.absorb(&AgentEvent::ToolCall {
            id: "p".into(),
            name: "helper".into(),
            input: serde_json::json!({}),
        });
        app.transcript.absorb(&AgentEvent::Nested {
            tool: "helper".into(),
            id: Some("p".into()),
            event: Box::new(AgentEvent::ToolCall {
                id: "c".into(),
                name: "echo".into(),
                input: serde_json::json!({}),
            }),
        });

        let text = frame_text(&mut app, 80, 12, None);
        let parent = text.lines().find(|l| l.contains("helper")).unwrap();
        let child = text.lines().find(|l| l.contains("echo")).unwrap();
        assert!(parent.starts_with("● "), "parent at the margin: {parent:?}");
        assert!(child.starts_with("  ● "), "child one level in: {child:?}");
    }

    fn picker(n: usize) -> Picker {
        Picker {
            title: String::new(),
            items: (0..n)
                .map(|i| (i.to_string(), super::command::Command::Usage))
                .collect(),
            selected: 0,
        }
    }

    #[test]
    fn the_selection_wraps_at_both_ends() {
        // A list this short is faster to cycle than to bound, and stopping dead
        // at the last entry reads as a stuck key.
        let mut p = picker(3);
        p.move_by(1);
        assert_eq!(p.selected, 1);
        p.move_by(1);
        p.move_by(1);
        assert_eq!(p.selected, 0, "did not wrap forwards");

        p.move_by(-1);
        assert_eq!(p.selected, 2, "did not wrap backwards");
    }

    #[test]
    fn an_empty_list_does_not_panic_or_move() {
        // `% 0` panics, and a config with no providers is a real state.
        let mut p = picker(0);
        p.move_by(1);
        p.move_by(-1);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn the_cursor_tracks_plain_wrapping() {
        // 10 columns: "abcdefghij" fills a row, so the eleventh character is at
        // the start of the next one.
        assert_eq!(input_layout("abcdefghijk", 11, 10), (1, 1, 2));
        assert_eq!(input_layout("abc", 3, 10), (3, 0, 1));
        assert_eq!(input_layout("", 0, 10), (0, 0, 1));
    }

    #[test]
    fn a_pasted_newline_breaks_the_line_instead_of_being_counted_as_a_character() {
        // The bug this pins: the old arithmetic divided the character count by
        // the width, so any pasted snippet put the cursor somewhere else
        // entirely and the box was drawn too short.
        let text = "one\ntwo";
        assert_eq!(input_layout(text, text.len(), 40), (3, 1, 2));

        let three = "a\nb\nc";
        assert_eq!(input_layout(three, three.len(), 40), (1, 2, 3));
    }

    #[test]
    fn a_cursor_in_the_middle_of_pasted_text_lands_on_the_right_row() {
        let text = "one\ntwo\nthree";
        // Just after the second newline: start of the third row.
        assert_eq!(input_layout(text, 8, 40), (0, 2, 3));
    }

    #[test]
    fn a_zero_width_terminal_does_not_divide_by_zero() {
        // A pty with no window size reports 0 columns, and this used to be the
        // arithmetic that ran first.
        let (_, _, rows) = input_layout("abc", 3, 0);
        assert!(rows >= 1);
    }
}
