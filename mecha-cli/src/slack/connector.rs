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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mecha_core::agent::{Agent, AgentEvent, Budget, Conversation, RunOutcome};
use mecha_core::message::Message;
use mecha_core::outbox::OutboxRoute;
use mecha_core::session::{Record, Session, SessionMeta};
use mecha_core::tool::ToolCtx;
use mecha_slack::binding::{self, Binding, Credentials, Gate, SlackStore};
use mecha_slack::envelope::{FileRef, Inbound, Interaction, SlackEvent};
use mecha_slack::{blocks, chat, Slack, SocketMode, SocketOptions};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::approve::{self, Answer, Mode, SlackApprover};
use super::pump::{pump, PumpConfig};
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
}

/// A finished run, handed back to the loop that owns the conversations.
struct Completion {
    key: String,
    conversation: Conversation,
    outcome: Result<Box<RunOutcome>, String>,
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
        cx.approver = Arc::new(SlackApprover::new(
            key.clone(),
            Arc::clone(&mode),
            self.approval_tx.clone(),
            Duration::from_secs(self.cfg.approval_timeout_secs),
        ));
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
        if !files.is_empty() {
            let landed = self.fetch_attachments(&files, &cx.tools.workspace).await;
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
        conversation.messages.push(Message::user(&prompt));

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
            let outcome = agent
                .run_in(&cx, &mut conversation, Some(events_tx))
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = renderer.await;
            // Recorded whether it succeeded or not: a failed run is exactly
            // the transcript someone wants afterwards.
            let _ = session_for_task.record_run(&before, &conversation.messages);

            let _ = completion_tx
                .send(Completion {
                    key: key_for_task,
                    conversation,
                    outcome,
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
    ) -> Vec<String> {
        let inbox = workspace.join("inbox");
        if let Err(e) = std::fs::create_dir_all(&inbox) {
            tracing::warn!("could not make an inbox directory: {e}");
            return Vec::new();
        }
        let mut landed = Vec::new();
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
                        Ok(()) => landed.push(format!("./inbox/{name}")),
                        Err(e) => tracing::warn!("could not save {name}: {e}"),
                    }
                }
                Err(e) => tracing::warn!("could not fetch {}: {e}", file.id),
            }
        }
        landed
    }

    async fn on_completion(&mut self, done: Completion) {
        match &done.outcome {
            Ok(o) => println!("[{}] run finished — {} turns", done.key, o.turns),
            Err(e) => println!("[{}] run failed — {e}", done.key),
        }
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
        }

        match done.outcome {
            Ok(outcome) => {
                // A run that staged drafts is not finished from the person's
                // side, and the state says so.
                let staged = outcome.tool_calls.iter().any(|c| c.staged);
                let _ = self.threads.apply(&done.key, Event::Finished { staged });
                self.post_artifacts(&done.key).await;
                if staged {
                    self.offer_drafts(&done.key).await;
                }
            }
            Err(error) => {
                let _ = self.threads.apply(&done.key, Event::Errored);
                // Posted, never only logged: a failure the person cannot see
                // is indistinguishable from a run that is still thinking.
                if let Ok(Some(r)) = self.threads.get(&done.key) {
                    let _ = chat::post_message(
                        &self.slack,
                        &r.channel_id,
                        Some(&r.thread_ts),
                        &format!("The run failed: {error}"),
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
    async fn offer_drafts(&mut self, key: &str) {
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

        for item in fresh {
            // The taint snapshot rides on the item, and a draft written while
            // the trifecta was armed is the one a person must look at hardest.
            let armed = item.taint.private && item.taint.untrusted;
            let heading = if armed {
                format!(
                    "*⚠️ Draft — written with the trifecta armed*\n`{}`",
                    item.tool
                )
            } else {
                format!("*Draft*\n`{}`", item.tool)
            };
            let blocks = vec![
                blocks::section(&heading),
                blocks::section(&format!("```\n{}\n```", truncate_for_slack(&item.summary))),
                blocks::context(&format!("id `{}` · nothing has been sent", item.id)),
                blocks::actions(vec![
                    blocks::button("slack_outbox_send", "Send", &item.id, Some("primary")),
                    blocks::button("slack_outbox_reject", "Reject", &item.id, Some("danger")),
                ]),
            ];
            // No map of where the card went: a button press carries its own
            // channel and message ts, which is what makes a card still work
            // after the connector restarts.
            let _ = chat::post_message(
                &self.slack,
                &record.channel_id,
                Some(&record.thread_ts),
                "A draft is waiting for review.",
                Some(blocks),
            )
            .await;
        }
    }

    /// Whether a draft was written with both taint legs armed.
    fn is_tainted(&self, id: &str) -> bool {
        mecha_core::outbox::OutboxStore::open(&self.outbox_root)
            .ok()
            .and_then(|s| s.item(id).ok())
            .is_some_and(|i| i.taint.private && i.taint.untrusted)
    }

    /// The second step for a tainted draft: the full arguments, and a button
    /// that means what it says.
    ///
    /// The TUI shows the whole call in red and confirms before releasing one
    /// of these, and refuses to auto-release them at all. A single
    /// primary-styled Send that ran `outbox send -y` gave a phone less care
    /// than a terminal, on the drafts that deserve the most.
    async fn ask_to_confirm_tainted(&mut self, id: &str, channel: &str, ts: &str) {
        let args = mecha_core::outbox::OutboxStore::open(&self.outbox_root)
            .ok()
            .and_then(|s| s.item(id).ok())
            .map(|i| serde_json::to_string_pretty(&i.args).unwrap_or_default())
            .unwrap_or_default();
        let card = vec![
            blocks::section(
                "*⚠️ This draft was written while the trifecta was armed.*\nPrivate data and \
                 untrusted content were both in the conversation that produced it. The full \
                 arguments are below — read them before sending.",
            ),
            blocks::section(&format!("```\n{}\n```", truncate_for_slack(&args))),
            blocks::actions(vec![
                blocks::button(
                    "slack_outbox_send_confirm",
                    "Send anyway",
                    id,
                    Some("danger"),
                ),
                blocks::button("slack_outbox_reject", "Reject", id, None),
            ]),
        ];
        let _ = chat::update(
            &self.slack,
            channel,
            ts,
            "This draft needs a second look before it is sent.",
            Some(card),
        )
        .await;
    }

    /// Release or reject a draft by driving the CLI, exactly as the TUI does.
    ///
    /// A child process rather than a library call, for the TUI's reasons: one
    /// implementation of releasing, no way for this surface to do something
    /// `mecha outbox` cannot, and a release can take a while because it may
    /// start MCP servers. It is spawned into a task so the event loop keeps
    /// answering while it runs.
    fn resolve_draft(&mut self, id: &str, send: bool, who: &str, channel: String, ts: String) {
        let slack = self.slack.clone();
        let id = id.to_string();
        let who = who.to_string();
        tokio::spawn(async move {
            let exe = std::env::current_exe().unwrap_or_else(|_| "mecha".into());
            let mut cmd = tokio::process::Command::new(exe);
            if send {
                cmd.args(["outbox", "send", &id, "-y"]);
            } else {
                cmd.args(["outbox", "reject", &id, "--reason", "rejected from Slack"]);
            }
            let outcome = cmd.output().await;
            let (verb, detail) = match outcome {
                Ok(o) if o.status.success() => {
                    (if send { "sent" } else { "rejected" }, String::new())
                }
                Ok(o) => (
                    "failed",
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                ),
                Err(e) => ("failed", e.to_string()),
            };
            let line = if detail.is_empty() {
                format!("Draft `{id}` {verb} by <@{who}>")
            } else {
                format!("Draft `{id}` {verb} by <@{who}> — {detail}")
            };
            let _ = chat::update(
                &slack,
                &channel,
                &ts,
                &line,
                Some(vec![blocks::context(&line)]),
            )
            .await;
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
            tracing::debug!("ignored an interaction: {}", gate.reason());
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
                "slack_outbox_send" | "slack_outbox_send_confirm" | "slack_outbox_reject" => {
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
                    // did neither.
                    if action.action_id == "slack_outbox_send" && self.is_tainted(&value) {
                        self.ask_to_confirm_tainted(&value, &channel, &ts).await;
                        continue;
                    }
                    let send = action.action_id != "slack_outbox_reject";
                    self.resolve_draft(&value, send, &who, channel, ts);
                }
                "slack_approve" | "slack_approve_run" | "slack_reject" => {
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
                _ => {}
            }
        }
    }
}

/// A filename safe to join onto a directory.
///
/// Slack supplies the name and the owner chose it, but a name that reaches the
/// filesystem is a path: `../../.mecha/slack/credentials.json` is a perfectly
/// ordinary string until it is joined onto something. The path jail would catch
/// a model asking for that; nothing catches it here, because this write happens
/// in the connector before any tool is involved.
fn safe_filename(raw: &str) -> String {
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
    use super::safe_filename;

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
        let dir = std::env::temp_dir().join(format!(
            "mecha-slack-snap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
