//! The crisis sensor's model tier (`docs/PERSONA-DESIGN.md` §12.2): a
//! structured screen of one owner message, shaped by the first questions of
//! the Columbia Suicide Severity Rating Scale — wish to be dead, thoughts of
//! suicide, method, intent, preparation — plus self-harm.
//!
//! **A quarantined pass** (`quarantine::QuarantinedPass`): no tools, no
//! history, one message. The judge reads the owner's words and nothing it
//! writes reaches anyone; its only effect is a verdict the harness acts on.
//!
//! **A check that cannot run is never silent** (§12). An error, a refusal, an
//! empty answer (a reasoning model that spent its budget thinking — the
//! llama-server trap `CLAUDE.md` names) and an unreadable one are each
//! [`Verdict::Unchecked`], never [`Verdict::Clear`]: "couldn't check" is a
//! state of its own, and the chat reports crisis detection as degraded.

use serde::Deserialize;

use crate::message::StopReason;
use crate::provider::Provider;
use crate::quarantine::QuarantinedPass;

/// Which judge made a crisis record, as `safety::DETECTOR` names the keyword
/// tier: bumped whenever the prompt or the schema changes.
pub const JUDGE: &str = "cssrs-judge-1";

const SYSTEM: &str = "\
You screen one message that a person wrote, for signs that they may be at \
risk of suicide or of harming themselves, following the first questions of \
the Columbia Suicide Severity Rating Scale. You are not talking to them, and \
nothing you write reaches them. Answer only with the JSON object asked for.

Read the message as the person's own words about themselves, now. A story, a \
quotation, news or a question about someone else, or an idiom (\"this \
deadline is killing me\") is not a sign. When the words are ambiguous but \
could be about themselves, answer true.";

/// The screen's answer, one field per question.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Screen {
    wish_to_be_dead: bool,
    suicidal_thoughts: bool,
    method: bool,
    intent: bool,
    preparation: bool,
    self_harm: bool,
}

impl Screen {
    fn concern(&self) -> bool {
        self.wish_to_be_dead
            || self.suicidal_thoughts
            || self.method
            || self.intent
            || self.preparation
            || self.self_harm
    }
}

/// The answer's shape, for a provider that can hold a reply to a schema.
pub fn schema() -> serde_json::Value {
    let flag = serde_json::json!({ "type": "boolean" });
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["wish_to_be_dead", "suicidal_thoughts", "method", "intent", "preparation", "self_harm"],
        "properties": {
            "wish_to_be_dead": flag,
            "suicidal_thoughts": flag,
            "method": flag,
            "intent": flag,
            "preparation": flag,
            "self_harm": flag,
        }
    })
}

/// What the judge concluded — or that it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// At least one sign: the harness pauses the persona.
    Concern,
    /// Screened, no sign.
    Clear,
    /// The judge did not answer usably, and why. Never read as `Clear`.
    Unchecked(String),
}

/// Read a verdict out of the judge's text: the JSON object in it, whole. A
/// reply that holds no parseable object is `Unchecked`.
pub fn parse(text: &str) -> Verdict {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Verdict::Unchecked("the judge's answer held no JSON object".into());
    };
    if end < start {
        return Verdict::Unchecked("the judge's answer held no JSON object".into());
    }
    match serde_json::from_str::<Screen>(&text[start..=end]) {
        Ok(s) if s.concern() => Verdict::Concern,
        Ok(_) => Verdict::Clear,
        Err(e) => Verdict::Unchecked(format!("the judge's answer did not read: {e}")),
    }
}

/// Screen one owner message. Envelope first: a refusal and an empty answer
/// are read before any content is.
pub async fn screen(provider: &dyn Provider, model: &str, text: &str) -> Verdict {
    // Not 4096: that is the router's reasoning budget, and a judge given
    // exactly its thinking allowance spends it all and answers empty — every
    // chat would read "keywords only" forever (review of #426;
    // `docs/LLAMA-SERVER.md` names that value). The one shared number.
    let pass = QuarantinedPass::new(model, crate::provider::LOCAL_MAX_TOKENS)
        .system(SYSTEM)
        .response_schema(provider.structured_output().then(schema));
    let request = pass.ask(format!(
        "The message, between the markers:\n<<<\n{text}\n>>>\n\n\
         Answer with JSON: {{\"wish_to_be_dead\": bool, \"suicidal_thoughts\": bool, \
         \"method\": bool, \"intent\": bool, \"preparation\": bool, \"self_harm\": bool}}"
    ));
    let response = match provider.complete(&request, None).await {
        Ok(r) => r,
        Err(e) => return Verdict::Unchecked(format!("the judge could not be reached: {e:#}")),
    };
    if response.stop_reason == StopReason::Refusal {
        return Verdict::Unchecked("the judge refused".into());
    }
    let body = response.message.text();
    if body.trim().is_empty() {
        return Verdict::Unchecked(if response.stop_reason == StopReason::MaxTokens {
            "the judge ran out of tokens before answering".into()
        } else {
            "the judge's answer was empty".into()
        });
    }
    parse(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Block, CompletionRequest, CompletionResponse, Message, Usage};
    use anyhow::Result;

    /// A provider that answers with a fixed body and stop reason, or fails.
    struct Fixed(Option<(&'static str, StopReason)>);

    #[async_trait::async_trait]
    impl Provider for Fixed {
        fn id(&self) -> &str {
            "test"
        }
        fn default_model(&self) -> &str {
            "test"
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _: Option<&crate::provider::StreamSink>,
        ) -> Result<CompletionResponse> {
            // Quarantined: no tools, one message — and room to answer after
            // a reasoning model's thinking budget.
            assert!(req.tools.is_empty());
            assert_eq!(req.messages.len(), 1);
            assert!(
                req.max_tokens >= crate::provider::LOCAL_MAX_TOKENS,
                "{}",
                req.max_tokens
            );
            let Some((text, stop)) = self.0 else {
                anyhow::bail!("connection refused");
            };
            Ok(CompletionResponse {
                message: Message::assistant(vec![Block::Text { text: text.into() }]),
                stop_reason: stop,
                usage: Usage::default(),
                refusal: None,
                model: "test".into(),
                malformed_tool_args: 0,
            })
        }
    }

    const CLEAR: &str = r#"{"wish_to_be_dead":false,"suicidal_thoughts":false,"method":false,"intent":false,"preparation":false,"self_harm":false}"#;
    const CONCERN: &str = r#"{"wish_to_be_dead":true,"suicidal_thoughts":false,"method":false,"intent":false,"preparation":false,"self_harm":false}"#;

    #[tokio::test]
    async fn the_judge_answers_concern_or_clear() {
        let screen_with = |body| async move {
            screen(&Fixed(Some((body, StopReason::EndTurn))), "m", "hi").await
        };
        assert_eq!(screen_with(CONCERN).await, Verdict::Concern);
        assert_eq!(screen_with(CLEAR).await, Verdict::Clear);
        // Prose around the object, as a model without a schema writes it.
        assert_eq!(
            parse(&format!("Here you go:\n```json\n{CONCERN}\n```")),
            Verdict::Concern
        );
    }

    /// Every way the judge can fail is "couldn't check" — never clear.
    #[tokio::test]
    async fn a_judge_that_cannot_answer_is_unchecked_never_clear() {
        let cases = [
            Fixed(None),
            Fixed(Some(("I can't help with that.", StopReason::Refusal))),
            Fixed(Some(("", StopReason::EndTurn))),
            Fixed(Some(("   ", StopReason::MaxTokens))),
            Fixed(Some(("no json here", StopReason::EndTurn))),
            Fixed(Some((
                r#"{"wish_to_be_dead": "maybe"}"#,
                StopReason::EndTurn,
            ))),
            Fixed(Some((r#"{"wish_to_be_dead":false}"#, StopReason::EndTurn))),
        ];
        for provider in cases {
            let v = screen(&provider, "m", "hi").await;
            assert!(matches!(v, Verdict::Unchecked(_)), "{v:?}");
        }
        let v = screen(&Fixed(Some(("", StopReason::MaxTokens))), "m", "hi").await;
        assert!(
            matches!(v, Verdict::Unchecked(ref why) if why.contains("ran out of tokens")),
            "{v:?}"
        );
    }

    #[test]
    fn the_schema_requires_every_question() {
        let s = schema();
        assert_eq!(s["required"].as_array().unwrap().len(), 6);
        assert_eq!(s["additionalProperties"], false);
    }
}
