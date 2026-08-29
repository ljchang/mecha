#!/bin/bash
S=$(ls -d ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--Qwen3.6-35B-A3B-MTP-GGUF/snapshots/*/)
M="$S/Qwen3.6-35B-A3B-UD-Q4_K_M.gguf"
# **A vision model is two files, and this one is multimodal.** The weights
# carry the language model; the vision tower ships beside them as a separate
# mmproj-*.gguf. Without --mmproj the server loads, answers, reports
# modalities.vision:false, and a model asked about a screenshot says it cannot
# see images — which reads as a limitation of the model rather than a flag
# nobody passed. --mmproj-auto is enabled by default and only fires for -hf
# downloads, so it does nothing for any script here, all of which use -m.
# mecha's startup preflight reads GET /props and warns in both directions.
source "$(dirname "$0")/mmproj.sh"
MMPROJ=$(mmproj_or_die "$S" unsloth/Qwen3.6-35B-A3B-MTP-GGUF)
# **`-c` is divided across slots, and that is the trap to keep in mind.**
# `-c 262144 -np 4` is four slots of 65,536, not four of 262,144 — which is why
# the build's own 4-slot default was dangerous: it silently gave each request an
# eighth of what `context_window` claimed, and past the real limit the server
# context-shifts instead of erroring, so the model sees a mangled transcript and
# the symptom is empty completions that look like a model failure. Found
# 2026-08-05 after the empty-EndTurn deaths in the k=5 compaction runs. The
# defence is not `-np 1`; it is setting `-c` to `slots x window` deliberately,
# which is what CTX below does.
#
# -np 4 since 2026-08-20, replacing -np 1. Two reasons, and the second is the
# real one:
#
# 1. Measured cost is small. Re-measured on a quiet machine with
#    `scripts/bench-slots.sh`, 262,144 per slot in every arm, 300 generated
#    tokens, single-stream median of 3, throughput by WALL CLOCK:
#
#      -c  262144 -np 1   single 88.56   4-stream  85.70 tok/s   43 GB
#      -c  524288 -np 2   single 85.51   4-stream 107.39 tok/s   40 GB
#      -c 1048576 -np 4   single 83.24   4-stream 138.45 tok/s   53 GB
#
#    Four slots cost **6% of single-stream speed** — not the 12% recorded on
#    2026-08-10, which compared arms at different `-c` — and return **1.62x
#    throughput**, which does match. On a 500-token answer the interactive cost
#    is under a second.
#
#    Measure throughput by wall clock and nothing else. llama-server times a
#    request only while it is *running*, so summing its per-request rates hides
#    queue wait entirely: at `-np 1` four serialized streams each report their
#    full solo rate and the sum reads 350 tok/s, on the one configuration that
#    cannot run them concurrently. bench-slots.sh prints both and labels which
#    is which.
#
# 2. `mecha batch` and `mecha eval` have BOTH defaulted to `--concurrency 4`
#    all along. Against one slot that was not merely capped — it was worse than
#    serial: four conversations round-robined through a single KV cache, each
#    evicting the previous one's prefix on arrival, which is what the 341
#    "making room for prompt cache entry" evictions in the journal are. -np 4
#    does not add a capability here; it stops defeating one that already
#    shipped.
#
# Generation is bandwidth bound and `--spec-type draft-mtp` is exactly what
# batching dilutes — the speculation that makes one stream fast (~0.7 draft
# acceptance, mean accepted length ~3.1) has less to offer four. That is why
# the return is 1.6x and not 4x, and it is not a misconfiguration.
#
# -c 262144 since 2026-08-10 — the model's whole trained window
# (`qwen35moe.context_length` = 262144), so no RoPE scaling and nothing
# extrapolated. The story of that number is worth keeping, because for three
# days this script said "do not raise it" on the strength of one bad afternoon.
#
# Raising it to 131072 on 2026-08-07 took generation from 64 tokens in 1.06s to
# 64 tokens in 52.6s — a 50x collapse, the process pinned at ~97% of one core,
# nothing logged. That measurement was real. The conclusion drawn from it, that
# a large `-c` cannot work here, was not: 2026-08-07 is the day a runaway test
# OOMed this machine and systemd tore down both llama-servers, so the KV cache
# was competing for memory that was not there and the offload fell apart.
#
# Re-measured 2026-08-10 on a quiet machine, one server at a time, median of
# three runs per point, identical prompts across arms (tok/s generated):
#
#   prompt tokens | -c 32768 | -c 65536 | -c 131072 | -c 262144
#          1,075  |    92.44 |    92.23 |     92.99 |     91.96
#          4,309  |    88.96 |    89.09 |     89.87 |     88.67
#         30,153  |    80.74 |    81.44 |     81.79 |     80.30
#         55,675  |        — |    74.59 |         — |         —
#         64,621  |        — |        — |     72.17 |         —
#        107,699  |        — |        — |     63.35 |         —
#
# **`-c` costs nothing in speed.** Generation is a function of how much context
# is actually *used*, not of how much was allocated. What does cost is real
# context: 92 tok/s at 1k against 63 tok/s at 108k, which is attention doing
# what attention does — and prefill, which is the bigger bill. A 188k-token
# prompt takes ~120s to read at ~1,570 tok/s before a single token comes back.
#
# What it costs in memory is a **reservation made at startup**, which is why
# filling 99k tokens of context moved the number not at all — the room was
# already taken. Measured by stop/start on an idle machine (total system used,
# minus a 7.15 GB baseline):
#
#   -c  32,768  ->  21.4 GB      -c 131,072  ->  25.3 GB
#   -c  65,536  ->  22.7 GB*     -c 262,144  ->  28.5 GB
#
# which fits weights ~20.7 GB plus ~32 KiB per token of KV cache (*interpolated).
#
# The 82 KiB/token the geometry seemed to predict — 41 layers x 2 KV heads x 512
# x 2 bytes — was resolved 2026-08-20 and the answer is that **this is a hybrid
# model**. The GGUF carries `qwen35moe.full_attention_interval = 4` and a family
# of `qwen35moe.ssm.*` keys, and counting tensors settles it:
#
#   layers with attn_k (full attention, KV grows):  11  -> 3,7,11..39, and 40
#   layers with ssm_*  (gated DeltaNet, O(1) state): 30
#
# Only 11 of 41 layers hold a KV cache at all; the other 30 carry a fixed-size
# recurrent state (~64 MiB per slot, flat in context length). So the per-layer
# arithmetic was right and was applied to the wrong layer count:
# 82 x (11/41) = 22.0 KiB/token at f16, plus n_ctx-scaled compute buffers.
#
# This is also why generation decays so gently with depth — 63 tok/s at 108k
# against 92 at 1k, where a dense 35B would fall much harder. Long context is
# cheap here in a way it is not on a pure-attention model.
#
# KV type is `-ctk/-ctv f16` by default and deliberately left there: q8_0 would
# halve the growing part, but the memory is available and the property being
# protected is exact long-context retrieval — the needle at 21% depth below.
#
# The window is usable, not merely allocatable: a needle placed at 21% depth in
# a 188,546-token prompt was retrieved exactly.
#
# One trap found while measuring, and it is the likely shape of the 08-07
# result: **a server that loads while memory is contended stays slow for its
# whole life.** A 64k instance started alongside another resident model held
# ~82 tok/s at a 1k prompt and did not recover when the other was stopped; a
# fresh 64k instance on a quiet machine gave 92.23. Whatever placement decision
# is made at load is never revisited. So the standing advice survives its own
# reversal: **measure tokens/sec after restarting this, not just that the
# server came back up** — and if it is slow, restart it on a quiet machine
# before believing anything about the flags.
#
# --reasoning-budget 4096 was added 2026-08-07 believing it fixed the empty
# completions this model produces on hard tasks — "unbounded, it reasons
# without terminating".
#
# **That diagnosis was retired on 2026-08-10 and this comment did not follow.**
# Replaying the exact prefixes that went quiet reproduced an empty turn whose
# "reasoning" was a complete, unparsed tool call — in one case 120 characters
# that were *only* a tool call, with no deliberation at all. `finish_reason:
# "stop"`, no content, no `tool_calls`. The model emitted its call before
# closing `</think>`, and the harness read no `reasoning_content` at all. No
# token budget was ever involved. See CHANGELOG.md under 0.1.2.
#
# The flag is harmless and stays: it bounds thinking, which is genuinely useful
# for batch work like mecha-graph's extractor. It must be a *server* flag — the
# per-request `reasoning_budget` field is silently ignored by this build.
#
# The reason this correction is written out at length rather than deleted: the
# retired explanation sat here for ten days and was read back as fact on
# 2026-08-20, propagating into two other documents before CHANGELOG 0.1.2 was
# re-read. A superseded diagnosis left in a comment is worse than no comment.
#
# Anything setting `max_tokens` against this server must leave room for an
# answer *after* the budget: above 4096, and comfortably. `~/.mecha/config.toml`
# and `bench/mecha_agent.py` both carry it, alongside a `context_window` that
# has to equal **CTX/NP** — the per-slot window, not CTX. That changed on
# 2026-08-20 when NP stopped being 1; before then the two were the same number
# and the distinction cost nothing to ignore. At CTX 1048576 / NP 4 the answer
# is still 262144, so `context_window` did not move — but anyone changing NP
# without changing CTX moves it silently, which is the whole hazard.
# Four numbers, and a mismatch in any of them is silent.
#
# Overridable so `scripts/bench-slots.sh` can measure a candidate without
# editing this file. The committed values are the measured ones; the standing
# rule above — measure tok/s after a restart, not just that it came back up —
# is what these exist to make cheap.
#
# NOTE on CTX: it is the budget across ALL slots, divided evenly. CTX/NP is
# what a single request may use, and that quotient is the number
# `context_window` in ~/.mecha/config.toml must equal — not CTX itself.
NP="${MECHA_LLAMA_NP:-4}"
CTX="${MECHA_LLAMA_CTX:-1048576}"
# The prompt cache holds evicted slot states so a returning prefix is restored
# instead of re-prefilled. The 8192 MiB default is smaller than ONE 262k slot's
# KV (~5.5 GB at f16), so it thrashed: 341 "making room for prompt cache entry"
# evictions between Aug 19 and Aug 20, each one paying a re-prefill at
# ~1,570 tok/s. Matters more with every slot added. At 32768 there have been
# zero evictions.
CRAM="${MECHA_LLAMA_CRAM:-32768}"

# **`--cache-idle-slots` is deliberately absent. Do not add it back.**
#
# It was added with -np 4 on 2026-08-20, on the reasoning that saving idle slot
# state to the prompt cache would help prefix reuse. It did the opposite. It
# saves an idle slot on a new task *and clears it*, so the slot holding a live
# conversation's prefix gets wiped, LCP similarity then finds nothing, and slot
# selection falls through to LRU — onto a cold slot. Measured over one TUI
# session: 3 of 25 turns picked a slot by LRU and re-prefilled the whole
# transcript, the worst costing 20.5s for 29,570 tokens. mecha's own cache lens
# is what caught it ("prompt cache reuse dropped: re-paid 15733 input tokens").
#
# With the flag removed, same test: 1 LRU selection in 44 (the cold start), and
# a 14,715-token prefix reused on every subsequent turn — 48 to 106 tokens
# re-prefilled per turn instead of 15,000. Throughput was unaffected
# (single 84.92 vs 84.99, 4-stream 135.32 vs 140.33 tok/s), so it bought
# nothing and cost the thing slots exist to protect.
#
# The general lesson, and the reason this paragraph is long: a throughput
# benchmark CANNOT see this. bench-slots.sh sends independent prompts, which is
# precisely the workload with no prefix to lose. Slot affinity needs its own
# test — a multi-turn conversation with a large stable prefix — and the metric
# is `prompt eval time = ... / N tokens` staying small, not tok/s.

# **Qwen3.6-35B-A3B's published PRECISE-CODING thinking recipe**, spelled out
# in full rather than inherited, for parity with start-qwen38.sh — which has
# always set all six and is the reason the drift below was visible at all.
#
# The card gives two thinking profiles: *general* (temp 1.0, presence 1.5) and
# *precise coding / WebDev* (temp 0.6, presence 0.0). This server is the second
# one. Almost everything mecha asks it for is structured or exacting — tool
# calls in the agent loop, and JSON verdicts out of the reflector, learner,
# judge and distiller — and it was already serving presence_penalty 0.0, which
# is half of that profile arriving from the GGUF.
#
# Before this it served the GGUF's metadata defaults (temp 1.0, top_p 0.95,
# top_k 20, repeat 1.0, min_p 0.05) — the *general* profile with an off-spec
# min_p — while `[providers.local] temperature = 0.8` overrode the temperature
# from the client with a value belonging to neither profile. Set explicitly so
# a requantisation cannot move any of them quietly.
#
# **`--temp` here is overridden by `[providers.local] temperature`**, because
# mecha sends temperature on every request; the two must agree or the model is
# silently un-tuned. The other five have no client-side equivalent —
# `ProviderConfig` carries only `temperature` and `seed` — so they can only be
# set here. Same trap start-qwen38.sh documents.
#
# presence_penalty stays 0.0. The card lists 1.5 for *general* thinking and
# 0.0 for the precise-coding profile, and Qwen3.8-27B lists 0.0 for thinking
# throughout; agentic tool use is the second shape, not the first. Worth
# knowing when reading the ollama post-mortem above, which lists an inherited
# `presence_penalty 1.5` among that setup's costs: the complaint was that
# nobody chose it, not that the number was wrong for every profile.
#
# **Sampling is measured, not assumed.** `curl -s localhost:8080/props` prints
# what the server will actually use; a flag here only means something if it
# shows up there after a restart.

exec ${LLAMA_SERVER:-llama-server} -m "$M" \
  --mmproj "$MMPROJ" \
  --host 127.0.0.1 --port 8080 -ngl 999 -c "$CTX" -np "$NP" --alias qwen3.6-35b-a3b --jinja \
  -cram "$CRAM" \
  --reasoning-budget 4096 \
  --temp 0.6 --top-p 0.95 --top-k 20 --min-p 0.0 \
  --presence-penalty 0.0 --repeat-penalty 1.0 \
  --spec-type draft-mtp
