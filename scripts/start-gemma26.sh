#!/bin/bash
S=$(ls -d /home/ljchang/.cache/huggingface/hub/models--unsloth--gemma-4-26B-A4B-it-GGUF/snapshots/*/)
exec /home/ljchang/.local/bin/llama-server -m "$S/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf" \
  --host 127.0.0.1 --port 8082 -ngl 999 -c 32768 --alias gemma-4-26b-a4b --jinja \
  --spec-type draft-mtp -md "$S/mtp-gemma-4-26B-A4B-it.gguf" -ngld 999
