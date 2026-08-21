//! OpenAI-compatible `/v1/chat/completions`.
//!
//! One implementation covers OpenAI itself, llama.cpp's `llama-server`, vLLM,
//! Ollama's compat endpoint, and anything else speaking the same dialect.
//! Point `base_url` at whichever you're running.
//!
//! The shape is lossier than Anthropic's: no cache breakpoints, no effort.
//! Those fields are accepted and ignored.
//!
//! Reasoning is the exception. Servers that split it out (llama.cpp's
//! `--reasoning-format auto`, vLLM, DeepSeek) return it as
//! `reasoning_content`, which decodes into a `Block::Thinking` so it can be
//! shown and recorded. It is never *output* — see `produced_output`.
//!
//! It is currently one-way, and that is a **known gap, not a decision**.
//! `reasoning_content` is a request field too: measured against llama-server
//! on 2026-08-10 via `/apply-template`, an assistant message carrying it
//! renders back into the prompt as a `<think>` block, and without it the same
//! turn renders as a bare `<tool_call>` with no thinking at all. So every
//! prior assistant turn in a mecha conversation shows this model calling
//! tools without reasoning — which is both a lost prior and the suspected
//! cause of the empty-turn bug, since the malformation reproduced 7/7 was a
//! bare tool call emitted with no think block.
//!
//! `anthropic.rs` already replays thinking (signature-gated, see
//! `encode_block`); this backend has simply never had the code. Fixing it
//! needs care about context cost — reasoning runs to thousands of tokens a
//! turn — and about servers in this dialect that reject the field.

use crate::config::ProviderConfig;
use crate::message::*;
use crate::provider::{Provider, StreamEvent, StreamSink};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct OpenAiCompatible {
    http: reqwest::Client,
    api_key: Option<String>,
    base_url: String,
    default_model: String,
    temperature: Option<f64>,
    seed: Option<u64>,
    id: String,
    vision: bool,
    retry: crate::provider::retry::RetryPolicy,
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
            default_model: cfg
                .model
                .clone()
                .unwrap_or_else(|| "gpt-4o-mini".to_string()),
            temperature: cfg.temperature,
            seed: cfg.seed,
            id: cfg.kind.clone(),
            vision: cfg.vision_enabled(),
            retry: crate::provider::retry::RetryPolicy::from_config(cfg),
        })
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let mut messages = Vec::new();
        if let Some(system) = &req.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        for m in &req.messages {
            encode_message(m, &mut messages, self.vision);
        }

        let mut body = json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "messages": messages,
        });
        let obj = body.as_object_mut().unwrap();
        if let Some(t) = self.temperature {
            obj.insert("temperature".into(), json!(t));
        }
        if let Some(s) = self.seed {
            obj.insert("seed".into(), json!(s));
        }
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

    fn vision(&self) -> bool {
        self.vision
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse> {
        let body = self.body(req, sink.is_some());
        // Retries cover the send and the status line — nothing has streamed
        // yet. Mid-stream failures below propagate without a `ProviderError`,
        // which is what keeps them out of the retry and failover paths:
        // deltas may already be on the user's screen.
        let resp = crate::provider::retry::send_with_retry(|| self.request(&body), &self.retry)
            .await
            .map_err(|f| {
                let message = match f.status {
                    Some(status) => format!(
                        "{} {status}: {}",
                        self.id,
                        f.detail.chars().take(500).collect::<String>()
                    ),
                    None => format!("{}: {}", self.id, f.detail),
                };
                anyhow::Error::new(f.class).context(message)
            })?;

        let Some(sink) = sink else {
            let v: Value = resp.json().await.context("malformed response body")?;
            return decode_response(&v);
        };

        let mut acc = Accumulator::default();
        let mut buf = crate::provider::sse::SseBuffer::default();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            buf.push(&chunk?);
            // Lines are split on bytes, not decoded text: a network chunk can
            // end mid-character, and only a complete line is guaranteed to be
            // complete UTF-8.
            while let Some(line) = buf.next_segment(b"\n") {
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
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
/// Visible to `anthropic.rs`'s test that the two backends word an unseen
/// image identically. Two renderings of the same block that drift apart is
/// exactly the kind of thing no single-backend test can see.
#[cfg(test)]
pub(crate) fn encode_message_for_test(m: &Message, out: &mut Vec<Value>, vision: bool) {
    encode_message(m, out, vision)
}

fn encode_message(m: &Message, out: &mut Vec<Value>, vision: bool) {
    match m.role {
        Role::Assistant => {
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut tool_calls = Vec::new();
            for b in &m.content {
                match b {
                    Block::Text { text: t } => text.push_str(t),
                    // Sent back, because the model's own history is what
                    // teaches it the shape of a turn. Measured 2026-08-10
                    // against llama-server on the prefix that had gone quiet
                    // seven times out of seven: with reasoning stripped from
                    // the history — which is what this backend did — the model
                    // produced a bare `<tool_call>` with no think block, 6 of
                    // 6, and the server misfiled the whole thing as reasoning
                    // so the turn arrived empty. With reasoning restored to
                    // the history and nothing else changed, 0 of 6. The
                    // parser bug upstream is real, but this is the half that
                    // was ours: we showed the model turn after turn of itself
                    // calling tools without thinking, and it obliged.
                    //
                    // Self-gating, which is what keeps it honest: on this path
                    // a Thinking block exists only because a server sent
                    // `reasoning_content` in the first place. So this rides
                    // back only to servers that speak the field — llama.cpp,
                    // vLLM, DeepSeek — and never to an endpoint that would
                    // reject an unknown one. No provider sniffing, no flag.
                    //
                    // `signature` is dropped: it is Anthropic's echo-back
                    // token and has no counterpart here. That asymmetry is
                    // why `anthropic.rs` refuses to replay an unsigned block
                    // while this backend replays every one it has.
                    Block::Thinking { text: t, .. } => reasoning.push_str(t),
                    Block::ToolUse { id, name, input } => tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": input.to_string()},
                    })),
                    // A result is its own message, never part of the turn.
                    Block::ToolResult { .. } => {}
                    // Nothing constructs one on an assistant turn — a model
                    // returns text, thinking and calls, never pixels — and
                    // the type cannot say so. Dropped rather than rendered,
                    // because inventing a place for it here would be
                    // inventing a shape no server accepts.
                    Block::Image { .. } => {}
                }
            }
            let mut msg = json!({"role": "assistant"});
            let obj = msg.as_object_mut().unwrap();
            obj.insert(
                "content".into(),
                if text.is_empty() {
                    Value::Null
                } else {
                    json!(text)
                },
            );
            if !tool_calls.is_empty() {
                obj.insert("tool_calls".into(), json!(tool_calls));
            }
            // Absent rather than empty when there was no thinking: a server
            // that renders the field conditionally must see it missing, not
            // see an empty think block.
            if !reasoning.is_empty() {
                obj.insert("reasoning_content".into(), json!(reasoning));
            }
            out.push(msg);
        }
        Role::User => {
            let mut text = String::new();
            let mut images = Vec::new();
            for b in &m.content {
                match b {
                    Block::Text { text: t } => text.push_str(t),
                    Block::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => out.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content,
                    })),
                    // A model with no eyes is told what it was handed, in
                    // words. That keeps a run against a text-only server
                    // behaving exactly as it did before images existed —
                    // and, more importantly, keeps the *conversation* the
                    // same object across a `/model` switch: the same
                    // transcript renders as pixels to one backend and as a
                    // line of prose to the other, with nothing lost from
                    // the record either way.
                    Block::Image {
                        media_type,
                        data,
                        source,
                    } if vision => images.push(json!({
                        "type": "image_url",
                        "image_url": {"url": format!("data:{media_type};base64,{data}")},
                    })),
                    Block::Image {
                        media_type, source, ..
                    } => {
                        if !text.is_empty() && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str(&Block::image_placeholder(media_type, source.as_deref()));
                    }
                    _ => {}
                }
            }
            // **A parts array only when there is an image**, never as the
            // uniform shape. `{"content": "..."}` is what every request this
            // backend has ever sent looks like, plenty of OpenAI-compatible
            // shims accept nothing else, and a prompt cache is a prefix
            // match over bytes — so changing the encoding of every message
            // in order to support the rare one would invalidate the cached
            // prefix of every run that never sends an image.
            if images.is_empty() {
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            } else {
                let mut parts = Vec::with_capacity(images.len() + 1);
                // Text first. Measured on gemma-4 through llama-server, and
                // it is also what both providers' own documentation shows:
                // the question is what the image is being looked at *for*,
                // and a model that meets the pixels first has to hold them
                // with no idea what to attend to.
                if !text.is_empty() {
                    parts.push(json!({"type": "text", "text": text}));
                }
                parts.append(&mut images);
                out.push(json!({"role": "user", "content": parts}));
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
    let prompt = g("prompt_tokens");

    // The two dialects disagree about what the prompt count contains, and
    // `Usage::total_input` sums all three fields. Anthropic reports
    // `input_tokens` *beside* the cache tiers; OpenAI reports `prompt_tokens`
    // with `cached_tokens` already *inside* it. Carrying the cached half over
    // without subtracting it would report every prompt at nearly twice its
    // size — and the compaction threshold reads exactly that number, so a long
    // run would start summarising itself at half the window it actually had.
    // Saturated because a provider is not to be trusted to keep the subset
    // relation it documents.
    let cached = v
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(prompt);

    // No write tier in this dialect: a local server's prefix cache is filled
    // as a side effect of serving, never billed or reported separately.
    Usage {
        input_tokens: prompt - cached,
        output_tokens: g("completion_tokens"),
        cache_read_input_tokens: cached,
        ..Usage::default()
    }
}

/// Whether a turn produced anything the loop counts as output.
///
/// Deliberately the same definition `agent.rs` decides `produced_nothing` on —
/// `Message::text()` collects only `Block::Text`, and `tool_uses()` only
/// `Block::ToolUse`. Thinking is not output: a model that reasons and says
/// nothing has still said nothing, and the day this disagrees with the loop is
/// the day a reasoning-only turn ends a run with an empty answer instead of
/// being nudged.
fn produced_output(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| match b {
        Block::Text { text } => !text.trim().is_empty(),
        Block::ToolUse { .. } => true,
        // An image is something handed *to* the model, so it is no more
        // output than a tool result is. It also only ever rides on a user
        // turn, so this arm is unreachable in practice and exists to keep
        // the definition honest rather than to be hit.
        Block::Thinking { .. } | Block::ToolResult { .. } | Block::Image { .. } => false,
    })
}

/// The bytes a turn arrived with, when it produced no output at all.
///
/// `llama-server --reasoning-format` defaults to `auto`, which for a thinking
/// model routes the whole `<think>` block into `message.reasoning_content` and
/// leaves `message.content` null. That channel is now decoded into a
/// `Block::Thinking` — visible, recorded, and dropped again by
/// `encode_message` so it is never sent back — but it is still not *output*,
/// so two very different turns arrive at the loop identically, with no text
/// and no calls and `finish_reason: "stop"`:
///
/// - the model reasoned and then genuinely said nothing, and
/// - the model said something, or called a tool, inside the reasoning channel.
///
/// The agent loop answers both with a nudge and a retry. That is correct for
/// the first and pure waste for the second — a re-prefill of the whole
/// transcript to ask for output that was already produced. Measured on the
/// 2026-08-10 Terminal-Bench run: every one of `break-filter-js-from-html`'s
/// ten nudges was followed immediately by a well-formed `shell` call, twice a
/// near-duplicate of one issued a few messages earlier, which is what a lost
/// tool call looks like — and also what a compliant answer to the nudge looks
/// like. The two are not separable without these bytes.
///
/// So this reports them at the point they are dropped, and deliberately does
/// **not** change the decode. Recovering a tool call out of a think block is a
/// behaviour change that wants evidence first, and this is the instrument that
/// produces it.
/// Syntaxes that mean "a tool call was written here", across the model
/// families this backend actually meets.
///
/// A table rather than one string, on purpose. The *phenomenon* is general —
/// a reasoning model writes its action before closing the think block, the
/// server's parser never sees it, and the turn arrives empty — and only the
/// syntax is per-family. Measured on Qwen3.6 (2026-08-10): the reasoning of a
/// reproduced empty turn held a complete `<tool_call><function=shell>…` that
/// was never parsed. Gemma, Llama and DeepSeek would each write that same
/// intent differently, and a matcher that knew only Qwen would report their
/// identical failure as an unexplained silence.
///
/// This is a **hint on a warning, never a decision**: nothing branches on it,
/// so a family missing from this list costs a vaguer log line and nothing
/// more. Extend it when a new one turns up rather than reaching for a regex —
/// the failure mode of a clever matcher is a false positive on a model merely
/// *discussing* tool calls in prose, and this must not become the thing that
/// decides whether a call gets executed.
const TOOL_CALL_MARKERS: &[&str] = &[
    "<tool_call>",           // Qwen, Hermes
    "<function=",            // Qwen's inner form, also emitted bare
    "<|python_tag|>",        // Llama 3.x
    "<｜tool▁call▁begin｜>", // DeepSeek, fullwidth delimiters
    "```tool_code",          // Gemma
    "<function_call>",       // assorted OpenAI-compatible shims
];

#[derive(Debug, PartialEq)]
struct DroppedReasoning<'a> {
    chars: usize,
    /// A model that started calling before closing its think block. When this
    /// is true the empty turn was a lost call, not a silent model.
    looks_like_tool_call: bool,
    /// The end of the reasoning, which is where a call or a concluded answer
    /// would sit. The head is throat-clearing.
    tail: &'a str,
}

/// `None` when there is nothing to explain: the turn produced output, or the
/// reasoning channel was empty too.
fn dropped_reasoning(produced_output: bool, reasoning: &str) -> Option<DroppedReasoning<'_>> {
    if produced_output || reasoning.trim().is_empty() {
        return None;
    }
    // Counted in chars and sliced on a char boundary: reasoning is model prose
    // and slicing it by byte would panic on the first multibyte character.
    let tail = match reasoning.char_indices().rev().nth(400) {
        Some((i, _)) => &reasoning[i..],
        None => reasoning,
    };
    Some(DroppedReasoning {
        chars: reasoning.chars().count(),
        looks_like_tool_call: TOOL_CALL_MARKERS.iter().any(|m| reasoning.contains(m)),
        tail,
    })
}

fn log_dropped_reasoning(produced_output: bool, reasoning: &str, finish: Option<&str>) {
    if let Some(d) = dropped_reasoning(produced_output, reasoning) {
        tracing::warn!(
            reasoning_chars = d.chars,
            looks_like_tool_call = d.looks_like_tool_call,
            finish_reason = finish.unwrap_or("<absent>"),
            tail = d.tail,
            "turn produced no output but the response carried reasoning_content"
        );
        // The whole trace, at debug, because the tail is enough to classify a
        // silence and never enough to explain it. An empty turn is not in the
        // transcript at all — the loop nudges and continues before pushing the
        // message, and the loop holds no session to record it into — so this
        // log is the only durable record that the turn happened or what was in
        // it. `MECHA_LOG=debug` is what the bench adapter already runs with,
        // and it downloads the stderr beside the transcript.
        tracing::debug!(reasoning = reasoning, "the dropped reasoning, in full");
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
    let choice = v.pointer("/choices/0").context("response has no choices")?;
    let msg = choice.get("message").context("choice has no message")?;

    let mut content = Vec::new();
    // Thinking first, as Anthropic orders it, so a transcript reads in the
    // order the model produced it. `signature: None` — that field is
    // Anthropic's opaque echo-back token and this dialect has no equivalent;
    // `encode_message` drops the whole block for this API regardless, so
    // nothing is ever sent back and no context is spent on it.
    let reasoning = msg
        .get("reasoning_content")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !reasoning.is_empty() {
        content.push(Block::Thinking {
            text: reasoning.to_string(),
            signature: None,
        });
    }
    if let Some(text) = msg.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content.push(Block::text(text));
        }
    }
    for call in msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
    {
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
            id: call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input,
            name,
        });
    }

    let finish = choice.get("finish_reason").and_then(Value::as_str);
    log_dropped_reasoning(produced_output(&content), reasoning, finish);

    Ok(CompletionResponse {
        message: Message::assistant(content),
        stop_reason: decode_finish(finish),
        usage: decode_usage(v.get("usage")),
        refusal: None,
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
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
    /// Accumulated for the diagnostic only — never turned into a block. See
    /// `DroppedReasoning`. The streamed dialect carries it as
    /// `delta.reasoning_content`, alongside the `delta.content` this decodes.
    reasoning: String,
}

impl Accumulator {
    fn push(&mut self, v: &Value, sink: &StreamSink) {
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            self.model = m.to_string();
        }
        if let Some(u) = v.get("usage") {
            if !u.is_null() {
                self.usage = decode_usage(Some(u));
                // Reported as it arrives, so cancelling mid-stream does not
                // throw away the count with the dropped future. This dialect
                // usually sends it only in the final chunk (`include_usage`),
                // so there is often nothing to salvage — but when there is, it
                // beats reporting zero.
                let _ = sink.send(StreamEvent::Usage(self.usage.clone()));
            }
        }
        let Some(choice) = v.pointer("/choices/0") else {
            return;
        };
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(decode_finish(Some(f)));
        }
        let Some(delta) = choice.get("delta") else {
            return;
        };

        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            self.text.push_str(t);
            let _ = sink.send(StreamEvent::TextDelta(t.to_string()));
        }
        // `ThinkingDelta`, not `TextDelta`: front-ends render the two
        // differently and an answer is not made of reasoning. This is the
        // event `anthropic.rs` has always emitted and this dialect never did,
        // which is why the reasoning toggle did nothing against a local model.
        if let Some(r) = delta.get("reasoning_content").and_then(Value::as_str) {
            self.reasoning.push_str(r);
            let _ = sink.send(StreamEvent::ThinkingDelta(r.to_string()));
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let entry = self.calls.entry(idx).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                entry.0 = id.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                if entry.1.is_empty() && !name.is_empty() {
                    let _ = sink.send(StreamEvent::ToolUseStart {
                        name: name.to_string(),
                    });
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
        if !self.reasoning.is_empty() {
            content.push(Block::Thinking {
                text: self.reasoning.clone(),
                signature: None,
            });
        }
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
        log_dropped_reasoning(produced_output(&content), &self.reasoning, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(temperature: Option<f64>, seed: Option<u64>) -> OpenAiCompatible {
        OpenAiCompatible::from_config(&ProviderConfig {
            kind: "local".into(),
            temperature,
            seed,
            ..Default::default()
        })
        .unwrap()
    }

    fn plain_req() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: Vec::new(),
            max_tokens: 64,
            effort: None,
            thinking: false,
            cache_prompt: false,
        }
    }

    #[test]
    fn a_pinned_sampler_is_sent_and_an_unpinned_one_is_absent() {
        let body = provider(Some(0.8), Some(42)).body(&plain_req(), false);
        assert_eq!(body["temperature"], json!(0.8));
        assert_eq!(body["seed"], json!(42));

        // Absent, not defaulted: unset means "the server's choice", and
        // inventing a value here would misrecord what was measured.
        let body = provider(None, None).body(&plain_req(), false);
        assert!(body.get("temperature").is_none());
        assert!(body.get("seed").is_none());
    }

    fn sink() -> (
        StreamSink,
        tokio::sync::mpsc::UnboundedReceiver<StreamEvent>,
    ) {
        tokio::sync::mpsc::unbounded_channel()
    }

    /// One SSE chunk carrying a delta for choice 0.
    fn chunk(delta: Value) -> Value {
        json!({"choices": [{"index": 0, "delta": delta}]})
    }

    #[test]
    fn a_turn_that_produced_output_reports_no_dropped_reasoning() {
        // The diagnostic is about turns that arrive empty. A turn with output
        // is ordinary, however much it reasoned on the way, and firing here
        // would bury the real signal under every thinking turn in the run.
        assert_eq!(dropped_reasoning(true, "a long think"), None);
    }

    #[test]
    fn thinking_is_not_output_so_a_reasoning_only_turn_still_reports() {
        // The load-bearing one. `decode_response` now turns reasoning into a
        // Block::Thinking, so a reasoning-only turn is no longer block-empty —
        // but it is still output-empty, and must still be both nudged by the
        // loop and reported here. Counting blocks instead of output would
        // silence the instrument at the exact moment it matters.
        let blocks = vec![Block::Thinking {
            text: "thinking".into(),
            signature: None,
        }];
        assert!(!produced_output(&blocks));
        assert!(dropped_reasoning(produced_output(&blocks), "thinking").is_some());
    }

    #[test]
    fn whitespace_only_text_is_not_output_either() {
        // `agent.rs` decides on `text.trim().is_empty()`; disagreeing here
        // would report a turn the loop nudges, or stay silent on one it does.
        assert!(!produced_output(&[Block::text("  \n ")]));
        assert!(produced_output(&[Block::text("an answer")]));
        assert!(produced_output(&[Block::ToolUse {
            id: "t1".into(),
            name: "shell".into(),
            input: json!({}),
        }]));
    }

    #[test]
    fn an_empty_turn_with_an_empty_reasoning_channel_reports_nothing() {
        // Nothing arrived anywhere: the model really did say nothing, and
        // there are no dropped bytes to show. Whitespace counts as empty, or
        // a lone newline would be reported as if it were evidence.
        assert_eq!(dropped_reasoning(false, ""), None);
        assert_eq!(dropped_reasoning(false, "  \n "), None);
    }

    #[test]
    fn an_empty_turn_carrying_a_tool_call_in_its_reasoning_is_named_as_one() {
        // The whole question the instrument exists to answer: was the empty
        // turn a silent model, or a call we failed to see?
        let d = dropped_reasoning(
            false,
            "let me check the file\n<tool_call>\n{\"name\": \"shell\"}",
        )
        .expect("an empty turn with reasoning is reportable");
        assert!(
            d.looks_like_tool_call,
            "a <tool_call> in the think block is the lost-call signature"
        );
    }

    #[test]
    fn reasoning_survives_the_round_trip_and_rides_back_with_the_turn() {
        // The whole fix, end to end: what the server sends as
        // `reasoning_content` must come back as `reasoning_content`, beside
        // the call it belongs to. Stripping it is what produced a history of
        // bare tool calls with no thinking, and with it the model reproduced
        // that shape 6 times in 6 — see `encode_message`.
        let v = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "I should list the directory first.",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "shell", "arguments": "{\"command\": \"ls\"}"},
                    }],
                },
            }],
            "model": "qwen3.6-35b-a3b",
        });
        let decoded = decode_response(&v).unwrap();

        let mut out = Vec::new();
        encode_message(&decoded.message, &mut out, false);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0]["reasoning_content"], "I should list the directory first.",
            "the reasoning was dropped on the way back out"
        );
        assert_eq!(out[0]["tool_calls"][0]["function"]["name"], "shell");
        assert_eq!(
            out[0]["content"],
            Value::Null,
            "reasoning must not leak into content"
        );
    }

    #[test]
    fn cached_prompt_tokens_are_split_out_rather_than_counted_twice() {
        // The load-bearing one. `total_input` sums the three fields and the
        // compaction threshold reads it, so carrying `cached_tokens` over
        // without removing it from `prompt_tokens` would report a 1,000-token
        // prompt as 1,800 and compact at little over half the real window.
        let u = decode_usage(Some(&json!({
            "prompt_tokens": 1000,
            "completion_tokens": 42,
            "prompt_tokens_details": {"cached_tokens": 800},
        })));
        assert_eq!(u.input_tokens, 200, "the uncached remainder");
        assert_eq!(u.cache_read_input_tokens, 800);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(
            u.total_input(),
            1000,
            "the reported prompt size must survive the split unchanged"
        );
    }

    #[test]
    fn a_server_that_reports_no_cache_detail_is_unchanged() {
        // Most of this dialect's servers say nothing about caching. They must
        // keep reporting exactly what they did before this field was read.
        let u = decode_usage(Some(&json!({"prompt_tokens": 500, "completion_tokens": 7})));
        assert_eq!(u.input_tokens, 500);
        assert_eq!(u.cache_read_input_tokens, 0);
        assert_eq!(u.total_input(), 500);
    }

    #[test]
    fn a_cached_count_larger_than_the_prompt_cannot_underflow() {
        // Subset by documentation, not by guarantee — and `input_tokens` is
        // unsigned, so believing a bad provider would panic in release-mode
        // wrapping or produce an astronomical prompt size.
        let u = decode_usage(Some(&json!({
            "prompt_tokens": 100,
            "completion_tokens": 1,
            "prompt_tokens_details": {"cached_tokens": 9999},
        })));
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 100);
        assert_eq!(u.total_input(), 100);
    }

    #[test]
    fn a_turn_with_no_thinking_sends_no_reasoning_field() {
        // Absent, not empty. A server rendering the field conditionally must
        // see it missing rather than see an empty think block — and this is
        // also what keeps the round trip self-gating: an endpoint that never
        // sends `reasoning_content` never receives one.
        let mut out = Vec::new();
        encode_message(
            &Message::assistant(vec![Block::text("done")]),
            &mut out,
            false,
        );
        assert!(
            out[0].get("reasoning_content").is_none(),
            "an unrelated endpoint must not be sent a field it never spoke"
        );
    }

    #[test]
    fn the_lost_call_signature_is_not_only_qwens() {
        // The failure is a property of reasoning models, not of one vendor,
        // and this backend serves every OpenAI-compatible server there is.
        // Recognising only the family that happened to be on the bench would
        // report an identical Gemma or Llama failure as an unexplained
        // silence — which is how a diagnostic quietly stops working when the
        // model changes.
        for (family, reasoning) in [
            (
                "gemma",
                "let me check\n```tool_code\nprint(shell(...))\n```",
            ),
            (
                "llama",
                "first I will look\n<|python_tag|>{\"name\": \"shell\"}",
            ),
            ("deepseek", "checking\n<｜tool▁call▁begin｜>shell"),
            ("hermes", "<function_call>{\"name\": \"shell\"}"),
        ] {
            let d = dropped_reasoning(false, reasoning)
                .unwrap_or_else(|| panic!("{family}: reportable"));
            assert!(d.looks_like_tool_call, "{family} went unrecognised");
        }
    }

    #[test]
    fn reasoning_without_a_call_is_reported_but_not_labelled_a_call() {
        let d = dropped_reasoning(false, "I think the answer is 42, so I am done.")
            .expect("an empty turn with reasoning is reportable");
        assert!(!d.looks_like_tool_call);
        assert_eq!(d.chars, 39);
    }

    #[test]
    fn the_tail_is_kept_and_multibyte_reasoning_does_not_panic() {
        // The end is where a call or a concluded answer sits, so the tail is
        // the part worth keeping. Slicing model prose by byte offset would
        // panic on the first multibyte character — and reasoning is exactly
        // where an em dash or a CJK identifier turns up.
        let long = format!("{}—the answer is 42", "x".repeat(5_000));
        let d = dropped_reasoning(false, &long).expect("reportable");
        assert_eq!(d.chars, 5_017);
        assert!(d.tail.ends_with("—the answer is 42"));
        assert!(
            d.tail.chars().count() <= 401,
            "the tail is bounded, not the whole think block"
        );

        // Short reasoning is kept whole rather than sliced to nothing.
        let d = dropped_reasoning(false, "早い").expect("reportable");
        assert_eq!(d.tail, "早い");
    }

    #[test]
    fn llama_servers_empty_turn_shape_decodes_to_no_blocks_and_is_reported() {
        // The exact wire shape behind the 2026-08-07 and 2026-08-10 empty
        // turns: `--reasoning-format` defaults to `auto`, which puts the think
        // block in `reasoning_content` and leaves `content` null, with
        // `finish_reason: "stop"` — NOT "length", so this is not truncation
        // and no budget raise addresses it.
        let v = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "reasoning_content": "I should read the file first.\n<tool_call>",
                },
            }],
            "model": "qwen3.6-35b-a3b",
        });
        let resp = decode_response(&v).unwrap();

        assert_eq!(resp.stop_reason, StopReason::EndTurn);

        // The reasoning is now kept, as a Thinking block — visible in the TUI
        // and recorded in the transcript, where before it was discarded on the
        // floor.
        assert_eq!(
            resp.message.content.len(),
            1,
            "reasoning_content should survive decoding as a Thinking block"
        );
        assert!(matches!(resp.message.content[0], Block::Thinking { .. }));

        // But it is NOT an answer. `text()` is what the loop reads, and it must
        // stay empty or a reasoning-only turn ends the run with nothing said.
        assert_eq!(
            resp.message.text(),
            "",
            "reasoning_content must never silently become the answer"
        );
        assert!(resp.message.tool_uses().is_empty());
        assert!(!produced_output(&resp.message.content));

        // Still the shape the diagnostic exists for, and still reported.
        let d = dropped_reasoning(
            produced_output(&resp.message.content),
            "I should read the file first.\n<tool_call>",
        )
        .expect("this is the shape the diagnostic exists for");
        assert!(d.looks_like_tool_call);
    }

    #[test]
    fn streamed_reasoning_arrives_as_thinking_and_never_as_answer_text() {
        // Same rule on the streaming path, which had the same blind spot. The
        // reasoning reaches the front-end as ThinkingDelta — the event
        // anthropic.rs has always sent and this dialect never did — and still
        // never counts as the answer.
        let (tx, mut rx) = sink();
        let mut acc = Accumulator::default();
        acc.push(&chunk(json!({"reasoning_content": "thinking "})), &tx);
        acc.push(&chunk(json!({"reasoning_content": "hard"})), &tx);
        acc.push(
            &json!({"choices": [{"index": 0, "finish_reason": "stop", "delta": {}}]}),
            &tx,
        );

        // Asserted before `finish` consumes it, and asserted at all because
        // the rest of this test is about absence: without this, deleting the
        // accumulation entirely would leave every assertion below still true.
        assert_eq!(
            acc.reasoning, "thinking hard",
            "deltas must accumulate, or the diagnostic has nothing to report"
        );

        let resp = acc.finish();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(
            resp.message.text(),
            "",
            "a reasoning-only stream produced no answer"
        );
        assert!(
            !produced_output(&resp.message.content),
            "and must still be nudged rather than ending the run"
        );

        // The deltas went out as thinking, never as text.
        rx.close();
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        let thinking: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ThinkingDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thinking, vec!["thinking ", "hard"]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta(_))),
            "reasoning must not be emitted as a TextDelta"
        );
    }

    fn call_delta(index: u64, id: Option<&str>, name: Option<&str>, args: &str) -> Value {
        let mut function = serde_json::Map::new();
        if let Some(name) = name {
            function.insert("name".into(), json!(name));
        }
        function.insert("arguments".into(), json!(args));

        let mut call = serde_json::Map::new();
        call.insert("index".into(), json!(index));
        if let Some(id) = id {
            call.insert("id".into(), json!(id));
        }
        call.insert("type".into(), json!("function"));
        call.insert("function".into(), Value::Object(function));

        chunk(json!({"tool_calls": [Value::Object(call)]}))
    }

    #[test]
    fn tool_call_arguments_split_across_chunks_reassemble_into_one_object() {
        // Arguments arrive as partial JSON that only parses once complete, and
        // the name can be split too. Every fragment has to land in the same
        // slot or the call is silently corrupted rather than loudly broken.
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();

        acc.push(&call_delta(0, Some("call_1"), Some("fs_"), ""), &tx);
        acc.push(&call_delta(0, None, Some("read"), "{\"pa"), &tx);
        acc.push(&call_delta(0, None, None, "th\": \"notes/"), &tx);
        acc.push(&call_delta(0, None, None, "a.md\"}"), &tx);

        let resp = acc.finish();
        let calls = resp.message.tool_uses();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "call_1");
        assert_eq!(calls[0].1, "fs_read");
        assert_eq!(calls[0].2, &json!({"path": "notes/a.md"}));
        assert_eq!(resp.malformed_tool_args, 0);
    }

    #[test]
    fn parallel_tool_calls_are_kept_apart_by_their_index() {
        // Interleaved on purpose: the index is the only thing tying a fragment
        // to its call, and merging two calls produces arguments that parse.
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();

        acc.push(
            &call_delta(0, Some("call_a"), Some("fs_read"), "{\"path\":"),
            &tx,
        );
        acc.push(
            &call_delta(1, Some("call_b"), Some("shell"), "{\"cmd\":"),
            &tx,
        );
        acc.push(&call_delta(0, None, None, " \"a.md\"}"), &tx);
        acc.push(&call_delta(1, None, None, " \"ls\"}"), &tx);

        let resp = acc.finish();
        let calls = resp.message.tool_uses();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "fs_read");
        assert_eq!(calls[0].2, &json!({"path": "a.md"}));
        assert_eq!(calls[1].1, "shell");
        assert_eq!(calls[1].2, &json!({"cmd": "ls"}));
    }

    #[test]
    fn a_call_with_no_arguments_becomes_an_empty_object_not_a_parse_failure() {
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();
        acc.push(&call_delta(0, Some("call_1"), Some("todo_read"), ""), &tx);

        let resp = acc.finish();
        assert_eq!(resp.message.tool_uses()[0].2, &json!({}));
        assert_eq!(resp.malformed_tool_args, 0);
    }

    #[test]
    fn malformed_arguments_are_counted_and_handed_back_rather_than_killing_the_turn() {
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();
        acc.push(
            &call_delta(0, Some("call_1"), Some("fs_read"), "{\"path\": "),
            &tx,
        );

        let resp = acc.finish();

        // The model gets an error result it can retry from, and the count is
        // the reliability signal worth comparing models on.
        assert_eq!(resp.malformed_tool_args, 1);
        assert!(resp.message.tool_uses()[0]
            .2
            .get("__malformed_arguments")
            .is_some());
    }

    #[test]
    fn tool_calls_are_still_decoded_when_the_server_says_the_turn_merely_stopped() {
        // llama-server reports `finish_reason: "stop"` alongside tool_calls.
        // The loop re-classifies any turn containing tool_use blocks, but that
        // only works if the blocks survive decoding in the first place.
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();
        acc.push(
            &call_delta(0, Some("call_1"), Some("fs_read"), "{\"path\": \"a.md\"}"),
            &tx,
        );
        acc.push(
            &json!({"choices": [{"index": 0, "finish_reason": "stop", "delta": {}}]}),
            &tx,
        );

        let resp = acc.finish();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(
            resp.message.tool_uses().len(),
            1,
            "the calls were dropped with the label"
        );

        // And the same on the non-streaming path.
        let v = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "fs_read", "arguments": "{\"path\": \"a.md\"}"},
                    }],
                },
            }],
            "model": "local",
        });
        let resp = decode_response(&v).unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert_eq!(resp.message.tool_uses().len(), 1);
    }

    #[test]
    fn text_and_tool_calls_in_one_turn_both_survive() {
        let (tx, _rx) = sink();
        let mut acc = Accumulator::default();
        acc.push(&chunk(json!({"content": "let me look. "})), &tx);
        acc.push(&call_delta(0, Some("call_1"), Some("fs_read"), "{}"), &tx);
        acc.push(&chunk(json!({"content": "one moment."})), &tx);

        let resp = acc.finish();
        assert_eq!(resp.message.text(), "let me look. one moment.");
        assert_eq!(resp.message.tool_uses().len(), 1);
    }

    #[test]
    fn tool_results_become_their_own_messages_and_a_steer_follows_them() {
        // The OpenAI half of the steering placement: results are `role: "tool"`
        // messages, and the queued text trails them as a `role: "user"` — never
        // ahead, which would put a user message between the assistant's call
        // and its result.
        let mut out = Vec::new();
        encode_message(
            &Message::tool_results(vec![
                Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "42".into(),
                    is_error: false,
                },
                Block::ToolResult {
                    tool_use_id: "t2".into(),
                    content: "7".into(),
                    is_error: false,
                },
                Block::text("actually, focus on X"),
            ]),
            &mut out,
            false,
        );

        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["role"], "tool");
        assert_eq!(out[0]["tool_call_id"], "t1");
        assert_eq!(out[1]["role"], "tool");
        assert_eq!(out[1]["tool_call_id"], "t2");
        assert_eq!(out[2]["role"], "user");
        assert_eq!(out[2]["content"], "actually, focus on X");
    }

    #[test]
    fn an_image_rides_as_a_parts_array_only_when_the_model_can_see() {
        let msg = Message {
            role: Role::User,
            content: vec![
                Block::text("what is this?"),
                Block::image("image/png", b"\x89PNG-ish", Some("shot.png".into())),
            ],
        };

        let mut seeing = Vec::new();
        encode_message(&msg, &mut seeing, true);
        assert_eq!(seeing.len(), 1);
        let parts = seeing[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        // Text first: a model that meets the pixels before the question has
        // nothing to attend to.
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "what is this?");
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(
            url.starts_with("data:image/png;base64,"),
            "the `data:` prefix is this dialect's, added at the wire: {url}"
        );

        // The same conversation against a text-only server. This is the arm
        // that has to keep working: it is what every local model here did
        // before a projector was loaded, and what they do again the moment
        // one is not.
        let mut blind = Vec::new();
        encode_message(&msg, &mut blind, false);
        assert_eq!(blind.len(), 1);
        let content = blind[0]["content"].as_str().expect("a plain string");
        assert!(content.contains("what is this?"));
        assert!(
            content.contains("shot.png"),
            "a model that cannot see is still told what it was handed: {content}"
        );
        assert!(
            !content.contains("base64") && !content.contains("PNG-ish"),
            "and never the payload: {content}"
        );
    }

    /// The cached prefix is a byte-prefix match, so the encoding of a message
    /// with no image must not change at all. Verified to fail on the obvious
    /// wrong implementation — making the parts array the uniform shape.
    #[test]
    fn a_message_with_no_image_is_encoded_exactly_as_it_always_was() {
        let msg = Message::user("ordinary text");
        for vision in [true, false] {
            let mut out = Vec::new();
            encode_message(&msg, &mut out, vision);
            assert_eq!(out.len(), 1);
            assert_eq!(
                out[0]["content"],
                json!("ordinary text"),
                "a bare string, never a one-element parts array (vision={vision})"
            );
        }
    }

    #[test]
    fn an_assistant_turn_carries_its_tool_calls_inline_with_arguments_as_a_string() {
        let mut out = Vec::new();
        encode_message(
            &Message::assistant(vec![Block::ToolUse {
                id: "call_1".into(),
                name: "fs_read".into(),
                input: json!({"path": "a.md"}),
            }]),
            &mut out,
            false,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "assistant");
        assert_eq!(out[0]["content"], Value::Null);
        // A JSON *string*, not an object — sending the object is a 400 here.
        let args = out[0]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(args).unwrap(),
            json!({"path": "a.md"})
        );
    }
}
