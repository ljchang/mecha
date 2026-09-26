#!/bin/bash
# The chat models, behind one llama-server in router mode on :8080 — the
# authority on every chat model's flags (REMOTE-SURFACE-DESIGN §14, D12).
#
# One router process; one child llama-server per *loaded* model; the request's
# `model` field picks the child. `--models-max 1` because memory decides it:
# production alone is ~53 GB of a 121 GB pool shared with embeddings and
# ComfyUI, so a pick swaps (≈9 s warm) rather than stacking. Only an idle
# model is ever evicted, so a switch never cuts a reply off.
#
# **The section name IS the model name.** The router overwrites --alias with
# it, and mecha's `[providers.*] model` must equal it — that string is what
# routes a request, so it is also, by construction, what answered.
#
# The INI is generated rather than tracked because a preset cannot glob:
# snapshot directories are content hashes that change on re-download, and the
# paths are this machine's. The flags below are the only copy of the router's
# flags; the reasoning behind each is docs/LLAMA-SERVER.md's and the comments
# in the single-model scripts, which stay as the fallback. Change both.
#
# The embedding server is NOT here and must never be: an embedding request
# must not be able to evict the chat model, nor a chat pick the embedder.
set -euo pipefail
HUB="${HF_HUB:-$HOME/.cache/huggingface/hub}"
source "$(dirname "$0")/mmproj.sh"
PORT="${MECHA_ROUTER_PORT:-8080}"
OUT="${XDG_RUNTIME_DIR:?no XDG_RUNTIME_DIR}/mecha-router/models.ini"
mkdir -p "$(dirname "$OUT")"

snapshot() { ls -d "$HUB"/models--"$1"/snapshots/*/ 2>/dev/null | head -1; }
warn() { echo "$(basename "$0"): $*" >&2; }

# The three Qwens' sampling, as the Qwen model cards give it; temp differs.
# Deliberately not in [*]: Gemma runs on llama-server's defaults, and a shared
# line would silently retune it.
qwen_sampling() {
  printf '%s\n' "temp = $1" "top-p = 0.95" "top-k = 20" "min-p = 0.0" \
    "presence-penalty = 0.0" "repeat-penalty = 1.0" "reasoning-budget = 4096"
}

printf '%s\n' "version = 1" "" "[*]" "n-gpu-layers = 999" "jinja = true" >"$OUT"

# Every projector is resolved into a variable before its heredoc, never inside
# it: a failing $(...) in a heredoc does not trip `set -e`, and would write an
# empty `mmproj =` — a text-only server that looks like a model limitation.

# Production. Required: a router without it answers nothing the triggers ask
# for. -c is DIVIDED across -np (four slots of 262,144), and
# `[providers.local] context_window` must equal c / np.
S=$(snapshot unsloth--Qwen3.6-35B-A3B-MTP-GGUF) || true
[ -n "$S" ] || { warn "production model missing: hf download unsloth/Qwen3.6-35B-A3B-MTP-GGUF"; exit 1; }
MP=$(mmproj_or_die "$S" unsloth/Qwen3.6-35B-A3B-MTP-GGUF)
cat >>"$OUT" <<EOF

[qwen3.6-35b-a3b]
model = $S/Qwen3.6-35B-A3B-UD-Q4_K_M.gguf
mmproj = $MP
ctx-size = ${MECHA_LLAMA_CTX:-1048576}
parallel = ${MECHA_LLAMA_NP:-4}
cache-ram = ${MECHA_LLAMA_CRAM:-32768}
spec-type = draft-mtp
load-on-startup = true
$(qwen_sampling 0.6)
EOF

# The HauhauCS abliteration: production's geometry and sampling, and no MTP
# head in the GGUF (block_count 40, no nextn), so no spec-type — passing it
# fails the child's start. Its projector ships under its own name.
S=$(snapshot HauhauCS--Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive) || true
F="${S}Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q4_K_M.gguf"
MP="${S}mmproj-Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-f16.gguf"
if [ -n "$S" ] && [ -f "$F" ] && [ -f "$MP" ]; then
  cat >>"$OUT" <<EOF

[qwen3.6-35b-a3b-uncensored]
model = $F
mmproj = $MP
ctx-size = ${MECHA_LLAMA_CTX:-1048576}
parallel = ${MECHA_LLAMA_NP:-4}
cache-ram = ${MECHA_LLAMA_CRAM:-32768}
$(qwen_sampling 0.6)
EOF
else
  warn "skipping qwen3.6-35b-a3b-uncensored: weights or projector not on disk"
fi

# Dense 27B: the whole trained window in one slot; MTP is in the file itself.
S=$(snapshot unsloth--Qwen3.8-27B-GGUF) || true
if [ -n "$S" ] && [ -f "${S}Qwen3.8-27B-Q4_K_M.gguf" ] &&
  MP=$(mmproj_or_die "$S" unsloth/Qwen3.8-27B-GGUF 2>/dev/null); then
  cat >>"$OUT" <<EOF

[qwen3.8-27b]
model = ${S}Qwen3.8-27B-Q4_K_M.gguf
mmproj = $MP
ctx-size = 262144
parallel = 1
spec-type = draft-mtp
spec-draft-n-max = 4
$(qwen_sampling 1.0)
EOF
else
  warn "skipping qwen3.8-27b: weights or projector not on disk"
fi

# Gemma's MTP head ships as a separate draft file.
S=$(snapshot unsloth--gemma-4-26B-A4B-it-GGUF) || true
if [ -n "$S" ] && [ -f "${S}gemma-4-26B-A4B-it-UD-Q4_K_M.gguf" ] &&
  MP=$(mmproj_or_die "$S" unsloth/gemma-4-26B-A4B-it-GGUF 2>/dev/null); then
  cat >>"$OUT" <<EOF

[gemma-4-26b-a4b]
model = ${S}gemma-4-26B-A4B-it-UD-Q4_K_M.gguf
mmproj = $MP
ctx-size = 32768
parallel = 1
spec-type = draft-mtp
model-draft = ${S}mtp-gemma-4-26B-A4B-it.gguf
n-gpu-layers-draft = 999
EOF
else
  warn "skipping gemma-4-26b-a4b: weights or projector not on disk"
fi

# **An empty cache, or the router offers every GGUF on the machine.** Presets
# are one of three sources; the Hugging Face cache is another and is always
# on. Measured 2026-09-26: without this, GET /models listed sixteen models —
# the embedders and an MTP-only draft file among them — each loadable by name
# with bare flags (no projector, default context). The children load by
# absolute path and never read the cache, so emptying it costs nothing.
mkdir -p "$(dirname "$OUT")/no-cache"
LLAMA_CACHE="$(dirname "$OUT")/no-cache" \
  exec ${LLAMA_SERVER:-llama-server} --host 127.0.0.1 --port "$PORT" \
  --models-preset "$OUT" --models-max 1
