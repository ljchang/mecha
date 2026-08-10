#!/bin/bash
M=$(ls ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--Qwen3.6-35B-A3B-MTP-GGUF/snapshots/*/Qwen3.6-35B-A3B-UD-Q4_K_M.gguf)
# -np 1 is load-bearing: newer llama-server defaults to 4 parallel slots,
# which silently splits -c across them — 8192 per request, not the 32768
# that mecha's `context_window` promises. Past 8192 the server context-
# shifts instead of erroring, the model sees a mangled transcript, and the
# symptom is empty completions that look like a model failure. Found
# 2026-08-05 after the empty-EndTurn deaths in the k=5 compaction runs.
# Concurrent eval requests serialize instead; the MoE is fast enough.
#
# What `-np` actually buys, measured 2026-08-10 (short prompt, 300 generated
# tokens, so this is generation and not prefill):
#
#   -c 131072 -np 1   1 stream                 79.8 tok/s   (three runs, ±0.1)
#   -c 262144 -np 4   1 stream                 70.5 tok/s
#   -c 262144 -np 4   4 streams   ~35 each,   ~129 tok/s aggregate
#
# So four slots cost **12% of single-stream speed** and return **1.6x
# aggregate**. Four independent tasks finish in about 0.6x the wall clock of
# running them one after another — not 0.25x. Generation here is bandwidth
# bound, and `--spec-type draft-mtp` is exactly the thing batching dilutes: the
# speculation that makes one stream fast has less to offer four.
#
# The choice is therefore per workload, not global. Interactive use — chat, the
# TUI, Slack, a trigger — is single-stream and should keep `-np 1`. A batch or
# eval sweep that fans out is the case for `-np 4`, and it wants `-c` raised
# with it, because **`-c` is divided across slots**: `-c 262144 -np 4` is four
# slots of 65,536, not four of 262,144. That division is the same trap as the
# build's old 4-slot default, which silently gave each slot an eighth of what
# the config claimed.
#
# The model is trained for 262,144 (`qwen35moe.context_length`), so 131,072 is
# half its native window and needs no RoPE scaling. There is room above this.
#
# -c 131072 since 2026-08-10, and the story of that number is worth keeping,
# because for three days this script said "do not raise it" on the strength of
# one bad afternoon.
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
#   prompt tokens |  -c 32768 |  -c 65536 | -c 131072
#          1,075  |     92.44 |     92.23 |     92.99
#          4,309  |     88.96 |     89.09 |     89.87
#         30,153  |     80.74 |     81.44 |     81.79
#         55,675  |         — |     74.59 |         —
#         64,621  |         — |         — |     72.17
#        107,699  |         — |         — |     63.35
#
# **`-c` costs nothing.** Generation speed is a function of how much context is
# actually *used*, not of how much was allocated — and RSS is 21.5 GB at both
# 32768 and 131072, so the allocation is not even visible in memory. What does
# cost is real context: 92 tok/s at 1k against 63 tok/s at 108k, which is
# attention doing what attention does.
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
# --reasoning-budget 4096 is what actually fixes the empty completions this
# model produces on hard tasks. Unbounded, it reasons without terminating:
# measured here, max_tokens 8192/16384/24576 each returned reasoning and an
# empty `content`, and 12288 produced an answer in only 2 of 4 samples — a coin
# flip, which is why some benchmark trials passed and some died. With the
# budget the server closes the thinking tag, injects
# --reasoning-budget-message, and the model answers: 4 of 4. Note it must be a
# server flag — the per-request `reasoning_budget` field is silently ignored by
# this build, A/B tested at identical max_tokens with the "with" arm still
# coming back empty.
#
# Anything setting `max_tokens` against this server must leave room for an
# answer *after* the budget: above 4096, and comfortably. `~/.mecha/config.toml`
# and `bench/mecha_agent.py` both carry it, alongside a `context_window` that
# has to equal the `-c` here. Four numbers, and a mismatch in any of them is
# silent.
exec ${LLAMA_SERVER:-llama-server} -m "$M" \
  --host 127.0.0.1 --port 8080 -ngl 999 -c 131072 -np 1 --alias qwen3.6-35b-a3b --jinja \
  --reasoning-budget 4096 \
  --spec-type draft-mtp
