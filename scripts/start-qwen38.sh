#!/bin/bash
# Qwen3.8-27B — the 3.8 generation's only locally runnable model.
#
# Read this before assuming it is a drop-in for start-moe-mtp.sh: **it is
# dense, and that one word is the whole story.** Qwen3.8 shipped exactly two
# models, and neither is a mid-size MoE. The only 3.8 MoE is
# Qwen3.8-2.4T-A95B, whose 4-bit quant (UD-IQ4_XS) is 1,310.9 GB against this
# machine's 121 GB of unified memory; even its most brutal 1-bit is 397 GB. It
# is not runnable here and never will be. So Qwen3.6-35B-A3B stays the fast
# model — 3B active params per token — and this is the quality/latency
# experiment sitting beside it, not its replacement. Verified 2026-08-14
# against the HF API, not from a blog post.
#
# What is actually here: 27B dense, Apache 2.0 (the 2.4T is *not* — it carries
# a custom license with revenue triggers), 262,144 native context, released
# ~2026-08-12. Days old at the time of writing, so treat quirks as unmapped
# rather than absent.
M=$(ls ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--Qwen3.8-27B-GGUF/snapshots/*/Qwen3.8-27B-Q4_K_M.gguf)

# Speculative decoding needs a SECOND file here, unlike the 3.6 MoE.
#
# unsloth published Qwen3.6-35B-A3B-MTP-GGUF with the multi-token-prediction
# tensors baked into the single file, which is why start-moe-mtp.sh passes
# `--spec-type draft-mtp` and nothing else. For 3.8 they published no MTP
# variant at all — only the plain Qwen3.8-27B-GGUF — and the standard GGUF
# conversion drops the MTP tensors on the floor.
#
# The base model does have the head: Qwen/Qwen3.8-27B's config.json declares
# `mtp_num_hidden_layers: 1`. a4lg/Qwen3.8-27B-MTP-ONLY-GGUF is that head
# extracted from the official weights and nothing else (2.03 GB at Q4_K_M,
# Apache 2.0), published precisely so models "without MTP tensors" can still
# speculate. Its own README says to benchmark rather than trust it, which is
# also this file's position.
#
# This is the repo's Method 1 (separate draft file). Method 2 grafts the MTP
# tensors into the main GGUF with a conversion script, saving the memory the
# two files duplicate and slightly changing the acceptance rate. Worth doing if
# this arm earns its place; not worth doing to find out whether it does.
D=$(ls ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--a4lg--Qwen3.8-27B-MTP-ONLY-GGUF/snapshots/*/Qwen3.8-27B-MTP-ONLY-Q4_K_M.gguf)

# Port 8083: 8080 is the qwen3.6 MoE, 8081 gemma-4-e4b, 8082 gemma-4-26b-a4b.
# Standing this up does not disturb 8080 — that is the point of a fourth port
# rather than an edit to the third.
#
# -c 262144 is the model's whole trained window
# (text_config.max_position_embeddings), so nothing is extrapolated and no RoPE
# scaling applies — the same reasoning start-moe-mtp.sh records at length.
#
# It is affordable here for a reason specific to this architecture. Qwen3.8-27B
# is a Gated DeltaNet hybrid: only 16 of its 64 layers are `full_attention`,
# the rest are `linear_attention` whose state does not grow with context. With
# 4 KV heads at head_dim 256, the KV cache costs ~64 KB per token — about
# 16.8 GB at the full 262,144, on top of 17.1 GB of weights and 2.0 GB of
# draft. Roughly 36 GB resident against 121 GB total. A dense 27B with full
# attention on every layer would not fit this window and this is why it does.
#
# -np 1 for the same load-bearing reason as the MoE script: `-c` is DIVIDED
# across parallel slots, so a default 4 would silently hand each request 65,536
# tokens while config.toml promises 262,144. Past that the server
# context-shifts instead of erroring and the symptom is empty completions that
# look like a model failure. Raise `-np` only for a batch sweep, and raise `-c`
# with it.
#
# --reasoning-budget 4096 is here for PARITY, and you should know that the
# story attached to it in start-moe-mtp.sh is retired. That file still explains
# the flag as the mitigation for "non-terminating reasoning" — empty completions
# the project chased from 2026-08-07. That diagnosis was wrong, and mecha's own
# CHANGELOG 0.1.2 says so: replaying the quiet prefixes showed the model had
# emitted a tool call *before* closing `</think>`, one of them 120 characters
# long, so llama.cpp filed the entire turn as reasoning. No token budget was
# ever involved, and the fix was harness-side — read `reasoning_content`, and
# stop stripping `<think>` blocks out of the history sent back.
#
# So the flag neither causes nor cures anything known. It is kept because it is
# harmless, because the other local server runs with it, and because an arm
# meant for comparison should differ from its baseline in as few places as
# possible. If you are benchmarking *quality*, A/B it anyway: a budget that is
# too small and a model that gives up early are indistinguishable from outside.
#
# Whatever sets max_tokens against this server must still leave room for an
# answer after the budget — comfortably above 4096. Four numbers move together
# and a mismatch in any of them is silent: `-c` here, `context_window` in
# ~/.mecha/config.toml, `max_tokens` in [agent], and --reasoning-budget.
#
# Start it the way the others are started — a transient unit, not a tmux pane,
# so it survives the terminal and `systemctl --user status` can answer for it:
#
#   systemd-run --user --unit=llama-qwen38 scripts/start-qwen38.sh
#
# Finally, the standing advice from start-moe-mtp.sh applies unchanged and is
# doubly apt for an arm whose whole purpose is comparison: **measure tokens/sec
# after starting this, not just that it came up** — and measure on a quiet
# machine, with 8080 stopped if you want a clean number. Expect it to be
# materially slower than the MoE. Every generated token reads all ~17 GB of
# weights here, against roughly 3B active params there; speculative decoding is
# the thing that might close part of that gap, which is exactly what the draft
# model above is for.
exec ${LLAMA_SERVER:-llama-server} -m "$M" \
  --host 127.0.0.1 --port 8083 -ngl 999 -c 262144 -np 1 --alias qwen3.8-27b --jinja \
  --reasoning-budget 4096 \
  --model-draft "$D" \
  --spec-type draft-mtp \
  --spec-draft-n-max 4
