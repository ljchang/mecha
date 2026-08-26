//! `mecha questions` — what the agent is stuck on, and answering it.
//!
//! The review surface for [`mecha_core::questions`], and the other half of
//! D13. A delegated run that needed a decision ended rather than waited; this
//! is where the owner finds the question and where the answer goes back in.
//!
//! **Answering is resuming**, and there is deliberately no other way to reply.
//! The answer becomes the next user turn of the conversation that asked, in
//! the jail it asked from, with its plan restored — because a reply that did
//! not continue the conversation would be a note to nobody. That also means
//! this verb spends the model, which is why the store's own `answer` does not:
//! recording an answer and acting on it are separate, so a listing can never
//! start a run.

use anyhow::{bail, Context, Result};
use mecha_core::questions::{ParkingAsker, Question, QuestionStore};
use mecha_core::session::{Record, Session};

use crate::setup::{staged_ids, withhold_tool};
use crate::{setup, GlobalOpts};

#[derive(clap::Args, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(clap::Subcommand, Debug)]
pub enum Cmd {
    /// What is waiting on you (default).
    List {
        /// Include answered and abandoned questions — the history.
        #[arg(long)]
        all: bool,
    },
    /// One question in full, with where it came from.
    Show { question: String },
    /// Answer, and resume the run that asked.
    Answer {
        question: String,
        /// Nobody is at a terminal — take the unattended posture.
        ///
        /// **The same D3 decision `tasks work --unattended` carries, and it
        /// is load-bearing rather than ergonomic here.** Resuming builds an
        /// interactive agent, which installs `TerminalApprover`; detached
        /// from the web its stdin is `/dev/null`, `read_line` returns
        /// `Ok(0)`, and EOF-is-not-consent turns every approval into
        /// `Decision::Deny("the user declined this call")` — which the loop
        /// renders `"Denied by the user: "`, the exact string the learning
        /// miner reads a *correction* out of. So the run would not merely
        /// fail; it would teach mecha rules from a person who was never
        /// asked. `ModeApprover` says no in the machine's voice instead, and
        /// D3 stands: a run gets more permission by acquiring a human, never
        /// by asking for one.
        #[arg(long)]
        unattended: bool,
        /// Your answer. Trailing words are joined, so it needs no quoting.
        #[arg(required = true, num_args = 1..)]
        answer: Vec<String>,
    },
    /// Give up on a question without answering it. The run is not resumed.
    Abandon { question: String },
}

pub async fn run(global: &GlobalOpts, args: Args) -> Result<()> {
    match args.cmd.unwrap_or(Cmd::List { all: false }) {
        Cmd::List { all } => list(all),
        Cmd::Show { question } => show(&question),
        Cmd::Answer {
            question,
            unattended,
            answer,
        } => answer_and_resume(global, &question, &answer.join(" "), unattended).await,
        Cmd::Abandon { question } => {
            let q = store()?.abandon(&question)?;
            // `short`, not `&id[..8]` — the head of a `Session::new_id` is a
            // date, so two questions abandoned on one day printed the same
            // handle. The one call site that had its own copy of the bug.
            println!("{} abandoned — the run was not resumed", short(&q.id));
            Ok(())
        }
    }
}

fn store() -> Result<QuestionStore> {
    QuestionStore::open(QuestionStore::default_root()?)
}

fn short(id: &str) -> &str {
    QuestionStore::short(id)
}

fn list(all: bool) -> Result<()> {
    let store = store()?;
    let items = if all {
        store.items()?
    } else {
        store.open_items()?
    };
    if items.is_empty() {
        println!(
            "{}",
            if all {
                "no questions have ever been asked"
            } else {
                "nothing is waiting on you"
            }
        );
        return Ok(());
    }
    for q in &items {
        // The taint marker is not decoration. A question is composed by a
        // model that may have been reading third-party text, so an armed
        // snapshot means "review the question itself", not only its answer.
        let mark = if q.taint.untrusted { " ⚠" } else { "" };
        println!(
            "{:<10}  {:<10}{}  {}",
            short(&q.id),
            q.status,
            mark,
            q.summary()
        );
    }
    println!("\n{} question(s)", items.len());
    Ok(())
}

fn show(id: &str) -> Result<()> {
    let q = store()?.find(id)?;
    warn_if_tainted(&q);
    println!("{}\n", q.question.trim());
    for opt in &q.options {
        println!("  - {opt}");
    }
    if !q.options.is_empty() {
        println!();
    }
    println!("id       {}", q.id);
    println!("status   {}", q.status);
    println!("asked    {}", q.asked_at);
    if let Some(t) = &q.task_id {
        println!("task     {t}");
    }
    println!("session  {}", q.session_id);
    if let Some(a) = &q.answer {
        println!(
            "\nanswered {}\n{a}",
            q.answered_at.as_deref().unwrap_or("?")
        );
    }
    Ok(())
}

/// Say so when the asking conversation held third-party content.
///
/// **A question is an inbound request for information, and an injected model
/// asks well-formed questions.** "Which credential should I use for the
/// deploy?" is indistinguishable in shape from a reasonable one. The interlock
/// still governs whatever the run does afterwards, so this is not the only
/// control — but the owner is the one composing the answer, and they are
/// entitled to know what was in the room when the question was written.
///
/// It warns and does not block, unlike `outbox send`'s confirmation. The
/// difference is who authors the bytes: a release executes words the *model*
/// wrote, where an answer is the owner's own, typed deliberately. A y/n prompt
/// immediately after they typed it would buy nothing and teach them to hit
/// enter — which is the habit the outbox's confirmations then have to fight.
fn warn_if_tainted(q: &Question) {
    if q.taint.untrusted {
        eprintln!(
            "⚠ third-party content was in this conversation when the question was asked.\n\
             \x20 Read the question itself as possibly not the assistant's own — an injected\n\
             \x20 run asks well-formed questions too.\n"
        );
    }
}

async fn answer_and_resume(
    global: &GlobalOpts,
    id: &str,
    answer: &str,
    unattended: bool,
) -> Result<()> {
    let store = store()?;
    let q = store.find(id)?;
    if !q.is_open() {
        bail!("question {} is already {}", short(&q.id), q.status);
    }
    warn_if_tainted(&q);

    // **D11 on this door too: one live run per task.**
    //
    // `tasks work` has refused to start a second run on a held task since
    // phase 1; the resume — which is also a run, on the same session, moving
    // the same board — never learned it, and unconditionally
    // `mark_running`s below. Answering two questions on one session is not a
    // theoretical path: a run can park more than one, and the surfaces now
    // *tell* the owner to answer the others too. Three things break
    // concurrently: the second `mark_running` overwrites the first's pid, so
    // `tasks stop` ends the wrong run; whichever finishes first calls
    // `clear`, disarming the stop button for the one still going; and both
    // record against an independently loaded transcript, so `record_run`
    // sees divergence and writes a `rewrite` that clobbers the other's
    // history.
    //
    // **A resumed delegation takes a seat too when nobody is watching.** It
    // is still a background run on the shared model, so leaving it uncounted
    // would make answering a question the way to exceed the pool. What it
    // does *not* do is wait: the answer is already stored, so a full pool
    // reports and this verb can simply be run again. An attended resume takes
    // none, like an attended `tasks work` — the reserve exists for the person
    // at the keyboard.
    let _seat = if unattended {
        match super::tasks::permits()?.take(&format!("answer {}", q.session_id))? {
            Ok(held) => Some(held),
            Err(busy) => anyhow::bail!(
                "the model is busy with {} background run(s) — your answer is saved; run \
                 `mecha questions answer` again when one ends",
                busy.len()
            ),
        }
    } else {
        None
    };

    // Checked before `setup::prepare`, which pays an MCP startup: a refusal
    // that costs a second is a refusal people read.
    //
    // Keyed on the task rather than the session because the marker store is,
    // and because that is what `stop` addresses. A question with no task
    // cannot be guarded here — no front-end produces one today, since both
    // askers are installed with a task in hand.
    if let Some(task) = q.task_id.as_deref() {
        if super::tasks::markers()?.running(task).is_some() {
            bail!(
                "a run is already working {task} — answering now would start a second \
                 one on the same conversation. `mecha tasks stop {task}` ends it \
                 first, or wait for it to finish and answer then."
            );
        }
    }

    // The recorded jail, for the outbox's reason one door over: a continuation
    // resolved against a different root is a different run.
    let mut opts = global.clone();
    if opts.workspace.is_none() {
        opts.workspace = q.workspace.clone();
    }
    let mut prepared = setup::prepare(&opts, !unattended).await?;

    // The same refusal `tasks work` makes, for the same reason: this is that
    // delegated run continuing, with the same tools and the same ability to
    // send. Without the route a `mail_send` here delivers for real, and
    // answering a question is exactly the moment nobody is re-reading config.
    if prepared.agent.context().outbox.is_none() {
        bail!(
            "resuming needs the outbox: name your send tools in `[outbox] tools` so drafts \
             are staged instead of delivered"
        );
    }
    if q.task_id.is_some() {
        let holders = setup::subagents_holding(&prepared.config, "kg_task_update");
        if !holders.is_empty() {
            bail!(
                "subagent(s) {} allowlist `kg_task_update`, so the resumed run could close its \
                 own task. Remove it from their `tools` in config first.",
                holders.join(", ")
            );
        }
    }

    let dir = Session::default_dir()?;
    let path = Session::find(&dir, &q.session_id)
        .with_context(|| format!("the session that asked ({}) is gone", q.session_id))?;
    let (meta, prior) = Session::load(&path)?;

    // D15 — the plan comes back with the conversation, so the resumed run
    // reads its own list rather than rebuilding one from the summary.
    if let Some(todo) = &prepared.todo {
        let ws = prepared.agent.context().tools.workspace.clone();
        if let Some(n) = todo.rehydrate(&ws, &prior.messages) {
            eprintln!("restored a plan of {n} item(s)");
        }
    }

    let session = Session { meta, path };

    // The same surface the asking run had: it still may not close its own
    // task (D6), and it may still need to ask again — a task can take more
    // than one round trip, and the second question is as legitimate as the
    // first.
    // Kept, not discarded: the same handle that takes the tool off the
    // model's surface is what the harness moves the board with (D5/D6).
    let update = match q.task_id.as_deref() {
        Some(_) => withhold_tool(prepared.agent.registry_mut(), "kg_task_update").map(|(_, t)| t),
        None => None,
    };
    let questions = std::sync::Arc::new(store);
    let asker = std::sync::Arc::new(ParkingAsker::new(
        std::sync::Arc::clone(&questions),
        &q.session_id,
        q.task_id.clone(),
    ));
    prepared.agent.registry_mut().insert(std::sync::Arc::new(
        mecha_core::tool::ask::AskUserTool::new(
            std::sync::Arc::clone(&asker) as std::sync::Arc<dyn mecha_core::tool::ask::Asker>
        ),
    ));
    if let Some(route) = &prepared.agent.context().outbox {
        route.set_session_id(&q.session_id);
    }

    // After the withhold and the insert, never before: `RunConfig::of` reads
    // the registry at call time, so an earlier append records a surface that
    // never existed — one still holding `kg_task_update`, still missing
    // `ask_user`. `mecha replay` rebuilds the run from this.
    session.append(&Record::Config(mecha_core::session::RunConfig::of(
        &prepared.agent,
        &prepared.config,
        &prepared.provider_name,
    )))?;

    let staged_before = staged_ids(&q.session_id);
    let tctx = std::sync::Arc::clone(&prepared.agent.context().tools);

    // The resumed run is a run: it sets `waiting_on` to the agent, so every
    // surface renders "mecha is on it" *and a stop button* — and without a
    // marker that button found nothing to stop, printed "nothing is running",
    // and left the card pulsing at a run that carried on. A run visible as
    // running must be stoppable by the same token.
    let run_markers = super::tasks::markers()?;
    if let Some(task) = q.task_id.as_deref() {
        // Named with its transcript, like the first run of this task was:
        // a resumed delegation is a run like any other, and a `resume`
        // elsewhere must be able to see that this process holds the file.
        run_markers.mark_running_for(task, None, Some(&q.session_id))?;
    }

    // The ball comes back to the agent for as long as the run lasts. Without
    // this the board would say the task is waiting on *you* while a run is
    // actively working it — the same lie `tasks work` moves the status to
    // avoid, one door over.
    if let (Some(update), Some(task)) = (&update, q.task_id.as_deref()) {
        if let Err(e) =
            super::tasks::move_task(update, &tctx, task, "waiting", super::tasks::AGENT, None).await
        {
            eprintln!("warning: the board still says you hold {task}: {e:#}");
        }
    }

    // Recorded before the run, so a run that dies still leaves the question
    // closed and the answer on file. The alternative loses the owner's words
    // and asks them the same thing again.
    let recorded_q = questions.answer(&q.id, answer)?;

    let mut convo = prior;
    let text = format!(
        "You asked: {}\n\nThe owner answers: {answer}",
        recorded_q.question.trim()
    );
    // **Folded, not pushed.** The asking run was stopped by a cancel, and a
    // cancel keeps the partial turn — so depending on where it landed, the
    // recorded transcript may end on the *user* message carrying the tool
    // results of the turn the question was asked in. Pushing there makes two
    // user messages in a row, which is invalid on the Anthropic backend and
    // merely tolerated by llama-server: the shape that passes locally and
    // 400s in production. `append_user_text` is the same fold steering uses,
    // and it is a no-op difference when the transcript ends on an assistant
    // turn, which is what the first live run happened to produce.
    // Snapshotted **before** the fold, and nothing is appended by hand. A
    // fold modifies the last message in place, so an explicit
    // `Record::Message` would duplicate it and a snapshot taken afterwards
    // would hide it — `record_run` compares the two states and writes an
    // append or a `rewrite` as the change actually was. That is the same
    // mechanism compaction relies on, used here for the same reason: the file
    // must not be able to disagree with the conversation.
    let recorded = convo.messages.clone();
    mecha_core::agent::append_user_text(&mut convo.messages, text);

    eprintln!("resuming {} with the answer", q.session_id);
    // Steerable for the same reason the first pass is: this run is just as
    // detached, just as long, and the owner is just as absent from it.
    let steering = std::sync::Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::<String>::new(),
    ));
    let mut cx = (**prepared.agent.context())
        .clone()
        .with_queued_input(std::sync::Arc::clone(&steering));
    // A resumed delegation is a delegation: same ceiling, or answering a
    // question would drop the run back to the terminal's twelve turns.
    if cx.budget.max_turns.is_none() {
        cx.budget.max_turns = Some(super::tasks::TASK_MAX_TURNS);
    }
    let outcome = crate::interrupt::run_interruptible_watching(
        &prepared.agent,
        &cx,
        &mut convo,
        None,
        q.task_id.as_deref().map(|task| {
            let m = super::tasks::markers().ok();
            let id = task.to_string();
            std::sync::Arc::new(move || m.as_ref().is_some_and(|m| m.cancel_requested(&id)))
                as std::sync::Arc<dyn Fn() -> bool + Send + Sync>
        }),
        q.task_id
            .as_deref()
            .map(|task| super::tasks::steer_pump(task, steering)),
    )
    .await;
    if let Some(task) = q.task_id.as_deref() {
        run_markers.clear(task);
    }
    session.record_run(&recorded, &convo)?;
    session.append(&Record::Taint(convo.taint))?;
    // **How the run went, beside what it said.** A resumed delegation is a
    // run like any other, and every other front-end writes this — without it
    // a task's transcript records what was said and nothing about whether the
    // saying worked, so `runlog` cannot see delegations at all and a card has
    // no way to tell "it finished" from "it broke" (D16's rule that `failed`
    // must never render as `idle`).
    //
    // After the transcript, never before, and by reference so the failure
    // branch below still owns the error: an outcome describes a transcript
    // that is already safe on disk.
    if let Ok(o) = &outcome {
        if let Err(e) = session.record_outcome(o) {
            eprintln!("warning: this run's outcome was not recorded: {e:#}");
        }
    }
    asker.stamp_taint(convo.taint);

    if let Err(e) = outcome {
        // The same restore `tasks work` does, for the same reason: this run
        // moved the board to say the agent had the task, and a task pinned to
        // the agent with no process running is the queue growing for a reason
        // nobody can see. It was missing here because the move back sat below
        // the bail — the hazard is identical, the door is different.
        if let (Some(update), Some(task)) = (&update, q.task_id.as_deref()) {
            if let Err(restore) =
                super::tasks::move_task(update, &tctx, task, "waiting", super::tasks::OWNER, None)
                    .await
            {
                eprintln!("warning: the board still says the agent has {task}: {restore:#}");
            }
        }
        bail!("the resumed run failed: {e:#}");
    }

    // And back to you when it stops, for the reason it went the other way.
    if let (Some(update), Some(task)) = (&update, q.task_id.as_deref()) {
        if let Err(e) =
            super::tasks::move_task(update, &tctx, task, "waiting", super::tasks::OWNER, None).await
        {
            eprintln!(
                "warning: the board still says {} has {task}: {e:#}",
                super::tasks::AGENT
            );
        }
    }

    let staged: Vec<String> = staged_ids(&q.session_id)
        .into_iter()
        .filter(|s| !staged_before.contains(s))
        .collect();
    println!();
    if !staged.is_empty() {
        println!(
            "{} draft(s) staged — `mecha outbox` to review, nothing has been sent",
            staged.len()
        );
    }
    if let Some(next) = asker.parked().first() {
        if let Ok(nq) = questions.get(next) {
            println!("it needs another answer:\n\n  {}\n", nq.question.trim());
            println!("  mecha questions answer {} \"...\"", short(&nq.id));
            return Ok(());
        }
    }
    if let Some(task) = &q.task_id {
        println!("{task} is `waiting` — `mecha tasks set {task} --status done` when you agree");
    }
    Ok(())
}
