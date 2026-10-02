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

# A file of a Hugging Face repo, from whichever snapshot holds it — newest
# first. **Not "the first snapshot, then the file":** a repo gains a snapshot
# every time a new file is fetched from a newer revision, and a file already
# on disk stays in the old one. Found 2026-09-26, when fetching unsloth's
# UD-Q4_K_XL put it in a second snapshot and the first-snapshot lookup lost
# the Q4_K_M and its projector beside it (the qwen3.8-27b preset vanished).
#
# Prints nothing — and **still succeeds** — when no snapshot holds the file:
# every caller is a bare `F=$(hub_file …)` under `set -euo pipefail`, where a
# non-zero helper would end the script silently, before any `warn` could say
# why (found on review). The explicit `return 0` is what guards it: the loop
# below ends on a failed `read`, and the first version's `ls | head` pipeline
# carried ls's exit 2 on an unmatched glob. And only a file that is really
# there counts: snapshot entries are symlinks into blobs/, and a pruned blob
# leaves one dangling, which `ls` still lists.
#
# Newest-first also means a repo that re-uploads a file under the same name
# (a re-quant) is served at the next restart under an unchanged alias, with
# nothing said — for production as for the rest. The served file is in each
# model's `/props` (`model_path`) when a run needs to know which.
hub_file() {
  local f
  # Read line by line rather than word-split, so a space in HF_HUB is a path,
  # not two (found on review).
  while IFS= read -r f; do
    [ -f "$f" ] && { echo "$f"; return 0; }
  done < <(ls -t "$HUB"/models--"$1"/snapshots/*/"$2" 2>/dev/null)
  return 0
}

# The newest snapshot holding BOTH files, as real files — for a pair that must
# come from one revision (Gemma's weights and their MTP draft). Not "the
# newest weights, then the draft beside them": weights re-fetched into a newer
# snapshot would orphan a matched pair still sitting in an older one (found on
# review). Prints the snapshot directory, or nothing; always succeeds.
hub_pair() {
  local d
  while IFS= read -r d; do
    [ -f "$d$2" ] && [ -f "$d$3" ] && { echo "$d"; return 0; }
  done < <(ls -dt "$HUB"/models--"$1"/snapshots/*/ 2>/dev/null)
  return 0
}

# A repo's vision projector, as mmproj.sh names them (BF16 first); when none is
# on disk, mmproj_or_die's message — with the download line — and a failure.
#
# Searched across snapshots independently of the weights, **on purpose**: a
# repo re-uploads its weights without re-uploading an unchanged projector, so
# pinning the projector to the weights' snapshot would lose vision on exactly
# the upgrade it should survive. unsloth's Qwen3.8 is that case: the
# UD-Q4_K_XL sits in the newer snapshot, the projector (unchanged since
# 2026-08-14) only in the older one. The draft head is the exception, paired
# by hub_pair, because a draft from another revision mis-drafts silently.
hub_mmproj() {
  local d
  while IFS= read -r d; do
    mmproj_in "$d" && return 0
  done < <(ls -dt "$HUB"/models--"$1"/snapshots/*/ 2>/dev/null)
  # The newest snapshot is where the fetch line it prints should write; a
  # caller only gets here after finding the weights, so there is one.
  mmproj_or_die "$(ls -dt "$HUB/models--$1"/snapshots/*/ 2>/dev/null | head -1 || true)" "${1/--//}"
}
warn() { echo "$(basename "$0"): $*" >&2; }

# Every Qwen preset's sampling, as the Qwen model cards give it; temp differs.
# Deliberately not in [*]: Gemma runs on llama-server's defaults, and a shared
# line would silently retune it.
qwen_sampling() {
  printf '%s\n' "temp = $1" "top-p = 0.95" "top-k = 20" "min-p = 0.0" \
    "presence-penalty = 0.0" "repeat-penalty = 1.0" "reasoning-budget = 4096" \
    "reasoning-preserve = true"
}
# **reasoning-preserve is set, never left to the build** (owner's ruling,
# 2026-09-26). It decides whether earlier turns' thinking stays in the prompt,
# and llama.cpp flipped its default under us: at c841aee it deferred to the
# template, from #28174 on it is enabled. Qwen3.6's template drops old thinking
# unless told otherwise (`preserve_thinking is defined and … true`), so the
# flip would have changed production's prompts silently at the install; the
# Qwen3.8 templates keep it by default (`is undefined or … true`), so this
# changes nothing there. Kept: mecha sends reasoning_content back, and keeping
# it makes each prompt a prefix of the next — the cached prefix survives a new
# user turn instead of re-reading from the first dropped block — at the cost
# of context that fills sooner. Gemma's template has no such rule. Persona
# chats send no earlier reasoning back at all (`PriorThinking::Drop`), so for
# them this flag renders nothing; it governs the assistant's runs.
#
# A Qwen projector's image-token floor. Qwen's own default lets an image
# shrink to 8 tokens (clip.cpp `set_limit_image_tokens(8, 4096)`), and the
# build warns at every load that grounding — bounding boxes — needs at least
# 1024: with small images Qwen3-VL placed boxes wrongly, and raising the floor
# "significantly improves the results" (llama.cpp #16842, fixed by #16878's
# smart resize). mecha's own cap (image.rs, 1568 px long edge) already puts a
# full screenshot at ~1100–2400 tokens, so the floor only lifts images under
# about a megapixel — at most ~+1000 tokens each, 0.4% of a 262,144-token
# slot. No max: mecha's largest image is under the default 4096. Gemma sizes
# its own images (70–1120 tokens) and never warns, so it gets neither line.
# Both builds take the flag (asked of `--help` on 95887577 and c841aee).
qwen_vision() {
  printf '%s\n' "image-min-tokens = 1024"
}

# Both builds take the flag — asked of the binaries, not the changelog, on
# 2026-09-26: the installed c841aee's `--help` lists `--reasoning-preserve,
# --no-reasoning-preserve … (default: template default)`, 95887577's the same
# with "(default: enabled)". So it is safe in the single-model rollbacks
# (start-moe-mtp.sh, start-qwen38.sh) on either build.

printf '%s\n' "version = 1" "" "[*]" "n-gpu-layers = 999" "jinja = true" >"$OUT"

# Every projector is resolved into a variable before its heredoc, never inside
# it: a failing $(...) in a heredoc does not trip `set -e`, and would write an
# empty `mmproj =` — a text-only server that looks like a model limitation.

# Production. Required: a router without it answers nothing the triggers ask
# for. -c is DIVIDED across -np (four slots of 262,144), and
# `[providers.local] context_window` must equal c / np.
F=$(hub_file unsloth--Qwen3.6-35B-A3B-MTP-GGUF Qwen3.6-35B-A3B-UD-Q4_K_M.gguf)
[ -n "$F" ] || { warn "production model missing: hf download unsloth/Qwen3.6-35B-A3B-MTP-GGUF Qwen3.6-35B-A3B-UD-Q4_K_M.gguf"; exit 1; }
MP=$(hub_mmproj unsloth--Qwen3.6-35B-A3B-MTP-GGUF)
cat >>"$OUT" <<EOF

[qwen3.6-35b-a3b]
model = $F
mmproj = $MP
$(qwen_vision)
ctx-size = ${MECHA_LLAMA_CTX:-1048576}
parallel = ${MECHA_LLAMA_NP:-4}
cache-ram = ${MECHA_LLAMA_CRAM:-16384}
spec-type = draft-mtp
load-on-startup = true
$(qwen_sampling 0.6)
EOF

# The HauhauCS abliteration: production's geometry and sampling, and no MTP
# head in the GGUF (block_count 40, no nextn), so no spec-type — passing it
# fails the child's start. Its projector ships under its own name.
R=HauhauCS--Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive
F=$(hub_file "$R" Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-Q4_K_M.gguf)
MP=$(hub_file "$R" mmproj-Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-f16.gguf)
if [ -n "$F" ] && [ -n "$MP" ]; then
  cat >>"$OUT" <<EOF

[qwen3.6-35b-a3b-uncensored]
model = $F
mmproj = $MP
$(qwen_vision)
ctx-size = ${MECHA_LLAMA_CTX:-1048576}
parallel = ${MECHA_LLAMA_NP:-4}
cache-ram = ${MECHA_LLAMA_CRAM:-16384}
$(qwen_sampling 0.6)
EOF
else
  warn "skipping qwen3.6-35b-a3b-uncensored: weights or projector not on disk"
fi

# Dense 27B: the whole trained window in one slot; MTP is in the file itself.
# Every Qwen3.8-27B preset shares this geometry, the thinking-mode sampling and
# the in-file MTP head (block_count 65, blk.64.nextn.*, checked in each GGUF's
# header before it was added). `qwen38 NAME FILE PROJECTOR` writes one.
qwen38() {
  cat >>"$OUT" <<EOF

[$1]
model = $2
mmproj = $3
$(qwen_vision)
ctx-size = 262144
parallel = 1
spec-type = draft-mtp
spec-draft-n-max = 4
$(qwen_sampling 1.0)
EOF
}

# Measured 2026-09-26 on llama.cpp 95887577, through a test router, one
# stream, 400 tokens, same prompt and flags for all four rows below
# (docs/LLAMA-SERVER.md §Router mode):
#
#   file                          decode       MTP draft acceptance
#   unsloth Q4_K_M (withdrawn)    21.1 tok/s   0.38
#   unsloth UD-Q4_K_XL            22.9 tok/s   0.45
#   HauhauCS Q4_K_P (uncensored)  26.8 tok/s   0.57
#   huihui UD-Q4_K_XL (abliter.)  22.1 tok/s   0.42
#
# All four read an image and answered a reasoning check correctly.
#
# The official model: unsloth's "Dynamic 3.0" UD-Q4_K_XL — the same chat
# template, MTP in the file, a 1251-chunk imatrix against the Q4_K_M's 45.
# unsloth deleted the plain Q4_K_M upstream on 2026-08-19, so it is only a
# fallback for a machine that still has it; the projector is unchanged since
# 2026-08-14 and serves both.
R=unsloth--Qwen3.8-27B-GGUF
F=$(hub_file "$R" Qwen3.8-27B-UD-Q4_K_XL.gguf)
FALLBACK=""
if [ -z "$F" ]; then
  F=$(hub_file "$R" Qwen3.8-27B-Q4_K_M.gguf)
  [ -n "$F" ] && FALLBACK=1
fi
if [ -n "$F" ] && MP=$(hub_mmproj "$R"); then
  qwen38 qwen3.8-27b "$F" "$MP"
  # Said only once the preset is really written, so a missing projector
  # never reads as "served" and "skipped" in one run (found on review).
  if [ -n "$FALLBACK" ]; then
    warn "qwen3.8-27b: UD-Q4_K_XL not on disk, serving the withdrawn Q4_K_M — hf download unsloth/Qwen3.8-27B-GGUF Qwen3.8-27B-UD-Q4_K_XL.gguf"
  fi
else
  warn "skipping qwen3.8-27b: weights or projector not on disk"
fi

# Two uncensored builds of the same model, kept side by side to compare
# (owner's call, 2026-09-26). Both keep the MTP head and ship a projector.
#
# HauhauCS "Aggressive": method undisclosed (the maker of the Qwen3.6
# uncensored arm above); users report its reasoning forced into English.
R=HauhauCS--Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF
F=$(hub_file "$R" Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf)
MP=$(hub_file "$R" mmproj-Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-BF16.gguf)
if [ -n "$F" ] && [ -n "$MP" ]; then
  qwen38 qwen3.8-27b-uncensored "$F" "$MP"
else
  warn "skipping qwen3.8-27b-uncensored: weights or projector not on disk"
fi

# huihui-ai: abliteration (refusal-direction ablation, layers 17-52) over
# unsloth's UD quant; MTP and vision untouched, per its README and its header.
R=huihui-ai--Huihui-Qwen3.8-27B-abliterated-GGUF
F=$(hub_file "$R" Huihui-Qwen3.8-27B-abliterated-UD-Q4_K_XL.gguf)
MP=$(hub_file "$R" mmproj-model-bf16.gguf)
if [ -n "$F" ] && [ -n "$MP" ]; then
  qwen38 qwen3.8-27b-abliterated "$F" "$MP"
else
  warn "skipping qwen3.8-27b-abliterated: weights or projector not on disk"
fi

# Gemma's MTP head ships as a separate draft file.
R=unsloth--gemma-4-26B-A4B-it-GGUF
# Weights and MTP draft from one snapshot, never two revisions: a draft head
# from a different revision than its weights is a worse mismatch than a
# projector (found on review).
P=$(hub_pair "$R" gemma-4-26B-A4B-it-UD-Q4_K_M.gguf mtp-gemma-4-26B-A4B-it.gguf)
F="${P:+${P}gemma-4-26B-A4B-it-UD-Q4_K_M.gguf}"
D="${P:+${P}mtp-gemma-4-26B-A4B-it.gguf}"
if [ -n "$F" ] && [ -n "$D" ] && MP=$(hub_mmproj "$R"); then
  cat >>"$OUT" <<EOF

[gemma-4-26b-a4b]
model = $F
mmproj = $MP
ctx-size = 32768
parallel = 1
spec-type = draft-mtp
model-draft = $D
n-gpu-layers-draft = 999
EOF
else
  warn "skipping gemma-4-26b-a4b: weights, MTP draft or projector not on disk"
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
