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

# MTP is IN THE FILE, exactly like the 3.6 MoE — an earlier version of this
# script was wrong about that, and the correction is worth its history.
#
# This script originally loaded a4lg/Qwen3.8-27B-MTP-ONLY-GGUF as a separate
# `--model-draft` (2.03 GB), on the strength of that repo's README: standard
# GGUF conversions drop the MTP tensors, here is the extracted head. True of
# the conversions it was written for — fine-tunes, abliterations — but not of
# unsloth's: inspected 2026-08-15 with gguf-py, Qwen3.8-27B-Q4_K_M already
# carries all fifteen blk.64.* tensors and declares block_count=65 and
# nextn_predict_layers=1. Every tensor in the a4lg file is already in the
# main one. The graft the README calls Method 2 had nothing to graft.
#
# Proof it works from the single file, because the server logs never mention
# speculation either way: with no --model-draft, timings still report
# draft_n/draft_n_accepted (86/128 on the smoke prompt). Those fields are the
# only observable evidence — check them, not the log, after touching this.
#
# The a4lg download can be deleted from the HF cache; nothing uses it now.
# If a future quant of this model really does lack blk.64.* (llama-server
# would fail to start with --spec-type draft-mtp, or timings would carry no
# draft stats), that repo and its Method 1 are the remedy — see its README.

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
# possible.
#
# It is also, on this model, INERT — measured 2026-08-14. Qwen3.8-27B does not
# over-think: on a deliberately hard riddle it spent ~230 tokens of thinking,
# and even a 200-token budget never fired. The flag only bites if you set it
# absurdly low; at 20 it does work exactly as documented, cutting the trace and
# injecting --reasoning-budget-message inline, after which the model closes the
# think block and still answers correctly. Semantics, from --help: -1
# unrestricted (the llama.cpp default), 0 immediate end, N>0 a real token cap.
#
# **The lever that actually moves this model is not this flag.** Qwen3.8's chat
# template takes two variables, and llama-server exposes them per request under
# `chat_template_kwargs` (NOT as top-level OpenAI fields — a top-level
# `reasoning_effort` or `enable_thinking` is silently ignored, verified):
#
#   chat_template_kwargs: {"reasoning_effort": "low"|"medium"|"xhigh"}
#   chat_template_kwargs: {"enable_thinking": false}
#
# `reasoning_effort` defaults to xhigh ('high' is aliased to it). It is a
# PROMPT, not a cap: xhigh injects "think carefully… validate key assumptions…
# consider plausible alternatives", low injects "keep your thinking brief", and
# medium injects nothing at all. Which is why the measured effect is not what
# the names promise — median thinking over 3 seeds on one hard prompt:
#
#   xhigh (default)   653 chars    374 out tok   16.1 s   <- shortest, fastest
#   low             1,038 chars    589 out tok   21.8 s
#   medium          1,176 chars    697 out tok   26.3 s
#   thinking off        0 chars    667 out tok   25.2 s
#
# Two things worth keeping from that. The default is the fastest setting, so
# do not "optimize" it downward without measuring. And turning thinking OFF did
# not save time — the tokens simply moved from the think block into a longer
# answer. One prompt, n=3, so treat it as a caution against assuming, not as a
# result about the model.
#
# None of those kwargs are reachable from mecha today: ProviderConfig carries
# no extra-body passthrough, so a mecha run gets whatever this server was
# started with. If reasoning control ever needs to be per-run rather than
# per-server, that passthrough is the change to make. For the record, the
# per-request budget field does work here (`reasoning_budget_tokens`, upstream
# #23116) — with one trap: it defaults to -1, and -1 means "defer to the server
# flag", so a request CANNOT ask for unlimited thinking against a server
# started with a finite budget. The flag is a ceiling, not a default.
#
# Two upstream notes, both verified against this server on 2026-08-14.
#
# **Do not "fix" tool-call trouble with --reasoning-format none.** That is the
# workaround in ggml-org/llama.cpp#20837 — still OPEN, filed against the same
# `qwen35` architecture this model reuses — for tool calls emitted inside the
# thinking block, which is precisely the malformation behind mecha's old
# empty-turn bug. It is the wrong medicine here: with format `none` the server
# stops splitting thoughts out, `reasoning_content` comes back null, and the
# raw `<think>…</think>` lands in `message.content`. mecha decodes
# `reasoning_content` into `Block::Thinking` and replays it (openai.rs), so
# that setting would both show thinking as the answer and disable the replay
# that CHANGELOG 0.1.2 measured at 7/7 → 0/6. Leave the format alone.
#
# **The template hard-rejects a system message that is not first**, raising
# `System message must be at the beginning.` (line 110) — the server answers
# HTTP 400, reproduced here, not a warning. It has bitten Claude Code
# (llama.cpp discussion #27081) and ollama (#17757), both of which inject
# system turns mid-conversation. mecha is structurally immune: `Role` is only
# `User | Assistant`, and the system prompt is a separate `req.system` field
# pushed ahead of every message. Worth knowing before anything is tempted to
# add a mid-conversation system reminder to this backend.
#
# Finally, `--chat-template-kwargs enable_thinking=...` is deprecated upstream
# in favour of `--reasoning on|off`; that is the flag to reach for if this arm
# should ever run without thinking at all.
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
# Sampling is Qwen's published recipe for THINKING mode, not llama.cpp's
# defaults, and three of the six disagree — so the defaults are wrong here
# rather than merely different:
#
#                 llama.cpp default   Qwen3.8 thinking
#   temperature              0.80              1.0
#   top_k                      40               20
#   min_p                    0.05              0.0   (i.e. disabled)
#   top_p                    0.95             0.95
#   presence_penalty         0.00              0.0
#   repeat_penalty           1.00              1.0
#
# min_p is the one to notice: llama.cpp truncates the tail by default and Qwen
# asks for that off entirely. Qwen's non-thinking recipe differs (temp 0.7,
# top_p 0.80, presence_penalty 1.5) — if this arm is ever run with
# `--reasoning off`, those are the numbers, and the model card warns that a
# high presence_penalty can cause language mixing.
#
# These are server DEFAULTS, which is the only place most of them can live:
# mecha's ProviderConfig carries `temperature` and `seed` and nothing else, so
# top_k/top_p/min_p/penalties have no path through a request and a mecha run
# would silently get llama.cpp's defaults instead. Note the consequence for
# temperature specifically — mecha *does* send it, so `[providers.qwen38]
# temperature` OVERRIDES the value below. The two are pinned to 1.0 together
# and must move together.
exec ${LLAMA_SERVER:-llama-server} -m "$M" \
  --host 127.0.0.1 --port 8083 -ngl 999 -c 262144 -np 1 --alias qwen3.8-27b --jinja \
  --reasoning-budget 4096 \
  --temp 1.0 --top-p 0.95 --top-k 20 --min-p 0.0 \
  --presence-penalty 0.0 --repeat-penalty 1.0 \
  --spec-type draft-mtp \
  --spec-draft-n-max 4
