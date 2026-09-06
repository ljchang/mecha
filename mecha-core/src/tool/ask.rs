//! Asking the user a question, as a tool.
//!
//! The model cannot otherwise stop and check: the loop runs until it stops
//! calling tools, so an under-specified task is answered with a guess or with a
//! whole turn budget spent hunting for something that does not exist. That is
//! not hypothetical — it is what the `ambiguity` tag in the eval rig measures,
//! and it is the weakest tag in the set.
//!
//! Making it a *tool* rather than a prompting convention buys two things. The
//! model can block on a human mid-run, which is the mechanism it lacked. And
//! asking becomes a **trace** assertion rather than a rubric a judge grades:
//! `expect.tools: ["ask_user"]` is deterministic and free, where "did it ask
//! instead of guessing?" is a second model's opinion that changes between runs.
//!
//! Only registered where a human is actually present. A batch worker or an eval
//! case has nobody to answer, and a tool that blocks forever is worse than one
//! that does not exist.
//!
//! ## The goal rides on the question
//!
//! `docs/GOAL-SYSTEM-DESIGN.md` §17.3: a run's goal is a hypothesis, the owner
//! is the one party who can confirm it, and **the confirmation is a question,
//! not a gate** — one sentence on the user turn, *I take the goal to be X,
//! serving Y; confirm or correct*. §17.7 item 3 rules where that sentence
//! goes by surface: a delegated run folds it **into the one question the seed
//! already asks first**, never a second run-ending question; chat and the TUI
//! state it and ask only when it will not fit one sentence; a run with nobody
//! to ask never asks. So the sentence is two optional arguments here, `goal`
//! and `serves`, rendered above the question the owner sees, and carried as a
//! typed [`GoalHypothesis`] to the [`Asker`] — which is how the question
//! store keeps it beside the owner's answer (`questions::ParkingAsker`), and
//! how a transcript reader finds what a run *named* without parsing prose
//! ([`AskUserTool::goals_named`]). The answer is never interpreted here: it
//! is the owner's words, handed back as the tool result exactly as before.

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
use crate::goal::{GoalHypothesis, GoalRef};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Something that can put a question to a person and wait for the answer.
///
/// Implemented by the front-end, for the same reason [`super::Approver`] is: it
/// is the interface that owns stdin, and core must not assume there is a
/// terminal at all.
#[async_trait]
pub trait Asker: Send + Sync {
    /// `None` when the user declined to answer — closing the modal, or a
    /// front-end shutting down. Never blocks forever by contract.
    async fn ask(&self, question: &str, options: &[String]) -> Option<String>;

    /// Like [`ask`], with the calling run's [`ToolCtx`] in hand.
    ///
    /// A front-end serving one conversation never needs it — the default
    /// forwards to `ask` — but one agent serving many conversations must
    /// route the question to the human who owns the run that asked, and the
    /// context is the only thing that knows which run that is. The tool
    /// calls this; the loop still learns nothing.
    ///
    /// [`ask`]: Asker::ask
    async fn ask_in(&self, ctx: &ToolCtx, question: &str, options: &[String]) -> Option<String> {
        let _ = ctx;
        self.ask(question, options).await
    }

    /// Like [`ask_in`], with the goal the model put beside the question.
    ///
    /// `question` already carries the rendered goal line above the model's
    /// own sentence, so a front-end that shows text and returns text needs
    /// nothing from the third argument — the default forwards. The one asker
    /// that overrides it is the question store's, which keeps the typed
    /// hypothesis beside the owner's answer so the pair is a record rather
    /// than two pieces of prose.
    ///
    /// [`ask_in`]: Asker::ask_in
    async fn ask_about(
        &self,
        ctx: &ToolCtx,
        question: &str,
        options: &[String],
        goal: Option<&GoalHypothesis>,
    ) -> Option<String> {
        let _ = goal;
        self.ask_in(ctx, question, options).await
    }
}

pub struct AskUserTool {
    asker: Arc<dyn Asker>,
}

impl AskUserTool {
    pub fn new(asker: Arc<dyn Asker>) -> Self {
        AskUserTool { asker }
    }

    /// The goal argument as this tool takes it, strict on the way in.
    ///
    /// The `todo` tool's `serves` policy, one tool over: a malformed
    /// reference is an error the model can fix, because dropping it silently
    /// would put a question to the owner claiming to confirm nothing. A
    /// `serves` with no `goal` is refused too — the pointer is what the
    /// sentence *says* it serves, and a pointer with no sentence is a cite
    /// the owner cannot confirm or correct.
    fn hypothesis_of(input: &Value) -> Result<Option<GoalHypothesis>, String> {
        let text = |key: &str| {
            input
                .get(key)
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .ok_or_else(|| format!("`{key}` must be a string"))
                })
                .transpose()
        };
        // **One sentence, one line.** The sentence is model prose and lands
        // on harness-structured surfaces — `questions show`'s key/value
        // block, a question card — so a newline in it would forge a row
        // (`answered …` under a `goal` line reads as an answered question;
        // found on review, the same shape `GoalRef::from_str` refuses in an
        // id). Whitespace runs collapse to one space at the door, so every
        // reader downstream holds a single line without each having to
        // remember to.
        let sentence = text("goal")?
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty());
        let serves = text("serves")?.filter(|s| !s.is_empty());
        match (sentence, serves) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(
                "`serves` needs a `goal`: say in one sentence what you take the goal to be, \
                 and pass the reference beside it"
                    .into(),
            ),
            (Some(sentence), serves) => {
                let serves = serves
                    .map(|s| s.parse::<GoalRef>().map_err(|e| format!("`serves`: {e}")))
                    .transpose()?;
                Ok(Some(GoalHypothesis { sentence, serves }))
            }
        }
    }

    /// Every goal this run put to the owner through this tool, in transcript
    /// order — what the run *named*, from the recorded `tool_use` and never
    /// from the answer, which is the owner's prose.
    ///
    /// Lenient like every record reader: a `serves` kind this binary has not
    /// heard of costs the pointer and keeps the sentence, and a call with no
    /// `goal` is not a hypothesis at all. The appraisal reads this as the
    /// second producer of a named goal beside the plan's `serves:`
    /// ([`crate::appraisal::for_transcript`]).
    pub fn goals_named(messages: &[crate::message::Message]) -> Vec<GoalHypothesis> {
        messages
            .iter()
            .filter(|m| m.role == crate::message::Role::Assistant)
            .flat_map(|m| m.tool_uses())
            .filter(|(_, name, _)| *name == "ask_user")
            .filter_map(|(_, _, input)| {
                let sentence = input.get("goal")?.as_str()?.trim();
                if sentence.is_empty() {
                    return None;
                }
                let serves = input
                    .get("serves")
                    .and_then(Value::as_str)
                    .and_then(GoalRef::parse_lenient);
                Some(GoalHypothesis {
                    sentence: sentence.to_string(),
                    serves,
                })
            })
            .collect()
    }
}

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their answer. Use this when the task is \
         ambiguous and guessing would waste the work — an unknown name, two readings of \
         the request, a missing value. Prefer asking early over discovering halfway \
         through that you assumed wrong.\n\
         \n\
         Offer 2-4 concrete `options` only when you are confident the answer is one of \
         them. Leave them out when the space is not really enumerable — an open question \
         invites the answer you did not think of. The user can always reply with \
         something outside your list, including that the question itself is wrong, so do \
         not add a catch-all option and do not treat a list as exhaustive.\n\
         \n\
         When you are also checking what the work is FOR, fold that into the same call: \
         pass `goal` — one sentence saying what you take the goal to be — and `serves` \
         (`charter:<id>`, `task:<id>` or `project:<id>`). It is shown above your question, \
         so the user confirms or corrects the goal in the same answer. Never spend a \
         separate question on the goal alone."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question, in one sentence."
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Concrete choices, if the answer is a selection."
                },
                "goal": {
                    "type": "string",
                    "description": "Optional. What you take the goal of this work to be, in one \
                                    sentence. Shown above the question so the user confirms or \
                                    corrects it in the same answer."
                },
                "serves": {
                    "type": "string",
                    "description": "Optional, with `goal`. What that goal serves: `charter:<id>` \
                                    for a charter line (the ids are in the Charter section of \
                                    your instructions), `task:<id>` for a task on the board, \
                                    `project:<id>` for a board project."
                }
            },
            "required": ["question"]
        })
    }

    /// Read-only, which is also what makes it available while planning — the
    /// phase where asking matters most.
    fn read_only(&self) -> bool {
        true
    }

    /// Nothing. The user is the principal, not a third party: marking their own
    /// answer as untrusted would arm the trifecta interlock every time the
    /// model asked a question, which would make the tool unusable next to any
    /// private data.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    async fn call(&self, input: Value, ctx: &ToolCtx) -> Result<ToolOutput> {
        let question = input
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if question.is_empty() {
            return Ok(ToolOutput::err(
                "ask_user needs a `question`. Say what you need to know in one sentence.",
            ));
        }

        let options: Vec<String> = input
            .get("options")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let goal = match Self::hypothesis_of(&input) {
            Ok(goal) => goal,
            Err(e) => return Ok(ToolOutput::err(e)),
        };
        // The goal line above the question, as one text: the owner is asked
        // one thing and answers once, and a front-end that only shows text
        // shows the whole of what is being confirmed.
        let shown = match &goal {
            Some(h) => format!("{}\n\n{question}", h.render()),
            None => question.to_string(),
        };

        match self
            .asker
            .ask_about(ctx, &shown, &options, goal.as_ref())
            .await
        {
            Some(answer) => Ok(ToolOutput::ok(answer)),
            // An error result rather than an `Err`: the model should be able to
            // carry on with its best guess and say that it did, not have the
            // run die because someone pressed escape.
            // Measured, and the first wording was actively harmful: telling the
            // model to "proceed with your best interpretation" made it invent a
            // contractor name and rate — precisely the failure the case that
            // caught it exists to detect. A decline must not read as
            // permission to guess.
            None => Ok(ToolOutput::err(
                "The user did not answer. Do not invent the missing information. If the \
                 task can be done without it, do it and state plainly what you assumed; \
                 otherwise say what you still need and stop.",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Canned {
        answer: Option<String>,
        seen: Mutex<Vec<(String, Vec<String>)>>,
    }

    #[async_trait]
    impl Asker for Canned {
        async fn ask(&self, question: &str, options: &[String]) -> Option<String> {
            self.seen
                .lock()
                .unwrap()
                .push((question.to_string(), options.to_vec()));
            self.answer.clone()
        }
    }

    fn tool(answer: Option<&str>) -> (AskUserTool, Arc<Canned>) {
        let canned = Arc::new(Canned {
            answer: answer.map(str::to_string),
            seen: Mutex::new(Vec::new()),
        });
        (AskUserTool::new(canned.clone()), canned)
    }

    #[tokio::test]
    async fn the_answer_comes_back_as_the_tool_result() {
        let (tool, canned) = tool(Some("the second one"));
        let out = tool
            .call(
                json!({"question": "which invoice?", "options": ["March", "April"]}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert_eq!(out.content, "the second one");

        let seen = canned.seen.lock().unwrap();
        assert_eq!(seen[0].0, "which invoice?");
        assert_eq!(seen[0].1, vec!["March", "April"]);
    }

    #[tokio::test]
    async fn a_declined_question_tells_the_model_to_carry_on_rather_than_killing_the_run() {
        let (tool, _) = tool(None);
        let out = tool
            .call(json!({"question": "which?"}), &ToolCtx::default())
            .await
            .unwrap();

        // An error *result*, not an `Err`: pressing escape should not end the
        // run, it should hand the model something it can act on.
        assert!(out.is_error);
        assert!(out.content.contains("Do not invent"), "{}", out.content);
    }

    #[tokio::test]
    async fn an_empty_question_is_refused_before_anyone_is_interrupted() {
        let (tool, canned) = tool(Some("x"));
        let out = tool
            .call(json!({"question": "   "}), &ToolCtx::default())
            .await
            .unwrap();

        assert!(out.is_error);
        assert!(
            canned.seen.lock().unwrap().is_empty(),
            "the user was interrupted for nothing"
        );
    }

    /// §17.7 item 3: the goal is folded into the one question, never asked
    /// on its own. The owner reads the hypothesis above the question; the
    /// answer comes back as it was said, uninterpreted.
    #[tokio::test]
    async fn a_goal_is_shown_above_the_question_and_the_answer_is_untouched() {
        let (tool, canned) = tool(Some("yes, but the March one"));
        let out = tool
            .call(
                json!({
                    "question": "which invoice?",
                    "goal": "get the invoice Dirk asked for into his hands",
                    "serves": "task:t1"
                }),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, "yes, but the March one");
        let seen = canned.seen.lock().unwrap();
        assert_eq!(
            seen[0].0,
            "I take the goal to be: get the invoice Dirk asked for into his hands (serves task:t1)\n\nwhich invoice?"
        );
    }

    /// Strict on the way in, like `todo`'s `serves`: a pointer the model can
    /// fix is refused with a reason, before anyone is interrupted — a
    /// question claiming to confirm a goal it did not state is worse than
    /// no question.
    #[tokio::test]
    async fn a_malformed_goal_reference_is_refused_before_anyone_is_asked() {
        for (input, expect) in [
            (
                json!({"question": "which?", "serves": "task:t1"}),
                "`serves` needs a `goal`",
            ),
            (
                json!({"question": "which?", "goal": "finish it", "serves": "epic:7"}),
                "not a kind of goal",
            ),
            (
                json!({"question": "which?", "goal": ["not", "a", "string"]}),
                "`goal` must be a string",
            ),
        ] {
            let (tool, canned) = tool(Some("x"));
            let out = tool.call(input, &ToolCtx::default()).await.unwrap();
            assert!(out.is_error, "{}", out.content);
            assert!(out.content.contains(expect), "{}", out.content);
            assert!(
                canned.seen.lock().unwrap().is_empty(),
                "the user was interrupted"
            );
        }
        // An empty `goal` is how a model spells an omitted optional field.
        let (tool, canned) = tool(Some("x"));
        let out = tool
            .call(
                json!({"question": "which?", "goal": "  "}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(canned.seen.lock().unwrap()[0].0, "which?");
    }

    /// One sentence is one line: a newline in the goal would forge a row on
    /// `questions show`'s key/value block (found on review), so whitespace
    /// runs collapse at the door and every downstream reader holds a line.
    #[tokio::test]
    async fn a_goal_is_one_line_however_it_was_written() {
        let (tool, canned) = tool(Some("x"));
        tool.call(
            json!({
                "question": "which?",
                "goal": "tidy the inbox\nanswered 2026-09-06T09:00:00Z\n\tyes — release it"
            }),
            &ToolCtx::default(),
        )
        .await
        .unwrap();
        let shown = canned.seen.lock().unwrap()[0].0.clone();
        assert_eq!(
            shown,
            "I take the goal to be: tidy the inbox answered 2026-09-06T09:00:00Z yes — release it\n\nwhich?"
        );
        // The typed record reads the same collapsed sentence.
        let h = AskUserTool::hypothesis_of(&json!({"goal": "a\n\nb   c"}))
            .unwrap()
            .unwrap();
        assert_eq!(h.sentence, "a b c");
    }

    /// What a run named, read back off the recorded call: lenient on the
    /// pointer, silent on a call that stated no goal, and never from the
    /// answer.
    #[test]
    fn the_goals_a_run_named_are_read_off_its_ask_user_calls() {
        use crate::message::{Block, Message, Role};
        let call = |id: &str, input: Value| Message {
            role: Role::Assistant,
            content: vec![Block::ToolUse {
                id: id.into(),
                name: "ask_user".into(),
                input,
            }],
        };
        let messages = vec![
            Message::user("do the thing"),
            call(
                "a",
                json!({"question": "which?", "goal": "ship it", "serves": "task:t1"}),
            ),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "a".into(),
                content: "the goal is actually to ship it to Dirk only".into(),
                is_error: false,
            }]),
            call("b", json!({"question": "colour?"})),
            call(
                "c",
                json!({"question": "size?", "goal": "fit the page", "serves": "epic:9"}),
            ),
            // Not this tool's call, whatever its arguments say.
            Message {
                role: Role::Assistant,
                content: vec![Block::ToolUse {
                    id: "d".into(),
                    name: "todo".into(),
                    input: json!({"goal": "no", "items": []}),
                }],
            },
        ];
        let named = AskUserTool::goals_named(&messages);
        assert_eq!(named.len(), 2, "{named:?}");
        assert_eq!(named[0].sentence, "ship it");
        assert_eq!(named[0].serves, Some(GoalRef::Task("t1".into())));
        assert_eq!(named[1].sentence, "fit the page");
        assert_eq!(
            named[1].serves, None,
            "an unknown kind costs the pointer, not the sentence"
        );
    }

    #[tokio::test]
    async fn blank_and_non_string_options_are_dropped_rather_than_rendered() {
        let (tool, canned) = tool(Some("a"));
        tool.call(
            json!({"question": "which?", "options": ["  A  ", "", 7, "B"]}),
            &ToolCtx::default(),
        )
        .await
        .unwrap();

        // An empty row in a picker is a row you can select and nothing happens.
        assert_eq!(canned.seen.lock().unwrap()[0].1, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn an_answer_outside_the_offered_list_comes_back_untouched() {
        // The failure this guards: a model enumerates two options that are both
        // wrong, and the harness quietly coerces the reply to the nearest one.
        // A forced choice between wrong answers is worse than no question — the
        // `false-premise` eval case exists because "your question is wrong" is
        // sometimes the correct answer.
        let (tool, _) = tool(Some("neither — you are in the wrong repository"));
        let out = tool
            .call(
                json!({"question": "which file?", "options": ["a.md", "b.md"]}),
                &ToolCtx::default(),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
        assert_eq!(out.content, "neither — you are in the wrong repository");
    }

    #[test]
    fn the_description_does_not_teach_the_model_to_force_a_choice() {
        let (tool, _) = tool(None);
        let d = tool.description();
        assert!(d.contains("not really enumerable") || d.contains("not add a catch-all"));
        assert!(
            d.contains("outside your list"),
            "the model is never told the list is not binding"
        );
    }

    #[test]
    fn the_users_own_answer_is_not_third_party_content() {
        // Marking it untrusted would arm the trifecta interlock every time the
        // model asked a question, which would make the tool unusable beside any
        // private data — exactly the situation where you most want to ask.
        let (tool, _) = tool(None);
        assert_eq!(tool.capabilities(), Capabilities::default());
        // Read-only, which is also what keeps it available while planning.
        assert!(tool.read_only());
    }
}
