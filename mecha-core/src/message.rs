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
}

impl Block {
    pub fn text(s: impl Into<String>) -> Self {
        Block::Text { text: s.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![Block::text(text)],
        }
    }

    pub fn assistant(content: Vec<Block>) -> Self {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    /// Tool results always go back as a single user message — splitting them
    /// across messages teaches the model to stop calling tools in parallel.
    pub fn tool_results(results: Vec<Block>) -> Self {
        Message {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
