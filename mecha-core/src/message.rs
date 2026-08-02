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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Block {
    pub fn text(s: impl Into<String>) -> Self {
        Block::Text { text: s.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message { role: Role::User, content: vec![Block::text(text)] }
    }

    pub fn assistant(content: Vec<Block>) -> Self {
        Message { role: Role::Assistant, content }
    }

    /// Tool results always go back as a single user message — splitting them
    /// across messages teaches the model to stop calling tools in parallel.
    pub fn tool_results(results: Vec<Block>) -> Self {
        Message { role: Role::User, content: results }
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
            other => Err(format!("unknown effort {other:?} (low|medium|high|xhigh|max)")),
        }
    }
}

/// One request to a provider. Stateless — the full history goes every time.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
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
