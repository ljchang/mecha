//! The delegation's return path: what the agent is stuck on, answered from
//! the phone.
//!
//! **Phase 4 could start a delegation and not finish one.** *Ask mecha* has
//! been on the task row since 2026-08-26; a run that needed a decision ended
//! and stored its question (D13), and the only surface that could answer was
//! a terminal. So the gesture the phone is for opened a loop the phone could
//! not close, and the board sat in `waiting` with nobody able to say why from
//! the device it was read on.
//!
//! Two shapes, and which applies is decided by who owns the store:
//!
//! - **Store reads for display**, like `review.rs` and unlike `board.rs`. The
//!   question store is mecha's own type, so reading it here is one reader of
//!   one schema. The board goes through the CLI because its store belongs to
//!   another repository and is reached over MCP, where a second reader would
//!   be a second thing to keep true.
//! - **Every mutation is a `mecha …` child**, so nothing is reachable from a
//!   browser that a script cannot do.
//!
//! And the two mutations are spawned differently, because they are different
//! sizes. **Answering is resuming** — the whole point of D13, and therefore a
//! whole agent run that can take twenty minutes, so it detaches exactly as
//! `/api/tasks/work` does and the board is the meeting point. **Abandoning
//! writes one record**, so it is synchronous and says whether it worked,
//! beside `/api/tasks/stop` for the same reason.
//!
//! The detached child is passed `--unattended`, and that flag is load-bearing
//! rather than cosmetic: without it the resume builds an interactive agent,
//! whose `TerminalApprover` reads `/dev/null`, takes EOF as a refusal, and
//! files it as `"Denied by the user: "` — the string the learning miner reads
//! a *correction* out of. A run answered from a phone would have taught mecha
//! rules from a person who was never asked.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use mecha_core::questions::{Question, QuestionStore};

use super::review::verb;

type St = State<super::WebState>;

/// One open question, whole.
///
/// **Everything needed to answer it, on the card**, on the `DraftView` rule
/// one store over: a person deciding without reading is the failure these
/// surfaces exist to prevent, and a phone is where people read least
/// carefully. So the question text is never clipped here — the list is short
/// by construction (a question is one delegation frozen mid-flight, not a
/// feed), and a truncated question is one answered from its first line.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// The short handle the CLI prints, so the two surfaces name the same
    /// thing out loud. The full id rides along for the verbs.
    handle: String,
    id: String,
    question: String,
    options: Vec<String>,
    /// The conversation that asked — the way into it from the drawer.
    session: String,
    /// The board item, when the run was working one.
    task: Option<String>,
    asked_at: String,
    /// Third-party content was in the room when this was composed.
    ///
    /// **Not decoration.** An injected run asks well-formed questions: *"which
    /// credential should I use for the deploy?"* is indistinguishable in shape
    /// from a reasonable one. The owner composes the answer, so they are
    /// entitled to know what was in the conversation when the question was
    /// written — which is `mecha questions`' own `⚠`, on the surface that
    /// cannot see stderr.
    tainted: bool,
}

impl From<&Question> for Row {
    fn from(q: &Question) -> Self {
        Row {
            handle: QuestionStore::short(&q.id).to_string(),
            id: q.id.clone(),
            question: q.question.trim().to_string(),
            options: q.options.clone(),
            session: q.session_id.clone(),
            task: q.task_id.clone(),
            asked_at: q.asked_at.clone(),
            tainted: q.taint.untrusted,
        }
    }
}

/// GET /api/questions — what is waiting on you.
///
/// Open items only. The history is `mecha questions list --all`, and it is
/// deliberately not here: this endpoint answers "what is blocked", and an
/// answered question mixed into that list is a card offering to resume a
/// conversation that already moved on.
///
/// **An absent store is an empty queue and says so; an unreadable one is an
/// error.** Doctor's dash-never-zero rule: "nothing is waiting" and "I could
/// not look" are opposite findings, and a reader that rendered its own
/// failure as a quiet queue would reproduce the bug the queue exists to
/// catch. The store is opened only if it already exists, so drawing the page
/// never creates the thing it is examining.
pub async fn list(State(state): St) -> Response {
    let _ = state;
    let Some(store) = QuestionStore::open_existing_default() else {
        return Json(serde_json::json!({ "items": [] })).into_response();
    };
    match store.open_items() {
        Ok(items) => {
            let rows: Vec<Row> = items.iter().map(Row::from).collect();
            Json(serde_json::json!({ "items": rows })).into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}\n")).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct AnswerBody {
    pub question: String,
    pub answer: String,
}

/// POST /api/questions/answer — answer, and resume the run that asked.
///
/// **Detached, because answering is a whole agent run.** There is no other
/// way to reply: the answer becomes the next user turn of the conversation
/// that asked, in the jail it asked from, with its plan restored. A request
/// holding a connection open for that is a request that times out, so this
/// acknowledges and the board says what happened next — `waiting_on` moves
/// back to the agent for the life of the resumed run, exactly as it does for
/// `/api/tasks/work`.
///
/// The store records the answer *before* the run starts (that is
/// `questions answer`'s own ordering), so a resume that dies still leaves the
/// owner's words on file and does not ask them the same thing twice. That is
/// what makes detaching safe here: the part that must not be lost is written
/// synchronously by the child before it spends a token.
pub async fn answer(State(state): St, Json(body): Json<AnswerBody>) -> Response {
    let _ = state; // the store and the board are the meeting points, not this process
    let id = body.question.trim();
    // Not `is_empty` on the answer alone: a blank answer would resume the
    // conversation with a user turn saying nothing, which is worse than not
    // resuming — the model would guess, which is the measured `ask_user`
    // failure this whole store exists to avoid.
    let answer = body.answer.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which question?\n").into_response();
    }
    if answer.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "an empty answer — say something, or abandon the question\n",
        )
            .into_response();
    }
    let argv: Vec<String> = vec![
        "questions".into(),
        "answer".into(),
        id.into(),
        "--unattended".into(),
        // `--` before the answer: it is the owner's prose, and prose can start
        // with a dash. Without it clap reads their first word as a flag it
        // does not have and refuses the resume — the same guard `/api/notes`
        // carries, for the same reason.
        "--".into(),
        answer.into(),
    ];
    super::mail::spawn_detached_note(
        &argv,
        "resuming — the board says who has it, and the conversation is on the card",
    )
}

#[derive(serde::Deserialize)]
pub struct AbandonBody {
    pub question: String,
}

/// POST /api/questions/abandon — give up on a question. The run is not
/// resumed.
///
/// Synchronous, unlike answering: writing one record is instant, and the
/// answer worth having is whether there was anything to abandon.
///
/// It does not delete. Nothing in this store ever does — an abandoned
/// question is the record of a delegation the owner chose not to unblock,
/// and it is the evidence for whether the gesture is worth offering at all.
pub async fn abandon(State(state): St, Json(body): Json<AbandonBody>) -> Response {
    let id = body.question.trim();
    if id.is_empty() {
        return (StatusCode::BAD_REQUEST, "which question?\n").into_response();
    }
    verb(&state, &["questions", "abandon", id]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_core::agent::Taint;

    fn question(id: &str, tainted: bool) -> Question {
        Question {
            id: id.into(),
            status: "open".into(),
            question: "  Which mailing address should the letter go to?  ".into(),
            options: vec!["the department".into(), "the home address on file".into()],
            session_id: "20260826-abcdef".into(),
            task_id: Some("task-1a2b3c4d".into()),
            workspace: None,
            taint: Taint {
                private: true,
                untrusted: tainted,
            },
            asked_at: "2026-08-26T10:00:00Z".into(),
            answered_at: None,
            answer: None,
        }
    }

    /// The two surfaces have to name the same thing out loud. The CLI prints
    /// `QuestionStore::short`, and a card printing a different handle means a
    /// person reading the phone cannot type the command the terminal wants —
    /// which is the whole reason the row carries it at all.
    #[test]
    fn the_card_and_the_terminal_call_a_question_by_one_name() {
        let q = question("20260826-abcdef0123", false);
        let row = Row::from(&q);
        assert_eq!(row.handle, QuestionStore::short(&q.id));
        assert_eq!(row.id, q.id, "the verbs still take the whole id");
    }

    /// **The taint marker survives the trip to the page.** `mecha questions`
    /// warns on stderr, which a browser cannot see, so a card without this
    /// flag would be the one surface that shows a question composed while
    /// third-party text was in the room and says nothing about it. An
    /// injected run asks well-formed questions; the owner writing the answer
    /// is entitled to know.
    #[test]
    fn a_question_asked_in_a_tainted_conversation_says_so() {
        assert!(Row::from(&question("a", true)).tainted);
        assert!(!Row::from(&question("b", false)).tainted);
    }

    /// The question is shown whole. A phone is where people read least
    /// carefully, and a clipped question is one answered from its first line
    /// — the `DraftView` rule, one store over.
    #[test]
    fn the_question_reaches_the_card_entire() {
        let q = question("a", false);
        let row = Row::from(&q);
        assert_eq!(row.question, q.question.trim());
        assert_eq!(row.options, q.options, "and its proposed answers with it");
    }
}
