#!/bin/bash
# Resolve a multimodal projector, and say what to do when it is missing.
#
# Sourced by every start script that serves a vision model. It exists because
# the failure it prevents is invisible: a multimodal model started without
# --mmproj loads, answers, reports modalities.vision:false, and tells anyone
# who sends it a screenshot that it cannot see images. That reads as a
# limitation of the model. It is a flag nobody passed.
#
# Four of four multimodal models on this machine were being served that way on
# 2026-08-21, and one of them had had its projector sitting on disk, unused,
# since the day it was downloaded -- which is why this is a shared function and
# not a line copied into each script.
#
#   mmproj_or_die <snapshot-dir> <hf-repo>
#
# Echoes the projector path, or exits with the command that fetches it. Dying
# is deliberate and is the whole point: starting anyway is what produced a
# month of a model quietly having no eyes. `--no-mmproj` is the escape hatch
# for someone who genuinely wants the text-only arm, and it is explicit.
mmproj_or_die() {
  local snapshot="$1" repo="$2"
  local found
  # BF16 first, then F16 -- unsloth ships both and they are the same size;
  # F32 is twice the memory for a tower whose precision is not the bottleneck.
  for p in "$snapshot/mmproj-BF16.gguf" "$snapshot/mmproj-F16.gguf"; do
    [ -f "$p" ] && { echo "$p"; return 0; }
  done
  cat >&2 <<EOF
$(basename "$0"): this model is multimodal and its vision tower is not on disk.

The weights are one file and the projector is another. Starting without it
gives a server that reports modalities.vision:false and a model that says it
cannot see images -- which looks like the model's limitation and is not.

Fetch it:

  S=\$(ls -d "$snapshot")
  curl -L --fail -o "\$S/mmproj-BF16.gguf" \\
    "https://huggingface.co/$repo/resolve/main/mmproj-BF16.gguf"

Or start deliberately text-only by adding --no-mmproj to this script.
EOF
  exit 1
}
