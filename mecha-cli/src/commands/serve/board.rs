//! Phase 3 of `mecha serve`, continued: tasks and notes.
//!
//! Both stores are the knowledge graph's, reached exactly as the TUI reaches
//! them — every read and mutation drives the CLI (`mecha tasks …`,
//! `mecha kg …`), which reaches the graph over its MCP surface. The board's
//! rules ride along for free: nothing here confirms (every status is one tap
//! from where it was, and the tool surface has no delete), and a note is
//! evidence — the graph's extractor mines it, and candidates wait in the
//! owner's review queue, never entering belief directly.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::review::{self_json, verb};

type St = State<super::WebState>;

/// GET /api/tasks — the board, straight off `kg_task_list`'s own JSON, with
/// how each delegated run *went* attached (D16).
///
/// The child pays an MCP startup (~a second against the graph server), which
/// is why the timeout is generous and the page shows its own loading state.
///
/// **Attached here rather than fetched per card**, and that is the whole of
/// D16 rather than a performance note. The rule is that no two card states
/// render alike and that `failed` must never render as `idle` — a board that
/// paints every task as idle and then flickers into its real state is that
/// rule broken for the first second, on the surface people glance at. One
/// request, every card correct on first paint.
///
/// Bounded the way `runlog`'s scans are: only sessions the board actually
/// names, and only their outcome records — no message parsing, no
/// `Session::load`. Measured, a task transcript is 9–76 KB and a board names
/// a handful, so this is a few tens of milliseconds beside a child process
/// that pays an MCP startup.
pub async fn tasks(State(state): St) -> Response {
    // **`--closed`, because the page has a view for closed tasks.** The
    // drawer's `done` filter selects `done | dropped`, and without this the
    // list it filters never contains either — so that view has been
    // structurally empty since it shipped, in the way that reads as "you have
    // finished nothing" rather than as a filter that cannot match. No other
    // view widens: the three above it select `next|inbox`, `scheduled` and
    // `waiting`, none of which a closed task can be.
    let mut board = match self_json(&state, &["tasks", "list", "--closed", "--json"]).await {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    };
    if let Ok(dir) = mecha_core::session::Session::default_dir() {
        // **Off the executor.** Even resolved to one `stat` and one read per
        // delegated task, this is blocking filesystem work inside an async
        // handler, and `Tasks.svelte` polls this route every five seconds
        // while a run is in flight. `spawn_blocking` keeps a slow disk from
        // stalling the SSE streams and approval cards this process is also
        // holding.
        board = match tokio::task::spawn_blocking(move || {
            attach_runs(&mut board, &dir);
            board
        })
        .await
        {
            Ok(b) => b,
            // The board itself is the answer; the run summaries are a
            // decoration on it. Losing the join is not a reason to fail the
            // page — but it is a reason to say so rather than serve a board
            // whose every card silently reads `idle`.
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("reading how the runs went: {e}\n"),
                )
                    .into_response()
            }
        };
    }
    Json(board).into_response()
}

/// Put each task's run beside it, and **only where there was one**.
///
/// A task nobody ever handed to the agent gets no `run` key at all, rather
/// than an empty one. That absence is what the card reads as `idle`, and it
/// is the common case — most of a board is things the owner typed. An
/// object saying "no outcome recorded" on every hand-written task would put
/// every one of them into the state reserved for a delegation that broke.
fn attach_runs(board: &mut serde_json::Value, dir: &std::path::Path) {
    let Some(items) = board["items"].as_array_mut() else {
        return;
    };
    // **Is the run the board describes actually alive?** Asked of the run
    // markers, because the board cannot answer it: `tasks work` restores the
    // status on every exit path it controls, and a `SIGKILL` controls none of
    // them — so a killed delegation leaves `waiting_on` naming the agent
    // forever, and every surface that reads only the board renders it as
    // *mecha is on it*, pulsing at a run that died. The marker knows within
    // seconds (it is pid-checked and sweeps itself), and nothing was asking.
    //
    // One directory read for the whole board rather than one per row: the
    // sweep inside `live` rewrites the directory, so calling it per task
    // would have each row re-scan what the last one just cleaned.
    let alive: std::collections::HashSet<String> = crate::commands::tasks::markers()
        .map(|m| {
            m.dir()
                .read_dir()
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| {
                            let name = e.file_name();
                            let name = name.to_str()?.strip_suffix(".running")?.to_string();
                            m.running(&name).map(|_| name)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    for task in items.iter_mut() {
        // **A board that says the agent has it, with no run behind it.** Not
        // rewritten here — this reads stores and never heals them, doctor's
        // rule — but named, so the card can render *the run is gone* instead
        // of a run in flight. The field is absent in the ordinary case, so a
        // page that does not know about it behaves exactly as it did.
        if task["waiting_on"].as_str() == Some(crate::commands::tasks::AGENT) {
            if let Some(id) = task["id"].as_str() {
                if !alive.contains(id) {
                    task["stalled"] = serde_json::Value::Bool(true);
                }
            }
        }
        let Some(id) = task["session"].as_str().map(str::to_string) else {
            continue;
        };
        task["run"] = run_summary(dir, &id);
    }
}

/// Is `id` exactly one ordinary path component — never a root, a `..`, empty,
/// or more than one segment?
///
/// A denylist of specific characters was the first cut here and a review
/// caught it citing the wrong file for its own precedent (`valid_key`, an
/// **allowlist**, lives in `chat.rs`) while pointing at the deeper gap: a
/// denylist is complete only for the platform it was checked against.
/// `std::path::Component::Normal` is what the standard library itself calls
/// "an ordinary path segment", so asking it directly — one `Normal`
/// component and nothing else — is correct on whatever platform this runs
/// on rather than needing its own list of what that platform's separators
/// and prefixes are.
fn is_bare_path_component(id: &str) -> bool {
    let mut components = std::path::Path::new(id).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

/// How the run on a session went — or that nobody can say.
///
/// **Three answers, never two.** An outcome record says how the loop stopped;
/// its *absence* on a session that exists says the run did not get as far as
/// recording one — a crash, a kill, or a transcript written before the record
/// existed. That third answer is reported as itself and never folded into
/// either of the others: doctor's rule that a rate over a zero denominator is
/// `None` and never zero, one surface over, where the consequence of getting
/// it wrong is a delegation that died reading exactly like one nobody started
/// (or, in the other direction, every run from before this shipped shouting
/// that it failed).
///
/// The last outcome wins. A session is resumed by answering a question, and
/// what the owner is deciding about is where it stands now.
///
/// **Resolved by name, not by search.** `Session::find` scans the whole
/// session directory and reads the header of every transcript in it, because
/// it accepts an id *prefix*; the board always stores a full id, and
/// `Session::create` names a file `<id>.jsonl`, so a `join` answers it. The
/// first cut called `find` once per delegated task — 423 header reads apiece
/// on this box, on a route polled every five seconds — while carrying a
/// comment claiming it was bounded. The scan survives only as the fallback
/// for a stored id that is somehow not a filename.
fn run_summary(dir: &std::path::Path, session_id: &str) -> serde_json::Value {
    // `session_id` is the board's own `session` field off `kg_task_list` — a
    // read on somebody else's store, not a value this process minted — and
    // it reaches this join with nothing between them. `Path::join` discards
    // `dir` entirely for an absolute argument, so an id shaped like
    // `/etc/passwd` or containing `..` would otherwise be resolved with no
    // containment check at all. `is_bare_path_component` refuses it the same
    // way `chat.rs`'s `valid_key` refuses a session key before it becomes a
    // directory component: a real session id is never a path.
    if !is_bare_path_component(session_id) {
        // Not silent: a `session` field shaped like a path means the graph
        // store is corrupt or being actively pushed at, and the refusal
        // otherwise renders identically to an ordinary swept transcript —
        // "an unreadable store reports as a dash, never as zero," one
        // finding over. Costs nothing on the common path, where this never
        // fires.
        tracing::warn!("refusing a session id shaped like a path: {session_id:?}");
        return serde_json::json!({ "recorded": false, "transcript": false });
    }
    let direct = dir.join(format!("{session_id}.jsonl"));
    let path = if direct.is_file() {
        direct
    } else {
        match mecha_core::session::Session::find(dir, session_id) {
            Ok(p) => p,
            // The transcript is gone. Not "the run failed" and not "it
            // finished": the record this answer is made of is missing.
            Err(_) => return serde_json::json!({ "recorded": false, "transcript": false }),
        }
    };
    // `last_outcome`, not `outcomes`: this needs where the session stands
    // now, and an outcome is appended last, so scanning backwards finds it in
    // one parse instead of one per line of a transcript that is mostly
    // messages.
    let Ok(Some(stats)) = mecha_core::session::Session::last_outcome(&path) else {
        return serde_json::json!({ "recorded": false, "transcript": true });
    };
    serde_json::json!({
        "recorded": true,
        "transcript": true,
        // `None` when the loop never named one; the page renders that as
        // unknown rather than as completion, for the reason above.
        "stop_cause": stats.stop_cause,
        // **`cut_short`, which is `StopCause::cut_short()` and not
        // `is_early()`.** The two disagree on exactly one variant —
        // `Interrupted` — and that is the variant a person pressing stop
        // produces, which is the system working and must never read as a
        // failure. Shipping `is_early()` under this name put the broad
        // definition behind the narrow word: harmless only because the page
        // happened to test `stop_cause === 'interrupted'` on the line before
        // it read this, so reordering two lines or adding one consumer would
        // have rendered every stopped run as failed. `cut_short()` exists
        // precisely because there were once two definitions and they
        // disagreed; using the other one here recreated that.
        "cut_short": stats.stop_cause.is_some_and(|c| c.cut_short()),
        // The silent failure: the model stopped of its own accord with its
        // last call failed and answered as though it had not. An observation
        // rather than an error — a task whose right answer is "that file does
        // not exist" ends this way legitimately — so the card shows it and
        // does not rule on it.
        "ended_on_failed_call": stats.ended_on_failed_call,
        // The evidence `ready for review` is supposed to arrive with.
        "turns": stats.turns,
        "tool_calls": stats.tool_calls,
        // Never `tool_errors` alone: a denial is the approver doing its job,
        // and averaging it into failure is how a read-only run reports the
        // harness working as a harness fault.
        "tool_errors": stats.tool_errors,
        "tool_denied": stats.tool_denied,
        "tool_staged": stats.tool_staged,
    })
}

#[derive(serde::Deserialize)]
pub struct TaskSetBody {
    pub task: String,
    pub status: Option<String>,
    pub due: Option<String>,
    pub defer: Option<String>,
    pub context: Option<String>,
}

/// POST /api/tasks/set — lifecycle and scheduling, through the one driver.
/// Omitted fields stay omitted: the unset-vs-clear distinction is the
/// tool's, and flattening it here would wipe due dates on status changes.
pub async fn task_set(State(state): St, Json(body): Json<TaskSetBody>) -> Response {
    let mut args: Vec<&str> = vec!["tasks", "set", &body.task];
    if let Some(v) = &body.status {
        args.extend(["--status", v]);
    }
    if let Some(v) = &body.due {
        args.extend(["--due", v]);
    }
    if let Some(v) = &body.defer {
        args.extend(["--defer", v]);
    }
    if let Some(v) = &body.context {
        args.extend(["--context", v]);
    }
    if args.len() == 3 {
        return (StatusCode::BAD_REQUEST, "nothing to change\n").into_response();
    }
    verb(&state, &args).await
}

#[derive(serde::Deserialize)]
pub struct TaskWorkBody {
    pub task: String,
    pub note: Option<String>,
}

/// POST /api/tasks/work — hand a task to the agent.
///
/// **Detached and unattended**, which are two decisions rather than one.
/// Detached because this is a whole agent run that can take twenty minutes,
/// and a request holding a connection open for that is a request that times
/// out — the mail page's drafting verbs spawn the same way, and the board is
/// the meeting point rather than the child.
///
/// Unattended because the approver in this process cannot reach a child
/// process: the web approval cards belong to a chat session's `RunContext`,
/// and a spawned `mecha tasks work` builds its own agent. So the run takes
/// the trigger posture — reads run, sends stage, anything needing approval is
/// refused — which is D3's rule exactly: *a run gets more permission by
/// acquiring a human, never by asking for one*. The way to give this run more
/// is to open its conversation, where `ask` mode's cards already work.
///
/// Nothing is returned but an acknowledgement. What happened is on the board:
/// `waiting_on` names the agent while the run is in flight and the owner when
/// it stops, so the page derives the card's state from the store rather than
/// from anything the run says about itself (D5, D16).
pub async fn task_work(State(state): St, Json(body): Json<TaskWorkBody>) -> Response {
    let _ = state; // the board is the meeting point, not this process
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    let mut argv: Vec<String> = vec![
        "tasks".into(),
        "work".into(),
        task.into(),
        "--unattended".into(),
    ];
    if let Some(note) = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        argv.push("--note".into());
        argv.push(note.into());
    }
    super::mail::spawn_detached_note(
        &argv,
        "handed to mecha — the board says who has it, and the conversation is on the card",
    )
}

#[derive(serde::Deserialize)]
pub struct TaskPlanBody {
    pub session: String,
}

/// POST /api/tasks/plan — the plan a task run wrote, read from its transcript.
///
/// **Not from the shared agent's todo tool**, and the difference is not an
/// implementation detail. A chat session runs *in this process*, so its plan
/// is in memory keyed by the session's jail. A `tasks work` run is a separate
/// process: nothing of its list is here, and asking the tool would answer
/// with an empty list for every task on the board.
///
/// The transcript is the meeting point, exactly as it is for the board and
/// the outbox — and `TodoTool::from_transcript` already knows how to read a
/// plan back out of one, including the compacted case where the `todo` calls
/// are gone and only the carried block survives. Built for resuming (D15),
/// reused here without change.
pub async fn task_plan(State(state): St, Json(body): Json<TaskPlanBody>) -> Response {
    let _ = state;
    let id = body.session.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which session?\n").into_response();
    }
    let found = mecha_core::session::Session::default_dir()
        .and_then(|dir| mecha_core::session::Session::find(&dir, id))
        .and_then(|path| mecha_core::session::Session::load(&path));
    match found {
        Ok((_, convo)) => Json(serde_json::json!({
            "todo": mecha_core::tool::todo::TodoTool::from_transcript(&convo.messages)
                .unwrap_or_default(),
        }))
        .into_response(),
        // A session that cannot be read is not a task with no plan. The page
        // shows nothing either way, but saying which is which keeps a broken
        // store from reading as a quiet one.
        Err(e) => (StatusCode::NOT_FOUND, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TaskSourceBody {
    pub task: String,
}

/// POST /api/tasks/source — what the task was captured from, in full.
///
/// **The pointer is on the board and the bytes come from the source**, which
/// is the same split `/api/outbox/{id}` uses for a draft: ids on the wire,
/// content from the store, because a reviewer reading one thing while acting
/// on another is what these surfaces exist to prevent. The page already holds
/// `captured_from` from `/api/tasks` — enough to decide whether to *offer* a
/// way back — and asks here only when somebody actually opens it, so a board
/// of twenty tasks does not fetch twenty mail threads.
///
/// Through the CLI like every other verb here, so the one reader per kind is
/// `mecha tasks source`'s and the page cannot drift from the terminal. Its
/// budget is `self_text`'s minutes rather than seconds: following a mail
/// pointer pays an MCP startup and may refresh an OAuth token.
///
/// **The answer is third-party text and reaches the page as text**, never as
/// anything a run reads back. Nothing here re-enters a prompt and no taint
/// moves: these bytes were accounted for when the mail was first read.
pub async fn task_source(State(state): St, Json(body): Json<TaskSourceBody>) -> Response {
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    match super::mail::self_text(&state, &["tasks", "source", task]).await {
        Ok(text) => text.into_response(),
        // A source that cannot be read is not a task without one. The card
        // says which, because a broken pointer and a hand-typed task look
        // identical from a blank panel.
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TaskStopBody {
    pub task: String,
}

/// POST /api/tasks/stop — ask the run working a task to stop.
///
/// Synchronous, unlike starting one: writing a sentinel is instant, and the
/// answer worth having is whether there was anything to stop. `tasks stop`
/// says so rather than pretending, and that distinction has to survive the
/// wire — a page told "ok" for a stop that stopped nothing will show the run
/// as ended while it carries on.
///
/// It does not kill anything. The run finishes its current step and keeps the
/// partial turn, exactly as Ctrl-C does in a terminal — Copilot's documented
/// recovery for a stuck session is to unassign and reassign, and there is no
/// excuse for reproducing that.
pub async fn task_stop(State(state): St, Json(body): Json<TaskStopBody>) -> Response {
    let task = body.task.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    verb(&state, &["tasks", "stop", task]).await
}

#[derive(serde::Deserialize)]
pub struct TaskChatBody {
    pub task: String,
}

/// POST /api/tasks/chat — open the conversation about a task.
///
/// **D2, which the detached path had quietly dropped.** *"The run is a
/// conversation from the start, not a fire-and-forget job"* — and `ask mecha`
/// spawned an unattended child, so the board moved to `waiting` on the tap
/// and the only conversation available was the one you could read afterwards.
/// This opens the same chat surface everything else here uses: the model's
/// transcript streams to the page, voice and uploads work because they are
/// the chat's, a question is a card, and typing at a run in flight is
/// steering the loop already understands.
///
/// **Nothing on the board moves.** `waiting_on` names who has the ball, and
/// while the owner is in the conversation they do — so the card stays where
/// it was instead of vanishing out of the view it was tapped in. The session
/// id is recorded on the task, because that link is how the card offers the
/// way back, and it is set by the harness rather than by the model (D5).
///
/// Returns the session id and not the key: the page navigates to
/// `#chat/<id>`, whose `resume` already hands back the live key when this
/// process holds the conversation. One door into the chat view.
pub async fn task_chat(State(state): St, Json(body): Json<TaskChatBody>) -> Response {
    let task_id = body.task.trim().to_string();
    if task_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    match super::chat::open_task_conversation(&state, &task_id).await {
        Ok(session) => Json(serde_json::json!({ "session": session })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TaskHandoverBody {
    pub task: String,
    pub note: Option<String>,
}

/// POST /api/tasks/handover — let the conversation carry on without you.
///
/// **The crossing between the two postures**, and the only place the choice
/// is made. Planning happens in a chat session in this process, where a
/// question is a card and an answer is a second away; autonomous work happens
/// in a detached child, where a question **ends the run** and waits in the
/// question store until morning. Neither is more capable — the chat loop runs
/// unattended just as long — the difference is what happens when nobody is
/// there, and that is a fact about the owner rather than about the run.
///
/// What crossing buys, precisely: the child survives a restart of this
/// process, its questions park instead of expiring on a 120-second card, and
/// the board can honestly say `waiting on mecha`, because now it is.
///
/// Release first, then spawn. Reversing them would have two processes willing
/// to append to one transcript for as long as the child takes to start, which
/// is the failure every `resume` surface here is guarded against.
pub async fn task_handover(State(state): St, Json(body): Json<TaskHandoverBody>) -> Response {
    let task = body.task.trim().to_string();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    let session = match super::chat::release_task_conversation(&state, &task).await {
        Ok(id) => id,
        Err(e) => return (StatusCode::CONFLICT, format!("{e:#}\n")).into_response(),
    };
    let mut argv: Vec<String> = vec![
        "tasks".into(),
        "work".into(),
        task,
        "--unattended".into(),
        "--again".into(),
        "--resume".into(),
        session,
    ];
    if let Some(note) = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        argv.push("--note".into());
        argv.push(note.into());
    }
    super::mail::spawn_detached_note(
        &argv,
        "handed over — it carries on from here, and its questions will wait for you",
    )
}

#[derive(serde::Deserialize)]
pub struct TaskSteerBody {
    pub task: String,
    pub text: String,
}

/// POST /api/tasks/steer — redirect a run in flight without stopping it.
///
/// **The one thing a detached run could not be told.** Everything else the
/// card offers is a fact about the board; this is text for the run itself,
/// and the run is in another process — so it travels the way `stop` does, as
/// a file the runner polls, and lands in the same `queued_input` a TUI's
/// typed steering goes into. The loop sees what it always saw: an
/// instruction arriving on the message that carries the tool results.
///
/// Synchronous, unlike `work`. Queueing is a file write, and the answer the
/// page needs — *was anything actually running?* — is known immediately;
/// spawning it detached would report success for a run that had already
/// ended, which is the confusion `stop`'s non-zero exit was written to stop.
pub async fn task_steer(State(state): St, Json(body): Json<TaskSteerBody>) -> Response {
    let task = body.task.trim();
    let text = body.text.trim();
    if task.is_empty() {
        return (StatusCode::BAD_REQUEST, "which task?\n").into_response();
    }
    if text.is_empty() {
        // Nothing to say is not an instruction. Refused here rather than
        // queued, because an empty steer would still cost the run a turn's
        // attention on a user message with no content in it.
        return (StatusCode::BAD_REQUEST, "steer it with what?\n").into_response();
    }
    verb(&state, &["tasks", "steer", task, text]).await
}

#[derive(serde::Deserialize)]
pub struct TaskAddBody {
    pub name: String,
    pub due: Option<String>,
    pub context: Option<String>,
}

/// POST /api/tasks/add — capture; lands in `inbox`, committed to nothing.
pub async fn task_add(State(state): St, Json(body): Json<TaskAddBody>) -> Response {
    let name = body.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "a task needs a name\n").into_response();
    }
    let mut args: Vec<&str> = vec!["tasks", "add", name];
    if let Some(v) = &body.due {
        args.extend(["--due", v]);
    }
    if let Some(v) = &body.context {
        args.extend(["--context", v]);
    }
    verb(&state, &args).await
}

#[derive(serde::Deserialize)]
pub struct ParseBody {
    pub text: String,
}

/// POST /api/tasks/parse — what a capture says about *when* (B2).
///
/// **In-process, and the only handler here that spawns nothing.** Every other
/// verb on this page drives a `mecha …` child, which is right when the child
/// owns something — the board, the outbox, a run. This owns nothing: it is
/// `capture::find_when`, a pure function over a string, called while somebody
/// is typing. A child process per keystroke would pay a fork and an MCP
/// startup to answer a question with no state in it, which is how a feature
/// meant to make capture cheaper makes it slower than the form it replaces.
///
/// The `/triggers` rule is untouched by that: it says a UI must not reach past
/// what the command line can do, and `mecha tasks add` runs the same function
/// on the same string. Nothing here is reachable only from a browser.
///
/// **Returns the span, never a rewritten name.** The page draws a dismissable
/// chip and sends the owner's words unchanged; Things' side of the one
/// disagreement the surveyed apps have.
pub async fn task_parse(State(state): St, Json(body): Json<ParseBody>) -> Response {
    let _ = state;
    match mecha_core::capture::find_when(&body.text) {
        Some(w) => Json(serde_json::json!({
            "due": w.due,
            "text": w.text,
            "start": w.start,
            "end": w.end,
        }))
        .into_response(),
        // A capture with no date is the ordinary case, not a failure — most
        // things somebody types have no deadline in them.
        None => Json(serde_json::json!(null)).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct NoteBody {
    pub text: String,
}

/// POST /api/notes — the owner's own words, staged to the graph as an
/// episode (`kg note`); entities named in it are linked on landing.
///
/// `--` before the text because a note is prose and prose can start with a
/// dash; without it clap reads the owner's first word as a flag it does not
/// have and refuses the capture.
pub async fn note(State(state): St, Json(body): Json<NoteBody>) -> Response {
    let text = body.text.trim();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "an empty note\n").into_response();
    }
    verb(&state, &["kg", "note", "--", text]).await
}

#[derive(serde::Deserialize)]
pub struct NoteEditBody {
    /// The episode key from a `kg_notes` row — not the `uid`, which names
    /// the row but cannot write to it.
    pub source_id: String,
    pub text: String,
}

/// POST /api/notes/edit — rewrite one note in place.
///
/// The whole edit lives in `mecha kg note --edit`, including the part that
/// matters (the note's original `occurred_at` is read back and preserved, so
/// fixing a typo does not restamp when the thing happened). This handler
/// only checks that both halves are present: a note page that could reach a
/// rewrite the terminal cannot is the shape `/tasks` and `/triggers` both
/// refuse.
pub async fn note_edit(State(state): St, Json(body): Json<NoteEditBody>) -> Response {
    let id = body.source_id.trim();
    let text = body.text.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which note?\n").into_response();
    }
    if text.is_empty() {
        // Emptying a note is not editing it, and there is deliberately no
        // delete here: a note is evidence, and what the graph derived from it
        // is retracted in review, not by blanking the record.
        return (
            StatusCode::BAD_REQUEST,
            "an empty note — clear it in review instead\n",
        )
            .into_response();
    }
    verb(&state, &["kg", "note", "--edit", id, "--", text]).await
}

#[derive(serde::Deserialize)]
pub struct FindQuery {
    pub q: String,
}

/// GET /api/find?q= — `kg search` (the TUI /find verb), for the notes page's search box.
pub async fn find(
    State(state): St,
    axum::extract::Query(query): axum::extract::Query<FindQuery>,
) -> Response {
    let q = query.q.trim();
    if q.is_empty() {
        return Json(serde_json::json!([])).into_response();
    }
    match self_json(&state, &["kg", "search", q, "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct EntityQuery {
    pub name: String,
}

/// GET /api/entity — everything the graph knows about one entity, the
/// `kg_entity` envelope verbatim: node, facts (tier and polarity
/// included), episodes, interaction recency, coverage. Ambiguity is the
/// tool's own disambiguation envelope, passed through for the page to
/// render as a candidate list.
///
/// This page existing is itself part of review-on-use: opening an entity
/// is a review trigger, so the unreviewed (`tier != "reviewed"`) facts it
/// shows carry verdict buttons — wired to `/api/queue/shadow/verdict`,
/// the same owner-shaped path the review page uses.
pub async fn entity(
    State(state): St,
    axum::extract::Query(q): axum::extract::Query<EntityQuery>,
) -> Response {
    let name = q.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "name is required
",
        )
            .into_response();
    }
    match self_json(&state, &["kg", "entity", name, "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            format!(
                "{e:#}
"
            ),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct RelatedQuery {
    pub name: String,
    pub hops: Option<u8>,
}

/// GET /api/related — the bounded neighborhood around one node, the
/// `kg_related` envelope verbatim. This is the one graph *rendering* the
/// evidence supports: 1–2 hops around the entity being read, never a global
/// view (`NOTES-GRAPH-DESIGN.md` §2.2 — a graph rendering is a scoped
/// answer to a question, not a homepage).
pub async fn related(
    State(state): St,
    axum::extract::Query(q): axum::extract::Query<RelatedQuery>,
) -> Response {
    let name = q.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required\n").into_response();
    }
    // Clamped here as well as tool-side, because this string becomes an argv.
    let hops = q.hops.unwrap_or(1).clamp(1, 2).to_string();
    match self_json(&state, &["kg", "related", name, "--hops", &hops, "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct TimelineQuery {
    pub name: String,
}

/// GET /api/timeline — bi-temporal history for one entity, the `kg_timeline`
/// envelope verbatim: superseded facts beside what replaced them, and the
/// episode timeline. The entity page shows only `valid_from` without this;
/// history is the other half of the store's answer.
pub async fn timeline(
    State(state): St,
    axum::extract::Query(q): axum::extract::Query<TimelineQuery>,
) -> Response {
    let name = q.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required\n").into_response();
    }
    match self_json(&state, &["kg", "timeline", name, "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

/// GET /api/notes — recent notes, the `kg_notes` envelope verbatim.
pub async fn notes(State(state): St) -> Response {
    match self_json(&state, &["kg", "notes", "--json"]).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::StopCause;
    use mecha_core::session::{Record, RunStats, Session, SessionMeta};

    /// A private directory per test, on this repo's own convention — no
    /// `tempfile` dependency in the binary that reads the owner's stores.
    fn tmpdir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mecha-board-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session(dir: &std::path::Path, id: &str) -> Session {
        Session::create(
            dir,
            SessionMeta {
                id: id.into(),
                created_at: chrono::Utc::now(),
                provider: "local".into(),
                model: "a-model".into(),
                workspace: dir.to_path_buf(),
                title: Some("task: something".into()),
            },
        )
        .unwrap()
    }

    /// **The third answer, and the reason it exists.** A transcript with no
    /// outcome record is a run that never got as far as saying how it went —
    /// a crash, a kill, or a session written before the record existed. It is
    /// not a failure and it is not a completion, and folding it into either
    /// is how every delegation from before this shipped would shout that it
    /// broke (or how one that really did break would read as idle, which is
    /// the rule D16 states outright).
    #[test]
    fn a_run_that_recorded_no_outcome_says_so_rather_than_guessing() {
        let dir = tmpdir("unrecorded");
        session(&dir, "20260826T090000-aaaaaaaa");
        let v = run_summary(&dir, "20260826T090000-aaaaaaaa");
        assert_eq!(v["transcript"], serde_json::json!(true));
        assert_eq!(v["recorded"], serde_json::json!(false));
        assert!(
            v["stop_cause"].is_null(),
            "and it must not invent one — unknown is the answer"
        );
    }

    /// A session the board names that is not on disk is a third thing again:
    /// the record this answer would be made of is missing.
    #[test]
    fn a_missing_transcript_is_not_a_failed_run() {
        let dir = tmpdir("missing");
        let v = run_summary(&dir, "20260826T090000-nosuch01");
        assert_eq!(v["transcript"], serde_json::json!(false));
        assert_eq!(v["recorded"], serde_json::json!(false));
    }

    /// `session_id` comes off `kg_task_list` — a read on somebody else's
    /// store — and `Path::join` discards `dir` entirely for an absolute
    /// argument, so a value shaped like a path used to reach the
    /// filesystem with nothing in between. Refused before that join, not
    /// after it.
    ///
    /// **A denylist, not an allowlist.** Kept beside the real regression
    /// test below because `is_bare_path_component`'s own doc names the
    /// difference: these six inputs are the ones a hand-picked list would
    /// name, and every one of them is still worth pinning down even though
    /// none of them can distinguish the fixed code from the unfixed code on
    /// an *empty* directory (see the next test for why that distinction
    /// needs a planted file, not an empty one).
    ///
    /// **Not `"a\\b"`.** On this platform `\` is not a separator, so
    /// `is_bare_path_component` correctly *accepts* it as one ordinary
    /// segment (asserted directly in that function's own test) — a case
    /// asserting it must be refused here would describe Windows, not the
    /// platform this suite actually runs on.
    #[test]
    fn a_session_id_shaped_like_a_path_is_refused_not_joined() {
        let dir = tmpdir("path-safety");
        for hostile in ["../../etc/passwd", "/etc/passwd", "..", ".", "", "a/b"] {
            let v = run_summary(&dir, hostile);
            assert_eq!(
                v,
                serde_json::json!({ "recorded": false, "transcript": false }),
                "{hostile:?} must be refused, not joined onto a directory"
            );
        }
    }

    /// **The regression test the guard actually needs.** Found on review:
    /// the test above passes against the *unguarded* code too, because
    /// `tmpdir` starts empty — every hostile id falls through to
    /// `Session::find` on nothing and returns byte-identical JSON either
    /// way, so a future refactor could delete the guard with the suite
    /// green. Proving the guard does something means a transcript that
    /// genuinely exists at the escaped location: a real session, one
    /// directory above `dir`, reached by `../<that directory's name>/<id>`.
    /// Without the guard, `direct.is_file()` is true for that joined path
    /// and this returns `{"recorded": false, "transcript": true}` — the
    /// planted session has no outcome recorded, but it is *found*. With the
    /// guard, `is_bare_path_component` refuses the escape before the join
    /// is ever built, and the answer is the same "nothing to see" as every
    /// other refusal.
    #[test]
    fn an_id_that_would_have_escaped_to_a_real_transcript_is_still_refused() {
        let dir = tmpdir("path-safety-inside");
        let outside = tmpdir("path-safety-outside");
        let planted_id = "20260826T090000-aaaaaaaa";
        session(&outside, planted_id);

        // Sanity: the planted session is genuinely readable at the
        // unescaped path, so the assertion below is about the guard and
        // not about a fixture that never existed.
        assert!(outside.join(format!("{planted_id}.jsonl")).is_file());

        let escape = format!(
            "../{}/{planted_id}",
            outside.file_name().unwrap().to_str().unwrap()
        );
        // Sanity in the other direction: the escape really does resolve to
        // the planted file when nothing stops it, which is what makes the
        // assertion below a test of the guard rather than of geometry that
        // happens not to line up.
        assert!(
            dir.join(format!("{escape}.jsonl")).is_file(),
            "the escape must reach the planted transcript for this test to mean anything"
        );
        assert_eq!(
            run_summary(&dir, &escape),
            serde_json::json!({ "recorded": false, "transcript": false }),
            "an id that resolves outside `dir` must be refused even when \
             something real sits at the far end of it"
        );
    }

    /// The state D16 says must never render as `idle`. `NoOutput` is a real
    /// failure the loop names — the run produced nothing and did not recover
    /// — and the card has to be able to tell it from a completion.
    #[test]
    fn a_run_the_harness_cut_short_is_distinguishable_from_one_that_finished() {
        let dir = tmpdir("cutshort");
        let s = session(&dir, "20260826T090000-bbbbbbbb");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::NoOutput),
            tool_calls: 3,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-bbbbbbbb");
        assert_eq!(v["recorded"], serde_json::json!(true));
        assert_eq!(v["cut_short"], serde_json::json!(true));

        let s = session(&dir, "20260826T090000-cccccccc");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            tool_calls: 3,
            tool_staged: 1,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-cccccccc");
        assert_eq!(v["cut_short"], serde_json::json!(false));
        assert_eq!(v["tool_staged"], serde_json::json!(1));
    }

    /// **A person pressing stop is not a failed run**, and `cut_short` is the
    /// field that decides which. `is_early()` and `cut_short()` differ on
    /// exactly this variant, so the first cut's `is_early()` reported every
    /// stopped run as cut short — masked only by the page testing
    /// `stop_cause` first, which means reordering two lines of Svelte or
    /// adding a second consumer would have rendered a Ctrl-C as "the run
    /// failed". Doctor excludes `Interrupted` from its own thresholds for
    /// this reason; so does this.
    #[test]
    fn a_run_a_person_stopped_is_not_reported_as_cut_short() {
        let dir = tmpdir("stopped");
        let s = session(&dir, "20260826T090000-ffffffff");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Interrupted),
            tool_calls: 4,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-ffffffff");
        assert_eq!(
            v["cut_short"],
            serde_json::json!(false),
            "Interrupted is the system working — `is_early()` would say true here"
        );
        assert_eq!(v["stop_cause"], serde_json::json!("interrupted"));
    }

    /// **The path comes from the id, not from a search.** `Session::find`
    /// reads the header of every transcript in the directory because it
    /// accepts a prefix; the board stores full ids, and this route is polled
    /// every five seconds. The assertion is that a decoy session in the same
    /// directory is never opened to answer about another one — which a scan
    /// cannot promise.
    #[test]
    fn a_session_is_found_by_name_rather_than_by_reading_every_transcript() {
        let dir = tmpdir("byname");
        let wanted = session(&dir, "20260826T090000-11111111");
        wanted
            .append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                turns: 3,
                ..RunStats::default()
            }))
            .unwrap();
        // A sibling that must not be consulted, and would be by a scan.
        let decoy = dir.join("20260826T090000-22222222.jsonl");
        std::fs::write(
            &decoy,
            "this is not JSON at all
",
        )
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-11111111");
        assert_eq!(v["turns"], serde_json::json!(3));
        // And a torn sibling does not make the answer about it fail either.
        let v = run_summary(&dir, "20260826T090000-22222222");
        assert_eq!(v["transcript"], serde_json::json!(true));
        assert_eq!(v["recorded"], serde_json::json!(false));
    }

    /// **A task nobody delegated has no run, not an empty one.** The card
    /// reads the key's absence as `idle`, and most of a board is things the
    /// owner typed — so a summary attached to every row would put every
    /// hand-written task into the state reserved for a delegation that broke.
    #[test]
    fn a_task_that_was_never_delegated_gets_no_run_at_all() {
        let dir = tmpdir("never");
        let mut board = serde_json::json!({
            "items": [
                { "id": "task-1", "session": null },
                { "id": "task-2" },
                { "id": "task-3", "session": "20260826T090000-eeeeeeee" },
            ]
        });
        session(&dir, "20260826T090000-eeeeeeee");
        attach_runs(&mut board, &dir);
        assert!(board["items"][0].get("run").is_none());
        assert!(board["items"][1].get("run").is_none());
        assert_eq!(
            board["items"][2]["run"]["transcript"],
            serde_json::json!(true),
            "and the one that was delegated is answered for"
        );
    }

    /// **The last outcome wins.** A session is resumed by answering a
    /// question, so a task that asked, was answered and then finished must
    /// not still be reported by the run that stopped to ask — which is the
    /// state the owner already dealt with.
    #[test]
    fn a_resumed_session_is_reported_by_where_it_stands_now() {
        let dir = tmpdir("resumed");
        let s = session(&dir, "20260826T090000-dddddddd");
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Interrupted),
            ..RunStats::default()
        }))
        .unwrap();
        s.append(&Record::Outcome(RunStats {
            stop_cause: Some(StopCause::Completed),
            turns: 9,
            ..RunStats::default()
        }))
        .unwrap();
        let v = run_summary(&dir, "20260826T090000-dddddddd");
        assert_eq!(v["cut_short"], serde_json::json!(false));
        assert_eq!(v["turns"], serde_json::json!(9));
    }
}
