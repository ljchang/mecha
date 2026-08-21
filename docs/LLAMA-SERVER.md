# llama-server: the operational reference

Everything mecha and mecha-graph both depend on about the local inference
server, written down on 2026-08-20 when the two stopped running separate
engines. Most of it was learned by measuring something that had already gone
wrong, so each item carries the measurement rather than the conclusion alone.

The flags themselves live in `scripts/start-moe-mtp.sh`, which is the
authority; this file is the reasoning and the numbers.

## Two servers, one model each

llama-server holds **one model per process**. So:

```
:8080   qwen3.6-35b-a3b   chat/agent    mecha's [providers.local], mecha-graph's extractor
:8081   an embedding model              mecha-graph's embed + retrieval
```

Pointing both at one port silently sends embedding requests to the chat model.
There is a test asserting `DEFAULT_EMBED_URL != llm::DEFAULT_BASE_URL`.

**Why ollama was removed.** It ran its own `llama-server` underneath, so the
choice was never about the engine — only about who sets the flags. What it
cost: a second 29 GB copy of the same 35B model (57.5 GB of a 121 GB pool for
one model), a *different quantisation*, a **chatml** template override
(`--no-jinja`) that made per-request template controls inert, `--context-shift`
silently mangling transcripts, no `--spec-type` speculation, no
`--reasoning-budget`, and `presence_penalty 1.5` inherited from a Modelfile.
Measured cost of the contention: mecha's interactive generation ran at **28
tok/s against a recorded 79.8 baseline** while the graph's nightly ran, and
nothing anywhere reported it.

## Slots and context

**`-c` is the budget across ALL slots, divided evenly.** `-c 262144 -np 4` is
four slots of 65,536, not four of 262,144. Past the real per-slot limit the
server context-shifts instead of erroring, the model sees a mangled transcript,
and the symptom is empty completions that look like a model failure (found
2026-08-05, the empty-EndTurn deaths in the k=5 compaction runs).

The number `[providers.local] context_window` must equal is **`CTX / NP`**, the
per-slot window — not `CTX`. Before `-np` moved off 1 these were the same
number and the distinction cost nothing to ignore.

Confirm it from the startup line rather than from arithmetic:

```
srv load_model: initializing, n_slots = 4, n_ctx_slot = 262144, kv_unified = 'false'
```

`kv_unified = false` (the default whenever `-np` is passed explicitly) means each
slot owns its own KV cells and no slot can evict another's. Do not pass `-kvu`.

## What the KV cache actually costs

The arithmetic that seemed not to work — "41 layers × 2 KV heads × 512 × 2
bytes = 82 KiB/token, 2.6× more than measured" — was right per layer and wrong
about the layer count. **Qwen3.6-35B-A3B is a hybrid model.** The GGUF carries
`qwen35moe.full_attention_interval = 4` and a family of `qwen35moe.ssm.*` keys,
and counting tensors settles it:

```
layers with attn_k (full attention, KV grows with context):  11   (3,7,11…39, and 40)
layers with ssm_*  (gated DeltaNet, constant-size state):    30
```

Only 11 of 41 layers hold a KV cache. Per token, per full-attention layer:
`n_head_kv(2) × (k_len 256 + v_len 256) × bytes`.

| KV type | per token | 262,144 tokens |
|---|---|---|
| **f16** (default) | **22.0 KiB** | 5.5 GB |
| q8_0 | 11.7 KiB | 2.9 GB |
| q4_0 | 6.2 KiB | 1.5 GB |

Plus a constant **~64 MiB per slot** of SSM recurrent state, flat in context
length — so adding a slot is nearly free and adding *context* to a slot is what
costs.

This also explains a number that looked surprising: generation holds **63 tok/s
at 108k context against 92 at 1k**, where a dense 35B would fall much harder.
Only 11 layers do O(n) attention work per token; the other 30 are O(1)
recurrent updates. **Long context is cheap on this model in a way it is not on
a pure-attention one.**

KV stays at f16 deliberately. q8_0 would halve the growing part, but the memory
is available and the property being protected is exact long-context retrieval —
a needle at 21% depth in a 188,546-token prompt was retrieved exactly. An
embedder's or a retriever's quantisation error has no sampling step to absorb
it.

## Measured: what slots buy

Re-measured 2026-08-20 on a quiet machine, **262,144 per slot in every arm**
(the 2026-08-10 table compared arms at different `-c`, which is where its "12%"
came from), 300 generated tokens, single-stream median of 3:

| | single-stream | 4-stream throughput | memory |
|---|---|---|---|
| `-c 262144 -np 1` | 88.56 | 85.70 | 43 GB |
| `-c 524288 -np 2` | 85.51 | 107.39 | 40 GB |
| `-c 1048576 -np 4` | **83–85** | **135–140** | 53 GB |

Four slots cost **~5% of single-stream speed** and return **~1.6× throughput**.
On a 500-token answer the interactive cost is under a second.

The return is 1.6× and not 4× because generation is bandwidth-bound and
`--spec-type draft-mtp` is exactly what batching dilutes — the speculation that
makes one stream fast (~0.7 draft acceptance, mean accepted length ~3.1) has
less to offer four. That is not a misconfiguration.

**The second reason for `-np 4` is the load-bearing one:** `mecha batch` and
`mecha eval` have both defaulted to `--concurrency 4` since they shipped.
Against one slot that was not merely capped — it was *worse than serial*, four
conversations round-robining through a single KV cache, each evicting the
previous one's prefix on arrival. The 341 "making room for prompt cache entry"
evictions in the journal between Aug 19 and Aug 20 are that. `-np 4` does not
add a capability; it stops defeating one that already shipped.

## How to measure it — two tests, two different things

**`scripts/bench-slots.sh` — throughput.** Uses `/completion` with
`ignore_eos` so every run generates exactly `n_predict` tokens, and reads the
server's own timings so nothing measures curl.

> **Measure throughput by wall clock and nothing else.** llama-server times a
> request only while it is *running*, so summing its per-request rates hides
> queue wait entirely. At `-np 1`, four serialized streams each report their
> full solo rate and the sum reads **350 tok/s** — on the one configuration
> that cannot run them concurrently. That number nearly became evidence that
> slots are unnecessary. The script prints both and labels which is which.

**`scripts/affinity-test.py` — prefix reuse.** A multi-turn conversation with a
large stable prefix. **A throughput benchmark structurally cannot see slot
affinity**, because it sends independent prompts — precisely the workload with
no prefix to lose. The metric here is not tok/s at all; it is
`prompt eval time = … / N tokens` staying small across turns:

```
turn  1:  prompt eval  7728 ms / 14715 tokens    ← cold start, expected
turn  2:  prompt eval   259 ms /    48 tokens    ← only the new tokens
turn 12:  prompt eval   318 ms /    97 tokens
```

And the standing rule from `start-moe-mtp.sh`, which survived its own reversal:
**measure tokens/sec after restarting, not just that the server came back up.**
A server that loads while memory is contended stays slow for its whole life —
whatever placement decision is made at load is never revisited.

## Flags that cost something to learn

- **`--cache-idle-slots` is deliberately absent. Do not add it.** It saves an
  idle slot to the prompt cache on a new task *and clears it*, so the slot
  holding a live conversation's prefix is wiped, LCP similarity finds nothing,
  and slot selection falls through to LRU — onto a cold slot. Measured over one
  TUI session: 3 of 25 turns re-prefilled the whole transcript, the worst
  costing 20.5 s for 29,570 tokens. Removing it: 1 LRU selection in 44 (the
  cold start), 48–106 tokens re-prefilled per turn instead of 15,000, and
  throughput unchanged (84.92 vs 84.99 single, 135.32 vs 140.33 4-stream). It
  bought nothing and cost the thing slots exist to protect. mecha's own cache
  lens is what caught it.
- **`-cram` (prompt cache) defaults to 8192 MiB**, which is smaller than *one*
  262k slot's KV at f16 (~5.5 GB), so it thrashes. At 32768 there have been zero
  evictions.
- **`--reasoning-budget` is a server flag; the per-request `reasoning_budget`
  field is silently ignored by this build.** ollama's runner never passes the
  flag, so a model served through it reasons unbounded.

  **Be careful what you attribute to that.** This flag was added on 2026-08-07
  believing it fixed "non-terminating reasoning", and that diagnosis was
  *retired on 2026-08-10*: replay showed the empty turns were tool calls
  emitted before `</think>` closed, with `reasoning_content` unparsed by the
  harness — `finish_reason: "stop"`, no content, no `tool_calls`, and no token
  budget involved at any point (CHANGELOG 0.1.2). The flag is harmless and
  stays. The stale explanation survived in this script's comments for ten days
  and was still being repeated on 2026-08-20, which is the actual lesson:
  a retired diagnosis left in a comment gets re-derived as fact by the next
  reader.

  What mecha-graph's 300 s extraction timeouts under ollama were *actually*
  caused by is **not established**. The measured contributor is contention —
  two copies of the model on one GPU put interactive generation at 28 tok/s
  against a 79.8 baseline, which turns a documented 45 s/episode into minutes.
  Unbounded reasoning is a plausible additional factor and was never isolated.
- **`--jinja` uses the model's own chat template**, and without it every
  per-request template control is silently inert.
- **`-np 1` is not the defence against the `-c` division trap.** Setting `-c`
  to `slots × window` deliberately is.

## Vision — a model is two files

**Every model this repository serves is multimodal, and on 2026-08-21 every
one of them was being served text-only.** The symptom is a model that answers
"I don't have the ability to view image files directly", which reads as a
limitation of the weights. It is a flag nobody passed.

The weights hold the language model. The vision tower ships beside them as a
separate **`mmproj-*.gguf`** in the same repository, and `--mmproj` must name
it. Three things conspire to hide this:

- **`--mmproj-auto` defaults to enabled**, so the flag list looks handled. It
  only fires for `-hf` downloads. Every script here uses `-m <path>`, so the
  default does nothing exactly where it is needed.
- **`GET /props` answers a different question than the one being asked.**
  `modalities.vision` reports what is *loaded*, never what the architecture
  supports. A multimodal model with no projector reports `false` and is
  indistinguishable from a text-only one.
- **Nothing fails.** The server starts, answers, and serves well. There is no
  error to find.

### How to tell a multimodal GGUF from its metadata

Three independent tells, none of which requires loading the model:

| Key | Multimodal |
|---|---|
| `general.tags` | contains `image-text-to-text` |
| `<arch>.rope.dimension_sections` | present — mRoPE, e.g. `[11, 11, 10, 0]`. A text-only model has none. |
| `tokenizer.chat_template` | handles `image_url` items and emits `<\|vision_start\|><\|image_pad\|><\|vision_end\|>` |

And the confirming absence: **no vision tensors in the file.** Qwen3.6-35B-A3B
carries 750 `blk.*`, `token_embd`, `output`, `output_norm` and nothing else.
All three tells said "multimodal"; the tensor list said "half of it is
missing".

### What it costs, measured

Sending the screenshot that started this (a photo of a laptop screen) through
`/v1/chat/completions` as an `image_url` data URI:

| | raw | resized |
|---|---|---|
| file | 2222x1548, 5.7 MB PNG | 1568x1092, 179 KB JPEG |
| `prompt_tokens` | **294** | **294** |
| wall clock | 9.4 s | 5.9 s |
| text read back | correct, verbatim | correct, verbatim |

**The token cost is identical**, because the server tiles the image to a fixed
count before the model sees it. So resizing buys nothing in context and 32x on
the wire and in the session file — which is why `mecha_core::image` caps at
the door rather than per turn: the transcript is append-only and every turn
resends the whole history, so the resize is paid once and the saving is
collected on every turn afterwards.

- **`--image-min-tokens` / `--image-max-tokens`** bound what one image may
  cost, if the default tiling is ever the wrong trade.
- **The `max_tokens` trap above is worse here.** Vision prompts reason
  longer: at `max_tokens: 300` this returned 300 tokens of
  `reasoning_content` and an empty `content`. The rule is unchanged and bites
  sooner.
- **`--mmproj` and `--spec-type draft-mtp` coexist.** Verified on gemma-4-E4B
  with a control: the `[spec] failed to measure draft model memory` warning
  appears with and without `--mmproj`, so it is not caused by the projector.

### The guard

`scripts/mmproj.sh` is sourced by every start script here. It resolves
`mmproj-BF16.gguf` then `mmproj-F16.gguf`, and when neither is present it
**exits with the `curl` that fetches it** rather than starting. Refusing is
the decision: starting anyway is what produced four servers quietly without
eyes. `--no-mmproj` is the explicit way to ask for a text-only arm.

A shared function rather than a line per script, because a line per script is
precisely how gemma-4-26B ended up with its projector sitting on disk, unused,
from the day it was downloaded.

### And config has to agree

`[providers.X] vision = true` is what makes mecha *send* an image; a loaded
projector alone changes nothing. `provider::preflight` reads `/props` once at
startup and warns in **both** directions, because they fail differently:
declared-but-not-served silently degrades every image to a line of text, and
served-but-not-declared means a projector is loaded, paid for in memory, and
never used. Warning rather than refusing — a preflight that can stop a working
machine from starting is one people turn off.

## The request contract

- **`max_tokens` must exceed `--reasoning-budget` (4096), comfortably.** Below
  it, the thinking block consumes the whole allowance and the reply arrives as
  **HTTP 200 with an empty `content`** — which reads as "the model had nothing
  to say". Measured: at `max_tokens` 1024, 1024 tokens of reasoning and no
  answer; at 8192, `finish_reason: stop` and valid output. Any client here
  should refuse an empty completion by name rather than treat it as data.
- **`response_format: json_schema` and thinking coexist.** llama.cpp applies
  the grammar *lazily*, after the thinking block closes — measured at 3,240
  chars of reasoning followed by schema-valid JSON. So a closed vocabulary can
  be an `enum` the sampler cannot violate, without giving up deliberation.
- **`chat_template_kwargs` (e.g. `enable_thinking`) requires `--jinja`.** Under
  a chatml override it is accepted and ignored. Note the corollary: a client
  that sends *no* template kwargs has no dependency on the model's template and
  survives a model swap.
- **Ask what is served, do not assert it.** `GET /props` returns `model_alias`.
  llama-server serves whatever is loaded and ignores the `model` field of a
  request, so naming a model is not selecting one — it only decides what gets
  written down. mecha-graph records the served alias in `extract_state.model`
  for exactly this reason.

## Embeddings

```
--embeddings --pooling last --embd-normalize 2 -c 32768
```

- `--pooling last` for the decoder-only families (Qwen3-Embedding, Harrier);
  BERT-style embedders want `mean`.
- `--embd-normalize 2` (L2) works on this build — vectors come back exactly
  unit-norm, so the "you must normalise manually" advice in older llama.cpp
  discussions does not apply here. Verify with a norm check, not by assuming.
- **A batch is one HTTP request, billed as a whole against `-c`.** A batch of
  ordinarily-sized inputs can overflow a context that every individual input
  fits in comfortably — this killed a full re-embed 348 s in, on a single
  9,292-token request. The client splits and retries on overflow, which
  converges on the one input that genuinely does not fit.
- Measured full re-embed of 27,140 vectors (20,444 episodes + 6,696 facts):
  **0.6B ≈ 9–10 min, 4B ≈ 27 min** and double the storage.

## Related

- `scripts/start-moe-mtp.sh` — the flags, and the history behind each number
- `scripts/mmproj.sh` — the projector guard every start script sources
- `provider/preflight.rs` — one `GET /props`, checked against config
- `scripts/bench-slots.sh` — throughput
- `scripts/affinity-test.py` — prefix reuse
- `cache_lens.rs` — the per-run observer that caught the affinity regression
