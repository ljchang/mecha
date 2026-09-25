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
    http: crate::provider::HttpClients,
    api_key: String,
    base_url: String,
    default_model: String,
    vision: bool,
    structured_output: bool,
    retry: crate::provider::retry::RetryPolicy,
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
        anyhow::ensure!(
            cfg.structured_output != crate::config::StructuredOutput::LlamaJson,
            "llama_json is not an Anthropic structured-output format"
        );
        let api_key = cfg
            .resolve_api_key()
            .context("no Anthropic credentials found. Set ANTHROPIC_API_KEY, or put api_key_env / api_key in the provider config")?;
        Ok(Self {
            // A stall fails a stream; a long answer does not. A non-streaming
            // request keeps the whole-exchange cap. Two clients, because the
            // per-read bound is a client setting — see `provider::HttpClients`.
            http: crate::provider::HttpClients::build()?,
            api_key,
            base_url: cfg
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            default_model: cfg
                .model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            vision: cfg.vision_enabled(),
            structured_output: cfg.structured_output == crate::config::StructuredOutput::JsonSchema,
            retry: crate::provider::retry::RetryPolicy::from_config(cfg),
        })
    }

    /// Send with the retry policy; on giving up, keep this provider's error
    /// shape with the classification underneath for policy layers to match.
    async fn send(&self, body: &Value) -> Result<reqwest::Response> {
        crate::provider::retry::send_with_retry(|| self.request(body), &self.retry)
            .await
            .map_err(|f| {
                let message = match f.status {
                    Some(status) => format!("anthropic {status}: {}", api_error(&f.detail)),
                    None => format!("anthropic: {}", f.detail),
                };
                anyhow::Error::new(f.class).context(message)
            })
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Result<Value> {
        let mut messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| encode_message(m, self.vision))
            .collect();

        // A second, *moving* breakpoint on the last content block. The static
        // one below pins tools+system; without this one the entire message
        // history is re-sent uncached on every turn, and where it has been
        // measured cache operations are ~80% of billed agent cost. The
        // transcript is append-only between turns, so each request is a strict
        // prefix of the next: this turn's cache write is next turn's cache
        // read, which is exactly the trade the write premium is for. Never on
        // a thinking block — the API rejects the marker there.
        if req.cache_prompt {
            let last_block = messages.iter_mut().rev().find_map(|m| {
                m.get_mut("content")?
                    .as_array_mut()?
                    .iter_mut()
                    .rev()
                    .find(|b| b.get("type").and_then(Value::as_str) != Some("thinking"))
            });
            if let Some(block) = last_block {
                block
                    .as_object_mut()
                    .unwrap()
                    .insert("cache_control".into(), json!({"type": "ephemeral"}));
            }
        }

        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": messages,
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

        if let Some(schema) = &req.response_schema {
            anyhow::ensure!(self.structured_output, "provider does not support structured output; configure structured_output = json_schema after verifying endpoint support");
            obj.entry("output_config").or_insert_with(|| json!({}))["format"] =
                json!({"type":"json_schema", "schema":schema});
        }
        Ok(body)
    }

    fn request(&self, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .for_body(body)
            .post(format!(
                "{}/v1/messages",
                self.base_url.trim_end_matches('/')
            ))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(body)
    }
}

#[async_trait]
impl Provider for Anthropic {
    fn supports_effort(&self) -> bool {
        true
    }

    fn structured_output(&self) -> bool {
        self.structured_output
    }
    fn id(&self) -> &str {
        "anthropic"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn vision(&self) -> bool {
        self.vision
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
        let text = self.send(&body).await?.text().await?;
        let v: Value = serde_json::from_str(&text).context("malformed response body")?;
        decode_response(&v)
    }

    async fn complete_streaming(
        &self,
        req: &CompletionRequest,
        sink: &StreamSink,
    ) -> Result<CompletionResponse> {
        let body = self.body(req, true)?;
        // Retries cover the send and the status line — nothing has streamed
        // yet. From here down a failure is mid-stream: deltas may already be
        // on the user's screen, so it propagates as-is, carrying no
        // `ProviderError` — which is also what tells the failover wrapper it
        // must not re-issue the request.
        let resp = self.send(&body).await?;

        let mut acc = StreamAccumulator::default();
        let mut buf = crate::provider::sse::SseBuffer::default();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            buf.push(&chunk?);
            // SSE frames are separated by a blank line. Split on bytes, not
            // decoded text: a network chunk can end mid-character, and only a
            // complete frame is guaranteed to be complete UTF-8.
            while let Some(frame) = buf.next_segment(b"\n\n") {
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
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

        acc.closed_cleanly()?;
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

fn encode_message(m: &Message, vision: bool) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content: Vec<Value> = m
        .content
        .iter()
        .filter_map(|b| encode_block(b, vision))
        .collect();
    json!({"role": role, "content": content})
}

fn encode_block(b: &Block, vision: bool) -> Option<Value> {
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
        Block::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
        // `data` is bare base64 in the type, because this is the dialect
        // that wants it bare — the `data:` prefix belongs to the other one.
        Block::Image {
            media_type, data, ..
        } if vision => json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data},
        }),
        // Same degrade as the OpenAI backend, and it has to be the same
        // words: a conversation carried across a `/model` switch would
        // otherwise describe its own history differently depending on who
        // was asked, which is two answers to "what was in this turn".
        Block::Image {
            media_type, source, ..
        } => json!({
            "type": "text",
            "text": Block::image_placeholder(media_type, source.as_deref()),
        }),
    })
}

fn decode_block(v: &Value) -> Option<Block> {
    match v.get("type")?.as_str()? {
        "text" => Some(Block::Text {
            text: v.get("text")?.as_str().unwrap_or_default().to_string(),
        }),
        "thinking" => Some(Block::Thinking {
            text: v
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            signature: v
                .get("signature")
                .and_then(Value::as_str)
                .map(str::to_string),
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

/// Overlay a cumulative usage frame onto the running total: each field the
/// frame carries replaces; a field it omits is left alone. See the
/// `message_delta` arm for why this is not `Usage::add`.
fn apply_cumulative_usage(usage: &mut Usage, v: Option<&Value>) {
    let Some(v) = v else { return };
    // Monotone: a cumulative counter never goes down, so the larger of the
    // two is the truth. A compatible gateway that emits `"input_tokens": 0`
    // in its delta would otherwise zero the prompt total that `pressure`,
    // the compaction threshold and the cache lens all read — the same blast
    // radius as the double-count this replaces, in the other direction.
    let set = |key: &str, slot: &mut u64| {
        if let Some(n) = v.get(key).and_then(Value::as_u64) {
            *slot = (*slot).max(n);
        }
    };
    set("input_tokens", &mut usage.input_tokens);
    set("output_tokens", &mut usage.output_tokens);
    set(
        "cache_creation_input_tokens",
        &mut usage.cache_creation_input_tokens,
    );
    set(
        "cache_read_input_tokens",
        &mut usage.cache_read_input_tokens,
    );
}

fn decode_refusal(v: &Value) -> Option<Refusal> {
    let d = v.get("stop_details")?;
    if d.is_null() {
        return None;
    }
    Some(Refusal {
        category: d
            .get("category")
            .and_then(Value::as_str)
            .map(str::to_string),
        explanation: d
            .get("explanation")
            .and_then(Value::as_str)
            .map(str::to_string),
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
    /// `message_stop` arrived: the server closed the envelope itself.
    stopped: bool,
}

impl StreamAccumulator {
    /// Did the stream end the way a finished message ends?
    ///
    /// A finished message carries a `stop_reason` in its `message_delta` and
    /// closes with `message_stop`. A stream that ends with neither is a
    /// dropped connection, and `finish()` would have handed back the partial
    /// text as a plausible whole turn with `StopReason::Other`, into the
    /// transcript, with nothing to say it was cut.
    fn closed_cleanly(&self) -> Result<()> {
        if self.stopped || self.stop_reason.is_some() {
            return Ok(());
        }
        bail!(
            "the stream ended before message_stop with no stop_reason, so the answer may be \
             truncated; the connection was probably dropped"
        )
    }
}

enum PartialBlock {
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    Ignored,
}

impl StreamAccumulator {
    fn push(&mut self, event: &Value, sink: &StreamSink) -> Result<()> {
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                if let Some(m) = event.get("message") {
                    self.model = m
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
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
                        cb.get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    "thinking" => PartialBlock::Thinking {
                        text: cb
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        signature: None,
                    },
                    "tool_use" => {
                        let name = cb
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let _ = sink.send(StreamEvent::ToolUseStart { name: name.clone() });
                        PartialBlock::ToolUse {
                            id: cb
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
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
                let Some(delta) = event.get("delta") else {
                    return Ok(());
                };
                let Some(block) = self.blocks.get_mut(&idx) else {
                    return Ok(());
                };
                match (
                    delta.get("type").and_then(Value::as_str).unwrap_or(""),
                    block,
                ) {
                    ("text_delta", PartialBlock::Text(buf)) => {
                        let t = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        buf.push_str(t);
                        let _ = sink.send(StreamEvent::TextDelta(t.to_string()));
                    }
                    ("thinking_delta", PartialBlock::Thinking { text, .. }) => {
                        let t = delta
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        text.push_str(t);
                        let _ = sink.send(StreamEvent::ThinkingDelta(t.to_string()));
                    }
                    ("signature_delta", PartialBlock::Thinking { signature, .. }) => {
                        let s = delta
                            .get("signature")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        signature.get_or_insert_with(String::new).push_str(s);
                    }
                    ("input_json_delta", PartialBlock::ToolUse { json, .. }) => {
                        json.push_str(
                            delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
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
                // **Cumulative, so it replaces rather than adds.** The
                // `message_delta` frame carries the message's running
                // totals — the same `output_tokens` that `message_start`
                // opened at 1, and on the current API the input and both
                // cache tiers again. Adding it on top of `message_start`
                // over-reported every streamed turn by one output token at
                // best and doubled the prompt at worst, and the prompt total
                // is what `pressure` predicts from, what the compaction
                // threshold compares against, and what the cache lens
                // judges. Only fields the frame actually carries replace;
                // an absent field keeps what `message_start` said.
                apply_cumulative_usage(&mut self.usage, event.get("usage"));
                let _ = sink.send(StreamEvent::Usage(self.usage.clone()));
            }
            "message_stop" => self.stopped = true,
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
        client_at("http://localhost:1")
    }

    /// The wire contract: `message_start` opens the usage (input, both cache
    /// tiers, `output_tokens: 1`) and `message_delta` carries the message's
    /// *cumulative* totals. Adding the two counted the prompt twice and the
    /// first output token twice — and the doubled prompt fed `pressure`, the
    /// compaction threshold and the cache lens.
    #[test]
    fn a_streamed_message_delta_replaces_the_usage_instead_of_adding_to_it() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = StreamAccumulator::default();
        acc.push(
            &serde_json::json!({
                "type": "message_start",
                "message": {
                    "model": "m",
                    "usage": {
                        "input_tokens": 100,
                        "cache_read_input_tokens": 50,
                        "cache_creation_input_tokens": 0,
                        "output_tokens": 1
                    }
                }
            }),
            &tx,
        )
        .unwrap();
        acc.push(
            &serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn"},
                "usage": {
                    "input_tokens": 100,
                    "cache_read_input_tokens": 50,
                    "cache_creation_input_tokens": 0,
                    "output_tokens": 12
                }
            }),
            &tx,
        )
        .unwrap();
        assert_eq!(acc.usage.input_tokens, 100);
        assert_eq!(acc.usage.cache_read_input_tokens, 50);
        assert_eq!(acc.usage.output_tokens, 12);
        assert_eq!(acc.usage.total_input(), 150);

        // A gateway that writes zeros into the delta must not zero the input
        // either: cumulative counters only go up.
        acc.push(
            &serde_json::json!({
                "type": "message_delta",
                "delta": {},
                "usage": {"input_tokens": 0, "cache_read_input_tokens": 0, "output_tokens": 14}
            }),
            &tx,
        )
        .unwrap();
        assert_eq!(acc.usage.input_tokens, 100);
        assert_eq!(acc.usage.cache_read_input_tokens, 50);
        assert_eq!(acc.usage.output_tokens, 14);

        // An older-shaped delta that carries only `output_tokens` must not
        // zero the input that `message_start` reported.
        let mut acc = StreamAccumulator::default();
        acc.push(
            &serde_json::json!({
                "type": "message_start",
                "message": {"model": "m", "usage": {"input_tokens": 100, "output_tokens": 1}}
            }),
            &tx,
        )
        .unwrap();
        acc.push(
            &serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": 12}
            }),
            &tx,
        )
        .unwrap();
        assert_eq!(acc.usage.input_tokens, 100);
        assert_eq!(acc.usage.output_tokens, 12);
    }

    /// A stream that ends without `message_stop` or a `stop_reason` is a
    /// dropped connection, not a finished answer.
    #[test]
    fn a_stream_that_ends_without_message_stop_is_an_error_not_a_turn() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut acc = StreamAccumulator::default();
        acc.push(
            &serde_json::json!({"type": "message_start", "message": {"model": "m", "usage": {}}}),
            &tx,
        )
        .unwrap();
        acc.push(
            &serde_json::json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            &tx,
        )
        .unwrap();
        acc.push(
            &serde_json::json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "half an ans"}}),
            &tx,
        )
        .unwrap();
        assert!(acc.closed_cleanly().is_err());

        // Either closing signal suffices.
        acc.push(
            &serde_json::json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 3}}),
            &tx,
        )
        .unwrap();
        assert!(acc.closed_cleanly().is_ok());
        let mut acc = StreamAccumulator::default();
        acc.push(&serde_json::json!({"type": "message_stop"}), &tx)
            .unwrap();
        assert!(acc.closed_cleanly().is_ok());
    }

    /// Shared with `retry_tests` below.
    pub(super) fn client_at(base_url: &str) -> Anthropic {
        Anthropic {
            http: crate::provider::HttpClients::plain(),
            api_key: "test-key".into(),
            base_url: base_url.into(),
            default_model: DEFAULT_MODEL.into(),
            vision: true,
            structured_output: true,
            // Fast retries: these tests measure counts and outcomes, not
            // wall clock.
            retry: crate::provider::retry::RetryPolicy {
                base_delay: std::time::Duration::from_millis(1),
                ..Default::default()
            },
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            response_schema: None,
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
            assert!(
                tool.get("cache_control").is_none(),
                "a tool carried the breakpoint too"
            );
        }
    }

    #[test]
    fn the_moving_breakpoint_sits_on_the_last_message_block_and_only_there() {
        let r = CompletionRequest {
            system: Some("you are a harness".into()),
            tools: vec![spec("fs_read")],
            cache_prompt: true,
            messages: vec![
                Message::user("first"),
                Message::assistant(vec![Block::text("ok")]),
                Message::user("second"),
            ],
            ..req()
        };
        let body = client().body(&r, false).unwrap();
        let messages = body["messages"].as_array().unwrap();

        // Exactly one marker in the messages, on the very last block: the
        // transcript is append-only between turns, so this turn's request is a
        // prefix of the next and the whole history reads from cache. Two
        // markers total (system + here) stays well under the API's limit of 4.
        assert!(!mentions_key(&messages[0], "cache_control"));
        assert!(!mentions_key(&messages[1], "cache_control"));
        let last = messages[2]["content"].as_array().unwrap();
        assert_eq!(
            last.last().unwrap()["cache_control"],
            json!({"type": "ephemeral"})
        );
    }

    #[test]
    fn the_moving_breakpoint_never_lands_on_a_thinking_block() {
        // The API rejects cache_control on thinking blocks. When one ends the
        // transcript, the marker backs up to the nearest markable block.
        let r = CompletionRequest {
            cache_prompt: true,
            messages: vec![
                Message::user("go"),
                Message::assistant(vec![
                    Block::text("partial answer"),
                    Block::Thinking {
                        text: "hmm".into(),
                        signature: Some("sig".into()),
                    },
                ]),
            ],
            ..req()
        };
        let body = client().body(&r, false).unwrap();
        let blocks = body["messages"].as_array().unwrap()[1]["content"]
            .as_array()
            .unwrap();

        assert_eq!(blocks[1]["type"], "thinking");
        assert!(
            blocks[1].get("cache_control").is_none(),
            "marker on a thinking block 400s"
        );
        assert_eq!(blocks[0]["cache_control"], json!({"type": "ephemeral"}));
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

        assert!(
            tools[0].get("cache_control").is_none(),
            "the breakpoint must be last, not first"
        );
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
        assert!(!mentions_key(
            &client().body(&r, false).unwrap(),
            "cache_control"
        ));
    }

    #[test]
    fn thinking_is_adaptive_rather_than_a_token_budget() {
        let body = client()
            .body(
                &CompletionRequest {
                    thinking: true,
                    ..req()
                },
                false,
            )
            .unwrap();
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn disabling_thinking_above_high_effort_is_refused_before_the_request_is_sent() {
        // On Opus 5 the API rejects this combination. Failing here costs a
        // function call; failing there costs a round trip and returns an error
        // that does not say which knob to move.
        for effort in [Effort::XHigh, Effort::Max] {
            let r = CompletionRequest {
                thinking: false,
                effort: Some(effort),
                ..req()
            };
            let err = client().body(&r, false).unwrap_err().to_string();
            assert!(
                err.contains(effort.as_str()),
                "the error should name the effort: {err}"
            );
            assert!(
                err.contains("high"),
                "the error should say what to do: {err}"
            );
        }

        // At `high` and below it is accepted, and must actually be sent.
        for effort in [
            None,
            Some(Effort::Low),
            Some(Effort::Medium),
            Some(Effort::High),
        ] {
            let r = CompletionRequest {
                thinking: false,
                effort,
                ..req()
            };
            let body = client().body(&r, false).unwrap();
            assert_eq!(body["thinking"], json!({"type": "disabled"}));
        }
    }

    #[test]
    fn a_thinking_block_with_no_signature_is_dropped_rather_than_replayed() {
        // Signatures are opaque and checked. Sending a reconstructed one 400s
        // the *next* turn, which is a confusing place to discover it.
        let dropped = encode_block(
            &Block::Thinking {
                text: "reasoning".into(),
                signature: None,
            },
            true,
        );
        assert!(dropped.is_none());

        let kept = encode_block(
            &Block::Thinking {
                text: "reasoning".into(),
                signature: Some("sig-abc".into()),
            },
            true,
        )
        .unwrap();
        assert_eq!(kept["signature"], "sig-abc");
        assert_eq!(kept["thinking"], "reasoning");
    }

    #[test]
    fn an_image_is_a_base64_source_block_when_seen_and_a_named_line_when_not() {
        let block = Block::image("image/png", b"\x89PNG-ish", Some("shot.png".into()));

        let seen = encode_block(&block, true).unwrap();
        assert_eq!(seen["type"], "image");
        assert_eq!(seen["source"]["type"], "base64");
        assert_eq!(seen["source"]["media_type"], "image/png");
        let data = seen["source"]["data"].as_str().unwrap();
        assert!(
            !data.starts_with("data:"),
            "bare base64 here — the `data:` prefix is the other dialect's: {data}"
        );

        let blind = encode_block(&block, false).unwrap();
        assert_eq!(blind["type"], "text", "degrades to text, never dropped");
        let text = blind["text"].as_str().unwrap();
        assert!(text.contains("shot.png"), "{text}");
        assert!(!text.contains("PNG-ish"), "and never the payload: {text}");
    }

    /// The two backends must describe the same block the same way, or a
    /// conversation carried across a `/model` switch tells two stories about
    /// its own history.
    #[test]
    fn both_backends_name_an_unseen_image_identically() {
        let block = Block::image("image/png", b"x", Some("shot.png".into()));
        let mine = encode_block(&block, false).unwrap();

        let mut theirs = Vec::new();
        crate::provider::openai::encode_message_for_test(
            &Message {
                harness: false,
                planning: None,
                tool_provenance: Default::default(),
                role: Role::User,
                content: vec![block],
            },
            &mut theirs,
            false,
        );
        assert_eq!(
            mine["text"].as_str().unwrap(),
            theirs[0]["content"].as_str().unwrap()
        );
    }

    #[test]
    fn tool_results_and_a_steer_ride_in_one_user_message() {
        // There is no legal slot for a user message between a `tool_use` and
        // its result, so steering text has to travel as another block of the
        // message already carrying the results.
        let encoded = encode_message(
            &Message::tool_results(vec![
                Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "42".into(),
                    is_error: false,
                },
                Block::text("actually, focus on X"),
            ]),
            true,
        );

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

#[cfg(test)]
mod retry_tests {
    use super::tests::client_at;
    use super::*;
    use crate::provider::retry::ProviderError;
    use crate::provider::Provider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn req() -> CompletionRequest {
        CompletionRequest {
            response_schema: None,
            model: DEFAULT_MODEL.into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            max_tokens: 64,
            effort: None,
            thinking: false,
            cache_prompt: false,
        }
    }

    fn ok_body(text: &str) -> String {
        serde_json::json!({
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "model": DEFAULT_MODEL,
        })
        .to_string()
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    type MockResponse = (u16, Vec<(&'static str, String)>, String);

    /// A scripted HTTP server: each connection gets the next canned response
    /// and is closed. Anything past the script gets a 500, which fails the
    /// test through the assertion on the request count.
    async fn mock_http(responses: Vec<MockResponse>) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);

        tokio::spawn(async move {
            let mut responses = responses.into_iter();
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                counter.fetch_add(1, Ordering::SeqCst);

                // Read the request head, then its content-length body, so the
                // client never sees a reset while still writing.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let (head_end, body_len) = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break (buf.len(), 0);
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = find(&buf, b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                let l = l.to_ascii_lowercase();
                                l.strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + body_len {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }

                let (status, headers, body) =
                    responses
                        .next()
                        .unwrap_or((500, Vec::new(), "script exhausted".into()));
                let mut resp = format!(
                    "HTTP/1.1 {status} R\r\ncontent-length: {}\r\nconnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    resp.push_str(&format!("{k}: {v}\r\n"));
                }
                resp.push_str("\r\n");
                resp.push_str(&body);
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}"), count)
    }

    #[tokio::test]
    async fn transient_failures_are_retried_until_the_request_succeeds() {
        let (url, count) = mock_http(vec![
            (
                429,
                vec![("retry-after", "0".into())],
                "rate limited".into(),
            ),
            (
                429,
                vec![("retry-after", "0".into())],
                "rate limited".into(),
            ),
            (200, vec![], ok_body("recovered")),
        ])
        .await;

        let response = client_at(&url).complete(&req(), None).await.unwrap();
        assert_eq!(response.message.text(), "recovered");
        assert_eq!(count.load(Ordering::SeqCst), 3, "two retries, then success");
    }

    #[tokio::test]
    async fn max_retries_zero_disables_retrying() {
        let (url, count) = mock_http(vec![
            (
                429,
                vec![("retry-after", "0".into())],
                "rate limited".into(),
            ),
            (200, vec![], ok_body("never reached")),
        ])
        .await;

        let mut provider = client_at(&url);
        provider.retry.max_retries = 0;
        let err = provider.complete(&req(), None).await.unwrap_err();
        assert!(err.to_string().contains("429"), "{err:#}");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn auth_failures_are_terminal_by_request_count_not_by_elapsed_time() {
        let (url, count) = mock_http(vec![(401, vec![], "invalid x-api-key".into())]).await;

        let err = client_at(&url).complete(&req(), None).await.unwrap_err();
        assert!(err.to_string().contains("401"), "{err:#}");
        assert_eq!(
            err.downcast_ref::<ProviderError>(),
            Some(&ProviderError::Auth),
            "the class rides under the message"
        );
        // The assertion that matters: one request. A retried 401 is a lockout
        // risk, and elapsed-time assertions measure the scheduler.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_retry_after_past_the_cap_is_a_failure_not_a_nap() {
        let (url, count) = mock_http(vec![(
            429,
            vec![("retry-after", "3600".into())],
            "later".into(),
        )])
        .await;

        let err = client_at(&url).complete(&req(), None).await.unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ProviderError>(),
            Some(ProviderError::RateLimit { .. })
        ));
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "an hour-long wait must not be slept"
        );
    }

    #[tokio::test]
    async fn context_overflow_stays_out_of_the_retry_path_and_reaches_compaction() {
        let (url, count) = mock_http(vec![(
            400,
            Vec::new(),
            r#"{"error":{"type":"exceed_context_size_error","message":"too big"}}"#.into(),
        )])
        .await;

        let err = client_at(&url).complete(&req(), None).await.unwrap_err();
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "overflow retried with the same payload"
        );
        // The loop's compact-and-retry-once recovery keys on this.
        assert!(crate::agent::is_context_overflow(&err), "{err:#}");
    }

    #[tokio::test]
    async fn a_mid_run_transient_error_never_re_executes_a_tool() {
        use crate::agent::{Agent, Conversation};
        use crate::config::{AgentConfig, PermissionMode};
        use crate::tool::{ModeApprover, Registry, Tool, ToolCtx, ToolOutput};

        // Turn 1 asks for a tool; the request carrying its result dies with a
        // retryable 429; the retry succeeds. The tool must have run exactly
        // once — the retry re-sends the HTTP request, never the turn.
        struct CountingTool(Arc<AtomicUsize>);
        #[async_trait]
        impl Tool for CountingTool {
            fn name(&self) -> &str {
                "echo"
            }
            fn description(&self) -> &str {
                "counts"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            fn read_only(&self) -> bool {
                true
            }
            async fn call(&self, _input: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(ToolOutput::ok("ran"))
            }
        }

        let tool_use_body = serde_json::json!({
            "content": [{"type": "tool_use", "id": "t1", "name": "echo", "input": {}}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1, "output_tokens": 1},
            "model": DEFAULT_MODEL,
        })
        .to_string();
        let (url, requests) = mock_http(vec![
            (200, vec![], tool_use_body),
            (
                429,
                vec![("retry-after", "0".into())],
                "rate limited".into(),
            ),
            (200, vec![], ok_body("done")),
        ])
        .await;

        let executions = Arc::new(AtomicUsize::new(0));
        let mut registry = Registry::new();
        registry.insert(Arc::new(CountingTool(Arc::clone(&executions))));
        let agent = Agent::new(
            Box::new(client_at(&url)),
            registry,
            Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            }),
            ToolCtx {
                workspace: std::env::temp_dir(),
                ..Default::default()
            },
            AgentConfig {
                thinking: false,
                force_final_answer: false,
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let mut convo = Conversation::user("go");
        let outcome = agent.run(&mut convo, None).await.unwrap();

        assert_eq!(outcome.text, "done");
        assert_eq!(
            requests.load(Ordering::SeqCst),
            3,
            "turn 2 was retried at the HTTP layer"
        );
        assert_eq!(
            executions.load(Ordering::SeqCst),
            1,
            "the retry duplicated a tool execution"
        );
    }
}

#[cfg(test)]
mod structured_output_tests {
    use super::*;
    #[test]
    fn schema_keeps_effort_and_quarantine_and_is_not_silently_dropped() {
        let schema =
            json!({"type":"object","properties":{},"required":[],"additionalProperties":false});
        let req = crate::quarantine::QuarantinedPass::new(DEFAULT_MODEL, 100)
            .effort(Some(Effort::High))
            .response_schema(Some(schema.clone()))
            .ask("JSON please");
        let mut cfg = ProviderConfig {
            kind: "anthropic".into(),
            api_key: Some("test".into()),
            structured_output: crate::config::StructuredOutput::JsonSchema,
            ..Default::default()
        };
        let p = Anthropic::from_config(&cfg).unwrap();
        let body = p.body(&req, false).unwrap();
        assert_eq!(body["output_config"]["format"]["schema"], schema);
        assert_eq!(body["output_config"]["effort"], "high");
        assert!(body.get("tools").is_none());
        cfg.structured_output = crate::config::StructuredOutput::Disabled;
        assert!(Anthropic::from_config(&cfg)
            .unwrap()
            .body(&req, false)
            .is_err());
    }
}

#[cfg(test)]
mod planning_metadata_tests {
    use super::*;
    #[test]
    fn planning_sensor_metadata_never_reaches_either_provider() {
        let plain = Message::user("plan result");
        let mut observed = plain.clone();
        let feedback = crate::planning::Feedback {
            steps: Vec::new(),
            decisions: vec![crate::planning::Decision {
                anticipation_evidence: Some(crate::anticipation::Evidence {
                    expected_outcome: Some("PRIVATE OWNER APPRAISAL EVIDENCE".into()),
                    ..crate::anticipation::Evidence::default()
                }),
                anticipation: None,
                goal: None,
                anchor: None,
                open_steps: 1,
                unverified_steps: 0,
                charter_observed: true,
                charter: vec![crate::planning::Gap {
                    goal: Some(crate::goal::GoalRef::Charter("secret-sensor".into())),
                    remaining: Some(0.876543),
                    items_over: Some(86_421),
                    guilt: Some(0.135_792),
                }],
                action: crate::planning::Action::Continue,
                applied: false,
            }],
        };
        observed.planning = Some(feedback);
        observed.harness = true;
        assert_eq!(
            encode_message(&plain, false),
            encode_message(&observed, false)
        );
        let mut a = Vec::new();
        let mut b = Vec::new();
        crate::provider::openai::encode_message_for_test(&plain, &mut a, false);
        crate::provider::openai::encode_message_for_test(&observed, &mut b, false);
        assert_eq!(a, b);
        let record = serde_json::to_string(&observed).unwrap();
        assert!(
            record.contains("secret-sensor"),
            "local recording retains the evidence"
        );
        assert!(record.contains("86421"), "and the per-item count beside it");
        assert!(
            record.contains("0.135792"),
            "and the per-commitment guilt beside that"
        );
        let loaded: Message = serde_json::from_str(&record).unwrap();
        assert_eq!(loaded, observed);
        let mut future = serde_json::to_value(observed).unwrap();
        future["planning"]["decisions"][0]["action"] = serde_json::json!("future_action");
        let loaded: Message = serde_json::from_value(future).unwrap();
        assert_eq!(loaded.text(), plain.text());
        assert!(
            loaded.planning.is_none(),
            "unknown metadata costs no transcript"
        );
    }
}

/// G4 (`docs/APPRAISAL-WIRING-DESIGN.md`), the block-text half: no sensor
/// number, setpoint or numeric valence reaches either encoder's request body.
///
/// `planning_sensor_metadata_never_reaches_either_provider` above proves the
/// *metadata* channel — `Message::planning` is dropped by both encoders. It
/// cannot see a number that arrives as **text**: a status line folded into a
/// user turn, a tool result that prints a reading, a brief that leaks. R21
/// draws the line these tests hold: budget *facts* (turns left, context
/// remaining) may be numbers; a reading against a setpoint, guilt and valence
/// stay harness-side, because a model handed a bounded numeric target drifts
/// into maximising it.
///
/// Two tests, and the split is the point. The run test scans every request a
/// recorded two-run fixture actually sent — through both encoders' whole
/// bodies, system prompt and tool specs included — for every rendering of
/// the values that fixture holds, so it fails on a leak nobody thought to
/// inject. The control test injects each of those renderings as a status
/// line into a tool result and into a user turn and requires the scanner to
/// catch it out of both encoders, so the run test's silence is a finding
/// rather than a scanner that cannot see.
///
/// **The situation brief, since 3a.** With `[agent] situation_brief` on, the
/// brief's *words* (`brief::render`) reach the run's first user turn by
/// design, and with them its board counts and ids — pointers R21 allows.
/// The run test runs twice: off, where every rendering of the brief (its
/// record, its fields, its pointers and counts, its words, its stem) is a
/// leak; and on, where its words and pointers may pass and every sensor
/// number, setpoint, guilt, valence, commitment count or age, and the brief's
/// own instants and seconds still may not. The control catches each of those
/// renderings, the words included, in either slot.
///
/// Scope: the acting run's requests. The quarantined harness brief —
/// `diagnose::Evidence::brief` (guilt and pressure means, behind
/// `[agent] sensors_in_brief`) — hands numbers to a model by design and is
/// not a run request. (The counts-only appraiser's brief was the other; it
/// was retired in row 2a-3, and the text appraisal's inputs carry words.)
#[cfg(test)]
mod numbers_never_reach_the_model_tests {
    use super::tests::client_at;
    use super::*;
    use crate::agent::{Agent, Conversation, RunContext};
    use crate::appraisal::Valence;
    use crate::backlog::{Backlog, Depth};
    use crate::charter::{Charter, Setpoint};
    use crate::config::{AgentConfig, PermissionMode};
    use crate::homeostat::Homeostat;
    use crate::provider::openai::OpenAiCompatible;
    use crate::reading::{CorpusRate, Observed, Reading, Sources};
    use crate::session::{Session, SessionMeta};
    use crate::tool::{ModeApprover, Registry, ToolCtx};
    use chrono::{DateTime, Duration, Utc};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// Setpoints with digits no ordinary request carries, so a hit is a leak
    /// and never a coincidence. Every `SensorKind` is here — `world()`
    /// refuses a charter missing one — and one line has no sensor, which is
    /// the ordinary kind.
    const CHARTER: &str = r#"
[[line]]
id = "replies"
text = "Answer the people who are waiting on the owner."
[line.sensor]
kind = "outbox_age"
setpoint = "13h37m"

[[line]]
id = "queue"
text = "Keep the review queue short enough to read."
[line.sensor]
kind = "outbox_waiting"
setpoint = 27182

[[line]]
id = "questions"
text = "Do not leave the owner's questions parked."
[line.sensor]
kind = "question_latency"
setpoint = "4h19m"

[[line]]
id = "autonomy"
text = "Need the owner less often."
[line.sensor]
kind = "intervention_rate"
setpoint = "0.3141"

[[line]]
id = "strangers"
text = "Close what strangers ask for, one way or the other."
[line.sensor]
kind = "request_closure"
setpoint = "2h41m"

[[line]]
id = "craft"
text = "Leave work better than you found it."
"#;

    const STEER: &str = "Start with the oldest reply.";

    /// The valence one steer signs: a single negative, from the owner. The
    /// run test asserts its live readout equals this, so the control proves
    /// catchable exactly the valence the run test scans for.
    fn steered() -> Valence {
        Valence {
            negative: 1.0,
            negatives: 1,
            ..Valence::default()
        }
    }

    fn now() -> DateTime<Utc> {
        "2030-06-02T08:00:00Z".parse().unwrap()
    }

    struct World {
        charter: Charter,
        homeostat: Homeostat,
        /// The situation brief the run carries (B1, 1h) — recorded on the
        /// run, delivered nowhere — so the scan covers every field of it.
        brief: crate::brief::SituationBrief,
    }

    /// A board big enough that its counts have digits no request carries:
    /// 2,917 open tasks, 2,603 of them overdue, 1,873 held by the agent,
    /// each row carrying a name that must never reach the brief either.
    fn g4_board() -> Value {
        let mut items = Vec::new();
        for i in 0..2_917 {
            let (id, due, overdue) = if i == 0 {
                ("task-g4-own".to_string(), "2030-06-03", false)
            } else if i <= 2_603 {
                (format!("task-g4-late-{i:04}"), "2030-05-01", true)
            } else if i <= 2_620 {
                (format!("task-g4-soon-{i:04}"), "2030-06-04", false)
            } else {
                (format!("task-g4-later-{i:04}"), "2030-09-01", false)
            };
            items.push(json!({
                "id": id,
                "name": format!("Fixture task number {i}"),
                "status": "next",
                "due_at": due,
                "overdue": overdue,
                "project_id": if i == 0 { json!("project-g4-aurora") } else { Value::Null },
                "waiting_on": if i < 1_873 { json!("mecha") } else { Value::Null },
            }));
        }
        json!({"v": 1, "today": "2030-06-02", "truncated": false, "items": items})
    }

    /// The conditions a run starts under, produced by the real producers —
    /// `reading::read_lines` over a backlog, `guilt::read_commitments` over
    /// its items, `Backlog::delta` over a before/after pair — rather than
    /// written in as literals, so the values scanned for are the ones the
    /// loop actually carries.
    fn world() -> World {
        let charter = Charter::parse(CHARTER).unwrap();
        // A kind added later must join the fixture, or its reading is a
        // number nothing here scans for.
        for kind in crate::charter::SensorKind::ALL {
            assert!(
                charter
                    .lines()
                    .iter()
                    .any(|l| l.sensor.as_ref().is_some_and(|s| s.kind == kind)),
                "the fixture charter has no `{}` line",
                kind.wire()
            );
        }
        let now = now();
        let ago = |secs: i64| Some((now - Duration::seconds(secs)).to_rfc3339());
        let before = Backlog {
            outbox: Some(Depth {
                waiting: 31_337,
                oldest: ago(299_580),
                ..Depth::default()
            }),
            questions: Some(Depth {
                waiting: 3,
                oldest: ago(18_181),
                ..Depth::default()
            }),
            frontdoor: Some(Depth {
                waiting: 2,
                oldest: ago(11_111),
                ..Depth::default()
            }),
            ..Backlog::default()
        };
        // The subset a person owes an answer to — all of it, here.
        let requests_on_owner = before.frontdoor.clone();
        let after = Backlog {
            outbox: Some(Depth {
                waiting: 30_011,
                oldest: ago(299_580),
                ..Depth::default()
            }),
            ..before.clone()
        };
        // The items behind those depths, so the per-item readings and the
        // per-run flows (S5) carry numbers of their own to scan for: the
        // outbox's oldest draft beside 31,336 fresh ones — 4,155 of them
        // past the count setpoint — and a run window that cleared 3,794 and
        // staged 2,468, which is the 30,011 `after` holds.
        let waiters = |prefix: &str, n: usize, oldest: i64, fresh: i64| {
            (0..n)
                .map(|i| {
                    let age = if i == 0 { oldest } else { fresh };
                    crate::backlog::Waiter::new(
                        format!("{prefix}{i}"),
                        (now - Duration::seconds(age)).to_rfc3339(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let start_items = crate::backlog::Inventory {
            outbox: Some(waiters("o", 31_337, 299_580, 1_200)),
            questions: Some(waiters("q", 3, 18_181, 60)),
            frontdoor: Some(waiters("r", 2, 11_111, 60)),
            requests_on_owner: Some(waiters("r", 2, 11_111, 60)),
            ..Default::default()
        };
        let mut end_outbox = waiters("o", 31_337 - 3_794, 299_580, 1_200);
        end_outbox.extend(waiters("n", 2_468, 30, 30));
        let end_items = crate::backlog::Inventory {
            outbox: Some(end_outbox),
            ..start_items.clone()
        };
        let mut readings = crate::reading::read_lines(
            &charter,
            &Sources {
                backlog: &before,
                requests_on_owner,
                corpus: CorpusRate::Share(0.4271),
                items: Some(&start_items),
            },
            now,
        );
        crate::reading::with_deltas(&mut readings, &start_items, &end_items);
        // Past the setpoint on every line: an `Unread` or `Nothing` reading
        // carries no value, so a line reading one would scan for nothing.
        for r in &readings {
            assert!(
                matches!(r.reading, Reading::Observed { over: true, .. }),
                "`{}` should read past its setpoint: {:?}",
                r.line,
                r.reading
            );
        }
        let mut delta = Backlog::delta(&before, &after);
        delta.flow = Some(crate::backlog::Inventory::flows(&start_items, &end_items));
        // Every pending commitment with its own guilt (S7): the oldest item
        // in each store is past its line's setpoint, so each store carries
        // a number of its own, and the readout is the largest.
        let commitments = crate::guilt::read_commitments(&start_items, &charter, now);
        for s in &commitments {
            assert!(
                s.any_owed(),
                "{:?} should hold a commitment past its patience",
                s.store
            );
        }
        let homeostat = Homeostat {
            load_avg_1m: Some(13.57),
            mem_available_kb: Some(24_681_357),
            backlog: Some(before),
            backlog_delta: Some(delta),
            anticipated_guilt: crate::guilt::readout(&commitments),
            commitments: Some(commitments),
            charter: Some(readings),
            ..Homeostat::default()
        };
        // The brief, from the real producers where a field has one: the
        // board reduced by `board_of`, the chain by `goal_chain`, the
        // commitments through the 1f accessors, local time and quiet hours
        // by `local_time`, the slots by `slots_of`.
        use crate::brief::*;
        let board = g4_board();
        let anchor = crate::goal::GoalRef::Task("task-g4-own".into());
        let brief = SituationBrief {
            assembled_at: now,
            goal: Some(goal_chain(
                Some(&anchor),
                Ok(&board),
                Ok(&charter),
                Err("no trigger store in this fixture"),
            )),
            board: Some(board_of(Ok(&board), Some("task-g4-own"))),
            commitments: Some(commitments_of(Some(&homeostat))),
            time: Some(local_time(
                now,
                Some("America/Los_Angeles"),
                Ok(Some(crate::workflow::AttentionPolicy {
                    timezone: chrono_tz::America::Los_Angeles,
                    quiet_start: 23,
                    quiet_end: 7,
                    digest_hour: 8,
                })),
            )),
            seats: Some(Seats::Read {
                capacity: 3,
                held: 2,
                holders: vec!["task-g4-seat-holder".into(), "answer g4-parked".into()],
                unreadable: 0,
            }),
            runs: Some(Runs {
                tasks: Flight::Read {
                    others: vec!["task-g4-inflight".into()],
                    unreadable: 0,
                },
                triggers: Flight::Read {
                    others: vec!["g4-nightly-digest".into()],
                    unreadable: 0,
                },
            }),
            slots: Some(slots_of(
                200,
                r#"[{"is_processing":true},{"is_processing":true},{"is_processing":false},{"is_processing":false},{"is_processing":false},{"is_processing":false},{"is_processing":false}]"#,
            )),
            voice: Some(Voice::Idle {
                last_turn_secs: 4_242,
            }),
            budget: Some(Budget {
                max_turns: 200,
                max_output_tokens: Some(48_000),
                max_cost_usd: Some(1.25),
                context_window: Some(65_536),
                compact_at_tokens: Some(52_428),
                context_used_tokens: None,
            }),
        };
        assert!(
            brief.complete(),
            "every brief field should read, so each has values to scan for: {:?}",
            brief.fields()
        );
        World {
            charter,
            homeostat,
            brief,
        }
    }

    /// One value, one rendering of it.
    struct Needle {
        what: &'static str,
        text: String,
    }

    /// Every rendering of every number the world holds that a leak would
    /// plausibly print: the raw value, the repo's own formatters
    /// (`LineReading::summary`, `reading::render_secs`, `Valence::compact`),
    /// the fixed-precision forms a status line would use, and the stored
    /// record's JSON. Anything under four characters is dropped — `0.8`
    /// collides with ordinary text, and every value keeps a longer rendering.
    fn needles(world: &World, valence: &Valence) -> Vec<Needle> {
        fn floats(what: &'static str, v: f32, out: &mut Vec<Needle>) {
            for text in [format!("{v}"), format!("{v:.2}"), format!("{v:.3}")] {
                out.push(Needle { what, text });
            }
        }
        let mut out = Vec::new();
        for line in world.charter.lines() {
            let Some(sensor) = &line.sensor else { continue };
            out.push(Needle {
                what: "setpoint as the owner wrote it",
                text: sensor.setpoint_text.clone(),
            });
            out.push(Needle {
                what: "setpoint in its unit",
                text: match sensor.setpoint {
                    Setpoint::Duration(d) => d.as_secs().to_string(),
                    Setpoint::Count(n) => n.to_string(),
                    Setpoint::Rate(r) => r.to_string(),
                },
            });
        }
        let h = &world.homeostat;
        for r in h.charter.as_deref().unwrap_or_default() {
            out.push(Needle {
                what: "reading summary",
                text: r.summary(),
            });
            if let Reading::Observed { value, excess, .. } = r.reading {
                match value {
                    Observed::Seconds(s) => {
                        out.push(Needle {
                            what: "reading value",
                            text: s.to_string(),
                        });
                        out.push(Needle {
                            what: "reading value",
                            text: crate::reading::render_secs(s),
                        });
                    }
                    Observed::Count(n) => out.push(Needle {
                        what: "reading value",
                        text: n.to_string(),
                    }),
                    Observed::Rate(x) => out.push(Needle {
                        what: "reading value",
                        text: x.to_string(),
                    }),
                }
                floats("reading excess", excess, &mut out);
            }
            // The per-item form and the run's delta (S5): numbers of their
            // own, recorded beside the level and never sent.
            if let Some(items) = &r.items {
                for n in [items.waiting, items.over, items.unknown]
                    .into_iter()
                    .chain(items.oldest_secs)
                {
                    out.push(Needle {
                        what: "per-item reading",
                        text: n.to_string(),
                    });
                }
            }
            if let Some(d) = &r.delta {
                for n in [d.added, d.cleared] {
                    out.push(Needle {
                        what: "per-run delta",
                        text: n.to_string(),
                    });
                }
            }
        }
        for v in [h.anticipated_guilt, h.guilt_after_relief]
            .into_iter()
            .flatten()
        {
            floats("anticipated guilt", v, &mut out);
        }
        // Per-commitment guilt (S7, 1f): each pending commitment's own
        // value, named by R21 as a score a model would move. Only the
        // non-zero ones — a zero renders as `0.00`, which ordinary text
        // carries and which would make a hit a coincidence.
        for g in h
            .commitments
            .as_deref()
            .unwrap_or_default()
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(|i| i.guilt)
            .filter(|g| *g > 0.0)
        {
            floats("per-commitment guilt", g, &mut out);
        }
        if let Some(load) = h.load_avg_1m {
            floats("load average", load, &mut out);
        }
        if let Some(kb) = h.mem_available_kb {
            out.push(Needle {
                what: "memory available",
                text: kb.to_string(),
            });
        }
        if let Some(d) = &h.backlog_delta {
            for n in [d.outbox, d.questions, d.frontdoor].into_iter().flatten() {
                out.push(Needle {
                    what: "backlog delta",
                    text: n.to_string(),
                });
                out.push(Needle {
                    what: "backlog delta",
                    text: n.unsigned_abs().to_string(),
                });
            }
        }
        // The situation brief (B1, 1h): every field's stored JSON and the
        // brief's whole, each pointer it keeps, its counts with digits of
        // their own, the local time it read and the voice reading. Recorded,
        // never sent — a hit is the brief reaching the model before phase 3
        // delivers it. The zone name and weekday are left out on purpose:
        // `date_context` already sends those, and a hit on them would be a
        // coincidence, not a leak.
        let b = &world.brief;
        out.push(Needle {
            what: "situation brief record",
            text: serde_json::to_string(b).unwrap(),
        });
        for field in [
            serde_json::to_string(&b.goal),
            serde_json::to_string(&b.board),
            serde_json::to_string(&b.commitments),
            serde_json::to_string(&b.time),
            serde_json::to_string(&b.seats),
            serde_json::to_string(&b.runs),
            serde_json::to_string(&b.slots),
            serde_json::to_string(&b.voice),
            serde_json::to_string(&b.budget),
        ] {
            out.push(Needle {
                what: "situation brief field",
                text: field.unwrap(),
            });
        }
        if let Some(crate::brief::Board::Read(c)) = &b.board {
            for id in c.overdue_ids.iter().chain(&c.due_soon_ids) {
                out.push(Needle {
                    what: "situation brief pointer",
                    text: id.clone(),
                });
            }
            for n in [c.open, c.overdue, c.waiting_on_agent] {
                out.push(Needle {
                    what: "situation brief count",
                    text: n.to_string(),
                });
            }
        }
        if let Some(crate::brief::GoalChain::Anchored {
            project: crate::brief::Tier::Known { id, .. },
            ..
        }) = &b.goal
        {
            out.push(Needle {
                what: "situation brief pointer",
                text: id.clone(),
            });
        }
        if let Some(crate::brief::Seats::Read { holders, .. }) = &b.seats {
            for h in holders {
                out.push(Needle {
                    what: "situation brief pointer",
                    text: h.clone(),
                });
            }
        }
        if let Some(runs) = &b.runs {
            for f in [&runs.tasks, &runs.triggers] {
                if let crate::brief::Flight::Read { others, .. } = f {
                    for o in others {
                        out.push(Needle {
                            what: "situation brief pointer",
                            text: o.clone(),
                        });
                    }
                }
            }
        }
        if let Some(crate::brief::LocalTime {
            zone: crate::brief::Zone::Set { local, .. },
            ..
        }) = &b.time
        {
            out.push(Needle {
                what: "situation brief local time",
                text: local.clone(),
            });
        }
        if let Some(crate::brief::Voice::Idle { last_turn_secs }) = &b.voice {
            out.push(Needle {
                what: "situation brief voice",
                text: last_turn_secs.to_string(),
            });
        }
        // Since 3a: the brief's words, whole, and the stem they open with —
        // a leak with the delivery lever off, the delivery with it on. And
        // the commitments as the brief records them, which are sensor-side
        // under R21 whether or not the brief is delivered: each store's
        // counts, each item's age in seconds and as `render_secs` prints it.
        out.push(Needle {
            what: "situation brief words",
            text: crate::brief::render(b),
        });
        out.push(Needle {
            what: "situation brief stem",
            text: crate::brief::BRIEF_STEM.to_string(),
        });
        if let Some(crate::brief::Commitments::Read { stores, .. }) = &b.commitments {
            for s in stores {
                for n in [s.waiting.unwrap_or(0), s.owed, s.undated] {
                    out.push(Needle {
                        what: "situation brief commitment",
                        text: n.to_string(),
                    });
                }
                for age in s.items.iter().filter_map(|i| i.age_secs) {
                    out.push(Needle {
                        what: "situation brief commitment",
                        text: age.to_string(),
                    });
                    out.push(Needle {
                        what: "situation brief commitment",
                        text: crate::reading::render_secs(age),
                    });
                }
            }
        }
        out.push(Needle {
            what: "valence",
            text: valence.compact(),
        });
        out.push(Needle {
            what: "valence record",
            text: serde_json::to_string(valence).unwrap(),
        });
        out.retain(|n| n.text.chars().count() >= 4);
        for class in [
            "setpoint as the owner wrote it",
            "setpoint in its unit",
            "reading summary",
            "reading value",
            "reading excess",
            "per-item reading",
            "per-run delta",
            "anticipated guilt",
            "per-commitment guilt",
            "backlog delta",
            "situation brief record",
            "situation brief field",
            "situation brief pointer",
            "situation brief count",
            "situation brief local time",
            "situation brief voice",
            "situation brief words",
            "situation brief stem",
            "situation brief commitment",
            "valence",
        ] {
            assert!(
                out.iter().any(|n| n.what == class),
                "the fixture holds no `{class}` to scan for"
            );
        }
        out
    }

    /// What a run's requests must not carry: every needle, with the brief's
    /// delivery off; with it on, every needle but the brief's words, its
    /// stem, its pointers and its board counts — the parts R21 lets through
    /// (3a). Everything sensor-side stays, the brief's own commitment
    /// numbers, instant and voice seconds among it: the words are bands.
    fn forbidden(needles: Vec<Needle>, delivered: bool) -> Vec<Needle> {
        const DELIVERED: [&str; 4] = [
            "situation brief words",
            "situation brief stem",
            "situation brief pointer",
            "situation brief count",
        ];
        needles
            .into_iter()
            .filter(|n| !delivered || !DELIVERED.contains(&n.what))
            .collect()
    }

    /// The body as bytes, plus every string leaf decoded — a needle carrying
    /// a quote or a non-ASCII minus is escaped in the serialisation but not
    /// in the leaf.
    fn haystack(body: &Value) -> String {
        fn walk(v: &Value, out: &mut String) {
            match v {
                Value::String(s) => {
                    out.push_str(s);
                    out.push('\n');
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
                Value::Object(o) => o.values().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
        let mut out = body.to_string();
        out.push('\n');
        walk(body, &mut out);
        out
    }

    fn leaks<'a>(body: &Value, needles: &'a [Needle]) -> Vec<&'a Needle> {
        let hay = haystack(body);
        needles.iter().filter(|n| hay.contains(&n.text)).collect()
    }

    /// The request through both encoders, whole.
    fn bodies(req: &CompletionRequest) -> [(&'static str, Value); 2] {
        let openai = OpenAiCompatible::from_config(&ProviderConfig {
            kind: "local".into(),
            ..Default::default()
        })
        .unwrap();
        [
            (
                "anthropic",
                client_at("http://127.0.0.1:9").body(req, true).unwrap(),
            ),
            ("openai", openai.body_for_test(req)),
        ]
    }

    fn describe(found: &[&Needle]) -> String {
        found
            .iter()
            .map(|n| format!("{} `{}`", n.what, n.text))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Replays a script and keeps every request the loop built. Queues the
    /// owner's steer once the first request is out, so it folds into the
    /// message carrying the tool results — a real intervention, which is
    /// what gives the fixture run a valence to scan for.
    struct Recorder {
        turns: Mutex<VecDeque<CompletionResponse>>,
        seen: Arc<Mutex<Vec<CompletionRequest>>>,
        steer: Arc<Mutex<VecDeque<String>>>,
    }

    #[async_trait]
    impl Provider for Recorder {
        fn id(&self) -> &str {
            "recorder"
        }
        fn default_model(&self) -> &str {
            "fixture-model"
        }
        async fn complete(
            &self,
            req: &CompletionRequest,
            _sink: Option<&StreamSink>,
        ) -> Result<CompletionResponse> {
            let mut seen = self.seen.lock().unwrap();
            seen.push(req.clone());
            if seen.len() == 1 {
                self.steer.lock().unwrap().push_back(STEER.into());
            }
            self.turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow!("the recorder ran out of scripted turns"))
        }
    }

    fn turn(blocks: Vec<Block>, stop: StopReason) -> CompletionResponse {
        CompletionResponse {
            message: Message::assistant(blocks),
            stop_reason: stop,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Usage::default()
            },
            refusal: None,
            model: "fixture-model".into(),
            malformed_tool_args: 0,
        }
    }

    /// A recorded run, resumed: run one writes a plan serving a sensored
    /// charter line under the fixture's conditions with goal guidance on, is
    /// steered, and is recorded to a session file with its outcome; run two
    /// loads that file and continues. Every request either run sent goes
    /// through both encoders and is scanned for every rendering of every
    /// sensor number, setpoint and valence the fixture holds — and, with the
    /// brief's delivery off (the default), every rendering of the brief.
    #[tokio::test]
    async fn a_recorded_run_carries_no_sensor_number_setpoint_or_valence_to_either_encoder() {
        recorded_run_scan(false).await;
    }

    /// The same run with the brief delivered (3a): its words reach the
    /// first user turn, once, and never the system prompt; its pointers and
    /// board counts may pass; nothing else of the fixture's numbers does —
    /// not a sensor reading, a setpoint, guilt, valence, a commitment's
    /// count or age, the brief's local instant or its voice seconds.
    #[tokio::test]
    async fn a_delivered_brief_carries_words_and_no_sensor_number_to_either_encoder() {
        recorded_run_scan(true).await;
    }

    async fn recorded_run_scan(delivered: bool) {
        let root = crate::mismatch::Workspace::new().unwrap();
        let world = world();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let steer = Arc::new(Mutex::new(VecDeque::new()));
        let plan = json!({
            "items": [
                {"content": "Draft a reply to the oldest waiting message", "status": "in_progress"},
                {"content": "Stage it for review", "status": "pending"}
            ],
            "serves": "charter:replies"
        });
        let recorder = Recorder {
            turns: Mutex::new(VecDeque::from([
                turn(
                    vec![Block::ToolUse {
                        id: "toolu_fixture_plan".into(),
                        name: "todo".into(),
                        input: plan,
                    }],
                    StopReason::ToolUse,
                ),
                turn(
                    vec![Block::Text {
                        text: "Drafted the oldest reply and staged it.".into(),
                    }],
                    StopReason::EndTurn,
                ),
                turn(
                    vec![Block::Text {
                        text: "One question is parked; it needs your answer.".into(),
                    }],
                    StopReason::EndTurn,
                ),
            ])),
            seen: Arc::clone(&seen),
            steer: Arc::clone(&steer),
        };
        let mut registry = Registry::new();
        registry.insert(Arc::new(crate::tool::todo::TodoTool::new()));
        let charter_block = crate::charter::prompt_block(&world.charter).unwrap();
        let tools = ToolCtx {
            workspace: root.path().to_path_buf(),
            ..Default::default()
        };
        let approver = Arc::new(ModeApprover {
            mode: PermissionMode::Allow,
        });
        let agent = Agent::new(
            Box::new(recorder),
            registry,
            approver.clone(),
            tools.clone(),
            AgentConfig {
                system_prompt: Some(format!("You are a personal assistant.\n\n{charter_block}")),
                thinking: false,
                force_final_answer: false,
                goal_guidance: true,
                situation_brief: delivered,
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .with_clock(Arc::new(crate::clock::FixedClock(now())));
        let mut cx = RunContext::new(tools, approver).with_queued_input(Arc::clone(&steer));
        // Everything but the backlog: `Homeostat::finish` re-reads the live
        // backlog to take the run's delta at run end, and a unit test must
        // not read the owner's stores. The delta and the per-commitment guilt
        // are already on the snapshot, from the fixture's own items.
        cx.homeostat = Some(Homeostat {
            backlog: None,
            ..world.homeostat.clone()
        });
        // And the situation brief a front-end assembles before the run: the
        // loop carries it to the record and must build no request from it.
        cx.brief = Some(Arc::new(world.brief.clone()));

        // Run one, recorded exactly as a front-end records it.
        let session = Session::create(
            &root.path().join("sessions"),
            SessionMeta {
                id: "fixture-g4".into(),
                created_at: now(),
                provider: "recorder".into(),
                model: "fixture-model".into(),
                workspace: root.path().into(),
                title: None,
                kind: None,
            },
        )
        .unwrap();
        let mut convo = Conversation::user("Work through the replies that are waiting on me.");
        let before = convo.messages.clone();
        session.append_messages(&before).unwrap();
        let first = agent.run_in(&cx, &mut convo, None).await.unwrap();
        session.record_run(&before, &convo).unwrap();
        session.record_outcome(&first).unwrap();
        let valence = crate::appraisal::live_readout("fixture-g4", &first, &convo, 0).valence;
        // Not silent, and the one the control proves catchable: a readout
        // that drifted (a `partial` flag, a second error) would change the
        // renderings, and this is where that surfaces rather than in a
        // scan nothing showed could see it.
        assert_eq!(
            valence,
            steered(),
            "the steer should sign the run exactly as the control assumes"
        );

        // Run two, from the file.
        let (_, mut resumed) = Session::load(&session.path).unwrap();
        resumed.push(Message::user("And the parked questions?"));
        agent.run_in(&cx, &mut resumed, None).await.unwrap();

        let needles = forbidden(needles(&world, &valence), delivered);
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 3, "two turns in run one, one in run two");

        // Not vacuous, four ways. The numbers are *on disk*: the session
        // file holds the homeostat, readings and guilt this run carried.
        let record = std::fs::read_to_string(&session.path).unwrap();
        let h = &world.homeostat;
        let replies = &h.charter.as_ref().unwrap()[0];
        let Reading::Observed { excess, .. } = replies.reading else {
            panic!("the replies line should read past its setpoint: {replies:?}");
        };
        // The per-commitment guilt the replies line weighs: the outbox's
        // oldest draft, past the line's setpoint.
        let owed = h
            .commitments
            .as_deref()
            .unwrap()
            .iter()
            .find(|s| s.line.as_deref() == Some("replies"))
            .unwrap()
            .max()
            .unwrap();
        assert!(owed > 0.0);
        for kept in [
            format!("{}", h.anticipated_guilt.unwrap()),
            format!("{owed}"),
            format!("{excess}"),
            replies.setpoint.clone(),
            "299580".to_string(),
        ] {
            assert!(record.contains(&kept), "the record should keep `{kept}`");
        }
        // The brief is on the record too, whole: carried by the loop from
        // the context to the outcome, and from there to the run's row.
        let recorded = Session::outcomes_attributed(&session.path)
            .unwrap()
            .into_iter()
            .next()
            .expect("run one's outcome")
            .2
            .brief
            .expect("the run's brief is recorded");
        assert_eq!(*recorded, world.brief);
        assert!(record.contains("task-g4-late-0001"));
        // They are *in the request objects*: the resumed run's request
        // carries the planning metadata whose gap is that excess, and the
        // per-commitment guilt the decision was made on beside it.
        let gaps: Vec<&crate::planning::Gap> = requests[2]
            .messages
            .iter()
            .filter_map(|m| m.planning.as_ref())
            .flat_map(|f| &f.decisions)
            .flat_map(|d| &d.charter)
            .collect();
        assert!(
            gaps.iter().any(|g| g.remaining == Some(excess)),
            "the resumed request should hold the reading as metadata"
        );
        assert!(
            gaps.iter().any(|g| g.guilt == Some(owed)),
            "the resumed request should hold the per-commitment guilt as metadata"
        );
        // The sensor *drove* the run: the decision it made reached the model
        // as fixed words, which is what R21 allows through.
        let guidance = crate::planning::Action::ReviewCommitment.guidance();
        assert!(
            requests[1].messages.iter().flat_map(|m| &m.content).any(
                |b| matches!(b, Block::ToolResult { content, .. } if content.contains(guidance))
            ),
            "the over-setpoint reading should have produced the review guidance"
        );
        // And the charter reached the model as its lines, not its sensors.
        assert!(requests[0]
            .system
            .as_deref()
            .is_some_and(|s| s.contains("`replies`")));

        // Delivered: the words are in the run's first user turn, exactly
        // once in every request (run two resumes a transcript that already
        // states the same situation, so nothing is re-folded), and never in
        // the system prompt. Not delivered: the scan below carries the words
        // and the stem as needles.
        let words = crate::brief::render(&world.brief);
        for (i, req) in requests.iter().enumerate() {
            let briefs: Vec<&str> = req
                .messages
                .iter()
                .filter(|m| m.role == Role::User)
                .flat_map(|m| &m.content)
                .filter_map(|b| match b {
                    Block::Text { text }
                        if text.trim_start().starts_with(crate::brief::BRIEF_STEM) =>
                    {
                        Some(text.trim_start())
                    }
                    _ => None,
                })
                .collect();
            if delivered {
                assert_eq!(briefs, vec![words.as_str()], "request {i}");
                assert!(
                    req.messages[0]
                        .content
                        .iter()
                        .any(|b| matches!(b, Block::Text { text } if text.trim_start() == words)),
                    "request {i}: the brief rides the run's first user turn"
                );
            } else {
                assert!(briefs.is_empty(), "request {i} carried the brief");
            }
            assert!(
                !req.system
                    .as_deref()
                    .unwrap_or_default()
                    .contains(crate::brief::BRIEF_STEM),
                "request {i}: the brief is never the prefix"
            );
        }

        for (i, req) in requests.iter().enumerate() {
            for (encoder, body) in bodies(req) {
                let found = leaks(&body, &needles);
                assert!(
                    found.is_empty(),
                    "request {i} leaked through the {encoder} encoder: {}",
                    describe(&found)
                );
            }
        }
    }

    /// 3a's placement, through both encoders: the same run with the brief's
    /// delivery off and on sends **the same prefix bytes** — tools, system
    /// prompt, and every request field but the messages — and differs only
    /// by one block, the brief's words, appended to the run's own first user
    /// message (no second user message, which is invalid). Append-only too:
    /// every request's messages are a prefix of the next, so the moving
    /// cache breakpoint still reads the whole history from cache.
    #[tokio::test]
    async fn the_brief_rides_the_user_turn_and_the_cached_prefix_is_the_same_bytes_on_and_off() {
        let world = world();
        let root = crate::mismatch::Workspace::new().unwrap();
        let charter_block = crate::charter::prompt_block(&world.charter).unwrap();
        let mut arms: Vec<Vec<CompletionRequest>> = Vec::new();
        for delivered in [false, true] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let recorder = Recorder {
                turns: Mutex::new(VecDeque::from([
                    turn(
                        vec![Block::ToolUse {
                            id: "toolu_fixture_plan".into(),
                            name: "todo".into(),
                            input: json!({"items": [
                                {"content": "Draft the oldest reply", "status": "in_progress"}
                            ]}),
                        }],
                        StopReason::ToolUse,
                    ),
                    turn(vec![Block::text("Drafted.")], StopReason::EndTurn),
                ])),
                seen: Arc::clone(&seen),
                steer: Arc::default(),
            };
            let mut registry = Registry::new();
            registry.insert(Arc::new(crate::tool::todo::TodoTool::new()));
            let tools = ToolCtx {
                workspace: root.path().to_path_buf(),
                ..Default::default()
            };
            let approver = Arc::new(ModeApprover {
                mode: PermissionMode::Allow,
            });
            let agent = Agent::new(
                Box::new(recorder),
                registry,
                approver.clone(),
                tools.clone(),
                AgentConfig {
                    system_prompt: Some(format!(
                        "You are a personal assistant.\n\n{charter_block}"
                    )),
                    thinking: false,
                    force_final_answer: false,
                    situation_brief: delivered,
                    ..Default::default()
                },
                None,
            )
            .unwrap()
            .with_clock(Arc::new(crate::clock::FixedClock(now())));
            let mut cx = RunContext::new(tools, approver);
            cx.brief = Some(Arc::new(world.brief.clone()));
            let mut convo = Conversation::user("Work through the replies that are waiting on me.");
            agent.run_in(&cx, &mut convo, None).await.unwrap();
            arms.push(seen.lock().unwrap().clone());
        }
        let (off, on) = (&arms[0], &arms[1]);
        assert_eq!(off.len(), 2);
        assert_eq!(on.len(), off.len());

        for (i, (a, b)) in off.iter().zip(on).enumerate() {
            for ((encoder, mut a), (_, mut b)) in bodies(a).into_iter().zip(bodies(b)) {
                let ma = a["messages"].take();
                let mb = b["messages"].take();
                // Everything outside the messages — the Anthropic tools and
                // system blocks, the OpenAI tools — is one set of bytes.
                assert_eq!(
                    a.to_string(),
                    b.to_string(),
                    "request {i}: the {encoder} prefix moved with the lever"
                );
                // The OpenAI encoder carries the system prompt as the first
                // message; that is prefix too.
                assert!(a.get("tools").is_some(), "{encoder}: no tools to compare");
                if encoder == "anthropic" {
                    assert!(a.get("system").is_some(), "no system blocks to compare");
                }
                if encoder == "openai" {
                    assert_eq!(ma[0]["role"], "system");
                    assert_eq!(ma[0], mb[0], "request {i}: the system message moved");
                }
                assert!(
                    !a.to_string().contains(crate::brief::BRIEF_STEM),
                    "the brief is never in the prefix"
                );
            }
            // The one difference: the brief, last in the run's first user
            // message, in the same number of messages.
            assert_eq!(a.messages.len(), b.messages.len(), "request {i}");
            let mut expected = a.messages[0].clone();
            expected
                .content
                .push(Block::text(crate::brief::block(&world.brief)));
            assert_eq!(b.messages[0], expected, "request {i}");
            assert_eq!(b.messages[0].role, Role::User);
            assert_eq!(&a.messages[1..], &b.messages[1..], "request {i}");
        }
        for w in on.windows(2) {
            assert_eq!(
                &w[1].messages[..w[0].messages.len()],
                &w[0].messages[..],
                "each request is a prefix of the next"
            );
        }
    }

    /// The control that keeps the run test honest: every rendering the run
    /// test scans for, injected as a status line into a tool result and into
    /// a user turn, is caught out of both encoders — and the same request
    /// with the reading only as `Message::planning` metadata stays clean.
    #[test]
    fn a_status_line_carrying_a_sensor_reading_is_caught_in_a_tool_result_or_a_user_turn() {
        let world = world();
        let needles = needles(&world, &steered());
        let base = |result: String, user: String| -> CompletionRequest {
            CompletionRequest {
                response_schema: None,
                model: "fixture-model".into(),
                system: Some("You are a personal assistant.".into()),
                messages: vec![
                    Message::user(user),
                    Message::assistant(vec![Block::ToolUse {
                        id: "toolu_status".into(),
                        name: "todo".into(),
                        input: json!({"items": []}),
                    }]),
                    Message::tool_results(vec![Block::ToolResult {
                        tool_use_id: "toolu_status".into(),
                        content: result,
                        is_error: false,
                    }]),
                ],
                tools: Vec::new(),
                max_tokens: 1024,
                effort: None,
                thinking: false,
                cache_prompt: false,
            }
        };
        let plain_result = "1. [>] Draft a reply".to_string();
        let plain_user = "Work through the replies.".to_string();

        let clean = base(plain_result.clone(), plain_user.clone());
        for (encoder, body) in bodies(&clean) {
            let found = leaks(&body, &needles);
            assert!(found.is_empty(), "{encoder}: {}", describe(&found));
        }

        // Metadata is not block text: the reading riding on the message is
        // dropped by both encoders, as the metadata test above proves.
        let mut metadata = clean.clone();
        let excess = match world.homeostat.charter.as_ref().unwrap()[0].reading {
            Reading::Observed { excess, .. } => excess,
            ref other => panic!("{other:?}"),
        };
        metadata.messages[2].planning = Some(crate::planning::Feedback {
            steps: Vec::new(),
            decisions: vec![crate::planning::Decision {
                anticipation_evidence: None,
                anticipation: None,
                goal: None,
                anchor: None,
                open_steps: 1,
                unverified_steps: 0,
                charter_observed: true,
                charter: vec![crate::planning::Gap {
                    goal: Some(crate::goal::GoalRef::Charter("replies".into())),
                    remaining: Some(excess),
                    items_over: None,
                    guilt: world.homeostat.anticipated_guilt,
                }],
                action: crate::planning::Action::ReviewCommitment,
                applied: true,
            }],
        });
        for (encoder, body) in bodies(&metadata) {
            let found = leaks(&body, &needles);
            assert!(found.is_empty(), "{encoder}: {}", describe(&found));
        }

        // Block text is: each rendering, as a status line, in each slot.
        for needle in &needles {
            let status = format!("Status: {}", needle.text);
            for (slot, req) in [
                (
                    "tool result",
                    base(format!("{plain_result}\n\n{status}"), plain_user.clone()),
                ),
                (
                    "user turn",
                    base(plain_result.clone(), format!("{plain_user}\n\n{status}")),
                ),
            ] {
                for (encoder, body) in bodies(&req) {
                    assert!(
                        leaks(&body, &needles).iter().any(|n| n.text == needle.text),
                        "a {} `{}` in the {slot} got past the scan of the {encoder} body",
                        needle.what,
                        needle.text
                    );
                }
            }
        }
    }
}
