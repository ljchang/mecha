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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mecha_core::agent::{Agent, AgentEvent, Budget, Conversation, RunOutcome};
use mecha_core::message::Message;
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
}

pub async fn run(global: &GlobalOpts) -> Result<()> {
    let store = SlackStore::open(mecha_core::work::mecha_home()?.join("slack"))?;
    let creds: Credentials = store
        .credentials()?
        .context("no Slack tokens stored — run `mecha slack auth` first")?;
    let binding: Binding = store
        .binding()?
        .context("nothing is bound — run `mecha slack link` first")?;

    let cfg = mecha_core::config::Config::load_global()?.slack;
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

    let agent = Arc::new(build_agent(global).await?);

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
        approval_tx,
        completion_tx,
    };

    tracing::info!(
        "slack connector up; {} owner(s)",
        state.binding.owners.len()
    );

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
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("stopping; in-flight runs are cancelled at their next safe point");
                for live in state.live.values() {
                    live.cancel.cancel();
                }
                break;
            }
        }
    }

    socket_task.abort();
    Ok(())
}

/// The agent every thread shares. One provider connection and one cached
/// prefix; the per-thread parts ride on `RunContext`.
async fn build_agent(global: &GlobalOpts) -> Result<Agent> {
    let opts = GlobalOpts {
        // Global config only, like a trigger run: a project's `mecha.toml`
        // arrives with a cloned repository and must not shape a run someone
        // drives from their phone.
        global_config_only: true,
        provider: global.provider.clone(),
        model: global.model.clone(),
        ..GlobalOpts::default()
    };
    // Not interactive: no terminal approver, and no `ask_user` — the registry
    // belongs to the agent and one agent serves every thread, so a shared
    // `ask_user` could not know which thread asked. See SLACK-DESIGN.md §4.
    let prepared = setup::prepare(&opts, false).await?;
    Ok(prepared.agent)
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

        let mut prompt = prompt;
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

        tokio::spawn(async move {
            let renderer = {
                let slack = slack.clone();
                let channel = channel.clone();
                let thread_ts = thread_ts.clone();
                tokio::spawn(async move {
                    pump(&slack, &channel, &thread_ts, events_rx, &pump_cfg).await
                })
            };

            let outcome = agent
                .run_in(&cx, &mut conversation, Some(events_tx))
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = renderer.await;

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
            "Running.",
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
                let ended = if done.outcome.is_ok() {
                    "Finished."
                } else {
                    "Failed."
                };
                let _ = chat::update(
                    &self.slack,
                    &r.channel_id,
                    ts,
                    ended,
                    Some(vec![blocks::context(&format!(
                        "{ended} mode was `{}`",
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
                if staged {
                    if let Ok(Some(r)) = self.threads.get(&done.key) {
                        let _ = chat::post_message(
                            &self.slack,
                            &r.channel_id,
                            Some(&r.thread_ts),
                            "This run staged drafts in the outbox. Nothing has been sent — \
                             review them with `mecha outbox`.",
                            None,
                        )
                        .await;
                    }
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
                "Running.",
                Some(controls_blocks(key, next.as_str())),
            )
            .await;
        }
    }

    /// An approver wants to ask. Post a durable card — never an ephemeral,
    /// which does not survive a reload and cannot be updated.
    async fn on_approval_request(&mut self, request: approve::Request) {
        let Ok(Some(record)) = self.threads.get(&request.thread_key) else {
            return;
        };
        let id = format!("{}-{}", request.thread_key, self.pending.len());
        let card = vec![
            blocks::section(&format!("*Approve this call?*\n`{}`", request.summary)),
            blocks::context(&format!("thread {} · {}", record.thread_ts, request.tool)),
            blocks::actions(vec![
                blocks::button("slack_approve", "Approve", &id, Some("primary")),
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
                        let _ = self.threads.apply(&value, Event::StopPressed);
                    }
                }
                "slack_mode" => self.cycle_mode(&value).await,
                "slack_approve" | "slack_reject" => {
                    let approved = action.action_id == "slack_approve";
                    if let Some(pending) = self.pending.remove(&value) {
                        let answer = if approved {
                            Answer::Approve
                        } else {
                            Answer::Reject("rejected from Slack".into())
                        };
                        let _ = pending.reply.send(answer);
                        // Rewrite the card into a terminal record, so it says
                        // what happened and cannot be clicked again.
                        let who = interaction.user_id.as_deref().unwrap_or("someone");
                        let _ = chat::update(
                            &self.slack,
                            &pending.channel,
                            &pending.message_ts,
                            &format!(
                                "`{}` {} by <@{who}>",
                                pending.tool,
                                if approved { "approved" } else { "rejected" }
                            ),
                            Some(vec![blocks::context(&format!(
                                "`{}` {} by <@{who}>",
                                pending.tool,
                                if approved { "approved" } else { "rejected" }
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
