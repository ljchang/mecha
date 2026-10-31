//! A one-shot request with no tools and no conversation.
//!
//! **The safety property is that there is nothing for an injected instruction
//! to reach**, and it is the whole reason the front door's extractor can be
//! pointed at a stranger's prose: the model may obey the text completely and
//! still do nothing, because it has no tool to call, no history to poison and
//! no prefix shared with anything that does.
//!
//! That property was established by hand at nine call sites — the front
//! door's extractor, the mail classifier, the mail *corrections* reflector,
//! the distiller, the reflector, the learner, the eval judge, the compaction
//! summariser and its validator — each spelling out `tools: Vec::new()` and a
//! single-element `messages` vector beside four other fields. Two of them are
//! visibly copies of a third, down to the comment, and the ninth was found by
//! a reviewer rather than by the sweep that produced this module: it lives in
//! `mecha-cli`, and the sweep had grepped `mecha-core`. Which is the argument
//! restated — a property held by convention is held only as far as whoever
//! last looked. A property re-asserted eight times is one
//! `messages.push` away from not holding, and the failure is silent: the call
//! succeeds, the answer parses, and the isolation is gone.
//!
//! So the property moves into the type. [`QuarantinedPass`] has no field for
//! tools and no way to add a second message; [`QuarantinedPass::ask`] is the
//! only way to obtain a request from it. That is the same move as
//! `frontdoor::Record::for_privileged_run`, which is a function with no
//! argument that returns the prose — a boundary you cannot ask to be crossed
//! is worth more than one everybody remembers not to cross.
//!
//! **What this is not for.** A tool-less request is not automatically a
//! quarantined one. `Agent::final_answer` also sends `tools: Vec::new()` —
//! there it is load-bearing for a different reason, forcing prose out of a
//! model that would rather call something — and it sends *the whole
//! conversation* with the agent's own system prompt. Migrating it to this
//! type would silently discard the run. The distinction is history: a
//! quarantined pass has none, by construction.
//!
//! Three fields are fixed rather than offered:
//!
//! - **`tools` is always empty**, which is the point. The mail corrections
//!   reflector states the stakes best, in its own comment: *a reflector with a
//!   tool surface is a reflector that can be talked into using it, and the mail
//!   it reads is the least trusted input in the system.*
//! - **`messages` is always exactly one user turn.** Each [`ask`] builds a
//!   fresh request, so a retry loop that calls it twice sends two isolated
//!   questions rather than growing a conversation — which is what the
//!   extractor's and the classifier's parse-retry both need, and what they
//!   each open-coded. The one exception is [`follow_up`]: a second turn
//!   whose only history is this pass's own question and answer, for a
//!   caller that wants the first request's prefix reused (the distiller's
//!   appraisal). It is never how a retry is built.
//! - **`thinking` is always false.** It asks the provider for a readable
//!   summary of the model's reasoning, which exists for a human watching a
//!   run. Nothing watches these. Every one of the eight sites set it false.
//!
//! Two knobs are offered on top. [`no_thinking`](QuarantinedPass::no_thinking)
//! asks a reasoning model not to reason at all (`think: Some(false)`), for a
//! one-shot on a latency path — the voice director — where a thinking pass
//! would cost seconds per sentence; [`think`](QuarantinedPass::think) says
//! either way, for a caller whose default is off but which may ask for it (a
//! code-mode sub-call). Unset, nothing is sent and the server's default
//! stands, exactly as before. And [`local_only`](QuarantinedPass::local_only)
//! marks the request as one whose bytes must not leave the machine
//! (`CompletionRequest::local_only`), which a provider that is not local
//! refuses by name.
//!
//! [`ask`]: QuarantinedPass::ask
//! [`follow_up`]: QuarantinedPass::follow_up

use crate::message::{CompletionRequest, Effort, Message};

/// A picture ready to go before a quarantined question
/// ([`QuarantinedPass::ask_about`]): made only from a picture's bytes,
/// through the door's cap (`image::rendered_block`), so no other block can
/// stand in for it. Made once and asked about many times.
#[derive(Debug, Clone)]
pub struct Shown(crate::message::Block);

impl Shown {
    /// Bytes that are not a picture are an error.
    pub fn of(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(Shown(crate::image::rendered_block(bytes, None)?))
    }
}

/// The shape of a quarantined call: a model, an optional frame, and budgets.
///
/// Build one, then [`ask`](QuarantinedPass::ask) it a question. The request
/// it returns carries no tools and one user message, and there is no
/// constructor, setter or field that changes either.
#[derive(Debug, Clone)]
pub struct QuarantinedPass {
    model: String,
    system: Option<String>,
    max_tokens: u32,
    effort: Option<Effort>,
    cache_prompt: bool,
    response_schema: Option<serde_json::Value>,
    think: Option<bool>,
    local_only: bool,
}

impl QuarantinedPass {
    /// A pass with no system frame, no cached prefix and default effort.
    ///
    /// The defaults are the conservative ones: an unframed question, and
    /// nothing shared with any other call. Caching a stranger's text across
    /// calls is a property nobody asked for, so a caller that wants a shared
    /// prefix says so.
    pub fn new(model: impl Into<String>, max_tokens: u32) -> Self {
        QuarantinedPass {
            model: model.into(),
            system: None,
            max_tokens,
            effort: None,
            cache_prompt: false,
            response_schema: None,
            think: None,
            local_only: false,
        }
    }

    /// Give the pass a system frame — the instructions it answers under.
    pub fn system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Reasoning effort, where the provider takes one.
    pub fn effort(mut self, effort: Option<Effort>) -> Self {
        self.effort = effort;
        self
    }

    /// Mark the frame cacheable. Worth it only when the same frame is asked
    /// many questions — a judge over a case set, a reflector over a batch.
    pub fn cache_prompt(mut self, cache: bool) -> Self {
        self.cache_prompt = cache;
        self
    }

    /// Constrain the answer while retaining the no-tools, no-history boundary.
    pub fn response_schema(mut self, schema: Option<serde_json::Value>) -> Self {
        self.response_schema = schema;
        self
    }

    /// Ask the model not to reason before answering. Only for a short pass
    /// on a latency path; the default leaves the server's choice alone.
    pub fn no_thinking(mut self) -> Self {
        self.think = Some(false);
        self
    }

    /// Ask the model to reason first, or not to: `think: Some(on)`, sent
    /// either way. A request cannot prove it was honoured (a template
    /// without `--jinja` ignores it), so a caller that bounds thinking
    /// counts what came back, never what it asked for.
    pub fn think(mut self, on: bool) -> Self {
        self.think = Some(on);
        self
    }

    /// Refuse to send this pass anywhere but this machine
    /// (`CompletionRequest::local_only`, R8).
    pub fn local_only(mut self) -> Self {
        self.local_only = true;
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Build the request for one question.
    ///
    /// The only way to get a [`CompletionRequest`] out of this type, and the
    /// place the invariant lives: no tools, exactly one user message, no
    /// reasoning summary. Calling it twice yields two independent requests.
    pub fn ask(&self, user: impl Into<String>) -> CompletionRequest {
        CompletionRequest {
            response_schema: self.response_schema.clone(),
            model: self.model.clone(),
            system: self.system.clone(),
            messages: vec![Message::user(user.into())],
            tools: Vec::new(),
            max_tokens: self.max_tokens,
            effort: self.effort,
            thinking: false,
            cache_prompt: self.cache_prompt,
            think: self.think,
            think_budget: None,
            local_only: self.local_only,
            trailing_notes: 0,
            tool_choice: crate::message::ToolChoice::Auto,
        }
    }

    /// [`ask`](Self::ask) with a picture before the question, in the one
    /// user turn: a judge reading what a picture shows (`persona::
    /// replay_judge`). Still no tools and no history, and a [`Shown`] is
    /// the only thing that can go before the question (review of #644).
    pub fn ask_about(&self, picture: &Shown, user: impl Into<String>) -> CompletionRequest {
        let mut request = self.ask(user);
        request.messages[0].content.insert(0, picture.0.clone());
        request
    }

    /// A second question on the same pass: [`ask`](Self::ask)`(asked)`'s
    /// request, its answer appended verbatim, and one more user turn.
    ///
    /// **The one history a quarantined pass may carry is its own.** Nothing
    /// enters but the question this pass asked and the answer this pass
    /// received, so there is still no tool to reach and no one else's
    /// conversation to poison — the property the module is for. What it
    /// buys is the prefix: the first turn is built by `ask` itself, so the
    /// system frame and the first user turn are the first request's bytes,
    /// and a server that caches by prefix (llama-server's slot KV, chosen
    /// by longest-common-prefix similarity) prefills only the answer and
    /// the new turn. The distiller's appraisal rides on this (R25, ruled
    /// 2026-09-25): the episode's prompt stays byte-identical and the
    /// appraisal costs a generation, not a second read of the transcript.
    pub fn follow_up(
        &self,
        asked: impl Into<String>,
        answered: Message,
        then: impl Into<String>,
    ) -> CompletionRequest {
        let mut request = self.ask(asked);
        request.messages.push(answered);
        request.messages.push(Message::user(then.into()));
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture rides in the one user turn, before the question.
    #[test]
    fn a_question_about_a_picture_is_still_one_turn_with_no_tools() {
        let mut png = Vec::new();
        image::RgbImage::new(4, 3)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let pass = QuarantinedPass::new("m", 8);
        let req = pass.ask_about(&Shown::of(&png).unwrap(), "how many?");
        assert!(req.tools.is_empty());
        assert_eq!(req.messages.len(), 1);
        assert!(matches!(
            req.messages[0].content[0],
            crate::message::Block::Image { .. }
        ));
        assert_eq!(req.messages[0].content.len(), 2);
        assert!(req.messages[0].text().ends_with("how many?"));
        // Only a picture goes before the question.
        assert!(Shown::of(b"not a picture").is_err());
    }

    #[test]
    fn a_quarantined_request_carries_no_tools_and_exactly_one_user_turn() {
        let req = QuarantinedPass::new("m", 4096).ask("what is this?");
        assert!(req.tools.is_empty());
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].role, crate::message::Role::User);
        assert_eq!(req.messages[0].text(), "what is this?");
        assert!(!req.thinking);
    }

    #[test]
    fn thinking_is_left_to_the_server_unless_a_pass_declines_it() {
        assert_eq!(QuarantinedPass::new("m", 64).ask("q").think, None);
        assert_eq!(
            QuarantinedPass::new("m", 64).no_thinking().ask("q").think,
            Some(false)
        );
    }

    /// A pass may say either way about thinking, and may refuse to leave the
    /// machine; neither is set unless asked for.
    #[test]
    fn thinking_either_way_and_local_only_are_asked_for_explicitly() {
        let plain = QuarantinedPass::new("m", 64).ask("q");
        assert!(!plain.local_only);
        let on = QuarantinedPass::new("m", 64)
            .think(true)
            .local_only()
            .ask("q");
        assert_eq!(on.think, Some(true));
        assert!(on.local_only);
        assert_eq!(
            QuarantinedPass::new("m", 64).think(false).ask("q").think,
            Some(false)
        );
    }

    #[test]
    fn the_frame_is_absent_until_asked_for_and_nothing_is_cached_by_default() {
        let bare = QuarantinedPass::new("m", 128).ask("q");
        assert_eq!(bare.system, None);
        assert!(!bare.cache_prompt);

        let framed = QuarantinedPass::new("m", 128)
            .system("you are a classifier")
            .cache_prompt(true)
            .ask("q");
        assert_eq!(framed.system.as_deref(), Some("you are a classifier"));
        assert!(framed.cache_prompt);
    }

    /// The retry case, which the extractor and the classifier each open-coded:
    /// a second attempt is a second *isolated* question, never a follow-up
    /// turn. If `ask` ever accumulated, a parse retry would hand the model its
    /// own malformed output as conversation — and the second call is the one
    /// carrying a stranger's prose.
    #[test]
    fn asking_twice_does_not_accumulate_a_conversation() {
        let pass = QuarantinedPass::new("m", 128);
        let first = pass.ask("attempt one");
        let second = pass.ask("attempt two");
        assert_eq!(first.messages.len(), 1);
        assert_eq!(second.messages.len(), 1);
        assert_eq!(second.messages[0].text(), "attempt two");
        assert!(second.tools.is_empty());
    }

    /// A follow-up is the first request with the answer and one more turn
    /// appended — the first turn built by `ask` itself, so the frame and
    /// the question are the first request's, and still no tool.
    #[test]
    fn a_follow_up_is_the_first_request_extended_and_carries_no_tool() {
        let pass = QuarantinedPass::new("m", 128)
            .system("frame")
            .cache_prompt(true);
        let first = pass.ask("the question");
        let answer = Message::assistant(vec![crate::message::Block::text("the answer")]);
        let second = pass.follow_up("the question", answer.clone(), "and then?");
        assert!(second.tools.is_empty());
        assert_eq!(second.system, first.system);
        assert_eq!(second.messages.len(), 3);
        assert_eq!(second.messages[0], first.messages[0]);
        assert_eq!(second.messages[1], answer);
        assert_eq!(second.messages[2].text(), "and then?");
        assert!(!second.thinking && second.cache_prompt);
    }
}
