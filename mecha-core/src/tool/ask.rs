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

use super::{Capabilities, Tool, ToolCtx, ToolOutput};
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
}

pub struct AskUserTool {
    asker: Arc<dyn Asker>,
}

impl AskUserTool {
    pub fn new(asker: Arc<dyn Asker>) -> Self {
        AskUserTool { asker }
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
         the request, a missing value. Offer 2-4 concrete `options` when the choice is \
         between known alternatives; leave them out for an open question. Prefer asking \
         early over discovering halfway through that you assumed wrong."
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

    async fn call(&self, input: Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
        let question = input.get("question").and_then(Value::as_str).unwrap_or("").trim();
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

        match self.asker.ask(question, &options).await {
            Some(answer) => Ok(ToolOutput::ok(answer)),
            // An error result rather than an `Err`: the model should be able to
            // carry on with its best guess and say that it did, not have the
            // run die because someone pressed escape.
            None => Ok(ToolOutput::err(
                "The user did not answer. Proceed with your best interpretation, \
                 and say plainly which one you chose.",
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
            self.seen.lock().unwrap().push((question.to_string(), options.to_vec()));
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
        let out = tool.call(json!({"question": "which?"}), &ToolCtx::default()).await.unwrap();

        // An error *result*, not an `Err`: pressing escape should not end the
        // run, it should hand the model something it can act on.
        assert!(out.is_error);
        assert!(out.content.contains("best interpretation"), "{}", out.content);
    }

    #[tokio::test]
    async fn an_empty_question_is_refused_before_anyone_is_interrupted() {
        let (tool, canned) = tool(Some("x"));
        let out = tool.call(json!({"question": "   "}), &ToolCtx::default()).await.unwrap();

        assert!(out.is_error);
        assert!(canned.seen.lock().unwrap().is_empty(), "the user was interrupted for nothing");
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
