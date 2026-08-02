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
                        serde_json::from_str(&json).with_context(|| {
                            format!("tool {name} streamed unparseable arguments: {json}")
                        })?
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
        })
    }
}
