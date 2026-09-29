#!/bin/bash
# Install the layout stage of document OCR (docs/DOCUMENT-EXTRACTION-DESIGN.md
# §5): PP-DocLayoutV3's ONNX export and a Python environment with ONNX
# Runtime to run it, under ~/.mecha/layout/ ($MECHA_HOME/layout/):
#
#   venv/                  onnxruntime + numpy, every file pinned by hash
#   PP-DocLayoutV3.onnx    -> the Hugging Face cache's copy, revision pinned,
#                             sha256 checked
#
# Nothing here runs as a service or stays resident: mecha starts the worker,
# confined, for one extraction and kills it after. `[documents] layout_python`
# and `layout_model` point elsewhere if you keep these elsewhere.
#
#   scripts/layout/install.sh            install (idempotent)
#   scripts/layout/install.sh --remove   remove the directory (the model stays
#                                        in the Hugging Face cache)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dir="${MECHA_HOME:-$HOME/.mecha}/layout"

# The publisher's own ONNX export (Apache-2.0), at the revision measured.
repo="PaddlePaddle/PP-DocLayoutV3_onnx"
rev="46bbdf188bb0a772c08aed74882ce7e51a8f1ea6"
sha="45bf71750b00739a41fc209f132eb104a4d6b5bb29483c9078164d8b87cf28ba"

if [ "${1:-}" = "--remove" ]; then
  rm -rf "$dir"
  echo "removed $dir"
  exit 0
fi

mkdir -p "$dir"
chmod 700 "$dir"

# The environment. The system interpreter, so the confinement already has it
# read-only (/usr); a venv on another interpreter works too — mecha binds the
# interpreter's prefix — but there is no reason to.
python="${MECHA_LAYOUT_PYTHON:-/usr/bin/python3}"
if [ ! -x "$dir/venv/bin/python" ]; then
  if command -v uv >/dev/null; then
    uv venv --quiet --python "$python" "$dir/venv"
  else
    "$python" -m venv "$dir/venv"
  fi
fi
if command -v uv >/dev/null; then
  uv pip install --quiet --python "$dir/venv/bin/python" --require-hashes -r "$here/requirements.txt"
else
  "$dir/venv/bin/python" -m pip install --quiet --require-hashes -r "$here/requirements.txt"
fi

# The model.
if ! command -v hf >/dev/null; then
  echo "install the Hugging Face CLI (hf) first, or download $repo@$rev by hand" >&2
  exit 1
fi
path="$(hf download "$repo" inference.onnx --revision "$rev" --quiet 2>/dev/null || hf download "$repo" inference.onnx --revision "$rev")"
path="$(echo "$path" | tail -1)"
got="$(sha256sum "$path" | cut -d' ' -f1)"
if [ "$got" != "$sha" ]; then
  echo "the downloaded model's sha256 is $got, not $sha — refusing it" >&2
  exit 1
fi
ln -sfn "$path" "$dir/PP-DocLayoutV3.onnx"

# Prove it loads — the way mecha runs it, minus the confinement.
"$dir/venv/bin/python" -I -B -c 'import onnxruntime as ort, sys; ort.InferenceSession(sys.argv[1], providers=["CPUExecutionProvider"]); print("layout model loads: onnxruntime", ort.__version__)' "$dir/PP-DocLayoutV3.onnx"
echo "installed in $dir"
