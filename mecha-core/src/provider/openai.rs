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
            retry: crate::provider::retry::RetryPolicy::from_config(cfg),
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
            out.push(msg);
        }
        Role::User => {
            let mut text = String::new();
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

/// The bytes a turn arrived with, when it decoded to nothing at all.
///
/// `llama-server --reasoning-format` defaults to `auto`, which for a thinking
/// model routes the whole `<think>` block into `message.reasoning_content` and
/// leaves `message.content` null. This decoder reads `content` and
/// `tool_calls` and nothing else, so two very different turns arrive here
/// identically — no blocks, `finish_reason: "stop"`:
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
#[derive(Debug, PartialEq)]
struct DroppedReasoning<'a> {
    chars: usize,
    /// Qwen emits `<tool_call>` inside the think block when it starts calling
    /// before closing the tag. If this is ever true, the empty turn was a lost
    /// call and not a silent model.
    looks_like_tool_call: bool,
    /// The end of the reasoning, which is where a call or a concluded answer
    /// would sit. The head is throat-clearing.
    tail: &'a str,
}

/// `None` when there is nothing to explain: the turn produced blocks, or the
/// reasoning channel was empty too.
fn dropped_reasoning(decoded_blocks: usize, reasoning: &str) -> Option<DroppedReasoning<'_>> {
    if decoded_blocks > 0 || reasoning.trim().is_empty() {
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
        looks_like_tool_call: reasoning.contains("<tool_call>")
            || reasoning.contains("</tool_call>"),
        tail,
    })
}

fn log_dropped_reasoning(decoded_blocks: usize, reasoning: &str, finish: Option<&str>) {
    if let Some(d) = dropped_reasoning(decoded_blocks, reasoning) {
        tracing::warn!(
            reasoning_chars = d.chars,
            looks_like_tool_call = d.looks_like_tool_call,
            finish_reason = finish.unwrap_or("<absent>"),
            tail = d.tail,
            "turn decoded to no blocks but the response carried reasoning_content; \
             these are the bytes being dropped"
        );
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
    log_dropped_reasoning(
        content.len(),
        msg.get("reasoning_content")
            .and_then(Value::as_str)
            .unwrap_or(""),
        finish,
    );

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
        // Not sent to the sink: this is not the answer, and surfacing it would
        // put a model's private reasoning on the user's screen. It is kept
        // only so `finish` can say what a nothing-turn actually contained.
        if let Some(r) = delta.get("reasoning_content").and_then(Value::as_str) {
            self.reasoning.push_str(r);
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
        log_dropped_reasoning(content.len(), &self.reasoning, None);
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
    fn a_turn_that_decoded_something_reports_no_dropped_reasoning() {
        // The diagnostic is about turns that arrive empty. A turn with blocks
        // is ordinary, however much it reasoned on the way, and firing here
        // would bury the real signal under every thinking turn in the run.
        assert_eq!(dropped_reasoning(1, "a long think"), None);
    }

    #[test]
    fn an_empty_turn_with_an_empty_reasoning_channel_reports_nothing() {
        // Nothing arrived anywhere: the model really did say nothing, and
        // there are no dropped bytes to show. Whitespace counts as empty, or
        // a lone newline would be reported as if it were evidence.
        assert_eq!(dropped_reasoning(0, ""), None);
        assert_eq!(dropped_reasoning(0, "  \n "), None);
    }

    #[test]
    fn an_empty_turn_carrying_a_tool_call_in_its_reasoning_is_named_as_one() {
        // The whole question the instrument exists to answer: was the empty
        // turn a silent model, or a call we failed to see?
        let d = dropped_reasoning(
            0,
            "let me check the file\n<tool_call>\n{\"name\": \"shell\"}",
        )
        .expect("an empty turn with reasoning is reportable");
        assert!(
            d.looks_like_tool_call,
            "a <tool_call> in the think block is the lost-call signature"
        );
    }

    #[test]
    fn reasoning_without_a_call_is_reported_but_not_labelled_a_call() {
        let d = dropped_reasoning(0, "I think the answer is 42, so I am done.")
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
        let d = dropped_reasoning(0, &long).expect("reportable");
        assert_eq!(d.chars, 5_017);
        assert!(d.tail.ends_with("—the answer is 42"));
        assert!(
            d.tail.chars().count() <= 401,
            "the tail is bounded, not the whole think block"
        );

        // Short reasoning is kept whole rather than sliced to nothing.
        let d = dropped_reasoning(0, "早い").expect("reportable");
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

        // Unchanged behaviour: the turn is still empty and still EndTurn. The
        // patch is an instrument, not a fix — recovering the call is a
        // behaviour change that wants this evidence first.
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        assert!(
            resp.message.content.is_empty(),
            "reasoning_content must not silently become an answer"
        );

        let d = dropped_reasoning(
            resp.message.content.len(),
            "I should read the file first.\n<tool_call>",
        )
        .expect("this is the shape the diagnostic exists for");
        assert!(d.looks_like_tool_call);
    }

    #[test]
    fn streamed_reasoning_deltas_never_become_answer_text() {
        // Same rule on the streaming path, which had the same blind spot: the
        // reasoning is accumulated for the diagnostic and must not leak into
        // the message or onto the user's screen.
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
        assert!(
            resp.message.content.is_empty(),
            "a reasoning-only stream is still an empty turn"
        );
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        rx.close();
        assert!(
            rx.try_recv().is_err(),
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
    fn an_assistant_turn_carries_its_tool_calls_inline_with_arguments_as_a_string() {
        let mut out = Vec::new();
        encode_message(
            &Message::assistant(vec![Block::ToolUse {
                id: "call_1".into(),
                name: "fs_read".into(),
                input: json!({"path": "a.md"}),
            }]),
            &mut out,
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
