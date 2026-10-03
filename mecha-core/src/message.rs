//! Provider-agnostic conversation types.
//!
//! Every provider translates to and from these on the wire. Nothing in here
//! knows about Anthropic, OpenAI, or any particular JSON shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One piece of a message. A single assistant turn is often several blocks:
/// thinking, then text, then one or more tool calls.
///
/// `PartialEq` because session recording decides between "append the new
/// tail" and "the transcript was rewritten in place" by comparing the
/// messages a run started from with what it left behind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// Reasoning. `signature` is opaque and must be echoed back unchanged when
    /// continuing on the same model.
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// An image the user put in front of the model, or one a tool made.
    ///
    /// **User turns only, and that is a portability decision rather than a
    /// simplification.** Anthropic accepts an image inside a `tool_result`;
    /// the OpenAI dialect's `role: "tool"` messages carry a string and
    /// nothing else, and llama-server is the same. A tool that returned
    /// pixels would therefore work on one backend and silently lose them on
    /// the other — the shape of failure this project keeps finding, in the
    /// one place where the missing thing is what the whole turn was about.
    /// So an image enters the conversation the way a person hands one over,
    /// and `encode_message` renders it only on a user message — which is
    /// also where a tool's picture goes (`ToolOutput::image`, from
    /// `image_view`): into the user turn carrying the results, after them,
    /// never inside one.
    ///
    /// Not every model has eyes. `Provider::vision` says whether the one on
    /// the other end does, and a backend that cannot see renders this block
    /// as a line of text naming the file instead — so a run against a
    /// text-only model behaves exactly as it did before this variant
    /// existed, rather than failing on a request it cannot serve.
    Image {
        /// An IANA media type: `image/png`, `image/jpeg`, `image/gif`,
        /// `image/webp`. Both providers require it and neither sniffs.
        media_type: String,
        /// Base64, with **no `data:` prefix**. The prefix is a rendering
        /// detail of the OpenAI dialect — `anthropic.rs` wants the payload
        /// bare — so it belongs to the backend that needs it and not to the
        /// type every backend shares.
        data: String,
        /// What the file was called where it came from.
        ///
        /// Never sent to a provider. It exists because every *human*-facing
        /// reader of a transcript — `mecha sessions`, the TUI, `recall` —
        /// otherwise has a megabyte of base64 and no way to say what it was.
        /// It is also what the text-only rendering names.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
}

impl Block {
    pub fn text(s: impl Into<String>) -> Self {
        Block::Text { text: s.into() }
    }

    /// An image block from raw file bytes.
    ///
    /// One encoder, so the `data:` prefix question is answered once: what
    /// goes in is bare base64, and the one dialect that wants a prefix adds
    /// it at the wire.
    pub fn image(media_type: impl Into<String>, bytes: &[u8], source: Option<String>) -> Self {
        use base64::Engine as _;
        Block::Image {
            media_type: media_type.into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            source,
        }
    }

    /// How a person, or a model with no eyes, is told an image was here.
    ///
    /// Shared so the text-only provider rendering and every transcript
    /// reader say the same thing. A reader that invented its own wording
    /// would be a second answer to "what was in this turn".
    pub fn image_placeholder(media_type: &str, source: Option<&str>) -> String {
        match source {
            Some(name) => format!("[image: {name} ({media_type})]"),
            None => format!("[image: {media_type}]"),
        }
    }
}

/// The media type for a path, or `None` when it is not an image this system
/// will send.
///
/// An allowlist keyed on extension, deliberately, and deliberately short: it
/// is the intersection of what Anthropic accepts and what llama-server's
/// mtmd stack decodes. Sniffing the bytes would be more general and would
/// answer the wrong question — the point is not "is this an image" but "will
/// the thing on the other end take it", and a TIFF is an image that neither
/// backend will read.
pub fn image_media_type(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Local provenance keyed by tool-use id. Absent means unknown for old
    /// recordings; provider encoders never put this metadata on the wire.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tool_provenance: std::collections::BTreeMap<String, bool>,
    /// True for messages generated by the harness, never model or owner output.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub harness: bool,
    /// Harness observations, persisted locally and never encoded for a provider.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::planning::de_lenient"
    )]
    pub planning: Option<crate::planning::Feedback>,
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message {
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
            role: Role::User,
            content: vec![Block::text(text)],
        }
    }

    pub fn assistant(content: Vec<Block>) -> Self {
        Message {
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
            role: Role::Assistant,
            content,
        }
    }

    /// Tool results always go back as a single user message — splitting them
    /// across messages teaches the model to stop calling tools in parallel.
    pub fn tool_results(results: Vec<Block>) -> Self {
        Message {
            harness: false,
            planning: None,
            tool_provenance: Default::default(),
            role: Role::User,
            content: results,
        }
    }

    /// Concatenated text blocks, ignoring thinking and tool traffic.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Concatenated thinking blocks. Deliberately separate from `text`: this
    /// is deliberation, not an answer, and the two must never be confused —
    /// `text` is what the loop grades a turn on and what a caller receives.
    /// Read this only where the alternative is having nothing at all.
    pub fn thinking(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                Block::Thinking { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// A user message with no tool result in it — the person's own text (or
    /// the harness's folded beside it), never a completed tool round, which
    /// rides in a user message too. The one definition; see
    /// `agent::is_plain_user_text` for the bug that centralised it.
    pub fn is_plain_user_text(&self) -> bool {
        self.role == Role::User
            && !self
                .content
                .iter()
                .any(|b| matches!(b, Block::ToolResult { .. }))
    }

    pub fn tool_uses(&self) -> Vec<(&str, &str, &Value)> {
        self.content
            .iter()
            .filter_map(|b| match b {
                Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
                _ => None,
            })
            .collect()
    }
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Finished naturally.
    EndTurn,
    /// Wants one or more tools executed.
    ToolUse,
    /// Hit the output cap. Output is truncated.
    MaxTokens,
    /// Declined on safety grounds. `content` may be empty or partial.
    Refusal,
    /// Server-side tool loop paused; resend to continue.
    PauseTurn,
    Other,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }

    /// Total prompt size: the uncached remainder plus both cache tiers.
    pub fn total_input(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }

    /// What this cost, if the provider has prices configured.
    ///
    /// Cache reads and writes are billed at different multiples of the input
    /// rate, so a run that looks cheap on raw token counts can be anything but.
    pub fn cost_usd(&self, pricing: &Pricing) -> f64 {
        let per_input = pricing.input_per_mtok / 1_000_000.0;
        let per_output = pricing.output_per_mtok / 1_000_000.0;
        self.input_tokens as f64 * per_input
            + self.cache_creation_input_tokens as f64 * per_input * pricing.cache_write_multiplier
            + self.cache_read_input_tokens as f64 * per_input * pricing.cache_read_multiplier
            + self.output_tokens as f64 * per_output
    }
}

/// Per-million-token prices. Configured, never guessed — hardcoding a price
/// table guarantees it is wrong within a quarter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    /// Cache writes usually cost more than plain input.
    pub cache_write_multiplier: f64,
    /// Cache reads usually cost far less.
    pub cache_read_multiplier: f64,
}

impl Default for Pricing {
    fn default() -> Self {
        // The prevailing Anthropic ratios; override per provider in config.
        Pricing {
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            cache_write_multiplier: 1.25,
            cache_read_multiplier: 0.1,
        }
    }
}

/// A tool as the model sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// How hard the model should work. Maps to Anthropic's `output_config.effort`;
/// other providers approximate or ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

impl std::str::FromStr for Effort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Ok(Effort::Low),
            "medium" | "med" => Ok(Effort::Medium),
            "high" => Ok(Effort::High),
            "xhigh" | "x-high" => Ok(Effort::XHigh),
            "max" => Ok(Effort::Max),
            other => Err(format!(
                "unknown effort {other:?} (low|medium|high|xhigh|max)"
            )),
        }
    }
}

/// One request to a provider. Stateless — the full history goes every time.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Requested constrained JSON output. Providers must reject unsupported requests.
    pub response_schema: Option<serde_json::Value>,
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub effort: Option<Effort>,
    /// Ask the provider for a readable summary of the model's reasoning.
    pub thinking: bool,
    /// Mark the stable prefix (tools + system) as cacheable.
    pub cache_prompt: bool,
    /// Whether the model may reason before answering, where the server lets
    /// a request choose. `None` sends nothing and leaves the server's own
    /// default, which is what every request did before this field existed;
    /// `Some(false)` is for a short one-shot on the critical path (the voice
    /// director) that a thinking model would otherwise spend seconds on.
    /// Distinct from `thinking`, which only asks for a readable summary.
    pub think: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// Populated on `StopReason::Refusal`.
    pub refusal: Option<Refusal>,
    /// The model that actually served the response.
    pub model: String,
    /// Tool calls whose arguments did not parse as JSON. The single most
    /// useful reliability signal when comparing models: a model that is
    /// smarter but malforms arguments is worse in a loop.
    pub malformed_tool_args: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refusal {
    pub category: Option<String>,
    pub explanation: Option<String>,
}

/// What happens to reasoning from before the turn being answered when the
/// history goes back to the model. The transcript keeps every thinking
/// block either way; this decides only what is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriorThinking {
    /// Sent back as recorded — the assistant's runs, where a follow-up
    /// ("now the other file") leans on the analysis behind the last answer,
    /// and where keeping it makes each prompt a prefix of the next.
    #[default]
    Keep,
    /// Left out of earlier *replies* (a turn that called a tool keeps its
    /// reasoning; see `drops_thinking`): a persona chat. Replayed on
    /// 2026-10-02, a persona answering a four-character owner turn
    /// re-derived its earlier plan from its own preserved thinking and sent
    /// an earlier reply again word for word; with old thinking left out
    /// (and no fixed seed) 0 of 8 replies copied one.
    Drop,
}

/// The newest user message that carries no tool result — the turn the model
/// is answering now. Tool results come back as user messages, so "the last
/// user message" would move the cut inside a run and take the reasoning that
/// chose a call away from the step that reads its result.
fn answering(messages: &[Message]) -> Option<usize> {
    messages.iter().rposition(Message::is_plain_user_text)
}

/// Whether `message` loses its thinking under [`PriorThinking::Drop`]: an
/// assistant *reply* ahead of the turn being answered.
///
/// - **Never a turn that called a tool.** With reasoning stripped from
///   tool-calling turns in the history, llama-server's model emitted a bare
///   `<tool_call>` with no think block, 6 of 6, and the turn arrived empty
///   (2026-08-10, `provider::openai::encode_message`): shown its own calls
///   without thinking, it obliged. The replies that came back word for word
///   were plain text, so the repetition this cut is for is not there.
/// - **Never an all-thinking message**, which would be left empty — a 400 on
///   every provider.
fn drops_thinking(message: &Message, index: usize, cut: usize) -> bool {
    index < cut
        && message.role == Role::Assistant
        && message
            .content
            .iter()
            .any(|b| matches!(b, Block::Thinking { .. }))
        && !message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }))
        && message
            .content
            .iter()
            .any(|b| !matches!(b, Block::Thinking { .. }))
}

impl PriorThinking {
    /// The history as it goes on the wire.
    pub fn wire<'a>(self, messages: &'a [Message]) -> std::borrow::Cow<'a, [Message]> {
        let cut = match (self, answering(messages)) {
            (PriorThinking::Drop, Some(cut)) => cut,
            _ => return std::borrow::Cow::Borrowed(messages),
        };
        std::borrow::Cow::Owned(
            messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let mut m = m.clone();
                    if drops_thinking(&m, i, cut) {
                        m.content.retain(|b| !matches!(b, Block::Thinking { .. }));
                    }
                    m
                })
                .collect(),
        )
    }

    /// `pressure::message_bytes` of [`PriorThinking::wire`], without building
    /// it: every pressure reading asks this, several times a request, and a
    /// whole-history clone per reading is the cost the walk is kept cheap to
    /// avoid. The total less the thinking the cut leaves out.
    pub fn wire_bytes(self, messages: &[Message]) -> usize {
        let total = crate::pressure::message_bytes(messages);
        let cut = match (self, answering(messages)) {
            (PriorThinking::Drop, Some(cut)) => cut,
            _ => return total,
        };
        let dropped: usize = messages
            .iter()
            .enumerate()
            .filter(|(i, m)| drops_thinking(m, *i, cut))
            .flat_map(|(_, m)| &m.content)
            .filter(|b| matches!(b, Block::Thinking { .. }))
            .map(crate::pressure::block_bytes)
            .sum();
        total - dropped
    }
}

/// What happens to an earlier reply that stops mid-sentence when the history
/// goes back to the model. The transcript keeps every reply as it was
/// written or heard either way; this decides only what is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriorTails {
    /// Sent as recorded: the assistant's runs.
    #[default]
    Keep,
    /// Cut back to the last complete sentence ([`dangling_tail`]): a persona
    /// chat. A reply interrupted mid-sentence is stored as heard, and a model
    /// shown its own replies ending "…\n\nI" writes more of them: on
    /// 2026-10-03, 41 of a persona's 75 earlier replies ended mid-sentence
    /// and were spoken that way. Replayed with that history, 4 of 4 replies
    /// stopped mid-sentence; with the same history cut back, 0 of 4.
    Trim,
}

/// Where `text` is cut so it ends on a complete sentence, when it stops
/// mid-sentence: `Some(len)` to keep `text[..len]`, `None` to leave it.
///
/// **Mid-sentence is narrow on purpose:** the last character continues a
/// clause — a letter, a digit, or `, ; : - – —`. A reply ending on a
/// sentence mark, an ellipsis, a closing quote or an emoji is whole, and a
/// reply with no complete sentence before the break ("Mmm, I") is left
/// alone rather than emptied.
pub fn dangling_tail(text: &str) -> Option<usize> {
    let body = text.trim_end();
    let last = body.chars().next_back()?;
    if !(last.is_alphanumeric() || matches!(last, ',' | ';' | ':' | '-' | '–' | '—')) {
        return None;
    }
    // The end of the last sentence mark (with any closing quote, bracket or
    // emphasis after it) that whitespace follows.
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    let mut cut = None;
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i].1, '.' | '!' | '?' | '…') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '…') {
                j += 1;
            }
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '"' | '\'' | '”' | '’' | ')' | ']' | '*' | '_' | '~'
                )
            {
                j += 1;
            }
            if j < chars.len() && chars[j].1.is_whitespace() {
                cut = Some(chars[j].0);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    cut
}

/// The text block a trim applies to: the last one of an earlier plain reply.
fn tail_block(message: &Message, index: usize, cut: usize) -> Option<usize> {
    if index >= cut
        || message.role != Role::Assistant
        || message
            .content
            .iter()
            .any(|b| matches!(b, Block::ToolUse { .. }))
    {
        return None;
    }
    message
        .content
        .iter()
        .rposition(|b| matches!(b, Block::Text { .. }))
}

impl PriorTails {
    /// `messages` with every earlier reply that stops mid-sentence cut back.
    pub fn wire<'a>(
        self,
        messages: std::borrow::Cow<'a, [Message]>,
    ) -> std::borrow::Cow<'a, [Message]> {
        let cut = match (self, answering(&messages)) {
            (PriorTails::Trim, Some(cut)) => cut,
            _ => return messages,
        };
        let trims: Vec<(usize, usize, usize)> = messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let b = tail_block(m, i, cut)?;
                let Block::Text { text } = &m.content[b] else {
                    return None;
                };
                Some((i, b, dangling_tail(text)?))
            })
            .collect();
        if trims.is_empty() {
            return messages;
        }
        let mut owned = messages.into_owned();
        for (i, b, len) in trims {
            if let Block::Text { text } = &mut owned[i].content[b] {
                text.truncate(len);
            }
        }
        std::borrow::Cow::Owned(owned)
    }

    /// The bytes [`PriorTails::wire`] cuts, for `wire_bytes`.
    pub fn dropped_bytes(self, messages: &[Message]) -> usize {
        let cut = match (self, answering(messages)) {
            (PriorTails::Trim, Some(cut)) => cut,
            _ => return 0,
        };
        messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let Block::Text { text } = &m.content[tail_block(m, i, cut)?] else {
                    return None;
                };
                Some(text.len() - dangling_tail(text)?)
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn think(t: &str) -> Block {
        Block::Thinking {
            text: t.into(),
            signature: None,
        }
    }

    fn thoughts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .map(Message::thinking)
            .filter(|t| !t.is_empty())
            .collect()
    }

    #[test]
    fn dropping_prior_thinking_cuts_at_the_turn_being_answered() {
        let call = |id: &str| Block::ToolUse {
            id: id.into(),
            name: "echo".into(),
            input: json!({}),
        };
        let history = vec![
            Message::user("hi"),
            // An earlier turn's tool call keeps its reasoning: stripped, the
            // model learns to call without thinking (`drops_thinking`).
            Message::assistant(vec![think("chose"), call("t0")]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t0".into(),
                content: "ok".into(),
                is_error: false,
            }]),
            // An earlier reply does not.
            Message::assistant(vec![think("old"), Block::text("hello")]),
            // All thinking: kept whole, or the message would be empty.
            Message::assistant(vec![think("alone")]),
            Message::user("mm"),
            Message::assistant(vec![
                think("now"),
                Block::ToolUse {
                    id: "t1".into(),
                    name: "echo".into(),
                    input: json!({}),
                },
            ]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            }]),
        ];
        let wire = PriorThinking::Drop.wire(&history);
        assert_eq!(thoughts(&wire), ["chose", "alone", "now"]);
        // The pressure reading measures exactly what is sent, both ways.
        for prior in [PriorThinking::Drop, PriorThinking::Keep] {
            assert_eq!(
                prior.wire_bytes(&history),
                crate::pressure::message_bytes(&prior.wire(&history))
            );
        }
        assert!(
            PriorThinking::Drop.wire_bytes(&history) < PriorThinking::Keep.wire_bytes(&history)
        );
        assert_eq!(wire[3].text(), "hello");
        assert_eq!(wire.len(), history.len());
        // Keep borrows: no copy of a long history on every request.
        assert!(matches!(
            PriorThinking::Keep.wire(&history),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(thoughts(&history), ["chose", "old", "alone", "now"]);
    }

    #[test]
    fn with_no_owner_turn_nothing_is_dropped() {
        let only_results = vec![Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "t1".into(),
            content: "ok".into(),
            is_error: false,
        }])];
        assert!(matches!(
            PriorThinking::Drop.wire(&only_results),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn message_text_ignores_thinking_and_tool_traffic() {
        // `text()` is what every grader, session summary and final answer reads.
        // Letting reasoning leak into it would put the model's scratchpad in
        // front of the user and into eval assertions.
        let m = Message::assistant(vec![
            Block::Thinking {
                text: "let me think".into(),
                signature: Some("sig".into()),
            },
            Block::text("the answer is "),
            Block::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({}),
            },
            Block::text("42"),
        ]);

        assert_eq!(m.text(), "the answer is 42");
    }

    #[test]
    fn tool_uses_reports_every_call_in_order() {
        let m = Message::assistant(vec![
            Block::ToolUse {
                id: "t1".into(),
                name: "fs_read".into(),
                input: json!({"path": "a"}),
            },
            Block::text("and also"),
            Block::ToolUse {
                id: "t2".into(),
                name: "shell".into(),
                input: json!({"cmd": "ls"}),
            },
        ]);

        let calls = m.tool_uses();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].0, calls[0].1), ("t1", "fs_read"));
        assert_eq!((calls[1].0, calls[1].1), ("t2", "shell"));
    }

    #[test]
    fn tool_results_travel_as_one_user_message() {
        // Splitting them across messages teaches the model to stop calling
        // tools in parallel, which is a behavioural regression no test of the
        // wire format would catch.
        let m = Message::tool_results(vec![
            Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "a".into(),
                is_error: false,
            },
            Block::ToolResult {
                tool_use_id: "t2".into(),
                content: "b".into(),
                is_error: true,
            },
        ]);

        assert_eq!(m.role, Role::User);
        assert_eq!(m.content.len(), 2);
    }

    #[test]
    fn a_block_round_trips_through_the_session_format() {
        // Transcripts are JSONL, so every block has to survive serialisation.
        // A thinking block with no signature must not grow a null one: the API
        // rejects reconstructed signatures, and `None` is how we know to drop
        // it rather than replay it.
        let blocks = vec![
            Block::text("hello"),
            Block::Thinking {
                text: "hm".into(),
                signature: None,
            },
            Block::Thinking {
                text: "hm".into(),
                signature: Some("sig".into()),
            },
            Block::ToolUse {
                id: "t1".into(),
                name: "echo".into(),
                input: json!({"v": 1}),
            },
            Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "1".into(),
                is_error: true,
            },
        ];

        let encoded = serde_json::to_string(&blocks).unwrap();
        assert!(
            !encoded.contains("\"signature\":null"),
            "an absent signature was written out"
        );

        let decoded: Vec<Block> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.len(), blocks.len());
        match &decoded[1] {
            Block::Thinking { signature, .. } => assert!(signature.is_none()),
            other => panic!("expected thinking, got {other:?}"),
        }
        match &decoded[4] {
            Block::ToolResult { is_error, .. } => assert!(is_error),
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    #[test]
    fn an_older_transcript_without_is_error_still_loads() {
        // `is_error` is `#[serde(default)]` precisely so a transcript written
        // before it existed still resumes.
        let block: Block = serde_json::from_value(
            json!({"type": "tool_result", "tool_use_id": "t1", "content": "x"}),
        )
        .unwrap();
        match block {
            Block::ToolResult { is_error, .. } => assert!(!is_error),
            other => panic!("expected a tool result, got {other:?}"),
        }
    }

    #[test]
    fn total_input_counts_both_cache_tiers() {
        // The compaction threshold reads the *reported* prompt size, so a
        // total that forgot the cached tiers would let a session grow past the
        // window while claiming to be small.
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 3000,
        };
        assert_eq!(usage.total_input(), 3300);
    }

    #[test]
    fn usage_accumulates_every_field() {
        let mut a = Usage {
            input_tokens: 1,
            output_tokens: 2,
            ..Usage::default()
        };
        a.add(&Usage {
            input_tokens: 10,
            output_tokens: 20,
            cache_creation_input_tokens: 30,
            cache_read_input_tokens: 40,
        });

        assert_eq!(a.input_tokens, 11);
        assert_eq!(a.output_tokens, 22);
        assert_eq!(a.cache_creation_input_tokens, 30);
        assert_eq!(a.cache_read_input_tokens, 40);
    }

    #[test]
    fn cache_reads_and_writes_are_priced_off_the_input_rate() {
        // A run that looks cheap on raw token counts can be anything but, which
        // is the whole reason the tiers are tracked separately.
        let pricing = Pricing {
            input_per_mtok: 1_000_000.0, // one dollar per token, to keep it readable
            output_per_mtok: 2_000_000.0,
            cache_write_multiplier: 1.25,
            cache_read_multiplier: 0.1,
        };
        let usage = Usage {
            input_tokens: 1,
            output_tokens: 1,
            cache_creation_input_tokens: 1,
            cache_read_input_tokens: 1,
        };

        // 1 + 2 + 1.25 + 0.1
        assert!((usage.cost_usd(&pricing) - 4.35).abs() < 1e-9);
    }

    #[test]
    fn a_provider_with_no_prices_configured_costs_nothing_rather_than_guessing() {
        // Hardcoding a price table guarantees it is wrong within a quarter, so
        // the default has to be zero rather than a plausible number.
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Usage::default()
        };
        assert_eq!(usage.cost_usd(&Pricing::default()), 0.0);
    }

    #[test]
    fn effort_parses_its_aliases_and_refuses_anything_else() {
        use std::str::FromStr;

        for (input, expected) in [
            ("low", Effort::Low),
            ("MEDIUM", Effort::Medium),
            ("med", Effort::Medium),
            ("high", Effort::High),
            ("xhigh", Effort::XHigh),
            ("x-high", Effort::XHigh),
            ("max", Effort::Max),
        ] {
            assert_eq!(
                Effort::from_str(input).unwrap(),
                expected,
                "parsing {input}"
            );
        }

        let err = Effort::from_str("turbo").unwrap_err();
        assert!(
            err.contains("turbo") && err.contains("low|medium|high"),
            "unhelpful: {err}"
        );
    }

    #[test]
    fn every_effort_round_trips_through_its_wire_name() {
        use std::str::FromStr;
        for effort in [
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::XHigh,
            Effort::Max,
        ] {
            assert_eq!(Effort::from_str(effort.as_str()).unwrap(), effort);
        }
    }

    #[test]
    fn a_reply_that_stops_mid_sentence_is_cut_to_its_last_whole_sentence() {
        let cut = |t: &str| dangling_tail(t).map(|n| t[..n].to_string());
        // The endings the persona wrote and spoke on 2026-10-03.
        assert_eq!(
            cut("You love it, don't you? \n\nI").as_deref(),
            Some("You love it, don't you?")
        );
        assert_eq!(
            cut("Fuck me harder, baby.\"\n\nI").as_deref(),
            Some("Fuck me harder, baby.\"")
        );
        assert_eq!(
            cut("Are you almost done with your errands, or are").as_deref(),
            None,
            "no whole sentence before it: left alone, never emptied"
        );
        assert_eq!(
            cut("Done already? Are you getting more tech").as_deref(),
            Some("Done already?")
        );
        assert_eq!(cut("It's late… so").as_deref(), Some("It's late…"));
        // Whole replies are untouched.
        for whole in [
            "Tell me what you're getting.",
            "Shhh.. that's it..",
            "Come here 😘",
            "\"I need you.\"",
            "Mmm, I",
            "",
        ] {
            assert_eq!(dangling_tail(whole), None, "{whole:?}");
        }
        // A sentence mark inside a word is not a sentence end.
        assert_eq!(cut("See v1.2 and then"), None);
    }

    #[test]
    fn trimming_tails_cuts_earlier_plain_replies_only_and_counts_what_it_cut() {
        let history = vec![
            Message::user("hi"),
            Message::assistant(vec![think("old"), Block::text("Hello there. \n\nI")]),
            // A turn that called a tool is left as it was.
            Message::assistant(vec![
                Block::text("Let me look. Then"),
                Block::ToolUse {
                    id: "t0".into(),
                    name: "echo".into(),
                    input: json!({}),
                },
            ]),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t0".into(),
                content: "ok".into(),
                is_error: false,
            }]),
            Message::assistant(vec![Block::text("Found it. And")]),
            Message::user("mm"),
            // The turn being answered: never touched.
            Message::assistant(vec![Block::text("Still going. So")]),
        ];
        let sent = PriorTails::Trim.wire(std::borrow::Cow::Borrowed(&history[..]));
        let texts: Vec<String> = sent.iter().map(Message::text).collect();
        assert_eq!(texts[1], "Hello there.");
        assert_eq!(texts[2], "Let me look. Then", "a tool-calling turn");
        assert_eq!(texts[4], "Found it.");
        assert_eq!(texts[6], "Still going. So", "the reply being written");
        assert_eq!(
            sent[1].thinking(),
            "old",
            "thinking is PriorThinking's to drop"
        );
        assert_eq!(
            PriorTails::Trim.dropped_bytes(&history),
            crate::pressure::message_bytes(&history) - crate::pressure::message_bytes(&sent),
            "wire_bytes must measure what is sent"
        );
        let kept = PriorTails::Keep.wire(std::borrow::Cow::Borrowed(&history[..]));
        assert!(matches!(kept, std::borrow::Cow::Borrowed(_)));
        assert_eq!(PriorTails::Keep.dropped_bytes(&history), 0);
    }
}
