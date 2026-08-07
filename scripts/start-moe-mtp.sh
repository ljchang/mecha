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
# -c 32768 is also load-bearing, and it is a *performance* ceiling rather than
# a capability one. Raising it to 131072 on 2026-08-07 — to give a thinking
# turn more room — took generation from 64 tokens in 1.06s to 64 tokens in
# 52.6s, a 50x collapse, with the process pinned at ~97% of one core: the KV
# cache no longer fits alongside the model and the offload falls apart. The
# failure is silent. Nothing errors, the server answers every request, and the
# only symptom is that a benchmark trial sits on its first turn for forty
# minutes and then trips the harness's agent timeout. Measure tokens/sec after
# touching this, not just that the server came back up.
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
  --host 127.0.0.1 --port 8080 -ngl 999 -c 32768 -np 1 --alias qwen3.6-35b-a3b --jinja \
  --reasoning-budget 4096 \
  --spec-type draft-mtp
