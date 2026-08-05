#!/bin/bash
S=$(ls -d ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--gemma-4-26B-A4B-it-GGUF/snapshots/*/)
# -np 1: see start-moe-mtp.sh — 4 default slots would quarter the context
# to 8192 per request, and the nightly validate's judge runs 16384-token
# verdict budgets that cannot fit in that.
exec ${LLAMA_SERVER:-llama-server} -m "$S/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf" \
  --host 127.0.0.1 --port 8082 -ngl 999 -c 32768 -np 1 --alias gemma-4-26b-a4b --jinja \
  --spec-type draft-mtp -md "$S/mtp-gemma-4-26B-A4B-it.gguf" -ngld 999
