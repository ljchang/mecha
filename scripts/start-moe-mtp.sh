#!/bin/bash
M=$(ls /home/ljchang/.cache/huggingface/hub/models--unsloth--Qwen3.6-35B-A3B-MTP-GGUF/snapshots/*/Qwen3.6-35B-A3B-UD-Q4_K_M.gguf)
exec /home/ljchang/.local/bin/llama-server -m "$M" \
  --host 127.0.0.1 --port 8080 -ngl 999 -c 32768 --alias qwen3.6-35b-a3b --jinja \
  --spec-type draft-mtp
