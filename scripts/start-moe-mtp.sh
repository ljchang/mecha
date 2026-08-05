#!/bin/bash
M=$(ls ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--Qwen3.6-35B-A3B-MTP-GGUF/snapshots/*/Qwen3.6-35B-A3B-UD-Q4_K_M.gguf)
# -np 1 is load-bearing: newer llama-server defaults to 4 parallel slots,
# which silently splits -c across them — 8192 per request, not the 32768
# that mecha's `context_window` promises. Past 8192 the server context-
# shifts instead of erroring, the model sees a mangled transcript, and the
# symptom is empty completions that look like a model failure. Found
# 2026-08-05 after the empty-EndTurn deaths in the k=5 compaction runs.
# Concurrent eval requests serialize instead; the MoE is fast enough.
exec ${LLAMA_SERVER:-llama-server} -m "$M" \
  --host 127.0.0.1 --port 8080 -ngl 999 -c 32768 -np 1 --alias qwen3.6-35b-a3b --jinja \
  --spec-type draft-mtp
