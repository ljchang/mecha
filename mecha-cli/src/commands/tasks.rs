//! `mecha tasks` — the GTD board in the knowledge graph, from the terminal.
//!
//! The command line does everything first and the `/tasks` modal drives it,
//! on the front door's rule: one implementation per verb, and no way for a UI
//! to do something the terminal cannot.
//!
//! **The board is reached the same way the model reaches it — through the MCP
//! tool surface.** `mecha-cli` has no dependency on the graph and does not
//! gain one here: `kg_task_list` already answers in JSON, so this driver reads
//! the same bytes the loop would, and a deployment that renames the server or
//! turns `prefix_tools` on keeps working because the lookup matches on the
//! suffix. Reaching past the tools into the SQLite file would be a second
//! implementation of a schema that lives in another repository.
//!
//! No approver and no interlock, deliberately, exactly as `mecha mail task`
//! does it: the person at the keyboard *is* the authority a tool approval
//! exists to consult, and the board reaches nobody — `kg_task_*` carries
//! `openWorldHint: false`, so none of it is a send.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::setup::{find_tool, staged_ids, tool_ctx, withhold_tool};
use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// The board: actionable statuses first, then by due date (default).
    List {
        /// Also show done and dropped tasks — the history.
        #[arg(long)]
        closed: bool,
        /// Machine output: the tool's own JSON, which is what the modal reads.
        #[arg(long)]
        json: bool,
    },
    /// Capture a task. Lands in `inbox` — captured, not yet committed to.
    Add {
        /// The task, phrased as an action. Trailing words are joined, so it
        /// needs no quoting.
        #[arg(required = true, num_args = 1..)]
        name: Vec<String>,
        /// YYYY-MM-DD, `today`, `tomorrow`, or `+Nd`.
        #[arg(long)]
        due: Option<String>,
        /// Parent project — must name a node the graph already has.
        #[arg(long)]
        project: Option<String>,
        /// GTD context tag, e.g. `@email`, `@lab`.
        #[arg(long)]
        context: Option<String>,
    },
    /// Move a task through its lifecycle, or edit its scheduling.
    ///
    /// Omitting a field leaves it untouched; passing an empty string clears
    /// it. That distinction is the tool's, and it is passed through rather
    /// than reinterpreted here — a driver that turned "unset" into "clear"
    /// would silently wipe a due date every time somebody changed a status.
    Set {
        /// The task's node id, e.g. `task-1a2b3c4d`, from `tasks list`.
        task: String,
        /// next | inbox | scheduled | waiting | done | dropped.
        #[arg(long)]
        status: Option<String>,
        /// New due date; `""` clears it.
        #[arg(long)]
        due: Option<String>,
        /// Hide until this date; `""` clears it.
        #[arg(long)]
        defer: Option<String>,
        /// New context tag; `""` clears it.
        #[arg(long)]
        context: Option<String>,
        /// Who has the ball — a name the graph knows, or `@owner` for
        /// yourself; `""` clears it.
        #[arg(long)]
        waiting_on: Option<String>,
    },
    /// Hand a task to the agent: a seeded run in its own session.
    ///
    /// The task becomes the thing the run is *about* — a fresh conversation,
    /// a prompt built from the record, and anything outbound staged in the
    /// outbox rather than sent. The agent is delegated to, never assigned:
    /// the task stays yours, and it cannot close it.
    Work {
        /// The task's node id, e.g. `task-1a2b3c4d`, from `tasks list`.
        task: String,
        /// Extra instruction for this run. Trailing words are joined.
        #[arg(long, num_args = 1..)]
        note: Vec<String>,
        /// Start a run on a task already handed over.
        #[arg(long)]
        again: bool,
    },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List {
        closed: false,
        json: false,
    }) {
        Cmd::List { closed, json } => list(global, closed, json).await,
        Cmd::Add {
            name,
            due,
            project,
            context,
        } => add(global, &name.join(" "), due, project, context).await,
        Cmd::Set {
            task,
            status,
            due,
            defer,
            context,
            waiting_on,
        } => set(global, &task, status, due, defer, context, waiting_on).await,
        Cmd::Work { task, note, again } => {
            let note = (!note.is_empty()).then(|| note.join(" "));
            work(global, &task, note.as_deref(), again).await
        }
    }
}

/// Call one `kg_task_*` tool and return its parsed answer.
///
/// The absence of the graph is a *named* condition rather than a panic or an
/// empty board: a machine with no `[[mcp]]` graph server has no tasks to show
/// and needs to be told which of those two it is.
async fn call(global: &GlobalOpts, tool: &str, args: Value) -> Result<Value> {
    let prepared = setup::prepare_tools(global, false).await?;
    let found = find_tool(&prepared.registry, tool).with_context(|| {
        format!("no knowledge-graph server in this configuration — `{tool}` is not on the tool surface. Is `[[mcp]]` enabled?")
    })?;
    let out = found.call(args, &tool_ctx(&prepared)).await?;
    if out.is_error {
        bail!("{}: {}", tool, out.content.trim());
    }
    serde_json::from_str(&out.content)
        .with_context(|| format!("{tool} did not answer with JSON: {}", out.content))
}

async fn list(global: &GlobalOpts, closed: bool, as_json: bool) -> Result<()> {
    let board = call(global, "kg_task_list", json!({ "include_closed": closed })).await?;
    if as_json {
        println!("{board}");
        return Ok(());
    }

    let today = board["today"].as_str().unwrap_or_default();
    let items = board["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.is_empty() {
        println!("nothing on the board — `mecha tasks add <what>` captures one");
        return Ok(());
    }

    for t in items {
        let due = match t["due_at"].as_str() {
            Some(d) if t["overdue"].as_bool().unwrap_or(false) => format!("{d} overdue"),
            Some(d) => d.to_string(),
            None => "—".into(),
        };
        println!(
            "{:<10}  {:<18}  {:<52}  {}",
            t["status"].as_str().unwrap_or("?"),
            due,
            t["name"].as_str().unwrap_or(""),
            t["id"].as_str().unwrap_or(""),
        );
        // The tail exists only when it says something. A row of empty columns
        // reads as data about a task that has none.
        let tail: Vec<String> = [
            ("project", "project"),
            ("context", "context"),
            ("waiting_on", "waiting on"),
        ]
        .iter()
        .filter_map(|(key, label)| {
            t[*key]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(|v| format!("{label} {v}"))
        })
        .collect();
        if !tail.is_empty() {
            println!("{:<10}  {}", "", tail.join(" · "));
        }
    }
    println!("\n{} task(s) · today is {today}", items.len());
    Ok(())
}

async fn add(
    global: &GlobalOpts,
    name: &str,
    due: Option<String>,
    project: Option<String>,
    context: Option<String>,
) -> Result<()> {
    let mut args = json!({ "name": name });
    for (key, value) in [("due", due), ("project", project), ("context", context)] {
        if let Some(v) = value {
            args[key] = json!(v);
        }
    }
    let out = call(global, "kg_task_create", args).await?;
    println!(
        "{}  {}",
        out["id"].as_str().unwrap_or("created"),
        // The tool resolves `tomorrow` and `+3d` itself, so report what it
        // stored rather than what was typed — the two differ on purpose.
        match out["due_at"].as_str() {
            Some(d) => format!("due {d}"),
            None => "no due date".into(),
        }
    );
    println!("{name}");
    Ok(())
}

async fn set(
    global: &GlobalOpts,
    task: &str,
    status: Option<String>,
    due: Option<String>,
    defer: Option<String>,
    context: Option<String>,
    waiting_on: Option<String>,
) -> Result<()> {
    let mut args = json!({ "task": task });
    // Every field `kg_task_update` takes, because the modal drives the CLI and
    // a verb the terminal cannot reach is one the UI must not offer either.
    for (key, value) in [
        ("status", status),
        ("due", due),
        ("defer", defer),
        ("context", context),
        ("waiting_on", waiting_on),
    ] {
        if let Some(v) = value {
            args[key] = json!(v);
        }
    }
    if args.as_object().is_some_and(|o| o.len() == 1) {
        bail!(
            "nothing to change — pass at least one of --status, --due, --defer, --context, \
             --waiting-on"
        );
    }
    let out = call(global, "kg_task_update", args).await?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// The agent, as the board names it. A node of kind `agent`, shipped with the
/// graph's schema — deliberately not a person, because delegation is not
/// assignment and responsibility does not transfer.
pub(crate) const AGENT: &str = "mecha";

/// Whoever this graph is about, resolved graph-side so mecha never has to
/// carry the owner's name.
pub(crate) const OWNER: &str = "@owner";

/// Move a task's status and who holds it, through the withheld tool. The
/// harness's hand, not the model's — see [`work`].
///
/// Both in one call because they are one fact about the task: "waiting" with
/// nobody named is the ambiguity this whole phase exists to remove, and two
/// calls could leave the board in exactly that state if the second failed.
pub(crate) async fn move_task(
    update: &std::sync::Arc<dyn mecha_core::tool::Tool>,
    ctx: &mecha_core::tool::ToolCtx,
    task: &str,
    status: &str,
    waiting_on: &str,
) -> Result<()> {
    let out = update
        .call(
            json!({ "task": task, "status": status, "waiting_on": waiting_on }),
            ctx,
        )
        .await?;
    if out.is_error {
        bail!("kg_task_update: {}", out.content.trim());
    }
    Ok(())
}

/// Hand one task to the agent: a seeded run whose subject is the task.
///
/// The shape is `mail draft`'s, one noun over — a fresh `Conversation`, a
/// prompt built deterministically from the record, a recorded session titled
/// after the item, the outbox route bound to that session id, one
/// interruptible run, and a staged-id diff naming exactly what it produced.
/// Three decisions are worth not undoing:
///
/// - **Delegated, never assigned** (D1). The task stays the owner's. The run
///   is a conversation from the start rather than a fire-and-forget job,
///   because the measured case is that a human joins the loop: 52.3% of
///   Copilot's agent PRs over ten months needed direct human commits, and its
///   success ran 86.2% with intervention against 55.1% fully autonomous.
/// - **The agent cannot close its own task** (D6). `kg_task_update` is taken
///   off the surface and kept by this function, so "is it done" is not a
///   question the party under test may answer. A lane must not promote
///   itself — `ladder.rs`'s oldest rule, one store over.
/// - **The seed is built here, from the record** (D4). No model writes it. A
///   model-written seed is an unreviewed instruction entering a privileged
///   run, which is the front door's argument arriving through another door.
async fn work(global: &GlobalOpts, task_id: &str, note: Option<&str>, again: bool) -> Result<()> {
    // **Interactive, unlike `mail draft`**, and the difference is what the two
    // runs are for. Drafting only ever needs to *stage*, so the outbox catches
    // its one outbound act and a blocked write costs nothing. A task run does
    // work — it writes files, it runs commands — and non-interactive means
    // `ModeApprover` refuses every one of them with "nothing is watching to
    // answer". Found by running it: the first live run made three `fs_write`
    // calls, had all three blocked, and reported back the contents of files it
    // had not been allowed to create.
    //
    // This is D3 satisfied rather than bypassed: *a run gets more permission by
    // acquiring a human, never by asking for one*, and the human is the person
    // who just typed the command. `--yes` stays the unattended path, and the
    // phone's button (Phase 4) will acquire its human through the web
    // approver instead.
    let mut prepared = setup::prepare(global, true).await?;

    // `mail draft`'s rule: without the route, a send the model makes actually
    // sends. A task run is exactly the context where that is discovered too
    // late, so it is refused up front rather than run unrouted.
    if prepared.agent.context().outbox.is_none() {
        bail!(
            "handing a task to the agent needs the outbox: name your send tools in \
             `[outbox] tools` so drafts are staged instead of delivered"
        );
    }

    let tctx = std::sync::Arc::clone(&prepared.agent.context().tools);

    // The board is read through the tool surface, like every other verb in
    // this file — no second reader of a schema that lives in another repo.
    let list = find_tool(prepared.agent.registry(), "kg_task_list")
        .cloned()
        .context(
            "no knowledge-graph server in this configuration — `kg_task_list` is not on the \
             tool surface. Is `[[mcp]]` enabled?",
        )?;
    let out = list.call(json!({ "include_closed": true }), &tctx).await?;
    if out.is_error {
        bail!("kg_task_list: {}", out.content.trim());
    }
    let board: Value = serde_json::from_str(&out.content)
        .with_context(|| format!("kg_task_list did not answer with JSON: {}", out.content))?;

    let task = board["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .find(|t| t["id"].as_str() == Some(task_id))
        .cloned()
        .with_context(|| format!("no such task: {task_id} — `mecha tasks list` shows the board"))?;

    let name = task["name"].as_str().unwrap_or("(unnamed)").to_string();
    let was = task["status"].as_str().unwrap_or("inbox").to_string();
    // Captured so a failed run can put the board back exactly as it was.
    // Restoring the status while leaving `waiting_on` pointing at the agent
    // would say the run is still going, which is the more misleading half.
    let was_waiting_on = task["waiting_on"].as_str().unwrap_or("").to_string();

    // A closed task is not work, and reopening it is the owner's decision.
    if matches!(was.as_str(), "done" | "dropped") {
        bail!("{task_id} is {was} — `mecha tasks set {task_id} --status next` reopens it first");
    }
    // D11 as far as Phase 1 can see it. `waiting` is the board's own record
    // that somebody already has the ball; without `waiting_on` (the graph
    // change that is Phase 2) this cannot tell the agent from a person, so it
    // refuses and names the override rather than guessing which.
    if was == "waiting" && !again {
        bail!(
            "{task_id} is already waiting on someone — `mecha tasks work {task_id} --again` \
             starts a run anyway"
        );
    }

    // D5/D6, structurally: the tool that moves a status leaves the model's
    // surface and stays here. Not "the model is told not to" — the model has
    // nothing to call.
    // A child registry is built before this runs and keeps its own handle, so
    // withholding here would leave delegation as the way around D6.
    let holders = setup::subagents_holding(&prepared.config, "kg_task_update");
    if !holders.is_empty() {
        bail!(
            "subagent(s) {} allowlist `kg_task_update`, so a delegated run could close its own \
             task. Remove it from their `tools` in config before handing tasks to the agent.",
            holders.join(", ")
        );
    }
    let Some((_withheld, update)) = withhold_tool(prepared.agent.registry_mut(), "kg_task_update")
    else {
        bail!("`kg_task_update` is not on the tool surface — this run could not be recorded");
    };

    let session_dir = mecha_core::session::Session::default_dir()?;
    let session = mecha_core::session::Session::create(
        &session_dir,
        mecha_core::session::SessionMeta {
            id: mecha_core::session::Session::new_id(),
            created_at: chrono::Utc::now(),
            provider: prepared.provider_name.clone(),
            model: prepared.model.clone(),
            workspace: prepared.workspace.clone(),
            // D10 — the drawer filters on this prefix. A run the owner cannot
            // find is a run they will start twice.
            title: Some(format!("task: {name}")),
        },
    )?;
    if let Some(route) = &prepared.agent.context().outbox {
        route.set_session_id(&session.meta.id);
    }

    // **D13.** `ask_user` is registered here, and only here, because this is
    // the front-end that owns the human — asynchronously. The asker does not
    // block on an answer; it stores the question and ends the run, so the
    // owner can answer at breakfast without a slot and a cached prefix being
    // held all night waiting for them. Registered *after* the session exists,
    // because the session id is how an answer finds its way back.
    let questions = std::sync::Arc::new(mecha_core::questions::QuestionStore::open(
        mecha_core::questions::QuestionStore::default_root()?,
    )?);
    let asker = std::sync::Arc::new(mecha_core::questions::ParkingAsker::new(
        std::sync::Arc::clone(&questions),
        &session.meta.id,
        Some(task_id.to_string()),
    ));
    prepared.agent.registry_mut().insert(std::sync::Arc::new(
        mecha_core::tool::ask::AskUserTool::new(
            std::sync::Arc::clone(&asker) as std::sync::Arc<dyn mecha_core::tool::ask::Asker>
        ),
    ));

    // **After every registry mutation, not before.** `RunConfig::of` snapshots
    // the tool list at call time, so appending it earlier recorded a surface
    // that never existed — one still holding `kg_task_update` and still
    // missing `ask_user`. This record is what makes the withholding evidence
    // rather than a claim, and what `mecha replay` rebuilds the run from, so a
    // record of the wrong surface is worse than none.
    session.append(&mecha_core::session::Record::Config(
        mecha_core::session::RunConfig::of(
            &prepared.agent,
            &prepared.config,
            &prepared.provider_name,
        ),
    ))?;

    let staged_before = staged_ids(&session.meta.id);

    // Moved before the model sees anything, so the board tells the truth for
    // the whole time the run is in flight rather than only after it lands —
    // and names the agent, so the Waiting view distinguishes a task the agent
    // is working from one a person owes you.
    move_task(&update, &tctx, task_id, "waiting", AGENT).await?;

    eprintln!(
        "working {task_id} with {} ({}) · session {}",
        prepared.model, prepared.provider_name, session.meta.id
    );
    eprintln!("{name}");

    let mut convo = mecha_core::agent::Conversation::new();
    let user = mecha_core::message::Message::user(work_prompt(
        &task,
        board["today"].as_str().unwrap_or_default(),
        note,
    ));
    convo.push(user.clone());
    session.append(&mecha_core::session::Record::Message(user))?;
    let recorded = convo.messages.clone();

    let outcome = crate::interrupt::run_interruptible(
        &prepared.agent,
        prepared.agent.context(),
        &mut convo,
        None,
    )
    .await;
    // Not `?`. The status was moved to `waiting` before the run, and every
    // early return between here and the restore below leaves the board saying
    // somebody has the ball with no run in flight — the queue growing for a
    // reason nobody can see, which is what the restore exists to prevent.
    let recording = session
        .record_run(&recorded, &convo)
        .and_then(|()| session.append(&mecha_core::session::Record::Taint(convo.taint)));
    if let Err(e) = &recording {
        eprintln!("warning: this run was not fully recorded: {e:#}");
    }
    // Taint lives on the conversation and a tool cannot see it, so the
    // snapshot is stamped on afterwards. The run ended at the question, so
    // this *is* the question's taint — and where a question shared a turn
    // with something that armed it, over-tainting is the direction to err in.
    asker.stamp_taint(convo.taint);

    // Only a failed *run* restores the status. A run that worked and then
    // failed to record is a task that genuinely is waiting on the owner —
    // drafts may be staged and files written — so putting the board back to
    // `next` would be the lie in the other direction. The warning above is the
    // right response to a torn transcript; a status change is not.
    if let Err(e) = outcome {
        // Nothing happened, so the board must not say something did. A task
        // parked in `waiting` by a run that died is the queue growing for a
        // reason nobody can see — which is the whole failure `/queues` exists
        // to catch, reproduced one store over.
        if let Err(restore) = move_task(&update, &tctx, task_id, &was, &was_waiting_on).await {
            eprintln!("warning: could not put {task_id} back to {was}: {restore:#}");
        }
        bail!("the run failed, nothing staged: {e:#}");
    }

    // The run is over, so the ball is yours — whether it staged drafts, parked
    // a question, or simply reported. Leaving it on the agent would make every
    // finished delegation look like one still running, which is the state the
    // Waiting view now exists to tell apart.
    if let Err(e) = move_task(&update, &tctx, task_id, "waiting", OWNER).await {
        eprintln!("warning: the board still says {AGENT} has {task_id}: {e:#}");
    }

    let staged: Vec<String> = staged_ids(&session.meta.id)
        .into_iter()
        .filter(|id| !staged_before.contains(id))
        .collect();

    println!();
    if staged.is_empty() {
        // Not an error, on `mail draft`'s reasoning: a run that worked the
        // task and had nothing to send has done its job, and inventing a
        // draft to have something to show would be the failure.
        println!("nothing staged.");
    } else {
        println!(
            "{} draft(s) staged — `mecha outbox` to review, nothing has been sent",
            staged.len()
        );
    }
    // A parked question is the *reason* the run stopped, so it leads. The
    // owner's next move is answering it, not disposing of the task, and a
    // task blocked on them reads very differently from one merely finished.
    if let Some(id) = asker.parked().first() {
        if let Ok(q) = questions.get(id) {
            println!("it needs an answer before it can go further:\n");
            println!("  {}", q.question.trim());
            for opt in &q.options {
                println!("    - {opt}");
            }
            let short = mecha_core::questions::QuestionStore::short(&q.id);
            println!("\n  mecha questions answer {short} \"...\"   # resumes the run");
            return Ok(());
        }
    }

    println!("{task_id} is `waiting` — you decide what it becomes next:");
    println!("  mecha tasks set {task_id} --status done     # or next, dropped");
    println!(
        "  mecha chat --resume {}   # keep working on it",
        session.meta.id
    );
    Ok(())
}

/// What the run is asked to do, built from the record and nothing else (D4).
///
/// The board's fields are named rather than pasted as prose, and the standing
/// paragraph tells the model the three things about this run it cannot
/// discover from its own tool list: that its sends stage, that the status is
/// not its to move, and that nobody is watching to answer a question. The
/// last is Phase 1's honest posture — D13 turns "stop and say what you need"
/// into a stored question the owner answers later, and until it exists,
/// stopping is better than guessing.
fn work_prompt(task: &Value, today: &str, note: Option<&str>) -> String {
    let field = |k: &str| task[k].as_str().filter(|v| !v.is_empty());
    let mut p =
        String::from("You have been handed one task from the owner's task board.\n\nTask: ");
    p.push_str(field("name").unwrap_or("(unnamed)"));
    p.push('\n');
    for (key, label) in [
        ("id", "Id"),
        ("status", "Status when handed over"),
        ("project", "Project"),
        ("context", "Context"),
        ("waiting_on", "Waiting on"),
    ] {
        if let Some(v) = field(key) {
            p.push_str(&format!("{label}: {v}\n"));
        }
    }
    if let Some(due) = field("due_at") {
        let overdue = if task["overdue"].as_bool().unwrap_or(false) {
            " (overdue)"
        } else {
            ""
        };
        p.push_str(&format!("Due: {due}{overdue}\n"));
    }
    if !today.is_empty() {
        p.push_str(&format!("Today: {today}\n"));
    }
    if let Some(note) = note {
        p.push_str(&format!("\nThe owner adds: {note}\n"));
    }
    p.push_str(
        "\nWork this task as far as you can. How this run works, so you can plan around it:\n\n\
         - Anything you send or publish is STAGED for the owner to review, not delivered. \
         Draft it properly and say what you staged. Do not look for a way around the queue.\n\
         - You cannot change this task's status and have no tool that does. Whether it is \
         finished is the owner's call, not yours. Report what you did and what is left.\n\
         - If a decision is genuinely the owner's to make, ask with `ask_user`. They are \
         not sitting here, so the run will END on your question and resume later with their \
         answer as the next turn — no time is spent waiting. Ask early rather than guessing. \
         Say where you got to in your last words, because they are what you will be reading \
         when you come back.\n\
         - If this takes more than a few steps, keep a `todo` list. The owner can watch it, \
         and it survives into the conversation if they pick this up later.\n",
    );
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Value {
        json!({
            "id": "task-1a2b3c4d",
            "name": "Follow up with Dirk about the Psych 62 approval",
            "status": "next",
            "project": "teaching",
            "context": "@email",
            "due_at": "2026-09-01",
            "overdue": true
        })
    }

    /// D4: the seed is the record, and every field of it reaches the run.
    #[test]
    fn the_seed_is_built_from_the_record() {
        let p = work_prompt(&task(), "2026-08-26", Some("keep it short"));
        for expect in [
            "Follow up with Dirk about the Psych 62 approval",
            "Id: task-1a2b3c4d",
            "Status when handed over: next",
            "Project: teaching",
            "Context: @email",
            "Due: 2026-09-01 (overdue)",
            "Today: 2026-08-26",
            "The owner adds: keep it short",
        ] {
            assert!(p.contains(expect), "missing {expect:?} in:\n{p}");
        }
    }

    /// An absent field says nothing rather than saying nothing *emptily*: a
    /// line reading `Project: ` is a claim about a task that has none, and
    /// the board's own renderer omits the tail for the same reason.
    #[test]
    fn absent_fields_are_omitted_not_blanked() {
        let bare = json!({"id": "task-9", "name": "Water the plants", "status": "inbox"});
        let p = work_prompt(&bare, "", None);
        for absent in [
            "Project:",
            "Context:",
            "Waiting on:",
            "Due:",
            "Today:",
            "The owner adds:",
        ] {
            assert!(
                !p.contains(absent),
                "{absent:?} should be absent from:\n{p}"
            );
        }
        assert!(p.contains("Water the plants"));
    }

    /// The three things a run cannot learn from its own tool list, and the
    /// reason each is stated. Losing any of them silently changes what an
    /// unattended run believes it may do.
    #[test]
    fn the_run_is_told_what_it_cannot_discover() {
        let p = work_prompt(&task(), "2026-08-26", None);
        assert!(p.contains("STAGED"), "sends stage, and it must know");
        assert!(
            p.contains("cannot change this task's status"),
            "D6: the owner disposes"
        );
        assert!(
            p.contains("ask_user"),
            "D13: asking is the move, and the run ends on it"
        );
        assert!(
            p.contains("END on your question"),
            "the model must know asking costs the run, so it asks early"
        );
    }

    /// A note is the owner's words and is passed through, not summarised or
    /// reworded — the same rule capture follows for a task's own name.
    #[test]
    fn the_note_is_passed_through_verbatim() {
        let note = "ask for the *signed* copy, not the scan";
        let p = work_prompt(&task(), "2026-08-26", Some(note));
        assert!(p.contains(note));
    }
}
