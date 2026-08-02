#!/bin/bash
S=$(ls -d /home/ljchang/.cache/huggingface/hub/models--unsloth--gemma-4-E4B-it-qat-GGUF/snapshots/*/)
exec /home/ljchang/.local/bin/llama-server -m "$S/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf" \
  --host 127.0.0.1 --port 8081 -ngl 999 -c 16384 --alias gemma-4-e4b --jinja \
  --spec-type draft-mtp -md "$S/mtp-gemma-4-E4B-it.gguf" -ngld 999
