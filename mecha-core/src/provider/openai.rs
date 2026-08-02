//! OpenAI-compatible `/v1/chat/completions`.
//!
//! One implementation covers OpenAI itself, llama.cpp's `llama-server`, vLLM,
//! Ollama's compat endpoint, and anything else speaking the same dialect.
//! Point `base_url` at whichever you're running.
//!
//! The shape is lossier than Anthropic's: no thinking blocks, no cache
//! breakpoints, no effort. Those fields are accepted and ignored.

use crate::config::ProviderConfig;
use crate::message::*;
use crate::provider::{Provider, StreamEvent, StreamSink};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct OpenAiCompatible {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    default_model: String,
    id: String,
}

impl OpenAiCompatible {
    pub fn from_config(cfg: &ProviderConfig) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(900))
                .build()?,
            // Local servers usually don't check it.
            api_key: cfg.resolve_api_key(),
            base_url: cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string()),
            default_model: cfg.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string()),
            id: cfg.kind.clone(),
        })
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let mut messages = Vec::new();
        if let Some(system) = &req.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        for m in &req.messages {
            encode_message(m, &mut messages);
        }

        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": messages,
        });
        let obj = body.as_object_mut().unwrap();
        if stream {
            obj.insert("stream".into(), json!(true));
            obj.insert("stream_options".into(), json!({"include_usage": true}));
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }})
                })
                .collect();
            obj.insert("tools".into(), json!(tools));
        }
        body
    }

    fn request(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .post(format!(
                "{}/v1/chat/completions",
                self.base_url.trim_end_matches('/')
            ))
            .header("content-type", "application/json");
        if let Some(key) = &self.api_key {
            rb = rb.bearer_auth(key);
        }
        rb.json(body)
    }
}

#[async_trait]
impl Provider for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.id
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let body = self.body(req, sink.is_some());
        let resp = self.request(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("{} {}: {}", self.id, status, text.chars().take(500).collect::<String>());
        }

        let Some(sink) = sink else {
            let v: Value = resp.json().await.context("malformed response body")?;
            return decode_response(&v);
        };

        let mut acc = Accumulator::default();
        let mut buf = String::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(idx) = buf.find('\n') {
                let line: String = buf.drain(..idx + 1).collect();
                let Some(data) = line.trim().strip_prefix("data:") else { continue };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let v: Value = serde_json::from_str(data).context("malformed SSE data frame")?;
                acc.push(&v, sink);
            }
        }
        Ok(acc.finish())
    }
}

/// Expand our block list into however many OpenAI messages it takes: an
/// assistant turn carries its tool calls inline, but every tool result is its
/// own `role: "tool"` message.
fn encode_message(m: &Message, out: &mut Vec<Value>) {
    match m.role {
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            for b in &m.content {
                match b {
                    Block::Text { text: t } => text.push_str(t),
                    Block::ToolUse { id, name, input } => tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": input.to_string()},
                    })),
                    // No wire representation for these on this API.
                    Block::Thinking { .. } | Block::ToolResult { .. } => {}
                }
            }
            let mut msg = json!({"role": "assistant"});
            let obj = msg.as_object_mut().unwrap();
            obj.insert("content".into(), if text.is_empty() { Value::Null } else { json!(text) });
            if !tool_calls.is_empty() {
                obj.insert("tool_calls".into(), json!(tool_calls));
            }
            out.push(msg);
        }
        Role::User => {
            let mut text = String::new();
            for b in &m.content {
                match b {
                    Block::Text { text: t } => text.push_str(t),
                    Block::ToolResult { tool_use_id, content, .. } => out.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    })),
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.push(json!({"role": "user", "content": text}));
            }
        }
    }
}

fn decode_finish(s: Option<&str>) -> StopReason {
    match s {
        Some("stop") => StopReason::EndTurn,
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some("content_filter") => StopReason::Refusal,
        _ => StopReason::Other,
    }
}

fn decode_usage(v: Option<&Value>) -> Usage {
    let Some(v) = v else { return Usage::default() };
    let g = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: g("prompt_tokens"),
        output_tokens: g("completion_tokens"),
        ..Usage::default()
    }
}

fn parse_arguments(name: &str, raw: &str) -> Result<Value> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw)
        .with_context(|| format!("tool {name} returned unparseable arguments: {raw}"))
}

fn decode_response(v: &Value) -> Result<CompletionResponse> {
    let mut malformed = 0u32;
    let choice = v
        .pointer("/choices/0")
        .context("response has no choices")?;
    let msg = choice.get("message").context("choice has no message")?;

    let mut content = Vec::new();
    if let Some(text) = msg.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content.push(Block::text(text));
        }
    }
    for call in msg.get("tool_calls").and_then(Value::as_array).unwrap_or(&vec![]) {
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let raw = call
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or("");
        let input = match parse_arguments(&name, raw) {
            Ok(v) => v,
            Err(_) => {
                malformed += 1;
                json!({"__malformed_arguments": raw})
            }
        };
        content.push(Block::ToolUse {
            id: call.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            input,
            name,
        });
    }

    Ok(CompletionResponse {
        message: Message::assistant(content),
        stop_reason: decode_finish(choice.get("finish_reason").and_then(Value::as_str)),
        usage: decode_usage(v.get("usage")),
        refusal: None,
        model: v.get("model").and_then(Value::as_str).unwrap_or_default().to_string(),
        malformed_tool_args: malformed,
    })
}

#[derive(Default)]
struct Accumulator {
    text: String,
    /// Keyed by the `index` field, which is how deltas identify their call.
    calls: BTreeMap<u64, (String, String, String)>,
    finish: Option<StopReason>,
    usage: Usage,
    model: String,
}

impl Accumulator {
    fn push(&mut self, v: &Value, sink: &StreamSink) {
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            self.model = m.to_string();
        }
        if let Some(u) = v.get("usage") {
            if !u.is_null() {
                self.usage = decode_usage(Some(u));
            }
        }
        let Some(choice) = v.pointer("/choices/0") else { return };
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(decode_finish(Some(f)));
        }
        let Some(delta) = choice.get("delta") else { return };

        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            self.text.push_str(t);
            let _ = sink.send(StreamEvent::TextDelta(t.to_string()));
        }
        for call in delta.get("tool_calls").and_then(Value::as_array).unwrap_or(&vec![]) {
            let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let entry = self.calls.entry(idx).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.0 = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                if entry.1.is_empty() && !name.is_empty() {
                    let _ = sink.send(StreamEvent::ToolUseStart { name: name.to_string() });
                }
                entry.1.push_str(name);
            }
            if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                entry.2.push_str(args);
            }
        }
    }

    fn finish(self) -> CompletionResponse {
        let mut content = Vec::new();
        let mut malformed = 0u32;
        if !self.text.is_empty() {
            content.push(Block::text(self.text));
        }
        for (_, (id, name, args)) in self.calls {
            // A model that streams malformed arguments gets told so via an
            // error tool result rather than killing the whole turn.
            let input = match parse_arguments(&name, &args) {
                Ok(v) => v,
                Err(_) => {
                    malformed += 1;
                    json!({"__malformed_arguments": args})
                }
            };
            content.push(Block::ToolUse { id, name, input });
        }
        CompletionResponse {
            message: Message::assistant(content),
            stop_reason: self.finish.unwrap_or(StopReason::Other),
            usage: self.usage,
            refusal: None,
            model: self.model,
            malformed_tool_args: malformed,
        }
    }
}
