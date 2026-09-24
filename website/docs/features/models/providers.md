---
title: Providers
sidebar_position: 1
description: The Provider trait, the raw-HTTP Anthropic backend, the OpenAI-compatible one, prompt caching, and the retry taxonomy.
---

# Providers

A provider translates a `CompletionRequest` onto some wire protocol and
translates the reply back into a `CompletionResponse`. Everything above that
layer — the agent loop, tools, sessions, compaction — is provider-agnostic. If
you find yourself matching on a provider name inside `agent.rs`, the
abstraction has leaked.

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable identifier used in config and `--provider`.
    fn id(&self) -> &str;

    /// Model used when the caller doesn't name one.
    fn default_model(&self) -> &str;

    /// Whether this provider will actually put an image in front of a model.
    /// Defaults to `false`: a provider added later is text-only until
    /// somebody says otherwise.
    fn vision(&self) -> bool {
        false
    }

    /// Whether this endpoint is configured to enforce a response schema.
    fn structured_output(&self) -> bool {
        false
    }

    /// Whether changing `CompletionRequest::effort` changes the wire request.
    /// Unknown adapters opt out so a measurement cannot compare identical arms.
    fn supports_effort(&self) -> bool {
        false
    }

    /// Run one turn. With `sink`, stream and emit deltas as they arrive; the
    /// accumulated response is still returned.
    async fn complete(
        &self,
        req: &CompletionRequest,
        sink: Option<&StreamSink>,
    ) -> Result<CompletionResponse>;
}
```

Two backends ship: `anthropic` speaks the Messages API over raw HTTP, and
`openai` / `openai-compatible` / `local` all build the same
`/v1/chat/completions` client. Config picks between them with `kind`:

```toml
default_provider = "anthropic"

[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
context_window = 200000               # your model's window; unset means no compaction

[providers.local]                     # llama-server, vLLM, Ollama
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "qwen3-14b"
context_window = 32768
```

## Streaming

`complete` takes an optional `StreamSink`. With one, the provider emits
`StreamEvent`s as they arrive — `TextDelta`, `ThinkingDelta`, `ToolUseStart`,
and `Usage` — while still returning the accumulated response.

`Usage` is emitted *as it arrives*, cumulatively, rather than only at the end,
so a run you cancel mid-turn still reports the tokens it spent — including the
input, usually the expensive half, which is known from the first frame.

## The Anthropic backend

There is no official Anthropic SDK for Rust, so `provider/anthropic.rs` speaks
raw HTTP against `POST /v1/messages` with `anthropic-version: 2023-06-01`. Four
things will 400 if forgotten.

### No sampler parameters

`temperature`, `top_p` and `top_k` are **rejected** on the Claude 5 family. The
backend never sends them, and it refuses a config that names them rather than
dropping them silently:

```
the Anthropic API rejects `temperature` and has no `seed`; remove them from
this provider's config. Sampling cannot be pinned on this provider
```

Refused up front on purpose: someone who pinned the sampler for repeatable
evals must not go on believing it is pinned.

### Thinking is `adaptive`, not a token budget

`budget_tokens` is gone. Thinking is:

```json
{"thinking": {"type": "adaptive", "display": "summarized"}}
```

On Opus 5 thinking is **on by default**, and `{"type": "disabled"}` is only
accepted at effort `high` or below. `Anthropic::body` checks this itself and
fails with a message that says what to do:

```
thinking cannot be disabled at effort xhigh: lower effort to `high` or leave
thinking on
```

Effort rides separately, as `output_config: {"effort": "..."}`.

### Refusals arrive as HTTP 200

`stop_reason: "refusal"` is a successful response. Always check the stop reason
before reading content. The backend decodes it into `StopReason::Refusal` and
pulls `stop_details` into a `Refusal { category, explanation }`, and `mecha run`
exits `2` on it.

### Unsigned thinking blocks cannot be replayed

A `Block::Thinking` with no signature is dropped when re-encoding rather than
sent back — the API rejects reconstructed ones.

Model ids are exact strings with no date suffix: `claude-opus-5`.

## Prompt caching

Caching is a prefix match, and the render order is **tools → system →
messages**. Two breakpoints follow from that.

**A static breakpoint on the last system block** covers the tool definitions
too, because they render ahead of it. When there is no system prompt at all,
the marker falls onto the last tool definition instead.

**A second, moving breakpoint on the last message block.** The transcript is
append-only between turns, so each request is a strict prefix of the next: this
turn's cache write is next turn's cache read, which is exactly the trade the
write premium is for. Without it the whole message history is re-sent uncached
every turn.

The moving breakpoint is **never placed on a thinking block** — the API rejects
the marker there. The search walks the message content backwards and skips
thinking blocks to find a legal home.

```json
{"type": "text", "text": "...", "cache_control": {"type": "ephemeral"}}
```

Verified live: a two-request tool round-trip paid 8 uncached input tokens
total, with turn 1's write (18,494 tokens) read back in full by turn 2. Where
it has been measured, cache operations are around 80% of billed agent cost.

Two consequences elsewhere. The tool registry is a `BTreeMap` so tool order is
stable — the list is the very front of the prefix, and reordering it
invalidates the cache every turn. And switching phase (planning hides the
writing tools) sends a shorter tool list, which changes the front of the prefix
and makes the next turn re-pay for it; that is the price of the tools being
genuinely absent rather than merely refused.

`cache_prompt` is on by default and can be turned off in `[agent]`.

## The OpenAI-compatible backend

One implementation covers OpenAI itself, llama.cpp's `llama-server`, vLLM,
Ollama's compatibility endpoint, and anything else speaking the dialect. Point
`base_url` at whichever you are running; the API key is optional, because local
servers usually do not check it.

The shape is lossier than Anthropic's in places — no cache breakpoints, no
effort; those fields are accepted and ignored. `temperature` and `seed` *are*
sent here when configured, which is what makes a pinned local replay possible
at all.

**Reasoning is carried, in both directions.** A reasoning model puts its
thinking on its own channel, and this backend decodes `reasoning_content` into
a thinking block and streams it as a thinking delta — so reasoning is visible
live in the TUI and kept in the transcript, exactly as on the Anthropic path.
It then rides back to the provider on the next request. That is self-gating by
construction: a thinking block exists on this path only because a server sent
the field, so it returns only to servers that speak it and never to an endpoint
that would reject an unknown one. No provider name is tested anywhere.

It is never treated as *output*. A turn that only reasoned is nudged and
continued, not accepted as an answer.

Carrying it back matters: a history stripped of reasoning shows the model
turn after turn of itself calling tools without thinking, and a local model
imitates that into turns that come back empty.

**An empty turn leaves a record in the log, and only there.** When a response
produces no output but carried reasoning, the backend logs a warning with how
much reasoning there was, whether it looks like a tool call, the finish reason
and its tail; `MECHA_LOG=debug` adds the whole trace. Such a turn is nudged
and continued before it reaches any transcript, so if you are chasing silent
turns, keep the stderr.

**And a run that ends having only reasoned hands that reasoning back**,
labelled as deliberation rather than a committed answer, instead of reporting
that the model said nothing.

Message encoding differs structurally: an assistant turn carries its tool calls
inline as `tool_calls`, but every tool result becomes its own `role: "tool"`
message. One of mecha's messages can therefore expand into several.

This backend is also where **malformed tool arguments** are counted. Arguments
arrive as a JSON *string*; when it does not parse, the call is kept with
`{"__malformed_arguments": "..."}` as its input and `malformed_tool_args` is
incremented, so the eval rig can disqualify a model on it rather than the run
dying. (On the Anthropic path arguments arrive already parsed, so
non-streaming responses cannot be malformed.)

## Failure classification and retry

A transient 429, an overload, or a dropped keep-alive connection should not
kill a run — least of all midway through a `batch` or `eval` fan-out — so
`provider/retry.rs` classifies failures into eight classes and applies a policy
per class.

| Class | Trigger | Retried? |
|---|---|---|
| `RateLimit { retry_after }` | 429 | Yes — `Retry-After` honoured when sane |
| `Overloaded` | 529, or a 503 that says so | Yes |
| `ServerError` | any other 5xx | Yes |
| `Transport` | connect failures, timeouts, aborted writes | Yes |
| `Auth` | 401/403 | No |
| `Billing` | 402, or a body naming credit/billing | No |
| `ContextOverflow` | recognised by message text | No |
| `Invalid(detail)` | any other 4xx | No |

The terminal classes are terminal for concrete reasons: the same key fails the
same way every time, a retried 401 is a lockout risk, the same payload fails
identically, and overflow belongs to the loop's compaction path rather than to
a retry that cannot fit any better.

Classification reads **status and text**, and the text can outrank the status.
No backend gives context overflow a usable code — llama-server says
`exceed_context_size_error`, vLLM says "maximum context length", Anthropic says
"prompt is too long" — and llama-server reports overflow as a *500*. Classified
as `ServerError` it would be retried three times with the same payload and
never reach compaction recovery, so the overflow text check sits ahead of the
5xx arm.

Defaults: `max_retries = 3`, base backoff 2.5s doubling to a 30s ceiling,
`retry_after_cap_secs = 60`. A `Retry-After` above the cap is surfaced as a
**failure, not a nap** — sleeping an hour on a header's say-so takes the
process hostage past every budget, and control has to return to a layer that
could fall back.

```toml
[providers.anthropic]
max_retries = 3
retry_after_cap_secs = 60
```

### A retry must never duplicate work

This is the invariant the whole design rests on. Retrying is safe exactly when
nothing of the attempt has been acted on — no tool has run, no text has reached
your screen. So only the send and the status line are retried; once an answer
has started streaming, a failure ends the turn rather than re-asking, and it is
never handed to a [fallback](#fallbacks) either, which would replay half an
answer as a whole one.

## Fallbacks

`Failover` wraps a primary provider and tries each configured fallback in order
when the primary exhausts its retries on a *transient* failure.

```toml
[providers.anthropic]
kind = "anthropic"
model = "claude-opus-5"
fallbacks = ["local"]
```

Two rules carry it:

- **Only transient `ProviderError`s are eligible.** An error with no
  `ProviderError` in its chain is a mid-stream failure, and re-issuing it would
  replay half an answer as a whole one. `Invalid` and `ContextOverflow` fail
  identically everywhere; `Auth` and `Billing` are the primary's problem, not a
  routing decision. Each of these has a test.
- **Each fallback answers as itself.** The request's model name is rewritten to
  the fallback's own default before it is sent. Sending one server's model name
  to another server was a real recorded bug, found through `mecha replay -p`.

Failover is **turn-local**: the next turn starts from the primary again.

`fallbacks` is **empty by default**, and that is a deliberate strictness. A
silent switch to a different model is a worse failure than an error, because
everything downstream — a scorecard, a cost estimate, a judgement about
capability — is now about a model nobody named. `mecha eval` forces
`--no-fallback` regardless of config, like MCP, hooks and the outbox, because a
scorecard grades the model it names. `--no-fallback` is available on any
command.

A fallback that names a provider not in the config, or the provider naming
itself, is a startup error rather than a surprise during an outage.

## Context window and cost

Two provider fields exist because no response carries them.

```toml
[providers.local]
context_window = 32768            # per slot: the server's -c divided by -np

[providers.anthropic]
input_price_per_mtok = 5.0
output_price_per_mtok = 25.0
```

A provider reports what a prompt *cost*, never what is left, so
`context_window` has to be told (`mecha setup` can read it off a local
server's `/props`). Three things derive from it — the compaction threshold
(and with it whether the model is offered the `compact` tool), the per-turn
tool-output budget, and the TUI's fuel gauge — and without it all three
degrade silently: no threshold means no compaction until the server refuses
a request, and overflow recovery, which keys on that refusal rather than on
the window, is left as the only thing that compacts. See
[Compaction](/docs/features/models/compaction), and
[Serving a local model](/docs/features/models/serving) for how `context_window`
relates to the `-c` and `-np` a local server was actually started with.

Prices are required in **both** halves: knowing one is worse than knowing
neither, because it silently under-reports. Leave them unset for a local model
and `cost_usd` reports `null` rather than a misleading zero.
