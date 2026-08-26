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
        Cmd::Answer { question, answer } => {
            answer_and_resume(global, &question, &answer.join(" ")).await
        }
        Cmd::Abandon { question } => {
            let q = store()?.abandon(&question)?;
            println!(
                "{} abandoned — the run was not resumed",
                &q.id[..8.min(q.id.len())]
            );
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

async fn answer_and_resume(global: &GlobalOpts, id: &str, answer: &str) -> Result<()> {
    let store = store()?;
    let q = store.find(id)?;
    if !q.is_open() {
        bail!("question {} is already {}", short(&q.id), q.status);
    }
    warn_if_tainted(&q);

    // The recorded jail, for the outbox's reason one door over: a continuation
    // resolved against a different root is a different run.
    let mut opts = global.clone();
    if opts.workspace.is_none() {
        opts.workspace = q.workspace.clone();
    }
    let mut prepared = setup::prepare(&opts, true).await?;

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
    session.append(&Record::Config(mecha_core::session::RunConfig::of(
        &prepared.agent,
        &prepared.config,
        &prepared.provider_name,
    )))?;

    // The same surface the asking run had: it still may not close its own
    // task (D6), and it may still need to ask again — a task can take more
    // than one round trip, and the second question is as legitimate as the
    // first.
    if q.task_id.is_some() {
        withhold_tool(prepared.agent.registry_mut(), "kg_task_update");
    }
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
    let staged_before = staged_ids(&q.session_id);

    // Recorded before the run, so a run that dies still leaves the question
    // closed and the answer on file. The alternative loses the owner's words
    // and asks them the same thing again.
    let recorded_q = questions.answer(&q.id, answer)?;

    let mut convo = prior;
    let user = mecha_core::message::Message::user(format!(
        "You asked: {}\n\nThe owner answers: {answer}",
        recorded_q.question.trim()
    ));
    convo.push(user.clone());
    session.append(&Record::Message(user))?;
    let recorded = convo.messages.clone();

    eprintln!("resuming {} with the answer", q.session_id);
    let outcome = crate::interrupt::run_interruptible(
        &prepared.agent,
        prepared.agent.context(),
        &mut convo,
        None,
    )
    .await;
    session.record_run(&recorded, &convo)?;
    session.append(&Record::Taint(convo.taint))?;
    asker.stamp_taint(convo.taint);

    if let Err(e) = outcome {
        bail!("the resumed run failed: {e:#}");
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
