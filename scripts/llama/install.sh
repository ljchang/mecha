#!/bin/bash
# Install the on-demand OCR model server's units and launchers
# (docs/DOCUMENT-EXTRACTION-DESIGN.md §6). Copies, never symlinks: a unit must
# not run a working tree (the llama-embed rule of 2026-08-20), so what runs is
# what this script last copied, and a checkout switching branches changes
# nothing until it is run again.
#
#   scripts/llama/install.sh          install/refresh, enable the socket
#   scripts/llama/install.sh --remove stop and remove everything it installed
#
# It does not download the model: `hf download PaddlePaddle/PaddleOCR-VL-1.6-GGUF`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
units="$HOME/.config/systemd/user"

if [ "${1:-}" = "--remove" ]; then
  systemctl --user disable --now llama-ocr.socket 2>/dev/null || true
  systemctl --user stop llama-ocr-proxy.service llama-ocr.service 2>/dev/null || true
  rm -f "$units/llama-ocr.socket" "$units/llama-ocr-proxy.service" "$units/llama-ocr.service"
  rm -rf "$units/llama-ocr.service.d"
  rm -f "$bin/mecha-ocr-server"
  systemctl --user daemon-reload
  echo "removed llama-ocr (mecha-wait-healthy left in $bin; other units may use it)"
  exit 0
fi

mkdir -p "$bin" "$units/llama-ocr.service.d"
install -m 0755 "$here/mecha-ocr-server" "$bin/mecha-ocr-server"
install -m 0755 "$here/mecha-wait-healthy" "$bin/mecha-wait-healthy"
install -m 0644 "$here/llama-ocr.socket" "$units/llama-ocr.socket"
install -m 0644 "$here/llama-ocr-proxy.service" "$units/llama-ocr-proxy.service"
install -m 0644 "$here/llama-ocr.service" "$units/llama-ocr.service"
install -m 0644 "$here/path.conf" "$units/llama-ocr.service.d/path.conf"
systemctl --user daemon-reload
systemctl --user enable --now llama-ocr.socket
systemctl --user --no-pager status llama-ocr.socket | sed -n 1,5p
