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
//!   each open-coded.
//! - **`thinking` is always false.** It asks the provider for a readable
//!   summary of the model's reasoning, which exists for a human watching a
//!   run. Nothing watches these. Every one of the eight sites set it false.
//!
//! [`ask`]: QuarantinedPass::ask

use crate::message::{CompletionRequest, Effort, Message};

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
            model: self.model.clone(),
            system: self.system.clone(),
            messages: vec![Message::user(user.into())],
            tools: Vec::new(),
            max_tokens: self.max_tokens,
            effort: self.effort,
            thinking: false,
            cache_prompt: self.cache_prompt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
