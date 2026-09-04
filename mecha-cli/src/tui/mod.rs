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
mod charter;
mod command;
mod docs;
mod doctor;
mod entity;
mod find;
mod frontdoor;
mod learning;
mod mail;
mod outbox;
mod polls;
pub(crate) mod queues;
mod skills;
mod tasks;
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
use mecha_core::message::{Block as MsgBlock, Message, Usage};
use mecha_core::session::{Record, RunConfig, Session, SessionMeta};
use mecha_core::tool::{Approver, ModeApprover};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use transcript::{Entry, Transcript};

type RunResult = (Result<RunOutcome>, Conversation);

/// What the agent is doing, and everything needed to steer or stop it.
struct Running {
    handle: JoinHandle<RunResult>,
    cancel: mecha_core::agent::CancelHandle,
    /// Where a mid-run line goes. Shared with the [`RunContext`] the task holds.
    queue: Arc<Mutex<VecDeque<String>>>,
    started: std::time::Instant,
    /// Set once Ctrl-C has been pressed, so the status line can say so and a
    /// second press can mean something stronger.
    cancelling: bool,
    /// The messages already on disk when the run began — the whole list, not
    /// a count, because a mid-run compaction rewrites earlier messages and a
    /// count cannot tell "the run appended" from "the run rewrote history".
    ///
    /// Carried here because the run *owns* the conversation while it is in
    /// flight — `App::messages` is empty — so there is nothing to measure
    /// against when it comes back.
    persisted: Vec<Message>,
    /// Every outbox id that existed when the run started. What the run staged
    /// is the diff against this at completion — which is what scopes the
    /// review-now flow to *this run's* drafts and keeps `/review auto` from
    /// ever touching the overnight backlog. `None` when the snapshot could not
    /// be read, and then nothing is opened or released: acting on a guess
    /// about what a run staged is worse than a missed convenience.
    outbox_before: Option<std::collections::HashSet<String>>,
}

/// What an off-loop `/remote-control` did.
///
/// Three variants rather than a `Result`, because detaching is not the inverse
/// of attaching: it clears the handle, and a failure to post the closing line
/// must not leave the interface believing it is still mirroring when the
/// record already says it is not.
enum AttachOutcome {
    Attached(Box<crate::slack::remote::Attached>, String),
    Detached(String),
    /// The name it was attaching *as*, when there was one. Without it a
    /// failure — or a slow detach — clears a reservation belonging to a
    /// different, still-running attach, and the guard it exists to power stops
    /// firing exactly when two are in flight.
    Failed {
        name: Option<String>,
        error: String,
    },
}

/// One piece of detached work being watched for its outcome.
///
/// The polling is against the *stores*, never the child process: the store is
/// the record, a child that died without writing is indistinguishable from
/// one still working, and the `since` cap is what keeps a wedged child from
/// pinning the fast tick forever. (`Remedy` bends the first half because it
/// must: nothing durable records a `systemctl restart`, so the child's exit
/// is the cue — but the *outcome* reported is a fresh examination, never the
/// exit code alone.)
enum Watch {
    /// An outbox item whose release was spawned. `error_before` is the item's
    /// error at spawn time, because a failed release leaves the item pending
    /// with the error written on it — only an error that *changed* belongs to
    /// this attempt.
    Send {
        id: String,
        error_before: Option<String>,
        since: std::time::Instant,
    },
    /// A frontdoor record whose detached verb (extract, triage) should move
    /// its state.
    Request {
        seq: i64,
        state_before: String,
        since: std::time::Instant,
    },
    /// A doctor remedy spawned detached from the /doctor modal. When the
    /// child exits, a fresh examination is started and *that* is the outcome
    /// reported — a restarted unit can refail on its next tick, and the exit
    /// code says nothing about it.
    Remedy {
        child: std::process::Child,
        /// The command line, for the notice.
        argv_line: String,
        since: std::time::Instant,
        /// Still-running notices already posted (F5): the watch outlives a
        /// slow remedy — a trigger run gets twenty minutes — instead of
        /// abandoning it at five, and says so periodically until the hard
        /// cap kills, reaps and reports.
        notices: u32,
    },
    /// A `mecha doctor --json` examination running detached (F7). The
    /// synchronous `.output()` it replaces froze rendering and steering for
    /// as long as a sick `systemctl` cared to take. Same exception as
    /// `Remedy`: the child is the cue, the parsed JSON is the outcome —
    /// installed into the /doctor modal if it is still open, a transcript
    /// notice otherwise.
    Examine {
        child: std::process::Child,
        since: std::time::Instant,
    },
    /// A restart remedy's press-time re-examination (F4), off the event
    /// loop. The `y` handler used to run `unit_is_failed` — a blocking
    /// `systemctl` probe — inline, which froze rendering exactly when
    /// systemd was sick: the one situation a restart remedy exists for, and
    /// the same D-Bus stall `Examine` was detached over. The probe runs on
    /// its own thread instead; this watch collects the answer, execs the
    /// remedy only if the unit is still failed, and reports "already
    /// recovered" as the outcome otherwise.
    /// A `mecha mail show` fetching one thread for the /mail reader.
    ///
    /// The same exception `RestartProbe` takes, for the same reason: reading
    /// a thread in full starts an MCP server and makes a network call, and
    /// doing that on the event loop freezes the interface at the exact moment
    /// someone is waiting to read something. Nothing durable records a read,
    /// so the child's answer is the cue — on its own thread, because what is
    /// wanted back is the text and not an exit code.
    MailRead {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        /// For the reader's title and for saying which read failed.
        handle: String,
        since: std::time::Instant,
    },
    /// A `mecha tasks source …` read for the /tasks modal.
    ///
    /// [`Watch::MailRead`]'s shape one board over, and it takes the same
    /// exception for the same reason — following a mail pointer starts an MCP
    /// server and reaches the provider. Nothing durable records a read, so the
    /// child's answer is the cue.
    TaskSource {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        /// The label a person recognises the source by, for the title and for
        /// saying which read failed.
        handle: String,
        since: std::time::Instant,
    },
    /// A `mecha mail archive|spam|task …` triage action for the /mail modal.
    ///
    /// The same exception `MailRead` takes, and it was overdue: these each
    /// start an MCP server and make a network call, and running them inline
    /// froze the interface for a second or two per keystroke — the module's
    /// own comment admitted the contradiction. The child's answer is the cue
    /// (nothing durable records a triage action's completion); the refresh
    /// on landing is a local store read.
    MailAction {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        /// For the status line: which verb is in flight, on which handle.
        verb: String,
        handle: String,
        since: std::time::Instant,
    },
    /// A `mecha kg note …` capture. Fire-and-notice: a note has no modal,
    /// so the answer lands in the transcript wherever the person is now.
    KgNote {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        since: std::time::Instant,
    },
    /// A `mecha kg search --json` for the /find modal.
    KgSearch {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        query: String,
        since: std::time::Instant,
    },
    /// A `mecha kg entity --json` fetch for the /find modal's detail.
    KgEntity {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        name: String,
        since: std::time::Instant,
    },
    /// A `mecha review groups …` load for the /queues modal.
    ///
    /// Grouping embeds every pending candidate of the class, which is
    /// seconds on a thousand-item class — the one slow child the queues
    /// modal drives. The answer is the group JSON, installed only if the
    /// modal is still sitting at the class list it was asked from; a person
    /// who moved on is not yanked to a level they left.
    QueuesGroups {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        /// The class being grouped, or `None` for the global cross-class
        /// layer — which embeds the WHOLE pending queue and legitimately
        /// runs minutes, so its patience cap is its own.
        class: Option<(String, String)>,
        since: std::time::Instant,
    },
    /// A `mecha-docs …` child answering for the /docs modal.
    ///
    /// The same exception `MailRead` takes: listing the scope is a Drive
    /// request and finishing a pick is a token exchange, and either on the
    /// event loop freezes the interface at exactly the moment someone is
    /// waiting for it. On its own thread, because what is wanted back is the
    /// JSON and not an exit code.
    Docs {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        job: DocsJob,
        since: std::time::Instant,
    },
    RestartProbe {
        rx: std::sync::mpsc::Receiver<bool>,
        /// The remedy to spawn if the unit is still failed.
        argv: Vec<String>,
        unit: String,
        since: std::time::Instant,
    },
    /// A `mecha reflections|rules|proposals …` verb for the /learning modal.
    ///
    /// The same exception `MailAction` takes, and for a sharper reason: this
    /// child takes `LearningStore::lock()` — a *blocking* flock — and
    /// `reflect`/`learn` hold that same lock across a model call while they
    /// run. Waiting for it inline would freeze the whole event loop for
    /// however long that call takes, in the one modal whose job is reading a
    /// rule or lesson carefully before it enters every future prompt's
    /// cached prefix. On its own thread; the child's stdout is the outcome,
    /// same as `MailAction`.
    Learning {
        rx: std::sync::mpsc::Receiver<Result<String>>,
        verb: String,
        id: String,
        since: std::time::Instant,
    },
}

/// Which `mecha-docs` call a `Watch::Docs` is waiting on. The answer is JSON
/// either way; what differs is where it goes.
#[derive(Clone, Copy, PartialEq)]
enum DocsJob {
    /// `list --json` — the files this grant can reach.
    List,
    /// `pick --url --json` — an authorization URL, and an attempt recorded on
    /// disk for the second half to finish.
    PickUrl,
    /// `pick --redirect … --json` — the exchange, and what went into scope.
    PickDone,
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
    /// The skill tool the agent is actually using — the carried set for
    /// /skills, and which of them this conversation has loaded. Riding here
    /// for the same reason `todo` does: a `/model` switch rebuilds the agent
    /// and its tools wholesale, and a handle cached anywhere else would keep
    /// answering for the agent that was replaced.
    skill: Option<Arc<mecha_core::tool::skill::SkillTool>>,
    /// The levers this agent was built with off, carried from `Prepared`
    /// so every `config` record this session appends — at start, and after
    /// `/mode`, which rebuilds nothing — names the same value. `/mode` used
    /// to re-read `config.toml` and fold the flags into *that*, so editing
    /// `boredom = false` in the file and then switching mode recorded the
    /// lever off while the running agent kept issuing notices: "absent" of
    /// something that ran, the record's one forbidden lie (found on review,
    /// the fourth shape of it). A `/model` or `/mcp` switch rebuilds `Live`
    /// from a fresh `Prepared`, so this moves exactly when the agent does.
    levers_off: Vec<mecha_core::harness::Lever>,
    /// What the running agent carries of the learning store, for the same
    /// record and moving at the same moment as `levers_off`.
    rules: mecha_core::learning::RulesCarried,
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
            skill: p.skill,
            levers_off: p.levers_off,
            rules: p.rules,
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
    /// Selection mode: the mouse belongs to the terminal until it is turned
    /// off again, so a drag over the transcript selects instead of scrolling.
    ///
    /// A toggle rather than the default because the wheel is what capture
    /// buys, and losing it permanently to make copying possible is trading a
    /// thing used every minute for a thing used every hour. Explicit and
    /// visible in the status line for the same reason: "why has my wheel
    /// stopped scrolling" has to be answerable from the screen.
    selecting: bool,
    /// The help overlay is up. It exists to be glanced at and dismissed, so
    /// any key closes it — except the ones that scroll it, because the list
    /// is longer than a short terminal and a reference card that silently
    /// stops at `/outbox` is worse than none.
    help: bool,
    /// First body row the overlay shows. Reset every time it opens: a card
    /// that reopens halfway down is a card that looks broken.
    help_scroll: u16,
    /// The /tools modal, when open. Takes every key while it is up.
    tools: Option<tools::ToolsModal>,
    /// The /skills modal, when open. Takes every key while it is up.
    skills: Option<skills::SkillsModal>,
    /// The /charter modal, when open. Takes every key while it is up.
    charter: Option<charter::CharterModal>,
    /// `e` in /charter, deferred to the main loop for the same reason as
    /// `pending_outbox_edit` — handing the terminal to `$EDITOR` needs the
    /// terminal, which the key handler does not have.
    pending_charter_edit: bool,
    learning: Option<learning::LearningModal>,
    /// Where the skill store lives, resolved from `[skills] dir` once at
    /// startup — the same resolution the agent's own set came from, so the
    /// modal cannot end up describing a different directory than the run
    /// read.
    skills_dir: PathBuf,
    /// The /triggers modal, when open. Takes every key while it is up.
    scheduled: Option<triggers::TriggersModal>,
    /// The /outbox modal, when open. Takes every key while it is up.
    staged: Option<outbox::OutboxModal>,
    /// The /frontdoor modal, when open. Takes every key while it is up.
    requests: Option<frontdoor::FrontdoorModal>,
    mail: Option<mail::MailModal>,
    /// The /find modal: search the knowledge graph.
    find: Option<find::FindModal>,
    /// The /docs modal, when open. Takes every key while it is up.
    documents: Option<docs::DocsModal>,
    /// The /entity modal: repairing who is who in the knowledge graph.
    entities: Option<entity::EntityModal>,
    /// The /tasks modal, when open. Takes every key while it is up.
    tasks: Option<tasks::TasksModal>,
    /// The /queues modal — every store waiting on a human, including the
    /// graph's merge queue. Takes every key while it is up, like its
    /// siblings. Named for the stores rather than for the act, because
    /// `app.review` is already the outbox's release policy and two fields
    /// called review would be two different things one word away.
    queues: Option<queues::QueuesModal>,
    /// The /polls modal, when open. Takes every key while it is up.
    poll_monitor: Option<polls::PollsModal>,
    /// The /doctor modal, when open. Takes every key while it is up.
    health: Option<doctor::DoctorModal>,
    /// A doctor remedy that needs the real terminal (an OAuth flow), deferred
    /// to the event loop like an `$EDITOR` shell-out and for the same reason:
    /// suspending the TUI needs the terminal, which a key handler does not
    /// hold.
    pending_doctor_remedy: Option<mecha_core::doctor::Remedy>,
    /// A trigger file to open in $EDITOR, deferred to the event loop for the
    /// same reason `pending_editor` is: suspending the TUI needs the terminal.
    pending_trigger_edit: Option<String>,
    /// An outbox item's arguments to open in $EDITOR, same deferral.
    pending_outbox_edit: Option<String>,
    /// Pending outbox items, for the status-line badge. Refreshed at run end,
    /// on modal actions, and on the idle tick — never per frame, because the
    /// count is a directory read.
    outbox_pending: usize,
    /// §6.2's readout — how the *live* session's last finished run appraised,
    /// via `appraisal::live`. Set in `finish_run` beside `usage`; `None`
    /// before the first run and whenever the label is `Neutral`, which is
    /// what "show nothing" on the status strip keys off (§16 leaves "what to
    /// show on neutral" open; resolved here the conservative way).
    affect: Option<mecha_core::appraisal::Affect>,
    /// The dimensional half of the same readout — the signed magnitudes
    /// the record carries, shown as a number on this surface by the owner's
    /// ruling (`docs/APPRAISAL-RESEARCH.md` §3.1: a number on the TUI, a
    /// bar on the web). `None` when silent, same rule as `affect`.
    valence: Option<mecha_core::appraisal::Valence>,
    /// Detached work whose outcome should be reported without a reopen: a
    /// release, an extraction, a triage run. Polled from the tick — while any
    /// are live the idle tick tightens to a second — and a resolved watch
    /// becomes a transcript notice, a badge refresh, and a reload of whatever
    /// modal is showing the thing that changed.
    watches: Vec<Watch>,
    /// What happens when a run stages drafts. Set by `/review`, and only by
    /// `/review`: release policy must not be decidable from the prompt, which
    /// shares a context window with whatever third-party text a tool fetched.
    review: command::ReviewMode,
    /// Where a finished `!command` posts its output. The receiver lives in
    /// the event loop; running the command on a task keeps the input line
    /// live while it does.
    shell_tx: mpsc::UnboundedSender<Entry>,
    /// Where an off-loop attach reports back. Talking to Slack cannot happen
    /// on the event loop, and its outcome has to land on `App`, so this takes
    /// the same shape as `shell_tx`.
    attach_tx: mpsc::UnboundedSender<AttachOutcome>,
    /// The Slack thread this session is mirrored into.
    ///
    /// On `App` rather than `Live` deliberately: a `/model` or `/provider`
    /// switch rebuilds the agent and its tools wholesale, and an attachment
    /// that vanished when you changed model would be the `todo` handle bug in
    /// a surface where the loss is both silent and outbound.
    attached: Option<crate::slack::remote::Attached>,
    /// Images dropped onto the prompt and not yet sent, as
    /// `(chip text, block)`.
    ///
    /// Held here rather than folded into `input` because the input is a
    /// `String` a person edits with the arrow keys, and base64 is not
    /// something anyone should be able to put a cursor in the middle of.
    /// The chip is the handle: an entry is attached on submit **only if its
    /// chip is still in the text**, so deleting the chip really does detach
    /// the image, which is the only way a person can undo a drop.
    dropped: Vec<(String, MsgBlock)>,
    /// When the thread was last told a run is waiting on the terminal. A run
    /// in `ask` mode making several gated calls would otherwise post one DM
    /// each — flooding the scrollback and pushing the transport toward the
    /// per-channel rate limit, which then delays the stream it shares.
    last_waiting_note: Option<std::time::Instant>,
    /// A name whose attach is in flight.
    ///
    /// `attached` is only written when the outcome lands, so without this the
    /// "already attached" guard cannot fire in the window between the two —
    /// two enters on `/rc` would open two threads and leave the first with no
    /// handle, no closing line, and a live record nothing will ever cool.
    attaching: Option<String>,
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
    /// `/learning`'s `e`: open $EDITOR on one reflection's lesson. Same
    /// deferral and for the same reason as `pending_outbox_edit` — suspending
    /// the TUI needs the terminal, which a key handler does not hold.
    pending_lesson_edit: Option<String>,
    /// Every provider entry in config, as (name, model). Fixed for the session.
    providers: Vec<(String, String)>,
    /// Whether the terminal speaks the kitty keyboard protocol, which is what
    /// makes Shift+Enter distinguishable from Enter. Alt+Enter works either
    /// way; help text offers Shift+Enter only where it can actually arrive.
    kitty_keyboard: bool,
}

impl App {
    /// True while the terminal should own the mouse, so a drag selects text
    /// instead of being swallowed as a mouse event.
    ///
    /// Two cases, and they are priced differently.
    ///
    /// **Any modal that is up.** Capture buys exactly one thing here — the
    /// wheel scrolling the transcript — and while a modal covers the screen
    /// that scrolling happens behind it, where nobody can see it. So the
    /// mouse costs nothing to hand back and buys the thing every one of these
    /// panes is full of: an authorization link, a staged draft, somebody
    /// else's email, a doctor finding with a command in it. Asked as "is
    /// something up" rather than enumerated per modal, because a modal added
    /// later would otherwise be the one that quietly cannot be copied from.
    ///
    /// **Selection mode**, which is the transcript's answer and is a toggle
    /// for the reason `selecting` gives.
    ///
    /// Note what this does *not* fix: a bordered pane wraps long text between
    /// two `│`, and a drag across the rows takes them with it. That is what
    /// the picker's bare view (`s`) exists for, and any other pane wanting
    /// clean multi-line copy needs the same thing.
    fn wants_the_mouse_back(&self) -> bool {
        self.selecting || self.a_modal_is_up()
    }

    /// Is a full-screen pane covering the transcript?
    ///
    /// Deliberately not the key-routing chain in `handle_key`: that chain is
    /// ordered, because two modals up at once must resolve to one owner of
    /// the keyboard, and this question has no order to it. Kept beside the
    /// field list so a new modal is one line in both places.
    fn a_modal_is_up(&self) -> bool {
        self.help
            || self.pending.is_some()
            || self.asking.is_some()
            || self.picker.is_some()
            || self.tools.is_some()
            || self.skills.is_some()
            || self.charter.is_some()
            || self.scheduled.is_some()
            || self.staged.is_some()
            || self.requests.is_some()
            || self.mail.is_some()
            || self.documents.is_some()
            || self.entities.is_some()
            || self.tasks.is_some()
            || self.queues.is_some()
            || self.poll_monitor.is_some()
            || self.health.is_some()
            || self.learning.is_some()
    }

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

        // Same rule as the plan badge: drafts waiting on you is the exception
        // worth a coloured block, and zero drafts is the state that says
        // nothing. Visible while a run works too — the drafts are usually its.
        if self.outbox_pending > 0 {
            spans.push(Span::styled(
                format!(" outbox {} ", self.outbox_pending),
                Style::new().fg(Color::Black).bg(Color::Yellow),
            ));
        }

        // §6.2's readout. `None` covers both "no run has finished yet" and
        // "the last one was `Neutral`" — the overwhelming common case per
        // the rung 7 corpus — so the badge only ever appears when there is
        // something to say. One colour for every reachable label today:
        // only four exist, and none of them argues for a finer split
        // without a corpus to measure one from (rung 6's own precedent for
        // its own thresholds).
        // The badge is the number first and the word second: the valence
        // is signed on every run the record has anything to say about,
        // where the label needs a probe to say a word (`appraisal.rs`'s
        // module note, *The label is not the readout*). Both `None`: no
        // badge, and that absence is the common case on purpose.
        let mut badge = String::new();
        if let Some(affect) = self.affect {
            // `wire()`, not `{:?}` — the divergence `Affect::wire()` exists
            // to prevent (its own doc comment). Identical for all ten
            // current variants; a future two-word one would make this badge
            // and the web page's tint disagree silently.
            badge.push_str(&affect.wire());
        }
        if let Some(valence) = self.valence {
            if !badge.is_empty() {
                badge.push(' ');
            }
            badge.push_str(&valence.compact());
        }
        if !badge.is_empty() {
            // Amber on a negative reading, the muted default otherwise: a
            // run that only went well is worth a glance, not a hazard tick.
            // A label is negative unless it is one of the two positive
            // words — neither reachable today, and the sentence above would
            // be false the day one is (found on review).
            let negative = self.valence.is_some_and(|v| v.negatives > 0)
                || self.affect.is_some_and(|a| {
                    !matches!(
                        a,
                        mecha_core::appraisal::Affect::Pride
                            | mecha_core::appraisal::Affect::Excitement
                    )
                });
            spans.push(Span::styled(
                format!(" {badge} "),
                if negative {
                    Style::new().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::new().fg(Color::Black).bg(Color::Gray)
                },
            ));
        }

        // The same rule again, and here it is load-bearing rather than
        // informative: selection mode takes the scroll wheel away, and a wheel
        // that has stopped working with nothing on screen to explain it reads
        // as the session having broken.
        if self.selecting {
            spans.push(Span::styled(
                " select ^S ",
                Style::new().fg(Color::Black).bg(Color::Blue),
            ));
        }

        // A mirrored session is one whose output is leaving the machine, which
        // is exactly what the always-visible strip is for. In a modal, the
        // answer to "is anyone else seeing this" would cost a keystroke.
        if let Some(a) = &self.attached {
            spans.push(Span::styled(
                format!(" ⇄ {} ", a.name),
                Style::new().fg(Color::Black).bg(Color::Green),
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

    let session_dir = Session::default_dir()?;
    // One conversation for the session, so the taint accumulates across turns
    // the way the model's context does.
    let mut convo = Conversation::new();
    let mut session = None;

    if let Some(id) = &resume {
        let path = Session::find(&session_dir, id)?;
        let (meta, prior) = Session::load(&path)?;
        // D15, before anything renders: the live pane polls the todo handle,
        // and a resumed conversation whose plan is only in the transcript
        // shows an empty pane beside a model that knows exactly where it got
        // to.
        if let Some(todo) = &prepared.todo {
            let ws = prepared.agent.context().tools.workspace.clone();
            todo.rehydrate(&ws, &prior.messages);
        }
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
                kind: Some(mecha_core::session::SessionKind::Tui),
            },
        )?);
    }

    // On create and on resume both: a session picked up under different flags
    // is exactly what this record exists to catch.
    // Before the config record, which captures the tool list for replay.
    let max_upload_mb = prepared.config.slack.max_upload_mb;
    install_frontend_tools(&mut prepared.agent, &asker, session.as_ref(), max_upload_mb);

    if let Some(s) = &session {
        s.append(&Record::Config(RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
            &prepared.levers_off,
            Some(&prepared.rules),
        )))?;
        // Staged outbox items point back at the session that drafted them.
        if let Some(route) = &prepared.agent.context().outbox {
            route.set_session_id(&s.meta.id);
        }
        // The TUI is the same producer as `chat` — one interactive surface,
        // whichever front-end happens to be running it.
        if let Some(mb) = &prepared.mailbox {
            mb.attach("chat", &s.meta.id);
        }
    }

    let (shell_tx, mut shell_rx) = mpsc::unbounded_channel::<Entry>();
    let (attach_tx, mut attach_rx) = mpsc::unbounded_channel::<AttachOutcome>();
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
        selecting: false,
        help: false,
        help_scroll: 0,
        tools: None,
        skills: None,
        charter: None,
        pending_charter_edit: false,
        learning: None,
        pending_lesson_edit: None,
        skills_dir: prepared
            .config
            .skills
            .dir
            .clone()
            .or_else(|| mecha_core::skill::SkillStore::default_dir().ok())
            .unwrap_or_default(),
        sandbox_line: setup::sandbox_line(&prepared.sandbox),
        workspace: prepared.workspace.clone(),
        todo_visible: true,
        pending_editor: false,
        scheduled: None,
        staged: None,
        requests: None,
        mail: None,
        find: None,
        documents: None,
        entities: None,
        tasks: None,
        queues: None,
        poll_monitor: None,
        health: None,
        pending_doctor_remedy: None,
        pending_trigger_edit: None,
        pending_outbox_edit: None,
        outbox_pending: outbox_pending_count(),
        affect: None,
        valence: None,
        review: command::ReviewMode::default(),
        watches: Vec::new(),
        shell_tx,
        attach_tx,
        attached: None,
        dropped: Vec::new(),
        attaching: None,
        last_waiting_note: None,
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
            (true, true) => {
                " · already holds private data and third-party content, so outbound calls will be refused"
            }
            (true, false) => " · already holds private data",
            (false, true) => " · already holds third-party content",
            (false, false) => "",
        };
        app.transcript.push(Entry::Notice(format!(
            "resumed {} messages{carried}",
            app.convo.len()
        )));
    }

    // Kept out of `Live` so the exit path below can still reach it after the
    // event loop is done with everything else.
    let mailbox = prepared.mailbox.clone();
    if let Some(mb) = &mailbox {
        if !mb.delivers() {
            if let Ok(pending) = mb.store.pending_for("chat") {
                if !pending.is_empty() {
                    app.transcript.push(Entry::Notice(format!(
                        "{} message(s) waiting — `mecha msg list` to read them",
                        pending.len()
                    )));
                }
            }
        }
    }

    let mut live = Live::new(prepared, global.clone());
    let (mut terminal, kitty) = enter()?;
    // From here until `leave`, stderr is the alternate screen. Hold log lines
    // instead of letting them scribble through the frame; the loop drains them
    // into the transcript, where they can actually be read.
    crate::logs::capture();
    app.kitty_keyboard = kitty;
    set_title(&format!("mecha · {}", workspace_name(&app)));
    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut live,
        &mut approvals,
        &mut questions,
        &mut shell_rx,
        &mut attach_rx,
        session.as_ref(),
        &approver,
        &asker,
    )
    .await;
    leave(&mut terminal)?;
    // The screen is the user's again, so anything still held — including a
    // warning from the very last frame — goes where it was always headed.
    for line in crate::logs::release() {
        eprintln!("{line}");
    }

    // The thread is told before this process is gone. A hard kill instead of
    // an exit is covered by the record's pid reading as dead — three layers,
    // none of them load-bearing alone.
    if let Some(a) = app.attached.take() {
        if let Err(e) = crate::slack::remote::detach(&a, "the terminal session ended").await {
            eprintln!("could not close the Slack thread for `{}`: {e:#}", a.name);
        }
    }

    if let Some(s) = &session {
        println!(
            "session {} · {}",
            s.meta.id,
            crate::render::format_usage(&app.usage)
        );
        if let Some(mb) = &mailbox {
            mb.detach(&s.meta.id);
        }
        let cx = live.agent.context();
        cx.hooks
            .session_end(&s.meta.id, &s.path, &cx.tools.workspace)
            .await;
    }
    result
}

/// The approver a permission mode means.
///
/// `ask` is the one the event loop owns — a fresh terminal approver would
/// fight the interface for stdin — and everything else is mechanical.
///
/// A function because it is needed in two places and was only written in one.
/// `/mode` set it correctly; a `/model`, `/provider` or `/mcp` switch rebuilt
/// the agent around the *retained* ask-mode approver instead, so a session
/// switched to `read-only` and then to another model went on displaying
/// `read-only` while actually asking — the badge and the behaviour disagreeing,
/// in the direction that loosens. `allow` degraded the same way, harmlessly.
fn approver_for(mode: PermissionMode, retained: &Arc<dyn Approver>) -> Arc<dyn Approver> {
    match mode {
        PermissionMode::Ask => Arc::clone(retained),
        other => Arc::new(ModeApprover { mode: other }),
    }
}

/// The tools that belong to this *front-end* rather than to the agent's
/// configuration.
///
/// Installed in two places — at startup, and again after every `/model`,
/// `/provider` or `/mcp` switch, because those rebuild the agent and its
/// registry wholesale. Before this existed, a switch silently dropped all of
/// them: `ask_user` stopped existing mid-session, `recall` stopped existing,
/// and the model simply carried on without them with nothing said anywhere.
/// The same shape as the `todo` and `skill` handles riding on `Live`, and the
/// same fix — one function, so two call sites cannot disagree about the set.
fn install_frontend_tools(
    agent: &mut mecha_core::agent::Agent,
    asker: &Arc<dyn mecha_core::tool::ask::Asker>,
    session: Option<&Session>,
    max_upload_mb: u64,
) {
    // Only ever registered by the TUI: it is the one front-end that owns a
    // human to ask, and the one that can attach a session to a thread. A tool
    // that can never succeed is worse than one that is absent.
    agent
        .registry_mut()
        .insert(Arc::new(mecha_core::tool::ask::AskUserTool::new(
            Arc::clone(asker),
        )));
    agent
        .registry_mut()
        // The cap is passed in rather than read inside the tool: see the
        // field's own comment for the two hours-later parse errors that
        // argued for it.
        .insert(Arc::new(crate::slack::show::ShowFileTool::new(
            max_upload_mb,
        )));
    if let Some(s) = session {
        setup::register_recall(agent, s);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
    live: &mut Live,
    approvals: &mut mpsc::UnboundedReceiver<approve::Request>,
    questions: &mut mpsc::UnboundedReceiver<ask::Question>,
    shell_results: &mut mpsc::UnboundedReceiver<Entry>,
    attach_results: &mut mpsc::UnboundedReceiver<AttachOutcome>,
    session: Option<&Session>,
    approver: &Arc<dyn Approver>,
    asker: &Arc<dyn mecha_core::tool::ask::Asker>,
) -> Result<()> {
    let mut keys = EventStream::new();
    // Agent events arrive on a channel that is replaced per run. Holding a
    // sender here keeps the receiver alive between runs so `select!` has
    // something to poll rather than a closed branch.
    let (mut events_tx, mut events_rx) = mpsc::unbounded_channel::<AgentEvent>();
    // How often an attached session looks for text from its thread. A
    // deadline rather than a timer branch — see the check at the top of the
    // loop for why that distinction is the whole feature.
    const INBOUND_EVERY: std::time::Duration = std::time::Duration::from_millis(1_000);
    let mut last_inbound = std::time::Instant::now();

    loop {
        // Log lines held since the last frame. Into the transcript rather than
        // onto the terminal, which is the point of capturing them — and as
        // entries rather than a status flash, because a warning about a run
        // that finished on a failed tool call has to still be there when the
        // user looks up.
        for line in crate::logs::drain() {
            app.transcript.push(if crate::logs::is_alarming(&line) {
                Entry::Error(line)
            } else {
                Entry::Notice(line)
            });
        }

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
        let todo_items = live
            .todo
            .as_ref()
            .map(|t| t.items_in(&live.agent.context().tools.workspace));
        // CSI 2026: the terminal buffers everything between the pair and
        // presents it as one repaint. Follow-mode streaming scrolls the whole
        // transcript region every token, and over SSH that write arrives in
        // arbitrary packet-sized pieces — without this, visibly torn.
        // Terminals that do not know the mode ignore it by spec, so there is
        // nothing to probe.
        // Before the frame, not after: the pane that wants the mouse back is
        // the one about to be drawn, and a user reaching for the mouse the
        // instant it appears must not find it still captured.
        sync_mouse_capture(!app.wants_the_mouse_back());
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
            apply_switch(switch, app, live, approver, asker, session).await?;
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

        // Editing an outbox draft's arguments, same again.
        if let Some(id) = app.pending_outbox_edit.take() {
            suspend_and_edit_outbox(terminal, app, &id)?;
            continue;
        }

        // Editing the charter, same suspend dance.
        if app.pending_charter_edit {
            app.pending_charter_edit = false;
            suspend_and_edit_charter(terminal, app)?;
            continue;
        }

        // Editing a reflection's lesson, same again.
        if let Some(id) = app.pending_lesson_edit.take() {
            suspend_and_edit_lesson(terminal, app, &id)?;
            continue;
        }

        // A doctor remedy that needs the real terminal — an OAuth flow —
        // same suspend dance again.
        if let Some(remedy) = app.pending_doctor_remedy.take() {
            suspend_and_run_remedy(terminal, app, &remedy)?;
            continue;
        }

        if app.should_quit {
            return Ok(());
        }

        // **Inbound is checked here, not in the tick arm.** `tick` is a fresh
        // sleep constructed every iteration, so whichever branch fires first
        // discards it — and during a streaming turn `TextDelta` arrives far
        // more often than the 200ms tier, so the sleep never elapses and a
        // timer branch never runs. Steering from a phone *mid-run* is the
        // headline of the inbound path, and it was the one case that could
        // not happen. A deadline is unaffected by the restart: the loop turns
        // on every event, and this does a directory read at most once a
        // second.
        if app.attached.is_some() && last_inbound.elapsed() >= INBOUND_EVERY {
            last_inbound = std::time::Instant::now();
            deliver_inbound(app, live, session, &mut events_tx, &mut events_rx);
        }

        // A run in flight redraws on a timer so the elapsed clock ticks even
        // when nothing else is happening. A live watch tightens the idle tick
        // to a second — that is the whole polling loop behind "the result
        // lands here" — and the `since` cap in `poll_watches` is what
        // guarantees the fast tick ends.
        let tick = tokio::time::sleep(std::time::Duration::from_millis(if app.running.is_some() {
            200
        } else if !app.watches.is_empty() || app.attached.is_some() {
            // Attached, this tick is the whole inbound path: it is what makes
            // a line typed on a phone reach the run. A minute of latency would
            // make the remote half unusable, so an attachment joins the same
            // one-second tier a live watch does.
            1_000
        } else {
            60_000
        }));

        tokio::select! {
            Some(Ok(event)) = keys.next() => on_terminal_event(app, event, &mut events_tx, &mut events_rx, live, session)?,

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

            Some(request) = approvals.recv() => {
                // A run started from a phone in `ask` mode otherwise stops
                // dead: the approver emits no `AgentEvent`, so the thread
                // shows the tool's card in progress forever. `AwaitingInput`
                // exists in the Slack thread state machine precisely so
                // waiting and wedged are distinguishable; this is the same
                // distinction on the mirrored surface.
                let quiet_for = app
                    .last_waiting_note
                    .map(|t| t.elapsed() >= std::time::Duration::from_secs(60))
                    .unwrap_or(true);
                if let (Some(a), true) = (&app.attached, quiet_for) {
                    app.last_waiting_note = Some(std::time::Instant::now());
                    spawn_note(
                        a,
                        &format!(
                            "Waiting for you at the terminal: `{}` needs approval. \
                             `/mode allow` there, or answer the prompt.",
                            request.tool
                        ),
                    );
                }
                app.pending = Some(request);
            }
            Some(question) = questions.recv() => app.asking = Some(question),
            // A `!command` finished; its output enters the transcript and
            // nothing else — the model never sees it.
            Some(entry) = shell_results.recv() => app.transcript.push(entry),

            // An attach or detach finished. Only this arm writes `attached`,
            // so the handle and the transcript line cannot disagree.
            Some(outcome) = attach_results.recv() => match outcome {
                AttachOutcome::Attached(a, notice) => {
                    if app.attaching.as_deref() == Some(a.name.as_str()) {
                        app.attaching = None;
                    }
                    app.transcript.push(Entry::Notice(notice));
                    app.attached = Some(*a);
                }
                // Detaching never set `attaching`, so it has no business
                // clearing it — a slow detach landing after a new attach
                // began would otherwise open the door it was holding shut.
                AttachOutcome::Detached(notice) => {
                    app.transcript.push(Entry::Notice(notice));
                    app.attached = None;
                }
                AttachOutcome::Failed { name, error } => {
                    if name.is_some() && app.attaching == name {
                        app.attaching = None;
                    }
                    app.transcript.push(Entry::Error(error));
                }
            },

            // A finished run: collect the outcome and take the conversation back.
            outcome = wait_for_run(&mut app.running), if app.running.is_some() => {
                let persisted = app.running.as_mut().map(|r| std::mem::take(&mut r.persisted)).unwrap_or_default();
                let baseline = app.running.as_mut().and_then(|r| r.outbox_before.take());
                // **Steering the run ended before folding in.** The queue is
                // drained at the top of each turn, so a run that finishes in
                // one turn — no tool calls, just an answer — never reaches a
                // point where queued text can land. Typed at the keyboard that
                // loses a sentence; arriving from the remote inbox it loses it
                // *permanently*, because claiming already deleted it from the
                // store. Carried into a fresh turn instead of dropped.
                //
                // Joined rather than submitted one at a time: the first would
                // start a run and the rest would queue into it, which is the
                // same trap one turn later.
                let leftover: Vec<String> = app
                    .running
                    .as_mut()
                    .and_then(|r| r.queue.lock().ok().map(|mut q| q.drain(..).collect()))
                    .unwrap_or_default();
                // **Not after a cancel.** Ctrl-C on a run you have just
                // steered is very often *because* of what you steered it
                // with, and starting a fresh run on the sentence somebody
                // aborted is the worst possible reading of the gesture. The
                // queue is still drained — leaving it would fire it into the
                // next run instead — and what was dropped is named, because
                // silently discarding it is the other half of the same bug.
                let cancelled = app.running.as_ref().is_some_and(|r| r.cancelling);
                finish_run(app, outcome, persisted, baseline, session)?;
                if cancelled && !leftover.is_empty() {
                    app.transcript.push(Entry::Notice(format!(
                        "dropped {} queued line(s) — the run was stopped",
                        leftover.len()
                    )));
                } else if !leftover.is_empty() {
                    app.transcript.push(Entry::Notice(
                        "the run ended before folding these in — sending them now".into(),
                    ));
                    let carried = leftover.join("\n");
                    // `from_remote` suppresses the echo: whatever queued this
                    // already announced itself when it arrived.
                    if let Err(e) =
                        submit(app, carried, Vec::new(), &mut events_tx, &mut events_rx, live, session, true)
                    {
                        app.transcript
                            .push(Entry::Error(format!("could not carry steering over: {e:#}")));
                    }
                }
            }

            _ = tick => {
                poll_watches(app);
                // The idle tick doubles as the badge's clock: a trigger in
                // another process can stage drafts while this session sits
                // idle. Not while running — run end refreshes it anyway.
                if app.running.is_none() && app.watches.is_empty() {
                    app.outbox_pending = outbox_pending_count();
                }
            }
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
    persisted: Vec<Message>,
    baseline: Option<std::collections::HashSet<String>>,
    session: Option<&Session>,
) -> Result<()> {
    let (result, convo) = outcome;
    app.convo = convo;

    // Whether the run said everything it meant to. `/review auto` releases
    // nothing after an errored or early-stopped run: a cancelled run's drafts
    // are half a thought, and the same `is_early` lesson triage learned about
    // Ctrl-C applies to releasing as to state transitions.
    let mut finished_clean = false;

    match result {
        Ok(outcome) => {
            app.usage = Usage::default();
            app.usage.add(&outcome.usage);
            finished_clean = !outcome.stop_cause.is_early();
            if outcome.stop_cause.is_early() {
                app.transcript.push(Entry::Notice(format!(
                    "{} after {}",
                    outcome.stop_cause.describe(),
                    mecha_core::agent::turns_phrase(outcome.turns)
                )));
            }
            if let Some(s) = session {
                // Everything the run added — or, when a compaction rewrote
                // history mid-run, a rewrite record of the whole current
                // state. The opening user message was written when it was
                // submitted.
                s.record_run(&persisted, &app.convo)?;
                s.record_outcome(&outcome)?;
                s.append(&Record::Taint(app.convo.taint))?;
            }

            // §6.2's readout: how this session's just-finished run
            // appraises, right now — a different question from a
            // task's goal-closure appraisal (`mecha tasks set`), which
            // reads a finished session back off disk, possibly from
            // another process. `Neutral` (the overwhelming common case)
            // clears the badge rather than showing one. No drafts here
            // (`appraisal::live`'s own doc comment) — found on review, a
            // draft resolved on an earlier or later turn than this one
            // has nothing to do with how *this* run went.
            //
            // Outside the `session` block above on purpose: `live` is a
            // pure function of the outcome and the conversation, both in
            // hand regardless of whether a transcript is being written, and
            // the only thing the session contributed was an id stamped on a
            // record that is immediately discarded. A `--no-session` run
            // used to silently lose its badge to that accidental coupling.
            //
            // `persisted.len()`, not `app.convo.messages.len()`: `live`
            // needs where *this run's own* messages start, and
            // `persisted` is exactly that boundary — the conversation as
            // it stood right after the triggering user turn was
            // appended, before this run added anything of its own.
            let readout = mecha_core::appraisal::live_readout(
                session.map(|s| s.meta.id.as_str()).unwrap_or("unsaved"),
                &outcome,
                &app.convo,
                persisted.len(),
            );
            app.affect =
                (readout.label != mecha_core::appraisal::Affect::Neutral).then_some(readout.label);
            app.valence = (!readout.valence.is_silent()).then_some(readout.valence);
        }
        Err(e) => {
            app.transcript.push(Entry::Error(format!("error: {e:#}")));
            // Drop the dangling user turn so the next request doesn't resend
            // it — see `Conversation::roll_back_failed_turn` for why
            // restore-then-pop, and why a bare pop was wrong twice over.
            app.convo.roll_back_failed_turn(persisted.clone());
            // And the transcript must agree, or the failure survives a
            // resume (serve/chat's review finding, holding here verbatim):
            // the opening user message was written at submit, so without
            // this the file ends on a dangling user turn and `--resume`'s
            // next submit puts two user messages in a row. The rolled-back
            // list is not an extension of `persisted`, so `record_run`
            // writes the `Rewrite` that makes a resume load exactly what
            // memory holds; taint is persisted either way — a failed run
            // that read a hostile page still read it.
            if let Some(s) = session {
                // The one write whose failure reproduces the resume-time 400
                // this arm exists to prevent — surfaced in the transcript,
                // never silent.
                if let Err(e) = s.record_run(&persisted, &app.convo) {
                    app.transcript.push(Entry::Error(format!(
                        "the rollback was not recorded — resuming this session \
                         will replay the failed turn: {e:#}"
                    )));
                }
                let _ = s.append(&Record::Taint(app.convo.taint));
            }
            // A run with no `RunOutcome` has nothing to appraise — leaving
            // the previous run's badge up would read as *this* run's mood,
            // which is exactly the web page's own `sawAffectThisRun` rule
            // applied here: silence (or a failure) means neutral, not "no
            // update".
            app.affect = None;
            app.valence = None;
        }
    }

    app.running = None;
    set_title(&format!("mecha · {}", workspace_name(app)));
    settle_staged_drafts(app, baseline, finished_clean);
    Ok(())
}

/// What the finished run staged, and what to do about it — the `/review`
/// mode's dispatch point.
///
/// Scope is the id-diff against the submit-time snapshot, so every mode here
/// touches only *this run's* drafts: the overnight backlog neither opens nor
/// releases, whatever the mode. No baseline means no diff, and no diff means
/// the badge is all that updates.
fn settle_staged_drafts(
    app: &mut App,
    baseline: Option<std::collections::HashSet<String>>,
    finished_clean: bool,
) {
    app.outbox_pending = outbox_pending_count();
    let Some(baseline) = baseline else { return };
    let Ok(store) = crate::commands::outbox::open_store() else {
        return;
    };
    let Ok(items) = store.items() else { return };
    let staged = crate::review_policy::staged_since(items, &baseline);
    if staged.is_empty() {
        return;
    }

    use command::ReviewMode;
    match app.review {
        ReviewMode::Later => notice_staged(app, staged.len()),
        ReviewMode::Now => open_scoped_review(app, staged.iter().map(|i| i.id.clone()).collect()),
        ReviewMode::Auto => {
            // The whole release decision — the tainted exclusion (the
            // approval `/review auto` records was given before the run read
            // whatever armed the taint) *and* the early-stop exclusion — is
            // `review_policy::auto_releases`'s, shared with the Slack
            // connector so neither surface can forget half of it.
            let (clean, tainted): (Vec<_>, Vec<_>) = staged.into_iter().partition(|i| {
                crate::review_policy::auto_releases(
                    ReviewMode::Auto,
                    i.taint.trifecta_armed(),
                    finished_clean,
                )
            });
            if !finished_clean {
                app.transcript.push(Entry::Notice(
                    "the run stopped early — its drafts wait for review".into(),
                ));
                // The policy held everything back; `tainted` is the lot.
                open_scoped_review(app, tainted.iter().map(|i| i.id.clone()).collect());
                return;
            }
            if !clean.is_empty() {
                let mut args = vec!["outbox".to_string(), "send".to_string()];
                args.extend(clean.iter().map(|i| i.id.clone()));
                args.push("--yes".to_string());
                let argv: Vec<&str> = args.iter().map(String::as_str).collect();
                let spawned = spawn_detached(&argv);
                app.transcript.push(Entry::Notice(match &spawned {
                    Ok(_) => format!(
                        "review auto: releasing {} draft(s) — results will be reported here",
                        clean.len()
                    ),
                    Err(e) => format!(
                        "review auto: could not release {} draft(s): {e} — they stay pending",
                        clean.len()
                    ),
                }));
                if spawned.is_ok() {
                    let now = std::time::Instant::now();
                    app.watches.extend(clean.iter().map(|i| Watch::Send {
                        id: i.id.clone(),
                        error_before: i.error.clone(),
                        since: now,
                    }));
                }
            }
            if !tainted.is_empty() {
                app.transcript.push(Entry::Notice(format!(
                    "⚠ {} draft(s) were written under the trifecta and are never \
                     auto-released — review them",
                    tainted.len()
                )));
                open_scoped_review(app, tainted.iter().map(|i| i.id.clone()).collect());
            }
        }
    }
}

/// A restart probe's verdict lands where the person is looking: the /doctor
/// modal's title while it is open, the transcript otherwise — the same rule
/// an examination's verdict follows.
fn report_restart_probe(app: &mut App, line: String) {
    match &mut app.health {
        Some(modal) => modal.status = Some(line),
        None => app.transcript.push(Entry::Notice(line)),
    }
}

fn notice_staged(app: &mut App, n: usize) {
    app.transcript.push(Entry::Notice(format!(
        "{n} draft(s) staged — /outbox to review"
    )));
}

/// Check every live watch against its store, and report the ones that landed.
///
/// A resolved watch is a transcript notice; any resolution also refreshes the
/// badge and reloads whichever modal is showing the thing that changed, so
/// "reopen to see the result" stops being an instruction and becomes what the
/// screen already did. Watches that outlive their cap are dropped with a
/// still-working notice rather than kept — a wedged child must not pin the
/// one-second tick forever, and the store keeps the truth either way.
fn poll_watches(app: &mut App) {
    if app.watches.is_empty() {
        return;
    }
    let watches = std::mem::take(&mut app.watches);
    let (mut outbox_moved, mut requests_moved) = (false, false);

    for watch in watches {
        match watch {
            Watch::Send {
                id,
                error_before,
                since,
            } => {
                let item = crate::commands::outbox::open_store()
                    .and_then(|s| s.item(&id))
                    .ok();
                match item {
                    Some(item) if item.status != "pending" => {
                        app.transcript
                            .push(Entry::Notice(match item.status.as_str() {
                                "sent" => format!("sent `{id}` via `{}`", item.tool),
                                other => format!("`{id}` is now {other}"),
                            }));
                        outbox_moved = true;
                    }
                    // Still pending with a *changed* error: this attempt
                    // failed. The old error staying put says nothing.
                    Some(item) if item.error != error_before && item.error.is_some() => {
                        app.transcript.push(Entry::Notice(format!(
                            "release of `{id}` failed: {} — it stays pending — /doctor for a full report",
                            item.error.as_deref().unwrap_or("unknown")
                        )));
                        outbox_moved = true;
                    }
                    Some(_) if since.elapsed() > std::time::Duration::from_secs(300) => {
                        app.transcript.push(Entry::Notice(format!(
                            "`{id}` is still releasing after 5m — /outbox has the record — /doctor for a full report"
                        )));
                        outbox_moved = true;
                    }
                    Some(_) => app.watches.push(Watch::Send {
                        id,
                        error_before,
                        since,
                    }),
                    // Unreadable store or vanished item: the watch has nothing
                    // to stand on, and a guess would be worse than silence.
                    None => {}
                }
            }
            Watch::Request {
                seq,
                state_before,
                since,
            } => {
                let record = mecha_core::frontdoor::Frontdoor::open_default()
                    .and_then(|s| s.record(seq))
                    .ok();
                match record {
                    Some(record) if record.state != state_before => {
                        let drafts = if record.state == mecha_core::frontdoor::AWAITING_ME {
                            format!(" — {} draft(s) in /outbox", record.outbox.len())
                        } else {
                            String::new()
                        };
                        app.transcript.push(Entry::Notice(format!(
                            "request {seq}: {state_before} → {}{drafts}",
                            record.state
                        )));
                        requests_moved = true;
                        // Triage stages drafts, so the outbox side moved too.
                        outbox_moved = outbox_moved || !record.outbox.is_empty();
                    }
                    // Triage is a whole agent run; give it its twenty minutes
                    // plus slack before giving up on the fast tick.
                    Some(_) if since.elapsed() > std::time::Duration::from_secs(1800) => {
                        app.transcript.push(Entry::Notice(format!(
                            "request {seq} is still {state_before} after 30m — /frontdoor has the record — /doctor for a full report"
                        )));
                        requests_moved = true;
                    }
                    Some(_) => app.watches.push(Watch::Request {
                        seq,
                        state_before,
                        since,
                    }),
                    None => {}
                }
            }
            Watch::Remedy {
                mut child,
                argv_line,
                since,
                notices,
            } => {
                match child.try_wait() {
                    // The exit is the cue, never the outcome: what the remedy
                    // changed is answered by examining again, exactly as a
                    // release's outcome is read from the outbox store. The
                    // examination itself is detached work (F7) — its verdict
                    // lands when its own watch resolves.
                    Ok(Some(status)) => {
                        let exit = if status.success() {
                            "finished".to_string()
                        } else {
                            format!("exited with {status}")
                        };
                        app.transcript.push(Entry::Notice(format!(
                            "remedy `{argv_line}` {exit} — re-examining"
                        )));
                        start_examination(app);
                    }
                    // Still going. Keep the watch (F5): dropping it here
                    // leaked the child as a zombie and lost the outcome a
                    // twelve-minute trigger run was promised. Periodic
                    // notices until the hard cap, which kills, reaps and
                    // reports honestly.
                    Ok(None) => match doctor::remedy_poll(since.elapsed(), notices) {
                        doctor::RemedyPoll::Wait => app.watches.push(Watch::Remedy {
                            child,
                            argv_line,
                            since,
                            notices,
                        }),
                        doctor::RemedyPoll::Notice => {
                            app.transcript.push(Entry::Notice(format!(
                                "`{argv_line}` is still running after {}m — the outcome \
                                 will be reported here",
                                since.elapsed().as_secs() / 60
                            )));
                            app.watches.push(Watch::Remedy {
                                child,
                                argv_line,
                                since,
                                notices: notices + 1,
                            });
                        }
                        doctor::RemedyPoll::Kill => {
                            let _ = child.kill();
                            let _ = child.wait();
                            app.transcript.push(Entry::Notice(format!(
                                "`{argv_line}` did not finish after {}m and was stopped — \
                                 r in /doctor re-examines",
                                doctor::REMEDY_HARD_CAP.as_secs() / 60
                            )));
                        }
                    },
                    // A child that cannot be asked has nothing to stand on —
                    // but it is still this process's child: reap it rather
                    // than leaking a zombie, and say the outcome was lost.
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        app.transcript.push(Entry::Notice(format!(
                            "`{argv_line}` could not be checked ({e}) and was stopped — \
                             r in /doctor re-examines"
                        )));
                    }
                }
            }
            Watch::MailRead { rx, handle, since } => match rx.try_recv() {
                Ok(Ok(text)) => match &mut app.mail {
                    Some(modal) => {
                        modal.loading = None;
                        modal.status = None;
                        modal.reading = Some(mail::Reader::new(handle, &text));
                    }
                    // The modal was closed while it loaded. Printing a whole
                    // thread into the transcript is not the favour it looks
                    // like — say it is ready and let them ask again.
                    None => app.transcript.push(Entry::Notice(format!(
                        "{handle} finished loading after /mail closed"
                    ))),
                },
                Ok(Err(e)) => {
                    let line = format!("could not read {handle}: {e:#}");
                    match &mut app.mail {
                        Some(modal) => {
                            modal.loading = None;
                            modal.status = Some(line);
                        }
                        None => app.transcript.push(Entry::Error(line)),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(modal) = &mut app.mail {
                            modal.loading = None;
                            modal.status =
                                Some(format!("{handle} never answered — enter tries again"));
                        }
                    } else {
                        app.watches.push(Watch::MailRead { rx, handle, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(modal) = &mut app.mail {
                        modal.loading = None;
                        modal.status = Some(format!("the read of {handle} was lost"));
                    }
                }
            },
            Watch::TaskSource { rx, handle, since } => match rx.try_recv() {
                Ok(Ok(text)) => match &mut app.tasks {
                    Some(modal) => {
                        modal.status = None;
                        modal.reading = Some(mail::Reader::new(handle, &text));
                    }
                    // The modal was closed while it loaded. Printing a whole
                    // thread into the transcript is not the favour it looks
                    // like — say it is ready and let them ask again.
                    None => app.transcript.push(Entry::Notice(format!(
                        "{handle} finished loading after /tasks closed"
                    ))),
                },
                Ok(Err(e)) => {
                    // Named, never blank. A source that would not load and a
                    // task that never had one are opposite findings, and a
                    // reader showing nothing cannot tell them apart.
                    let line = format!("could not read {handle}: {e:#}");
                    match &mut app.tasks {
                        Some(modal) => modal.status = Some(line),
                        None => app.transcript.push(Entry::Error(line)),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(modal) = &mut app.tasks {
                            modal.status = Some(format!("{handle} never answered — o tries again"));
                        }
                    } else {
                        app.watches.push(Watch::TaskSource { rx, handle, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(modal) = &mut app.tasks {
                        modal.status = Some(format!("the read of {handle} was lost"));
                    }
                }
            },
            Watch::MailAction {
                rx,
                verb,
                handle,
                since,
            } => match rx.try_recv() {
                Ok(out) => {
                    let line = match out {
                        Ok(o) => o.lines().next().unwrap_or("done").to_string(),
                        Err(e) => format!("{verb} {handle}: {e:#}"),
                    };
                    match &mut app.mail {
                        Some(modal) => modal.status = Some(line),
                        None => app.transcript.push(Entry::Notice(line)),
                    }
                    refresh_mail(app);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(modal) = &mut app.mail {
                            modal.status =
                                Some(format!("{verb} {handle} never answered — check /doctor"));
                        }
                    } else {
                        app.watches.push(Watch::MailAction {
                            rx,
                            verb,
                            handle,
                            since,
                        });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(modal) = &mut app.mail {
                        modal.status = Some(format!("{verb} {handle} was lost"));
                    }
                }
            },
            Watch::KgNote { rx, since } => match rx.try_recv() {
                Ok(out) => app.transcript.push(match out {
                    Ok(o) => Entry::Notice(o.lines().next().unwrap_or("noted").to_string()),
                    Err(e) => Entry::Error(format!("note failed: {e:#}")),
                }),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        app.transcript.push(Entry::Error(
                            "the note never answered — check /doctor".into(),
                        ));
                    } else {
                        app.watches.push(Watch::KgNote { rx, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.transcript
                        .push(Entry::Error("the note was lost".into()));
                }
            },
            Watch::KgSearch { rx, query, since } => match rx.try_recv() {
                Ok(out) => {
                    let Some(m) = &mut app.find else { continue };
                    m.loading = false;
                    match out.and_then(|t| find::rows_from_pack(&t)) {
                        Ok(rows) => {
                            m.status = Some(match rows.len() {
                                0 => format!("nothing found for `{query}`"),
                                n => format!("{n} result(s) for `{query}`"),
                            });
                            m.rows = rows;
                            m.selected = 0;
                            m.typing = false;
                        }
                        Err(e) => m.status = Some(format!("search failed: {e:#}")),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(m) = &mut app.find {
                            m.loading = false;
                            m.status = Some("the search never answered — check /doctor".into());
                        }
                    } else {
                        app.watches.push(Watch::KgSearch { rx, query, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(m) = &mut app.find {
                        m.loading = false;
                        m.status = Some("the search was lost".into());
                    }
                }
            },
            Watch::KgEntity { rx, name, since } => match rx.try_recv() {
                Ok(out) => {
                    let Some(m) = &mut app.find else { continue };
                    m.loading = false;
                    match out.and_then(|t| find::entity_detail(&t)) {
                        Ok((title, lines)) => {
                            m.detail = Some((title, lines));
                            m.scroll = 0;
                        }
                        Err(e) => m.status = Some(format!("entity {name}: {e:#}")),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(m) = &mut app.find {
                            m.loading = false;
                            m.status = Some(format!("{name} never answered — check /doctor"));
                        }
                    } else {
                        app.watches.push(Watch::KgEntity { rx, name, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(m) = &mut app.find {
                        m.loading = false;
                        m.status = Some("the fetch was lost".into());
                    }
                }
            },
            Watch::QueuesGroups { rx, class, since } => match rx.try_recv() {
                Ok(Ok(text)) => {
                    let what = match &class {
                        Some((p, pr)) => format!("{p} · {pr}"),
                        None => "the whole queue".into(),
                    };
                    // Install into a modal at the level the load was asked
                    // from (the class list for a class grouping, the
                    // proposer list for the global layer) or already at the
                    // matching groups (a `[`/`]` threshold re-run). Anywhere
                    // else, a person moved on and keeps their place; the
                    // status says the work finished.
                    let entry_level = match &class {
                        Some(_) => queues::Level::Candidates,
                        None => queues::Level::Proposers,
                    };
                    match &mut app.queues {
                        Some(m)
                            if m.level == entry_level
                                || (m.level == queues::Level::Groups && m.item_class == class) =>
                        {
                            match queues::groups_from_json(&text) {
                                Ok((threshold, rows)) => {
                                    let n = rows.len();
                                    m.level = queues::Level::Groups;
                                    m.groups = rows;
                                    m.group_threshold = threshold;
                                    m.item_class = class;
                                    m.selected = 0;
                                    m.status = Some(match n {
                                        0 => "nothing repeats above the threshold".into(),
                                        n => format!(
                                            "{n} group(s) at cosine ≥ {threshold:.2} — \
                                             a/r verdicts a whole group, [/] adjusts"
                                        ),
                                    });
                                }
                                Err(e) => m.status = Some(format!("groups: {e:#}")),
                            }
                        }
                        Some(m) => {
                            m.status = Some(format!("groups of {what} ready — s re-opens them"));
                        }
                        None => app.transcript.push(Entry::Notice(format!(
                            "grouping {what} finished after /queues closed"
                        ))),
                    }
                }
                Ok(Err(e)) => {
                    let what = match &class {
                        Some((p, pr)) => format!("{p} · {pr}"),
                        None => "the whole queue".into(),
                    };
                    let line = format!("grouping {what} failed: {e:#}");
                    match &mut app.queues {
                        Some(m) => m.status = Some(line),
                        None => app.transcript.push(Entry::Error(line)),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // The global layer embeds ~7k statements and measured
                    // 40s on the real queue; give it minutes where a class
                    // gets the ordinary cap.
                    let cap = match &class {
                        Some(_) => doctor::EXAMINE_CAP,
                        None => std::time::Duration::from_secs(360),
                    };
                    if since.elapsed() > cap {
                        if let Some(m) = &mut app.queues {
                            m.status = Some("grouping never answered — is :8081 up?".into());
                        }
                    } else {
                        app.watches.push(Watch::QueuesGroups { rx, class, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(m) = &mut app.queues {
                        m.status = Some("the grouping was lost".into());
                    }
                }
            },
            Watch::Docs { rx, job, since } => match rx.try_recv() {
                Ok(answer) => install_docs_answer(app, job, answer),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(modal) = &mut app.documents {
                            modal.loading = false;
                            if let Some(pick) = &mut modal.pick {
                                pick.working = false;
                            }
                            modal.status = Some("mecha-docs never answered — r tries again".into());
                        }
                    } else {
                        app.watches.push(Watch::Docs { rx, job, since });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(modal) = &mut app.documents {
                        modal.loading = false;
                        if let Some(pick) = &mut modal.pick {
                            pick.working = false;
                        }
                        modal.status = Some("the call to mecha-docs was lost".into());
                    }
                }
            },
            Watch::RestartProbe {
                rx,
                argv,
                unit,
                since,
            } => match rx.try_recv() {
                Ok(failed) => {
                    // The shared guard, answered off the loop: exec only if
                    // the finding is still true when the probe lands.
                    if let Some(line) =
                        crate::commands::doctor::recovered_before_restart(&unit, failed)
                    {
                        report_restart_probe(app, line);
                    } else {
                        let argv_line = argv.join(" ");
                        match spawn_remedy(&argv) {
                            Ok(child) => {
                                report_restart_probe(
                                    app,
                                    format!(
                                        "running `{argv_line}` — the outcome will be \
                                         reported here"
                                    ),
                                );
                                app.watches.push(Watch::Remedy {
                                    child,
                                    argv_line,
                                    since: std::time::Instant::now(),
                                    notices: 0,
                                });
                            }
                            Err(e) => report_restart_probe(
                                app,
                                format!("could not start `{argv_line}`: {e}"),
                            ),
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // A probe that never answers is a wedged systemctl; give
                    // up on the examination cap's scale and run nothing —
                    // restarting on no proof is the thing the guard forbids.
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        report_restart_probe(
                            app,
                            format!(
                                "the {unit} probe never answered — nothing was run; \
                                 r in /doctor re-examines"
                            ),
                        );
                    } else {
                        app.watches.push(Watch::RestartProbe {
                            rx,
                            argv,
                            unit,
                            since,
                        });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    report_restart_probe(
                        app,
                        format!("the {unit} probe was lost — nothing was run"),
                    );
                }
            },
            Watch::Examine { mut child, since } => match child.try_wait() {
                Ok(Some(_)) => {
                    let verdict = match doctor::finish_examination(child) {
                        Ok(rows) => {
                            let broken = rows
                                .iter()
                                .filter(|r| r.severity == mecha_core::doctor::Severity::Broken)
                                .count();
                            let verdict = if rows.is_empty() {
                                "nothing wrong that this doctor can see".to_string()
                            } else {
                                format!("{} finding(s), {broken} broken", rows.len())
                            };
                            if app.health.is_some() {
                                install_doctor_rows(app, rows);
                            } else {
                                // The modal was closed while it examined (a
                                // remedy's refresh, say): the verdict still
                                // lands, as a notice.
                                app.transcript.push(Entry::Notice(format!(
                                    "doctor: {verdict} — /doctor has the report"
                                )));
                            }
                            verdict
                        }
                        Err(e) => {
                            let line = format!("{e:#}");
                            if app.health.is_none() {
                                app.transcript.push(Entry::Error(format!("doctor: {line}")));
                            }
                            line
                        }
                    };
                    if let Some(modal) = &mut app.health {
                        modal.examining = false;
                        modal.status = Some(verdict);
                    }
                }
                Ok(None) if since.elapsed() > doctor::EXAMINE_CAP => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let line = format!(
                        "the examination did not answer within {}s and was stopped — \
                         r in /doctor retries",
                        doctor::EXAMINE_CAP.as_secs()
                    );
                    if let Some(modal) = &mut app.health {
                        modal.examining = false;
                        modal.status = Some(line);
                    } else {
                        app.transcript
                            .push(Entry::Notice(format!("doctor: {line}")));
                    }
                }
                Ok(None) => app.watches.push(Watch::Examine { child, since }),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let line = format!("the examination could not be checked: {e}");
                    if let Some(modal) = &mut app.health {
                        modal.examining = false;
                        modal.status = Some(line);
                    } else {
                        app.transcript.push(Entry::Error(format!("doctor: {line}")));
                    }
                }
            },
            Watch::Learning {
                rx,
                verb,
                id,
                since,
            } => match rx.try_recv() {
                Ok(out) => {
                    let line = match &out {
                        Ok(_) => format!("{verb} {}", learning_short(&id)),
                        Err(e) => format!("{verb} failed: {e:#}"),
                    };
                    if let Some(m) = &mut app.learning {
                        m.busy = false;
                        m.status = Some(line);
                    }
                    reload_learning(app);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    if since.elapsed() > doctor::EXAMINE_CAP {
                        if let Some(m) = &mut app.learning {
                            m.busy = false;
                            m.status = Some(format!(
                                "{verb} {} never answered — check /doctor",
                                learning_short(&id)
                            ));
                        }
                    } else {
                        app.watches.push(Watch::Learning {
                            rx,
                            verb,
                            id,
                            since,
                        });
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(m) = &mut app.learning {
                        m.busy = false;
                        m.status = Some(format!("{verb} {} was lost", learning_short(&id)));
                    }
                }
            },
        }
    }

    if outbox_moved {
        app.outbox_pending = outbox_pending_count();
        reload_outbox(app);
    }
    if requests_moved {
        reload_frontdoor(app);
    }
}

/// Open /outbox scoped to `ids` — unless something already owns the keyboard,
/// in which case a notice is the polite version: an approval or a question is
/// a run blocked on you, and stacking a second demand over it helps neither.
fn open_scoped_review(app: &mut App, ids: Vec<String>) {
    let busy = app.pending.is_some()
        || app.asking.is_some()
        || app.picker.is_some()
        || app.tools.is_some()
        || app.skills.is_some()
        || app.charter.is_some()
        || app.scheduled.is_some()
        || app.staged.is_some()
        || app.requests.is_some()
        || app.mail.is_some()
        || app.find.is_some()
        || app.documents.is_some()
        || app.entities.is_some()
        || app.tasks.is_some()
        || app.queues.is_some()
        || app.learning.is_some()
        || app.poll_monitor.is_some()
        || app.health.is_some()
        || app.help;
    if busy {
        notice_staged(app, ids.len());
        return;
    }
    match outbox::load() {
        Ok(rows) => {
            let rows: Vec<outbox::OutboxRow> =
                rows.into_iter().filter(|r| ids.contains(&r.id)).collect();
            if rows.is_empty() {
                // Resolved or swept between the diff and this load; the
                // notice is all there is left to say.
                notice_staged(app, ids.len());
                return;
            }
            app.staged = Some(outbox::OutboxModal {
                scope: Some(ids),
                ..outbox::OutboxModal::new(rows)
            });
        }
        Err(e) => app.transcript.push(Entry::Error(format!("outbox: {e:#}"))),
    }
}

fn on_terminal_event(
    app: &mut App,
    event: Event,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    live: &Live,
    session: Option<&Session>,
) -> Result<()> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            on_key(app, key, events_tx, events_rx, live, session)
        }
        // Inserted whole, never submitted. Dropping a file onto the terminal
        // arrives here too: terminals send the *path* as pasted text, so a drop
        // is a paste as far as this is concerned.
        //
        // Which is also why a drop cannot work over SSH, and that is worth
        // knowing rather than debugging: the path pasted is the path on the
        // *laptop*, and this process resolves it on the box at the other end,
        // where it does not exist. Nothing here can fix that — the bytes never
        // left the laptop — and it is why the Slack conduit exists.
        Event::Paste(text) => {
            app.quit_armed = false;
            if paste_into_pick(app, &text) {
                return Ok(());
            }
            let insert = match dropped_images(app, &text, live) {
                Some(chips) => chips,
                None => text,
            };
            app.input.insert_str(app.cursor, &insert);
            app.cursor += insert.len();
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

/// Is this paste a file drop of images, and if so, take them.
///
/// Returns the chip text to insert in place of the paths, or `None` to insert
/// the paste unchanged.
///
/// **Every token must resolve to an existing image**, and that conjunction is
/// the safety property. A paste is not always a drop — it is also a paragraph
/// somebody copied off a web page — and a rule that attached any file whose
/// path appeared *somewhere* in pasted prose would let copied text pull bytes
/// off this disk into a request. Requiring the whole paste to be nothing but
/// paths makes "this was a drop" a decidable question rather than a guess.
///
/// A non-image file inserts its path unchanged, deliberately: that is already
/// useful, because `fs_read` can read it and the model needs to know where it
/// is. Only an image needs the other treatment, because only an image is
/// something no tool here can turn into text.
fn dropped_images(app: &mut App, paste: &str, live: &Live) -> Option<String> {
    let paths = drop_paths(paste)?;
    // Nothing is read or resized when the model has no eyes: that is a
    // megabyte of work whose only product would be a chip standing in for a
    // picture nobody can look at. The path goes in as it always did.
    if !live.agent.vision() {
        app.transcript.push(Entry::Notice(format!(
            "{} cannot see images — dropped the path instead. See `[providers.*] vision`.",
            live.agent.model()
        )));
        return None;
    }

    let mut chips = String::new();
    let mut attached = 0usize;
    let mut bytes = 0usize;
    for path in &paths {
        match mecha_core::image::block_from_path(path) {
            Ok(Some(block)) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                if let MsgBlock::Image { data, .. } = &block {
                    // Base64 is 4 characters per 3 bytes; the figure shown is
                    // the encoded size, because that is what is actually
                    // carried in every subsequent turn.
                    bytes += data.len() / 4 * 3;
                }
                let chip = format!("[image: {name}]");
                if !chips.is_empty() {
                    chips.push(' ');
                }
                chips.push_str(&chip);
                app.dropped.push((chip, block));
                attached += 1;
            }
            // Not an image: fall back to inserting the paste as typed, which
            // is what a dropped `.md` or `.csv` wants anyway.
            Ok(None) => return None,
            Err(e) => {
                app.transcript.push(Entry::Error(format!(
                    "could not take {}: {e:#}",
                    path.display()
                )));
                return None;
            }
        }
    }
    if attached == 0 {
        return None;
    }
    app.transcript.push(Entry::Notice(format!(
        "⇄ {attached} image{} attached · {}",
        if attached == 1 { "" } else { "s" },
        human_bytes(bytes)
    )));
    Some(chips)
}

/// Split a paste into paths, or `None` if it is not one.
///
/// Terminals do not agree on how a dropped path is escaped: some quote the
/// whole thing, some backslash-escape the spaces, most do nothing at all
/// because most paths have no spaces in them. All three are handled, because
/// the one that is not handled is the one with a space in it — which is every
/// screenshot macOS has ever named.
fn drop_paths(paste: &str) -> Option<Vec<std::path::PathBuf>> {
    let trimmed = paste.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    // **The whole paste as one path, first.** Terminals disagree about
    // escaping and some paste a dropped path raw, which makes
    // `/shots/a shot.png` indistinguishable from two files by splitting
    // alone. Asking the filesystem settles it: if the entire paste names a
    // file, it is one file, and no split can be more right than that.
    let whole = std::path::PathBuf::from(trimmed);
    if whole.is_file() {
        return Some(vec![whole]);
    }

    let mut out = Vec::new();
    for token in split_drop_tokens(trimmed) {
        let path = std::path::PathBuf::from(&token);
        // Must already exist. A path that does not is a person talking about
        // a file, not handing one over.
        if !path.is_file() {
            return None;
        }
        out.push(path);
    }
    (!out.is_empty()).then_some(out)
}

/// Quote-aware and backslash-aware whitespace split.
fn split_drop_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                cur.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn human_bytes(b: usize) -> String {
    if b >= 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", b as f64 / 1024.0)
    }
}

/// Take the dropped images whose chips survived editing.
///
/// Anything whose chip was deleted is dropped on the floor rather than
/// carried to the next turn: the person removed it from the sentence they
/// were writing, and an image that reappeared on a later prompt would be the
/// most confusing possible reading of that.
fn take_dropped(app: &mut App, text: &str) -> Vec<MsgBlock> {
    std::mem::take(&mut app.dropped)
        .into_iter()
        .filter(|(chip, _)| text.contains(chip.as_str()))
        .map(|(_, block)| block)
        .collect()
}

fn on_key(
    app: &mut App,
    key: KeyEvent,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    live: &Live,
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
            // In the detail the arrows scroll it, as they do in /skills: the
            // body is a tool description, and an MCP server's runs past any
            // box worth drawing.
            KeyCode::Up => modal.scroll_detail(-1),
            KeyCode::Down => modal.scroll_detail(1),
            KeyCode::PageUp => modal.scroll_detail(-10),
            KeyCode::PageDown => modal.scroll_detail(10),
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

    // The skills modal, same keys as /tools: the two are a pair and a user
    // arriving from one should not have to learn the other.
    if let Some(modal) = &mut app.skills {
        match key.code {
            KeyCode::Up if !modal.detail => modal.move_by(-1),
            KeyCode::Down if !modal.detail => modal.move_by(1),
            // In the detail view the same keys scroll the procedure — a
            // SKILL.md is a document the user wrote, so it has no length this
            // modal can assume.
            KeyCode::Up => modal.scroll_detail(-1),
            KeyCode::Down => modal.scroll_detail(1),
            KeyCode::Enter => modal.toggle_detail(),
            KeyCode::Esc | KeyCode::Char('q') => {
                if modal.detail {
                    modal.detail = false;
                } else {
                    app.skills = None;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // The charter modal, same keys as /skills — plus `e`, which is the whole
    // point: it hands the terminal to $EDITOR on the charter file itself.
    // Deferred to the main loop like every other editor hand-over, because
    // this handler does not hold the terminal.
    if let Some(modal) = &mut app.charter {
        match key.code {
            KeyCode::Up if !modal.detail => modal.move_by(-1),
            KeyCode::Down if !modal.detail => modal.move_by(1),
            KeyCode::Up => modal.scroll_detail(-1),
            KeyCode::Down => modal.scroll_detail(1),
            KeyCode::Enter => modal.toggle_detail(),
            KeyCode::Char('e') => app.pending_charter_edit = true,
            KeyCode::Esc | KeyCode::Char('q') => {
                if modal.detail {
                    modal.detail = false;
                } else {
                    app.charter = None;
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

    // The outbox and frontdoor modals, same rule again.
    if app.staged.is_some() {
        return handle_outbox_key(app, key);
    }
    if app.requests.is_some() {
        return handle_frontdoor_key(app, key);
    }
    if app.mail.is_some() {
        return handle_mail_key(app, key);
    }
    if app.find.is_some() {
        return handle_find_key(app, key);
    }
    if app.documents.is_some() {
        return handle_docs_key(app, key);
    }
    if app.entities.is_some() {
        return handle_entity_key(app, key);
    }
    if app.queues.is_some() {
        return handle_queues_key(app, key);
    }
    if app.learning.is_some() {
        return handle_learning_key(app, key);
    }
    if app.tasks.is_some() {
        return handle_tasks_key(app, key);
    }
    if app.poll_monitor.is_some() {
        return handle_polls_key(app, key);
    }
    if app.health.is_some() {
        return handle_doctor_key(app, key, live, session);
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
                        return run_command(app, cmd, live, session);
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
        // The scroll keys are the exception to "any key closes": the card is
        // taller than a short terminal, and the half below the fold is the
        // half nobody has memorised.
        match key.code {
            KeyCode::Up | KeyCode::PageUp => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
                return Ok(());
            }
            KeyCode::Down | KeyCode::PageDown => {
                app.help_scroll = app.help_scroll.saturating_add(1);
                return Ok(());
            }
            _ => {}
        }
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
                run.cancel.cancel(mecha_core::agent::CancelReason::Stopped);
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

        // Give the mouse back to the terminal so a drag selects the
        // transcript. `^S` and not a slash command: it is wanted at the
        // moment something worth copying appears, and a command means typing
        // into the box while looking at the thing you are about to lose. The
        // notice says how to get the wheel back, because a scroll wheel that
        // has stopped working is not a state anyone will guess the cause of.
        //
        // Safe as a chord despite its flow-control history: raw mode clears
        // IXON, so the terminal delivers it here rather than freezing output.
        KeyCode::Char('s') if ctrl => {
            app.selecting = !app.selecting;
            app.transcript.push(Entry::Notice(if app.selecting {
                "selecting — drag to select, your terminal's own copy · ^S gives the wheel back"
                    .into()
            } else {
                "selecting off — the wheel scrolls again".into()
            }));
        }

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
                // **The chip is the handle, so deleting it detaches.** A
                // dropped image is otherwise impossible to take back: the
                // bytes are not in the input line, so no amount of
                // backspacing reaches them, and the only visible sign of the
                // attachment would be a piece of text that did nothing. What
                // is on screen has to be what gets sent.
                let images = take_dropped(app, &text);
                submit(
                    app, text, images, events_tx, events_rx, live, session, false,
                )?;
            }
        }

        // Only on an empty line: with anything typed, `?` is a character in a
        // question the user is writing.
        KeyCode::Char('?') if app.input.is_empty() => {
            app.help = true;
            app.help_scroll = 0;
        }

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
#[allow(clippy::too_many_arguments)]
async fn apply_switch(
    switch: Switch,
    app: &mut App,
    live: &mut Live,
    approver: &Arc<dyn Approver>,
    asker: &Arc<dyn mecha_core::tool::ask::Asker>,
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
        agent.set_approver(approver_for(mode, approver));
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

    // **The mode in force, not the one the process started in.** See
    // `approver_for`: passing the retained approver here reverted the session
    // to asking while the status line still claimed otherwise.
    let prepared = match setup::prepare_with_approver(&opts, approver_for(app.mode, approver)).await
    {
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

    // **Re-installed, or the switch quietly takes them away.** A rebuilt agent
    // gets a registry from config alone, and everything this front-end added
    // is gone — which is invisible, because a model with no `show_file` simply
    // describes the chart instead.
    let mut prepared = prepared;
    // Re-read here on purpose, and only here: a switch rebuilds the agent
    // from config already, so this is the moment a config edit is meant to
    // take effect — and a broken file fails the switch the user just asked
    // for rather than a tool call an hour later.
    let max_upload_mb = prepared.config.slack.max_upload_mb;
    install_frontend_tools(&mut prepared.agent, asker, session, max_upload_mb);
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
    // The levers come from `Live`, never from the file just loaded: the file
    // may have changed since the agent was built, and `/mode` rebuilds
    // nothing, so a record computed from it would describe an agent that is
    // not running. `live.levers_off` is the value the running agent was
    // built with, and it is replaced exactly when the agent is.
    let mut record = RunConfig::of(
        &live.agent,
        &cfg,
        &live.provider,
        &live.levers_off,
        Some(&live.rules),
    );
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
    live: &Live,
    session: Option<&Session>,
) -> Result<()> {
    use command::Command;
    let agent = &live.agent;

    let mut say = |text: String| app.transcript.push(Entry::Notice(text));

    match cmd {
        Command::Help => {
            app.help = true;
            app.help_scroll = 0;
        }

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
                detail_scroll: 0,
                sandbox_line: app.sandbox_line.clone(),
            });
        }

        Command::Learning => match load_learning(learning::Pane::Reflections) {
            Ok(rows) => {
                app.learning = Some(learning::LearningModal::new(
                    learning::Pane::Reflections,
                    rows,
                ))
            }
            Err(e) => app
                .transcript
                .push(Entry::Error(format!("learning: {e:#}"))),
        },

        Command::Skills => {
            // Two reads, because neither source can answer alone. The agent
            // knows what this run carries and what it has loaded; only the
            // store knows what else is on disk and what failed to parse.
            let (store, errors) = mecha_core::skill::SkillStore::load(&app.skills_dir);
            let carried: Vec<&mecha_core::skill::Skill> =
                live.skill.iter().flat_map(|h| h.available()).collect();
            let loaded = live.skill.as_ref().map(|h| h.loaded()).unwrap_or_default();

            let mut rows: Vec<skills::SkillRow> = store
                .all()
                .iter()
                .map(|on_disk| {
                    // For a carried skill the agent's copy is the one that
                    // matters, and it is not always the file: skills are read
                    // once at startup, into the cached prefix, so an edit
                    // since then has changed the file and not the run. Showing
                    // the new body under a `loaded` badge, or a new `tools:`
                    // list as "narrows the tool surface to", would describe a
                    // procedure this conversation is not carrying and a
                    // narrowing that is not in force.
                    let mine = carried.iter().find(|s| s.name == on_disk.name);
                    let s = mine.copied().unwrap_or(on_disk);
                    skills::SkillRow {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        triggers: s.triggers.clone(),
                        narrows: s.tools.clone(),
                        body: s.body.clone(),
                        dir: s.dir.clone(),
                        carried: mine.is_some(),
                        loaded: loaded.iter().any(|n| n == &s.name),
                        error: None,
                    }
                })
                .collect();

            // A skill the run carries that the store no longer holds — the
            // file was edited or removed since startup. It stays on the list
            // because the *run* still has it: skills are read once, into the
            // cached prefix, so what is on disk now is not what this session
            // is carrying. Dropping it would make the modal disagree with the
            // agent it is describing.
            for skill in live.skill.iter().flat_map(|h| h.available()) {
                if rows.iter().any(|r| r.name == skill.name) {
                    continue;
                }
                rows.push(skills::SkillRow {
                    name: skill.name.clone(),
                    description: format!(
                        "{} (no longer in the store — carried from startup)",
                        skill.description
                    ),
                    triggers: skill.triggers.clone(),
                    narrows: skill.tools.clone(),
                    body: skill.body.clone(),
                    dir: skill.dir.clone(),
                    carried: true,
                    loaded: loaded.iter().any(|n| n == &skill.name),
                    error: None,
                });
            }

            // Failures last, and present rather than logged: `setup` warns
            // about these on stderr before the TUI takes the screen, so the
            // alternate screen covers the warning for the whole session.
            rows.extend(errors.into_iter().map(|e| {
                skills::SkillRow {
                    name: e
                        .dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string(),
                    description: String::new(),
                    triggers: Vec::new(),
                    narrows: None,
                    body: String::new(),
                    dir: e.dir,
                    carried: false,
                    loaded: false,
                    error: Some(e.why),
                }
            }));

            app.skills = Some(skills::SkillsModal {
                rows,
                selected: 0,
                detail: false,
                detail_scroll: 0,
                dir: app.skills_dir.clone(),
            });
        }

        Command::Charter => match mecha_core::charter::Charter::default_path() {
            Ok(path) => app.charter = Some(charter::CharterModal::load(path)),
            Err(e) => say(format!("charter: {e:#}")),
        },

        Command::Triggers => match triggers::load(5) {
            Ok(rows) => app.scheduled = Some(triggers::TriggersModal::new(rows)),
            Err(e) => say(format!("triggers: {e:#}")),
        },

        Command::Outbox => match outbox::load() {
            Ok(rows) => {
                app.outbox_pending = rows.iter().filter(|r| r.pending()).count();
                app.staged = Some(outbox::OutboxModal::new(rows));
            }
            Err(e) => say(format!("outbox: {e:#}")),
        },

        Command::Review(None) => say(format!(
            "review {} — {}. /review now|later|auto switches; /queues is the backlog",
            app.review.name(),
            app.review.describe()
        )),
        Command::Review(Some(mode)) => {
            app.review = mode;
            say(format!("review {} — {}", mode.name(), mode.describe()));
        }
        Command::BadReview(word) => say(format!("`{word}`? review is one of: now, later, auto")),

        Command::Frontdoor => match frontdoor::load() {
            Ok(rows) => app.requests = Some(frontdoor::FrontdoorModal::new(rows)),
            Err(e) => say(format!("frontdoor: {e:#}")),
        },

        Command::Mail => match mail::load() {
            Ok(rows) if rows.is_empty() => {
                say("nothing classified yet — `mecha mail classify` fills the queue".into())
            }
            Ok(rows) => app.mail = Some(mail::MailModal::new(rows)),
            Err(e) => say(format!("mail: {e:#}")),
        },

        // A capture, not a modal: the note goes where you are, off the event
        // loop (it starts an MCP server), and the landing is a notice.
        Command::Note(text) => match text {
            None => say("usage: /note <what happened> — it lands as a graph episode".into()),
            Some(text) => {
                app.transcript
                    .push(Entry::Notice(format!("noting: {text}")));
                spawn_kg_note(app, &text);
            }
        },

        // The modal opens at once — empty or already searching — because a
        // box that appears only after the network answers reads as a key
        // that did nothing (the /docs rule).
        Command::Find(query) => {
            let mut modal = find::FindModal::new(query.clone());
            if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
                modal.typing = false;
                modal.loading = true;
                modal.status = Some(format!("searching `{q}`…"));
                app.find = Some(modal);
                spawn_kg_search(app, &q);
            } else {
                app.find = Some(modal);
            }
        }

        // Opened at once in a loading state, with the listing fetched off the
        // event loop: it is a Drive request, and a modal that appears only
        // after the network answers reads as a key that did nothing.
        Command::Entity => {
            app.entities = Some(entity::EntityModal::new());
        }

        Command::Docs => match docs_accounts() {
            accounts if accounts.is_empty() => {
                say("no documents grant yet — run `mecha-docs auth` once, then \
                 `mecha-docs pick` or /docs to put a document in scope"
                    .into())
            }
            accounts => {
                let account = accounts[0].clone();
                app.documents = Some(docs::DocsModal::new(account.clone(), accounts));
                spawn_docs(
                    app,
                    DocsJob::List,
                    &["--account", &account, "list", "--json"],
                );
            }
        },

        Command::Tasks => match load_tasks(false) {
            Ok(modal) => app.tasks = Some(modal),
            Err(e) => say(format!("tasks: {e:#}")),
        },

        Command::Queues => match load_queues() {
            Ok(modal) => app.queues = Some(modal),
            Err(e) => say(format!("queues: {e:#}")),
        },

        Command::Polls => match polls::load() {
            Ok(rows) => app.poll_monitor = Some(polls::PollsModal::new(rows)),
            Err(e) => say(format!("polls: {e:#}")),
        },

        Command::Doctor => {
            // The modal opens at once in an examining state; the rows land
            // when the detached examination answers (F7). Steering and
            // rendering never wait on `systemctl`.
            app.health = Some(doctor::DoctorModal::examining());
            start_examination(app);
        }

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
            // Same rule, one field over: `affect` describes the *last run
            // of the conversation just discarded*, and leaving it up would
            // read as this brand-new conversation's own mood until the
            // next run finishes.
            app.affect = None;
            app.valence = None;
            // Whatever the tools were holding for it goes too, for the same
            // reason the taint does. A `skill` narrowing that survived a clear
            // would constrain a task nobody had started yet.
            agent.registry().forget_conversation_state();
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

        Command::Send(None) => app.transcript.push(Entry::Error(
            "/send needs a path — try `/send report.png`".into(),
        )),

        // **Resolved through the run's path jail, on the event loop.** Doing
        // it here rather than in the task is what makes "no such file" arrive
        // as itself, immediately, instead of arriving a second later mixed in
        // with whatever the network had to say. The jail is the same one every
        // other path in a session goes through: a user who means to send
        // something from outside the workspace can `!cp` it in, and one rule
        // for where paths point beats an exception that exists because typing
        // is tedious.
        Command::Send(Some(raw)) => match agent.context().tools.resolve(&raw) {
            Ok(path) => {
                app.transcript
                    .push(Entry::Notice(format!("sending {raw} to Slack…")));
                spawn_send(path, app.shell_tx.clone());
            }
            Err(e) => app
                .transcript
                .push(Entry::Error(format!("/send {raw}: {e:#}"))),
        },

        Command::RemoteControl(command::Remote::Show) => match &app.attached {
            Some(a) => say(format!(
                "attached as `{}` — this session is mirrored into your Slack DM",
                a.name
            )),
            None => say(
                "not attached — `/remote-control <name>` mirrors this session into a named \
                 Slack thread"
                    .into(),
            ),
        },

        Command::RemoteControl(command::Remote::Off) => match app.attached.take() {
            Some(a) => spawn_detach(a, "detached from the terminal", app.attach_tx.clone()),
            // An attach in flight cannot be cancelled from here — the task is
            // already talking to Slack — so this says so rather than silently
            // doing nothing and letting the attachment land against the
            // user's last instruction.
            None if app.attaching.is_some() => say(
                "an attach is still in flight — run `/remote-control off` again once it lands"
                    .into(),
            ),
            None => say("not attached".into()),
        },

        // Refused rather than swapped: re-pointing a live mirror at a second
        // thread mid-session would leave the first one silently ended with no
        // line in it saying so.
        Command::RemoteControl(command::Remote::Attach(name)) => match (&app.attached, session) {
            _ if app.attaching.is_some() => app.transcript.push(Entry::Error(format!(
                "already attaching as `{}` — wait for it to land",
                app.attaching.clone().unwrap_or_default()
            ))),
            (Some(current), _) => app.transcript.push(Entry::Error(format!(
                "already attached as `{}` — `/remote-control off` first",
                current.name
            ))),
            // Without a session there is no id to key the record on, and that
            // record is what the connector reads to learn a thread is spoken
            // for. Refusing beats attaching something nothing can find again.
            (None, None) => app.transcript.push(Entry::Error(
                "this session is not being recorded (--no-session), so there is nothing to \
                 attach"
                    .into(),
            )),
            (None, Some(s)) => {
                app.transcript
                    .push(Entry::Notice(format!("attaching as `{name}`…")));
                app.attaching = Some(name.clone());
                spawn_attach(
                    name,
                    s.meta.id.clone(),
                    agent.context().tools.workspace.clone(),
                    live.model.clone(),
                    (app.convo.taint.private, app.convo.taint.untrusted),
                    app.convo.len(),
                    app.attach_tx.clone(),
                );
            }
        },

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
#[allow(clippy::too_many_arguments)]
fn submit(
    app: &mut App,
    text: String,
    // Images to put in front of the model on this turn, already bounded by
    // `mecha_core::image`. Separate from `text` rather than folded into it
    // because everything above — the steering queue, the slash-command check,
    // the echo — is about the sentence, and only the message built at the end
    // is about the pixels.
    images: Vec<MsgBlock>,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    live: &Live,
    session: Option<&Session>,
    // Whether this text arrived from the mirrored Slack thread rather than
    // from the keyboard. It suppresses the echo: the line is already in the
    // thread, put there by the person who typed it, and posting it back would
    // have the bot quoting the user to themselves.
    from_remote: bool,
) -> Result<()> {
    let agent = &live.agent;
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
        return run_command(app, cmd, live, session);
    }

    if let Some(run) = &app.running {
        // Steering. The loop picks this up at the top of its next turn and
        // folds it in beside the tool results, so the model reads it without
        // the run being stopped and restarted.
        if let Some(a) = &app.attached.as_ref().filter(|_| !from_remote) {
            // No ordering to keep here: the stream is already open, so the
            // echo simply lands after it, which is what happened. Dropped
            // explicitly rather than `let _`, which on a `JoinHandle` reads as
            // an ignored future — the task is meant to detach and finish.
            drop(spawn_echo(a, &text, true));
        }
        if let Ok(mut queue) = run.queue.lock() {
            queue.push_back(text);
        }
        return Ok(());
    }

    // Text first, images after: the order both provider families document,
    // and the one `encode_message` preserves.
    let mut blocks = vec![MsgBlock::text(&text)];
    blocks.extend(images);
    // Folded, not pushed, when the tail is already a user message — Ctrl-C
    // mid-tool-turn keeps the partial turn (cancel's contract), so the
    // conversation can end on the user message carrying tool results, and
    // pushing there makes two user messages in a row. The fold is recorded
    // immediately (a rewrite, since it mutates the tail in place): `run`'s
    // `persisted` snapshot is taken at spawn, *after* this, and every
    // downstream contract — record_run's diff, the error arm's rollback —
    // assumes that snapshot is what the file holds.
    if app
        .convo
        .messages
        .last()
        .is_some_and(|m| m.role == mecha_core::message::Role::User)
    {
        if let Some(last) = app.convo.messages.last_mut() {
            last.content.extend(blocks);
        }
        if let Some(s) = session {
            // A direct `Rewrite`, not `record_run` — found on review:
            // between runs `convo.rewritten` still holds the *previous*
            // run's compaction states (only `run_in` clears it, at run
            // start), and `record_run` replays them first, writing
            // redundant full-history copies whose torn middle resumes at a
            // stale pre-compaction state. Those states are already on disk
            // from the previous run's own record; this fold is one mutation
            // and earns exactly one record.
            s.append(&Record::Rewrite {
                messages: app.convo.messages.clone(),
            })?;
        }
    } else {
        let user = Message {
            role: mecha_core::message::Role::User,
            content: blocks,
        };
        app.convo.push(user.clone());
        if let Some(s) = session {
            s.append(&Record::Message(user))?;
        }
    }
    // Kept, so the stream can wait for it. Both are posted to the same thread
    // and Slack orders by the timestamp *it* assigns, so firing them
    // concurrently is a race — and the losing arrangement reads as the answer
    // arriving before the question, which makes a scrollback somebody returns
    // to actively misleading rather than merely untidy.
    let echoed = app
        .attached
        .as_ref()
        .filter(|_| !from_remote)
        .map(|a| spawn_echo(a, &text, false));
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

    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let cx = agent
        .context()
        .as_ref()
        .clone()
        .with_cancel(CancellationToken::new())
        .with_phase(app.phase)
        .with_queued_input(Arc::clone(&queue));
    // The handle, not the bare token: Ctrl-C cancels *as the owner*, and the
    // outcome records `Stopped` rather than the unknown-which `Interrupted`.
    let cancel = cx.cancel_handle().expect("with_cancel just set it");

    // **Attached, one run's events go to two places.** The agent takes a single
    // sender, so the split is a task rather than a second subscription:
    // everything is forwarded to the interface and a clone of it to the Slack
    // pump. `AgentEvent` is `Clone`, so nothing is reconstructed and the two
    // views cannot disagree about what happened or in what order.
    //
    // The interface is sent first on purpose. The person at the terminal is
    // the one who can act on what they see, and an unbounded channel means
    // neither side can slow the other down.
    let run_tx = match &app.attached {
        None => tx,
        Some(a) => {
            let (from_agent, mut split_rx) = mpsc::unbounded_channel::<AgentEvent>();
            let (to_slack, slack_rx) = mpsc::unbounded_channel::<AgentEvent>();
            let (slack, channel, thread_ts) =
                (a.slack.clone(), a.channel_id.clone(), a.thread_ts.clone());
            let cfg = crate::slack::pump::PumpConfig {
                flush_chars: a.flush_chars,
                flush_ms: a.flush_ms,
            };
            tokio::spawn(async move {
                // Ordering, not synchronisation: the stream must not open
                // before the line it is answering has landed.
                // Bounded. A 429 on the echo honours `Retry-After` and can
                // sleep for a minute, and waiting that long would hold the
                // *entire* answer out of the thread — a worse outcome than
                // the mis-ordering this is here to prevent. Ordering is worth
                // a moment, not the mirror.
                if let Some(echoed) = echoed {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), echoed).await;
                }
                crate::slack::pump::pump(&slack, &channel, &thread_ts, slack_rx, &cfg).await;
            });
            tokio::spawn(async move {
                while let Some(event) = split_rx.recv().await {
                    let _ = tx.send(event.clone());
                    let _ = to_slack.send(event);
                }
            });
            from_agent
        }
    };

    let agent = Arc::clone(agent);
    // Everything up to and including the message just submitted is on disk.
    let persisted = app.convo.messages.clone();
    let mut convo = std::mem::take(&mut app.convo);
    let handle = tokio::spawn(async move {
        let result = agent.run_in(&cx, &mut convo, Some(run_tx)).await;
        (result, convo)
    });

    app.running = Some(Running {
        handle,
        cancel,
        queue,
        started: std::time::Instant::now(),
        cancelling: false,
        persisted,
        outbox_before: outbox_ids(),
    });
    Ok(())
}

/// Every outbox item id right now, or `None` if the store cannot be read.
///
/// The submit-time half of "what did this run stage": cheap enough to take on
/// every submit (a directory listing), and resolved through the same config
/// path as the review surfaces so the diff cannot be against a different
/// store.
fn outbox_ids() -> Option<std::collections::HashSet<String>> {
    let store = crate::commands::outbox::open_store().ok()?;
    Some(store.items().ok()?.into_iter().map(|i| i.id).collect())
}

/// How many items are waiting on a human, for the status-line badge.
/// Zero on any failure: the badge is an observer, and an observer must not
/// be load-bearing.
fn outbox_pending_count() -> usize {
    crate::commands::outbox::open_store()
        .and_then(|s| s.items())
        .map(|items| items.iter().filter(|i| i.status == "pending").count())
        .unwrap_or(0)
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
                modal.status = Some(match spawn_detached(&["trigger", "run", &name]) {
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

/// Keys for the /outbox modal.
///
/// Same shape as the triggers handler: every mutation shells out to
/// `mecha outbox ...`, so the modal can do exactly what the command line can
/// and no more. What differs is what asks first — **every send confirms**,
/// because a send is the one keystroke here that cannot be taken back, and a
/// tainted draft confirms with its arguments on screen.
fn handle_outbox_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.staged else {
        return Ok(());
    };

    // A pending approval swallows the keyboard: y releases, anything else
    // keeps the draft pending.
    if let Some(confirm) = modal.confirm.as_mut() {
        // Scrolling is not answering. A tainted draft's arguments can run far
        // past the box, so the reviewer has to move through them without the
        // keypress counting as "anything else" and quietly dropping the
        // confirmation — which would make a long draft the one you can
        // neither read nor approve.
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                confirm.scroll = confirm.scroll.saturating_sub(1);
                return Ok(());
            }
            KeyCode::Down | KeyCode::Char('j') => {
                confirm.scroll = confirm.scroll.saturating_add(1);
                return Ok(());
            }
            KeyCode::PageUp => {
                confirm.scroll = confirm.scroll.saturating_sub(10);
                return Ok(());
            }
            KeyCode::PageDown => {
                confirm.scroll = confirm.scroll.saturating_add(10);
                return Ok(());
            }
            KeyCode::Home => {
                confirm.scroll = 0;
                return Ok(());
            }
            _ => {}
        }
        let confirm = modal.confirm.take().expect("checked above");
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            // Detached, like a trigger's "run now": the release builds a tool
            // surface (MCP servers included), which has no place on the event
            // loop. `--yes` is safe *because* the confirmation just happened
            // here — the item's status and error field are where the result
            // lands, and the watch below reports it when it does.
            let outcome = spawn_detached(&["outbox", "approve", &confirm.id, "--yes"]);
            let watch = outcome.is_ok();
            modal.status = Some(match outcome {
                Ok(_) => format!(
                    "releasing `{}` — the result will be reported here",
                    confirm.id
                ),
                Err(e) => format!("could not start the release: {e}"),
            });
            if watch {
                app.watches.push(Watch::Send {
                    id: confirm.id,
                    error_before: confirm.error_before,
                    since: std::time::Instant::now(),
                });
            }
            reload_outbox(app);
        }
        return Ok(());
    }

    // The rejection reason being typed owns the keyboard. Esc backs out with
    // nothing rejected; Enter rejects, with the reason if one was given.
    if modal.rejecting.is_some() {
        match key.code {
            KeyCode::Esc => modal.rejecting = None,
            KeyCode::Enter => {
                let input = modal.rejecting.take().expect("checked above");
                let reason = input.buffer.trim().to_string();
                let mut args = vec!["outbox", "reject", input.id.as_str()];
                if !reason.is_empty() {
                    args.extend(["--reason", reason.as_str()]);
                }
                modal.status = Some(match self_cli(&args) {
                    Ok(_) => format!("rejected `{}`; nothing was sent", input.id),
                    Err(e) => format!("could not reject `{}`: {e}", input.id),
                });
                reload_outbox(app);
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut modal.rejecting {
                    input.buffer.pop();
                }
            }
            _ => {
                if let (Some(c), Some(input)) = (typed_char(&key), &mut modal.rejecting) {
                    input.buffer.push(c);
                }
            }
        }
        return Ok(());
    }

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
                app.staged = None;
            }
        }
        // The exact bytes, for checking what the readable view reshaped.
        KeyCode::Char('J') if modal.detail => {
            modal.show_raw = !modal.show_raw;
            modal.detail_scroll = 0;
        }
        // Decided items, hidden by default. Only from the list: the detail
        // is about one item, and a filter that silently changed which item
        // is under the cursor is the accident this modal must not have.
        KeyCode::Char('h') if !modal.detail => modal.toggle_history(),
        // `a` for approve, beside `e` edit and `r` reject. `s` still works:
        // it meant this same action until 2026-08-21, it cannot mean
        // anything else here, and muscle memory on the one key that
        // releases an outbound action is worth keeping.
        KeyCode::Char('a') | KeyCode::Char('s') => {
            if let Some(row) = modal.selected_row() {
                if row.pending() {
                    modal.confirm = Some(outbox::SendConfirm {
                        scroll: 0,
                        id: row.id.clone(),
                        summary: row.summary.clone(),
                        tainted: row.tainted,
                        args_text: row.args_text.clone(),
                        error_before: row.error.clone(),
                    });
                } else {
                    modal.status = Some(format!("`{}` is {}, not pending", row.id, row.status));
                }
            }
        }
        // Editing suspends the whole TUI, so it is deferred to the event loop
        // like a trigger edit. A publish is refused with the real action
        // named, exactly as the CLI refuses it.
        KeyCode::Char('e') => {
            if let Some(row) = modal.selected_row() {
                if !row.pending() {
                    modal.status = Some(format!("`{}` is {}, not pending", row.id, row.status));
                } else if row.kind == mecha_core::outbox::OutboxKind::Publish {
                    modal.status = Some(
                        "a publish is not editable — edit the source, re-render, \
                         and publish again, which stages a new item"
                            .into(),
                    );
                } else {
                    app.pending_outbox_edit = Some(row.id.clone());
                }
            }
        }
        KeyCode::Char('r') => {
            if let Some(row) = modal.selected_row() {
                if row.pending() {
                    modal.rejecting = Some(outbox::ReasonInput {
                        id: row.id.clone(),
                        buffer: String::new(),
                    });
                } else {
                    modal.status = Some(format!("`{}` is {}, not pending", row.id, row.status));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuild the /outbox modal's rows, keeping the cursor — and the scope —
/// where they were: acting on one of a run's drafts must not widen the view
/// to the whole backlog. The badge rides along, counted before the scope
/// filter so it always describes the store.
fn reload_outbox(app: &mut App) {
    let (selected, detail, status, scope, show_raw, history) = match &app.staged {
        Some(m) => (
            m.selected,
            m.detail,
            m.status.clone(),
            m.scope.clone(),
            m.show_raw,
            m.history,
        ),
        None => return,
    };
    match outbox::load() {
        Ok(rows) => {
            app.outbox_pending = rows.iter().filter(|r| r.pending()).count();
            let rows: Vec<outbox::OutboxRow> = match &scope {
                Some(ids) => rows.into_iter().filter(|r| ids.contains(&r.id)).collect(),
                None => rows,
            };
            let mut modal = outbox::OutboxModal {
                status,
                scope,
                show_raw,
                history,
                ..outbox::OutboxModal::new(rows)
            };
            // Clamped against what is *shown*, not against the whole record:
            // with history hidden, the two differ, and a cursor past the end
            // of the visible list is a `s` aimed at nothing.
            let visible = modal.shown().len();
            modal.selected = selected.min(visible.saturating_sub(1));
            // A detail view of an item that just left the list has nothing to
            // show — a send resolves the draft it was opened on.
            modal.detail = detail && visible > 0;
            app.staged = Some(modal);
        }
        Err(e) => {
            app.staged = None;
            app.transcript.push(Entry::Error(format!("outbox: {e:#}")));
        }
    }
}

/// Keys for the /frontdoor modal.
///
/// Extract and triage spawn detached — one is a model call per record, the
/// other a whole agent run — and their results are read back from the store
/// on reload, like a trigger's. `close` refuses an empty reason, the same
/// contract as the CLI's required `--reason`.
fn handle_frontdoor_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.requests else {
        return Ok(());
    };

    // A note being typed owns the keyboard.
    if modal.input.is_some() {
        match key.code {
            KeyCode::Esc => modal.input = None,
            KeyCode::Enter => {
                let input = modal.input.take().expect("checked above");
                let note = input.buffer.trim().to_string();
                let seq = input.seq.to_string();
                let outcome = match input.action {
                    frontdoor::NoteAction::Close if note.is_empty() => {
                        // Refused loudly rather than closed silently: `any →
                        // closed` is the one transition the design annotates
                        // "with a reason", and silence is the failure mode the
                        // component exists to fix.
                        modal.status = Some(format!("a close needs a reason — {seq} is unchanged"));
                        return Ok(());
                    }
                    frontdoor::NoteAction::Close => {
                        self_cli(&["frontdoor", "close", &seq, "--reason", &note])
                            .map(|_| format!("closed {seq}"))
                    }
                    frontdoor::NoteAction::NeedsInfo => {
                        let mut args = vec!["frontdoor", "needs-info", seq.as_str()];
                        if !note.is_empty() {
                            args.extend(["--note", note.as_str()]);
                        }
                        self_cli(&args).map(|_| format!("{seq} parked until they answer"))
                    }
                };
                modal.status = Some(match outcome {
                    Ok(done) => done,
                    Err(e) => format!("could not update {seq}: {e}"),
                });
                reload_frontdoor(app);
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut modal.input {
                    input.buffer.pop();
                }
            }
            _ => {
                if let (Some(c), Some(input)) = (typed_char(&key), &mut modal.input) {
                    input.buffer.push(c);
                }
            }
        }
        return Ok(());
    }

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
                app.requests = None;
            }
        }
        // The quarantined pass, detached: a model call per record. The CLI
        // decides what is extractable; the invalid guard here just answers
        // faster than a child process would.
        KeyCode::Char('x') => {
            if let Some(row) = modal.selected_row() {
                if !row.valid {
                    modal.status = Some(format!(
                        "{} is invalid — invalid records are never extracted",
                        row.seq
                    ));
                } else {
                    let (seq, state_before) = (row.seq, row.state.clone());
                    let spawned =
                        spawn_detached(&["frontdoor", "extract", "--seq", &seq.to_string()]);
                    let watch = spawned.is_ok();
                    modal.status = Some(match spawned {
                        Ok(_) => format!("extracting {seq} — the result will be reported here"),
                        Err(e) => format!("could not start the extraction: {e}"),
                    });
                    if watch {
                        app.watches.push(Watch::Request {
                            seq,
                            state_before,
                            since: std::time::Instant::now(),
                        });
                    }
                    reload_frontdoor(app);
                }
            }
        }
        // The privileged pass, detached: a whole agent run per record, ending
        // in drafts — which is where /outbox picks up.
        KeyCode::Char('t') => {
            if let Some(row) = modal.selected_row() {
                if row.state != mecha_core::frontdoor::EXTRACTED {
                    modal.status = Some(format!(
                        "{} is `{}` — triage runs on `extracted`",
                        row.seq, row.state
                    ));
                } else {
                    let (seq, state_before) = (row.seq, row.state.clone());
                    let spawned =
                        spawn_detached(&["frontdoor", "triage", "--seq", &seq.to_string()]);
                    let watch = spawned.is_ok();
                    modal.status = Some(match spawned {
                        Ok(_) => {
                            format!("triaging {seq} — its drafts will be reported when it finishes")
                        }
                        Err(e) => format!("could not start the triage: {e}"),
                    });
                    if watch {
                        app.watches.push(Watch::Request {
                            seq,
                            state_before,
                            since: std::time::Instant::now(),
                        });
                    }
                    reload_frontdoor(app);
                }
            }
        }
        KeyCode::Char('n') => {
            if let Some(row) = modal.selected_row() {
                modal.input = Some(frontdoor::NoteInput {
                    seq: row.seq,
                    action: frontdoor::NoteAction::NeedsInfo,
                    buffer: String::new(),
                });
            }
        }
        KeyCode::Char('c') => {
            if let Some(row) = modal.selected_row() {
                modal.input = Some(frontdoor::NoteInput {
                    seq: row.seq,
                    action: frontdoor::NoteAction::Close,
                    buffer: String::new(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuild the /frontdoor modal's rows, keeping the cursor where it was.
/// Keys for the /polls modal. Every mutation drives `factory-publish
/// polls …` — the polls' own CLI, one implementation per verb, and no way
/// for the TUI to do something the command line cannot. Fetches block for
/// one HTTP round-trip on purpose: the honest alternative is a watcher
/// nobody needs for a sub-second call, and the row records the moment it
/// was true.
fn handle_polls_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.poll_monitor else {
        return Ok(());
    };

    // A resolution being typed owns the keyboard. Empty is allowed: an
    // outcome is Loomio's statement for the page, not an accountability
    // requirement like the frontdoor's close reason.
    if modal.input.is_some() {
        match key.code {
            KeyCode::Esc => modal.input = None,
            KeyCode::Enter => {
                let input = modal.input.take().expect("checked above");
                let note = input.buffer.trim().to_string();
                let Some(row) = modal.selected_row() else {
                    return Ok(());
                };
                let instrument = row.instrument.clone();
                let poll_id = input.poll_id;
                let mut args = vec!["polls", "close", instrument.as_str(), poll_id.as_str()];
                if !note.is_empty() {
                    args.extend(["--resolution", note.as_str()]);
                }
                modal.status = Some(match factory_cli(&args) {
                    Ok(_) => format!("closed {poll_id}"),
                    Err(e) => format!("could not close {poll_id}: {e}"),
                });
                fetch_selected_poll(modal);
            }
            KeyCode::Backspace => {
                if let Some(input) = &mut modal.input {
                    input.buffer.pop();
                }
            }
            _ => {
                if let (Some(c), Some(input)) = (typed_char(&key), &mut modal.input) {
                    input.buffer.push(c);
                }
            }
        }
        return Ok(());
    }

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
            if !modal.detail {
                // Entering the detail is asking the gate: the tallies are
                // the point, and a stale pane would answer with silence.
                fetch_selected_poll(modal);
            }
            modal.detail = !modal.detail;
            modal.detail_scroll = 0;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            if modal.detail {
                modal.detail = false;
            } else {
                app.poll_monitor = None;
            }
        }
        KeyCode::Char('r') => fetch_selected_poll(modal),
        KeyCode::Char('c') => {
            if let Some(row) = modal.selected_row() {
                modal.input = Some(polls::ResolutionInput {
                    poll_id: row.poll_id.clone(),
                    buffer: String::new(),
                });
            }
        }
        KeyCode::Char('e') => {
            if let Some(row) = modal.selected_row() {
                let instrument = row.instrument.clone();
                let poll_id = row.poll_id.clone();
                let out = mecha_core::work::mecha_home().map(|home| {
                    home.join("factory")
                        .join("polls")
                        .join(format!("{poll_id}.csv"))
                });
                modal.status = Some(match out {
                    Ok(out) => {
                        let path = out.display().to_string();
                        match factory_cli(&[
                            "polls",
                            "export",
                            &instrument,
                            &poll_id,
                            "--out",
                            &path,
                        ]) {
                            Ok(_) => format!("exported → {path}"),
                            Err(e) => format!("export failed: {e}"),
                        }
                    }
                    Err(e) => format!("export failed: {e}"),
                });
            }
        }
        KeyCode::Char('s') => {
            if let Some(row) = modal.selected_row() {
                modal.status = Some(match &row.screen_url {
                    Some(url) => format!("projector: {url}"),
                    None => "no projector url on record — older poll, or a times poll".into(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Ask the gate about the selected poll — the CLI's own words, stamped
/// with the moment they were true.
fn fetch_selected_poll(modal: &mut polls::PollsModal) {
    let selected = modal.selected;
    let Some(row) = modal.rows.get_mut(selected) else {
        return;
    };
    let as_of = chrono::Local::now().format("%H:%M:%S").to_string();
    let instrument = row.instrument.clone();
    let poll_id = row.poll_id.clone();
    let result = factory_cli(&["polls", "status", &instrument, &poll_id]);
    row.install_fetch(as_of, result);
}

/// Run `factory-publish <args...>` and return its output. The polls'
/// verbs live in that binary (it holds the gate address and the slots
/// key); the TUI drives it exactly as it drives `mecha` itself. Found on
/// PATH, because it is another crate's binary — and its absence is named,
/// not mumbled.
fn factory_cli(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("factory-publish")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .context("running factory-publish — is it installed and on PATH?")?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("{}", err.trim().lines().next().unwrap_or("failed"))
    }
}

fn reload_frontdoor(app: &mut App) {
    let (selected, detail, status) = match &app.requests {
        Some(m) => (m.selected, m.detail, m.status.clone()),
        None => return,
    };
    match frontdoor::load() {
        Ok(rows) => {
            let selected = selected.min(rows.len().saturating_sub(1));
            app.requests = Some(frontdoor::FrontdoorModal {
                selected,
                detail: detail && !rows.is_empty(),
                status,
                ..frontdoor::FrontdoorModal::new(rows)
            });
        }
        Err(e) => {
            app.requests = None;
            app.transcript
                .push(Entry::Error(format!("frontdoor: {e:#}")));
        }
    }
}

/// Keys for the /doctor modal.
///
/// Acting on a finding routes through `doctor::dispatch`, a pure function
/// over the remedy, so the three arms live in one testable place: a remedy
/// whose surface is already a TUI modal deep-links to it (spawning a nested
/// CLI inside the TUI would be a terminal fighting itself); a `needs_terminal`
/// remedy defers to the event loop for the suspend dance an `$EDITOR` gets;
/// anything else confirms with y/N, spawns detached, and is watched — the
/// outcome reported from a fresh examination, never the exit code alone.
fn handle_doctor_key(
    app: &mut App,
    key: KeyEvent,
    live: &Live,
    session: Option<&Session>,
) -> Result<()> {
    let Some(modal) = &mut app.health else {
        return Ok(());
    };

    // A pending confirmation swallows the keyboard: y runs the remedy,
    // anything else leaves the machine as it is.
    if let Some(confirm) = modal.confirm.take() {
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            let argv_line = confirm.argv.join(" ");
            // F4: the same re-examination the Slack executor runs at tap time
            // (SLACK-ACTIONS-DESIGN §5), through the same shared guard. The
            // finding must still be true when the `y` lands — this confirm
            // may have sat under a stale modal for minutes, and restarting a
            // unit that recovered kills whatever it is mid-run. The probe is
            // spawned work, like the Slack side's `spawn_blocking` and the
            // examination itself (F7): `unit_is_failed` is a blocking
            // `systemctl` call, and run here it froze rendering exactly when
            // systemd was sick. The watch execs only if still failed and
            // reports "already recovered" as its outcome.
            if let Some(unit) = crate::commands::doctor::restart_unit_of(&confirm.argv) {
                let unit = unit.to_string();
                let (tx, rx) = std::sync::mpsc::channel();
                let probed = unit.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::commands::doctor::unit_is_failed(&probed));
                });
                modal.status = Some(format!(
                    "checking whether {unit} is still failed — the outcome will be \
                     reported here"
                ));
                app.watches.push(Watch::RestartProbe {
                    rx,
                    argv: confirm.argv,
                    unit,
                    since: std::time::Instant::now(),
                });
                return Ok(());
            }
            match spawn_remedy(&confirm.argv) {
                Ok(child) => {
                    modal.status = Some(format!(
                        "running `{argv_line}` — the outcome will be reported here"
                    ));
                    app.watches.push(Watch::Remedy {
                        child,
                        argv_line,
                        since: std::time::Instant::now(),
                        notices: 0,
                    });
                }
                Err(e) => modal.status = Some(format!("could not start `{argv_line}`: {e}")),
            }
        }
        return Ok(());
    }

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
                app.health = None;
            }
        }
        // A fresh examination on demand — the same child process the modal
        // opened with, so it can never see something `mecha doctor` cannot.
        // Detached and watched (F7); the rows land when it answers, and the
        // modal keeps taking keys meanwhile.
        KeyCode::Char('r') => {
            reload_doctor(app);
        }
        KeyCode::Char('a') => {
            let remedy = modal.selected_row().and_then(|r| r.remedy.clone());
            match remedy {
                None => {
                    modal.status =
                        Some("this finding carries no remedy — it is the diagnosis".into())
                }
                Some(remedy) => match doctor::dispatch(&remedy) {
                    doctor::RemedyDispatch::DeepLink(cmd) => {
                        // The surface the remedy names is a keystroke away —
                        // switch to it rather than spawning a nested CLI.
                        app.health = None;
                        return run_command(app, cmd, live, session);
                    }
                    doctor::RemedyDispatch::Interactive => {
                        app.pending_doctor_remedy = Some(remedy);
                    }
                    doctor::RemedyDispatch::Spawn => {
                        modal.confirm = Some(doctor::RemedyConfirm {
                            description: remedy.description,
                            argv: remedy.argv,
                        });
                    }
                },
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuild the /doctor modal's rows from a fresh examination, keeping the
/// cursor where it was. The examination is detached and watched (F7); until
/// it answers, the modal shows the previous rows and says it is examining.
fn reload_doctor(app: &mut App) {
    if app.health.is_none() {
        return;
    }
    start_examination(app);
}

/// Begin a detached `mecha doctor --json` and watch it (F7). The examination
/// reads every store and shells out to `systemctl --user`, which on a sick
/// D-Bus can take tens of seconds; run synchronously from a key handler it
/// froze rendering and steering — the two things the TUI exists to keep
/// live. One at a time: a second request while one is in flight just keeps
/// waiting for the answer already coming.
fn start_examination(app: &mut App) {
    if app
        .watches
        .iter()
        .any(|w| matches!(w, Watch::Examine { .. }))
    {
        if let Some(modal) = &mut app.health {
            modal.examining = true;
        }
        return;
    }
    match doctor::spawn_examination() {
        Ok(child) => {
            if let Some(modal) = &mut app.health {
                modal.examining = true;
            }
            app.watches.push(Watch::Examine {
                child,
                since: std::time::Instant::now(),
            });
        }
        Err(e) => {
            if let Some(modal) = &mut app.health {
                modal.examining = false;
                modal.status = Some(format!("doctor could not run: {e:#}"));
            } else {
                app.transcript.push(Entry::Error(format!("doctor: {e:#}")));
            }
        }
    }
}

/// Install freshly examined rows into the /doctor modal, if it is open,
/// keeping the cursor where it was. Split from `reload_doctor` so the remedy
/// watch — which already ran the examination for its notice — does not run
/// the child process twice.
fn install_doctor_rows(app: &mut App, rows: Vec<doctor::FindingRow>) {
    let (selected, detail, status) = match &app.health {
        Some(m) => (m.selected, m.detail, m.status.clone()),
        None => return,
    };
    let selected = selected.min(rows.len().saturating_sub(1));
    app.health = Some(doctor::DoctorModal {
        selected,
        detail: detail && !rows.is_empty(),
        status,
        ..doctor::DoctorModal::new(rows)
    });
}

/// Start a remedy's argv and hand back the child so a watch can notice it
/// exiting. Like `spawn_detached`, with two differences: the program may be
/// another binary entirely (`systemctl`, `mecha-mail`), and `mecha` itself
/// resolves to `current_exe` so a TUI run from `target/debug` drives the
/// build it is part of.
fn spawn_remedy(argv: &[String]) -> Result<std::process::Child> {
    let (program, rest) = argv.split_first().context("a remedy with an empty argv")?;
    let program: std::path::PathBuf = if program.as_str() == "mecha" {
        crate::exe::self_exe()
    } else {
        program.into()
    };
    std::process::Command::new(program)
        .args(rest)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting it")
}

/// Run a doctor remedy that needs the real terminal — an OAuth flow — then
/// refresh the modal from a fresh examination on return. The suspend is the
/// same dance as an outbox edit; what differs is that the child may be
/// another binary entirely, so the program is resolved rather than always
/// exec'ing ourselves.
fn suspend_and_run_remedy(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
    remedy: &mecha_core::doctor::Remedy,
) -> Result<()> {
    let argv_line = remedy.argv.join(" ");
    let result = with_terminal_suspended(terminal, || run_remedy_interactive(&remedy.argv))?;

    if let Some(modal) = &mut app.health {
        modal.status = Some(match &result {
            Ok(_) => format!("`{argv_line}` finished"),
            Err(e) => format!("`{argv_line}` failed: {e}"),
        });
    }
    // Loud as well as in the title, like a rejected trigger edit: a dead
    // login that stayed dead must not surface only at the next scheduled run.
    if let Err(e) = result {
        app.transcript
            .push(Entry::Error(format!("remedy `{argv_line}` failed: {e}")));
    }
    reload_doctor(app);
    Ok(())
}

/// Run a remedy inheriting the real terminal — stdin, stdout **and stderr**
/// (F2). The suspend dance already handed the whole screen over, and a
/// device-code sign-in prints its instructions to stderr (mecha-mail's
/// `eprintln!`): capturing it left a blank suspended terminal until the code
/// expired, with the one thing the person needed to read sitting in a pipe.
/// This is the `.output()`-hands-`$EDITOR`-a-pipe bug in a new costume — a
/// stream an interactive child needs must reach the real terminal. The cost
/// is that a failure's text lands on that terminal rather than in the
/// modal's status line; the exit status still names the failure there.
fn run_remedy_interactive(argv: &[String]) -> Result<()> {
    let (program, rest) = argv.split_first().context("a remedy with an empty argv")?;
    let program: std::path::PathBuf = if program.as_str() == "mecha" {
        crate::exe::self_exe()
    } else {
        program.into()
    };
    let status = std::process::Command::new(program)
        .args(rest)
        .status()
        .context("running it")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("exited with {status}")
    }
}

/// Read the board by driving `mecha tasks list --json` — the tool's own
/// answer, forwarded unchanged.
///
/// Blocks for the child, on the `/polls` reasoning: a board drawn from a stale
/// copy would answer a question about now with the state before the last
/// keypress, and the alternative is a watcher for something that finishes in
/// the time it takes to lift a finger.
///
/// **What it costs is a whole `prepare_tools`, not a SQLite open.** The child
/// connects *every* configured `[[mcp]]` server — mail with its token
/// lifecycle, the graph, whatever else — and runs the sandbox preflight, which
/// measures ~270ms here; a status change pays it twice, once to set and once
/// to re-read the reordered board. That is the real difference from `/mail`,
/// whose list comes from a local store. It is under the threshold where a
/// keypress stops feeling like one, and it is the price of the seam: the graph
/// is reached through the tool surface, and the tool surface is what
/// `prepare_tools` builds. If it ever needs to be cheaper, the fix is a way to
/// connect one named server, not a second path into the graph.
fn load_tasks(show_closed: bool) -> Result<tasks::TasksModal> {
    let mut args = vec!["tasks", "list", "--json"];
    if show_closed {
        args.push("--closed");
    }
    let (rows, today) = tasks::rows_from_json(&self_cli(&args)?)?;
    let mut modal = tasks::TasksModal::new(rows, today);
    modal.show_closed = show_closed;
    Ok(modal)
}

/// Re-read the board, keeping the cursor on the same *task*.
///
/// **By id, never by index.** The board is ordered actionable-first and then
/// by due date, so the one action that reloads it — changing a status — is
/// also the one that reorders it: a row carried across as a position would
/// put the cursor on a different task, and the next keypress might be `d`.
/// The `/outbox` hidden-items toggle learned this; here it is not an edge
/// case but the common path.
fn reload_tasks(app: &mut App, status: Option<String>) {
    let Some(old) = &app.tasks else {
        return;
    };
    let (detail, show_closed, help) = (old.detail, old.show_closed, old.help);
    let id = old.selected_row().map(|r| r.id.clone());
    let fallback = old.selected;
    match load_tasks(show_closed) {
        Ok(mut modal) => {
            let found = id.and_then(|id| modal.rows.iter().position(|r| r.id == id));
            modal.selected =
                found.unwrap_or_else(|| fallback.min(modal.rows.len().saturating_sub(1)));
            // **Keyed on whether the task was found, not on whether the board
            // is empty.** A task that left the board — marked done while
            // closed rows are hidden — must take its detail pane with it. The
            // cursor falls through to whichever row inherited the index, and a
            // detail pane left open would redraw as *that* task under a header
            // still offering `d`: a second keypress, the natural "did that
            // register?", would close a task nobody selected.
            modal.detail = detail && found.is_some();
            modal.help = help;
            modal.status = status;
            app.tasks = Some(modal);
        }
        // The board is unreadable, so there is nothing true to draw. Closing
        // with the reason beats leaving the last good board on screen under a
        // cursor whose keys now act on a store nobody can read.
        Err(e) => {
            app.tasks = None;
            app.transcript
                .push(transcript::Entry::Error(format!("tasks: {e:#}")));
        }
    }
}

fn tasks_cli(args: &[&str]) -> Result<String> {
    let mut full = vec!["tasks"];
    full.extend_from_slice(args);
    self_cli(&full)
}

// ─── /queues ─────────────────────────────────────────────────────────────────

/// Drive `mecha review …` — the `/tasks` rule: one implementation per verb,
/// and nothing the modal can do that the command line cannot.
fn review_cli(args: &[&str]) -> Result<String> {
    let mut full = vec!["review"];
    full.extend_from_slice(args);
    self_cli(&full)
}

/// Drive `mecha <args>` for the generic review level — same rule as
/// `review_cli`, but the verb varies by store.
fn self_cli_capture(args: &[&str]) -> Result<String> {
    self_cli(args)
}

/// Drive `mecha-graph <args>`. The graph's proposal queue lives in its own
/// binary, so the review level reaches it the way `/queues` already reaches
/// the graph's fact queue — as a child process, never by opening the store.
fn graph_cli_raw(args: &[&str]) -> Result<String> {
    let bin = graph_bin();
    let out = std::process::Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("running {bin}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    anyhow::bail!(
        "{bin} {}: {}",
        args.first().unwrap_or(&""),
        stderr.trim().lines().next().unwrap_or("failed")
    )
}

/// A row's title, short enough for a status line.
fn truncate_title(s: &str) -> String {
    if s.chars().count() <= 48 {
        return s.to_string();
    }
    s.chars().take(47).chain(std::iter::once('…')).collect()
}

/// A seed for a sample draw, from the clock.
///
/// Chosen on this side so the modal can name it and redraw it — the graph
/// will pick one if asked, but does not report it through `--json`.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

fn load_queues() -> Result<queues::QueuesModal> {
    let rows = queues::queues_from_json(&review_cli(&["queues", "--json"])?)?;
    Ok(queues::QueuesModal::new(rows))
}

/// Re-read whichever level is showing, keeping the cursor where it was.
///
/// The cursor is carried by index rather than by identity, which is right
/// here and would not be in `/outbox`: these lists are stable orderings of
/// classes and mechanisms, and a reload after `a` removes the row that was
/// under it — so landing on the next row is the behaviour somebody working
/// down a list wants. Nothing on the next keypress is irreversible: `a` and
/// `r` both stage a decision the graph records and neither sends anything.
fn reload_queues(app: &mut App, status: Option<String>) {
    let Some(old) = &app.queues else { return };
    let (level, filter, selected) = (old.level, old.filter.clone(), old.selected);
    let (item_class, item_seed, tier) = (old.item_class.clone(), old.item_seed, old.tier);
    let from_group = old.from_group.clone();
    let loaded = match level {
        // The review level reloads from whichever store it was opened on;
        // the source travels with the modal so the reload does not have to
        // work out which queue it came from.
        queues::Level::Review => {
            let Some(src) = &old.review_source else {
                return;
            };
            let argv: Vec<&str> = src.list.iter().map(String::as_str).collect();
            let out = if src.graph {
                graph_cli_raw(&argv)
            } else {
                self_cli_capture(&argv)
            };
            match out {
                Ok(t) => queues::review_from_json(&t).map(|rows| {
                    let mut m = queues::QueuesModal::new(vec![]);
                    m.level = queues::Level::Review;
                    m.review = rows;
                    m.review_source = old.review_source.clone();
                    m
                }),
                Err(e) => Err(e),
            }
        }
        queues::Level::Groups => {
            let Some((pb, pred)) = old.item_class.clone() else {
                return;
            };
            match review_cli(&["groups", "--proposer", &pb, "--predicate", &pred, "--json"]) {
                Ok(t) => queues::groups_from_json(&t).map(|(threshold, rows)| {
                    let mut m = queues::QueuesModal::new(vec![]);
                    m.level = queues::Level::Groups;
                    m.groups = rows;
                    m.group_threshold = threshold;
                    m.item_class = Some((pb.clone(), pred.clone()));
                    m
                }),
                Err(e) => return fail_queues(app, e),
            }
        }
        queues::Level::Items if from_group.is_some() => {
            // A group's items are a named set: re-fetch exactly those ids,
            // never a redraw — the set is what the sitting is about.
            let csv = from_group.clone().unwrap_or_default();
            match review_cli(&["items", "--ids", &csv, "--json"]) {
                Ok(t) => queues::items_from_json(&t).map(|rows| {
                    let mut m = queues::QueuesModal::new(vec![]);
                    m.level = queues::Level::Items;
                    m.items = rows;
                    m.from_group = Some(csv.clone());
                    m
                }),
                Err(e) => return fail_queues(app, e),
            }
        }
        queues::Level::Queues => {
            queues::queues_from_json(&match review_cli(&["queues", "--json"]) {
                Ok(t) => t,
                Err(e) => return fail_queues(app, e),
            })
            .map(|rows| {
                let mut m = queues::QueuesModal::new(rows);
                m.level = queues::Level::Queues;
                m
            })
        }
        queues::Level::Proposers => match review_cli(&["proposers", "--json"]) {
            Ok(t) => queues::proposers_from_json(&t).map(|rows| {
                let mut m = queues::QueuesModal::new(vec![]);
                m.level = queues::Level::Proposers;
                m.proposers = rows;
                m
            }),
            Err(e) => return fail_queues(app, e),
        },
        queues::Level::Items => {
            // A reload at the item level REDRAWS the same seed rather than
            // taking a fresh sample: after `a` on one item the other eleven
            // must still be the eleven that were drawn, or a sitting's
            // verdicts stop describing one sample. `n` is how you ask for a
            // new one, explicitly.
            let Some((pb, pred)) = old.item_class.clone() else {
                return;
            };
            let seed = old.item_seed;
            let seed_s = seed.map(|x| x.to_string());
            let mut args = vec!["sample", "--proposer", &pb, "--predicate", &pred, "--json"];
            if let Some(sd) = &seed_s {
                args.push("--seed");
                args.push(sd);
            }
            match review_cli(&args) {
                Ok(t) => queues::items_from_json(&t).map(|rows| {
                    let mut m = queues::QueuesModal::new(vec![]);
                    m.level = queues::Level::Items;
                    m.items = rows;
                    m.item_class = Some((pb.clone(), pred.clone()));
                    m.item_seed = seed;
                    m
                }),
                Err(e) => return fail_queues(app, e),
            }
        }
        queues::Level::Candidates => {
            let mut args = vec!["list", "--json"];
            if let Some(f) = &filter {
                args.push("--proposer");
                args.push(f);
            }
            match review_cli(&args) {
                Ok(t) => queues::candidates_from_json(&t).map(|rows| {
                    let mut m = queues::QueuesModal::new(vec![]);
                    m.level = queues::Level::Candidates;
                    m.candidates = rows;
                    m
                }),
                Err(e) => return fail_queues(app, e),
            }
        }
    };
    match loaded {
        Ok(mut m) => {
            m.filter = filter;
            if m.item_class.is_none() {
                m.item_class = item_class;
                m.item_seed = item_seed;
            }
            // The tier filter must be restored BEFORE the cursor: `len()`
            // counts the filtered list, so clamping against an unfiltered
            // one puts the cursor on a row the user cannot see — and the
            // next key there verdicts a whole class.
            m.tier = tier;
            m.selected = selected.min(m.len().saturating_sub(1));
            m.status = status;
            app.queues = Some(m);
        }
        Err(e) => fail_queues(app, e),
    }
}

/// A reload that cannot read closes the modal and says so in the transcript.
///
/// Leaving a stale list on screen would be worse: every key in it acts on
/// rows that may no longer be what the store holds.
fn fail_queues(app: &mut App, e: impl std::fmt::Display) {
    app.queues = None;
    app.transcript
        .push(transcript::Entry::Error(format!("queues: {e:#}")));
}

/// Keys for the /queues modal.
///
/// `a` and `r` verdict a whole CLASS, which is the unit the graph's own
/// review works in — one decision worth hundreds on instances. They drive
/// `mecha review accept|reject`, which drives `mecha-graph`; no model-facing
/// tool accepts a candidate, and that split is the point.
/// The bind prompt's keys: text, Enter, Esc. Nothing else does anything —
/// a stray arrow key must not move a selection the person cannot see moving.
fn handle_bind_prompt_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.queues else {
        return Ok(());
    };
    let Some(prompt) = &mut modal.bind_to else {
        return Ok(());
    };
    match key.code {
        KeyCode::Esc => {
            modal.bind_to = None;
            modal.status = Some("bind cancelled — nothing changed".into());
        }
        KeyCode::Backspace => {
            prompt.buffer.pop();
        }
        KeyCode::Char(c) => prompt.buffer.push(c),
        KeyCode::Enter => {
            let (id, to) = (prompt.id, prompt.buffer.trim().to_string());
            // An empty target is not "take the suggestion" — that is `b`,
            // and the reason this prompt exists is that `b` had none. Send
            // it and the graph refuses on a name that resolves to nothing,
            // which is a worse account of what happened than saying so.
            if to.is_empty() {
                modal.status = Some("name an entity, or Esc — b is the suggestion".into());
                return Ok(());
            }
            modal.bind_to = None;
            bind_candidate(app, id, Some(&to));
        }
        _ => {}
    }
    Ok(())
}

/// Bind one candidate's subject, with or without an explicit target, and
/// report the child's own line.
///
/// One function for the three call sites (`b` at two levels, the prompt's
/// Enter) because they differ only in `--to`. A failed `b` **opens the
/// prompt**: the graph's refusal there names naming a target as the remedy,
/// and a surface that prints a remedy it cannot perform is where this whole
/// gap was found in the first place — on a phone, holding an error with no
/// answer to it.
fn bind_candidate(app: &mut App, id: i64, to: Option<&str>) {
    let id_s = id.to_string();
    let mut argv: Vec<&str> = vec!["bind", &id_s];
    if let Some(t) = to {
        argv.extend(["--to", t]);
    }
    let result = review_cli(&argv);
    let Some(m) = &mut app.queues else {
        return;
    };
    match result {
        // Pass the child's report through rather than re-wording it:
        // `#id subject 'old' → New — accept to promote` is the whole
        // answer, next keypress included.
        Ok(report) => m.status = Some(report.trim().to_string()),
        Err(e) => {
            m.status = Some(format!("bind #{id_s} failed: {e:#}"));
            if to.is_none() {
                m.bind_to = Some(queues::BindPrompt {
                    id,
                    buffer: String::new(),
                });
            }
        }
    }
}

/// What a group verdict says it did, from the child's own tally.
///
/// Pure so the honesty is a unit test rather than a hope — the same reason
/// `why_nothing_landed` is. This one line is the ONLY thing telling a person
/// what a keystroke just covered, so what it cannot know it has to say.
///
/// `cascade` is `None` in two situations that mean opposite things, and
/// flattening them with `unwrap_or((0, 0))` is what this replaces:
///
/// - **No fan-out was asked for** — a singleton group takes `Fan::None`, no
///   `--cascade` reaches the child, and there is no `cascade:` line to read.
///   `×1` is then the whole truth.
/// - **A fan-out was asked for and the child did not report it** — an older
///   graph binary, a changed line, a cascade arm that died before printing.
///   Here `×1` is a claim nobody checked, and the missing "left pending" note
///   is worse than the wrong number: silence reads as "none left", so the
///   line could not distinguish *nothing was left pending* from *I do not
///   know whether anything was*.
///
/// `members` is what separates them, which is why the caller keeps it.
///
/// **Every count comes before the statement head**, which is the other half of
/// making this readable. `QueuesModal::draw` renders the status into a
/// `Rect { height: 1 }` with no `.wrap()`, so it CLIPS rather than wrapping —
/// and the box is `122.min(frame.width)` with `Borders::ALL` under a two-space
/// indent, leaving 76 columns on an eighty-column terminal. A caveat written
/// after the forty-eight-character head fell off the end there, which is this
/// function's own failure one layer down: the note absent, and its absence
/// read as "nothing to report". The head is the part that can be lost — the
/// group has already gone from the list above — so it goes last.
fn group_verdict_status(
    verb: &str,
    head: &str,
    members: usize,
    cascade: Option<(usize, usize)>,
) -> String {
    match cascade {
        Some((cascaded, 0)) => format!("{verb}ed ×{} — {head}", 1 + cascaded),
        Some((cascaded, left)) => format!(
            "{verb}ed ×{}, {left} similar left pending — {head}",
            1 + cascaded
        ),
        // Nothing was asked to fan out, so nothing is unaccounted for.
        None if members == 0 => format!("{verb}ed ×1 — {head}"),
        // A count this line does not have is not a count of one.
        None => format!(
            "{verb}ed the seed only — fan-out unreported, {members} similar still \
             pending — {head}"
        ),
    }
}

fn handle_queues_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.queues else {
        return Ok(());
    };
    if modal.help {
        // Any key leaves help — the same as its siblings.
        modal.help = false;
        return Ok(());
    }
    // The bind prompt owns the keyboard while it is up, and it has to: below
    // this point `a`, `r` and `d` are verdicts, so a target named "Dana"
    // typed into a live list would file three of them. Handled before the
    // level match rather than as arms inside it, so no key can be reached
    // by accident from here — the same reason a modal takes the screen.
    if modal.bind_to.is_some() {
        return handle_bind_prompt_key(app, key);
    }
    match key.code {
        KeyCode::Char('?') => modal.help = true,
        // Evidence filter. Display-only — the rows are already loaded, so a
        // toggle costs no subprocess.
        KeyCode::Char('t') if modal.tier_applies() => {
            modal.cycle_tier();
            let n = modal.len();
            modal.status = Some(match modal.tier {
                Some(t) => format!("{n} at `{}`", t.as_str()),
                None => format!("{n}, all tiers"),
            });
        }
        // At the review level with a detail open, j/k scroll the fetched
        // text rather than moving the cursor underneath it — the
        // `LearningModal` pattern one modal over: a cached detail is a
        // document about one record, and moving without dropping it leaves
        // stale text on screen while `selected` (and the next `a`/`r`)
        // points somewhere else. `move_sel` itself drops `review_detail`
        // for every other case that does move.
        KeyCode::Char('j') | KeyCode::Down => {
            if modal.level == queues::Level::Review && modal.review_detail.is_some() {
                modal.detail_scroll = modal.detail_scroll.saturating_add(1);
            } else {
                modal.move_sel(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if modal.level == queues::Level::Review && modal.review_detail.is_some() {
                modal.detail_scroll = modal.detail_scroll.saturating_sub(1);
            } else {
                modal.move_sel(-1);
            }
        }
        KeyCode::PageDown if modal.level == queues::Level::Items && modal.item_detail => {
            modal.detail_scroll = modal.detail_scroll.saturating_add(5);
        }
        KeyCode::PageUp if modal.level == queues::Level::Items && modal.item_detail => {
            modal.detail_scroll = modal.detail_scroll.saturating_sub(5);
        }
        // Jumps bypass `move_sel`'s clamped delta, so they clear the same
        // pair by hand — a jump to the top or bottom is still a move, and
        // the cached review detail belongs to whatever `selected` was.
        KeyCode::Char('g') | KeyCode::Home => {
            modal.selected = 0;
            modal.review_detail = None;
            modal.detail_scroll = 0;
        }
        KeyCode::Char('G') | KeyCode::End => {
            modal.selected = modal.len().saturating_sub(1);
            modal.review_detail = None;
            modal.detail_scroll = 0;
        }
        KeyCode::Esc | KeyCode::Char('q') => match modal.level {
            // Esc peels one layer at a time here too: the detail closes
            // before the level does.
            queues::Level::Review if modal.review_detail.is_some() => {
                modal.review_detail = None;
                modal.detail_scroll = 0;
            }
            queues::Level::Review => {
                modal.level = queues::Level::Queues;
                modal.selected = 0;
                modal.review.clear();
                modal.review_source = None;
                modal.review_detail = None;
                reload_queues(app, None);
            }
            // Esc peels one level at a time, never two — and the item
            // detail is a layer of its own.
            queues::Level::Items if modal.item_detail => {
                modal.item_detail = false;
                modal.detail_scroll = 0;
            }
            queues::Level::Items if modal.from_group.is_some() => {
                // Back to the groups, updated LOCALLY from what remains: the
                // items list already dropped every verdicted row, so the
                // group is rebuilt from the survivors rather than re-embedded
                // — no child call, and the listing cannot disagree with what
                // was just done in front of the user.
                let remaining: Vec<i64> = modal.items.iter().map(|r| r.id).collect();
                let csv = modal.from_group.take().unwrap_or_default();
                let ids: Vec<i64> = csv
                    .split(',')
                    .filter_map(|t| t.trim().parse().ok())
                    .collect();
                if let Some(pos) = modal
                    .groups
                    .iter()
                    .position(|g| ids.first() == Some(&g.leader_id))
                {
                    let survivors: Vec<i64> = ids
                        .iter()
                        .copied()
                        .filter(|i| remaining.contains(i))
                        .collect();
                    // Fewer than two survivors is not a group: nothing left,
                    // or a leader with nobody behind it, and a row offering
                    // `a` over `×1` covers exactly one candidate the item
                    // list is already showing. `split_first` kept the second
                    // case, which is the commonest group size there is.
                    match survivors.split_first().filter(|(_, rest)| !rest.is_empty()) {
                        Some((lead, rest)) => {
                            let g = &mut modal.groups[pos];
                            if *lead != g.leader_id {
                                g.statement = modal
                                    .items
                                    .iter()
                                    .find(|r| r.id == *lead)
                                    .map(|r| r.statement.clone())
                                    .unwrap_or_else(|| g.statement.clone());
                            }
                            g.leader_id = *lead;
                            g.member_ids = rest.to_vec();
                            // Rebuilt from the survivors rather than blanked.
                            // The sample is what the card shows UNDER the
                            // leader, and the rows are already in hand — the
                            // same place the promoted leader's statement
                            // above came from. Clearing it left a group whose
                            // face was one line and whose count said six,
                            // which reads as five members the view could not
                            // name. Same three as the child would send, in
                            // the order it sent them.
                            g.sample = rest
                                .iter()
                                .take(3)
                                .map(|id| {
                                    modal
                                        .items
                                        .iter()
                                        .find(|r| r.id == *id)
                                        .map(|r| r.statement.clone())
                                        .unwrap_or_else(|| format!("#{id}"))
                                })
                                .collect();
                            // The spans line goes when the group shrinks, the
                            // same move the web pane makes on the same
                            // finding. `classes` counts members per class and
                            // a removal cannot be attributed to one — the key
                            // is the graph's `cluster_key`, which this file
                            // must not keep a second copy of — but the row
                            // renders `×{size}` with `spans: c ×n, …` right
                            // underneath. Reject four of seven and it reads
                            // `×3` above spans summing to seven: two numbers
                            // disagreeing on one row. A named absence beats a
                            // count that has quietly stopped being true.
                            if survivors.len() != ids.len() {
                                g.classes.clear();
                            }
                        }
                        None => {
                            modal.groups.remove(pos);
                        }
                    }
                }
                modal.level = queues::Level::Groups;
                modal.items.clear();
                modal.item_detail = false;
                modal.detail_scroll = 0;
                modal.selected = modal.selected.min(modal.groups.len().saturating_sub(1));
                modal.status = None;
            }
            queues::Level::Items => {
                modal.level = queues::Level::Candidates;
                modal.item_class = None;
                modal.item_seed = None;
                modal.selected = 0;
                modal.status = None;
                reload_queues(app, None);
            }
            queues::Level::Groups => {
                // Back to the level the grouping was asked from: the class
                // list for a class grouping, the proposer list for the
                // global layer (whose item_class is None by construction).
                modal.level = match modal.item_class {
                    Some(_) => queues::Level::Candidates,
                    None => queues::Level::Proposers,
                };
                modal.groups.clear();
                modal.item_class = None;
                modal.selected = 0;
                modal.status = None;
                reload_queues(app, None);
            }
            queues::Level::Candidates => {
                modal.level = queues::Level::Proposers;
                modal.filter = None;
                modal.selected = 0;
                modal.status = None;
                reload_queues(app, None);
            }
            queues::Level::Proposers => {
                modal.level = queues::Level::Queues;
                modal.selected = 0;
                modal.status = None;
                reload_queues(app, None);
            }
            queues::Level::Queues => app.queues = None,
        },
        KeyCode::Enter => match modal.level {
            // Nothing. a/r are the verbs here, and a key that looks like
            // "open" but silently decides would be the worst possible
            // default on a surface whose whole job is deciding.
            queues::Level::Review => {
                // Read it whole before deciding. The list line is a summary
                // — "5 rule(s) from 10 reflection(s)" is a count, not
                // something anyone can accept on — so the item has to be
                // openable or `a` is a blind keystroke.
                //
                // This arm was a no-op with a comment arguing it should be,
                // and the argument was right until the detail view existed:
                // a key that reads as "open" but silently DECIDES is a bad
                // default on a deciding surface, so Enter did nothing while
                // a/r were the only verbs. Adding `show` inverted it and the
                // comment survived as a fossil. The compiler said so — `field
                // `show` is never read` — for every build in between.
                let Some(src) = modal.review_source.clone() else {
                    return Ok(());
                };
                let Some(row) = modal.review.get(modal.selected) else {
                    return Ok(());
                };
                let mut argv: Vec<String> = src.show.clone();
                argv.push(row.id.clone());
                let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                let out = if src.graph {
                    graph_cli_raw(&refs)
                } else {
                    self_cli_capture(&refs)
                };
                match out {
                    Ok(t) => modal.review_detail = Some(t),
                    Err(e) => modal.status = Some(format!("show failed: {e:#}")),
                }
            }
            queues::Level::Queues => {
                let Some(q) = modal.selected_queue() else {
                    return Ok(());
                };
                if q.is_graph() {
                    modal.level = queues::Level::Proposers;
                    modal.selected = 0;
                    reload_queues(app, None);
                } else if let Some(src) = q.review_source() {
                    // Reviewed in place. These three stores answer the same
                    // shape and take the same verbs, so they share one level
                    // rather than owning a modal each — and the row stops
                    // announcing a count it cannot open.
                    modal.level = queues::Level::Review;
                    modal.selected = 0;
                    modal.review_source = Some(src);
                    reload_queues(app, None);
                } else {
                    // Hand off to the modal that owns this queue — it holds
                    // the confirmations and taint warnings that make its
                    // approvals safe, and a second copy of those here would
                    // be a second thing to keep correct.
                    let opens = q.opens.clone();
                    app.queues = None;
                    match opens.as_str() {
                        "mecha outbox" => match outbox::load() {
                            Ok(rows) => app.staged = Some(outbox::OutboxModal::new(rows)),
                            Err(e) => app
                                .transcript
                                .push(transcript::Entry::Error(format!("outbox: {e:#}"))),
                        },
                        "mecha frontdoor list" => match frontdoor::load() {
                            Ok(rows) => app.requests = Some(frontdoor::FrontdoorModal::new(rows)),
                            Err(e) => app
                                .transcript
                                .push(transcript::Entry::Error(format!("frontdoor: {e:#}"))),
                        },
                        other => app.transcript.push(transcript::Entry::Notice(format!(
                            "no modal for that one yet — run `{other}`"
                        ))),
                    }
                }
            }
            queues::Level::Proposers => {
                let Some(p) = modal.selected_proposer() else {
                    return Ok(());
                };
                let name = p.proposer.clone();
                modal.level = queues::Level::Candidates;
                modal.filter = Some(name.clone());
                modal.selected = 0;
                reload_queues(app, Some(format!("classes proposed by {name}")));
            }
            queues::Level::Candidates => {
                // Drilling into a class draws a RANDOM sample, not its head.
                // The queue has an order and every order is correlated with
                // something, so verdicts collected off the top describe the
                // ordering rather than the class — which is the whole reason
                // 40.5% of this queue has no rate anybody should trust.
                let Some(c) = modal.selected_candidate() else {
                    return Ok(());
                };
                // The cluster key VERBATIM, parentheses and all: `sample`
                // filters on `precheck::cluster_key`, which returns
                // `(commitment)` *with* them. Trimming produced a filter
                // matching nothing, and an empty list looks exactly like an
                // empty class.
                let (pb, pred) = (c.proposer.clone(), c.predicate.clone());
                // The seed is chosen HERE and passed down, never left to
                // the child: `--json` does not report the seed it drew, so a
                // sample we did not seed is one we cannot redraw — and a
                // reload after each verdict would then quietly resample,
                // leaving a sitting's verdicts spread across a dozen
                // different samples instead of describing one.
                let seed = fresh_seed();
                let seed_s = seed.to_string();
                match review_cli(&[
                    "sample",
                    "--proposer",
                    &pb,
                    "--predicate",
                    &pred,
                    "--seed",
                    &seed_s,
                    "--json",
                ]) {
                    Ok(t) => match queues::items_from_json(&t) {
                        Ok(rows) => {
                            modal.level = queues::Level::Items;
                            modal.items = rows;
                            modal.item_class = Some((pb.clone(), pred.clone()));
                            modal.item_seed = Some(seed);
                            modal.selected = 0;
                            modal.status = Some(format!(
                                "random sample of {} from {pb} · {pred}",
                                modal.items.len()
                            ));
                        }
                        Err(e) => modal.status = Some(format!("sample: {e:#}")),
                    },
                    Err(e) => modal.status = Some(format!("sample failed: {e:#}")),
                }
            }
            queues::Level::Groups => {
                // Dive: the group's members in full, leader first. A named
                // set, not a sample — its verdicts are about exactly these.
                let Some(g) = modal.selected_group() else {
                    return Ok(());
                };
                let csv = g.all_ids_csv();
                match review_cli(&["items", "--ids", &csv, "--json"]) {
                    Ok(t) => match queues::items_from_json(&t) {
                        Ok(rows) => {
                            modal.level = queues::Level::Items;
                            modal.items = rows;
                            modal.from_group = Some(csv);
                            modal.item_seed = None;
                            modal.selected = 0;
                            modal.status =
                                Some("this group, one at a time — Esc returns to groups".into());
                        }
                        Err(e) => modal.status = Some(format!("items: {e:#}")),
                    },
                    Err(e) => modal.status = Some(format!("items failed: {e:#}")),
                }
            }
            queues::Level::Items => {
                if modal.selected_item().is_some() {
                    modal.item_detail = !modal.item_detail;
                    modal.detail_scroll = 0;
                }
            }
        },
        // Group level: one keystroke, one HUMAN verdict — the seed — with
        // the rest of the group following as a labeled machine cascade the
        // ladder never counts. The status line reports the child's own
        // numbers, never the row's size.
        // `b` binds the SEED's subject — the group shares its subject far
        // more often than not (that is what made it a group), so fixing the
        // one spelling unblocks the whole cascade: the alias is learned, and
        // every member resolves through it on the next `a`.
        // `B` skips straight to naming the target, for the case where a
        // suggestion was never possible.
        KeyCode::Char('b') | KeyCode::Char('B') if modal.level == queues::Level::Groups => {
            let Some(g) = modal.selected_group() else {
                return Ok(());
            };
            let id = g.leader_id;
            if key.code == KeyCode::Char('B') {
                modal.bind_to = Some(queues::BindPrompt {
                    id,
                    buffer: String::new(),
                });
                return Ok(());
            }
            bind_candidate(app, id, None);
            if let Some(m) = &mut app.queues {
                if m.bind_to.is_none() {
                    if let Some(st) = m.status.take() {
                        m.status = Some(if st.contains("failed") {
                            st
                        } else {
                            format!("{st} — a cascades the group")
                        });
                    }
                }
            }
        }
        KeyCode::Char('a') | KeyCode::Char('r') if modal.level == queues::Level::Review => {
            let Some(src) = modal.review_source.clone() else {
                return Ok(());
            };
            let Some(row) = modal.review.get(modal.selected) else {
                return Ok(());
            };
            let (id, title) = (row.id.clone(), row.title.clone());
            let accepting = key.code == KeyCode::Char('a');
            let base = if accepting { &src.accept } else { &src.reject };
            let mut argv: Vec<String> = base.clone();
            argv.push(id.clone());
            // `mecha proposals reject` and `mecha harness reject` both want a
            // reason, and a reason nobody typed is worse than none — say
            // where it came from rather than inventing a justification.
            if !accepting && !src.graph {
                argv.push("--reason".into());
                argv.push("rejected from /queues".into());
            }
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            let out = if src.graph {
                graph_cli_raw(&refs)
            } else {
                self_cli_capture(&refs)
            };
            let verb = if accepting { "accepted" } else { "rejected" };
            match out {
                Ok(t) => {
                    // The child's own first line, not a sentence composed
                    // here: an accept can APPLY something (a merge, a rename,
                    // an override) and what it did is the child's to report.
                    let said = t.trim().lines().next().unwrap_or("").to_string();
                    let note = if said.is_empty() {
                        format!("{verb} {}", truncate_title(&title))
                    } else {
                        said
                    };
                    reload_queues(app, Some(note));
                    // The item just decided is gone from the list, so a
                    // detail still showing it describes something that is no
                    // longer there.
                    if let Some(m) = &mut app.queues {
                        m.review_detail = None;
                    }
                }
                Err(e) => {
                    if let Some(m) = &mut app.queues {
                        m.status = Some(format!("{verb} #{id} failed: {e:#}"));
                    }
                }
            }
        }
        KeyCode::Char('a') | KeyCode::Char('r') | KeyCode::Char('A')
            if modal.level == queues::Level::Groups =>
        {
            let Some(g) = modal.selected_group() else {
                return Ok(());
            };
            let (leader, stmt) = (g.leader_id, g.statement.clone());
            // How many the fan-out was asked to cover, kept because the status
            // line has to be able to say what it does NOT know about them.
            let members = g.member_ids.len();
            // The cascade lands on the EXPLICIT member list this listing
            // showed — never a re-derivation: the ids on screen are what the
            // verdict is about, the graph vets them against the seed's
            // class, and no embedder runs, so the keystroke answers at
            // store speed. A singleton is the plain single-id path.
            let member_csv = g
                .member_ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            // `A` is the item level's escape hatch at group scale: the seed's
            // unknown subject becomes a new topic node, and the cascade then
            // resolves its members against the node the seed just created.
            // The human pressing it is the review that creation requires.
            let create = key.code == KeyCode::Char('A');
            let accept = create || key.code == KeyCode::Char('a');
            let verb = if accept { "accept" } else { "reject" };
            // A global group's members sit in other classes by design, so
            // its cascade rides the across vet; a class group keeps the
            // strict one. Which listing this is is exactly item_class.
            let fan = if member_csv.is_empty() {
                crate::commands::review::Fan::None
            } else if modal.item_class.is_none() {
                crate::commands::review::Fan::IdsAcross(&member_csv)
            } else {
                crate::commands::review::Fan::Ids(&member_csv)
            };
            let outcome = crate::commands::review::decide_report(
                verb,
                &[leader],
                None,
                None,
                None,
                None,
                create,
                false,
                fan,
            );
            match outcome {
                // **The exit code is not the verdict**, and this arm is the
                // third site in the repo to need saying so. mecha-graph
                // reports a per-candidate failure as `#id FAILED: …` and
                // exits 0, so a process-level `Ok` can carry a verdict that
                // did not happen. The item level below learned it the
                // expensive way (`accepted #2951` on an item whose subject
                // could not resolve: row gone locally, still pending in the
                // store) and the web route answers 409 on the same reading.
                // This arm did neither, and removed the group regardless —
                // the same lie one level up, over seven candidates instead of
                // one. Worse since the status line started naming what it
                // could not see: with an unresolvable seed there is no
                // `cascade:` line either, so it read as "the seed landed, the
                // fan-out is unknown" when nothing had landed at all.
                Ok(report) => {
                    let (landed, _failed) = crate::commands::review::tally_report(&report);
                    if let Some(m) = &mut app.queues {
                        if landed == 0 {
                            // The group STAYS: nothing changed in the store,
                            // and the ways through are the same two keys the
                            // Err arm names, on the item level's precedent.
                            // Keys FIRST, the child's sentence last — the
                            // ordering `group_verdict_status` uses one arm
                            // over, for the same reason. `why` is the graph's
                            // own `FAILED` line and its length is the
                            // subject's, so anything after it is at the mercy
                            // of a name: at 76 columns a hint written as a
                            // suffix lost `A accept new`, which is the one key
                            // that answers an unresolvable subject. The keys
                            // are bounded and the reason is not, so the
                            // bounded half goes where it cannot be clipped.
                            //
                            // Spelled as the key strip spells them, which is
                            // already the compact form — this is not new
                            // shorthand, it is the vocabulary the same screen
                            // uses two lines down.
                            let why = crate::commands::review::why_nothing_landed(&report);
                            m.status = Some(if create {
                                why
                            } else {
                                format!("b bind · A accept new — {why}")
                            });
                        } else {
                            let cascade = crate::commands::review::cascade_tally(&report);
                            m.groups.retain(|x| x.leader_id != leader);
                            m.selected = m.selected.min(m.groups.len().saturating_sub(1));
                            m.status = Some(group_verdict_status(
                                verb,
                                &stmt.chars().take(48).collect::<String>(),
                                members,
                                cascade,
                            ));
                        }
                    }
                }
                Err(e) => {
                    // The seed's verdict could not land (an unresolvable
                    // subject, most often), so nothing cascaded and the
                    // group stays. The hint names both ways through, on the
                    // item level's precedent — a failure that only says
                    // "failed" strands the person exactly where a key would
                    // have carried them. Circular after `A`, so that one
                    // gets the reason alone.
                    // Same reordering as the landed-on-nothing arm above, and
                    // for the same reason: `{e:#}` is an error chain of
                    // unbounded length, so a hint written after it is a hint
                    // that clips. Pre-existing, fixed in the same pass because
                    // both lines are read on the same screen and a reader
                    // cannot be expected to know which failure they hit.
                    if let Some(m) = &mut app.queues {
                        m.status = Some(if create {
                            format!("{verb} failed, nothing cascaded: {e:#}")
                        } else {
                            format!(
                                "b bind · A accept new — {verb} failed, nothing cascaded: {e:#}"
                            )
                        });
                    }
                }
            }
        }
        // Class level: `s` groups the class by semantic similarity — the
        // filter for a queue whose bulk is the same thing said many ways.
        KeyCode::Char('s') if modal.level == queues::Level::Candidates => {
            let Some(c) = modal.selected_candidate() else {
                return Ok(());
            };
            let (pb, pred) = (c.proposer.clone(), c.predicate.clone());
            let pending = c.pending;
            if pred.starts_with('(') {
                modal.status =
                    Some("commitments do not group — they are reviewed one at a time".into());
                return Ok(());
            }
            // Off the event loop: grouping embeds every pending candidate of
            // the class, which is seconds on a thousand-item class. The
            // status says what is running; `Watch::QueuesGroups` installs
            // the answer if the modal is still at this list.
            modal.status = Some(format!(
                "grouping {pending} pending in {pb} · {pred} — embedding…"
            ));
            spawn_group_load(app, Some((pb, pred)), None);
        }
        // Proposer level: `s` is the TOP layer — near-repeats across the
        // whole queue, every class at once, at the stricter global floor.
        // The invited crossing: a verdict here rides --across-classes, and
        // every group's spans line names what it reaches.
        KeyCode::Char('s') if modal.level == queues::Level::Proposers => {
            let pending: usize = modal.proposers.iter().map(|p| p.pending).sum();
            modal.status = Some(format!(
                "grouping {pending} pending across every class — embedding the whole queue, \
                 this runs a minute or two…"
            ));
            spawn_group_load(app, None, None);
        }
        // Adjust the grouping threshold and re-group. Steps from the value
        // the child reported it ran at — never from a local copy of the
        // constant — and re-embeds, so it goes through the same watch.
        KeyCode::Char('[') | KeyCode::Char(']') if modal.level == queues::Level::Groups => {
            let class = modal.item_class.clone();
            let step = if key.code == KeyCode::Char(']') {
                0.02
            } else {
                -0.02
            };
            // Coarser than 0.60 groups the unrelated; finer than 0.97 is
            // past the dedup line, where precheck already removed them.
            let next = (modal.group_threshold + step).clamp(0.60, 0.97);
            modal.status = Some(format!("re-grouping at cosine ≥ {next:.2}…"));
            spawn_group_load(app, class, Some(next));
        }
        // Item level: one verdict, one candidate. Distinct from the class
        // verdict a level up, and the key strip says which you are holding.
        KeyCode::Char('a') | KeyCode::Char('r') | KeyCode::Char('A')
            if modal.level == queues::Level::Items =>
        {
            let Some(it) = modal.selected_item() else {
                return Ok(());
            };
            let (id, stmt) = (it.id, it.statement.clone());
            // `A` is accept with the escape hatch: a subject the graph does
            // not know becomes a new topic node instead of a failure. The
            // human pressing it is the review that creation requires.
            let create = key.code == KeyCode::Char('A');
            let accept = create || key.code == KeyCode::Char('a');
            let verb = if accept { "accept" } else { "reject" };
            let id_s = id.to_string();
            let mut cli_args = vec![verb, id_s.as_str()];
            if create {
                cli_args.push("--create-subjects");
            }
            match review_cli(&cli_args) {
                // The exit code is not the verdict: mecha-graph reports a
                // per-item failure as `#id FAILED: …` and exits 0, so a
                // process-level Ok can carry an accept that did not happen.
                // Trusting it showed "accepted #2951" on an item whose
                // subject could not resolve — the row vanished locally, the
                // store still held it pending, and the sample refilled with
                // items the reviewer believed decided. Read the report.
                Ok(report) => {
                    let (done, _failed) = crate::commands::review::tally_report(&report);
                    if let Some(m) = &mut app.queues {
                        if done == 0 {
                            // The item stays: nothing changed in the store,
                            // and a row that disappears on a failed verdict
                            // is the lie this arm used to tell. The child's
                            // own line says why; this surface cannot fix a
                            // subject, so it says where that is done.
                            let why = report
                                .lines()
                                .find(|l| l.contains("FAILED"))
                                .unwrap_or("failed with no report")
                                .trim()
                                .to_string();
                            // The hint would be circular on the keys it
                            // names: a failed `A` gets the child's reason
                            // and nothing more.
                            m.status = Some(if create {
                                why
                            } else {
                                format!(
                                    "{why} — b binds the subject here; A accepts it as a new topic"
                                )
                            });
                            return Ok(());
                        }
                        // Drop it locally rather than refetching: the sample
                        // is fixed for this sitting, and re-running the draw
                        // would hand back a set that no longer matches what
                        // is on screen under the cursor.
                        m.items.retain(|r| r.id != id);
                        m.selected = m.selected.min(m.items.len().saturating_sub(1));
                        // A verdict closes the detail and lands back on the
                        // list — the list is where the change is visible:
                        // one row gone, the verdict in the status line.
                        // Flipping through items without deciding is what
                        // j/k in the detail is for.
                        m.item_detail = false;
                        m.detail_scroll = 0;
                        let did = if create { "accepted (new topic)" } else { verb };
                        m.status = Some(format!(
                            "{did}{} #{id} — {}",
                            if create { "" } else { "ed" },
                            stmt.chars().take(48).collect::<String>()
                        ));
                    }
                }
                Err(e) => {
                    if let Some(m) = &mut app.queues {
                        m.status = Some(format!("{verb} #{id} failed: {e:#}"));
                    }
                }
            }
        }
        // Rebind the subject to the graph's top suggestion — the other way
        // through `cannot resolve subject`, for a spelling the graph almost
        // knows. The row STAYS: a bound candidate is still pending, and the
        // child's own line says what moved and that `a` is the promotion.
        // The old spelling becomes an alias on the graph side, so the fix
        // outlives this one item.
        KeyCode::Char('b') | KeyCode::Char('B') if modal.level == queues::Level::Items => {
            let Some(it) = modal.selected_item() else {
                return Ok(());
            };
            let id = it.id;
            if key.code == KeyCode::Char('B') {
                modal.bind_to = Some(queues::BindPrompt {
                    id,
                    buffer: String::new(),
                });
                return Ok(());
            }
            bind_candidate(app, id, None);
        }
        // A fresh draw, explicitly asked for. Never automatic — see the
        // reload comment.
        KeyCode::Char('n') if modal.level == queues::Level::Items => {
            // A group's items are a named set, not a draw — there is
            // nothing to resample.
            if modal.from_group.is_some() {
                return Ok(());
            }
            let Some((pb, pred)) = modal.item_class.clone() else {
                return Ok(());
            };
            let seed = fresh_seed();
            let seed_s = seed.to_string();
            match review_cli(&[
                "sample",
                "--proposer",
                &pb,
                "--predicate",
                &pred,
                "--seed",
                &seed_s,
                "--json",
            ]) {
                Ok(t) => match queues::items_from_json(&t) {
                    Ok(rows) => {
                        modal.items = rows;
                        modal.item_seed = Some(seed);
                        modal.selected = 0;
                        modal.detail_scroll = 0;
                        modal.status = Some(format!("new sample of {}", modal.items.len()));
                    }
                    Err(e) => modal.status = Some(format!("sample: {e:#}")),
                },
                Err(e) => modal.status = Some(format!("sample failed: {e:#}")),
            }
        }
        KeyCode::Char('a') | KeyCode::Char('r') if modal.level == queues::Level::Candidates => {
            let Some(c) = modal.selected_candidate() else {
                return Ok(());
            };
            let (proposer, predicate, n) = (c.proposer.clone(), c.predicate.clone(), c.pending);
            let accept = key.code == KeyCode::Char('a');
            let _ = n;
            let verb = if accept { "accept" } else { "reject" };
            // Verbatim again, and the CLI refuses a cluster kind by name
            // rather than passing a filter that would match nothing. The
            // graph's bulk `--predicate` reads `payload["predicate"]`, which
            // a commitment does not have.
            let outcome = crate::commands::review::decide_report(
                verb,
                &[],
                None,
                Some(&proposer),
                Some(&predicate),
                None,
                false,
                false,
                crate::commands::review::Fan::None,
            );
            match outcome {
                // The CHILD's count, never the row's: the graph's bulk
                // proposer filter is a substring and `--limit` caps the set,
                // so how many were acted on is not this row's `pending`.
                Ok(report) => {
                    let (done, failed) = crate::commands::review::tally_report(&report);
                    let mut said = format!("{verb}ed {done} × {proposer} · {predicate}");
                    if failed > 0 {
                        said.push_str(&format!(" ({failed} failed)"));
                    }
                    reload_queues(app, Some(said))
                }
                Err(e) => {
                    if let Some(m) = &mut app.queues {
                        m.status = Some(format!("{verb} failed: {e:#}"));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Keys for the /tasks modal. Every mutation drives `mecha tasks …` — the
/// board's own CLI, one implementation per verb, and no way for the TUI to do
/// something the command line cannot.
fn handle_tasks_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // A form being filled in owns the keyboard: `a`, `d` and `x` are letters
    // somebody is typing into a task name, not verbs.
    if app.tasks.as_ref().is_some_and(|m| m.form.is_some()) {
        return handle_tasks_form_key(app, key);
    }
    let Some(modal) = &mut app.tasks else {
        return Ok(());
    };

    // The `?` overlay exists to be glanced at and dismissed, like the main
    // help. Any key closes it and does nothing else — a keypress that both
    // dismissed the reference card and acted on it would act on whatever the
    // cursor happened to be on while the card hid it.
    if modal.help {
        modal.help = false;
        return Ok(());
    }

    // An open source read owns the keyboard, exactly as a form does: `d` in a
    // thread is a letter somebody is scrolling past, not "done". Esc closes
    // the read and returns to the detail it was opened from, one layer at a
    // time — a key that unwound two would put a status keystroke on a board
    // whose cursor the reader had been hiding.
    if modal.reading.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => modal.reading = None,
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(r) = &mut modal.reading {
                    r.scroll_by(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(r) = &mut modal.reading {
                    r.scroll_by(1);
                }
            }
            KeyCode::PageUp => {
                if let Some(r) = &mut modal.reading {
                    r.scroll_by(-10);
                }
            }
            KeyCode::PageDown => {
                if let Some(r) = &mut modal.reading {
                    r.scroll_by(10);
                }
            }
            _ => {}
        }
        return Ok(());
    }

    modal.status = None;

    match key.code {
        // In the detail the arrows scroll it, as they do in /tools and
        // /skills: the name is whatever the user typed, so the body wraps
        // past the box on any task described in a sentence.
        KeyCode::Up if modal.detail => modal.scroll_detail(-1),
        KeyCode::Down if modal.detail => modal.scroll_detail(1),
        KeyCode::PageUp if modal.detail => modal.scroll_detail(-10),
        KeyCode::PageDown if modal.detail => modal.scroll_detail(10),
        KeyCode::Up | KeyCode::Char('k') => modal.move_by(-1),
        KeyCode::Down | KeyCode::Char('j') => modal.move_by(1),
        KeyCode::Enter => {
            if !modal.rows.is_empty() {
                modal.detail = !modal.detail;
            }
        }
        KeyCode::Char('?') => modal.help = true,
        KeyCode::Esc => {
            if modal.detail {
                modal.detail = false;
            } else {
                app.tasks = None;
            }
        }
        KeyCode::Char(c) => return run_task_action(app, c),
        _ => {}
    }
    Ok(())
}

fn run_task_action(app: &mut App, key: char) -> Result<()> {
    let Some(modal) = &mut app.tasks else {
        return Ok(());
    };
    let Some(action) = tasks::action_for(key) else {
        return Ok(());
    };

    // What the action needs off the selected row, taken before anything
    // borrows the modal mutably.
    let selected = modal
        .selected_row()
        .map(|r| (r.id.clone(), r.status.clone()));

    let status = match action {
        tasks::Action::Close => {
            if modal.detail {
                modal.detail = false;
            } else {
                app.tasks = None;
            }
            return Ok(());
        }
        tasks::Action::Add => {
            modal.form = Some(tasks::Form::capture());
            return Ok(());
        }
        tasks::Action::Edit => {
            if let Some(row) = modal.selected_row() {
                modal.form = Some(tasks::Form::edit(row));
            }
            return Ok(());
        }
        tasks::Action::Closed => {
            modal.show_closed = !modal.show_closed;
            let shown = modal.show_closed;
            reload_tasks(
                app,
                Some(if shown {
                    "showing done and dropped".into()
                } else {
                    "open tasks only".into()
                }),
            );
            return Ok(());
        }
        tasks::Action::Source => {
            // A task nobody captured from anywhere has no original, and that
            // is said rather than answered with an empty reader. "Captured
            // here" and "the read failed" must never print the same.
            let Some(row) = modal.selected_row() else {
                return Ok(());
            };
            let Some(captured) = row.captured_from.clone() else {
                modal.status = Some("captured on the board itself — no earlier original".into());
                return Ok(());
            };
            let (id, word) = (row.id.clone(), captured.word().to_string());
            let handle = captured
                .label
                .clone()
                .unwrap_or_else(|| format!("{} {}", captured.kind, captured.id));
            modal.status = Some(format!("reading the {word}…"));
            spawn_task_source(app, &id, &handle);
            return Ok(());
        }
        tasks::Action::Refresh => None,
        tasks::Action::Status(status) => Some(status),
        tasks::Action::Cycle => modal.next_in_cycle(),
    };

    let Some(status) = status else {
        reload_tasks(app, None);
        return Ok(());
    };
    let Some((id, was)) = selected else {
        return Ok(());
    };
    let note = match tasks_cli(&["set", &id, "--status", status]) {
        Ok(_) => format!("{was} → {status}"),
        Err(e) => format!("could not set {status}: {e}"),
    };
    reload_tasks(app, Some(note));
    Ok(())
}

/// Keys while a capture or an edit is being typed.
///
/// Submitting sends what is on screen, which for an edit is what the task
/// already is with whatever changed — the form was prefilled, so the fields
/// nobody touched arrive as the values they already had. That is why the
/// empty string means "clear" on the tool and is passed through as such: an
/// emptied box is somebody clearing a date, not a field they left alone.
fn handle_tasks_form_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.tasks else {
        return Ok(());
    };
    let Some(form) = &mut modal.form else {
        return Ok(());
    };

    match key.code {
        KeyCode::Esc => modal.form = None,
        KeyCode::Tab | KeyCode::Down => form.move_by(1),
        KeyCode::BackTab | KeyCode::Up => form.move_by(-1),
        KeyCode::Backspace => form.backspace(),
        KeyCode::Enter => return submit_task_form(app),
        _ => {
            if let Some(c) = typed_char(&key) {
                form.push(c);
            }
        }
    }
    Ok(())
}

fn submit_task_form(app: &mut App) -> Result<()> {
    let Some(modal) = &mut app.tasks else {
        return Ok(());
    };
    let Some(form) = &modal.form else {
        return Ok(());
    };

    let editing = form.editing.clone();
    let (due, defer, context, project, name) = (
        form.value("due").trim().to_string(),
        form.value("defer").trim().to_string(),
        form.value("context").trim().to_string(),
        form.value("project").trim().to_string(),
        form.value("name").trim().to_string(),
    );

    let result = match &editing {
        Some(id) => tasks_cli(&[
            "set",
            id,
            "--due",
            &due,
            "--defer",
            &defer,
            "--context",
            &context,
        ])
        .map(|_| "schedule saved".to_string()),
        None if name.is_empty() => Err(anyhow::anyhow!("a task needs a name")),
        None => {
            let mut args = vec!["add"];
            for (flag, value) in [
                ("--due", &due),
                ("--project", &project),
                ("--context", &context),
            ] {
                if !value.is_empty() {
                    args.extend([flag, value.as_str()]);
                }
            }
            // `--` before the name, always: a task called "-- rewrite the
            // intro" is a task, and without the separator it is a parse error
            // about an unknown flag.
            args.push("--");
            args.push(&name);
            tasks_cli(&args).map(|_| "captured".to_string())
        }
    };

    match result {
        Ok(note) => {
            modal.form = None;
            reload_tasks(app, Some(note));
        }
        // The form stays open with the typing intact, showing the graph's own
        // refusal — an unparseable date or a project it does not have. Closing
        // it would lose the words and teach nothing about why.
        Err(e) => {
            if let Some(form) = &mut modal.form {
                form.error = Some(format!("{e:#}"));
            }
        }
    }
    Ok(())
}

/// Run `mecha <args...>` and return its output.
///
/// `current_exe` rather than a bare `mecha`, so a TUI started from
/// `target/debug` drives the build it is part of and not whatever is on PATH —
/// otherwise testing a change to a subcommand would silently exercise the
/// installed binary. Every modal mutation goes through here: one
/// implementation of each verb, and no way for the TUI to do something the
/// command line cannot.
/// One pane's rows, from its own `list --json`.
///
/// A child process rather than a store read, on `/triggers`' rule and for the
/// same payoff: one implementation per verb, and nothing the modal can show
/// that the command line cannot. Cheap enough to be the reload after every
/// action — none of these commands calls a model or touches the network.
fn load_learning(pane: learning::Pane) -> Result<Vec<learning::Row>> {
    // `--all` on the Reflections pane only: `reflections list` hides
    // dropped rows by default, which made `u restore` unreachable here —
    // `d` succeeding removed the row from the very list the reload re-reads.
    // `Row::spent` exists to render a drop as shown-and-past, which is the
    // right default for a surface whose own argument is that a refusal
    // must stay visible. Rules and Proposals need no such flag: a retired
    // rule stays in `rules list`'s output, and a proposal is never dropped
    // at all.
    let mut args: Vec<&str> = vec![pane.verb(), "list"];
    if pane == learning::Pane::Reflections {
        args.push("--all");
    }
    args.push("--json");
    let text = self_cli(&args)?;
    learning::rows_from_json(pane, &text)
}

/// Reload the current pane and put the cursor back on the same record.
fn reload_learning(app: &mut App) {
    let Some(m) = &app.learning else { return };
    let pane = m.pane;
    match load_learning(pane) {
        Ok(rows) => {
            if let Some(m) = &mut app.learning {
                m.reload(rows);
            }
        }
        Err(e) => {
            if let Some(m) = &mut app.learning {
                m.status = Some(format!("could not reload: {e:#}"));
            }
        }
    }
}

/// Run one `mecha …` verb against the selected record, off the event loop.
///
/// **Not synchronous**, despite every verb here being a local file rewrite
/// that returns in milliseconds on its own: `reflections`/`rules`/`proposals`
/// all take `LearningStore::lock()` — a *blocking* flock — and `reflect` and
/// `learn` hold that same lock across a model call while they run. Waiting
/// for it inline would freeze the whole event loop for however long that
/// call takes, in the one modal whose whole job is reading a rule or a
/// lesson carefully before it enters every future prompt's cached prefix.
/// Spawned detached and watched instead, the `/outbox`/`/triggers` shape: a
/// poll against the child's own stdout (`Watch::Learning`), never the event
/// loop blocking on it, and the modal reloaded once it answers.
fn learning_act(app: &mut App, verb: &str, extra: &[&str]) {
    let Some(m) = &mut app.learning else { return };
    let Some(row) = m.selected().cloned() else {
        m.status = Some("nothing selected".into());
        return;
    };
    if m.busy {
        return;
    }
    m.busy = true;
    let pane = m.pane;
    let mut args: Vec<String> = vec![pane.verb().to_string(), verb.to_string(), row.id.clone()];
    args.extend(extra.iter().map(|s| s.to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    m.status = Some(format!("{verb} {}…", learning_short(&row.id)));
    app.watches.push(Watch::Learning {
        rx,
        verb: verb.to_string(),
        id: row.id,
        since: std::time::Instant::now(),
    });
}

fn learning_short(id: &str) -> String {
    id.chars().take(20).collect()
}

/// `/learning`'s keys.
///
/// Every mutation goes through a `mecha …` child and then reloads, so what the
/// list shows after a keypress is what the store says rather than what the
/// modal assumed — which is the same reason `/tasks` re-reads instead of
/// patching its rows in place.
fn handle_learning_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.learning else {
        return Ok(());
    };
    if modal.help {
        modal.help = false;
        return Ok(());
    }
    match key.code {
        KeyCode::Char('?') => modal.help = true,
        KeyCode::Esc => {
            // The detail first, then the modal: Esc closes the innermost
            // thing, so reading a record and then leaving is two presses and
            // never one that closes both.
            if modal.detail.is_some() {
                modal.detail = None;
                modal.detail_scroll = 0;
            } else {
                app.learning = None;
            }
        }
        KeyCode::Char('j') | KeyCode::Down => match modal.detail.is_some() {
            true => modal.detail_scroll = modal.detail_scroll.saturating_add(1),
            false => modal.move_by(1),
        },
        KeyCode::Char('k') | KeyCode::Up => match modal.detail.is_some() {
            true => modal.detail_scroll = modal.detail_scroll.saturating_sub(1),
            false => modal.move_by(-1),
        },
        KeyCode::Tab | KeyCode::BackTab => {
            let next = match key.code {
                KeyCode::BackTab => modal.pane.prev(),
                _ => modal.pane.next(),
            };
            match load_learning(next) {
                Ok(rows) => modal.set_pane(next, rows),
                Err(e) => modal.status = Some(format!("{}: {e:#}", next.label())),
            }
        }
        KeyCode::Enter => {
            if modal.detail.is_some() {
                modal.detail = None;
                modal.detail_scroll = 0;
            } else if let Some(row) = modal.selected().cloned() {
                let verb = modal.pane.verb();
                match self_cli(&[verb, "show", &row.id]) {
                    Ok(text) => {
                        if let Some(m) = &mut app.learning {
                            m.detail = Some(text);
                            m.detail_scroll = 0;
                        }
                    }
                    Err(e) => modal.status = Some(format!("could not read it: {e:#}")),
                }
            }
        }
        // The verbs, per pane. A key that does nothing in the pane you are
        // looking at is worse than no key, so each is matched with its pane
        // rather than dispatched from one table.
        KeyCode::Char('e') if modal.pane == learning::Pane::Reflections => {
            if let Some(row) = modal.selected() {
                app.pending_lesson_edit = Some(row.id.clone());
            }
        }
        KeyCode::Char('d') if modal.pane == learning::Pane::Reflections => {
            learning_act(app, "drop", &[])
        }
        KeyCode::Char('u') if modal.pane != learning::Pane::Proposals => {
            // A user rule (or a pre-identity learned one) has no id
            // `find_rule` can resolve on its own — `""` prefix-matches every
            // learned rule that has one, so acting on it would restore an
            // unrelated rule rather than fail cleanly.
            if modal.pane == learning::Pane::Rules
                && modal.selected().is_some_and(|r| r.mine || r.id.is_empty())
            {
                modal.status = Some("not a learned rule — nothing to restore".into());
            } else {
                learning_act(app, "restore", &[])
            }
        }
        KeyCode::Char('x') if modal.pane == learning::Pane::Rules => {
            if modal.selected().is_some_and(|r| r.mine || r.id.is_empty()) {
                modal.status = Some("not a learned rule — nothing to retire".into());
            } else {
                learning_act(app, "retire", &[])
            }
        }
        KeyCode::Char('a') if modal.pane == learning::Pane::Proposals => {
            learning_act(app, "accept", &[])
        }
        KeyCode::Char('r') if modal.pane == learning::Pane::Proposals => {
            // `mecha proposals reject` wants a reason and the help text here
            // promises one is recorded — `/queues`' own rule for the same
            // command: a reason nobody typed is worse than none, so say
            // where it came from rather than inventing a justification.
            learning_act(app, "reject", &["--reason", "rejected from /learning"])
        }
        _ => {}
    }
    Ok(())
}

/// `e` on a reflection: hand the terminal to `$EDITOR`, take it back, reload.
///
/// Through `mecha reflections edit`, so the promotion the CLI does — the
/// lesson becomes the owner's, the context is withheld, the provenance moves
/// — happens once and in one place, and the TUI cannot do something the
/// command line cannot.
fn suspend_and_edit_lesson(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
    id: &str,
) -> Result<()> {
    let result = with_terminal_suspended(terminal, || {
        self_cli_interactive(&["reflections", "edit", id])
    })?;
    if let Some(modal) = &mut app.learning {
        modal.status = Some(match &result {
            Ok(_) => format!("edited {} — the lesson is yours now", learning_short(id)),
            Err(e) => format!("{} unchanged: {e}", learning_short(id)),
        });
    }
    // Loud as well as in the modal, like a rejected outbox edit: a failed
    // edit must not surface only when the lesson turns up in a rule.
    if let Err(e) = &result {
        app.transcript
            .push(Entry::Error(format!("reflections edit: {e:#}")));
    }
    reload_learning(app);
    Ok(())
}

fn self_cli(args: &[&str]) -> Result<String> {
    let exe = crate::exe::self_exe();
    let out = std::process::Command::new(exe)
        .args(args)
        .output()
        .with_context(|| format!("running mecha {}", args.first().unwrap_or(&"")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("{}", err.trim().lines().next().unwrap_or("failed"))
    }
}

fn trigger_cli(args: &[&str]) -> Result<String> {
    let mut full = vec!["trigger"];
    full.extend_from_slice(args);
    self_cli(&full)
}

/// Start `mecha <args...>` and do not wait for it. Output goes nowhere: the
/// TUI owns the screen, and the work's real record is the store it writes —
/// a trigger's ledger, an outbox item's status, a frontdoor record's state.
///
/// stdin is null on purpose beyond tidiness: a child that asks a question
/// gets EOF, and EOF means "no" on every surface here — so a detached send
/// or triage can never sit blocked on a confirmation nobody can see.
fn spawn_detached(args: &[&str]) -> Result<()> {
    let exe = crate::exe::self_exe();
    std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting it")?;
    Ok(())
}

/// The suspend/restore dance around handing the terminal to another process.
///
/// The suspend mirrors `leave()` and the restore mirrors `enter()`, minus the
/// panic hook (still installed) and the probe (already answered — the kitty
/// flags are re-pushed if they were pushed before). The full `terminal.clear`
/// afterwards is load-bearing: whatever ran drew over everything, and a diff
/// against the pre-suspend buffer would restore only what happened to change.
fn with_terminal_suspended<T>(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    f: impl FnOnce() -> T,
) -> Result<T> {
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
    MOUSE_CAPTURED.store(false, std::sync::atomic::Ordering::SeqCst);

    let result = f();

    enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    // The next frame reconciles this against what is on screen, so a pane
    // that had handed the mouse back gets it back again without asking.
    MOUSE_CAPTURED.store(true, std::sync::atomic::Ordering::SeqCst);
    if kitty_pushed() {
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    terminal.clear()?;
    Ok(result)
}

/// Run `mecha <args...>` while the caller has suspended the TUI — for
/// subcommands that open `$EDITOR` themselves. stdin and stdout are inherited,
/// because the editor needs the real terminal: `self_cli`'s capture would hand
/// vim a pipe for a screen and a closed stdin for a keyboard. Only stderr is
/// captured, so a refusal's text can reach the modal's status line.
fn self_cli_interactive(args: &[&str]) -> Result<()> {
    let exe = crate::exe::self_exe();
    let child = std::process::Command::new(exe)
        .args(args)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("starting it")?;
    let out = child.wait_with_output().context("waiting for it")?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("{}", err.trim().lines().next().unwrap_or("failed"))
    }
}

/// Hand the terminal to `$EDITOR` with the current input, and take both back.
fn suspend_and_edit(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
) -> Result<()> {
    let result = with_terminal_suspended(terminal, || {
        crate::editor::edit_text(
            &app.input,
            &format!("mecha-compose-{}.txt", std::process::id()),
        )
    })?;

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
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
    name: &str,
) -> Result<()> {
    // Interactive, not captured: `trigger edit` opens `$EDITOR`, which needs
    // the real terminal this function just suspended.
    let result = with_terminal_suspended(terminal, || {
        self_cli_interactive(&["trigger", "edit", name])
    })?;

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

/// Edit the charter in `$EDITOR` — on the file itself — then reload the
/// modal and say what happened, where "what happened" is read off the file
/// rather than the editor's exit code.
///
/// The one write mecha ever makes here is the comments-only
/// `charter::TEMPLATE` when the file does not exist yet; every `[[line]]`
/// is the owner's own typing. Validation feedback is the reason this beats
/// a hand-run `vi`: a duplicate id or a typo'd table name is reported the
/// moment the editor closes, not at the next session's startup where the
/// alternate screen covers the warning.
fn suspend_and_edit_charter(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
) -> Result<()> {
    let Some(modal) = &app.charter else {
        return Ok(());
    };
    let path = modal.path.clone();
    // The template write, the hand-over, and the did-anything-actually-land
    // classification all live in `editor::edit_charter_with`, shared with
    // `mecha charter edit` — the two traps in it (a clean exit that saved
    // nothing, a `:cq` that exited non-zero after a save landed) are exactly
    // the kind that a second copy gets subtly wrong.
    use crate::editor::CharterEdit;
    // **The suspend wraps the whole thing, and its `?` is at function scope**
    // — the shape every sibling hand-over keeps, and getting it wrong here
    // was a real defect rather than a style point.
    //
    // `with_terminal_suspended` returns `Result<Result<_>>`: the outer error
    // is the suspend/restore dance itself (`disable_raw_mode`,
    // `LeaveAlternateScreen`, and crucially `enable_raw_mode` *after* the
    // editor returns), the inner one is the editor. Putting the `?` inside a
    // closure handed to `edit_charter_with` folded the first into the second,
    // so a failed `enable_raw_mode` — the alternate screen never re-entered,
    // raw mode off — was classified as `CharterEdit::EditorFailed` and
    // reported as "charter unchanged: …" while the TUI carried on drawing
    // into a terminal that no longer takes input. A terminal that could not
    // be restored is not an editor failure, and the classifier below has no
    // arm that means it.
    let outcome = with_terminal_suspended(terminal, || {
        crate::editor::edit_charter_with(&path, crate::editor::edit_file)
    })?;

    let mut editor_error = None;
    if let Some(modal) = &mut app.charter {
        // The honest clause in every "saved" arm: the charter is rendered
        // into the system prompt at agent build, so a saved edit changes the
        // next session (or this one after /model), never this conversation.
        modal.status = Some(match &outcome {
            // A store failure here is a status line, never a `?` — every
            // sibling hand-over routes its store errors into the modal, and
            // a full disk must not take down the whole session (partial
            // answer and all) over a template nobody typed.
            Err(e) => format!("{e:#}"),
            Ok(CharterEdit::TemplateCreated) => {
                "the template is in place — no lines yet; e edits it".to_string()
            }
            Ok(CharterEdit::Unchanged) => "unchanged".to_string(),
            Ok(CharterEdit::Saved) => {
                "saved — rides in the prompt from the next session (/model rebuilds this one)"
                    .to_string()
            }
            Ok(CharterEdit::SavedButInvalid(e)) => format!(
                "saved, but it will NOT load: {e} — every run starts uncharted until this is fixed (e re-edits)"
            ),
            Ok(CharterEdit::EditorFailedButChanged { error, loads }) => {
                // **Both editor-failure arms reach the transcript**, which
                // the refactor briefly stopped doing for this one. The old
                // shape was `if let Err(e) = result`, and that covered a
                // non-zero exit whether or not the file changed. A modal
                // status is transient — it goes when the modal closes — and
                // the transcript is the durable record of a thing that went
                // wrong, so dropping it here would have quietly narrowed
                // what a session keeps.
                editor_error = Some(error.clone());
                match loads {
                    None => format!(
                        "the editor exited with an error ({error}), but the file changed and loads"
                    ),
                    Some(le) => format!(
                        "the editor exited with an error ({error}); the file changed and will NOT load: {le}"
                    ),
                }
            }
            Ok(CharterEdit::EditorFailed(e)) => {
                editor_error = Some(e.clone());
                format!("charter unchanged: {e}")
            }
        });
        modal.reload();
    }
    if let Some(e) = editor_error {
        app.transcript
            .push(Entry::Error(format!("charter editor: {e}")));
    }
    Ok(())
}

/// Edit an outbox draft in `$EDITOR`, then reload the modal.
///
/// Saving goes through `mecha outbox edit`'s own path — which opens the prose
/// and writes it back to the field it came from, and never touches
/// `args_before` — so the learning capture that mines `diff(staged, sent)`
/// sees the TUI's edits exactly as it sees the command line's.
fn suspend_and_edit_outbox(
    terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>,
    app: &mut App,
    id: &str,
) -> Result<()> {
    let result =
        with_terminal_suspended(terminal, || self_cli_interactive(&["outbox", "edit", id]))?;

    if let Some(modal) = &mut app.staged {
        modal.status = Some(match &result {
            Ok(_) => format!("edited `{id}` — send releases the new arguments"),
            Err(e) => format!("`{id}` unchanged: {e}"),
        });
    }
    // Loud as well as in the title, same as a trigger edit: a rejected edit
    // must not surface only when the draft goes out unrevised.
    if let Err(e) = result {
        app.transcript
            .push(Entry::Error(format!("outbox `{id}` was not edited: {e}")));
    }
    reload_outbox(app);
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

/// Hand anything typed in the mirrored thread to this session.
///
/// Steering when a run is in flight and a fresh turn when it is not — the same
/// two paths a keystroke takes, because a line from a phone is the same user
/// saying the same thing from somewhere else.
///
/// **Slash commands and `!` escapes are refused here, and that is the one real
/// policy decision in this function.** They are not prompts: `/model` rebuilds
/// the agent, `/clear` drops the conversation and its taint, and `!` runs a
/// shell command with no approver in front of it. Those are affordances of
/// sitting at the machine, and the gap between "the owner typed this" and "the
/// owner is at the keyboard" is exactly where a remote surface should stay
/// narrow. Refusing costs a sentence; allowing costs a shell.
fn deliver_inbound(
    app: &mut App,
    live: &Live,
    session: Option<&Session>,
    events_tx: &mut mpsc::UnboundedSender<AgentEvent>,
    events_rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
) {
    let Some(attached) = app.attached.clone() else {
        return;
    };
    let Ok(store) = crate::slack::remote::RemoteStore::open_default() else {
        return;
    };
    let lines = match store.claim_inbound(&attached.name) {
        Ok(lines) => lines,
        Err(e) => {
            tracing::warn!("could not read the remote inbox: {e:#}");
            return;
        }
    };

    let workspace = live.agent.context().tools.workspace.clone();

    for line in lines {
        // **Files first, and independently of the text.** A screenshot sent
        // with a caption that happens to be a slash command is still a
        // screenshot the user meant to send, and discarding it to punish the
        // caption would lose the thing they cared about.
        let landed = match store.take_files(&attached.name, &line.files, &workspace) {
            Ok(landed) => landed,
            Err(e) => {
                // Said in the thread as well as the terminal. The connector's
                // own rule, two files over: a file that silently did not
                // arrive is indistinguishable from one the session chose to
                // ignore, and the person is on a phone with no way to tell.
                app.transcript
                    .push(Entry::Error(format!("could not save an attachment: {e:#}")));
                spawn_note(
                    &attached,
                    &format!("Could not save your attachment into the workspace: {e:#}"),
                );
                Vec::new()
            }
        };
        // **The file lands either way; the pixels are the extra.** Writing it
        // into the workspace is the durable half — it is what `fs_read`,
        // `shell` and tomorrow's run reach. Putting the image *into the turn*
        // is what makes it a feature, and it is conditional on there being
        // eyes: a text-only model would be handed a resized JPEG only to
        // render it as its own filename.
        let mut images = Vec::new();
        if live.agent.vision() {
            for rel in &landed {
                let path = workspace.join(rel.trim_start_matches("./"));
                match mecha_core::image::block_from_path(&path) {
                    Ok(Some(block)) => images.push(block),
                    Ok(None) => {}
                    // Reported in the thread as well as the terminal, on the
                    // rule two lines up: from the phone's side a picture was
                    // sent, and a picture that silently did not arrive is
                    // indistinguishable from one the session ignored.
                    Err(e) => {
                        app.transcript
                            .push(Entry::Error(format!("could not look at {rel}: {e:#}")));
                        spawn_note(
                            &attached,
                            &format!("Saved {rel}, but could not look at it: {e:#}"),
                        );
                    }
                }
            }
        }
        if !landed.is_empty() {
            app.transcript
                .push(Entry::Notice(format!("⇄ saved {}", landed.join(", "))));
            spawn_note(
                &attached,
                &format!("Saved to the workspace: {}", landed.join(", ")),
            );
        }

        if command::parse(&line.text).is_some() || command::shell_escape(&line.text).is_some() {
            let refusal = if landed.is_empty() {
                "Commands and `!` shell escapes only work at the terminal. Send a prompt \
                 instead."
                    .to_string()
            } else {
                format!(
                    "The attachment was saved. Commands and `!` shell escapes only work at \
                     the terminal, so `{}` was not run.",
                    line.text.trim()
                )
            };
            app.transcript.push(Entry::Notice(format!(
                "refused a command from Slack: {}",
                line.text
            )));
            spawn_note(&attached, &refusal);
            continue;
        }

        // Named as paths, never injected as content — the connector's rule for
        // its own attachments, and the right one: the model reaches the bytes
        // with `fs_read`, which already declares `private_data`, so the taint
        // arms through the path that exists rather than a parallel one.
        let mut prompt = line.text.trim().to_string();
        if !landed.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str("The user attached:\n");
            for path in &landed {
                prompt.push_str(&format!("- {path}\n"));
            }
        }
        if prompt.trim().is_empty() {
            continue;
        }

        // Marked, because the person at the terminal did not type it and the
        // difference matters when two people are looking at one session.
        app.transcript
            .push(Entry::Notice(format!("⇄ from Slack · {}", attached.name)));
        // **Never propagated.** Claiming already removed these from the store,
        // so an error escaping here would end the session *and* take the
        // remaining lines with it — they are gone from disk and would never be
        // delivered anywhere. One line failing is a line to report, not a
        // reason to stop reading the rest.
        if let Err(e) = submit(
            app, prompt, images, events_tx, events_rx, live, session, true,
        ) {
            app.transcript.push(Entry::Error(format!(
                "could not deliver a Slack line: {e:#}"
            )));
        }
    }
}

/// Say something in the mirrored thread that is not part of a run.
fn spawn_note(attached: &crate::slack::remote::Attached, text: &str) {
    let (slack, channel, thread_ts) = (
        attached.slack.clone(),
        attached.channel_id.clone(),
        attached.thread_ts.clone(),
    );
    let body = text.to_string();
    tokio::spawn(async move {
        let _ =
            mecha_slack::chat::post_message(&slack, &channel, Some(&thread_ts), &body, None).await;
    });
}

/// Echo what the user typed into the mirrored thread.
///
/// Fire-and-forget, and deliberately not reported on failure: the run is
/// already underway by the time this could fail, and an error line about a
/// missing echo would interrupt the thing the user is actually doing. The
/// consequence of losing one is a thread that reads slightly oddly, which the
/// answer beneath it still makes sense of.
fn spawn_echo(
    attached: &crate::slack::remote::Attached,
    text: &str,
    steering: bool,
) -> JoinHandle<()> {
    let (slack, channel, thread_ts) = (
        attached.slack.clone(),
        attached.channel_id.clone(),
        attached.thread_ts.clone(),
    );
    let body = crate::slack::remote::echo_text(text, steering);
    tokio::spawn(async move {
        let _ =
            mecha_slack::chat::post_message(&slack, &channel, Some(&thread_ts), &body, None).await;
    })
}

/// Claim a name and open its thread, off the event loop.
///
/// Everything decidable locally has already been decided by the caller — an
/// existing attachment, a missing session — so what is left here is the part
/// that genuinely needs the network, and its only failure mode is reported as
/// one line.
#[allow(clippy::too_many_arguments)]
fn spawn_attach(
    name: String,
    session_id: String,
    workspace: PathBuf,
    model: String,
    taint: (bool, bool),
    prior_messages: usize,
    tx: mpsc::UnboundedSender<AttachOutcome>,
) {
    tokio::spawn(async move {
        let outcome = match crate::slack::remote::attach(
            &name,
            &session_id,
            &workspace,
            &model,
            taint,
            prior_messages,
        )
        .await
        {
            Ok((attached, notice)) => AttachOutcome::Attached(Box::new(attached), notice),
            Err(e) => AttachOutcome::Failed {
                name: Some(name.clone()),
                error: format!("/remote-control {name}: {e:#}"),
            },
        };
        let _ = tx.send(outcome);
    });
}

/// End an attachment, off the event loop.
///
/// The handle is already gone from `App` by the time this runs — taken by the
/// caller — so a failure here reports the failure and never resurrects it. A
/// record that says cold while the interface says live is the disagreement
/// this whole surface exists to prevent, and the store is the one that wins.
fn spawn_detach(
    attached: crate::slack::remote::Attached,
    reason: &'static str,
    tx: mpsc::UnboundedSender<AttachOutcome>,
) {
    tokio::spawn(async move {
        let name = attached.name.clone();
        let outcome = match crate::slack::remote::detach(&attached, reason).await {
            Ok(()) => AttachOutcome::Detached(format!(
                "detached `{name}` — the thread and everything in it stay"
            )),
            Err(e) => AttachOutcome::Failed {
                name: None,
                error: format!("/remote-control off: {e:#}"),
            },
        };
        let _ = tx.send(outcome);
    });
}

/// Upload a file to the owner's Slack DM, reporting into the transcript.
///
/// In-process and async rather than a detached `mecha slack send`, on the
/// `run_shell_escape` precedent above: the work is one upload, the outcome is
/// one line, and there is nothing for a person to interact with in between.
/// `/triggers` shells out because firing a trigger builds an entire agent and
/// can run for twenty minutes, which is a different size of thing. The rule
/// that actually matters — nothing the TUI can do that the command line
/// cannot — is kept by both surfaces calling the same function, not by both
/// spawning the same process.
///
/// The failure is reported and never retried. A retry would double-post on
/// the half of the three-step upload that is not idempotent, and a duplicate
/// file in a DM is a worse answer than a line saying what went wrong.
fn spawn_send(path: PathBuf, tx: mpsc::UnboundedSender<Entry>) {
    tokio::spawn(async move {
        let entry = match crate::slack::send::send_file(&path, None).await {
            Ok(sent) => Entry::Notice(format!(
                "sent {} ({}) to your Slack DM",
                sent.filename,
                crate::slack::send::human(sent.bytes)
            )),
            // `{:#}` so the context chain arrives — "uploading chart.png:
            // nothing is bound" is the whole diagnosis, where either half
            // alone sends the reader looking in the wrong place.
            Err(e) => Entry::Error(format!("/send {}: {e:#}", path.display())),
        };
        // The receiver only closes when the TUI is exiting; an outcome
        // arriving after that has nowhere sensible to go anyway.
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

/// The character a key event actually typed, or `None` if it was a chord.
///
/// `KeyCode::Char('c')` with CONTROL held **is** Ctrl-C — crossterm reports
/// the modifier beside the letter rather than in place of it, so a `match` on
/// `KeyCode::Char(c)` alone sees the bare letter and cannot tell the two
/// apart. Every text field and every key table in this file did exactly that,
/// which was harmless in the input box (the `ctrl` branch runs first and
/// consumes them) and not harmless anywhere else:
///
/// - Six text inputs — an outbox rejection reason, a frontdoor note, a poll
///   note, the task form, a mail reason, the /docs paste field — inserted a
///   literal `c` when someone pressed Ctrl-C to back out.
/// - `/mail` was worse than cosmetic. Its keys go through `action_for`, so
///   **Ctrl-A archived the selected thread, Ctrl-D dismissed it, Ctrl-T made
///   a task of it and Ctrl-R started a drafting run** — on chords that mean
///   beginning-of-line, delete and refresh everywhere else in a terminal.
///
/// A helper rather than seven guards, on the `list_height` rule: a new modal
/// is written by copying whichever sibling is nearest, so the fix has to be
/// the thing that gets copied. SHIFT is *not* a chord — that is how capitals
/// arrive under the kitty protocol.
fn typed_char(key: &KeyEvent) -> Option<char> {
    let chord = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER;
    match key.code {
        KeyCode::Char(c) if !key.modifiers.intersects(chord) => Some(c),
        _ => None,
    }
}

fn prev_boundary(s: &str, at: usize) -> Option<usize> {
    s[..at].char_indices().next_back().map(|(i, _)| i)
}

fn next_boundary(s: &str, at: usize) -> usize {
    s[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// How the input box's text is laid out: which byte range each visual row
/// shows, and where the caret sits among them.
///
/// **This is the only wrapper the input box has**, and that is the fix rather
/// than an implementation detail. The box used to hand its text to
/// `Paragraph::wrap(Wrap { trim: false })` — ratatui's `WordWrapper`, which
/// breaks at *word* boundaries and measures in *display cells* — while this
/// function counted *characters* and broke at exactly `width`. Two
/// implementations of one question, and the caret followed this one while the
/// text followed that one, so they parted company the moment a word straddled
/// the right edge, which is most lines of prose. Measured on
/// `"can you help me prepare my schedule for my undergrad fMRI class"` at 30
/// columns: ratatui painted a last row reading `"class"` and the caret was
/// placed at column 3 of it. Wide characters were the same bug twice — one
/// CJK glyph is two cells and was counted as one.
///
/// So `draw` now renders the rows this returns and there is no `.wrap()`
/// anywhere near the input box. Nothing is left to disagree.
struct InputLayout {
    /// One byte range per visual row. A **partition** of `0..text.len()`:
    /// contiguous, gapless, covering everything — so no character can be shown
    /// twice or vanish, and the caret's row can be found by containment rather
    /// than by re-deriving the wrap a second time. A hard newline belongs to
    /// the row it ends.
    rows: Vec<std::ops::Range<usize>>,
    cursor_col: u16,
    cursor_row: u16,
}

/// How many terminal cells one character occupies. Control characters are
/// zero rather than an error: they still occupy *bytes*, and the partition
/// above is what keeps them from being silently eaten.
fn cell_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Greedy word wrap: a word moves down whole rather than being split, unless
/// it cannot fit a row at all, in which case it breaks at the edge.
fn input_layout(text: &str, cursor: usize, width: u16) -> InputLayout {
    // A pty with no window size reports zero columns, and this is the
    // arithmetic that runs first.
    let width = (width.max(1)) as usize;
    let mut rows: Vec<std::ops::Range<usize>> = Vec::new();
    // Where the row being built starts, and how many cells it has used.
    let (mut start, mut col) = (0usize, 0usize);

    let mut chars = text.char_indices().peekable();
    while let Some(&(i, ch)) = chars.peek() {
        if ch == '\n' {
            chars.next();
            // The newline belongs to the row it ends — that is what keeps the
            // ranges a partition, and it is what puts a caret typed just
            // before it on the right row rather than the next one.
            rows.push(start..i + 1);
            (start, col) = (i + 1, 0);
            continue;
        }
        if ch.is_whitespace() {
            // A space at the wrap point stays on the row it came from, even
            // past the edge, where the terminal clips it and nobody sees it.
            // The alternative — carrying it down — shifts the visible text of
            // every continuation row one column right, which is ugly on every
            // wrapped line to fix something invisible. It is also what ratatui
            // painted, so the box does not change shape under the fix.
            chars.next();
            col += cell_width(ch);
            continue;
        }

        // A word: the run of non-whitespace beginning here, measured before
        // anything is committed, because the decision is about the whole of it.
        let (mut end, mut w) = (i, 0usize);
        while let Some(&(j, c)) = chars.peek() {
            if c.is_whitespace() {
                break;
            }
            chars.next();
            end = j + c.len_utf8();
            w += cell_width(c);
        }
        if col > 0 && col + w > width {
            rows.push(start..i);
            (start, col) = (i, 0);
        }
        if col + w <= width {
            col += w;
            continue;
        }
        // Longer than a whole row even from column zero — a pasted URL, a
        // path. Break it wherever the edge falls; there is nowhere better.
        for (o, c) in text[i..end].char_indices() {
            let (j, cw) = (i + o, cell_width(c));
            if col > 0 && col + cw > width {
                rows.push(start..j);
                (start, col) = (j, 0);
            }
            col += cw;
        }
    }
    rows.push(start..text.len());

    let cursor = cursor.min(text.len());
    let (mut cursor_row, mut cursor_col) = (0u16, 0u16);
    for (n, r) in rows.iter().enumerate() {
        // `cursor == r.end` belongs to the *next* row: that is where the next
        // character typed will appear, and the caret has to be where the
        // character will be. The last row keeps it, having no next.
        if cursor < r.end || n + 1 == rows.len() {
            let upto = cursor.clamp(r.start, r.end);
            cursor_row = n as u16;
            cursor_col = text[r.start..upto].chars().map(cell_width).sum::<usize>() as u16;
            break;
        }
    }
    // A caret at or past the right edge belongs at the start of the next row —
    // that is where the next character will land, and a caret drawn on the
    // border is a caret in the wrong place. Two ways to be there: a row that
    // filled exactly (rows break lazily, when the character that overflows
    // arrives, so the next row does not exist yet) or a trailing space parked
    // past the edge by the rule above.
    if cursor_col as usize >= width {
        if cursor_row as usize + 1 == rows.len() {
            rows.push(text.len()..text.len());
        }
        cursor_row += 1;
        cursor_col = 0;
    }

    InputLayout {
        rows,
        cursor_col,
        cursor_row,
    }
}

/// The tallest the input box's text area gets before it scrolls. Past this the
/// box would be eating the transcript to show a message nobody is reading yet.
const INPUT_ROWS: u16 = 6;

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
    // Past `INPUT_ROWS` it scrolls vertically instead of growing further.
    //
    // The ghost completion is laid out *with* the text, not appended after it:
    // it occupies cells, so a wrap computed without it is a wrap the box does
    // not draw, and the box ends up a row short of its own content.
    let inner_width = frame.area().width.saturating_sub(2);
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
    let tail = if ghost.is_empty() { "" } else { "  tab" };
    let display = format!("{}{ghost}{tail}", app.input);
    let layout = input_layout(&display, app.cursor, inner_width);
    let visible = (layout.rows.len() as u16).clamp(1, INPUT_ROWS);
    let input_height = visible + 2;

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
        Some(run) if run.cancelling => (Color::Red, "stopping"),
        Some(_) => (Color::Yellow, "steer"),
        None => (Color::Cyan, "message"),
    };

    // Keep the caret's row on screen. Scrolling to the *last* visible row
    // rather than the first means typing at the end never jumps the view.
    let scroll = layout
        .cursor_row
        .saturating_sub(visible - 1)
        .min((layout.rows.len() as u16).saturating_sub(visible));

    // The ghost is the rest of what every candidate agrees on, dim, after the
    // cursor — shown rather than applied, so typing on never fights it. Two
    // candidate sources, one mechanism: an `@path` token at the cursor
    // completes against the workspace, anything else against command names.
    // Here it is only a matter of which spans get the dim style: the wrap
    // above already accounted for the cells.
    let (typed_end, ghost_end) = (app.input.len(), app.input.len() + ghost.len());
    let body: Vec<Line> = layout.rows[scroll as usize..]
        .iter()
        .take(visible as usize)
        .map(|r| {
            // The part of this row falling inside `[lo, hi)`. Every bound is a
            // char boundary — the row ranges by construction, the two marks
            // because they are lengths of whole strings — so the slice is safe.
            let clip = |lo: usize, hi: usize| {
                let a = r.start.max(lo).min(r.end);
                let b = r.end.min(hi).max(a);
                // The newline is in the range because the ranges partition the
                // text; it must not be in the row that gets drawn.
                display[a..b].trim_end_matches('\n')
            };
            let dim = Style::new().fg(Color::DarkGray);
            Line::from(vec![
                Span::raw(clip(0, typed_end)),
                Span::styled(clip(typed_end, ghost_end), dim),
                Span::styled(clip(ghost_end, display.len()), dim),
            ])
        })
        .collect();

    // No `.wrap()`: `input_layout` already did it, and a second wrapper is the
    // bug this whole shape exists to remove.
    let title = if layout.rows.len() as u16 > visible {
        // A box that silently looks shorter than its content is the thing the
        // outbox modal's hidden-item counter exists to avoid.
        format!(
            " {hint} · line {}/{} ",
            layout.cursor_row + 1,
            layout.rows.len()
        )
    } else {
        format!(" {hint} ")
    };
    let input = Paragraph::new(body).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border))
            .title(title),
    );
    frame.render_widget(input, chunks[3]);

    // The caret, in the same coordinates the rows above were drawn in. Guarded
    // because a terminal short enough to squeeze the box to its borders has no
    // row to put it on, and pointing at one is how a caret lands on a border.
    if chunks[3].height >= 3 {
        frame.set_cursor_position((
            chunks[3].x + 1 + layout.cursor_col.min(inner_width.saturating_sub(1)),
            chunks[3].y + 1 + layout.cursor_row.saturating_sub(scroll),
        ));
    }

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
        app.help_scroll = draw_help(frame, app.kitty_keyboard, app.help_scroll);
    }
    if let Some(modal) = &app.tools {
        modal.draw(frame);
    }
    if let Some(modal) = &app.skills {
        modal.draw(frame);
    }
    if let Some(modal) = &app.charter {
        modal.draw(frame);
    }
    if let Some(modal) = &app.learning {
        modal.draw(frame);
    }
    if let Some(modal) = &app.scheduled {
        modal.draw(frame);
    }
    if let Some(modal) = &app.staged {
        modal.draw(frame);
    }
    if let Some(modal) = &app.requests {
        modal.draw(frame);
    }
    if let Some(modal) = &app.mail {
        modal.draw(frame);
    }
    if let Some(modal) = &app.find {
        modal.draw(frame);
    }
    if let Some(modal) = &app.documents {
        modal.draw(frame);
    }
    if let Some(modal) = &app.entities {
        modal.draw(frame);
    }
    if let Some(modal) = &app.tasks {
        modal.draw(frame);
    }
    if let Some(modal) = &app.queues {
        modal.draw(frame);
    }
    if let Some(modal) = &app.poll_monitor {
        modal.draw(frame);
    }
    if let Some(modal) = &app.health {
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
/// The reference card. Returns the scroll position actually used, so the
/// caller's copy cannot run past the end of a list only this function knows
/// the length of.
///
/// Two things it used to do silently, both found by looking at it on a real
/// terminal rather than by reading it:
///
/// - **The box was a fixed 70 columns and the lines are not.** `/doctor` and
///   `/review` are past eighty characters, and a `Paragraph` with no `.wrap()`
///   truncates — so the two entries that most needed explaining ended
///   mid-sentence with nothing to say they had. The width now comes from the
///   content, capped by the terminal.
/// - **It clipped vertically with no bottom border and no marker.** On a
///   forty-row terminal the card ends around `/todo`; on a thirty-row one it
///   ends at `/outbox`, which is exactly the picture that reads as "the
///   overlay is broken" rather than "there is more". It scrolls now, and the
///   title says so.
fn draw_help(frame: &mut Frame, kitty: bool, scroll: u16) -> u16 {
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
        (
            "^s",
            "select text with the mouse (the wheel stops until you press it again)".into(),
        ),
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

    // Wide enough for the widest line, and never wider than the terminal.
    let widest = body.iter().map(Line::width).max().unwrap_or(0) as u16;
    let area = centered(
        frame.area(),
        widest.saturating_add(4).max(40),
        (body.len() as u16)
            .saturating_add(2)
            .min(frame.area().height),
    );
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = (body.len().saturating_sub(visible)) as u16;
    let scroll = scroll.min(max_scroll);
    let title = if max_scroll == 0 {
        " help · any key to close ".to_string()
    } else {
        format!(
            " help · {}–{} of {} · ↑↓ scrolls · any other key closes ",
            scroll as usize + 1,
            (scroll as usize + visible).min(body.len()),
            body.len()
        )
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).scroll((scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan))
                .title(title),
        ),
        area,
    );
    scroll
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

/// The height of a modal's list box for a terminal `terminal_height` rows tall.
///
/// Shared because the obvious inline spelling is a crash: `rows.clamp(1,
/// terminal_height.saturating_sub(4))` violates `min <= max` — a panic — the
/// moment the terminal is four rows or fewer, since the subtraction saturates
/// to zero. Flooring the upper bound at one row degrades a tiny terminal to a
/// one-row box instead. Found once in /doctor (F9) and written inline again in
/// /skills, which is what makes it a helper rather than a fix: the next modal
/// copies whichever sibling it happens to open.
fn list_height(rows: u16, terminal_height: u16) -> u16 {
    list_height_reserving(rows, terminal_height, 0)
}

/// The same, for a box that spends `reserved` of its own rows on something
/// that is not the list — /mail renders a key legend inside the block rather
/// than in the title.
///
/// It exists because the two-argument form **could not express that box**, and
/// that is why /mail kept its own spelling and why the sweep that fixed six
/// modals walked past it: the site read `clamp(2, …)` rather than `clamp(1, …)`
/// and matched no grep aimed at the others. It is also worse — a floor of two
/// collides with the ceiling one row *earlier*, so /mail died at five rows
/// where the rest died at four. A helper that cannot say what a caller means is
/// how a caller ends up saying it inline.
///
/// The degradation is the whole reason it is not `assert`-shaped: the ceiling
/// floors at one row, and then **the floor is pulled down to meet it** rather
/// than the other way around. A terminal too short for the strip *and* a row of
/// list gets a one-row box that the strip fills, which is useless and alive —
/// the fail-safe direction, since the alternative is taking the session down.
///
/// **Do not swap the last two arguments.** All three are `u16`, so the wrong
/// order compiles, and — because this function's job is to degrade rather than
/// panic — it returns a small box instead of complaining. The numbers, because
/// this is only obvious once the two are side by side:
///
/// ```text
/// list_height_reserving(12, 45, 1)  ==  15   // rows, terminal_height, reserved
/// list_height_reserving(12, 1, 45)  ==   3   // the same call, transposed
/// ```
///
/// Note what that costs: the panic-safety test cannot catch it, because a
/// three-row box is exactly what correct degradation looks like at a tiny
/// terminal. Replacing a crash with a floor buys a live session and gives up
/// self-reporting — a crash announces itself and a floor does not — which is
/// the silently-degrading shape this project keeps finding, one level up from
/// the bug the helper was written for. The durable answer is a `Reserved(u16)`
/// newtype, which would make the transposition fail to compile.
fn list_height_reserving(rows: u16, terminal_height: u16, reserved: u16) -> u16 {
    let max = terminal_height.saturating_sub(4).max(1);
    let min = reserved.saturating_add(1).min(max);
    rows.saturating_add(reserved).clamp(min, max) + 2
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

/// Whether the terminal is currently reporting mouse events to us.
///
/// Mouse capture is what makes the scroll wheel scroll the transcript — and it
/// is also what stops a drag from selecting text, because the terminal
/// forwards the drag here instead of drawing a selection. Most terminals let
/// you hold shift to bypass that, which is a rule nobody knows at the moment
/// they need it: the documents picker's own fallback ("the URL stays on screen
/// to be selected by hand as well") was, for exactly this reason, not
/// available at all. So a pane whose whole content is text meant to be copied
/// gives the mouse back for as long as it is up.
static MOUSE_CAPTURED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reconcile the terminal's mouse mode with what is on screen.
///
/// Called once a frame from the drawn state rather than at the points that
/// open and close a pane, on the `/outbox` reload rule: there are several ways
/// out of a pane (a key, a finished child process, a cancelled pick) and a
/// mode that has to be restored by remembering is a mode that eventually is
/// not. Idempotent, so the ordinary frame writes nothing.
fn sync_mouse_capture(want: bool) {
    use std::sync::atomic::Ordering::SeqCst;
    if MOUSE_CAPTURED.swap(want, SeqCst) == want {
        return;
    }
    let _ = if want {
        crossterm::execute!(std::io::stdout(), EnableMouseCapture)
    } else {
        crossterm::execute!(std::io::stdout(), DisableMouseCapture)
    };
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
    MOUSE_CAPTURED.store(true, std::sync::atomic::Ordering::SeqCst);

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

fn leave(terminal: &mut Terminal<impl Backend<Error: Send + Sync + 'static>>) -> Result<()> {
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
    MOUSE_CAPTURED.store(false, std::sync::atomic::Ordering::SeqCst);
    terminal.show_cursor()?;
    println!();
    Ok(())
}

/// Keys for the `/mail` modal.
///
/// Every mutation is `mecha mail …` in a child process — see `tui/mail.rs` for
/// why. Nothing here touches the store directly, so a thing the modal can do
/// is a thing a script or a trigger can do.
/// Keys for the /find modal. Two states, search-tool shaped: typing edits
/// the query, results navigate — and `/` returns to the query, because that
/// is what every search surface teaches the hand.
fn handle_find_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.find else {
        return Ok(());
    };
    // The detail is its own layer: scroll and leave.
    if modal.detail.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                modal.detail = None;
                modal.scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => modal.scroll = modal.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => modal.scroll = modal.scroll.saturating_add(1),
            KeyCode::PageUp => modal.scroll = modal.scroll.saturating_sub(10),
            KeyCode::PageDown => modal.scroll = modal.scroll.saturating_add(10),
            _ => {}
        }
        return Ok(());
    }
    if modal.typing {
        match key.code {
            KeyCode::Esc => app.find = None,
            KeyCode::Backspace => {
                modal.query.pop();
            }
            KeyCode::Enter => {
                let q = modal.query.trim().to_string();
                if q.is_empty() || modal.loading {
                    return Ok(());
                }
                modal.typing = false;
                modal.loading = true;
                modal.status = Some(format!("searching `{q}`…"));
                spawn_kg_search(app, &q);
            }
            _ => {
                if let Some(c) = typed_char(&key) {
                    modal.query.push(c);
                }
            }
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.find = None,
        KeyCode::Char('/') => {
            modal.typing = true;
        }
        KeyCode::Up | KeyCode::Char('k') => modal.move_sel(-1),
        KeyCode::Down | KeyCode::Char('j') => modal.move_sel(1),
        KeyCode::Enter => {
            if modal.loading {
                return Ok(());
            }
            let Some(row) = modal.selected_row() else {
                return Ok(());
            };
            match &row.entity {
                // An entity opens its full record — a fetch, so a watch.
                Some(name) => {
                    let name = name.clone();
                    modal.loading = true;
                    modal.status = Some(format!("fetching {name}…"));
                    spawn_kg_entity(app, &name);
                }
                // A fact or episode opens in place: the pack already
                // carried its text, and a detail from memory costs nothing.
                None => {
                    let title = format!("{} · {}", row.kind, row.when);
                    let lines: Vec<String> = row.full.lines().map(|l| l.to_string()).collect();
                    modal.detail = Some((title, lines));
                    modal.scroll = 0;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_mail_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.mail else {
        return Ok(());
    };

    // A note being typed owns the keyboard, like the front door's.
    if let Some(input) = &mut modal.input {
        match key.code {
            KeyCode::Esc => {
                modal.input = None;
            }
            KeyCode::Enter => {
                let Some(input) = modal.input.take() else {
                    return Ok(());
                };
                let Some(row) = modal.rows.get(modal.selected) else {
                    return Ok(());
                };
                if input.buffer.trim().is_empty() {
                    modal.status = Some("nothing typed — cancelled".into());
                    return Ok(());
                }
                let (thread, account) = (row.thread_id.clone(), row.account.clone());
                if input.verb == "forward" {
                    let to = input.buffer.trim().trim_end_matches(',').to_string();
                    if to.is_empty() {
                        modal.status = Some("no recipient — cancelled".into());
                        return Ok(());
                    }
                    spawn_draft(app, "forward", &thread, &account, Some(&to));
                    return Ok(());
                }
                let result = match input.verb {
                    "needs-info" => self_cli(&[
                        "mail",
                        "needs-info",
                        &thread,
                        "--account",
                        &account,
                        "--missing",
                        input.buffer.trim(),
                    ]),
                    // A correction names a bucket; anything else is a typo and
                    // the CLI says so with the alternatives.
                    _ => self_cli(&[
                        "mail",
                        "correct",
                        &thread,
                        "--account",
                        &account,
                        "--bucket",
                        input.buffer.trim(),
                    ]),
                };
                modal.status = Some(match result {
                    Ok(out) => out.lines().next().unwrap_or("done").to_string(),
                    Err(e) => format!("{e:#}"),
                });
                refresh_mail(app);
            }
            KeyCode::Backspace => input.backspace(),
            KeyCode::Left => {
                input.cursor = input.buffer[..input.cursor]
                    .chars()
                    .next_back()
                    .map(|c| input.cursor - c.len_utf8())
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                input.cursor = input.buffer[input.cursor..]
                    .chars()
                    .next()
                    .map(|c| input.cursor + c.len_utf8())
                    .unwrap_or(input.cursor);
            }
            // Completion only steers when there is something to steer.
            KeyCode::Up if !input.contacts.is_empty() => {
                input.pick = input.pick.saturating_sub(1);
            }
            KeyCode::Down if !input.contacts.is_empty() => {
                let n = input.candidates().len();
                input.pick = (input.pick + 1).min(n.saturating_sub(1));
            }
            KeyCode::Tab => {
                let chosen = input
                    .candidates()
                    .get(input.pick)
                    .map(|c| c.address.clone());
                if let Some(a) = chosen {
                    input.accept(&a);
                }
            }
            _ => {
                if let Some(c) = typed_char(&key) {
                    input.insert(c);
                }
            }
        }
        return Ok(());
    }

    // The key list swallows the next keypress, whatever it is: an overlay you
    // have to guess your way out of is not help.
    if modal.help {
        modal.help = false;
        return Ok(());
    }

    // An open thread takes the scrolling keys and gives everything else to
    // the action map below — reading a thread and then archiving it is one
    // motion, and making the reader a dead end would mean closing it first.
    if let Some(reader) = &mut modal.reading {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                modal.reading = None;
                return Ok(());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                reader.scroll_by(-1);
                return Ok(());
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => {
                reader.scroll_by(1);
                return Ok(());
            }
            KeyCode::PageUp => {
                reader.scroll_by(-15);
                return Ok(());
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                reader.scroll_by(15);
                return Ok(());
            }
            KeyCode::Char('?') => {
                modal.help = true;
                return Ok(());
            }
            // Anything else falls through to the action map, and acting
            // closes the reader: the row it was opened on has just moved.
            KeyCode::Char(c) if mail::action_for(c).is_some() => modal.reading = None,
            _ => return Ok(()),
        }
    }

    // A pending confirmation owns it next. EOF and anything that is not `y`
    // mean no, which is the outbox's rule and the doctor's.
    if modal.confirm.is_some() {
        let yes = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
        modal.confirm = None;
        if !yes {
            modal.status = Some("cancelled".into());
            return Ok(());
        }
        let Some(row) = modal.rows.get(modal.selected) else {
            return Ok(());
        };
        let (thread, account, handle) = (
            row.thread_id.clone(),
            row.account.clone(),
            row.handle.clone(),
        );
        spawn_mail_action(app, "spam", &thread, &account, &handle);
        return Ok(());
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mail = None;
        }
        KeyCode::Up | KeyCode::Char('k') => modal.move_by(-1),
        KeyCode::Down | KeyCode::Char('j') => modal.move_by(1),
        KeyCode::Char('?') => modal.help = true,
        // Enter opens the thread. It goes through the CLI like every other
        // action rather than being a second renderer of the same record — and
        // off the event loop, because it is a whole mail fetch.
        KeyCode::Enter => {
            if modal.loading.is_some() {
                return Ok(());
            }
            let Some(row) = modal.rows.get(modal.selected) else {
                return Ok(());
            };
            let (thread, account, handle) = (
                row.thread_id.clone(),
                row.account.clone(),
                row.handle.clone(),
            );
            modal.loading = Some(handle.clone());
            spawn_mail_read(app, &thread, &account, &handle);
        }
        KeyCode::Char(_) => {
            // Through `typed_char`, or Ctrl-A archives the thread under the
            // cursor and Ctrl-R starts a run.
            let Some(action) = typed_char(&key).and_then(mail::action_for) else {
                return Ok(());
            };
            let Some(row) = modal.rows.get(modal.selected) else {
                return Ok(());
            };
            let (thread, account) = (row.thread_id.clone(), row.account.clone());
            match action {
                mail::Action::Close => app.mail = None,
                mail::Action::Confirm(verb) => {
                    // Spam is the one triage action with an effect outside the
                    // user's own mailbox: it trains the provider's filter.
                    modal.confirm = Some(format!("mark as {verb}? trains the filter — y/N"));
                }
                mail::Action::Prompt(verb, label) => {
                    modal.input = Some(mail::MailInput::text(label, verb));
                }
                mail::Action::Now(verb) => {
                    // Off the event loop: `archive` and `task` start an MCP
                    // server and make a network call, which inline froze the
                    // interface for a second or two per keystroke — this
                    // module's own rule, finally applied to its own verbs.
                    // `Watch::MailAction` collects the answer and refreshes.
                    let handle = row.handle.clone();
                    spawn_mail_action(app, verb, &thread, &account, &handle);
                }
                mail::Action::Recipients(verb) => {
                    // Candidates from the store, loaded once when the input
                    // opens — filtering happens locally as they type, so no
                    // keystroke costs a query.
                    let mine = mecha_core::mail_triage::TriageStore::open_existing_default()
                        .and_then(|s| s.list().ok())
                        .map(|rows| mecha_core::mail_triage::contacts(&rows, &[]))
                        .unwrap_or_default();
                    modal.input = Some(mail::MailInput::recipients("forward to", verb, mine));
                }
                mail::Action::Detached(verb) => {
                    spawn_draft(app, verb, &thread, &account, None);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Re-read the store after a mutation, keeping the cursor where it was.
///
/// Reloading rather than patching the row in memory: the child process is the
/// only writer, so anything this modal believed about the record is now a
/// guess. Cheap — the store is small and local.
fn refresh_mail(app: &mut App) {
    let Some(current) = &mut app.mail else {
        return;
    };
    let Ok(rows) = mail::load() else {
        return;
    };
    // The rows are replaced; everything the *person* was doing — an open
    // thread, a read in flight, the cursor — is theirs and survives. Rebuilding
    // the modal here would close a thread they were halfway through reading
    // because an unrelated archive succeeded.
    current.rows = rows;
    current.selected = current.selected.min(current.rows.len().saturating_sub(1));
}

/// Fetch one thread for the reader, off the event loop.
///
/// A thread on purpose rather than a detached child: what is wanted back is
/// the text, and `self_cli` already knows how to get it. The watch collects
/// it a tick later and installs it into the modal.
/// Load similarity groups on a thread — one class's, or (`class: None`) the
/// cross-class global layer over the whole queue — at an optional explicit
/// threshold, collected through `Watch::QueuesGroups`.
fn spawn_group_load(app: &mut App, class: Option<(String, String)>, threshold: Option<f64>) {
    let mut args: Vec<String> = vec!["review".into(), "groups".into()];
    match &class {
        Some((pb, pred)) => {
            args.push("--proposer".into());
            args.push(pb.clone());
            args.push("--predicate".into());
            args.push(pred.clone());
        }
        None => args.push("--all".into()),
    }
    args.push("--json".into());
    if let Some(t) = threshold {
        args.push("--threshold".into());
        args.push(format!("{t:.2}"));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::QueuesGroups {
        rx,
        class,
        since: std::time::Instant::now(),
    });
}

/// Capture one note through `mecha kg note`, collected as a notice.
fn spawn_kg_note(app: &mut App, text: &str) {
    let args: Vec<String> = vec!["kg".into(), "note".into(), text.into()];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::KgNote {
        rx,
        since: std::time::Instant::now(),
    });
}

/// Run one graph search for the /find modal, off the event loop.
fn spawn_kg_search(app: &mut App, query: &str) {
    let args: Vec<String> = vec!["kg".into(), "search".into(), query.into(), "--json".into()];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::KgSearch {
        rx,
        query: query.to_string(),
        since: std::time::Instant::now(),
    });
}

/// Fetch one entity's record for the /find modal's detail.
fn spawn_kg_entity(app: &mut App, name: &str) {
    let args: Vec<String> = vec!["kg".into(), "entity".into(), name.into(), "--json".into()];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::KgEntity {
        rx,
        name: name.to_string(),
        since: std::time::Instant::now(),
    });
}

/// Run one triage verb (`archive`, `spam`, `task`) on its own thread and
/// collect the outcome through `Watch::MailAction`. Each of these starts an
/// MCP server and reaches the provider, so none may run on the event loop.
fn spawn_mail_action(app: &mut App, verb: &str, thread: &str, account: &str, handle: &str) {
    let args: Vec<String> = vec![
        "mail".into(),
        verb.into(),
        thread.into(),
        "--account".into(),
        account.into(),
    ];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    if let Some(modal) = &mut app.mail {
        modal.status = Some(format!("{verb} {handle}…"));
    }
    app.watches.push(Watch::MailAction {
        rx,
        verb: verb.to_string(),
        handle: handle.to_string(),
        since: std::time::Instant::now(),
    });
}

fn spawn_mail_read(app: &mut App, thread: &str, account: &str, handle: &str) {
    let args: Vec<String> = vec![
        "mail".into(),
        "show".into(),
        thread.into(),
        "--account".into(),
        account.into(),
    ];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::MailRead {
        rx,
        handle: handle.to_string(),
        since: std::time::Instant::now(),
    });
}

/// Follow a task's `captured_from` pointer on its own thread.
///
/// The same exception [`Watch::MailRead`] takes, and for the same reason:
/// following a mail pointer starts an MCP server, may refresh an OAuth token
/// and makes a network call, so doing it on the event loop freezes the
/// interface at the exact moment somebody is waiting to read something.
///
/// Through `mecha tasks source` rather than reaching for the thread directly:
/// there is one reader per kind and it belongs to the command line, so the
/// modal cannot drift from what `mecha tasks source` prints — and a kind added
/// later works here with no change at all.
fn spawn_task_source(app: &mut App, task: &str, handle: &str) {
    let args: Vec<String> = vec!["tasks".into(), "source".into(), task.into()];
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = tx.send(self_cli(&borrowed));
    });
    app.watches.push(Watch::TaskSource {
        rx,
        handle: handle.to_string(),
        since: std::time::Instant::now(),
    });
}

/// `mecha-docs`, resolved the way `spawn_remedy` resolves a doctor remedy's
/// program: a sibling of this binary first — an install puts them together,
/// and so does a `cargo build` target directory — then whatever is on `PATH`.
///
/// A *binary* rather than a library dependency, deliberately. `mecha-cli`
/// takes no `mecha-mail` dependency anywhere, which is what lets the mail and
/// documents surfaces be installed, upgraded and confined apart; buying four
/// MIME strings and a URL shape with a crate edge would be the wrong trade,
/// and `tui/docs.rs` carries its own copy of both for that reason.
fn docs_bin() -> std::path::PathBuf {
    // current_exe, not `exe::self_exe()`: this wants the *directory* beside
    // the binary, and `/proc/self/exe` has no useful parent. The "(deleted)"
    // suffix an install adds lives in the file name, so `.parent()` still
    // answers correctly on a stale session.
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("mecha-docs")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| "mecha-docs".into())
}

/// Run a `mecha-docs` verb on its own thread and collect the answer through a
/// watch. Every call here reaches the network, so none of them may happen on
/// the event loop.
fn spawn_docs(app: &mut App, job: DocsJob, args: &[&str]) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = std::process::Command::new(docs_bin())
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output();
        let _ = tx.send(match out {
            Ok(out) if out.status.success() => {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            // `mecha-docs` writes its prose to stderr and its data to stdout,
            // so a failure's first stderr line is the message — and the
            // fallback matters, because a missing binary fails with nothing
            // on stderr at all.
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                Err(anyhow::anyhow!(
                    "{}",
                    err.trim()
                        .lines()
                        .next_back()
                        .unwrap_or("mecha-docs failed")
                ))
            }
            Err(e) => Err(anyhow::anyhow!(
                "cannot run {}: {e} — is mecha-docs installed?",
                docs_bin().display()
            )),
        });
    });
    if let Some(m) = &mut app.documents {
        m.loading = true;
    }
    app.watches.push(Watch::Docs {
        rx,
        job,
        since: std::time::Instant::now(),
    });
}

/// Accounts with a documents grant. The directory listing *is* the list —
/// there is no registry file, for the reason `mecha-docs` gives: a registry
/// that can disagree with the filesystem is a second source of truth.
fn docs_accounts() -> Vec<String> {
    let Some(home) = dirs_home() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(home.join(".mecha").join("docs")) else {
        return out;
    };
    for entry in entries.flatten() {
        if entry.path().join("oauth.json").is_file() {
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Write an escape sequence straight to the terminal, around ratatui.
///
/// Only safe for sequences that paint nothing and move nothing — OSC 52 is
/// exactly that, which is why this takes no general-purpose name. Anything
/// that touched a cell would desynchronise ratatui's diff and stay on screen
/// forever, which is the bug `logs.rs` exists to stop happening by accident.
fn write_escape(seq: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

/// Put a paste into the documents picker's field, and say whether it went
/// there.
///
/// A modal that owns the keyboard owns a paste too — typing is typing — and
/// this field is the one whose whole purpose is receiving one: the pane says
/// "paste it here". Without this the address went into the message box
/// *behind* the modal, where nothing showed it until the pick was cancelled,
/// so the only way to fill a two-hundred-character redirect was to type it.
/// A function rather than an arm of the event match so it can be tested
/// without a live agent, which is what `Live` would otherwise require.
fn paste_into_pick(app: &mut App, text: &str) -> bool {
    let Some(pick) = app.documents.as_mut().and_then(|d| d.pick.as_mut()) else {
        return false;
    };
    // Mid-exchange the field is not accepting input; swallowing the paste is
    // still right, because the alternative is typing it into the prompt.
    if !pick.working {
        // Whitespace is stripped rather than inserted: an address copied out
        // of a wrapped terminal or a wrapped address bar arrives with newlines
        // in it, and a URL never legitimately contains a space — an encoded
        // one is `%20`.
        let clean: String = text.split_whitespace().collect();
        pick.buffer.insert_str(pick.cursor, &clean);
        pick.cursor += clean.len();
        // A paste means the browser leg is done, so the link view has served
        // its purpose and the field is what matters.
        pick.bare = false;
    }
    true
}

/// Hand a URL to whatever opens links on *this* machine.
///
/// The shortest path when the browser and the TUI are on the same box, and no
/// path at all when they are not — which is the case the picker was designed
/// around, so this is an extra door rather than the door. Spawned detached
/// with its output discarded: an opener that writes to a terminal in raw mode
/// draws over the frame, and one that lingers must not hold up the loop.
fn open_locally(url: &str) -> Result<()> {
    let opener = std::env::var("BROWSER")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "open".into()
            } else {
                "xdg-open".into()
            }
        });
    std::process::Command::new(&opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("cannot run {opener}"))?;
    Ok(())
}

// ─── /entity ─────────────────────────────────────────────────────────────────

/// Where `mecha-graph` lives, on `commands::review`'s rule: `$MECHA_GRAPH_BIN`
/// first, then the name on `PATH`, and deliberately never `mecha.toml` — a
/// project file arrives with a cloned repository, and a project that could
/// name a binary mecha runs as a child process has been handed arbitrary
/// execution.
fn graph_bin() -> String {
    std::env::var("MECHA_GRAPH_BIN").unwrap_or_else(|_| "mecha-graph".into())
}

/// Run `mecha-graph <args>` and hand back stdout, or the reason it failed.
///
/// Synchronous, unlike `/docs` and `/outbox`: an entity lookup against the
/// graph measures 7ms, so the detached-job machinery would be ceremony
/// around something faster than a keypress. A missing binary is reported by
/// name with the variable that fixes it — "No such file or directory" from a
/// child nobody mentioned is the least actionable error there is.
fn graph_cli(args: &[&str]) -> std::result::Result<String, String> {
    let bin = graph_bin();
    let out = std::process::Command::new(&bin).args(args).output();
    let out = match out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "`{bin}` not found — install mecha-graph, or set MECHA_GRAPH_BIN to its path"
            ));
        }
        Err(e) => return Err(format!("running {bin}: {e}")),
    };
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    // The reason may be on either stream, and `mecha-graph`'s refusals — the
    // collision messages this modal exists to surface — are multi-line. Keep
    // the first line, which is the sentence; the rest is the candidate list
    // and would not fit a status row anyway.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let reason = stderr
        .trim()
        .lines()
        .next()
        .filter(|l| !l.trim().is_empty())
        .or_else(|| stdout.trim().lines().next())
        .unwrap_or("failed")
        .trim_start_matches("error: ")
        .to_string();
    Err(reason)
}

/// Re-run the current query so the list reflects what just changed. A rename
/// reorders nothing here, but it does change the name being displayed, and a
/// row still showing the old one after a successful rename reads as a
/// failure.
fn entity_lookup(app: &mut App) {
    let Some(modal) = &mut app.entities else {
        return;
    };
    let q = modal.query.trim().to_string();
    if q.is_empty() {
        return;
    }
    match graph_cli(&["entity", &q, "--json"]) {
        Ok(json) => modal.install(&json),
        Err(e) => {
            modal.rows.clear();
            modal.fresh = false;
            modal.status = Some(e);
        }
    }
}

/// Keys for the `/entity` modal.
fn handle_entity_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.entities else {
        return Ok(());
    };

    // An edit in flight takes every key first — including `q`, `r` and `a`,
    // which are letters in a name before they are commands.
    if let Some((kind, buf)) = &mut modal.edit {
        let kind = *kind;
        match key.code {
            KeyCode::Esc => modal.edit = None,
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
            KeyCode::Enter => {
                let text = buf.trim().to_string();
                modal.edit = None;
                if text.is_empty() {
                    return Ok(());
                }
                let target = modal.selected_row().map(|r| r.id.clone());
                let args: Vec<String> = match kind {
                    entity::EditKind::NewPerson => vec!["new-person".into(), text.clone()],
                    _ => {
                        let Some(id) = target else {
                            modal.status = Some("no node selected".into());
                            return Ok(());
                        };
                        vec![kind.verb().into(), id, text.clone()]
                    }
                };
                let argv: Vec<&str> = args.iter().map(String::as_str).collect();
                match graph_cli(&argv) {
                    Ok(out) => {
                        modal.status =
                            Some(out.lines().next().unwrap_or("done").trim().to_string());
                        // Creating a person is also how you go and look at
                        // them: the query becomes their name so the refresh
                        // lands on the node just made.
                        if kind == entity::EditKind::NewPerson {
                            modal.query = text;
                        }
                        entity_lookup(app);
                    }
                    // A refusal keeps the page open — every collision here is
                    // a question, and answering it needs what is on screen.
                    Err(e) => modal.status = Some(e),
                }
            }
            _ => {}
        }
        return Ok(());
    }

    // A pending merge takes every key: it is the one irreversible action on
    // this modal, so nothing else may happen while it is on screen.
    if let Some((keep_id, keep, dup_id, dup)) = modal.merge_confirm.clone() {
        match key.code {
            KeyCode::Char('y') => {
                modal.merge_confirm = None;
                modal.merge_keep = None;
                match graph_cli(&["merge", &keep_id, &dup_id]) {
                    Ok(_) => {
                        modal.status = Some(format!("merged {dup:?} into {keep:?}"));
                        entity_lookup(app);
                    }
                    Err(e) => modal.status = Some(format!("merge failed: {e}")),
                }
            }
            // Anything that is not an explicit yes is a no. EOF-is-no, one
            // layer up — a confirmation that could be dismissed into
            // proceeding is not one.
            _ => {
                modal.merge_confirm = None;
                modal.merge_keep = None;
                modal.status = Some("merge cancelled".into());
            }
        }
        return Ok(());
    }

    if modal.help {
        modal.help = false;
        return Ok(());
    }

    match key.code {
        KeyCode::Esc if modal.merge_keep.is_some() => {
            modal.merge_keep = None;
            modal.status = Some("merge cancelled".into());
        }
        // Esc peels one layer at a time, as it does in /queues and /outbox:
        // a search clears before the modal closes. Without this the only way
        // to start a second lookup was to leave and come back, which is what
        // sent someone reaching for ctrl-n expecting "new search" and
        // creating a person named after their query instead.
        KeyCode::Esc if !modal.query.is_empty() || !modal.rows.is_empty() => {
            modal.query.clear();
            modal.rows.clear();
            modal.selected = 0;
            modal.fresh = true;
            modal.status = None;
        }
        KeyCode::Esc => app.entities = None,
        KeyCode::Char('?') => modal.help = true,
        KeyCode::Up => modal.move_sel(-1),
        KeyCode::Down => modal.move_sel(1),
        KeyCode::Enter => entity_lookup(app),
        KeyCode::Backspace => {
            modal.query.pop();
        }
        // Ctrl-N, not `n`: the query box is live, so a bare letter is a
        // letter. Prefilled with what was typed, because the moment you want
        // this is the moment a lookup came back empty.
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let seed = modal.query.trim().to_string();
            modal.edit = Some((entity::EditKind::NewPerson, seed));
        }
        // r and a act on the selected row, so they are only commands once
        // there is a row to act on — otherwise they are letters being typed
        // into the search box.
        KeyCode::Char('r') if modal.selected_row().is_some() => {
            modal.edit = Some((entity::EditKind::Rename, String::new()));
        }
        KeyCode::Char('a') if modal.selected_row().is_some() => {
            modal.edit = Some((entity::EditKind::Alias, String::new()));
        }
        // Two keystrokes on two rows rather than a form: what is being
        // merged is *these two nodes on screen*, and a field asking for an
        // id would be answered by copying one off the display.
        KeyCode::Char('m') if modal.selected_row().is_some() => {
            let row = modal.selected_row().expect("checked");
            let (id, name) = (row.id.clone(), row.name.clone());
            match modal.merge_keep.clone() {
                None => {
                    modal.merge_keep = Some(id);
                    modal.status = Some(format!("keeping {name:?} — now pick the duplicate"));
                }
                Some(keep_id) if keep_id == id => {
                    modal.merge_keep = None;
                    modal.status = Some("unmarked".into());
                }
                Some(keep_id) => {
                    let keep_name = modal
                        .rows
                        .iter()
                        .find(|r| r.id == keep_id)
                        .map(|r| r.name.clone())
                        .unwrap_or_else(|| keep_id.clone());
                    modal.merge_confirm = Some((keep_id, keep_name, id, name));
                }
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            modal.query.push(c);
        }
        _ => {}
    }
    Ok(())
}

/// Keys for the `/docs` modal.
///
/// Every action is a `mecha-docs …` child process, on the `/triggers` rule:
/// nothing the modal can do is missing from the command line.
fn handle_docs_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(modal) = &mut app.documents else {
        return Ok(());
    };
    if modal.help {
        modal.help = false;
        return Ok(());
    }

    // The picking pane owns the keyboard while it is up: it has a text field,
    // so a bare `q` is a character and not a close.
    if let Some(pick) = &mut modal.pick {
        match key.code {
            KeyCode::Esc => {
                modal.pick = None;
                modal.status = Some("pick cancelled — the grant is unchanged".into());
            }
            KeyCode::Enter if !pick.working => {
                let pasted = pick.buffer.trim().to_string();
                if pasted.is_empty() {
                    modal.status = Some("paste the address the browser landed on first".into());
                    return Ok(());
                }
                pick.working = true;
                let account = modal.account.clone();
                spawn_docs(
                    app,
                    DocsJob::PickDone,
                    &[
                        "--account",
                        &account,
                        "pick",
                        "--redirect",
                        &pasted,
                        "--json",
                    ],
                );
            }
            KeyCode::Backspace => {
                if let Some(at) = prev_boundary(&pick.buffer, pick.cursor) {
                    pick.buffer.remove(at);
                    pick.cursor = at;
                }
            }
            KeyCode::Left => pick.cursor = prev_boundary(&pick.buffer, pick.cursor).unwrap_or(0),
            KeyCode::Right => pick.cursor = next_boundary(&pick.buffer, pick.cursor),
            KeyCode::Home => pick.cursor = 0,
            KeyCode::End => pick.cursor = pick.buffer.len(),
            _ if typed_char(&key).is_some() => {
                let c = typed_char(&key).unwrap_or_default();
                // `y` copies only on an empty field. Once there is an address
                // being typed the same key is a character — an authorization
                // redirect is full of them, and a key that means two things
                // depending on nothing visible is a key that eats input.
                if c == 'y' && pick.buffer.is_empty() {
                    write_escape(&docs::clipboard_escape(&pick.url));
                    modal.status = Some(
                        "link sent to your clipboard — if your terminal allows OSC 52                          (tmux needs set-clipboard on)"
                            .into(),
                    );
                    return Ok(());
                }
                // `s` and `o` follow the same empty-field rule as `y`, and for
                // the same reason: once an address is being typed the letters
                // are letters.
                if c == 's' && pick.buffer.is_empty() {
                    pick.bare = !pick.bare;
                    modal.status = None;
                    return Ok(());
                }
                if c == 'o' && pick.buffer.is_empty() {
                    let url = pick.url.clone();
                    modal.status = Some(match open_locally(&url) {
                        // Hedged, like the clipboard write: an opener exits 0
                        // having handed the URL to a desktop that may not be
                        // there. Over SSH the browser is on the other machine
                        // and this cannot reach it — which is why the link
                        // stays on screen either way.
                        Ok(()) => "asked this machine to open it — nothing happens here if the \
                                   browser is on the machine you are sitting at"
                            .into(),
                        Err(e) => format!("cannot open it here: {e} — use s or y instead"),
                    });
                    return Ok(());
                }
                pick.buffer.insert(pick.cursor, c);
                pick.cursor += c.len_utf8();
            }
            _ => {}
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.documents = None,
        KeyCode::Up | KeyCode::Char('k') => modal.move_by(-1),
        KeyCode::Down | KeyCode::Char('j') => modal.move_by(1),
        KeyCode::Char('?') => modal.help = true,
        KeyCode::Char('r') => {
            let account = modal.account.clone();
            modal.status = None;
            spawn_docs(
                app,
                DocsJob::List,
                &["--account", &account, "list", "--json"],
            );
        }
        KeyCode::Char('a') => {
            if let Some(next) = modal.next_account() {
                modal.account = next.clone();
                modal.rows.clear();
                modal.selected = 0;
                modal.status = None;
                spawn_docs(app, DocsJob::List, &["--account", &next, "list", "--json"]);
            }
        }
        KeyCode::Char('p') => {
            let account = modal.account.clone();
            modal.status = Some("asking Google for a chooser link…".into());
            spawn_docs(
                app,
                DocsJob::PickUrl,
                &["--account", &account, "pick", "--url", "--json"],
            );
        }
        KeyCode::Char('y') => {
            let Some(row) = modal.current() else {
                return Ok(());
            };
            let url = row.url();
            write_escape(&docs::clipboard_escape(&url));
            modal.status = Some(format!(
                "{url} — sent to your clipboard if your terminal allows OSC 52"
            ));
        }
        // Into the message box, not onto the wire: composing someone's prompt
        // for them is not this modal's job, and the id is the only part they
        // could not have typed.
        KeyCode::Enter => {
            let Some(row) = modal.current() else {
                return Ok(());
            };
            let reference = row.reference();
            app.documents = None;
            if !app.input.is_empty() && !app.input.ends_with(' ') {
                app.input.push(' ');
            }
            app.input.push_str(&reference);
            app.cursor = app.input.len();
        }
        _ => {}
    }
    Ok(())
}

/// Fold a finished `mecha-docs` call back into the modal.
fn install_docs_answer(app: &mut App, job: DocsJob, answer: Result<String>) {
    let Some(modal) = &mut app.documents else {
        // The modal was closed while the call was in flight. A failure is
        // still worth saying — it is the same failure the next open will hit.
        if let Err(e) = answer {
            app.transcript.push(Entry::Error(format!("docs: {e:#}")));
        }
        return;
    };
    modal.loading = false;
    let text = match answer {
        Ok(text) => text,
        Err(e) => {
            if let Some(pick) = &mut modal.pick {
                pick.working = false;
            }
            modal.status = Some(format!("{e:#}"));
            return;
        }
    };

    match job {
        DocsJob::List => modal.install(&text),
        DocsJob::PickUrl => {
            let url = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["url"].as_str().map(str::to_string));
            match url {
                Some(url) => {
                    modal.status = None;
                    modal.pick = Some(docs::Pick {
                        url,
                        buffer: String::new(),
                        cursor: 0,
                        working: false,
                        bare: false,
                    });
                }
                None => modal.status = Some("mecha-docs did not return a link".into()),
            }
        }
        DocsJob::PickDone => {
            let picked: Vec<String> = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["picked"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(|f| {
                    format!(
                        "{} {}",
                        f["kind"].as_str().unwrap_or("file"),
                        f["name"].as_str().unwrap_or("")
                    )
                })
                .collect();
            modal.pick = None;
            // An empty pick is a real answer — someone opened the chooser and
            // chose nothing — and reads differently from a failure.
            modal.status = Some(if picked.is_empty() {
                "nothing was picked; the grant was renewed and scope is unchanged".into()
            } else {
                format!("in scope now: {}", picked.join(", "))
            });
            let account = modal.account.clone();
            spawn_docs(
                app,
                DocsJob::List,
                &["--account", &account, "list", "--json"],
            );
        }
    }
}

/// Start a drafting run and let it go.
///
/// **Detached, because it is a whole agent run.** It builds a tool surface,
/// reads a thread and writes prose — minutes, not milliseconds — and doing
/// that on the event loop freezes the interface. The result lands in
/// `/outbox`, which is where it is reviewed; this only reports that the run
/// started.
fn spawn_draft(app: &mut App, verb: &str, thread: &str, account: &str, to: Option<&str>) {
    let exe = crate::exe::self_exe();
    let mut args: Vec<String> = vec![
        "mail".into(),
        verb.into(),
        thread.into(),
        "--account".into(),
        account.into(),
    ];
    if let Some(to) = to {
        args.push("--to".into());
        args.push(to.into());
    }
    let spawned = std::process::Command::new(exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Some(m) = &mut app.mail {
        m.input = None;
        m.status = Some(match spawned {
            Ok(_) => format!("{verb} drafting in the background — watch /outbox"),
            Err(e) => format!("could not start {verb}: {e}"),
        });
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mecha-drop-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    fn png_at(p: &std::path::Path) {
        let img = image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([1, 2, 3]));
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(p, image::ImageFormat::Png)
            .unwrap();
    }

    /// The plain case, and the three escapings a terminal might apply to it.
    /// The one that matters is the space: every screenshot macOS names has
    /// one, so a splitter that only handles bare paths handles almost no real
    /// drops.
    #[test]
    fn a_dropped_path_is_recognised_however_the_terminal_escaped_it() {
        let p = tmp("a shot.png");
        png_at(&p);
        let d = p.display().to_string();
        for paste in [
            d.to_string(),
            format!("'{d}'"),
            format!("\"{d}\""),
            d.replace(' ', "\\ "),
        ] {
            let got = drop_paths(&paste).unwrap_or_else(|| panic!("not parsed: {paste}"));
            assert_eq!(got, vec![p.clone()], "from {paste}");
        }
        std::fs::remove_file(&p).ok();
    }

    /// **The safety property.** A paste is not always a drop, and prose that
    /// merely contains a real path must never pull bytes off this disk into a
    /// request. Verified to fail on the looser rule — "attach any existing
    /// image path found in the paste" — which would match here.
    #[test]
    fn pasted_prose_that_merely_mentions_a_real_image_is_not_a_drop() {
        let p = tmp("mentioned.png");
        png_at(&p);
        let prose = format!("the bug is visible in {} near the top", p.display());
        assert!(
            drop_paths(&prose).is_none(),
            "a sentence is not a drop, even when a path in it resolves"
        );
        std::fs::remove_file(&p).ok();
    }

    /// A path that does not exist is somebody talking about a file.
    #[test]
    fn a_path_that_does_not_exist_is_not_a_drop() {
        assert!(drop_paths("/no/such/shot.png").is_none());
        assert!(drop_paths("look at shot.png").is_none());
    }

    /// Several files at once is one drop, not several.
    #[test]
    fn two_files_dropped_together_are_both_taken() {
        let (a, b) = (tmp("one.png"), tmp("two.png"));
        png_at(&a);
        png_at(&b);
        let paste = format!("{} {}", a.display(), b.display());
        assert_eq!(drop_paths(&paste).unwrap().len(), 2);
        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }

    /// The chip is the handle, so deleting it has to actually detach — a
    /// dropped image cannot be reached with backspace, so this is the only
    /// undo there is.
    #[test]
    fn deleting_the_chip_detaches_the_image_and_keeping_it_does_not() {
        let block = MsgBlock::image("image/png", b"xx", Some("shot.png".into()));
        let mut app = crate::tui::tests::test_app();

        app.dropped = vec![("[image: shot.png]".into(), block.clone())];
        assert_eq!(
            take_dropped(&mut app, "what is this? [image: shot.png]").len(),
            1,
            "chip still in the text: the image is sent"
        );

        app.dropped = vec![("[image: shot.png]".into(), block)];
        assert!(
            take_dropped(&mut app, "never mind").is_empty(),
            "chip deleted: the image is not sent"
        );
        assert!(
            app.dropped.is_empty(),
            "and it does not linger to surprise the next turn"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::input_layout;
    use super::*;
    use ratatui::backend::TestBackend;

    use super::Picker;

    fn picking_app() -> App {
        let mut app = test_app();
        let mut modal = docs::DocsModal::new("personal".into(), vec!["personal".into()]);
        modal.pick = Some(docs::Pick {
            url: "https://accounts.google.com/o/oauth2/v2/auth?client_id=949".into(),
            buffer: String::new(),
            cursor: 0,
            working: false,
            bare: false,
        });
        app.documents = Some(modal);
        app
    }

    // ─── /entity ─────────────────────────────────────────────────────────

    fn entity_app() -> App {
        let mut app = test_app();
        let mut modal = entity::EntityModal::new();
        modal.install(
            r#"[{"id":"person-53667537","name":"Dana R. Whitfield","node_type":"person",
                 "aliases":["dana whitfield"],"interactions":1035,
                 "facts":[{"statement":"Dana co-authored the Trust replication."}]}]"#,
        );
        app.entities = Some(modal);
        app
    }

    fn ent_press(app: &mut App, code: KeyCode) {
        handle_entity_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    fn ent_press_ctrl(app: &mut App, code: KeyCode) {
        handle_entity_key(app, KeyEvent::new(code, KeyModifiers::CONTROL)).unwrap();
    }

    /// The lookup box is live, so a letter is a letter — except where it
    /// names an action on a row that exists.
    #[test]
    fn typing_reaches_the_query_and_r_opens_a_rename() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('r'));
        let modal = app.entities.as_ref().unwrap();
        assert_eq!(
            modal.edit.as_ref().map(|(k, _)| *k),
            Some(entity::EditKind::Rename),
            "r on a selected row starts a rename"
        );
        assert!(modal.query.is_empty(), "r must not also land in the query");
    }

    /// With no row selected, the same key is just a letter — otherwise the
    /// name "Rachel" would be untypeable.
    #[test]
    fn action_letters_are_letters_when_there_is_no_row() {
        let mut app = test_app();
        app.entities = Some(entity::EntityModal::new());
        for c in ['r', 'a', 'n'] {
            ent_press(&mut app, KeyCode::Char(c));
        }
        let modal = app.entities.as_ref().unwrap();
        assert_eq!(modal.query, "ran");
        assert!(modal.edit.is_none());
    }

    /// An edit in flight takes every key, including the ones that are
    /// commands outside it. "Sara" must be typeable into a rename box.
    #[test]
    fn an_edit_in_flight_swallows_the_action_letters() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('r'));
        for c in ['S', 'a', 'r', 'a'] {
            ent_press(&mut app, KeyCode::Char(c));
        }
        let modal = app.entities.as_ref().unwrap();
        assert_eq!(modal.edit.as_ref().unwrap().1, "Sara");
        assert!(modal.query.is_empty());
    }

    /// Esc backs out one layer at a time: the edit first, the modal second.
    /// Collapsing both would throw away a half-typed name on the keystroke
    /// people use to fix a typo.
    #[test]
    fn esc_closes_the_edit_before_it_closes_the_modal() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('a'));
        assert!(app.entities.as_ref().unwrap().edit.is_some());
        ent_press(&mut app, KeyCode::Esc);
        assert!(
            app.entities.is_some(),
            "the modal survives cancelling an edit"
        );
        assert!(app.entities.as_ref().unwrap().edit.is_none());
        // Then the search clears, and only then does the modal close —
        // three layers, three presses. This assertion used to expect two,
        // and updating it IS the behaviour change: there was previously no
        // way to start a second lookup without leaving.
        ent_press(&mut app, KeyCode::Esc);
        assert!(
            app.entities.is_some(),
            "the second esc clears the search, it does not close"
        );
        assert!(app.entities.as_ref().unwrap().rows.is_empty());
        ent_press(&mut app, KeyCode::Esc);
        assert!(app.entities.is_none(), "the third esc closes it");
    }

    /// Esc peels one layer at a time. Without this there was no way to
    /// start a second lookup without leaving the modal — which is what sent
    /// someone reaching for ctrl-n expecting "new search" and creating a
    /// person named after their query.
    #[test]
    fn esc_clears_the_search_before_it_closes_the_modal() {
        let mut app = entity_app();
        assert!(!app.entities.as_ref().unwrap().rows.is_empty());

        ent_press(&mut app, KeyCode::Esc);
        let m = app
            .entities
            .as_ref()
            .expect("still open after the first esc");
        assert!(m.rows.is_empty(), "the results should have cleared");
        assert!(m.query.is_empty());
        assert!(
            m.fresh,
            "a cleared modal has not searched, it is not a no-match"
        );

        ent_press(&mut app, KeyCode::Esc);
        assert!(app.entities.is_none(), "the second esc closes it");
    }

    /// And the layers stack in the right order: an edit cancels before the
    /// search clears, so backing out of a typo does not also lose the
    /// results it was going to act on.
    #[test]
    fn esc_cancels_an_edit_before_it_clears_the_search() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('r'));
        assert!(app.entities.as_ref().unwrap().edit.is_some());

        ent_press(&mut app, KeyCode::Esc);
        let m = app.entities.as_ref().unwrap();
        assert!(m.edit.is_none(), "the edit cancelled");
        assert!(!m.rows.is_empty(), "but the search survived");
    }

    /// A key advertised in the footer must DO something. This one was a
    /// no-op for three commits while the footer said "Enter read it" and
    /// the compiler warned `field 'show' is never read` — the bug reporting
    /// itself in two places at once, past a reviewer counting warnings
    /// instead of reading them.
    ///
    /// The assertion is deliberately weak about the outcome and strong
    /// about the attempt: `show` shells out to a real binary, which will
    /// fail in a test with no store behind it. Either a detail arrives or a
    /// status explains why — what must never happen is nothing at all.
    #[test]
    fn enter_at_the_review_level_is_not_a_no_op() {
        let mut app = test_app();
        let mut m = queues::QueuesModal::new(vec![]);
        m.level = queues::Level::Review;
        m.review_source = queues::QueueRow {
            name: "rule proposals".into(),
            depth: Some(1),
            detail: String::new(),
            opens: String::new(),
            oldest: None,
        }
        .review_source();
        m.review = queues::review_from_json(
            r#"[{"id":"nonexistent-id","kind":"behavior","title":"5 rule(s)","detail":"pending"}]"#,
        )
        .unwrap();
        app.queues = Some(m);

        handle_queues_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap();

        let m = app.queues.as_ref().expect("the modal stays open");
        assert!(
            m.review_detail.is_some() || m.status.is_some(),
            "Enter did nothing at all — it is advertised as 'read it'"
        );
    }

    /// Merging is two keystrokes on two rows, and the first one only marks.
    #[test]
    fn m_marks_a_survivor_before_it_merges_anything() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('m'));
        let m = app.entities.as_ref().unwrap();
        assert_eq!(m.merge_keep.as_deref(), Some("person-53667537"));
        assert!(m.merge_confirm.is_none(), "one row is not a merge");
    }

    /// Pressing m on the marked row again unmarks it — the same key backs
    /// out of its own first step.
    #[test]
    fn m_on_the_marked_row_unmarks_it() {
        let mut app = entity_app();
        ent_press(&mut app, KeyCode::Char('m'));
        ent_press(&mut app, KeyCode::Char('m'));
        assert!(app.entities.as_ref().unwrap().merge_keep.is_none());
    }

    fn two_row_app() -> App {
        let mut app = test_app();
        let mut modal = entity::EntityModal::new();
        modal.install(
            r#"[{"id":"person-a","name":"Grace Choi","node_type":"person"},
                {"id":"person-b","name":"Youn Ji Choi","node_type":"person"}]"#,
        );
        app.entities = Some(modal);
        app
    }

    /// The only irreversible action on this modal is the only one that
    /// confirms — and anything that is not an explicit yes is a no.
    #[test]
    fn a_merge_confirms_and_anything_but_y_cancels() {
        for dismissal in [KeyCode::Esc, KeyCode::Char('n'), KeyCode::Char('q')] {
            let mut app = two_row_app();
            ent_press(&mut app, KeyCode::Char('m'));
            ent_press(&mut app, KeyCode::Down);
            ent_press(&mut app, KeyCode::Char('m'));
            let m = app.entities.as_ref().unwrap();
            let (keep, _, dup, _) = m.merge_confirm.clone().expect("confirm pending");
            assert_eq!(keep, "person-a");
            assert_eq!(dup, "person-b");

            ent_press(&mut app, dismissal);
            let m = app.entities.as_ref().unwrap();
            assert!(m.merge_confirm.is_none(), "{dismissal:?} left it pending");
            assert!(m.merge_keep.is_none(), "{dismissal:?} left a mark");
            assert!(app.entities.is_some(), "{dismissal:?} closed the modal");
        }
    }

    /// While a merge is pending, nothing else responds — a stray `r` must
    /// not open a rename behind a confirmation.
    #[test]
    fn a_pending_merge_swallows_every_other_key() {
        let mut app = two_row_app();
        ent_press(&mut app, KeyCode::Char('m'));
        ent_press(&mut app, KeyCode::Down);
        ent_press(&mut app, KeyCode::Char('m'));
        ent_press(&mut app, KeyCode::Char('r'));
        assert!(
            app.entities.as_ref().unwrap().edit.is_none(),
            "a rename opened behind the confirmation"
        );
    }

    /// Ctrl-N seeds the new-person box with what was typed, because the
    /// moment you want it is the moment a lookup came back empty.
    #[test]
    fn ctrl_n_prefills_the_new_person_with_the_query() {
        let mut app = test_app();
        app.entities = Some(entity::EntityModal::new());
        for c in ['D', 'a', 'n', 'a'] {
            ent_press(&mut app, KeyCode::Char(c));
        }
        ent_press_ctrl(&mut app, KeyCode::Char('n'));
        let (kind, buf) = app.entities.as_ref().unwrap().edit.as_ref().unwrap();
        assert_eq!(*kind, entity::EditKind::NewPerson);
        assert_eq!(buf, "Dana");
    }

    #[test]
    fn a_pane_of_text_to_copy_hands_the_mouse_back_to_the_terminal() {
        // Mouse capture is why a drag never selected the authorization URL:
        // the terminal forwarded it here instead of drawing a selection, and
        // the pane's documented fallback — "the URL stays on screen to be
        // selected by hand" — was therefore not available at all.
        assert!(picking_app().wants_the_mouse_back());
        // Every modal, not just that one: while a pane covers the screen the
        // only thing capture buys is a wheel scrolling the transcript behind
        // it, which nobody can see.
        let mut listing = picking_app();
        listing.documents.as_mut().unwrap().pick = None;
        assert!(listing.wants_the_mouse_back());
        let mut helping = test_app();
        helping.help = true;
        assert!(helping.wants_the_mouse_back());
        // And with nothing up the wheel is the transcript's, until asked for.
        assert!(!test_app().wants_the_mouse_back());
    }

    #[test]
    fn selection_mode_takes_the_mouse_and_says_so_on_the_strip() {
        let mut app = test_app();
        app.selecting = true;
        assert!(app.wants_the_mouse_back());
        // The badge is not decoration: selection mode is the state in which
        // the scroll wheel stops working, and a wheel that has stopped with
        // nothing on screen to explain it reads as a broken session.
        let strip: String = app
            .status("claude-opus-5", "anthropic", 10)
            .spans
            .iter()
            .map(|s| s.content.clone().into_owned())
            .collect();
        assert!(strip.contains("select ^S"), "{strip}");
        app.selecting = false;
        let off: String = app
            .status("claude-opus-5", "anthropic", 10)
            .spans
            .iter()
            .map(|s| s.content.clone().into_owned())
            .collect();
        assert!(!off.contains("select"), "{off}");
    }

    #[test]
    fn a_paste_while_picking_lands_in_the_field_and_not_the_message_box() {
        let mut app = picking_app();
        // As it arrives from a browser's address bar copied out of a wrapped
        // display: one address, broken across lines.
        assert!(paste_into_pick(
            &mut app,
            "http://127.0.0.1:8765/callback?state=pEykz\n6Jtni&code=4/0AX"
        ));
        let pick = app.documents.as_ref().unwrap().pick.as_ref().unwrap();
        assert_eq!(
            pick.buffer,
            "http://127.0.0.1:8765/callback?state=pEykz6Jtni&code=4/0AX"
        );
        assert_eq!(pick.cursor, pick.buffer.len());
        assert!(
            app.input.is_empty(),
            "it went to the prompt: {:?}",
            app.input
        );
    }

    #[test]
    fn a_paste_with_no_pick_up_is_still_the_message_box_s() {
        let mut app = test_app();
        assert!(!paste_into_pick(&mut app, "some prose"));
    }

    /// An `App` with nothing going on, for frame tests. Fields that need a
    /// live agent stay inert: `running` is `None`, channels dangle unused.
    pub(super) fn test_app() -> App {
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
            selecting: false,
            help: false,
            help_scroll: 0,
            tools: None,
            skills: None,
            charter: None,
            pending_charter_edit: false,
            learning: None,
            pending_lesson_edit: None,
            skills_dir: std::path::PathBuf::from("/nonexistent-skills"),
            sandbox_line: "sandbox: none — commands run as you, with your credentials".into(),
            workspace: std::env::temp_dir(),
            todo_visible: true,
            pending_editor: false,
            scheduled: None,
            staged: None,
            requests: None,
            mail: None,
            find: None,
            documents: None,
            entities: None,
            tasks: None,
            queues: None,
            poll_monitor: None,
            health: None,
            pending_doctor_remedy: None,
            pending_trigger_edit: None,
            pending_outbox_edit: None,
            outbox_pending: 0,
            affect: None,
            valence: None,
            review: command::ReviewMode::default(),
            watches: Vec::new(),
            shell_tx,
            attach_tx: mpsc::unbounded_channel().0,
            attached: None,
            dropped: Vec::new(),
            attaching: None,
            last_waiting_note: None,
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

    /// What `deliver_inbound` refuses. A slash command from a phone would
    /// rebuild the agent or drop the conversation and its taint; a `!` escape
    /// would run a shell command with no approver in front of it. The gap
    /// between "the owner typed this" and "the owner is at the keyboard" is
    /// exactly where a remote surface should stay narrow.
    #[test]
    fn remote_text_that_would_be_a_command_is_recognised_as_one() {
        for dangerous in ["/clear", "/model claude-opus-5", "/mode allow", "!rm -rf ."] {
            assert!(
                command::parse(dangerous).is_some() || command::shell_escape(dangerous).is_some(),
                "{dangerous} would have been sent to the model as a prompt"
            );
        }
        // And ordinary prompts are not caught by it — a refusal that fires on
        // real questions is a remote control nobody uses.
        for ordinary in [
            "summarise the inbox",
            "what did the 7am briefing say?",
            "why / how did that fail",
        ] {
            assert!(
                command::parse(ordinary).is_none() && command::shell_escape(ordinary).is_none(),
                "{ordinary} would have been refused"
            );
        }
    }

    /// A switch that rebuilds the agent must carry the mode in force, not the
    /// one the process launched with. Asserted by pointer identity, because
    /// the bug was precisely that the *retained* ask-mode approver was reused
    /// for every mode — leaving `/mode read-only` displaying read-only while
    /// the harness asked, which loosens rather than tightens.
    #[test]
    fn a_rebuild_keeps_the_permission_mode_it_was_in() {
        let (tui, _rx) = approve::TuiApprover::new();
        let retained: Arc<dyn Approver> = Arc::new(tui);

        let ask = approver_for(PermissionMode::Ask, &retained);
        assert!(
            Arc::ptr_eq(&ask, &retained),
            "ask must reinstate the approver wired to the event loop"
        );
        for other in [PermissionMode::ReadOnly, PermissionMode::Allow] {
            let got = approver_for(other, &retained);
            assert!(
                !Arc::ptr_eq(&got, &retained),
                "{other:?} silently fell back to asking"
            );
        }
    }

    /// A mirrored session is one whose output is leaving the machine, so the
    /// fact rides on the strip that is always visible rather than in a modal.
    /// The failure this guards is not cosmetic: not knowing you are attached
    /// is not knowing where what you type is going.
    #[test]
    fn an_attached_session_says_so_on_the_always_visible_strip() {
        let mut app = test_app();
        assert!(
            !frame_text(&mut app, 110, 12, None).contains("⇄"),
            "an unattached session must claim nothing"
        );

        app.attached = Some(crate::slack::remote::Attached {
            name: "lab".into(),
            channel_id: "D1".into(),
            thread_ts: "1755.0001".into(),
            slack: mecha_slack::Slack::new("xoxb-not-a-real-token"),
            flush_chars: 400,
            flush_ms: 700,
        });
        let attached = frame_text(&mut app, 110, 12, None);
        assert!(attached.contains("⇄ lab"), "{attached}");
    }

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
            cancel: mecha_core::agent::CancelHandle::new(),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            started: std::time::Instant::now(),
            cancelling: false,
            persisted: Vec::new(),
            outbox_before: None,
        });
        let text = frame_text(&mut app, 80, 12, None);
        assert!(text.contains("working"), "{text}");
        assert!(text.contains("type to steer"), "{text}");
    }

    #[test]
    fn the_help_overlay_advertises_the_newline_key_only_where_it_exists() {
        // On a terminal without the kitty protocol, Shift+Enter *submits* —
        // help that teaches it as a newline is worse than no help.
        // 40 rows: the whole card, keys plus every HELP line, has to fit —
        // the /clear assertion below reads the far end of it.
        let mut app = test_app();
        app.help = true;
        let plain = frame_text(&mut app, 100, 40, None);
        assert!(plain.contains("alt+enter"), "{plain}");
        assert!(!plain.contains("shift+enter"), "{plain}");
        assert!(
            plain.contains("/clear"),
            "commands render from HELP: {plain}"
        );

        app.kitty_keyboard = true;
        let kitty = frame_text(&mut app, 100, 40, None);
        assert!(kitty.contains("shift+enter"), "{kitty}");
    }

    #[test]
    fn the_help_overlay_shows_its_longest_line_in_full() {
        // The box was a fixed 70 columns and `/doctor` is past eighty
        // characters, so the entry that most needed explaining ended
        // mid-sentence — and a `Paragraph` with no `.wrap()` truncates in
        // silence. Every HELP line, whole, or the card is lying about what
        // the commands do.
        let mut app = test_app();
        app.help = true;
        // Tall enough to hold every HELP line at once. The overlay itself
        // scrolls, so a user never loses one — but this test asserts on the
        // rendered text, and a line that scrolled off is indistinguishable
        // here from a line that was truncated. Grow this with HELP.
        let text = frame_text(&mut app, 120, 60, None);
        for line in command::HELP.lines() {
            assert!(
                text.contains(line.trim_end()),
                "truncated: {:?}\n{text}",
                line
            );
        }
    }

    #[test]
    fn a_short_terminal_scrolls_the_help_instead_of_swallowing_half_of_it() {
        // Twenty-six rows is an ordinary split pane, and the card ends around
        // `/outbox` there — with no bottom border, which is exactly the
        // picture that reads as a broken overlay rather than as more to come.
        let mut app = test_app();
        app.help = true;
        let top = frame_text(&mut app, 120, 26, None);
        assert!(top.contains("↑↓ scrolls"), "{top}");
        assert!(top.contains("enter"), "{top}");
        assert!(!top.contains("/exit"), "the tail is below the fold: {top}");

        // Scrolled to the end, the tail is reachable and the head is gone.
        app.help_scroll = 99;
        let bottom = frame_text(&mut app, 120, 26, None);
        assert!(bottom.contains("/exit"), "{bottom}");
        // And the caller's copy was clamped rather than left past the end.
        assert!(app.help_scroll < 99, "scroll was not clamped");
    }

    /// The badge follows the plan badge's rule: pending drafts are the
    /// exception worth a coloured block; zero is the state that says nothing.
    #[test]
    fn the_outbox_badge_appears_only_when_something_is_pending() {
        let mut app = test_app();
        let clear = frame_text(&mut app, 110, 12, None);
        assert!(!clear.contains("outbox"), "{clear}");

        app.outbox_pending = 3;
        let badged = frame_text(&mut app, 110, 12, None);
        assert!(badged.contains("outbox 3"), "{badged}");
    }

    /// §6.2: `None` — no run yet, or the last one was `Neutral` — clears the
    /// badge entirely, which is what "show nothing" on neutral means in
    /// practice.
    #[test]
    fn the_affect_badge_appears_only_when_the_label_is_not_neutral() {
        let mut app = test_app();
        let clear = frame_text(&mut app, 110, 12, None);
        assert!(!clear.contains("anger"), "{clear}");

        app.affect = Some(mecha_core::appraisal::Affect::Anger);
        let badged = frame_text(&mut app, 110, 12, None);
        assert!(badged.contains("anger"), "{badged}");
    }

    /// The owner's ruling for this surface: a number. A run with a signed
    /// reading and no label shows the reading alone; with both, the word
    /// then the number.
    #[test]
    fn the_valence_badge_is_a_number_and_shows_without_a_label() {
        let mut app = test_app();
        app.valence = Some(mecha_core::appraisal::Valence {
            positive: 1.0,
            negative: 0.5,
            positives: 1,
            negatives: 1,
            visible: false,
            partial: false,
        });
        let badged = frame_text(&mut app, 110, 12, None);
        assert!(badged.contains("+1.0 \u{2212}0.5"), "{badged}");
        assert!(!badged.contains("neutral"), "{badged}");

        app.affect = Some(mecha_core::appraisal::Affect::Anger);
        let both = frame_text(&mut app, 110, 12, None);
        assert!(both.contains("anger +1.0 \u{2212}0.5"), "{both}");
    }

    fn pending_row(id: &str) -> outbox::OutboxRow {
        outbox::OutboxRow {
            id: id.into(),
            status: "pending".into(),
            kind: mecha_core::outbox::OutboxKind::Message,
            summary: "docs__docs_replace".into(),
            tainted: true,
            edited: false,
            args_text: "find  Spring 2024".into(),
            error: None,
            detail: Vec::new(),
            raw: Vec::new(),
        }
    }

    fn press(app: &mut App, c: char) {
        handle_outbox_key(
            app,
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: crossterm::event::KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            },
        )
        .unwrap();
    }

    /// `a` for approve, beside `e` edit and `r` reject — and `s`, which meant
    /// this same action before the rename, still works. A key that releases
    /// an outbound action is the wrong one to retire out from under someone's
    /// fingers, and here it cannot mean anything else.
    #[test]
    fn approve_answers_to_a_and_still_to_s() {
        for key in ['a', 's'] {
            let mut app = test_app();
            app.staged = Some(outbox::OutboxModal::new(vec![pending_row("abc123")]));
            press(&mut app, key);
            let confirm = &app.staged.as_ref().unwrap().confirm;
            assert!(
                confirm.as_ref().is_some_and(|c| c.id == "abc123"),
                "`{key}` did not open the approval"
            );
        }
    }

    /// Scrolling a long draft's arguments is not answering the question. Any
    /// other key still keeps it pending — that half must not regress.
    #[test]
    fn scroll_keys_do_not_dismiss_the_approval() {
        let mut app = test_app();
        app.staged = Some(outbox::OutboxModal::new(vec![pending_row("abc123")]));
        press(&mut app, 'a');

        let scroll = |app: &App| {
            app.staged
                .as_ref()
                .unwrap()
                .confirm
                .as_ref()
                .map(|c| c.scroll)
        };

        press(&mut app, 'j');
        assert_eq!(scroll(&app), Some(1), "`j` did not scroll down");
        press(&mut app, 'j');
        assert_eq!(scroll(&app), Some(2), "`j` did not keep scrolling");
        press(&mut app, 'k');
        assert_eq!(scroll(&app), Some(1), "`k` did not scroll back up");
        // Floors rather than wrapping to u16::MAX.
        for _ in 0..5 {
            press(&mut app, 'k');
        }
        assert_eq!(scroll(&app), Some(0), "scrolling up past the top wrapped");

        // And the guard it must not weaken.
        press(&mut app, 'z');
        assert!(
            app.staged.as_ref().unwrap().confirm.is_none(),
            "an unrelated key must still keep the draft pending"
        );
    }

    /// A tainted draft's send confirmation puts the arguments on screen —
    /// what is approved must be what was read, through the real draw path.
    #[test]
    fn the_outbox_confirm_puts_a_tainted_drafts_arguments_on_screen() {
        let mut app = test_app();
        app.staged = Some(outbox::OutboxModal {
            confirm: Some(outbox::SendConfirm {
                scroll: 0,
                id: "abc123".into(),
                summary: "mail to a@example.com".into(),
                tainted: true,
                args_text: "{\n  \"to\": \"a@example.com\"\n}".into(),
                error_before: None,
            }),
            ..outbox::OutboxModal::new(Vec::new())
        });
        let frame = frame_text(&mut app, 110, 35, None);
        assert!(frame.contains("attacker"), "{frame}");
        assert!(frame.contains("a@example.com"), "{frame}");
        assert!(frame.contains("y approve"), "{frame}");

        // Untainted: no warning, but still a confirmation — an approval is the
        // one keystroke here that cannot be taken back.
        app.staged.as_mut().unwrap().confirm = Some(outbox::SendConfirm {
            scroll: 0,
            id: "abc123".into(),
            summary: "mail to a@example.com".into(),
            tainted: false,
            args_text: String::new(),
            error_before: None,
        });
        let frame = frame_text(&mut app, 110, 35, None);
        assert!(!frame.contains("attacker"), "{frame}");
        assert!(frame.contains("approve abc123"), "{frame}");
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
            detail_scroll: 0,
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

    fn skill_row(name: &str) -> skills::SkillRow {
        skills::SkillRow {
            name: name.into(),
            description: "how to answer a rec-letter request".into(),
            triggers: Vec::new(),
            narrows: None,
            body: "1. read the request\n2. draft it".into(),
            dir: std::path::PathBuf::from("/skills").join(name),
            carried: true,
            loaded: false,
            error: None,
        }
    }

    /// The three states a user opens /skills to tell apart have to be
    /// distinguishable on the *list*, not only in the detail view — the whole
    /// reason a withheld skill is listed rather than omitted is that "not
    /// carried" and "the model didn't use it" look identical otherwise.
    #[test]
    fn the_skills_list_separates_loaded_withheld_and_failed() {
        let mut app = test_app();
        app.skills = Some(skills::SkillsModal {
            rows: vec![
                skills::SkillRow {
                    loaded: true,
                    ..skill_row("rec-letter")
                },
                skills::SkillRow {
                    carried: false,
                    ..skill_row("expenses")
                },
                skills::SkillRow {
                    error: Some("missing `description`".into()),
                    carried: false,
                    ..skill_row("halfwritten")
                },
            ],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: std::path::PathBuf::from("/skills"),
        });
        let text = frame_text(&mut app, 110, 30, None);
        assert!(text.contains("rec-letter"), "{text}");
        assert!(text.contains("loaded"), "{text}");
        assert!(text.contains("withheld"), "{text}");
        assert!(
            text.contains("failed") && text.contains("missing `description`"),
            "a broken SKILL.md is only ever visible here — the startup warning \
             goes to a stderr the alternate screen ate: {text}"
        );
        assert!(
            text.contains("1 of 2 skills carried"),
            "the ratio excludes what could not load: {text}"
        );
    }

    /// Loading a skill narrows the tool surface for the rest of the
    /// conversation and there is no unload, so the detail view has to say so.
    /// Nothing else in the TUI reports it: /tools lists the whole registry,
    /// not the narrowed dispatch set.
    #[test]
    fn the_skills_detail_names_the_narrowing_and_that_it_is_in_force() {
        let mut app = test_app();
        app.skills = Some(skills::SkillsModal {
            rows: vec![skills::SkillRow {
                narrows: Some(vec!["fs_read".into(), "mail_send".into()]),
                loaded: true,
                ..skill_row("rec-letter")
            }],
            selected: 0,
            detail: true,
            detail_scroll: 0,
            dir: std::path::PathBuf::from("/skills"),
        });
        let text = frame_text(&mut app, 100, 30, None);
        assert!(text.contains("narrows the tool surface to"), "{text}");
        assert!(text.contains("fs_read"), "{text}");
        assert!(text.contains("in force until /clear"), "{text}");
        assert!(
            text.contains("read the request"),
            "the procedure itself is the point of the detail view: {text}"
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
                ..Default::default()
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

    /// `(cursor_col, cursor_row, rows)` — the shape the old tuple-returning
    /// `input_layout` had, so these tests still read as what they assert.
    fn at(text: &str, cursor: usize, width: u16) -> (u16, u16, usize) {
        let l = input_layout(text, cursor, width);
        (l.cursor_col, l.cursor_row, l.rows.len())
    }

    /// The rows as they would be drawn — what `draw` builds its `Line`s from.
    fn painted(text: &str, width: u16) -> Vec<String> {
        input_layout(text, 0, width)
            .rows
            .iter()
            .map(|r| text[r.clone()].trim_end_matches('\n').to_string())
            .collect()
    }

    #[test]
    fn a_chord_is_not_the_letter_it_is_spelled_with() {
        use crossterm::event::KeyEventKind;
        let key = |c, m| KeyEvent {
            code: KeyCode::Char(c),
            modifiers: m,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        // Plain, and shifted — that is how a capital arrives under the kitty
        // protocol, and refusing it would make the caps-lock key eat input.
        assert_eq!(typed_char(&key('c', KeyModifiers::NONE)), Some('c'));
        assert_eq!(typed_char(&key('C', KeyModifiers::SHIFT)), Some('C'));

        // The bug: `KeyCode::Char('a')` with CONTROL held is Ctrl-A, and
        // `/mail` fed it straight to `action_for`, which archives.
        for m in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            assert_eq!(typed_char(&key('a', m)), None, "{m:?}");
        }
        // Named keys are never a typed character.
        assert_eq!(
            typed_char(&KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }),
            None
        );
    }

    #[test]
    fn no_mail_action_can_be_reached_by_a_chord() {
        // The half that matters: every letter `/mail` acts on is a mutation,
        // and Ctrl-A / Ctrl-D / Ctrl-T / Ctrl-R are chords a person presses
        // meaning something else entirely.
        for c in ['a', 's', 't', 'd', 'n', 'r', 'f', 'e', 'q'] {
            assert!(mail::action_for(c).is_some(), "{c} should be an action");
            let chord = KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::CONTROL,
                kind: crossterm::event::KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            };
            assert!(
                typed_char(&chord).and_then(mail::action_for).is_none(),
                "ctrl+{c} must not reach an action"
            );
        }
    }

    #[test]
    fn the_rows_partition_the_text_so_nothing_is_shown_twice_or_lost() {
        // The invariant the caret arithmetic stands on: find the row that
        // contains the cursor and the column is a width of one slice. It only
        // works if every byte is in exactly one row.
        for text in [
            "",
            "\n",
            "a\n\nb",
            "short",
            "a much longer line that has to wrap somewhere",
            "https://example.com/a/very/long/path/that/cannot/fit/in/one/row",
            "  leading and trailing   ",
            "naïve 日本語 mixed",
        ] {
            for width in [1u16, 3, 7, 12, 40] {
                let rows = input_layout(text, 0, width).rows;
                assert_eq!(rows[0].start, 0, "{text:?} @ {width}");
                assert_eq!(rows.last().unwrap().end, text.len(), "{text:?} @ {width}");
                for pair in rows.windows(2) {
                    assert_eq!(pair[0].end, pair[1].start, "{text:?} @ {width}");
                }
            }
        }
    }

    #[test]
    fn a_word_moves_down_whole_and_the_caret_goes_with_it() {
        // The bug this pins, measured against what ratatui actually painted:
        // at 30 columns the last row reads "class", and the old character
        // wrapper put the caret at column 3 of it — because it broke at
        // exactly 30 while the Paragraph beside it broke at word boundaries.
        // Two wrappers, one caret. Now there is one wrapper.
        let text = "can you help me prepare my schedule for my undergrad fMRI class";
        assert_eq!(
            painted(text, 30),
            vec![
                "can you help me prepare my ",
                "schedule for my undergrad fMRI ",
                "class",
            ]
        );
        assert_eq!(at(text, text.len(), 30), (5, 2, 3));
    }

    #[test]
    fn a_wide_character_costs_two_cells() {
        // The other half of the same bug: the old wrapper counted characters,
        // so one CJK glyph read as one cell and the caret sat a cell left of
        // the text for every glyph before it.
        assert_eq!(at("日本語", 9, 10), (6, 0, 1));
        // Three glyphs at two cells each is six, so a fourth does not fit in
        // seven columns.
        assert_eq!(painted("日本語だ", 7), vec!["日本語", "だ"]);
    }

    #[test]
    fn the_cursor_tracks_plain_wrapping() {
        // 10 columns, and "abcdefghijk" is one word too long for a row, so it
        // breaks at the edge — there is nowhere better to break it.
        assert_eq!(at("abcdefghijk", 11, 10), (1, 1, 2));
        assert_eq!(at("abc", 3, 10), (3, 0, 1));
        assert_eq!(at("", 0, 10), (0, 0, 1));
    }

    #[test]
    fn a_caret_at_the_end_of_a_full_row_starts_the_next_one() {
        // Rows break lazily — when the character that overflows arrives — so
        // at the end of an exactly-full row there is no next row yet, and the
        // caret would be drawn on the border. It belongs where the next
        // character will land.
        assert_eq!(at("abcdefghij", 10, 10), (0, 1, 2));
    }

    #[test]
    fn a_pasted_newline_breaks_the_line_instead_of_being_counted_as_a_character() {
        // The bug this pins: the arithmetic before either wrapper divided the
        // character count by the width, so any pasted snippet put the cursor
        // somewhere else entirely and the box was drawn too short.
        let text = "one\ntwo";
        assert_eq!(at(text, text.len(), 40), (3, 1, 2));

        let three = "a\nb\nc";
        assert_eq!(at(three, three.len(), 40), (1, 2, 3));

        // Just before the newline is the end of the first row, not the start
        // of the second — the newline is in the row it ends, which is what
        // keeps the ranges a partition.
        assert_eq!(at(text, 3, 40), (3, 0, 2));
        assert_eq!(at(text, 4, 40), (0, 1, 2));
    }

    #[test]
    fn a_trailing_newline_leaves_an_empty_row_to_type_on() {
        assert_eq!(at("hi\n", 3, 40), (0, 1, 2));
        assert_eq!(painted("hi\n", 40), vec!["hi", ""]);
    }

    #[test]
    fn a_cursor_in_the_middle_of_pasted_text_lands_on_the_right_row() {
        let text = "one\ntwo\nthree";
        // Just after the second newline: start of the third row.
        assert_eq!(at(text, 8, 40), (0, 2, 3));
    }

    /// **A fan-out the child did not report is not a fan-out of nothing.**
    ///
    /// The status line after a group verdict was built from
    /// `cascade_tally(&report).unwrap_or((0, 0))`, and `cascade_tally`
    /// answers `None` whenever the report carries no `cascade:` line. That
    /// flattening made two opposite situations render identically:
    ///
    /// A singleton group asks for no fan-out, so there is no line to read and
    /// `×1` is the whole truth. But a group of seven whose cascade arm
    /// produced nothing readable — an older graph binary, a changed line —
    /// also came out `×1`, and `left > 0` was false so the "(N similar left
    /// pending)" note never rendered. Silence there reads as "none left", so
    /// the only surface telling a person what their keystroke covered could
    /// not distinguish *nothing was left pending* from *I do not know whether
    /// anything was* — the absence of the note being itself the claim.
    ///
    /// Same flattening the web route carried until #128 took it out one
    /// function over; this is pre-existing rather than that branch's doing —
    /// `cascade_tally` has always returned `Option`.
    #[test]
    fn a_group_verdict_says_what_it_does_not_know() {
        // Read, and complete.
        assert_eq!(
            group_verdict_status("reject", "Sage plays cello", 6, Some((6, 0))),
            "rejected ×7 — Sage plays cello"
        );
        // Read, and partial: the members the graph could not sweep are named.
        assert!(
            group_verdict_status("accept", "Sage plays cello", 6, Some((4, 2)))
                .contains("2 similar left pending")
        );
        // Nothing was asked to fan out, so nothing is unaccounted for.
        assert_eq!(
            group_verdict_status("reject", "Sage plays cello", 0, None),
            "rejected ×1 — Sage plays cello"
        );
        // Asked, and unreported. The old line said exactly the same thing as
        // the singleton above, which is the bug.
        let unknown = group_verdict_status("reject", "Sage plays cello", 6, None);
        assert_ne!(unknown, "rejected ×1 — Sage plays cello");
        assert!(
            unknown.contains("unreported") && unknown.contains("still pending"),
            "an unreadable tally must say so, and say what is unaccounted for: {unknown}"
        );
    }

    /// **What the line says has to survive being drawn.**
    ///
    /// `QueuesModal::draw` renders the status as a `Paragraph` into a
    /// `Rect { height: 1 }` with no `.wrap()`, so it clips. The box is
    /// `122.min(frame.width)` with `Borders::ALL`, and the line carries a
    /// two-space indent — 76 columns of message on an eighty-column terminal.
    ///
    /// The first fix put its caveat *after* the forty-eight-character
    /// statement head, which pushed it past that budget: on a normal terminal
    /// the reader saw `rejected the seed — Sage plays…` and no warning at all.
    /// That is the bug this pair of tests exists for, one layer down — the
    /// note absent, and its absence read as nothing to report.
    ///
    /// **A zero exit code is not a verdict, and this arm removes a row.**
    ///
    /// mecha-graph reports a per-candidate failure as `#id FAILED: …` and
    /// exits 0, so `Ok(report)` can carry a verdict that did not happen. The
    /// item level learned it on `#2951` — row gone locally, still pending in
    /// the store — and the web route answers 409 on the same reading. The
    /// group arm did neither.
    ///
    /// The two readings have to come apart on the SAME report, which is what
    /// this pins: a report whose only line is a `FAILED` has `tally_report`
    /// saying nothing landed, so the group must stay and the child's own
    /// sentence must reach the screen. It also carries no `cascade:` line, so
    /// the status builder — asked in isolation — would call it "the seed
    /// landed, the fan-out is unknown". Which is why the guard belongs at the
    /// call site, ahead of it, and not in another arm of the formatter.
    #[test]
    fn a_verdict_that_landed_on_nothing_keeps_its_group() {
        use crate::commands::review::{cascade_tally, tally_report, why_nothing_landed};
        let failed = "#9281 FAILED: cannot resolve subject 'Sage'\n";

        let (landed, _) = tally_report(failed);
        assert_eq!(landed, 0, "a FAILED line must not read as a landed verdict");
        assert!(
            why_nothing_landed(failed).contains("cannot resolve subject"),
            "the child's own reason is what the reviewer needs to act"
        );

        // The same report through the status builder alone, to show what the
        // guard is standing in front of: no `cascade:` line, so this would
        // announce a seed that never landed.
        assert!(
            group_verdict_status("reject", "Sage plays cello", 6, cascade_tally(failed))
                .contains("the seed only"),
            "the formatter cannot see this case — the caller must"
        );

        // And a report that DID land still reads as one, or the guard would
        // swallow every good verdict.
        let ok = "#9281 rejected\ncascade: 6 rejected\n";
        assert_eq!(tally_report(ok).0, 1);
        assert_eq!(cascade_tally(ok), Some((6, 0)));
    }

    /// So the counts precede the head everywhere, and this **draws the modal
    /// and reads the buffer** rather than counting characters.
    ///
    /// A first version asserted against a hand-written budget of 76 columns,
    /// which is three constants from `queues.rs` copied into a comment:
    /// `122u16.min(frame.area().width)`, `Borders::ALL`, and the two-space
    /// indent in `format!("  {s}")`. Widen the box, drop the indent, or add a
    /// prefix span, and the caveat starts clipping again while the test stays
    /// green — the same failure restored one constant later. Grade the
    /// artifact: `QueuesModal::draw` is public and `it_draws_at_tiny_sizes`
    /// already drives it through a `TestBackend`, so what a reader sees is
    /// available to assert on directly.
    #[test]
    fn the_verdict_line_survives_an_eighty_column_terminal() {
        // What the modal actually puts on an 80×24 screen.
        fn on_screen(status: &str) -> String {
            let mut m = queues::QueuesModal::new(
                queues::queues_from_json(
                    r#"[{"queue":"graph candidates","depth":6434,"detail":"d","opens":"o"}]"#,
                )
                .unwrap(),
            );
            m.status = Some(status.to_string());
            let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
            term.draw(|f| m.draw(f)).unwrap();
            let buf = term.backend().buffer().clone();
            (0..24)
                .map(|y| {
                    (0..80)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        // The longest head the caller can produce: `handle_queues_key` clips
        // its statement to 48 characters before calling in.
        let head: String = "a".repeat(48);

        let partial = on_screen(&group_verdict_status("reject", &head, 6, Some((4, 2))));
        assert!(
            partial.contains("2 similar left pending"),
            "a partial sweep must still say so on an 80-column screen:\n{partial}"
        );

        let unknown = on_screen(&group_verdict_status("reject", &head, 6, None));
        assert!(
            unknown.contains("unreported") && unknown.contains("still pending"),
            "the unreported caveat must reach the screen:\n{unknown}"
        );

        // A four-digit member count is the widest this arm can get.
        let wide = on_screen(&group_verdict_status("accept", &head, 9999, None));
        assert!(
            wide.contains("still pending"),
            "even the widest count must leave room for the caveat:\n{wide}"
        );

        // The failure lines are read on the same screen and were the only
        // ones here not graded against a buffer. Both put a bounded hint in
        // front of an unbounded tail — the graph's `FAILED` line, whose
        // length is the subject's, and an error chain — so both are checked
        // with a tail long enough to clip.
        let long_subject = "a".repeat(60);
        let why = crate::commands::review::why_nothing_landed(&format!(
            "#9281 FAILED: cannot resolve subject '{long_subject}'\n"
        ));
        let nothing_landed = on_screen(&format!("b bind · A accept new — {why}"));
        assert!(
            nothing_landed.contains("A accept new"),
            "the key that answers an unresolvable subject must reach the screen:\n{nothing_landed}"
        );

        let errored = on_screen(&format!(
            "b bind · A accept new — reject failed, nothing cascaded: {}",
            "chained context: ".repeat(6)
        ));
        assert!(
            errored.contains("A accept new"),
            "and must survive an error chain of any length:\n{errored}"
        );
    }

    #[test]
    fn a_zero_width_terminal_does_not_divide_by_zero() {
        // A pty with no window size reports 0 columns, and this is the
        // arithmetic that runs first.
        assert!(!input_layout("abc", 3, 0).rows.is_empty());
    }

    #[test]
    fn a_long_message_scrolls_instead_of_eating_the_transcript() {
        // Twelve rows of text in a box that shows six: the caret stays on
        // screen and the title says which line it is on, because a box that
        // silently looks shorter than its content is the shape the outbox
        // modal's hidden-item counter exists to avoid.
        let mut app = test_app();
        app.input = (0..12)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.cursor = app.input.len();
        let text = frame_text(&mut app, 40, 30, None);
        assert!(text.contains("line 12/12"), "{text}");
        // The last six rows are shown; the first six have scrolled off.
        assert!(text.contains("line11"), "{text}");
        assert!(text.contains("line6"), "{text}");
        assert!(!text.contains("line5"), "{text}");
    }

    #[test]
    fn the_input_box_draws_what_the_caret_was_measured_against() {
        // The end-to-end form of the whole fix: the row under the caret, as
        // painted, is `cursor_col` cells wide up to the cursor. It fails on
        // the old code, where the Paragraph wrapped one way and the caret was
        // computed another.
        let mut app = test_app();
        app.input = "can you help me prepare my schedule for my undergrad fMRI class".into();
        app.cursor = app.input.len();
        let (width, height) = (32u16, 20u16);
        let text = frame_text(&mut app, width, height, None);
        let layout = input_layout(&app.input, app.cursor, width - 2);
        let last = text
            .lines()
            .rev()
            .find(|l| l.contains("class"))
            .expect("the last row of the input is on screen");
        // Strip the border column at each end.
        let painted = last.trim_end().trim_start_matches('\u{2502}');
        let painted = painted.trim_end_matches('\u{2502}');
        assert_eq!(painted.trim_end(), "class");
        assert_eq!(layout.cursor_col as usize, "class".len());
    }
}
