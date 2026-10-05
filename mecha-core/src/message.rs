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
    /// The most tokens the model may reason for before it must answer, where
    /// the server lets a request choose (llama-server's
    /// `reasoning_budget_tokens`, which otherwise falls back to the server's
    /// `--reasoning-budget`). `None` sends nothing. Anthropic's adaptive
    /// thinking takes no budget, so it is not sent there.
    pub think_budget: Option<u32>,
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
    if !ends_mid_clause(body) {
        return None;
    }
    last_sentence_end(body)
}

/// Whether `text` stops mid-clause: its last character, past trailing
/// whitespace, is a letter, a digit, or `, ; : - – —`. The test
/// [`dangling_tail`] cuts on; after its cut, a reply still answering true
/// had no complete sentence to cut back to.
pub fn ends_mid_clause(text: &str) -> bool {
    text.trim_end().chars().next_back().is_some_and(|last| {
        last.is_alphanumeric() || matches!(last, ',' | ';' | ':' | '-' | '–' | '—')
    })
}

/// Where the last complete sentence of `text` ends: just past its last
/// sentence mark (with any closing quote, bracket or emphasis after it) that
/// whitespace follows, skipping a lone period that ends an abbreviation or an
/// initial. `None` when no sentence ends before the end of the text. The one
/// sentence boundary the persona's history views share: [`dangling_tail`]
/// cuts there, and `persona::variety` quotes what follows as a closing line
/// (review of #550).
pub fn last_sentence_end(text: &str) -> Option<usize> {
    let body = text.trim_end();
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    let mut cut = None;
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i].1, '.' | '!' | '?' | '…') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '…') {
                j += 1;
            }
            let lone_period = j == i + 1 && chars[i].1 == '.';
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '"' | '\'' | '”' | '’' | ')' | ']' | '*' | '_' | '~'
                )
            {
                j += 1;
            }
            if j < chars.len()
                && chars[j].1.is_whitespace()
                && !(lone_period && abbreviation(&chars, i, j))
            {
                cut = Some(chars[j].0);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    cut
}

/// Whether the lone period at `dot` ends an abbreviation rather than a
/// sentence: the word before it is a title or Latin short form ("Dr.",
/// "e.g.") or a single initial, or the text after it carries on in lower
/// case. A dangling reply is the only input [`dangling_tail`] sees, so a
/// mistaken mark is often the last one and would win the cut: "…I saw Dr."
/// is itself a reply ending mid-clause (review of #538).
fn abbreviation(chars: &[(usize, char)], dot: usize, after: usize) -> bool {
    const SHORT: &[&str] = &[
        "mr", "mrs", "ms", "dr", "st", "jr", "sr", "vs", "etc", "eg", "ie", "prof", "no",
    ];
    let start = chars[..dot]
        .iter()
        .rposition(|(_, c)| !(c.is_alphabetic() || *c == '.'))
        .map_or(0, |p| p + 1);
    let word: String = chars[start..dot]
        .iter()
        .map(|(_, c)| *c)
        .filter(|c| *c != '.')
        .flat_map(char::to_lowercase)
        .collect();
    let next = chars[after..]
        .iter()
        .map(|(_, c)| *c)
        .find(|c| !c.is_whitespace());
    word.chars().count() == 1
        || SHORT.contains(&word.as_str())
        || next.is_some_and(char::is_lowercase)
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

/// What happens to a one-turn harness nudge folded into an earlier owner turn
/// when the history goes back to the model. The transcript keeps every one;
/// this decides only what is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriorNudges {
    /// Sent as recorded.
    #[default]
    Keep,
    /// Every one but the newest left out, where that costs the cache nothing
    /// (`stale_nudges`): a persona chat.
    /// `persona::variety`'s note is folded each turn and names what is
    /// repeating *now*; kept, forty turns in the prefix would be forty stale,
    /// contradictory "don't end on X" lines, which is not the condition the
    /// note was measured in (one note, a clean history; review of #550).
    Drop,
}

/// Which one-turn nudge `block` is, if any: `persona::variety`'s note and
/// `persona::edit`'s. `persona::call::note` also lives one turn but is not
/// dropped, and a new one-turn nudge is not covered until it is added here.
fn one_turn_nudge(block: &Block) -> Option<Nudge> {
    match block {
        Block::Text { text } if crate::persona::variety::is_note(text) => Some(Nudge::Variety),
        Block::Text { text } if crate::persona::edit::is_note(text) => Some(Nudge::Edit),
        _ => None,
    }
}

/// The kinds of one-turn nudge, each kept or dropped on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nudge {
    Variety,
    Edit,
}

fn is_one_turn_nudge(block: &Block) -> bool {
    one_turn_nudge(block).is_some()
}

/// The one-turn nudges [`PriorNudges::Drop`] leaves out, as (message, block)
/// positions in `recorded`: every one but the newest of its kind, when
/// leaving it out is cheap. "Newest", not "the turn being answered": a turn
/// folded into an earlier owner message (a turn that died before a reply) can
/// put two notes in one message (review of #550). And the newest of a kind
/// stays only while it sits in the newest message carrying any nudge: an
/// edit turn carries the variety note and the edit note side by side, and
/// both are its own, but the edit note of a turn three back is stale once a
/// later turn has carried a variety note, though no edit note has followed it.
/// The edit note is dropped whatever the re-read costs (see the filter).
///
/// **Cheap** is [`reread_bytes`] within [`NUDGE_REREAD_BYTES`]. Removing a
/// note rewrites a request already sent, so the slot's cache diverges at
/// it, and the server re-reads what follows up to the point it would have
/// re-read anyway. That is nothing when the reply after the note goes out
/// *altered* — its thinking stripped (`PriorThinking::Drop`) or its tail
/// trimmed (`PriorTails::Trim`) — since the slot holds it as generated. A
/// reply that goes out as recorded matches the slot straight through: a turn
/// that called a tool keeps its thinking, byte-identical under the router's
/// `reasoning-preserve`, and so does a reply that came back with none
/// (review of #550). So a note ahead of a tool round trip costs that round
/// trip, once, on a spoken turn's latency path; the cap keeps the note
/// ahead of a large one. `earlier` is the history after the views that run
/// first: they alter assistant messages only and never remove one, so
/// positions in `recorded` hold in it. A block whose removal would leave its
/// message empty stays too.
fn stale_nudges(recorded: &[Message], earlier: &[Message]) -> Vec<(usize, usize)> {
    let all: Vec<(usize, usize, Nudge)> = recorded
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::User)
        .flat_map(|(i, m)| {
            m.content
                .iter()
                .enumerate()
                .filter_map(move |(j, b)| one_turn_nudge(b).map(|kind| (i, j, kind)))
        })
        .collect();
    let Some(&(newest, _, _)) = all.last() else {
        return Vec::new();
    };
    let kept = |i: usize, j: usize, kind: Nudge| {
        i == newest && all.iter().rev().find(|n| n.2 == kind).map(|n| (n.0, n.1)) == Some((i, j))
    };
    all.iter()
        .filter(|&&(i, j, kind)| !kept(i, j, kind))
        .filter(|&&(i, _, _)| recorded[i].content.iter().any(|b| !is_one_turn_nudge(b)))
        // The cap is a spoken turn's latency control; the edit note rides
        // typed turns only, and every edit turn is a tool round trip, the
        // shape the cap keeps a note ahead of (5 of 30 edit turns measured
        // past it, 2026-10-05). Kept, it would tell the persona a later
        // typed message came from the edit panel (review of #566).
        .filter(|&&(i, _, kind)| {
            kind == Nudge::Edit || reread_bytes(recorded, earlier, i + 1) <= NUDGE_REREAD_BYTES
        })
        .map(|&(i, j, _)| (i, j))
        .collect()
}

/// The most a stale nudge's removal may make the server re-read, once, on
/// the turn after: about 2,000 tokens, ~1 s of prefill at the router's
/// measured ~1,800 tokens/s (2026-10-04). The owner's ruling (2026-10-04)
/// between keeping every note ahead of a tool turn — a quarter of persona
/// turns call one, so a 40-turn chat would carry ~10 stale notes — and
/// re-reading every round trip in full (0.4 s at the median, ~3 s at p90,
/// ~12 s at the largest of 57 measured): this drops the note ahead of about
/// three round trips in four.
const NUDGE_REREAD_BYTES: usize = 8 * 1024;

/// What removing a nudge from the message before `from` makes the server
/// re-read: the messages from `from` on that go out as recorded, up to the
/// first one that goes out altered (the slot re-reads from there anyway) or
/// the next owner message (the turn's end). Bounded by the turn so the
/// answer never changes as the chat grows: a note dropped once stays
/// dropped, and one kept stays kept, since flipping either way would itself
/// re-read everything after it.
fn reread_bytes(recorded: &[Message], earlier: &[Message], from: usize) -> usize {
    let bytes = |m: &Message| crate::pressure::message_bytes(std::slice::from_ref(m));
    let mut total = 0;
    for (next, sent) in recorded.iter().zip(earlier).skip(from) {
        if next.is_plain_user_text() || bytes(sent) < bytes(next) {
            break;
        }
        total += bytes(sent);
    }
    total
}

impl PriorNudges {
    /// `earlier` (the history after the views that run first) with every
    /// one-turn nudge but the newest left out, where that is free, judged
    /// against `recorded` (`stale_nudges`).
    pub fn wire<'a>(
        self,
        recorded: &[Message],
        earlier: std::borrow::Cow<'a, [Message]>,
    ) -> std::borrow::Cow<'a, [Message]> {
        if self == PriorNudges::Keep {
            return earlier;
        }
        let stale = stale_nudges(recorded, &earlier);
        if stale.is_empty() {
            return earlier;
        }
        let mut owned = earlier.into_owned();
        // From the back, so earlier block indices stay valid.
        for &(i, j) in stale.iter().rev() {
            owned[i].content.remove(j);
        }
        std::borrow::Cow::Owned(owned)
    }

    /// The bytes [`PriorNudges::wire`] leaves out, for `wire_bytes`.
    pub fn dropped_bytes(self, recorded: &[Message], earlier: &[Message]) -> usize {
        if self == PriorNudges::Keep {
            return 0;
        }
        stale_nudges(recorded, earlier)
            .into_iter()
            .map(|(i, j)| crate::pressure::block_bytes(&recorded[i].content[j]))
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
        // Shaped like the endings a persona wrote and spoke on 2026-10-03; the
        // words are made up (no chat's text in the repository).
        assert_eq!(
            cut("You liked that one, didn't you? \n\nI").as_deref(),
            Some("You liked that one, didn't you?")
        );
        assert_eq!(
            cut("Read it to me again.\"\n\nI").as_deref(),
            Some("Read it to me again.\"")
        );
        assert_eq!(
            cut("Are you nearly at the station, or are").as_deref(),
            None,
            "no whole sentence before it: left alone, never emptied"
        );
        assert_eq!(
            cut("Done already? Are you buying more paint").as_deref(),
            Some("Done already?")
        );
        assert_eq!(
            cut("It's getting dark… so").as_deref(),
            Some("It's getting dark…")
        );
        // Whole replies are untouched.
        for whole in [
            "Tell me how the walk went.",
            "Hmm.. let me see..",
            "See you soon 😊",
            "\"I need a minute.\"",
            "Mmm, I",
            "",
        ] {
            assert_eq!(dangling_tail(whole), None, "{whole:?}");
        }
        // A sentence mark inside a word is not a sentence end.
        assert_eq!(cut("See v1.2 and then"), None);
        // Nor is an abbreviation (review of #538): the reviewer's two cases
        // are left whole rather than cut to "…Dr." or "…e.g.".
        assert_eq!(cut("Hmm, I saw Dr. Chen today and he"), None);
        assert_eq!(cut("I was at the store, e.g. the big one, and then"), None);
        assert_eq!(cut("We met J. Smith there and"), None);
        assert_eq!(
            cut("It was fine. I saw Dr. Chen and").as_deref(),
            Some("It was fine.")
        );
        // A real sentence end still cuts, with a capital or a quote after it.
        assert_eq!(cut("Come here. \"Now").as_deref(), Some("Come here."));
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

    #[test]
    fn an_edit_turn_keeps_both_its_notes_and_a_later_turn_drops_them() {
        let variety = |closer: &str| {
            Block::text(format!(
                "{} (\"{closer}\").)",
                crate::persona::variety::CLOSING_STEM
            ))
        };
        let edit_turn = |said: &str, closer: &str| {
            let mut m = Message::user(said);
            m.content.push(variety(closer));
            m.content.push(Block::text(crate::persona::edit::note()));
            m
        };
        let reply = |text: &str| Message::assistant(vec![Block::text(text)]);
        let kinds =
            |m: &Message| -> Vec<Nudge> { m.content.iter().filter_map(one_turn_nudge).collect() };
        let send = |history: &[Message]| -> Vec<Message> {
            PriorNudges::Drop
                .wire(history, std::borrow::Cow::Borrowed(history))
                .into_owned()
        };

        // The edit turn being answered carries both, and both go out: one
        // newest-of-all would drop its own variety note.
        let history = vec![
            Message::user("hi"),
            reply("Hello."),
            edit_turn("Edit images/a.png: make the sky pink", "Hello."),
        ];
        assert_eq!(kinds(&send(&history)[2]), vec![Nudge::Variety, Nudge::Edit]);

        // A later turn that carries only a variety note: the edit note is
        // stale with it, though no edit note followed.
        let mut later = Message::user("lovely");
        later.content.push(variety("Done."));
        let history = vec![
            edit_turn("Edit images/a.png: make the sky pink", "Hello."),
            reply("Done."),
            later,
        ];
        let sent = send(&history);
        assert_eq!(kinds(&sent[0]), Vec::<Nudge>::new());
        assert_eq!(kinds(&sent[2]), vec![Nudge::Variety]);

        // An edit turn is a tool round trip, and one past the re-read cap
        // keeps a variety note ahead of it; the edit note still goes, or the
        // next typed message would read as the panel's (review of #566).
        let call = Message::assistant(vec![
            Block::Thinking {
                text: "draw it".into(),
                signature: None,
            },
            Block::ToolUse {
                id: "i1".into(),
                name: "image_generate".into(),
                input: serde_json::json!({}),
            },
        ]);
        let drawn = Message::tool_results(vec![Block::ToolResult {
            tool_use_id: "i1".into(),
            content: "r".repeat(NUDGE_REREAD_BYTES + 1),
            is_error: false,
        }]);
        let mut later = Message::user("lovely");
        later.content.push(variety("Done."));
        let history = vec![
            edit_turn("Edit images/a.png: make the sky pink", "Hello."),
            call,
            drawn,
            reply("Done."),
            later,
        ];
        let sent = send(&history);
        assert_eq!(
            kinds(&sent[0]),
            vec![Nudge::Variety],
            "kept ahead of a big round trip"
        );
        assert_eq!(kinds(&sent[4]), vec![Nudge::Variety]);
    }

    #[test]
    fn only_the_turn_being_answered_keeps_its_one_turn_nudge() {
        let note = |closer: &str| {
            Block::text(format!(
                "{} (\"{closer}\").)",
                crate::persona::variety::CLOSING_STEM
            ))
        };
        let owner = |said: &str, closer: &str| {
            let mut m = Message::user(said);
            m.content.push(note(closer));
            m
        };
        // A reply as the persona model returns one: with its reasoning.
        let reply = |text: &str| {
            Message::assistant(vec![
                Block::Thinking {
                    text: "they asked again".into(),
                    signature: None,
                },
                Block::text(text),
            ])
        };
        let tool_turn = |id: &str| {
            Message::assistant(vec![
                Block::Thinking {
                    text: "look it up".into(),
                    signature: None,
                },
                Block::ToolUse {
                    id: id.into(),
                    name: "echo".into(),
                    input: serde_json::json!({}),
                },
            ])
        };
        let result = |id: &str, bytes: usize| {
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: id.into(),
                content: "r".repeat(bytes),
                is_error: false,
            }])
        };
        // The views in `Agent::wire`'s order, and the bytes check
        // `wire_bytes` relies on.
        let send = |history: &[Message]| -> Vec<Message> {
            let earlier = PriorTails::Trim.wire(PriorThinking::Drop.wire(history));
            let dropped = PriorNudges::Drop.dropped_bytes(history, &earlier);
            let before = crate::pressure::message_bytes(&earlier);
            let sent = PriorNudges::Drop.wire(history, earlier).into_owned();
            assert_eq!(
                dropped,
                before - crate::pressure::message_bytes(&sent),
                "wire_bytes must measure what is sent"
            );
            sent
        };
        let with_note = |sent: &[Message]| -> Vec<usize> {
            sent.iter()
                .enumerate()
                .filter(|(_, m)| m.content.iter().any(is_one_turn_nudge))
                .map(|(i, _)| i)
                .collect()
        };

        let history = vec![
            owner("hi", "Old one."),
            reply("Hello."),
            owner("and now", "Older two."),
            reply("Sure."),
            owner("again", "Current."),
        ];
        let sent = send(&history);
        assert_eq!(
            with_note(&sent),
            vec![4],
            "only the turn being answered keeps its note"
        );
        assert_eq!(sent[0].text(), "hi", "the owner's words stay");
        let kept = PriorNudges::Keep.wire(&history, std::borrow::Cow::Borrowed(&history[..]));
        assert!(matches!(kept, std::borrow::Cow::Borrowed(_)));

        // A message that is only a (stale) nudge is never emptied.
        let mut lone = Message::user("");
        lone.content = vec![note("Lone.")];
        let sent = send(&[lone, reply("Hm?"), owner("now", "Newer.")]);
        assert!(!sent[0].content.is_empty());

        // The fold path: a turn that died before a reply leaves the next
        // turn's note folded into the same owner message, so one message
        // carries two. Only the newer goes out.
        let mut folded = owner("hello, and hello again", "Older.");
        folded.content.push(note("Newest."));
        let sent = send(std::slice::from_ref(&folded));
        let left: Vec<&Block> = sent[0]
            .content
            .iter()
            .filter(|b| is_one_turn_nudge(b))
            .collect();
        assert_eq!(left.len(), 1);
        assert!(matches!(left[0], Block::Text { text } if text.contains("Newest.")));

        // Removing a note makes the server re-read what follows it up to the
        // first message that goes out altered (review of #550). A tool turn
        // keeps its thinking and goes back as the slot holds it, as does a
        // reply that came back with no reasoning, so the note ahead of one
        // costs that stretch: dropped when it is small, kept ahead of a
        // round trip past `NUDGE_REREAD_BYTES`.
        let history = vec![
            owner("hi", "Before a reply."),
            reply("Hello."),
            owner("look it up", "Before a small tool."),
            tool_turn("t1"),
            result("t1", 2_000),
            reply("Found it."),
            owner("and?", "Before a bare reply."),
            Message::assistant(vec![Block::text("Mm.")]),
            owner("read it all", "Before a big tool."),
            tool_turn("t2"),
            result("t2", NUDGE_REREAD_BYTES + 1),
            reply("That's long."),
            owner("thanks", "Current."),
        ];
        assert_eq!(with_note(&send(&history)), vec![8, 12]);
        // The stretch ends at the turn: a later turn never adds to it, so a
        // note's fate does not change as the chat grows.
        let earlier = PriorTails::Trim.wire(PriorThinking::Drop.wire(&history));
        assert_eq!(
            reread_bytes(&history, &earlier, 9),
            crate::pressure::message_bytes(&history[9..11]),
            "the call and its result, not the altered reply after them"
        );
        assert_eq!(
            reread_bytes(&history, &earlier, 7),
            crate::pressure::message_bytes(&history[7..8]),
            "the bare reply, and not the next turn"
        );

        // A turn cancelled after a small tool ran folds the next note beside
        // the tool result: one note goes out.
        let mut results = result("t0", 100);
        results.content.push(Block::text("and now?"));
        results.content.push(note("Newest."));
        let history = vec![owner("look it up", "Older."), tool_turn("t0"), results];
        assert_eq!(with_note(&send(&history)), vec![2]);

        // What the cap is priced against: the slot matches each request
        // against the *previous* one (and what it generated), not against
        // the recorded history. Grown turn by turn, every request repeats
        // the one before it exactly, up to the owner message whose note has
        // just gone stale — an older note dropped long ago diverges nothing,
        // because the previous request lacked it too (review of #550).
        let turns: Vec<Vec<Message>> = vec![
            vec![owner("hi", "One.")],
            vec![reply("Hello."), owner("read it all", "Two.")],
            vec![
                tool_turn("t2"),
                result("t2", NUDGE_REREAD_BYTES + 1),
                reply("That's long."),
                owner("and?", "Three."),
            ],
            vec![reply("Mm, yes."), owner("thanks", "Four.")],
        ];
        let mut history: Vec<Message> = Vec::new();
        let mut previous: Option<(Vec<Message>, usize)> = None;
        for turn in turns {
            history.extend(turn);
            let sent = send(&history);
            let answering = history.len() - 1;
            if let Some((before, stale)) = &previous {
                assert_eq!(
                    sent[..*stale],
                    before[..*stale],
                    "a request rewrote more than the note that just went stale"
                );
            }
            previous = Some((sent, answering));
        }
        // And the final request: "One." and "Three." dropped (a plain reply
        // after each), "Two." kept ahead of the large round trip — the state
        // the review read as "diverged at 0, so 8 KiB spent anyway".
        let sent = send(&history);
        assert_eq!(with_note(&sent), vec![2, 8]);
    }
}
