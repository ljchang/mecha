//! Anthropic Messages API (`POST /v1/messages`).
//!
//! Raw HTTP: there is no official Anthropic SDK for Rust. Shapes follow the
//! documented wire format for the Claude 5 family.
//!
//! Notes that bite if forgotten:
//!   * `temperature` / `top_p` / `top_k` are rejected with a 400 on Opus 5 and
//!     Fable 5. We never send them.
//!   * `budget_tokens` is gone; thinking is `{"type": "adaptive"}`.
//!   * On Opus 5 thinking is ON by default, and `{"type": "disabled"}` is only
//!     accepted at effort `high` or below.
//!   * `stop_reason: "refusal"` arrives as a 200. Check it before reading content.

use crate::config::ProviderConfig;
use crate::message::*;
use crate::provider::{Provider, StreamEvent, StreamSink};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const API_VERSION: &str = "2023-06-01";
pub const DEFAULT_MODEL: &str = "claude-opus-5";

pub struct Anthropic {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    default_model: String,
}

impl Anthropic {
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        // Refused up front rather than silently dropped: someone who pinned the
        // sampler for repeatable evals must not believe it is pinned here.
        if cfg.temperature.is_some() || cfg.seed.is_some() {
            bail!(
                "the Anthropic API rejects `temperature` and has no `seed`; remove them \
                 from this provider's config. Sampling cannot be pinned on this provider"
            );
        }
        let api_key = cfg
            .resolve_api_key()
            .context("no Anthropic credentials found. Set ANTHROPIC_API_KEY, or put api_key_env / api_key in the provider config")?;
        Ok(Self {
            http: reqwest::Client::builder()
                // Long turns are normal at high effort; the per-request cap is
                // generous and streaming keeps the connection alive anyway.
                .timeout(std::time::Duration::from_secs(900))
                .build()?,
            api_key,
            base_url: cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            default_model: cfg.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        })
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Result<Value> {
        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": req.messages.iter().map(encode_message).collect::<Vec<_>>(),
        });
        let obj = body.as_object_mut().unwrap();

        if stream {
            obj.insert("stream".into(), json!(true));
        }

        // Render order is tools -> system -> messages, so a cache breakpoint on
        // the last system block covers the tool definitions too.
        if !req.tools.is_empty() {
            let last = req.tools.len() - 1;
            let tools: Vec<Value> = req
                .tools
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut v = json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    });
                    // Only lands here when there's no system prompt to carry it.
                    if req.cache_prompt && req.system.is_none() && i == last {
                        v.as_object_mut()
                            .unwrap()
                            .insert("cache_control".into(), json!({"type": "ephemeral"}));
                    }
                    v
                })
                .collect();
            obj.insert("tools".into(), json!(tools));
        }

        if let Some(system) = &req.system {
            let mut block = json!({"type": "text", "text": system});
            if req.cache_prompt {
                block
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".into(), json!({"type": "ephemeral"}));
            }
            obj.insert("system".into(), json!([block]));
        }

        if req.thinking {
            obj.insert(
                "thinking".into(),
                json!({"type": "adaptive", "display": "summarized"}),
            );
        } else {
            // Disabling thinking above `high` effort is a 400. Fail here with a
            // message that says what to do rather than surfacing the API error.
            if matches!(req.effort, Some(Effort::XHigh) | Some(Effort::Max)) {
                bail!(
                    "thinking cannot be disabled at effort {}: lower effort to `high` or leave thinking on",
                    req.effort.unwrap().as_str()
                );
            }
            obj.insert("thinking".into(), json!({"type": "disabled"}));
        }

        if let Some(effort) = req.effort {
            obj.insert("output_config".into(), json!({"effort": effort.as_str()}));
        }

        Ok(body)
    }

    fn request(&self, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}/v1/messages", self.base_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(body)
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        match sink {
            Some(sink) => self.complete_streaming(req, sink).await,
            None => self.complete_once(req).await,
        }
    }
}

impl Anthropic {
    async fn complete_once(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let body = self.body(req, false)?;
        let resp = self.request(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            bail!("anthropic {}: {}", status, api_error(&text));
        }
        let v: Value = serde_json::from_str(&text).context("malformed response body")?;
        decode_response(&v)
    }

    async fn complete_streaming(
        &self,
        req: &CompletionRequest,
        sink: &StreamSink,
    ) -> Result<CompletionResponse> {
        let body = self.body(req, true)?;
        let resp = self.request(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("anthropic {}: {}", status, api_error(&text));
        }

        let mut acc = StreamAccumulator::default();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk?));
            // SSE frames are separated by a blank line.
            while let Some(idx) = buf.find("\n\n") {
                let frame: String = buf.drain(..idx + 2).collect();
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data.is_empty() {
                        continue;
                    }
                    let event: Value =
                        serde_json::from_str(data).context("malformed SSE data frame")?;
                    acc.push(&event, sink)?;
                }
            }
        }

        acc.finish()
    }
}

/// Pull the useful part out of an error body without assuming its shape.
fn api_error(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.chars().take(500).collect())
}

fn encode_message(m: &Message) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content: Vec<Value> = m.content.iter().filter_map(encode_block).collect();
    json!({"role": role, "content": content})
}

fn encode_block(b: &Block) -> Option<Value> {
    Some(match b {
        Block::Text { text } => json!({"type": "text", "text": text}),
        Block::Thinking { text, signature } => {
            // A thinking block without a signature can't be replayed; the API
            // rejects reconstructed ones. Drop it rather than send it back.
            let sig = signature.as_ref()?;
            json!({"type": "thinking", "thinking": text, "signature": sig})
        }
        Block::ToolUse { id, name, input } => {
            json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        Block::ToolResult { tool_use_id, content, is_error } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
    })
}

fn decode_block(v: &Value) -> Option<Block> {
    match v.get("type")?.as_str()? {
        "text" => Some(Block::Text {
            text: v.get("text")?.as_str().unwrap_or_default().to_string(),
        }),
        "thinking" => Some(Block::Thinking {
            text: v.get("thinking").and_then(Value::as_str).unwrap_or_default().to_string(),
            signature: v.get("signature").and_then(Value::as_str).map(str::to_string),
        }),
        "tool_use" => Some(Block::ToolUse {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            input: v.get("input").cloned().unwrap_or_else(|| json!({})),
        }),
        // Server-side tool traffic and anything else we don't model yet.
        _ => None,
    }
}

fn decode_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("refusal") => StopReason::Refusal,
        Some("pause_turn") => StopReason::PauseTurn,
        _ => StopReason::Other,
    }
}

fn decode_usage(v: Option<&Value>) -> Usage {
    let Some(v) = v else { return Usage::default() };
    let g = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: g("input_tokens"),
        output_tokens: g("output_tokens"),
        cache_creation_input_tokens: g("cache_creation_input_tokens"),
        cache_read_input_tokens: g("cache_read_input_tokens"),
    }
}

fn decode_refusal(v: &Value) -> Option<Refusal> {
    let d = v.get("stop_details")?;
    if d.is_null() {
        return None;
    }
    Some(Refusal {
        category: d.get("category").and_then(Value::as_str).map(str::to_string),
        explanation: d.get("explanation").and_then(Value::as_str).map(str::to_string),
    })
}

fn decode_response(v: &Value) -> Result<CompletionResponse> {
    let content = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("response has no content array"))?
        .iter()
        .filter_map(decode_block)
        .collect();

    Ok(CompletionResponse {
        message: Message::assistant(content),
        stop_reason: decode_stop_reason(v.get("stop_reason").and_then(Value::as_str)),
        usage: decode_usage(v.get("usage")),
        refusal: decode_refusal(v),
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        // Non-streaming responses carry already-parsed JSON, so arguments
        // cannot arrive malformed on this path.
        malformed_tool_args: 0,
    })
}

/// Reassembles a streamed message. Blocks arrive interleaved by index, and
/// tool arguments arrive as partial JSON that only parses once complete.
#[derive(Default)]
struct StreamAccumulator {
    blocks: BTreeMap<usize, PartialBlock>,
    stop_reason: Option<StopReason>,
    usage: Usage,
    refusal: Option<Refusal>,
    model: String,
}

enum PartialBlock {
    Text(String),
    Thinking { text: String, signature: Option<String> },
    ToolUse { id: String, name: String, json: String },
    Ignored,
}

impl StreamAccumulator {
    fn push(&mut self, event: &Value, sink: &StreamSink) -> Result<()> {
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                if let Some(m) = event.get("message") {
                    self.model = m.get("model").and_then(Value::as_str).unwrap_or_default().to_string();
                    self.usage.add(&decode_usage(m.get("usage")));
                    // Input tokens — including both cache tiers — are known
                    // from this first frame. Report them now so a run cancelled
                    // mid-stream still knows what the prompt cost.
                    let _ = sink.send(StreamEvent::Usage(self.usage.clone()));
                }
            }
            "content_block_start" => {
                let idx = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let cb = event.get("content_block").cloned().unwrap_or(Value::Null);
                let partial = match cb.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => PartialBlock::Text(
                        cb.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
                    ),
                    "thinking" => PartialBlock::Thinking {
                        text: cb.get("thinking").and_then(Value::as_str).unwrap_or_default().to_string(),
                        signature: None,
                    },
                    "tool_use" => {
                        let name = cb.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                        let _ = sink.send(StreamEvent::ToolUseStart { name: name.clone() });
                        PartialBlock::ToolUse {
                            id: cb.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                            name,
                            json: String::new(),
                        }
                    }
                    _ => PartialBlock::Ignored,
                };
                self.blocks.insert(idx, partial);
            }
            "content_block_delta" => {
                let idx = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let Some(delta) = event.get("delta") else { return Ok(()) };
                let Some(block) = self.blocks.get_mut(&idx) else { return Ok(()) };
                match (delta.get("type").and_then(Value::as_str).unwrap_or(""), block) {
                    ("text_delta", PartialBlock::Text(buf)) => {
                        let t = delta.get("text").and_then(Value::as_str).unwrap_or_default();
                        buf.push_str(t);
                        let _ = sink.send(StreamEvent::TextDelta(t.to_string()));
                    }
                    ("thinking_delta", PartialBlock::Thinking { text, .. }) => {
                        let t = delta.get("thinking").and_then(Value::as_str).unwrap_or_default();
                        text.push_str(t);
                        let _ = sink.send(StreamEvent::ThinkingDelta(t.to_string()));
                    }
                    ("signature_delta", PartialBlock::Thinking { signature, .. }) => {
                        let s = delta.get("signature").and_then(Value::as_str).unwrap_or_default();
                        signature.get_or_insert_with(String::new).push_str(s);
                    }
                    ("input_json_delta", PartialBlock::ToolUse { json, .. }) => {
                        json.push_str(
                            delta.get("partial_json").and_then(Value::as_str).unwrap_or_default(),
                        );
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(d) = event.get("delta") {
                    if let Some(sr) = d.get("stop_reason").and_then(Value::as_str) {
                        self.stop_reason = Some(decode_stop_reason(Some(sr)));
                    }
                    if let Some(r) = decode_refusal(d) {
                        self.refusal = Some(r);
                    }
                }
                self.usage.add(&decode_usage(event.get("usage")));
                let _ = sink.send(StreamEvent::Usage(self.usage.clone()));
            }
            "error" => {
                bail!(
                    "anthropic stream error: {}",
                    event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                );
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<CompletionResponse> {
        let mut content = Vec::new();
        let mut malformed = 0u32;
        for (_, block) in self.blocks {
            match block {
                PartialBlock::Text(text) => {
                    if !text.is_empty() {
                        content.push(Block::Text { text });
                    }
                }
                PartialBlock::Thinking { text, signature } => {
                    content.push(Block::Thinking { text, signature })
                }
                PartialBlock::ToolUse { id, name, json } => {
                    // An empty-argument call streams no partial_json at all.
                    let input = if json.trim().is_empty() {
                        json!({})
                    } else {
                        match serde_json::from_str(&json) {
                            Ok(v) => v,
                            Err(e) => {
                                // Don't kill the turn: hand the model an error
                                // result so it can retry, and count it as the
                                // reliability signal it is.
                                malformed += 1;
                                tracing::warn!(
                                    tool = %name,
                                    error = %e,
                                    "tool arguments did not parse"
                                );
                                json!({"__malformed_arguments": json})
                            }
                        }
                    };
                    content.push(Block::ToolUse { id, name, input });
                }
                PartialBlock::Ignored => {}
            }
        }

        Ok(CompletionResponse {
            message: Message::assistant(content),
            stop_reason: self.stop_reason.unwrap_or(StopReason::Other),
            usage: self.usage,
            refusal: self.refusal,
            model: self.model,
            malformed_tool_args: malformed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Anthropic {
        Anthropic {
            http: reqwest::Client::new(),
            api_key: "test-key".into(),
            base_url: "http://localhost:1".into(),
            default_model: DEFAULT_MODEL.into(),
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            model: DEFAULT_MODEL.into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            max_tokens: 1024,
            effort: None,
            thinking: true,
            cache_prompt: false,
        }
    }

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: "does a thing".into(),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn a_configured_temperature_is_refused_at_construction() {
        let cfg = crate::config::ProviderConfig {
            kind: "anthropic".into(),
            api_key: Some("test-key".into()),
            temperature: Some(0.0),
            ..Default::default()
        };
        let err = match Anthropic::from_config(&cfg) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a pinned temperature must not construct an Anthropic provider"),
        };
        assert!(err.contains("temperature"), "{err}");
    }

    /// Anywhere in the tree, at any depth — a knob smuggled into a nested
    /// object 400s exactly as loudly as one at the top level.
    fn mentions_key(v: &Value, key: &str) -> bool {
        match v {
            Value::Object(map) => {
                map.contains_key(key) || map.values().any(|v| mentions_key(v, key))
            }
            Value::Array(items) => items.iter().any(|v| mentions_key(v, key)),
            _ => false,
        }
    }

    #[test]
    fn the_sampling_knobs_are_never_sent_whatever_the_request_asks_for() {
        // The Claude 5 family rejects these outright, so the guarantee has to
        // hold across every shape of request rather than the common one.
        for thinking in [true, false] {
            for effort in [None, Some(Effort::Low), Some(Effort::High)] {
                for cache_prompt in [true, false] {
                    let r = CompletionRequest {
                        system: Some("be brief".into()),
                        tools: vec![spec("fs_read")],
                        thinking,
                        effort,
                        cache_prompt,
                        ..req()
                    };
                    let body = client().body(&r, false).unwrap();
                    for knob in ["temperature", "top_p", "top_k", "budget_tokens"] {
                        assert!(
                            !mentions_key(&body, knob),
                            "{knob} was sent (thinking={thinking}, effort={effort:?})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_cache_breakpoint_goes_on_the_last_system_block_and_nothing_after_it() {
        let r = CompletionRequest {
            system: Some("you are a harness".into()),
            tools: vec![spec("fs_read"), spec("shell")],
            cache_prompt: true,
            ..req()
        };
        let body = client().body(&r, false).unwrap();

        // Render order is tools -> system -> messages, so one marker on the
        // last system block covers the tool definitions too. Prompt caching is
        // a prefix match: a marker on the tools instead would leave the system
        // prompt outside the cached span.
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["cache_control"], json!({"type": "ephemeral"}));

        for tool in body["tools"].as_array().unwrap() {
            assert!(tool.get("cache_control").is_none(), "a tool carried the breakpoint too");
        }
        assert!(
            !mentions_key(&body["messages"], "cache_control"),
            "the messages are volatile and must stay outside the cached prefix"
        );
    }

    #[test]
    fn with_no_system_prompt_the_breakpoint_falls_to_the_last_tool() {
        let r = CompletionRequest {
            system: None,
            tools: vec![spec("fs_read"), spec("shell")],
            cache_prompt: true,
            ..req()
        };
        let body = client().body(&r, false).unwrap();
        let tools = body["tools"].as_array().unwrap();

        assert!(tools[0].get("cache_control").is_none(), "the breakpoint must be last, not first");
        assert_eq!(tools[1]["cache_control"], json!({"type": "ephemeral"}));
    }

    #[test]
    fn nothing_is_marked_cacheable_unless_it_was_asked_for() {
        let r = CompletionRequest {
            system: Some("you are a harness".into()),
            tools: vec![spec("fs_read")],
            cache_prompt: false,
            ..req()
        };
        assert!(!mentions_key(&client().body(&r, false).unwrap(), "cache_control"));
    }

    #[test]
    fn thinking_is_adaptive_rather_than_a_token_budget() {
        let body = client().body(&CompletionRequest { thinking: true, ..req() }, false).unwrap();
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn disabling_thinking_above_high_effort_is_refused_before_the_request_is_sent() {
        // On Opus 5 the API rejects this combination. Failing here costs a
        // function call; failing there costs a round trip and returns an error
        // that does not say which knob to move.
        for effort in [Effort::XHigh, Effort::Max] {
            let r = CompletionRequest { thinking: false, effort: Some(effort), ..req() };
            let err = client().body(&r, false).unwrap_err().to_string();
            assert!(err.contains(effort.as_str()), "the error should name the effort: {err}");
            assert!(err.contains("high"), "the error should say what to do: {err}");
        }

        // At `high` and below it is accepted, and must actually be sent.
        for effort in [None, Some(Effort::Low), Some(Effort::Medium), Some(Effort::High)] {
            let r = CompletionRequest { thinking: false, effort, ..req() };
            let body = client().body(&r, false).unwrap();
            assert_eq!(body["thinking"], json!({"type": "disabled"}));
        }
    }

    #[test]
    fn a_thinking_block_with_no_signature_is_dropped_rather_than_replayed() {
        // Signatures are opaque and checked. Sending a reconstructed one 400s
        // the *next* turn, which is a confusing place to discover it.
        let dropped = encode_block(&Block::Thinking { text: "reasoning".into(), signature: None });
        assert!(dropped.is_none());

        let kept = encode_block(&Block::Thinking {
            text: "reasoning".into(),
            signature: Some("sig-abc".into()),
        })
        .unwrap();
        assert_eq!(kept["signature"], "sig-abc");
        assert_eq!(kept["thinking"], "reasoning");
    }

    #[test]
    fn tool_results_and_a_steer_ride_in_one_user_message() {
        // There is no legal slot for a user message between a `tool_use` and
        // its result, so steering text has to travel as another block of the
        // message already carrying the results.
        let encoded = encode_message(&Message::tool_results(vec![
            Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "42".into(),
                is_error: false,
            },
            Block::text("actually, focus on X"),
        ]));

        assert_eq!(encoded["role"], "user");
        let content = encoded["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[1]["type"], "text");
    }

    #[test]
    fn a_refusal_arrives_as_an_ordinary_response_and_shows_up_in_the_stop_reason() {
        // HTTP 200. Reading `content` without checking the stop reason gets you
        // an empty string and no idea why.
        let v = json!({
            "content": [],
            "model": DEFAULT_MODEL,
            "stop_reason": "refusal",
            "stop_details": {"category": "policy", "explanation": "declined"},
            "usage": {"input_tokens": 12, "output_tokens": 0},
        });

        let resp = decode_response(&v).unwrap();
        assert_eq!(resp.stop_reason, StopReason::Refusal);
        let refusal = resp.refusal.expect("a refusal must carry its details");
        assert_eq!(refusal.category.as_deref(), Some("policy"));
        assert_eq!(resp.usage.input_tokens, 12);
    }

    #[test]
    fn an_ordinary_response_carries_no_refusal() {
        let v = json!({
            "content": [{"type": "text", "text": "hello"}],
            "model": DEFAULT_MODEL,
            "stop_reason": "end_turn",
            "stop_details": null,
            "usage": {"input_tokens": 5, "output_tokens": 2},
        });

        let resp = decode_response(&v).unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert!(resp.refusal.is_none());
        assert_eq!(resp.message.text(), "hello");
    }
}
