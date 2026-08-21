#!/bin/bash
S=$(ls -d ${HF_HUB:-$HOME/.cache/huggingface/hub}/models--unsloth--gemma-4-E4B-it-qat-GGUF/snapshots/*/)
source "$(dirname "$0")/mmproj.sh"
MMPROJ=$(mmproj_or_die "$S" unsloth/gemma-4-E4B-it-qat-GGUF)
# **A vision model is two files, and this one is multimodal.** The weights
# carry the language model; the vision tower ships beside them as a separate
# mmproj-*.gguf. Without --mmproj the server loads, answers, reports
# modalities.vision:false, and a model asked about a screenshot says it cannot
# see images -- which reads as a limitation of the model rather than a flag
# nobody passed. --mmproj-auto is enabled by default and only fires for -hf
# downloads, so it does nothing for any script here, all of which use -m.
# `mecha`'s startup preflight reads GET /props and warns in both directions;
# see docs/LLAMA-SERVER.md.
exec ${LLAMA_SERVER:-llama-server} -m "$S/gemma-4-E4B-it-qat-UD-Q4_K_XL.gguf" \
  --mmproj "$MMPROJ" \
  --host 127.0.0.1 --port 8081 -ngl 999 -c 16384 --alias gemma-4-e4b --jinja \
  --spec-type draft-mtp -md "$S/mtp-gemma-4-E4B-it.gguf" -ngld 999
