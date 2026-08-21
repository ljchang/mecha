#!/bin/bash
S=$(ls -d ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--gemma-4-26B-A4B-it-GGUF/snapshots/*/)
source "$(dirname "$0")/mmproj.sh"
MMPROJ=$(mmproj_or_die "$S" unsloth/gemma-4-26B-A4B-it-GGUF)
# -np 1: see start-moe-mtp.sh — 4 default slots would quarter the context
# to 8192 per request, and the nightly validate's judge runs 16384-token
# verdict budgets that cannot fit in that.
# **A vision model is two files, and this one is multimodal.** The weights
# carry the language model; the vision tower ships beside them as a separate
# mmproj-*.gguf. Without --mmproj the server loads, answers, reports
# modalities.vision:false, and a model asked about a screenshot says it cannot
# see images -- which reads as a limitation of the model rather than a flag
# nobody passed. --mmproj-auto is enabled by default and only fires for -hf
# downloads, so it does nothing for any script here, all of which use -m.
# `mecha`'s startup preflight reads GET /props and warns in both directions;
# see docs/LLAMA-SERVER.md.
exec ${LLAMA_SERVER:-llama-server} -m "$S/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf" \
  --mmproj "$MMPROJ" \
  --host 127.0.0.1 --port 8082 -ngl 999 -c 32768 -np 1 --alias gemma-4-26b-a4b --jinja \
  --spec-type draft-mtp -md "$S/mtp-gemma-4-26B-A4B-it.gguf" -ngld 999
