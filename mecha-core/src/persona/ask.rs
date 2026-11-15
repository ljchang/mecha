//! Did the owner ask for a picture this turn?
//!
//! A persona's picture tool is admitted per call, by the owner's words and
//! never by the persona's habit. A chat that has drawn a few pictures teaches
//! its model to draw on every turn: on 2026-11-15 one chat made 27 picture
//! calls, 25 of them on turns that asked for none, and drew straight through
//! five of the owner's thirteen requests to stop. Its own reasoning said the
//! owner wanted words alone and then planned the picture, copied from its
//! earlier turns. Notes did not hold it: replayed with the tool description
//! and the notes reworded it still drew 3 to 8 times in 10, and told on the
//! turn itself that no picture was asked for, 4 to 8 in 10.
//!
//! So the question goes to a reader that has none of the chat: the owner's
//! latest message and the few before it, one shot, no tools, no history
//! ([`crate::quarantine::QuarantinedPass`]). On 335 of the owner's real
//! messages (123 labelled by hand and tuned on, 212 held out and checked
//! against what was drawn) it missed no ask and said yes to 6 that were none,
//! which costs nothing unless the persona also tries to draw there.
//!
//! It fails open: a reader that cannot be reached, times out or answers
//! badly leaves the call drawn as sent. This is the owner's comfort, not a
//! guard, and a picture they asked for and did not get is the worse failure.

use crate::tool::ToolOutput;
use std::sync::Arc;
use std::time::Duration;

/// The reader's instructions.
pub const SYSTEM: &str = include_str!("ask.md");

/// How many of the owner's earlier messages the reader is shown: enough to
/// see a run of picture directions ("now from the side") or a standing
/// request for none, and no more of the chat than that.
pub const EARLIER: usize = 3;

/// The most of one message the reader is shown.
const LATEST_CHARS: usize = 1500;
const EARLIER_CHARS: usize = 600;

/// How long a picture call waits on the reader before it is drawn as sent.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What the persona is told when its picture call is not drawn. It says what
/// to do next, since the run goes on to a reply in words.
pub const NOT_ASKED: &str = "Nothing was made: no picture or video was requested in the owner's \
     last message, so none is drawn. Answer them in words alone, and call this only on a turn \
     where they ask to see something.";

/// How the run ends after a refusal: one more request, with no tools, so
/// the reply is words and the call is not made again (the queue's busy
/// refusal looped until it closed its run the same way). The line does not
/// say how long to be: the owner may have asked for more.
pub const CLOSING: crate::agent::Closing = crate::agent::Closing {
    line: "(From the harness: no picture was requested in the owner's last message, so none \
           was drawn. Reply to what they said in words alone, with no tool.)",
    reply: "That reply came back empty. Send your message again.",
};

/// Who answers whether this turn's owner asked for a picture. Handed to the
/// picture tools per turn (`ToolCtx::picture_ask`); `None` everywhere a
/// person is not being answered in a persona chat, and on a turn the owner
/// started from a picture's own button, which is the ask.
#[async_trait::async_trait]
pub trait AskReader: Send + Sync + std::fmt::Debug {
    /// Whether the owner asked, or why that could not be read.
    async fn asked(&self) -> Result<bool, String>;
}

/// The reader on the persona's own model, holding this turn's words.
pub struct ModelAsk {
    pub provider: Box<dyn crate::provider::Provider>,
    pub model: String,
    /// The owner's words this turn.
    pub latest: String,
    /// The owner's messages before it, oldest first, at most [`EARLIER`].
    pub earlier: Vec<String>,
}

impl std::fmt::Debug for ModelAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelAsk")
            .field("model", &self.model)
            .finish()
    }
}

/// The owner's messages before this turn's, oldest first: the last
/// [`EARLIER`] of `history`, whose final owner turn is this one.
pub fn earlier(history: &[crate::message::Message]) -> Vec<String> {
    let mut said = crate::title::owner_turns(history);
    said.pop();
    let from = said.len().saturating_sub(EARLIER);
    said.split_off(from)
}

fn clip(text: &str, n: usize) -> String {
    text.trim().chars().take(n).collect()
}

fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"asked": {"type": "boolean"}},
        "required": ["asked"],
        "additionalProperties": false
    })
}

/// The one-shot request: no tools, one message, no thinking, the schema
/// where the provider honours one.
pub fn request(
    model: &str,
    latest: &str,
    earlier: &[String],
    structured: bool,
) -> crate::message::CompletionRequest {
    let user = serde_json::json!({
        "earlier": earlier.iter().map(|e| clip(e, EARLIER_CHARS)).collect::<Vec<_>>(),
        "latest": clip(latest, LATEST_CHARS),
    });
    crate::quarantine::QuarantinedPass::new(model, 256)
        .system(SYSTEM)
        .no_thinking()
        .response_schema(structured.then(schema))
        .ask(user.to_string())
}

/// The reader's answer, read: the `asked` of the JSON object in it. Anything
/// else is a failure, never a no.
pub fn read(text: &str) -> Result<bool, String> {
    let (Some(from), Some(to)) = (text.find('{'), text.rfind('}')) else {
        return Err("the reader's answer held no JSON object".into());
    };
    serde_json::from_str::<serde_json::Value>(&text[from..=to])
        .ok()
        .and_then(|v| v.get("asked").and_then(serde_json::Value::as_bool))
        .ok_or_else(|| "the reader's answer had no `asked`".to_string())
}

#[async_trait::async_trait]
impl AskReader for ModelAsk {
    async fn asked(&self) -> Result<bool, String> {
        let request = request(
            &self.model,
            &self.latest,
            &self.earlier,
            self.provider.structured_output(),
        );
        let response = self
            .provider
            .complete(&request, None)
            .await
            .map_err(|e| format!("the reader could not be reached ({e:#})"))?;
        // The envelope before the content: a refusal arrives as success.
        if response.stop_reason == crate::message::StopReason::Refusal {
            return Err("the reader refused".into());
        }
        read(&response.message.text())
    }
}

/// A picture tool's first question: `Some` is the result that answers the
/// call instead of a picture, because the owner asked for none. `None` draws:
/// no reader this turn, a yes, or a reader that could not say.
pub async fn refusal(reader: Option<&Arc<dyn AskReader>>, tool: &str) -> Option<ToolOutput> {
    let reader = reader?;
    match tokio::time::timeout(TIMEOUT, reader.asked()).await {
        Ok(Ok(true)) => None,
        Ok(Ok(false)) => Some(ToolOutput::refusal(NOT_ASKED).closing_with(&CLOSING)),
        Ok(Err(why)) => {
            tracing::warn!("{tool}: whether the owner asked could not be read ({why}); drawn");
            None
        }
        Err(_) => {
            tracing::warn!("{tool}: the ask reader took too long; drawn");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[derive(Debug)]
    struct Says(Result<bool, String>);

    #[async_trait::async_trait]
    impl AskReader for Says {
        async fn asked(&self) -> Result<bool, String> {
            self.0.clone()
        }
    }

    fn reader(says: Result<bool, String>) -> Arc<dyn AskReader> {
        Arc::new(Says(says))
    }

    #[tokio::test]
    async fn only_a_read_no_refuses_and_everything_else_draws() {
        assert!(refusal(None, "image_generate").await.is_none());
        assert!(refusal(Some(&reader(Ok(true))), "image_generate")
            .await
            .is_none());
        // Fails open: a reader that cannot say is not a no.
        assert!(refusal(Some(&reader(Err("down".into()))), "image_generate")
            .await
            .is_none());
        let out = refusal(Some(&reader(Ok(false))), "image_generate")
            .await
            .expect("a no refuses");
        assert!(
            out.is_error && out.refusal,
            "the harness refusing, not the tool failing"
        );
        assert_eq!(out.content, NOT_ASKED);
        assert_eq!(out.closes, Some(&CLOSING), "the run closes in words");
    }

    #[test]
    fn an_answer_is_its_asked_and_anything_else_is_a_failure() {
        assert_eq!(read(r#"{"asked": true}"#), Ok(true));
        assert_eq!(read("```json\n{\"asked\": false}\n```"), Ok(false));
        assert!(read("no").is_err());
        assert!(read(r#"{"asked": "no"}"#).is_err());
        assert!(read(r#"{"answer": false}"#).is_err());
    }

    #[test]
    fn the_reader_sees_the_last_few_owner_messages_before_this_one() {
        let mut history = Vec::new();
        for n in 1..=5 {
            history.push(Message::user(format!("owner {n}")));
            history.push(Message::assistant(vec![crate::message::Block::text(
                format!("reply {n}"),
            )]));
        }
        history.pop();
        assert_eq!(earlier(&history), ["owner 2", "owner 3", "owner 4"]);
        assert!(earlier(&[Message::user("the first")]).is_empty());
    }

    #[test]
    fn the_request_carries_the_words_clipped_and_no_tools() {
        let long = "x".repeat(LATEST_CHARS + 50);
        let r = request("m", &long, &["before".into()], true);
        assert!(r.tools.is_empty());
        let user = r.messages.last().unwrap().text();
        let sent: serde_json::Value = serde_json::from_str(&user).unwrap();
        assert_eq!(sent["earlier"][0], "before");
        assert_eq!(
            sent["latest"].as_str().unwrap().chars().count(),
            LATEST_CHARS
        );
    }
}
