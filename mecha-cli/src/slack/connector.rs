//! `mecha slack connect` — the always-on front-end.
//!
//! One process, one `Agent`, one task per active thread. It owns the socket,
//! gates every inbound event, spawns runs, and renders them. See
//! `docs/SLACK-DESIGN.md`; the decisions that live in the code rather than the
//! prose are marked below.
//!
//! Three loops meet in one `select!`: what Slack sends, what an approver is
//! waiting to ask, and what a finished run hands back. Keeping them in one
//! place is what makes the thread map a plain `HashMap` rather than something
//! shared across tasks — the loop is the only writer, which is also what makes
//! the thread store's no-lock rule true rather than hoped-for.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mecha_core::agent::{Agent, AgentEvent, Budget, Conversation, RunOutcome};
use mecha_core::message::{Block, Message};
use mecha_core::outbox::OutboxRoute;
use mecha_core::session::{Record, Session, SessionMeta};
use mecha_core::tool::ToolCtx;
use mecha_slack::binding::{self, Binding, Credentials, Gate, SlackStore};
use mecha_slack::envelope::{FileRef, Inbound, Interaction, SlackEvent};
use mecha_slack::{blocks, chat, Slack, SocketMode, SocketOptions};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::actions::{self, Action, ActionLedger, Executor};
use super::approve::{self, Answer, Mode, SlackApprover};
use super::pump::{pump, PumpConfig};
use super::review::{self, ReviewMode};
use super::threads::{Event, RunMarker, ThreadRecord, ThreadStore};
use crate::{setup, GlobalOpts};

/// How many event ids are remembered for deduplication. Slack's redelivery
/// semantics across a dropped socket are undocumented, so handlers are
/// idempotent and this is what makes them so.
const SEEN_EVENTS: usize = 512;

/// A run in flight, and the handles that steer or stop it.
struct Live {
    cancel: CancellationToken,
    queue: Arc<Mutex<VecDeque<String>>>,
    mode: Arc<Mutex<Mode>>,
    /// The run's unanswered-approval latch, shared with its `SlackApprover`.
    /// Set after a card times out; while set, every gated call is refused
    /// without asking. The approver lives as long as the run, so only this
    /// loop can clear it — and it does, whenever the user is proven to be
    /// watching again: a steering message, or any approval-card button press.
    unanswered: Arc<AtomicBool>,
}

/// A finished run, handed back to the loop that owns the conversations.
struct Completion {
    key: String,
    conversation: Conversation,
    outcome: Result<Box<RunOutcome>, String>,
    /// Where this run's own messages start in `conversation` — the count
    /// before its user turn was appended — so the appraisal readout scopes
    /// interventions to this run and not to the whole thread.
    run_started_at: usize,
}

/// An approval waiting for a button.
struct PendingApproval {
    reply: oneshot::Sender<Answer>,
    channel: String,
    message_ts: String,
    tool: String,
    thread_key: String,
    /// When the approver stops waiting. The card is retired at this point:
    /// one that outlives the call it describes is worse than no card, because
    /// pressing it records an approval of something already refused.
    expires_at: std::time::Instant,
}

/// One connector at a time, enforced rather than assumed.
///
/// Two `mecha slack connect` processes would both hold a socket, both answer
/// the same message, and both write the thread store — which is what the
/// store's no-per-record-lock rule quietly depended on not happening. The
/// kernel releases this if the process dies, so a hard kill leaves nothing to
/// clean up; that is the same reason the trigger store uses one.
struct ConnectorLock {
    _file: std::fs::File,
}

impl ConnectorLock {
    fn take(path: &std::path::Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        // SAFETY: flock on a descriptor we own, held open by the returned guard.
        let rc = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if rc != 0 {
            bail!(
                "another `mecha slack connect` is already running (lock held on {}). \
                 Two connectors would both answer the same message.",
                path.display()
            );
        }
        Ok(Self { _file: file })
    }
}

pub async fn run(global: &GlobalOpts) -> Result<()> {
    let store = SlackStore::open(mecha_core::work::mecha_home()?.join("slack"))?;
    // Taken before anything else opens a socket or touches the store.
    let _lock = ConnectorLock::take(&store.root().join("connector.lock"))?;
    let creds: Credentials = store
        .credentials()?
        .context("no Slack tokens stored — run `mecha slack auth` first")?;
    let binding: Binding = store
        .binding()?
        .context("nothing is bound — run `mecha slack link` first")?;

    let global_cfg = mecha_core::config::Config::load_global()?;
    // Resolved here rather than by `open_existing_default`, which always uses
    // the default root: a configured `[outbox] dir` would otherwise be
    // silently ignored and every review card would go missing with no error.
    let outbox_root = match global_cfg.outbox.dir.clone() {
        Some(dir) => dir,
        None => mecha_core::outbox::OutboxStore::default_root()?,
    };
    let cfg = global_cfg.slack;
    let threads = ThreadStore::open(
        mecha_core::work::mecha_home()?
            .join("slack")
            .join("threads"),
    )?;
    let slack = Slack::new(&creds.bot_token);

    // Who we are, so our own messages are never mistaken for input.
    let me: serde_json::Value = slack.call("auth.test", serde_json::json!({})).await?;
    let my_user_id = me["user_id"].as_str().unwrap_or_default().to_string();

    // **Before anything else.** A run lives in this process; if the last one
    // died, its threads are still showing "working…" and the person watching
    // has no way to know. Announced, not quietly reset.
    for orphan in threads.sweep()? {
        // Retire the controls *first*. An orphaned thread kept a live Stop
        // button for a run that no longer exists — pressing it would find
        // nothing and do nothing, which is the same lie the completion path
        // already fixed and worse here, because this is the one moment a
        // reader most needs to trust what the thread shows.
        if let Some(ts) = &orphan.controls_ts {
            let _ = chat::update(
                &slack,
                &orphan.channel_id,
                ts,
                "✗ Run lost to a restart",
                Some(vec![blocks::context("✗ Run lost to a restart")]),
            )
            .await;
        }
        let _ = chat::post_message(
            &slack,
            &orphan.channel_id,
            Some(&orphan.thread_ts),
            "This run did not survive a restart of the connector. Nothing was lost that \
             was written down — send another message to pick it up.",
            None,
        )
        .await;
        let _ = threads.apply(&orphan.key, Event::OrphanAnnounced);
    }

    let prepared = build_agent(global, &cfg).await?;
    let provider = prepared.provider_name.clone();
    let model = prepared.model.clone();
    let prepared_config = prepared.config.clone();
    let agent = Arc::new(prepared.agent);

    let (inbound_tx, mut inbound_rx) = mpsc::channel(64);
    let (approval_tx, mut approval_rx) = mpsc::channel::<approve::Request>(32);
    let (completion_tx, mut completion_rx) = mpsc::channel::<Completion>(32);

    let socket = SocketMode::new(
        slack.clone(),
        SocketOptions {
            app_token: creds.app_token.clone(),
            debug_reconnects: false,
        },
    );
    let socket_task = tokio::spawn(async move { socket.run(inbound_tx, || false).await });

    let mut state = State {
        slack,
        binding,
        threads,
        remote: crate::slack::remote::RemoteStore::open_default()?,
        cfg,
        agent,
        my_user_id,
        live: HashMap::new(),
        conversations: HashMap::new(),
        pending: HashMap::new(),
        seen: VecDeque::new(),
        approval_seq: 0,
        staged_before: HashMap::new(),
        files_before: HashMap::new(),
        outbox_root,
        ledger: Arc::new(ActionLedger::open_default()),
        review: HashMap::new(),
        provider,
        model,
        config: prepared_config,
        approval_tx,
        completion_tx,
    };

    // **Printed, not logged.** A daemon that says nothing at startup is
    // indistinguishable from one that has wedged — the confusion this design
    // keeps citing in other people's software. It has to be `println!` rather
    // than `tracing::info!` because tracing is filtered off by default, so
    // under systemd the journal showed a started unit and no evidence it was
    // working, and in a terminal it looked like a hang.
    println!(
        "Connected to {} as {}. {} owner(s), {} thread(s) known.",
        me["team"].as_str().unwrap_or("slack"),
        me["user"].as_str().unwrap_or("mecha"),
        state.binding.owners.len(),
        state.threads.all().map(|t| t.len()).unwrap_or(0),
    );
    println!("Waiting for a direct message. Ctrl-C or SIGTERM to stop.");

    loop {
        tokio::select! {
            inbound = inbound_rx.recv() => match inbound {
                Some(inbound) => state.on_inbound(inbound).await,
                None => break,
            },
            request = approval_rx.recv() => if let Some(r) = request {
                state.on_approval_request(r).await;
            },
            done = completion_rx.recv() => if let Some(c) = done {
                state.on_completion(c).await;
            },
            // **Both signals.** The shipped unit stops this with SIGTERM and
            // its comment claims in-flight runs stop at a safe point keeping
            // their partial turn; handling only SIGINT made that false, and an
            // ordinary `systemctl restart` killed runs outright.
            // Retire approval cards whose call has already been refused. Without
            // this a card raised at 2am stayed clickable, and pressing it in
            // the morning rewrote the message to say the owner approved
            // something that was refused hours earlier.
            _ = tokio::time::sleep(Duration::from_secs(15)) => {
                state.retire_expired_approvals().await;
            }
            _ = shutdown_signal() => {
                println!("Stopping; in-flight runs cancel at their next safe point.");
                for live in state.live.values() {
                    live.cancel.cancel();
                }
                break;
            }
        }
    }

    // **The socket's failure is this process's exit code.** Returning `Ok`
    // here meant a `link_disabled` — or any terminal Slack error — exited 0,
    // which `Restart=on-failure` reads as a clean stop: the unit sits
    // "successfully exited" and Slack goes unanswered forever with nothing in
    // the journal saying why.
    if socket_task.is_finished() {
        match socket_task.await {
            Ok(Err(e)) => return Err(anyhow::anyhow!("slack socket stopped: {e}")),
            Ok(Ok(())) => {}
            Err(e) if !e.is_cancelled() => {
                return Err(anyhow::anyhow!("slack socket task failed: {e}"))
            }
            Err(_) => {}
        }
    } else {
        socket_task.abort();
    }
    Ok(())
}

/// SIGINT or SIGTERM, whichever arrives first.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The agent every thread shares. One provider connection and one cached
/// prefix; the per-thread parts ride on `RunContext`.
async fn build_agent(
    global: &GlobalOpts,
    cfg: &mecha_core::config::SlackConfig,
) -> Result<setup::Prepared> {
    let opts = GlobalOpts {
        // Global config only, like a trigger run: a project's `mecha.toml`
        // arrives with a cloned repository and must not shape a run someone
        // drives from their phone.
        global_config_only: true,
        provider: global.provider.clone(),
        model: global.model.clone(),
        // Narrowed if config says so. The cost of not narrowing is paid every
        // turn of every thread, in the window the model has left to think in.
        tools: cfg.tools.clone(),
        // **Where the MCP servers are rooted**, which is not the same question
        // as where a run is jailed, and the difference bit on the first real
        // task. Servers are spawned once when this agent is built, so they
        // cannot follow a per-thread jail; without this they inherit the
        // connector's working directory, and a `bundle_render` then resolved
        // paths against the repo while `fs_write` wrote into the thread's
        // workspace. The model spent five turns discovering that and worked
        // around it with `shell`.
        //
        // Rooting them at the producer directory makes the two agree: every
        // thread's jail is a subdirectory of it. What it does **not** do is
        // give MCP tools per-thread isolation — they can reach any thread's
        // files, and only the built-in tools honour the jail. That is a real
        // limitation, written down rather than papered over; closing it means
        // an agent per thread, which means an MCP startup per thread.
        workspace: Some(producer_root()?),
        // No skills, for the reason `ask_user` is absent and `recall` is
        // absent: the registry belongs to the agent and one agent serves every
        // thread, so a `skill` shared across them is shared *state*. Loading
        // one in thread A would narrow thread B's tool surface and splice A's
        // procedure into B's compaction. Per-thread skills means an agent per
        // thread, which is the same trade as per-thread MCP isolation above.
        no_skills: true,
        ..GlobalOpts::default()
    };
    // Not interactive: no terminal approver, and no `ask_user` — the registry
    // belongs to the agent and one agent serves every thread, so a shared
    // `ask_user` could not know which thread asked. See SLACK-DESIGN.md §4.
    setup::prepare(&opts, false).await
}

struct State {
    slack: Slack,
    binding: Binding,
    threads: ThreadStore,
    /// Which threads mirror a terminal session. Held rather than opened per
    /// message so the check is a directory scan and nothing else — but *read*
    /// per message, never cached, because a session that attaches after this
    /// process starts has to be recognised without restarting it.
    remote: crate::slack::remote::RemoteStore,
    cfg: mecha_core::config::SlackConfig,
    agent: Arc<Agent>,
    my_user_id: String,
    live: HashMap<String, Live>,
    conversations: HashMap<String, Conversation>,
    pending: HashMap<String, PendingApproval>,
    seen: VecDeque<String>,
    approval_seq: u64,
    /// Pending outbox ids as they were when each thread's run began.
    staged_before: HashMap<String, std::collections::HashSet<String>>,
    /// The workspace as it stood when each run began.
    files_before: HashMap<String, HashMap<PathBuf, (u64, std::time::SystemTime)>>,
    outbox_root: std::path::PathBuf,
    /// Who tapped what and what became of it — the audit trail no store
    /// carries, shared by every spawned execution.
    ledger: Arc<ActionLedger>,
    /// Per-thread release policy, keyed by thread key. **In memory on
    /// purpose, never on the thread record**: the mode is session-scoped and
    /// expires with this process — the same eviction that orphans a
    /// mid-flight run clears every mode with it, and a restart resets every
    /// thread to carding everything. See `review.rs`.
    review: HashMap<String, review::Setting>,
    provider: String,
    model: String,
    config: mecha_core::config::Config,
    approval_tx: mpsc::Sender<approve::Request>,
    completion_tx: mpsc::Sender<Completion>,
}

impl State {
    async fn on_inbound(&mut self, inbound: Inbound) {
        match inbound {
            Inbound::Event { event, .. } => self.on_event(*event).await,
            Inbound::Interactive { interaction, .. } => self.on_interaction(*interaction).await,
            _ => {}
        }
    }

    /// The gate. Runs before a message can become a prompt, on every event.
    fn allowed(&self, user: Option<&str>, team: Option<&str>) -> Gate {
        binding::check(Some(&self.binding), user, team)
    }

    async fn on_event(&mut self, event: SlackEvent) {
        if event.kind != "message" || !event.is_from_a_human() {
            return;
        }
        // Our own replies arrive back as events; without this the loop feeds
        // itself.
        if event.user.as_deref() == Some(self.my_user_id.as_str()) {
            return;
        }
        // Slack's redelivery across a dropped socket is undocumented, so
        // assume it happens.
        if !self.first_time(&event.event_id) {
            return;
        }

        let gate = self.allowed(event.user.as_deref(), event.team_id.as_deref());
        if !gate.is_allowed() {
            // A log line, never a reply: telling a stranger why they were
            // ignored tells them an agent is listening.
            println!("ignored a message: {}", gate.reason());
            return;
        }

        let (Some(channel), Some(thread_ts), Some(text)) = (
            event.channel.clone(),
            event.thread_key(),
            event.text.clone(),
        ) else {
            return;
        };

        // The `doctor` command word, matched before the text can become a
        // prompt or a steering line — the same precedence slash commands get
        // in the TUI. Owner-tier only by construction: the gate above already
        // refused everyone else, and the report names stores, accounts and
        // stuck items, which is the user's private surface. The examination
        // is spawned work, never the ack path — it reads every store and
        // shells out to systemctl, and the three-second ack budget is
        // Slack's.
        if super::doctor::is_doctor_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let (text, blocks) = super::doctor::report().await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &text, blocks).await;
            });
            return;
        }

        // The `triggers` command word — the schedule with per-row controls,
        // on the `doctor` pattern: gated before the word is matched, store
        // read for display in spawned work, every button a typed action.
        if super::triggers::is_triggers_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let (text, blocks) = super::triggers::listing().await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &text, blocks).await;
            });
            return;
        }

        // The `note` command word — a deterministic capture into the
        // knowledge graph, matched before the text can become a prompt: a
        // capture that depends on a model's mood is not a capture. The gate
        // above proved the speaker is an owner, which is what makes the
        // note's provenance the owner's own words. Spawned: it starts an
        // MCP server, and the ack budget is Slack's.
        if let Some(body) = super::kg::note_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let reply = super::kg::capture(&body).await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &reply, None).await;
            });
            return;
        }

        // The `queues` command word — the review backlog rollup, read-only,
        // on the `doctor` pattern: gated before the word is matched, the
        // stores read in spawned work.
        if super::kg::is_queues_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let reply = super::kg::queues_report().await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &reply, None).await;
            });
            return;
        }

        // The `tasks` command word — the GTD board with per-row controls, on
        // the `triggers` pattern: gated before the word is matched, the board
        // read in spawned work (the child starts an MCP server to reach the
        // graph), every button a typed action carrying only a task id.
        if super::tasks::is_tasks_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let (text, blocks) = super::tasks::board().await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &text, blocks).await;
            });
            return;
        }

        // The `task` command word — a deterministic capture onto the board,
        // the `note` rule one store over: matched before the text can become
        // a prompt, because a capture that depends on a model's mood is not a
        // capture. Owner-tier by the gate above; spawned for the same MCP
        // startup reason as the board.
        if let Some(body) = super::tasks::task_command(&text) {
            let slack = self.slack.clone();
            let (channel, thread_ts) = (channel.clone(), thread_ts.clone());
            tokio::spawn(async move {
                let reply = super::tasks::capture(&body).await;
                let _ = chat::post_message(&slack, &channel, Some(&thread_ts), &reply, None).await;
            });
            return;
        }

        // The `review` command word — the explicit owner gesture, and the
        // only thing that can set a thread's release policy. Matched with the
        // same precedence as `doctor`, before the text can become a prompt or
        // a steering line, so the model never sees it and nothing the model
        // says can be it: release policy must not be decidable by anything
        // sharing a context window with third-party text (the `/review`
        // rule). The gate above already proved the speaker is an owner.
        if let Some(asked) = review::command(&text) {
            let reply = match asked {
                Some(mode) => {
                    // Scope follows where the word was spoken (F8): inside a
                    // thread it governs that thread; as a top-level message it
                    // governs the channel's subsequent top-level prompts —
                    // keyed to its own message's ts it would confirm a policy
                    // no later message ever inherits. The raw `thread_ts`
                    // (never the fallen-back thread key) is what tells the
                    // two apart, and the confirmation names the real scope.
                    let (key, scope) = review::scope_for(&channel, event.thread_ts.as_deref());
                    let who = event.user.clone().unwrap_or_default();
                    self.review
                        .insert(key, review::Setting { mode, set_by: who });
                    format!(
                        "Review mode for {scope} is now `{}` — {}. Tainted drafts \
                         always stop for review, and the mode lasts only while the \
                         connector runs.",
                        mode.name(),
                        mode.describe()
                    )
                }
                None => {
                    let key = super::threads::key_for(&channel, &thread_ts);
                    let mode = review::effective(&self.review, &key, &channel)
                        .map(|s| s.mode)
                        .unwrap_or(ReviewMode::Now);
                    format!(
                        "Review mode is `{}` — {}. Set it with `review now|later|auto`.",
                        mode.name(),
                        mode.describe()
                    )
                }
            };
            let _ = chat::post_message(&self.slack, &channel, Some(&thread_ts), &reply, None).await;
            return;
        }

        // **A thread that mirrors a terminal session is not this process's to
        // run.** Without this the next line would mint a thread record, start
        // a fresh `Conversation` in a different workspace under a different
        // permission mode, and answer — looking exactly like the mirror
        // replying while knowing nothing about the conversation it appears to
        // be part of. Not a leak, since that conversation is clean, taint and
        // all; a stranger wearing the thread's clothes, which reads worse and
        // is just as wrong to act on.
        match self.remote.attached_thread(&channel, &thread_ts) {
            Ok(Some(rec)) if rec.is_live() => {
                let name = rec.name.clone();
                // Attachments are downloaded *here*, by the process holding
                // the token, and staged in a directory this process owns —
                // never written straight into the session's workspace, which
                // may be a real project directory. The owning process moves
                // them in when it claims the line.
                let mut staged = Vec::new();
                for file in &event.files {
                    let raw = file.name.as_deref().unwrap_or(&file.id);
                    match mecha_slack::files::download(
                        &self.slack,
                        file,
                        self.cfg.max_upload_mb.saturating_mul(1024 * 1024),
                    )
                    .await
                    {
                        Ok(bytes) => match self.remote.stage_file(&name, raw, &bytes) {
                            Ok(stored) => staged.push(stored),
                            Err(e) => {
                                tracing::warn!("could not stage {raw}: {e:#}");
                                let _ = chat::post_message(
                                    &self.slack,
                                    &channel,
                                    Some(&thread_ts),
                                    &format!("Could not save `{raw}`: {e:#}"),
                                    None,
                                )
                                .await;
                            }
                        },
                        // Said out loud. A screenshot that silently did not
                        // arrive is indistinguishable from one the session
                        // chose to ignore, and the person is on a phone with
                        // no way to tell which.
                        Err(e) => {
                            tracing::warn!("could not fetch {}: {e:#}", file.id);
                            let _ = chat::post_message(
                                &self.slack,
                                &channel,
                                Some(&thread_ts),
                                &format!("Could not fetch `{raw}`: {e:#}"),
                                None,
                            )
                            .await;
                        }
                    }
                }
                // Nothing to hand on: a message with no text and no file that
                // survived is not something to wake a session for.
                if text.trim().is_empty() && staged.is_empty() {
                    return;
                }
                if let Err(e) = self.remote.push_inbound(&name, &text, staged) {
                    // Fail loudly. A line that silently went nowhere is
                    // indistinguishable from a session ignoring you, and the
                    // person is on a phone with no way to tell which.
                    let _ = chat::post_message(
                        &self.slack,
                        &channel,
                        Some(&thread_ts),
                        &format!("Could not hand that to `{name}`: {e:#}"),
                        None,
                    )
                    .await;
                }
                return;
            }
            // The session that owned this thread has gone. Running something
            // unrelated in its scrollback is what the branch above exists to
            // prevent, so say what happened instead of quietly becoming a
            // different feature.
            Ok(Some(rec)) => {
                let ended = rec
                    .ended_reason
                    .clone()
                    .unwrap_or_else(|| "the session ended".to_string());
                let _ = chat::post_message(
                    &self.slack,
                    &channel,
                    Some(&thread_ts),
                    &format!(
                        "`{}` is not attached to a live session — {ended}. Run \
                         `/remote-control {}` in a terminal to pick this thread up again.",
                        rec.name, rec.name
                    ),
                    None,
                )
                .await;
                return;
            }
            Ok(None) => {}
            // An unreadable store must not silently downgrade into "not
            // attached", because that is the branch that starts a run.
            // Refusing costs one message; guessing costs the confusion above.
            Err(e) => {
                tracing::warn!("could not read the remote store: {e:#}");
                let _ = chat::post_message(
                    &self.slack,
                    &channel,
                    Some(&thread_ts),
                    "Could not tell whether this thread mirrors a terminal session, so \
                     nothing was started.",
                    None,
                )
                .await;
                return;
            }
        }

        let record = match self
            .threads
            .ensure(&channel, &thread_ts, &self.cfg.default_mode)
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("could not open the thread record: {e}");
                return;
            }
        };

        // Steering: the run keeps going and folds the text in at its next turn.
        if let Some(live) = self.live.get(&record.key) {
            if let Ok(mut queue) = live.queue.lock() {
                queue.push_back(text);
            }
            // The user spoke. If an earlier card timed out, this run's
            // approver latched and was refusing every gated call without
            // asking — and because this branch short-circuits the message
            // into the queue, no rebuild would ever clear it. A reply is
            // proof someone is watching, so the next gated call posts a
            // fresh card and waits normally.
            live.unanswered.store(false, Ordering::Relaxed);
            return;
        }

        if self.live.len() >= self.cfg.max_concurrent {
            // An honest refusal beats a run that starts twenty minutes later
            // against a workspace that has moved.
            let _ = chat::post_message(
                &self.slack,
                &channel,
                Some(&thread_ts),
                &format!(
                    "Already running {} threads, which is the configured limit. \
                     Send this again when one finishes.",
                    self.cfg.max_concurrent
                ),
                None,
            )
            .await;
            return;
        }

        self.start_run(record, text, event.files).await;
    }

    fn first_time(&mut self, event_id: &str) -> bool {
        if event_id.is_empty() {
            return true;
        }
        if self.seen.iter().any(|s| s == event_id) {
            return false;
        }
        self.seen.push_back(event_id.to_string());
        if self.seen.len() > SEEN_EVENTS {
            self.seen.pop_front();
        }
        true
    }

    async fn start_run(&mut self, record: ThreadRecord, prompt: String, files: Vec<FileRef>) {
        let key = record.key.clone();
        let channel = record.channel_id.clone();
        let thread_ts = record.thread_ts.clone();

        let workspace = match thread_workspace(&key) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("no workspace for {key}: {e}");
                return;
            }
        };

        let mode = Arc::new(Mutex::new(Mode::parse(&record.mode).unwrap_or(Mode::Ask)));
        let cancel = CancellationToken::new();
        let queue = Arc::new(Mutex::new(VecDeque::new()));

        // The per-thread half of the run. The approver rides here rather than
        // on the agent because one agent serves every thread, and widening it
        // on the agent would widen all of them at once.
        // **A session per thread.** Every other front-end records one, and
        // without it a Slack run left no transcript at all: invisible to
        // `mecha sessions`, unmineable by `reflect`, never distilled, and —
        // the load-bearing one — staging drafts that carry no session id, so
        // nothing downstream can say which run produced them.
        let session = match self.session_for(&record).await {
            Some(s) => s,
            None => return,
        };

        let mut cx = (**self.agent.context()).clone();
        cx.tools = Arc::new(ToolCtx {
            workspace,
            ..(*self.agent.ctx()).clone()
        });
        let approver = SlackApprover::new(
            key.clone(),
            Arc::clone(&mode),
            self.approval_tx.clone(),
            Duration::from_secs(self.cfg.approval_timeout_secs),
        );
        // Kept beside the steering queue in `Live`: the approver survives for
        // the whole run, so the loop needs its own handle to clear the
        // unanswered latch when the user shows up again.
        let unanswered = approver.unanswered_latch();
        cx.approver = Arc::new(approver);
        cx.budget = Budget {
            max_turns: Some(self.cfg.max_turns),
            max_cost_usd: self.cfg.max_cost_usd,
            ..Budget::default()
        };
        cx.cancel = Some(cancel.clone());
        cx.queued_input = Some(Arc::clone(&queue));
        // **Its own outbox route, carrying its own session id.** The agent's
        // route is one `Arc` shared by every thread, so stamping a session id
        // on it would race between concurrent runs — and it is the stamp that
        // lets a draft be attributed to the run that wrote it.
        if let Some(shared) = &self.agent.context().outbox {
            let Ok(store) = mecha_core::outbox::OutboxStore::open(&self.outbox_root) else {
                return;
            };
            let mine = OutboxRoute::new(
                store,
                shared.routed().map(String::from).collect::<Vec<_>>(),
                shared.publishes().map(String::from).collect::<Vec<_>>(),
            );
            mine.set_session_id(&session.meta.id);
            cx.outbox = Some(Arc::new(mine));
        }

        // The system prompt belongs to the agent and one agent serves every
        // thread, so "where am I" cannot live there. It rides on the turn
        // instead — and it earns its tokens: without it the first real task
        // spent five turns working out that its tools disagreed about paths.
        let mut prompt = format!(
            "[This thread's workspace is {}. Relative paths resolve there.]\n\n{prompt}",
            cx.tools.workspace.display()
        );
        let mut attached_images = Vec::new();
        if !files.is_empty() {
            let (landed, images) = self
                .fetch_attachments(&files, &cx.tools.workspace, self.agent.vision())
                .await;
            attached_images = images;
            if !landed.is_empty() {
                // Named as paths rather than injected as content: the agent
                // reaches the bytes with `fs_read`, which already declares
                // `private_data`, so the taint legs arm through the path that
                // already exists instead of a parallel one. It is also what
                // Claude Code's mobile app does with an attachment.
                prompt.push_str("\n\nThe user attached:\n");
                for path in &landed {
                    prompt.push_str(&format!("- {path}\n"));
                }
            }
        }

        // Snapshot the outbox before the run. Scoping by id-diff is what the
        // TUI does and for the same reason: nothing else reliably says which
        // drafts *this* run staged, and releasing another session's drafts
        // from a phone would be the worst possible surprise.
        let staged_before = pending_outbox_ids(&self.outbox_root);
        // What the workspace held before the run, so what it *made* can be
        // told from what was already there — the same id-diff reasoning the
        // outbox scoping uses, applied to files.
        let files_before = workspace_snapshot(&cx.tools.workspace);

        let mut conversation = self.conversations.remove(&key).unwrap_or_default();
        // Text first, images after — the order both provider families
        // document and the one `encode_message` preserves. A model that meets
        // the pixels before the question has nothing to attend to.
        let mut content = vec![Block::text(&prompt)];
        content.extend(attached_images);
        conversation.messages.push(Message {
            role: mecha_core::message::Role::User,
            content,
        });

        // The spawned task takes ownership of these; the controls message is
        // posted after it starts, so it needs its own copies.
        let controls_channel = channel.clone();
        let controls_thread_ts = thread_ts.clone();

        let (events_tx, events_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let agent = Arc::clone(&self.agent);
        let slack = self.slack.clone();
        let completion_tx = self.completion_tx.clone();
        let pump_cfg = PumpConfig {
            flush_chars: self.cfg.stream_flush_chars,
            flush_ms: self.cfg.stream_flush_ms,
        };
        let key_for_task = key.clone();
        let session_for_task = session;
        self.staged_before.insert(key.clone(), staged_before);
        self.files_before.insert(key.clone(), files_before);

        tokio::spawn(async move {
            let renderer = {
                let slack = slack.clone();
                let channel = channel.clone();
                let thread_ts = thread_ts.clone();
                tokio::spawn(async move {
                    pump(&slack, &channel, &thread_ts, events_rx, &pump_cfg).await
                })
            };

            let before = conversation.messages.clone();
            let run_started_at = before.len();
            let outcome = agent
                .run_in(&cx, &mut conversation, Some(events_tx))
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = renderer.await;
            // Recorded whether it succeeded or not: a failed run is exactly
            // the transcript someone wants afterwards.
            let _ = session_for_task.record_run(&before, &conversation);
            // Only on success: an errored run has no outcome to describe, and
            // the transcript above is the part worth keeping either way.
            if let Ok(o) = &outcome {
                let _ = session_for_task.record_outcome(o);
            }

            let _ = completion_tx
                .send(Completion {
                    key: key_for_task,
                    conversation,
                    outcome,
                    run_started_at,
                })
                .await;
        });

        let mode_label = mode.lock().map(|m| m.as_str()).unwrap_or("ask");
        let controls = chat::post_message(
            &self.slack,
            &controls_channel,
            Some(&controls_thread_ts),
            "Working…",
            Some(controls_blocks(&key, mode_label)),
        )
        .await;

        println!("[{key}] run started");
        self.live.insert(
            key.clone(),
            Live {
                cancel,
                queue,
                mode,
                unanswered,
            },
        );
        let _ = self.threads.apply(&key, Event::OwnerSpoke);
        if let Ok(Some(mut r)) = self.threads.get(&key) {
            r.run = Some(RunMarker::here());
            r.controls_ts = controls.ok();
            let _ = self.threads.put(&r);
        }
    }

    /// Download what the owner attached into the run's own jail.
    ///
    /// Every guard that matters lives in `mecha_slack::files::download` — the
    /// bearer header, no redirects, `text/html` refused even at HTTP 200, the
    /// byte count checked — because a Slack sign-in page reaching the model
    /// labelled as the user's screenshot is a silent failure. What is added
    /// here is the filename: Slack supplies it, and a name that reaches the
    /// filesystem is a path.
    async fn fetch_attachments(
        &self,
        files: &[FileRef],
        workspace: &std::path::Path,
        vision: bool,
    ) -> (Vec<String>, Vec<Block>) {
        let inbox = workspace.join("inbox");
        if let Err(e) = std::fs::create_dir_all(&inbox) {
            tracing::warn!("could not make an inbox directory: {e}");
            return (Vec::new(), Vec::new());
        }
        let mut landed = Vec::new();
        let mut images = Vec::new();
        for file in files {
            let name = safe_filename(file.name.as_deref().unwrap_or(&file.id));
            match mecha_slack::files::download(
                &self.slack,
                file,
                self.cfg.max_upload_mb.saturating_mul(1024 * 1024),
            )
            .await
            {
                Ok(bytes) => {
                    let path = inbox.join(&name);
                    match std::fs::write(&path, &bytes) {
                        Ok(()) => {
                            landed.push(format!("./inbox/{name}"));
                            // **The file lands either way, and the pixels are
                            // extra.** Writing it to the workspace is the
                            // durable half — it is what `fs_read`, `shell` and
                            // a later run reach, and it is what the design
                            // called a conduit. Putting the image *into the
                            // turn* is what makes it a feature, and it is
                            // conditional on there being eyes to see it: a
                            // text-only model would be handed a resized JPEG
                            // to render as its own filename.
                            if vision {
                                match mecha_core::image::block_from_path(&path) {
                                    Ok(Some(block)) => images.push(block),
                                    Ok(None) => {}
                                    // Named, never silent, and never fatal to
                                    // the attachment: the bytes are already on
                                    // disk and the model can still be told
                                    // where. An image too large to send is a
                                    // thing the person should hear about,
                                    // because from their side they sent one.
                                    Err(e) => {
                                        tracing::warn!("could not put {name} in the turn: {e}");
                                        landed.push(format!(
                                            "  (too large to look at directly: {e})"
                                        ));
                                    }
                                }
                            }
                        }
                        Err(e) => tracing::warn!("could not save {name}: {e}"),
                    }
                }
                Err(e) => tracing::warn!("could not fetch {}: {e}", file.id),
            }
        }
        (landed, images)
    }

    async fn on_completion(&mut self, done: Completion) {
        match &done.outcome {
            Ok(o) => println!("[{}] run finished — {} turns", done.key, o.turns),
            Err(e) => println!("[{}] run failed — {e}", done.key),
        }
        // §6.2's readout on this surface, computed while the conversation
        // is still in hand: a number in the thread, by the owner's ruling
        // (`docs/APPRAISAL-RESEARCH.md` §3.1). The word rides along only
        // when the label says one. Silent runs post nothing.
        let readout = done.outcome.as_ref().ok().map(|o| {
            mecha_core::appraisal::live_readout(
                &done.key,
                o,
                &done.conversation,
                done.run_started_at,
            )
        });
        self.live.remove(&done.key);
        self.conversations
            .insert(done.key.clone(), done.conversation);

        // The controls become a record rather than staying clickable: a Stop
        // button for a run that already ended is a lie the reader has to test.
        if let Ok(Some(r)) = self.threads.get(&done.key) {
            if let Some(ts) = &r.controls_ts {
                // Worded as a header, because it sits above the answer: the
                // controls must be posted before the stream exists so Stop is
                // pressable from the first moment, which puts them first in
                // the thread. "Finished." there read like a footer that had
                // wandered up the page.
                let ended = if done.outcome.is_ok() {
                    "✓ Run complete"
                } else {
                    "✗ Run failed"
                };
                let _ = chat::update(
                    &self.slack,
                    &r.channel_id,
                    ts,
                    ended,
                    Some(vec![blocks::context(&format!(
                        "{ended} · mode `{}`",
                        r.mode
                    ))]),
                )
                .await;
            }
            if let Some(readout) = readout.filter(|r| !r.is_silent()) {
                let line = appraisal_line(&readout);
                let _ = chat::post_message(
                    &self.slack,
                    &r.channel_id,
                    Some(&r.thread_ts),
                    &line,
                    Some(vec![blocks::context(&line)]),
                )
                .await;
            }
        }

        match done.outcome {
            Ok(outcome) => {
                // A run that staged drafts is not finished from the person's
                // side, and the state says so.
                let staged = outcome.tool_calls.iter().any(|c| c.staged);
                // Whether the run said everything it meant to. `review auto`
                // releases nothing after an errored or early-stopped run — a
                // cancelled run's drafts are half a thought — but its
                // untainted drafts still card below, so review stays
                // possible from the phone. The rule itself lives in
                // `review_policy::auto_releases` (F1/F10).
                let finished_clean = !outcome.stop_cause.is_early();
                let _ = self.threads.apply(&done.key, Event::Finished { staged });
                self.post_artifacts(&done.key).await;
                if staged {
                    self.offer_drafts(&done.key, finished_clean).await;
                }
            }
            Err(error) => {
                let _ = self.threads.apply(&done.key, Event::Errored);
                // Posted, never only logged: a failure the person cannot see
                // is indistinguishable from a run that is still thinking.
                // The breadcrumb is owner-tier by construction — the gate
                // lets nobody else start a run, so every thread this posts
                // into is an owner's.
                if let Ok(Some(r)) = self.threads.get(&done.key) {
                    let _ = chat::post_message(
                        &self.slack,
                        &r.channel_id,
                        Some(&r.thread_ts),
                        &format!("The run failed: {error} — send `doctor` for a health report"),
                        None,
                    )
                    .await;
                }
            }
        }
    }

    /// Widen or narrow one thread, and only that thread.
    ///
    /// Set by a button and never inferred from prompt text — permission policy
    /// must not be decidable by anything sharing a context window with
    /// third-party text, which is the rule `/review` already follows. The live
    /// cell is updated as well as the record, so a change lands on the run's
    /// **next** call rather than its next run.
    async fn cycle_mode(&mut self, key: &str) {
        let Ok(Some(mut record)) = self.threads.get(key) else {
            return;
        };
        let next = match Mode::parse(&record.mode).unwrap_or(Mode::Ask) {
            Mode::Ask => Mode::Allow,
            Mode::Allow => Mode::ReadOnly,
            Mode::ReadOnly => Mode::Ask,
        };
        record.mode = next.as_str().to_string();
        let _ = self.threads.put(&record);
        if let Some(live) = self.live.get(key) {
            if let Ok(mut m) = live.mode.lock() {
                *m = next;
            }
        }
        if let Some(ts) = &record.controls_ts {
            let _ = chat::update(
                &self.slack,
                &record.channel_id,
                ts,
                "Working…",
                Some(controls_blocks(key, next.as_str())),
            )
            .await;
        }
    }

    /// A session for this run.
    ///
    /// One per *run* rather than per thread, which is what a trigger does: the
    /// store has no re-attach constructor, and inventing one to make a
    /// thread's runs share a transcript would be a change to `session.rs` in
    /// service of a nicety. The thread record keeps the most recent id, so
    /// "what did that thread last do" is still answerable.
    async fn session_for(&mut self, record: &ThreadRecord) -> Option<Session> {
        let dir = Session::default_dir().ok()?;
        let session = Session::create(
            &dir,
            SessionMeta {
                id: Session::new_id(),
                created_at: chrono::Utc::now(),
                provider: self.provider.clone(),
                model: self.model.clone(),
                workspace: record.workspace.clone().unwrap_or_default(),
                title: Some(format!("slack: {}", record.key)),
                kind: Some(mecha_core::session::SessionKind::Slack),
            },
        )
        .ok()?;
        let _ = session.append(&Record::Config(mecha_core::session::RunConfig::of(
            &self.agent,
            &self.config,
            &self.provider,
        )));
        if let Ok(Some(mut r)) = self.threads.get(&record.key) {
            r.session_id = Some(session.meta.id.clone());
            let _ = self.threads.put(&r);
        }
        Some(session)
    }

    /// Send back what the run actually made.
    ///
    /// An answer that says "I wrote the chart to output.png" is useless on a
    /// phone, where there is no filesystem to go and look at. Files the run
    /// created or changed are uploaded into the thread — **privately, naming
    /// no channel at the upload step**, so nothing is shared until the
    /// completion call attaches it here.
    ///
    /// Bounded on purpose: a run that rewrites forty files should not post
    /// forty attachments, and anything past the cap is named rather than sent.
    /// Silence about what was skipped would be the same failure as a dropped
    /// Block Kit block.
    async fn post_artifacts(&mut self, key: &str) {
        const MAX_FILES: usize = 5;

        let Ok(Some(record)) = self.threads.get(key) else {
            return;
        };
        let before = self.files_before.remove(key).unwrap_or_default();
        let Some(workspace) = record
            .workspace
            .clone()
            .or_else(|| thread_workspace(key).ok())
        else {
            return;
        };
        let after = workspace_snapshot(&workspace);
        let mut changed: Vec<PathBuf> = after
            .iter()
            .filter(|(path, stamp)| before.get(*path).is_none_or(|old| old != *stamp))
            .map(|(path, _)| path.clone())
            .collect();
        if changed.is_empty() {
            return;
        }
        changed.sort();

        let limit = self.cfg.max_upload_mb.saturating_mul(1024 * 1024);
        let (send, skipped): (Vec<_>, Vec<_>) = changed.iter().partition(|p| {
            std::fs::metadata(p)
                .map(|m| m.len() <= limit)
                .unwrap_or(false)
        });

        let mut named = Vec::new();
        for path in send.iter().take(MAX_FILES) {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "artifact".into());
            let share = mecha_slack::files::Share {
                channel_id: Some(&record.channel_id),
                thread_ts: Some(&record.thread_ts),
                title: Some(&name),
                ..Default::default()
            };
            if let Err(e) = mecha_slack::files::upload(&self.slack, &name, &bytes, &share).await {
                tracing::warn!("could not upload {name}: {e}");
            } else {
                named.push(name);
            }
        }

        let over = send.len().saturating_sub(MAX_FILES);
        if over > 0 || !skipped.is_empty() {
            let mut note = String::new();
            if over > 0 {
                note.push_str(&format!("{over} more file(s) changed and were not sent. "));
            }
            if !skipped.is_empty() {
                note.push_str(&format!(
                    "{} file(s) are over the {} MB limit: {}.",
                    skipped.len(),
                    self.cfg.max_upload_mb,
                    skipped
                        .iter()
                        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let _ = chat::post_message(
                &self.slack,
                &record.channel_id,
                Some(&record.thread_ts),
                note.trim(),
                None,
            )
            .await;
        }
        if !named.is_empty() {
            println!("[{key}] posted {} artifact(s)", named.len());
        }
    }

    /// Post one card per draft this run staged, so releasing it never means
    /// finding a terminal.
    ///
    /// This is the phone UI `PUBLIC-SURFACE-DESIGN.md` §11 deferred as needing
    /// a home-side server. It needs none: the outbox is already the review
    /// surface, and Slack is already a screen the owner is holding.
    async fn offer_drafts(&mut self, key: &str, finished_clean: bool) {
        let Ok(Some(record)) = self.threads.get(key) else {
            return;
        };
        // **Scoped by session id, not by an outbox-wide diff.** The diff was
        // borrowed from the TUI, where it is safe because the TUI has one
        // conversation; here up to `max_concurrent` runs share the outbox with
        // every other mecha process, so a nightly trigger staging nine replies
        // mid-run would have had them carded in this thread and released from
        // it. The session id says which run actually wrote a draft.
        let Some(session_id) = record.session_id.clone() else {
            return;
        };
        let before = self.staged_before.remove(key).unwrap_or_default();
        let Ok(store) = mecha_core::outbox::OutboxStore::open(&self.outbox_root) else {
            return;
        };
        let Ok(items) = store.items() else { return };
        let fresh: Vec<_> = items
            .into_iter()
            .filter(|i| {
                i.status == "pending"
                    && i.session_id.as_deref() == Some(session_id.as_str())
                    && !before.contains(&i.id)
            })
            .collect();
        if fresh.is_empty() {
            return;
        }

        // The thread's release policy — its own, else its channel's (F8).
        // Default is `now` — card everything — and the setting only exists
        // while this process does (see `review.rs`). This point is reached
        // after early-stopped runs too: their drafts card, they just never
        // auto-release, and `finished_clean` below is what carries that.
        let setting = review::effective(&self.review, key, &record.channel_id).cloned();
        let mode = setting.as_ref().map(|s| s.mode).unwrap_or(ReviewMode::Now);

        if mode == ReviewMode::Later {
            let _ = chat::post_message(
                &self.slack,
                &record.channel_id,
                Some(&record.thread_ts),
                &format!(
                    "{} draft(s) staged — waiting in the outbox (`mecha outbox review`, \
                     or send `review now` and re-run).",
                    fresh.len()
                ),
                None,
            )
            .await;
            return;
        }

        // Auto mode after an early stop: say why nothing releases, once,
        // before the cards — the same notice the TUI posts.
        if mode == ReviewMode::Auto && !finished_clean {
            let _ = chat::post_message(
                &self.slack,
                &record.channel_id,
                Some(&record.thread_ts),
                "The run stopped early — nothing auto-releases. Its drafts are \
                 carded below for review.",
                None,
            )
            .await;
        }

        for item in fresh {
            // The taint snapshot rides on the item, and a draft written while
            // the trifecta was armed is the one a person must look at hardest.
            let armed = item.taint.private && item.taint.untrusted;
            // `review auto`, owner-set: untainted drafts this thread's run
            // staged release without a card. The whole decision — the tainted
            // exclusion (a mode set before the hostile page was fetched
            // authorises nothing about what came after) *and* the
            // early-stop exclusion — is `review_policy::auto_releases`'s,
            // shared with the TUI so neither surface can forget half of it.
            // The ledger row attributes the release to the mode and the
            // owner who set it, so "who released this" stays answerable.
            if releases_without_card(setting.as_ref(), armed, finished_clean) {
                let setter = setting
                    .as_ref()
                    .map(|s| s.set_by.clone())
                    .unwrap_or_default();
                self.dispatch_action(
                    Action::OutboxSend {
                        id: item.id.clone(),
                    },
                    &setter,
                    "review-auto",
                    ActionCard::Reply {
                        channel: record.channel_id.clone(),
                        thread_ts: record.thread_ts.clone(),
                    },
                );
                continue;
            }
            // No map of where the card went: a button press carries its own
            // channel and message ts, which is what makes a card still work
            // after the connector restarts.
            let _ = chat::post_message(
                &self.slack,
                &record.channel_id,
                Some(&record.thread_ts),
                "A draft is waiting for review.",
                Some(draft_offer_blocks(&item)),
            )
            .await;
        }
    }

    /// One draft, re-read from the store — the state every press-time
    /// decision runs against. An exact single-file read on the blocking pool,
    /// never `items()`'s scan of the whole outbox on the event-dispatch loop:
    /// this loop serves every thread's events, and a directory of drafts is
    /// as slow as its history is long. Button values carry full store-minted
    /// ids, and `item_exact` refuses a hostile shape before it touches the
    /// filesystem.
    async fn outbox_item(&self, id: &str) -> Option<mecha_core::outbox::OutboxItem> {
        let root = self.outbox_root.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            mecha_core::outbox::OutboxStore::open(&root)
                .ok()?
                .item_exact(&id)
                .ok()
                .flatten()
        })
        .await
        .ok()
        .flatten()
    }

    /// The second step for a tainted draft: the full arguments, and a button
    /// that means what it says. `item` is the read the caller already made
    /// for the taint decision — reusing it is what keeps one press one read.
    ///
    /// The TUI shows the whole call in red and confirms before releasing one
    /// of these, and refuses to auto-release them at all. A single
    /// primary-styled Send that ran `outbox send -y` gave a phone less care
    /// than a terminal, on the drafts that deserve the most. The card is a
    /// pure function (`tainted_confirm_card`) because its two rules have
    /// tests: no Send on a draft the surface could not fully show, and no
    /// Send on a draft the store could not read at all (F3) — a transient
    /// store failure used to unwrap to empty arguments and offer a live Send
    /// over zero shown bytes.
    async fn ask_to_confirm_tainted(
        &mut self,
        id: &str,
        item: Option<&mecha_core::outbox::OutboxItem>,
        channel: &str,
        ts: &str,
    ) {
        let (text, card) = tainted_confirm_card(id, item);
        let _ = chat::update(&self.slack, channel, ts, &text, Some(card)).await;
    }

    /// Dispatch one action: a ledger row, an eagerly retired control, the
    /// execution in spawned work, and the outcome read back from the store —
    /// never from the child's exit alone.
    ///
    /// A child process rather than a library call, for the TUI's reasons: one
    /// implementation of each verb, no way for this surface to do something
    /// the command line cannot, and every store guard (the send flock, the
    /// trigger flock, the pending checks) inherited rather than reimplemented.
    fn dispatch_action(
        &mut self,
        action: Action,
        who: &str,
        surface: &'static str,
        target: ActionCard,
    ) {
        let tap_id = actions::new_tap_id();
        let slack = self.slack.clone();
        let ledger = Arc::clone(&self.ledger);
        let executor = Executor {
            outbox_root: self.outbox_root.clone(),
        };
        let who = who.to_string();
        tokio::spawn(async move {
            // The dispatch row lands first, before anything runs: a crash
            // between here and the outcome row is exactly when the record
            // matters. Inside the spawned task rather than on the dispatch
            // loop — the append is blocking fs, and the loop it sat on
            // serves every thread's events.
            ledger.dispatched(&tap_id, &who, &action, surface);
            let pending = format!("⏳ {} — <@{}>", action.describe(), who);
            // Retire the control before the child runs. For a one-card
            // message that is the card itself; a multi-finding doctor report
            // cannot be rewritten without destroying the rest of it, so the
            // dispatch is announced as a reply and the store-state guards
            // (the restart's re-examination, the flocks, the pending checks)
            // are the defence — which §5 requires anyway: the card rewrite
            // is never the *only* one.
            //
            // `card_ts` is the message the outcome can edit; `reply_ts` is
            // where a fresh, notifying reply belongs when one is owed.
            let (channel, card_ts, reply_ts) = match &target {
                ActionCard::Rewrite {
                    channel,
                    ts,
                    thread_ts,
                } => {
                    let _ = chat::update(
                        &slack,
                        channel,
                        ts,
                        &pending,
                        Some(vec![blocks::context(&pending)]),
                    )
                    .await;
                    (
                        channel.clone(),
                        Some(ts.clone()),
                        thread_ts.clone().unwrap_or_else(|| ts.clone()),
                    )
                }
                ActionCard::Reply { channel, thread_ts } => {
                    let ts = chat::post_message(&slack, channel, Some(thread_ts), &pending, None)
                        .await
                        .ok();
                    (channel.clone(), ts, thread_ts.clone())
                }
            };

            let started = std::time::Instant::now();
            let outcome = executor.run(&action).await;
            ledger.resolved(&tap_id, &outcome.status, &outcome.line);

            let line = format!("{} · <@{}>", outcome.line, who);
            // One delivery rule for both targets (F12): the dispatch message
            // is updated when it exists, and a fresh reply lands when the
            // dispatch never reached the thread — the outcome must land
            // somewhere — or when the action ran long enough that a silent
            // card edit would go unseen (a card edit fires no notification,
            // and a person who tapped twenty minutes ago has stopped
            // watching; a slow outbox release with an MCP startup is as slow
            // as a trigger run). Never both fresh posts: the old shape
            // double-posted a slow action whose dispatch reply had failed.
            let (update_card, fresh_reply) = outcome_delivery(
                card_ts.is_some(),
                started.elapsed() > Duration::from_secs(60),
            );
            if update_card {
                if let Some(ts) = &card_ts {
                    let _ = chat::update(
                        &slack,
                        &channel,
                        ts,
                        &line,
                        Some(vec![blocks::context(&line)]),
                    )
                    .await;
                }
            }
            if fresh_reply {
                let _ = chat::post_message(&slack, &channel, Some(&reply_ts), &line, None).await;
            }

            // F13: a failed auto-release must leave the phone a path back.
            // Nobody was shown a card before the release was attempted, so a
            // draft that stayed pending would otherwise sit invisible until
            // someone found a terminal — post the card the `now` mode would
            // have posted, failure already reported above.
            if let Action::OutboxSend { id } = &action {
                let item = executor.item(id).await;
                if failed_auto_release_needs_card(surface, item.as_ref().map(|i| i.status.as_str()))
                {
                    let item = item.expect("checked by the predicate");
                    let _ = chat::post_message(
                        &slack,
                        &channel,
                        Some(&reply_ts),
                        "The auto-release failed and the draft is still pending — review it \
                         here.",
                        Some(draft_offer_blocks(&item)),
                    )
                    .await;
                }
            }
        });
    }

    /// Rewrite any card whose approver has stopped waiting, and forget it.
    async fn retire_expired_approvals(&mut self) {
        let now = std::time::Instant::now();
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(p) = self.pending.remove(&id) {
                let line = format!(
                    "`{}` was not approved — nobody answered in time, and the call was \
                     refused.",
                    p.tool
                );
                let _ = chat::update(
                    &self.slack,
                    &p.channel,
                    &p.message_ts,
                    &line,
                    Some(vec![blocks::context(&line)]),
                )
                .await;
                let _ = self.threads.apply(&p.thread_key, Event::InputSettled);
            }
        }
    }

    /// Refuse every approval a thread is waiting on, so a stopped run actually
    /// stops.
    ///
    /// `RunContext::cancel` is checked at turn boundaries and against the
    /// provider stream — never around `approve()`. A run parked in the
    /// approver therefore ignored Stop entirely and sat there for the whole
    /// timeout with its card still clickable, while the thread reported
    /// `cancelled`. Dropping the reply channels makes the approver return
    /// `Blocked` at once, which is what makes the state truthful.
    async fn refuse_pending_for(&mut self, thread_key: &str) {
        let theirs: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| p.thread_key == thread_key)
            .map(|(id, _)| id.clone())
            .collect();
        for id in theirs {
            if let Some(p) = self.pending.remove(&id) {
                drop(p.reply);
                let line = format!("`{}` was not approved — the run was stopped.", p.tool);
                let _ = chat::update(
                    &self.slack,
                    &p.channel,
                    &p.message_ts,
                    &line,
                    Some(vec![blocks::context(&line)]),
                )
                .await;
            }
        }
    }

    /// An approver wants to ask. Post a durable card — never an ephemeral,
    /// which does not survive a reload and cannot be updated.
    async fn on_approval_request(&mut self, request: approve::Request) {
        let Ok(Some(record)) = self.threads.get(&request.thread_key) else {
            return;
        };
        // **Monotonic, never `pending.len()`** — that shrinks when an entry is
        // removed, so ids were reused while older cards kept live buttons and
        // a stale card could resolve a later, unread call.
        self.approval_seq += 1;
        let id = format!("{}-{}", request.thread_key, self.approval_seq);
        let card = vec![
            blocks::section(&format!("*Approve this call?*\n`{}`", request.summary)),
            blocks::context(&format!("thread {} · {}", record.thread_ts, request.tool)),
            blocks::actions(vec![
                blocks::button("slack_approve", "Approve", &id, Some("primary")),
                blocks::button("slack_approve_run", "Allow for this run", &id, None),
                blocks::button("slack_reject", "Reject", &id, Some("danger")),
            ]),
        ];
        match chat::post_message(
            &self.slack,
            &record.channel_id,
            Some(&record.thread_ts),
            "An approval is waiting.",
            Some(card),
        )
        .await
        {
            Ok(ts) => {
                let _ = self
                    .threads
                    .apply(&request.thread_key, Event::AskedForInput);
                self.pending.insert(
                    id,
                    PendingApproval {
                        reply: request.reply,
                        channel: record.channel_id,
                        message_ts: ts,
                        tool: request.tool,
                        thread_key: request.thread_key.clone(),
                        expires_at: std::time::Instant::now()
                            + Duration::from_secs(self.cfg.approval_timeout_secs),
                    },
                );
            }
            Err(e) => {
                // Dropping the sender makes the approver fail closed, which is
                // the only safe reading of "nobody could be asked".
                tracing::warn!("could not post an approval card: {e}");
            }
        }
    }

    async fn on_interaction(&mut self, interaction: Interaction) {
        // **The gate, again, and on the field Slack signed** — never on the
        // button's value, which is a correlation id chosen by whatever composed
        // the message.
        let gate = self.allowed(
            interaction.user_id.as_deref(),
            interaction.team_id.as_deref(),
        );
        if !gate.is_allowed() {
            // `warn`, not `debug`: a refused button press has no other symptom
            // — an Approve tap silently does nothing and the card times out
            // minutes later — so which check failed, and whose tap it was,
            // must be visible under default logging.
            tracing::warn!(
                "dropped an interaction ({}): user {}, channel {}",
                gate.reason(),
                interaction.user_id.as_deref().unwrap_or("<none>"),
                interaction.channel_id.as_deref().unwrap_or("<none>"),
            );
            return;
        }

        // A modal submission rides the same envelope and passed the same gate
        // above, on the same signed field — the owner check runs before the
        // callback id is so much as read, exactly as it runs before a button
        // verb is.
        if interaction.kind == "view_submission" {
            self.on_view_submission(&interaction);
            return;
        }

        for action in &interaction.actions {
            let value = action.value.clone().unwrap_or_default();
            match action.action_id.as_str() {
                "slack_stop" => {
                    if let Some(live) = self.live.get(&value) {
                        live.cancel.cancel();
                        // Cancelling alone does not reach a run parked in the
                        // approver; refusing its pending asks does.
                        self.refuse_pending_for(&value).await;
                        let _ = self.threads.apply(&value, Event::StopPressed);
                    }
                }
                "slack_mode" => self.cycle_mode(&value).await,
                "slack_approve" | "slack_approve_run" | "slack_reject" => {
                    // Any press on an approval card is proof someone is
                    // watching — including a late tap on an expired card,
                    // whose own call stays refused below. Clear the thread's
                    // unanswered latch first, unconditionally, so the run's
                    // next gated call posts a fresh card instead of being
                    // refused invisibly for the rest of its budget.
                    if let Some((key, _)) = value.rsplit_once('-') {
                        if let Some(live) = self.live.get(key) {
                            live.unanswered.store(false, Ordering::Relaxed);
                        }
                    }
                    if let Some(pending) = self.pending.remove(&value) {
                        let answer = match action.action_id.as_str() {
                            "slack_approve" => Answer::Approve,
                            "slack_approve_run" => Answer::ApproveForRun,
                            _ => Answer::Reject("rejected from Slack".into()),
                        };
                        let verb = match action.action_id.as_str() {
                            "slack_approve" => "approved",
                            "slack_approve_run" => "allowed for this run",
                            _ => "rejected",
                        };
                        // The approver may have stopped waiting between the
                        // card being posted and this press. Saying "approved"
                        // then would be a lie the thread keeps forever.
                        let landed = pending.reply.send(answer).is_ok();
                        if !landed {
                            let line = format!(
                                "`{}` was already refused before this was pressed — nothing ran.",
                                pending.tool
                            );
                            let _ = chat::update(
                                &self.slack,
                                &pending.channel,
                                &pending.message_ts,
                                &line,
                                Some(vec![blocks::context(&line)]),
                            )
                            .await;
                            continue;
                        }
                        // Rewrite the card into a terminal record, so it says
                        // what happened and cannot be clicked again.
                        let who = interaction.user_id.as_deref().unwrap_or("someone");
                        let _ = chat::update(
                            &self.slack,
                            &pending.channel,
                            &pending.message_ts,
                            &format!("`{}` {verb} by <@{who}>", pending.tool),
                            Some(vec![blocks::context(&format!(
                                "`{}` {verb} by <@{who}>",
                                pending.tool
                            ))]),
                        )
                        .await;
                    }
                    if let Some(key) = value.rsplit_once('-').map(|(k, _)| k.to_string()) {
                        let _ = self.threads.apply(&key, Event::InputSettled);
                    }
                }
                // §6's doorways: a terminal-surface remedy translated, never
                // spawned. The press posts the pending items into the thread
                // as the cards this connector already knows how to make —
                // draft cards through the one composer (send/reject, tainted
                // two-step and all), request cards through the
                // `for_privileged_run` boundary. Nothing executes; a
                // replayed press can at most re-post cards.
                super::doctor::REVIEW_HERE_OUTBOX | super::doctor::REVIEW_HERE_FRONTDOOR => {
                    let Some(channel) = interaction.channel_id.clone() else {
                        continue;
                    };
                    let Some(thread_ts) = interaction
                        .thread_ts
                        .clone()
                        .or_else(|| interaction.message_ts.clone())
                    else {
                        continue;
                    };
                    let slack = self.slack.clone();
                    if action.action_id == super::doctor::REVIEW_HERE_OUTBOX {
                        let root = self.outbox_root.clone();
                        tokio::spawn(async move {
                            review_here_drafts(&slack, &root, &channel, &thread_ts).await;
                        });
                    } else {
                        tokio::spawn(async move {
                            review_here_requests(&slack, &channel, &thread_ts).await;
                        });
                    }
                }
                // The modal openers: they execute nothing — the typed action
                // is constructed only from the gated submission. The
                // `trigger_id` expires in three seconds and is one-use, so
                // the view opens before any other work on this press.
                super::frontdoor::CLOSE_OPEN | super::frontdoor::NEEDS_INFO_OPEN => {
                    let Some(trigger) = interaction.trigger_id.clone() else {
                        continue;
                    };
                    let Some(seq) = value.parse::<i64>().ok().filter(|s| *s > 0) else {
                        continue;
                    };
                    // Where the card lives, so the submission's outcome can
                    // retire it. Machine-authored, opaque to the person.
                    let meta = super::frontdoor::metadata(
                        seq,
                        interaction.channel_id.as_deref(),
                        interaction.message_ts.as_deref(),
                        interaction.thread_ts.as_deref(),
                    );
                    let view = if action.action_id == super::frontdoor::CLOSE_OPEN {
                        super::frontdoor::close_modal(seq, &meta)
                    } else {
                        super::frontdoor::needs_info_modal(seq, &meta)
                    };
                    if let Err(e) = mecha_slack::views::open(&self.slack, &trigger, view).await {
                        // A tap that silently does nothing is the failure
                        // this surface keeps naming; say so, and name the
                        // terminal that always works.
                        tracing::warn!("could not open the modal for request {seq}: {e}");
                        if let (Some(channel), Some(ts)) = (
                            interaction.channel_id.as_deref(),
                            interaction
                                .thread_ts
                                .as_deref()
                                .or(interaction.message_ts.as_deref()),
                        ) {
                            let _ = chat::post_message(
                                &self.slack,
                                channel,
                                Some(ts),
                                &format!(
                                    "The form could not open. At a terminal: \
                                     `mecha frontdoor close {seq} --reason ...` or \
                                     `mecha frontdoor needs-info {seq} --note ...`"
                                ),
                                None,
                            )
                            .await;
                        }
                    }
                }
                // The tainted draft's second tap, re-decided from CURRENT
                // store state (F6, design §5: the store is the defence, the
                // card is convenience). The card's ladder ran at composition;
                // a draft edited afterwards (`mecha outbox edit` keeps it
                // pending) would otherwise send bytes nobody read. The
                // button's value carries a fingerprint of the exact argument
                // bytes the card showed; a press whose store item no longer
                // matches — edited args, arguments that now truncate, a store
                // that cannot be read, a pre-fingerprint card — re-cards or
                // refuses instead of sending.
                actions::ids::OUTBOX_SEND_CONFIRM => {
                    let (Some(channel), Some(ts)) = (
                        interaction.channel_id.clone(),
                        interaction.message_ts.clone(),
                    ) else {
                        continue;
                    };
                    let Some((id, _)) = actions::parse_confirm_value(&value) else {
                        continue;
                    };
                    let item = self.outbox_item(id).await;
                    match confirm_press(&value, item.as_ref()) {
                        ConfirmPress::Send { id } => {
                            let who = interaction
                                .user_id
                                .as_deref()
                                .unwrap_or("someone")
                                .to_string();
                            self.dispatch_action(
                                Action::OutboxSend { id },
                                &who,
                                "draft-card",
                                ActionCard::Rewrite {
                                    channel,
                                    ts,
                                    thread_ts: interaction.thread_ts.clone(),
                                },
                            );
                        }
                        ConfirmPress::Recard { id } => {
                            // Nothing sends; the fresh card is composed from
                            // the store as it stands, so the truncated case
                            // comes back reject-only by the same rule as ever.
                            let (_, mut card) = tainted_confirm_card(&id, item.as_ref());
                            card.insert(
                                0,
                                blocks::context(
                                    "⚠️ The draft changed after this card was composed — \
                                     nothing was sent. Re-read it.",
                                ),
                            );
                            let _ = chat::update(
                                &self.slack,
                                &channel,
                                &ts,
                                "This draft changed after its card was composed — nothing \
                                 was sent.",
                                Some(card),
                            )
                            .await;
                        }
                        ConfirmPress::Unreadable { id } => {
                            let (text, card) = tainted_confirm_card(&id, None);
                            let _ =
                                chat::update(&self.slack, &channel, &ts, &text, Some(card)).await;
                        }
                    }
                }
                // Everything else is either an executable action or nothing.
                // `Action::from_payload` is the whole decision: a fixed verb
                // from a closed set, a value that is an object id re-resolved
                // against its store at execution time — an unknown verb or a
                // value shaped like anything but an id parses to `None` and
                // is dropped here, before any store is consulted.
                other => {
                    let Some(act) = Action::from_payload(other, &value) else {
                        continue;
                    };
                    let who = interaction
                        .user_id
                        .as_deref()
                        .unwrap_or("someone")
                        .to_string();
                    // **Where the card is comes from the payload**, not from an
                    // in-memory map: a connector restart used to make Send do
                    // nothing at all, silently, on a card still sitting in the
                    // thread.
                    let (Some(channel), Some(ts)) = (
                        interaction.channel_id.clone(),
                        interaction.message_ts.clone(),
                    ) else {
                        continue;
                    };
                    // A draft written with the trifecta armed gets a second
                    // step showing the full arguments, because every other
                    // release surface confirms and shows them — `-y` on one tap
                    // did neither. Only the first tap detours; the confirm
                    // verb resolves to the same typed action. One read decides
                    // *and* composes: the taint check and the confirm card
                    // share the same fetched item, instead of scanning the
                    // store once to decide and again to show.
                    if other == actions::ids::OUTBOX_SEND {
                        let item = self.outbox_item(&value).await;
                        if item
                            .as_ref()
                            .is_some_and(|i| i.taint.private && i.taint.untrusted)
                        {
                            self.ask_to_confirm_tainted(&value, item.as_ref(), &channel, &ts)
                                .await;
                            continue;
                        }
                    }
                    let (surface, target) = match &act {
                        // A draft card is one message: rewrite it, so the
                        // button is gone before the child runs.
                        Action::OutboxSend { .. } | Action::OutboxReject { .. } => (
                            "draft-card",
                            ActionCard::Rewrite {
                                channel,
                                ts,
                                thread_ts: interaction.thread_ts.clone(),
                            },
                        ),
                        // A board button reports beside the board, threaded
                        // where the listing is.
                        Action::TaskDone { .. } | Action::TaskNext { .. } => (
                            "tasks",
                            ActionCard::Reply {
                                thread_ts: interaction
                                    .thread_ts
                                    .clone()
                                    .unwrap_or_else(|| ts.clone()),
                                channel,
                            },
                        ),
                        // A doctor-report button reports beside the report,
                        // threaded where the report is.
                        _ => (
                            "doctor",
                            ActionCard::Reply {
                                thread_ts: interaction
                                    .thread_ts
                                    .clone()
                                    .unwrap_or_else(|| ts.clone()),
                                channel,
                            },
                        ),
                    };
                    self.dispatch_action(act, &who, surface, target);
                }
            }
        }
    }

    /// A modal came back. The gate has already run on the signed user; this
    /// parses and dispatches, fail-closed at every step — unusable metadata,
    /// missing text, or an unknown callback constructs nothing and nothing
    /// runs. `Action::from_submission` is the whole decision, exactly as
    /// `from_payload` is for a button: the seq re-resolves against the
    /// request store at execution time, and the owner-typed text travels
    /// only inside the typed action — which is also what puts it in the
    /// ledger's dispatch row.
    fn on_view_submission(&mut self, interaction: &Interaction) {
        let Some(view) = &interaction.view else {
            return;
        };
        let Some(meta) = super::frontdoor::parse_metadata(&view.private_metadata) else {
            tracing::warn!("a view submission carried unusable metadata; nothing ran");
            return;
        };
        let Some(text) = view.values.get(super::frontdoor::TEXT_INPUT) else {
            tracing::warn!("a view submission carried no text; nothing ran");
            return;
        };
        let Some(act) = Action::from_submission(&view.callback_id, &meta.seq.to_string(), text)
        else {
            tracing::warn!(
                "a view submission ({}) parsed to no action; nothing ran",
                view.callback_id
            );
            return;
        };
        let who = interaction
            .user_id
            .as_deref()
            .unwrap_or("someone")
            .to_string();
        // The request card the modal was opened from is rewritten in place —
        // buttons retired before the child runs, the outcome landing where
        // the card was — with a thread reply as the fallback when the
        // metadata could not say exactly where the card lives.
        let target = match (meta.channel, meta.ts, meta.thread_ts) {
            (Some(channel), Some(ts), thread_ts) => ActionCard::Rewrite {
                channel,
                ts,
                thread_ts,
            },
            (Some(channel), None, Some(thread_ts)) => ActionCard::Reply { channel, thread_ts },
            _ => {
                tracing::warn!("a view submission had nowhere to report; nothing ran");
                return;
            }
        };
        self.dispatch_action(act, &who, "frontdoor-modal", target);
    }
}

/// The outbox half of a Review-here press: every pending draft, carded into
/// the thread through the one draft-card composer — send/reject, the tainted
/// two-step and the truncated-reject-only rule all inherited, because the
/// cards press into the same verbs. Capped, with the cut visible.
async fn review_here_drafts(
    slack: &Slack,
    outbox_root: &std::path::Path,
    channel: &str,
    thread_ts: &str,
) {
    let items = mecha_core::outbox::OutboxStore::open(outbox_root)
        .ok()
        .and_then(|s| s.items().ok())
        .unwrap_or_default();
    let mut pending: Vec<_> = items
        .into_iter()
        .filter(|i| i.status == "pending")
        .collect();
    // Ids are timestamp-prefixed, so this is oldest first — the stuck ones
    // the finding was about lead.
    pending.sort_by(|a, b| a.id.cmp(&b.id));
    if pending.is_empty() {
        let _ = chat::post_message(
            slack,
            channel,
            Some(thread_ts),
            "Nothing is pending in the outbox.",
            None,
        )
        .await;
        return;
    }
    let total = pending.len();
    for item in pending.iter().take(REVIEW_HERE_MAX) {
        let _ = chat::post_message(
            slack,
            channel,
            Some(thread_ts),
            "A draft is waiting for review.",
            Some(draft_offer_blocks(item)),
        )
        .await;
    }
    if let Some(note) = review_here_note(total, "mecha outbox review") {
        let _ = chat::post_message(slack, channel, Some(thread_ts), &note, None).await;
    }
}

/// The frontdoor half: every waiting request as a card built from the
/// `for_privileged_run` boundary — a Slack thread is a model-adjacent
/// surface, so the stranger's prose stays at the terminal. Capped, with the
/// cut visible.
async fn review_here_requests(slack: &Slack, channel: &str, thread_ts: &str) {
    let records = mecha_core::frontdoor::Frontdoor::open_default()
        .and_then(|f| f.records())
        .unwrap_or_default();
    let waiting = super::frontdoor::waiting(&records);
    if waiting.is_empty() {
        let _ = chat::post_message(
            slack,
            channel,
            Some(thread_ts),
            "Nothing is waiting at the front door.",
            None,
        )
        .await;
        return;
    }
    let total = waiting.len();
    for record in waiting.iter().take(REVIEW_HERE_MAX) {
        let _ = chat::post_message(
            slack,
            channel,
            Some(thread_ts),
            &format!("Request {} is waiting.", record.seq),
            Some(super::frontdoor::request_card(record)),
        )
        .await;
    }
    if let Some(note) = review_here_note(total, "mecha frontdoor list") {
        let _ = chat::post_message(slack, channel, Some(thread_ts), &note, None).await;
    }
}

/// Where a dispatched action reports.
///
/// `Rewrite` retires a one-card message in place — the button is gone before
/// the child runs, and the outcome lands where the card was. `Reply` is for
/// controls that live inside a larger message (a doctor report), which cannot
/// be rewritten without destroying the findings around it: the dispatch and
/// outcome land as thread replies, and idempotence rests on the store guards,
/// which §5 requires to be the real defence in every case.
enum ActionCard {
    Rewrite {
        channel: String,
        ts: String,
        /// The thread the card sits in, when the payload said — where a slow
        /// action's fresh, notifying reply lands (F12). Absent, the card's
        /// own ts is the thread root and stands in.
        thread_ts: Option<String>,
    },
    Reply {
        channel: String,
        thread_ts: String,
    },
}

/// F12's one delivery rule: `(update the dispatch message, post a fresh
/// reply)`. The dispatch message is updated whenever it exists; a fresh reply
/// The appraisal footer for a finished run: `appraisal · −0.5`, or
/// `appraisal · anger −0.5` when the label says a word. Numbers and a closed
/// enum's wire word only — nothing here was written by a model.
fn appraisal_line(readout: &mecha_core::appraisal::Readout) -> String {
    let mut line = String::from("appraisal ·");
    if readout.label != mecha_core::appraisal::Affect::Neutral {
        line.push(' ');
        line.push_str(&readout.label.wire());
    }
    let n = readout.valence.compact();
    if !n.is_empty() {
        line.push(' ');
        line.push_str(&n);
    }
    line
}

/// is owed when the dispatch never landed — the outcome must land somewhere —
/// or when the action was slow, because a card edit fires no notification.
/// Never both fresh posts, which was the double-post.
fn outcome_delivery(dispatch_landed: bool, slow: bool) -> (bool, bool) {
    (dispatch_landed, !dispatch_landed || slow)
}

/// F13: whether a finished send needs the draft's card posted afterwards — a
/// release nobody was carded for (`review auto`) that left the item pending
/// has no other phone path back to review. Card-tapped releases already have
/// their card; other surfaces card on demand.
fn failed_auto_release_needs_card(surface: &str, item_status: Option<&str>) -> bool {
    surface == "review-auto" && item_status == Some("pending")
}

/// One pending draft as a review card — Send and Reject carrying the item id.
///
/// **The only draft-card composer.** `offer_drafts` (a run's own staged
/// drafts) and the doctor doorway's review-here batch both post exactly this,
/// which is what "zero new send paths" means structurally: a review-here card
/// presses into the same `OUTBOX_SEND` verb, so the tainted detour
/// (`ask_to_confirm_tainted`, and its truncated-arguments reject-only rule)
/// and the store's pending check cover it without knowing where the card came
/// from.
fn draft_offer_blocks(item: &mecha_core::outbox::OutboxItem) -> Vec<serde_json::Value> {
    let armed = item.taint.private && item.taint.untrusted;
    let heading = if armed {
        format!(
            "*⚠️ Draft — written with the trifecta armed*\n`{}`",
            item.tool
        )
    } else {
        format!("*Draft*\n`{}`", item.tool)
    };
    vec![
        blocks::section(&heading),
        blocks::section(&format!("```\n{}\n```", truncate_for_slack(&item.summary))),
        blocks::context(&format!("id `{}` · nothing has been sent", item.id)),
        blocks::actions(vec![
            blocks::button(actions::ids::OUTBOX_SEND, "Send", &item.id, Some("primary")),
            blocks::button(
                actions::ids::OUTBOX_REJECT,
                "Reject",
                &item.id,
                Some("danger"),
            ),
        ]),
    ]
}

/// How many items a review-here press cards into the thread before naming
/// the terminal for the rest. A thread is a screen, not a queue; forty cards
/// is a wall nobody reviews.
const REVIEW_HERE_MAX: usize = 8;

/// The visible cut when a review-here batch holds more than the cap: the
/// rest are named, with the terminal surface that shows them all.
fn review_here_note(total: usize, terminal_cmd: &str) -> Option<String> {
    let rest = total.saturating_sub(REVIEW_HERE_MAX);
    (rest > 0)
        .then(|| format!("{rest} more not shown here — the rest at the terminal: `{terminal_cmd}`"))
}

/// The tainted draft's confirm card, and the one rule it enforces: **you may
/// reject what you cannot fully read; you may not send it.**
///
/// For a draft whose arguments fit under Slack's block cap, the red card is
/// the TUI's full-arguments-in-red review and the confirm tap is as informed
/// as the terminal's `y`. For a draft the cap truncates, the reviewer would
/// be approving bytes the surface could not show — the exact "reading one
/// file while approving another" failure the outbox's workspace-resolution
/// rule exists to prevent, in miniature — so the card carries **no Send
/// button at all**: it shows what fits, says what was cut, and names the
/// terminal as where this draft is released. Reject stays; declining needs
/// no completeness.
fn tainted_confirm_blocks(id: &str, args: &str) -> Vec<serde_json::Value> {
    let shown = truncate_for_slack(args);
    let cut = shown != args;
    let mut card = vec![
        blocks::section(
            "*⚠️ This draft was written while the trifecta was armed.*\nPrivate data and \
             untrusted content were both in the conversation that produced it. The full \
             arguments are below — read them before sending.",
        ),
        blocks::section(&format!("```\n{shown}\n```")),
    ];
    if cut {
        card.push(blocks::section(&format!(
            "*The arguments are longer than this card can show* ({} of {} characters). \
             A draft you cannot fully read is not released from here: run \
             `mecha outbox show {id}` in a terminal to read all of it and release it \
             there.",
            shown.chars().count(),
            args.chars().count()
        )));
        card.push(blocks::actions(vec![blocks::button(
            actions::ids::OUTBOX_REJECT,
            "Reject",
            id,
            Some("danger"),
        )]));
    } else {
        card.push(blocks::actions(vec![
            // The value carries a fingerprint of the exact bytes this card
            // shows (F6): the press re-proves the store still holds them
            // before anything sends. Machine-authored correlation state, not
            // a command fragment — the id half still resolves against the
            // store, the fingerprint half only ever equals or differs.
            blocks::button(
                actions::ids::OUTBOX_SEND_CONFIRM,
                "Send anyway",
                &actions::confirm_value(id, args),
                Some("danger"),
            ),
            blocks::button(actions::ids::OUTBOX_REJECT, "Reject", id, None),
        ]));
    }
    card
}

/// The tainted second step, composed from CURRENT store state: the full red
/// card when the item could be read, and an error card — reject-only — when
/// it could not (F3). The old shape unwrapped an unreadable item to empty
/// arguments, `cut = false`, and offered a live Send over zero shown bytes;
/// nothing is offered for sending on bytes nobody can see.
fn tainted_confirm_card(
    id: &str,
    item: Option<&mecha_core::outbox::OutboxItem>,
) -> (String, Vec<serde_json::Value>) {
    match item {
        Some(item) => {
            let args = serde_json::to_string_pretty(&item.args).unwrap_or_default();
            (
                "This draft needs a second look before it is sent.".to_string(),
                tainted_confirm_blocks(id, &args),
            )
        }
        None => (
            format!("Draft `{id}` could not be read — nothing is offered for sending."),
            vec![
                blocks::section(&format!(
                    "*Draft `{id}` could not be read from the outbox.*\nNothing is \
                     offered for sending on bytes nobody can see. Read and release it \
                     at a terminal: `mecha outbox show {id}`."
                )),
                blocks::actions(vec![blocks::button(
                    actions::ids::OUTBOX_REJECT,
                    "Reject",
                    id,
                    Some("danger"),
                )]),
            ],
        ),
    }
}

/// What a press on a tainted draft's Send-anyway button may do, decided from
/// the store as it stands at press time (F6). Pure, so the ladder has a test:
/// the composition-time decision ran against the store of hours ago, and
/// `mecha outbox edit` keeps an edited item pending.
#[derive(Debug, PartialEq, Eq)]
enum ConfirmPress {
    /// The bytes the card showed are the bytes the store holds: release.
    Send { id: String },
    /// The store moved under the card — edited arguments, arguments that now
    /// truncate, or a pre-fingerprint card that cannot prove anything: show a
    /// fresh card instead of sending.
    Recard { id: String },
    /// The item cannot be read at all: an error card, never a send (F3).
    Unreadable { id: String },
}

fn confirm_press(value: &str, item: Option<&mecha_core::outbox::OutboxItem>) -> ConfirmPress {
    // The caller only reaches here for values `parse_confirm_value` accepted.
    let (id, fingerprint) = match actions::parse_confirm_value(value) {
        Some(parsed) => parsed,
        None => {
            return ConfirmPress::Recard {
                id: value.to_string(),
            }
        }
    };
    let Some(item) = item else {
        return ConfirmPress::Unreadable { id: id.to_string() };
    };
    let args = serde_json::to_string_pretty(&item.args).unwrap_or_default();
    // The ladder, re-run: a draft whose arguments no longer fit under the
    // card cap is reject-only however the press arrived — §8's rule keyed on
    // current bytes, not on the bytes of composition time.
    if truncate_for_slack(&args) != args {
        return ConfirmPress::Recard { id: id.to_string() };
    }
    match fingerprint {
        Some(fp) if fp == actions::fingerprint(&args) => ConfirmPress::Send { id: id.to_string() },
        // Drift — or a card composed before values carried a fingerprint,
        // which cannot prove the bytes and re-cards: one extra tap, never an
        // unread send.
        _ => ConfirmPress::Recard { id: id.to_string() },
    }
}

/// Whether one staged draft releases with no card at run completion: the
/// thread (or channel) must have an auto setting *and* the shared policy must
/// agree — the tainted exclusion and the early-stop rule (F1) both live in
/// `review_policy::auto_releases`, so this call site cannot hold half the
/// rule.
fn releases_without_card(
    setting: Option<&review::Setting>,
    tainted: bool,
    finished_clean: bool,
) -> bool {
    setting.is_some_and(|s| crate::review_policy::auto_releases(s.mode, tainted, finished_clean))
}

/// A filename safe to join onto a directory.
///
/// Slack supplies the name and the owner chose it, but a name that reaches the
/// filesystem is a path: `../../.mecha/slack/credentials.json` is a perfectly
/// ordinary string until it is joined onto something. The path jail would catch
/// a model asking for that; nothing catches it here, because this write happens
/// in the connector before any tool is involved.
pub(crate) fn safe_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['.', '-']).to_string();
    // A name of nothing but separators is legal on disk and meaningless to a
    // reader, which is its own small failure: the model is about to be told
    // this path exists.
    if !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
        return "attachment".into();
    }
    cleaned.chars().take(120).collect()
}

/// Every file in a run's workspace, with size and mtime, so a later pass can
/// tell what the run made.
///
/// `inbox/` is skipped: those are the files the *user* attached, and sending
/// them back is noise. Hidden entries are skipped for the same reason a person
/// would skip them. Bounded depth, because a workspace that has acquired a
/// `node_modules` should not turn every run into a directory walk.
fn workspace_snapshot(root: &std::path::Path) -> HashMap<PathBuf, (u64, std::time::SystemTime)> {
    fn walk(
        dir: &std::path::Path,
        depth: usize,
        out: &mut HashMap<PathBuf, (u64, std::time::SystemTime)>,
    ) {
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "inbox" {
                continue;
            }
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => walk(&path, depth - 1, out),
                Ok(meta) => {
                    let stamp = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    out.insert(path, (meta.len(), stamp));
                }
                Err(_) => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(root, 6, &mut out);
    out
}

/// Every pending draft's id, or an empty set if the outbox does not exist.
///
/// An empty set on failure is deliberate: it makes the id-diff find nothing
/// new rather than offering the whole outbox for release, which is the
/// direction a failure here has to fall.
fn pending_outbox_ids(root: &std::path::Path) -> std::collections::HashSet<String> {
    let Ok(store) = mecha_core::outbox::OutboxStore::open(root) else {
        return Default::default();
    };
    store
        .items()
        .map(|items| {
            items
                .into_iter()
                .filter(|i| i.status == "pending")
                .map(|i| i.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Slack section blocks cap at 3,000 characters and a code fence costs some of
/// that; a draft longer than this is read in the outbox proper.
fn truncate_for_slack(text: &str) -> String {
    mecha_slack::blocks::truncate(text, 2_600)
}

/// Stop and Mode, carrying the thread key as their correlation value. The
/// value authorises nothing: every press is gated on `payload.user.id`.
fn controls_blocks(key: &str, mode: &str) -> Vec<serde_json::Value> {
    vec![
        blocks::context(&format!("mode `{mode}`")),
        blocks::actions(vec![
            blocks::button("slack_stop", "Stop", key, Some("danger")),
            blocks::button("slack_mode", "Mode", key, None),
        ]),
    ]
}

/// Where every Slack thread's workspace lives, and where the shared MCP
/// servers are rooted so their paths and the runs' paths mean the same thing.
fn producer_root() -> Result<std::path::PathBuf> {
    let root = mecha_core::work::producer_dir("slack")?;
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    Ok(root)
}

/// One thread, one jail — under a single `slack` producer so `work clean`'s
/// per-producer retention retires whole old threads. A producer per thread
/// would accumulate directories nothing ever sweeps.
fn thread_workspace(key: &str) -> Result<std::path::PathBuf> {
    let root = mecha_core::work::producer_dir("slack")?.join(key);
    if root.exists() && !root.is_dir() {
        bail!("{} exists and is not a directory", root.display());
    }
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    mecha_core::work::ensure_outside_mecha_home(&root)?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::{
        appraisal_line, confirm_press, draft_offer_blocks, failed_auto_release_needs_card,
        outcome_delivery, releases_without_card, review_here_note, safe_filename,
        tainted_confirm_blocks, tainted_confirm_card, ConfirmPress, REVIEW_HERE_MAX,
    };
    use crate::review_policy::ReviewMode;
    use crate::slack::actions;
    use crate::slack::review::Setting;

    fn draft(id: &str, tainted: bool, summary: &str) -> mecha_core::outbox::OutboxItem {
        mecha_core::outbox::OutboxItem {
            id: id.into(),
            status: "pending".into(),
            tool: "mail__send".into(),
            kind: mecha_core::outbox::OutboxKind::Message,
            args_before: serde_json::json!({}),
            args: serde_json::json!({}),
            summary: summary.into(),
            session_id: None,
            workspace: None,
            taint: mecha_core::agent::Taint {
                private: tainted,
                untrusted: tainted,
            },
            created_at: "2026-08-14T00:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    /// Review-here posts drafts through the EXISTING machinery — this is the
    /// "zero new send paths" test. A tainted draft's card in a review-here
    /// batch carries the same `OUTBOX_SEND` verb every draft card carries,
    /// which is precisely the verb the connector detours through the red
    /// two-step (`ask_to_confirm_tainted`); and the second step for a draft
    /// whose arguments were truncated still offers Reject and never Send.
    /// One composer, one detour, no way for a batch to reach a send the
    /// one-off card could not.
    #[test]
    fn a_tainted_draft_in_a_review_here_batch_keeps_the_two_step_and_truncated_stays_reject_only() {
        let batch = [
            draft("a-1", false, "hello"),
            draft("a-2", true, "the tainted one"),
        ];
        let cards: Vec<String> = batch
            .iter()
            .map(|i| serde_json::to_string(&draft_offer_blocks(i)).unwrap())
            .collect();

        // Both cards press into the same verbs — the ones `from_payload`
        // parses and the tainted detour keys on.
        for card in &cards {
            assert!(card.contains(actions::ids::OUTBOX_SEND), "{card}");
            assert!(card.contains(actions::ids::OUTBOX_REJECT), "{card}");
            assert!(
                !card.contains(actions::ids::OUTBOX_SEND_CONFIRM),
                "the confirm verb only ever appears on the second step: {card}"
            );
        }
        // The tainted card says what it is before anything is pressed.
        assert!(cards[1].contains("trifecta armed"), "{}", cards[1]);
        assert!(!cards[0].contains("trifecta armed"), "{}", cards[0]);

        // And the second step it detours into, for arguments the card cannot
        // fully show, is reject-only — the batch inherits §8's rule because
        // it is the same code, not a copy of it.
        let second_step =
            serde_json::to_string(&tainted_confirm_blocks("a-2", &"x".repeat(10_000))).unwrap();
        assert!(
            !second_step.contains(actions::ids::OUTBOX_SEND_CONFIRM),
            "{second_step}"
        );
        assert!(
            second_step.contains(actions::ids::OUTBOX_REJECT),
            "{second_step}"
        );
    }

    /// F1, failing on the old encoding, which had no way to ask how the run
    /// ended: `review auto` released an interrupted run's drafts. The policy
    /// signature now carries the stop-cause cleanliness, so an early-stopped
    /// run's untainted drafts are carded — review stays possible — and never
    /// dispatched.
    #[test]
    fn review_auto_releases_nothing_after_an_early_stopped_run() {
        let auto = Setting {
            mode: ReviewMode::Auto,
            set_by: "U_OWNER".into(),
        };
        assert!(
            !releases_without_card(Some(&auto), false, false),
            "an interrupted run's untainted draft cards, never auto-releases"
        );
        assert!(
            releases_without_card(Some(&auto), false, true),
            "a clean finish still releases the untainted draft"
        );
        assert!(
            !releases_without_card(Some(&auto), true, true),
            "tainted never releases, clean finish or not"
        );
        assert!(
            !releases_without_card(None, false, true),
            "no setting means now: card everything"
        );
        for mode in [ReviewMode::Now, ReviewMode::Later] {
            let s = Setting {
                mode,
                set_by: "U_OWNER".into(),
            };
            assert!(!releases_without_card(Some(&s), false, true));
        }
    }

    /// F3, failing on the old `ask_to_confirm_tainted`, which unwrapped a
    /// transient store failure to empty arguments (`cut = false`) and offered
    /// a live Send over zero shown bytes. An unreadable item is an error
    /// card: the failure stated, the terminal named, Reject the only button.
    #[test]
    fn an_unreadable_item_gets_an_error_card_and_never_a_send() {
        let (text, card) = tainted_confirm_card("abc-123", None);
        let json = serde_json::to_string(&card).unwrap();
        assert!(
            !json.contains(actions::ids::OUTBOX_SEND_CONFIRM),
            "no Send on bytes nobody can see: {json}"
        );
        assert!(!json.contains("\"slack_outbox_send\""), "{json}");
        assert!(json.contains(actions::ids::OUTBOX_REJECT), "{json}");
        assert!(
            json.contains("could not be read"),
            "the error is stated: {json}"
        );
        assert!(json.contains("mecha outbox show abc-123"), "{json}");
        assert!(text.contains("could not be read"), "{text}");

        // Not vacuous: a readable item still composes the real second step.
        let item = draft("abc-123", true, "hello");
        let (_, card) = tainted_confirm_card("abc-123", Some(&item));
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains(actions::ids::OUTBOX_SEND_CONFIRM), "{json}");
    }

    /// F6, failing on the old press handling, which mapped the confirm verb
    /// straight to a send with no press-time re-check: a draft edited between
    /// composition and press (`mecha outbox edit` keeps it pending) sent
    /// bytes nobody read. The press now re-proves the store still holds the
    /// exact bytes the card showed.
    #[test]
    fn a_confirm_press_on_a_draft_edited_after_composition_recards_instead_of_sending() {
        let mut item = draft("abc-123", true, "the tainted one");
        item.args = serde_json::json!({"to": "a@x.org", "body": "as reviewed"});
        let shown = serde_json::to_string_pretty(&item.args).unwrap();
        let value = actions::confirm_value("abc-123", &shown);

        // The store still holds what the card showed: the press sends.
        assert_eq!(
            confirm_press(&value, Some(&item)),
            ConfirmPress::Send {
                id: "abc-123".into()
            }
        );

        // Edited between composition and press: not sent — re-carded.
        item.args = serde_json::json!({"to": "attacker@evil.example", "body": "as reviewed"});
        assert_eq!(
            confirm_press(&value, Some(&item)),
            ConfirmPress::Recard {
                id: "abc-123".into()
            },
            "changed bytes must never send"
        );

        // Unreadable at press time: the error card, never a send (F3 at the
        // verb).
        assert_eq!(
            confirm_press(&value, None),
            ConfirmPress::Unreadable {
                id: "abc-123".into()
            }
        );

        // A pre-fingerprint card cannot prove its bytes: one extra tap, not
        // an unread send.
        assert_eq!(
            confirm_press("abc-123", Some(&item)),
            ConfirmPress::Recard {
                id: "abc-123".into()
            }
        );

        // Arguments that now truncate re-card even when the fingerprint
        // matches — §8's ladder keyed on current bytes: the fresh card comes
        // back reject-only.
        item.args = serde_json::json!({"body": "x".repeat(10_000)});
        let long = serde_json::to_string_pretty(&item.args).unwrap();
        let long_value = actions::confirm_value("abc-123", &long);
        assert_eq!(
            confirm_press(&long_value, Some(&item)),
            ConfirmPress::Recard {
                id: "abc-123".into()
            }
        );
    }

    /// The composition half of F6: the Send-anyway button's value carries the
    /// fingerprint of the exact bytes the card shows, so press and
    /// composition cannot drift.
    #[test]
    fn a_tainted_confirm_cards_send_button_carries_the_shown_bytes_fingerprint() {
        let args = "{\n  \"to\": \"a@x.org\"\n}";
        let json = serde_json::to_string(&tainted_confirm_blocks("abc-123", args)).unwrap();
        assert!(
            json.contains(&actions::confirm_value("abc-123", args)),
            "the value is id#fingerprint: {json}"
        );
        // Reject still carries the bare id — declining needs no proof.
        assert!(json.contains("\"abc-123\""), "{json}");
    }

    /// F12: one delivery rule for every target. The old shape double-posted a
    /// slow Reply-target action whose dispatch post had failed, and gave a
    /// slow Rewrite (an outbox release with an MCP startup) no fresh
    /// notification at all.
    #[test]
    fn the_appraisal_footer_is_a_number_and_a_wire_word_at_most() {
        use mecha_core::appraisal::{Affect, Readout, Valence};
        let v = Valence {
            positive: 0.0,
            negative: 0.5,
            positives: 0,
            negatives: 1,
            visible: false,
            partial: false,
        };
        assert_eq!(
            appraisal_line(&Readout {
                label: Affect::Neutral,
                valence: v
            }),
            "appraisal · \u{2212}0.5"
        );
        assert_eq!(
            appraisal_line(&Readout {
                label: Affect::Anger,
                valence: v
            }),
            "appraisal · anger \u{2212}0.5"
        );
    }

    #[test]
    fn an_outcome_lands_exactly_once_plus_a_fresh_reply_when_slow_or_lost() {
        // (dispatch landed, slow) -> (update the card, fresh reply)
        assert_eq!(outcome_delivery(true, false), (true, false));
        assert_eq!(
            outcome_delivery(true, true),
            (true, true),
            "a slow rewrite earns the notifying reply too"
        );
        assert_eq!(outcome_delivery(false, false), (false, true));
        assert_eq!(
            outcome_delivery(false, true),
            (false, true),
            "never two fresh posts — that was the double-post"
        );
    }

    /// F13: a failed `review auto` release used to leave the draft pending
    /// with no card anywhere — no phone path back. The predicate that posts
    /// the Now-mode card afterwards.
    #[test]
    fn a_failed_auto_release_gets_the_draft_carded_for_the_phone() {
        assert!(failed_auto_release_needs_card(
            "review-auto",
            Some("pending")
        ));
        assert!(
            !failed_auto_release_needs_card("review-auto", Some("sent")),
            "a release that landed needs no card"
        );
        assert!(
            !failed_auto_release_needs_card("draft-card", Some("pending")),
            "a card-tapped release already has its card"
        );
        assert!(
            !failed_auto_release_needs_card("review-auto", None),
            "an unreadable item is reported, not guessed into a card"
        );
    }

    /// The handler's order is the guarantee, pinned the way the doctor
    /// command word pins it: `on_interaction` runs `binding::check` on
    /// `payload.user.id` — the field Slack signed — and returns before the
    /// `view_submission` branch is reached, so a non-owner's submission is
    /// dropped before its callback id, metadata or text are so much as read.
    /// Nothing the view carries participates in the gate.
    #[test]
    fn a_non_owners_view_submission_is_gated_on_the_signed_user_before_anything_is_read() {
        use mecha_slack::binding::{self, Binding};
        let bound = Binding {
            team_id: "T1".into(),
            enterprise_id: None,
            owners: vec!["U_OWNER".into()],
            bound_at: chrono::Utc::now(),
        };
        // A submission whose every view-carried field is well-formed — valid
        // callback, valid metadata, valid text — still authorises nothing:
        // the gate never looks at them.
        let stranger = binding::check(Some(&bound), Some("U_STRANGER"), Some("T1"));
        assert!(
            !stranger.is_allowed(),
            "a stranger's submission is refused before the view is read"
        );
        // And the well-formed view parses into a real action, which is
        // exactly why the gate has to come first.
        assert!(actions::Action::from_submission(
            actions::ids::FRONTDOOR_CLOSE_SUBMIT,
            "5",
            "a perfectly plausible reason"
        )
        .is_some());
        let owner = binding::check(Some(&bound), Some("U_OWNER"), Some("T1"));
        assert!(owner.is_allowed());
    }

    /// The block-cap rule for a review-here batch: past the cap, the cut is
    /// visible and the rest are at the terminal.
    #[test]
    fn a_review_here_batch_past_the_cap_names_the_terminal_for_the_rest() {
        assert_eq!(
            review_here_note(REVIEW_HERE_MAX, "mecha outbox review"),
            None
        );
        let note =
            review_here_note(REVIEW_HERE_MAX + 3, "mecha outbox review").expect("the cut says so");
        assert!(note.contains("3 more"), "{note}");
        assert!(note.contains("mecha outbox review"), "{note}");
        let front = review_here_note(20, "mecha frontdoor list").unwrap();
        assert!(front.contains("mecha frontdoor list"), "{front}");
    }

    /// §8's tightening, failing on the shipped behaviour: the old card put a
    /// Send-anyway button on every tainted confirm, including drafts whose
    /// arguments `truncate_for_slack` had cut — a reviewer approving bytes
    /// the surface could not show. You may reject what you cannot fully
    /// read; you may not send it.
    #[test]
    fn a_truncated_tainted_confirm_card_offers_reject_and_never_send() {
        let long_args = "x".repeat(10_000);
        let card = serde_json::to_string(&tainted_confirm_blocks("abc-123", &long_args)).unwrap();

        assert!(
            !card.contains("slack_outbox_send_confirm"),
            "no Send on a draft the card could not fully show: {card}"
        );
        assert!(
            card.contains("slack_outbox_reject"),
            "declining needs no completeness: {card}"
        );
        // The cut says so, and names the surface that can show everything.
        assert!(card.contains("mecha outbox show abc-123"), "{card}");
        assert!(card.contains("characters"), "{card}");
    }

    #[test]
    fn a_tainted_confirm_card_that_fits_keeps_the_informed_send() {
        // Not vacuous: the reject-only rule is about what could not be shown,
        // and a fully shown draft keeps the terminal's directness.
        let card =
            serde_json::to_string(&tainted_confirm_blocks("abc-123", "{\"to\": \"a@x.org\"}"))
                .unwrap();
        assert!(card.contains("slack_outbox_send_confirm"), "{card}");
        assert!(card.contains("slack_outbox_reject"), "{card}");
    }

    /// The review mode is session-scoped by construction: it lives in the
    /// connector's memory and is deliberately not a field of the thread
    /// record, so a restart — the thread-state eviction that orphans
    /// mid-flight runs — expires every mode with it. If someone persists it,
    /// this fails and the expiry story has to be re-argued, not silently
    /// widened.
    #[test]
    fn a_review_mode_is_never_written_to_the_thread_record() {
        let record = super::super::threads::ThreadRecord {
            key: "D1-1".into(),
            channel_id: "D1".into(),
            thread_ts: "1.0".into(),
            state: super::super::threads::ThreadState::Idle,
            session_id: None,
            workspace: None,
            mode: "ask".into(),
            last_seen_ts: None,
            run: None,
            stream_ts: None,
            controls_ts: None,
            updated_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(
            !json.contains("review"),
            "a persisted review mode would outlive the attention that set it: {json}"
        );
    }

    #[test]
    fn approval_ids_never_repeat_even_as_entries_are_removed() {
        // The bug: ids were minted from `pending.len()`, which *shrinks* when
        // an approval resolves, so a later call could reuse the id of a card
        // still sitting in the thread with live buttons — and pressing that
        // stale card would approve a call the reader never saw.
        let mut seq = 0u64;
        let mut pending: std::collections::HashSet<String> = Default::default();
        let mut minted = Vec::new();
        for round in 0..50 {
            seq += 1;
            let id = format!("D1-1.0-{seq}");
            assert!(pending.insert(id.clone()), "id {id} was reused");
            minted.push(id.clone());
            // Resolve an older one every other round, shrinking the map.
            if round % 2 == 1 {
                if let Some(old) = minted.first().cloned() {
                    pending.remove(&old);
                    minted.remove(0);
                }
            }
        }
        // The counter, unlike a length, only ever goes up.
        assert_eq!(seq, 50);
    }

    #[test]
    fn a_snapshot_ignores_what_the_user_sent_and_what_is_hidden() {
        // `inbox/` holds the files the *user* attached; posting them back is
        // noise, and it would look like the agent produced them.
        // Counter, not a timestamp: `as_nanos()` is only as fine-grained as
        // the platform's clock, and on macOS two parallel tests can land on
        // the same value and then share a directory.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mecha-slack-snap-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("made.txt"), "x").unwrap();
        std::fs::write(dir.join("sub/also.txt"), "y").unwrap();
        std::fs::write(dir.join("inbox/sent-by-user.png"), "z").unwrap();
        std::fs::write(dir.join(".hidden"), "h").unwrap();

        let snap = super::workspace_snapshot(&dir);
        let names: Vec<String> = snap
            .keys()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();

        assert!(names.contains(&"made.txt".to_string()), "{names:?}");
        assert!(
            names.contains(&"also.txt".to_string()),
            "recurses: {names:?}"
        );
        assert!(
            !names.contains(&"sent-by-user.png".to_string()),
            "{names:?}"
        );
        assert!(!names.contains(&".hidden".to_string()), "{names:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_draft_summary_fits_inside_a_section_block_with_its_fence() {
        // A section block caps at 3,000 characters and the code fence spends
        // some of them. Slack drops an oversized block with a warning nobody
        // reads, which would make a long draft's card arrive empty — the one
        // card where the reader most needs to see what they are releasing.
        let long = super::truncate_for_slack(&"x".repeat(10_000));
        let fenced = format!("```\n{long}\n```");
        assert!(
            fenced.chars().count() < mecha_slack::blocks::limits::SECTION_TEXT,
            "{} chars",
            fenced.chars().count()
        );
        assert!(long.contains("truncated"), "and the cut says so");
    }

    #[test]
    fn an_attachment_name_cannot_climb_out_of_the_inbox() {
        // The write happens in the connector, before any tool and therefore
        // before the path jail. This is the only thing between a Slack
        // filename and the filesystem.
        for hostile in [
            "../../.mecha/slack/credentials.json",
            "..\\..\\windows",
            "/etc/passwd",
            "..",
            "...",
        ] {
            let safe = safe_filename(hostile);
            assert!(!safe.contains('/'), "{hostile} -> {safe}");
            assert!(!safe.contains('\\'), "{hostile} -> {safe}");
            assert!(!safe.starts_with('.'), "{hostile} -> {safe}");
            assert!(!safe.is_empty(), "{hostile} -> {safe}");
        }
    }

    #[test]
    fn an_ordinary_name_survives_intact() {
        // Not vacuous: the sanitiser must not mangle the common case.
        assert_eq!(safe_filename("screenshot.png"), "screenshot.png");
        assert_eq!(safe_filename("notes-2026_08.md"), "notes-2026_08.md");
    }

    #[test]
    fn a_nameless_attachment_still_lands_somewhere() {
        assert_eq!(safe_filename(""), "attachment");
        assert_eq!(safe_filename("???"), "attachment");
    }

    #[test]
    fn a_threads_jail_sits_under_one_producer_so_retention_reaches_it() {
        // The trap: a producer per thread looks natural and is never swept,
        // because `work clean` keeps N entries *per producer* and never
        // removes a producer directory.
        let path = mecha_core::work::producer_dir("slack")
            .unwrap()
            .join("D1-1");
        let parent = path.parent().unwrap();
        assert!(
            parent.ends_with("slack"),
            "every thread must be an entry inside one producer: {}",
            path.display()
        );
    }
}
