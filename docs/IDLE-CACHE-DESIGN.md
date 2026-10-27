# Idle chats to disk — design, and the measurement that decides it

Status: **phase 0 (the sensor) is built; the cache itself is not, and waits on
what the sensor reports** (§5). Written 2026-10-27, against llama.cpp
`27637851` and the uncensored Qwen3.6-35B-A3B MTP graft on `:8080`.

## 1. What this is for

A chat the server has forgotten is read again from the first token. On this
machine that is ~1,400–2,150 tokens a second, so a 40k-token chat costs about
20 s before the first word of the reply and a 176k one more than two minutes.
The server keeps a chat in two places, both bounded and both lost on any
restart or model switch:

- **its slot** — four of them (`-np 4`), each a 262,144-token KV reservation
  held whether used or not;
- **the RAM prompt cache** — `cache-ram = 8192` MiB of host memory (the same
  unified pool the GPU draws on), filled with idle slots' states and evicted
  least-recently-used.

Saving an idle chat's slot state to disk and restoring it on the next message
would turn the re-read into a restore of a few seconds, survive restarts and
model switches, and let `cache-ram` shrink — which is the memory this would
actually free. It frees **no GPU memory**: the four slots' KV is reserved at
load whatever they hold.

## 2. What was measured (2026-10-27)

All on private ports, never the live server; numbers from one run of each
unless a repeat count is given.

| Question | Result |
|---|---|
| Save / restore on this hybrid (11 attention + 30 DeltaNet layers) model | Works. A restored 176k-token chat took its next turn reading 33 tokens instead of 175,706 |
| Through the router | Works when the body names `"model"` — `/slots` is proxied like the chat routes. Restore lands in any slot, not only the one saved from |
| Cost | Save 0.5–1.3 s; restore from SSD (page cache evicted) 1.7–4.2 s at 176k; a 40k chat's file is 0.9 GB (f16) — 1.1 GB on `27637851`, which appends checkpoints and the draft state |
| A branch after a restore (regenerate, edited message) | Re-read everything on `95887577`; **re-reads only past the nearest checkpoint on `27637851`** (516 tokens instead of 40,011: upstream #26004) |
| An extension after a restore | Reads only the new tokens on both builds |
| The RAM prompt cache | Already serves an extension when the entry is still there — disk adds only what it has evicted |

Rejected on the same day, with their measurements, so they are not
re-proposed as cheaper routes to the same memory:

- **q8_0 KV.** Retrieval held (12/12 needles at 176k against f16's 11/12) and
  it saves ~2 GB per 262k slot, but with MTP drafting (`spec-draft-n-max 2`)
  generation falls 12–36% from 32k up: on Ada-and-later GPUs ggml-cuda gives a
  quantized cache the vector kernel only for batches of ≤ 2, and draft
  verification is a batch of 3, so every step converts the whole cache to f16
  (`fattn.cu`, kernel selection; unchanged at `27637851`).
- **`--kv-unified`.** Overfilled, no request fails, but the server purges an
  idle slot's *whole* prefix to make room (`try_clear_id`) — the thrash the four
  separate slots exist to prevent (`0135697da`).

## 3. How often it would help — the upper bound

From the last seven days of outcome rows, before the sensor (§4) existed:
of 132 runs that continued a chat, **21 read more than 10k uncached tokens**
(13% of persona-chat runs, 22% of main-chat runs), together about **12 minutes
of prompt reading a week**, most of it in a handful of 87k–338k misses.

That is a ceiling, not the number this would save. `input_tokens` on a row is
everything not read from cache, which includes the turn's own tool results,
and a miss mecha caused by changing the prompt's head — a re-rendered system
prompt, a compaction, a persona context update — would miss a restored state
exactly as it misses the slot. Only an eviction (the server lost a prefix that
mecha sent unchanged) is what disk would fix.

## 4. Phase 0: the sensor (built)

`cache_lens` already told the two kinds of miss apart, but only within a run,
and in a chat one message is one run — so the request that matters, the first
one after an idle gap, was always a `Baseline`. The lens now rides the
`Conversation` across runs, and each run's outcome row records how its first
request met the cache (`RunStats::resume_cache`, `cache_lens::ResumeCache`):

| `verdict` | Meaning | Would disk have helped? |
|---|---|---|
| `stable` | The prefix came back | No — nothing was lost |
| `drop` | Nothing changed, the transcript only grew, and the previous prompt was paid for again (`repaid`, `prev_total`) | **Yes** — an eviction |
| `transcript_rewritten` | The messages are no longer an extension of the last request | No — mecha changed the head |
| `surface_changed` | System prompt, tools or model changed (`*_changed` says which) | No for system/tools; a model switch is a reload, which disk would survive |
| `baseline` | No earlier request in this process — a new chat, or one reopened after a serve restart | Unknown: the restart case is one disk would serve |
| `unobservable` | The provider has reported no cache figure yet | — |

Beside it: `idle_secs` (since the conversation's previous request), the first
request's `cache_read` and `uncached`, and `contended` (an agent that shares
slots by design). Numbers and a closed verdict only; the lens holds hashes,
never content.

**The readout**, after a week or two of normal use:

```bash
# Every resume verdict recorded, with its idle gap and what was re-read.
find ~/.mecha/sessions ~/.mecha/personas/*/sessions -name '*.jsonl' -mtime -14 -print0 \
  | xargs -0 cat \
  | jq -c 'select(.record=="outcome" and .resume_cache) | .resume_cache
           | {verdict, idle_secs, repaid, prev_total, uncached}'
```

## 5. The decision

Build §6 if, over the readout's window, `drop` verdicts on chats idle more
than a few minutes account for **most of the re-read time** and at least a few
waits a week of half a minute or more. If `transcript_rewritten` or
`surface_changed` dominate, the work is keeping mecha's prompt head stable, and
a disk cache would not help. If misses are rare once `27637851`'s RAM-cache
fixes are in (only restorable states cached, checkpoints kept), do nothing.

## 6. The cache, if built

Owned by `mecha-serve`, which owns the chats; a llama.cpp patch (a disk tier
under its own RAM cache) is the alternative and is not chosen, because it
would make the engine a fork the pinned-release route (`engine.rs`) cannot
upgrade.

- **When to save.** After a chat's run ends and it stays idle N minutes (start
  at 5) with no run in flight on the server, `POST /slots/{id}?action=save`
  with the model named. Saving is ~1 s and blocks that slot only.
- **Which slot.** The open problem. `/v1/chat/completions` does not report the
  slot that served a request (`id_slot` is in the native `/completion` result
  only). `id_slot` *is* accepted on chat requests, so mecha could pin each chat
  to a slot itself — but `serve` is not the only client of the four slots
  (triggers, batch, voice), and pinning overrides the server's
  prefix-similarity choice. `GET /slots` returns each slot's prompt text, which
  identifies the chat at the cost of up to four 262k-token prompts per call.
  Settle with a probe before writing code; a two-line upstream change to echo
  `id_slot` in the OpenAI-shaped response is the clean fix.
- **When to restore.** On a chat's next message, if a saved file exists and is
  newer than the chat's last request, restore into an idle slot before sending.
  Restoring overwrites whatever that slot held, which is itself in the RAM
  cache or on disk.
- **Validity.** A file is a state for one model file, one build, one KV type
  and one context geometry. A header beside it records all four and the token
  count; any mismatch deletes it rather than trying it.
- **Storage.** `~/.mecha/kvcache/`, mode 0700, one file per chat; an LRU byte
  budget (40k tokens ≈ 1 GB at f16, so a budget is necessary from the first
  day). Deleted with its chat (archive keeps it, delete removes it).
- **Privacy.** A slot file is the chat's tokens and their KV — derived chat
  content at rest, so it is private data with the transcript's lifecycle.
  **Incognito chats are never saved.** The files are excluded from backups.
- **What it is not.** It does not save GPU memory, and it does not fix a miss
  mecha caused by rewriting the prompt's head.

## 7. Owed

- The readout (§4) after one to two weeks on `27637851`, then the §5 call
  (owner).
- If built: the slot-identification probe (§6), then the owner's rulings on the
  idle threshold and the disk budget.
