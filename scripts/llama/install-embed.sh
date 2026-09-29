#!/bin/bash
# Move the embeddings server to on demand (docs/DOCUMENT-EXTRACTION-DESIGN.md
# §6): llama-embed.socket on :8081, a proxy that idles out, and
# llama-embed.service as the backend on :18081. Copies, never symlinks — a
# unit must not run a working tree.
#
#   scripts/llama/install-embed.sh          switch to on demand
#   scripts/llama/install-embed.sh --remove restore the always-on unit it saved
#
# **Do not run it before mecha-graph's `Embedder::available()` tolerates a cold
# start.** Its probe gave up after 1.5 s; a cold start takes ~4 s, and the probe
# gates semantic search per query and the graph MCP server's embedder for its
# whole life (see the design doc). The script refuses unless told otherwise.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
units="$HOME/.config/systemd/user"
bak="$units/llama-embed.service.always-on.bak"

if [ "${1:-}" = "--remove" ]; then
  [ -f "$bak" ] || { echo "no saved always-on unit at $bak" >&2; exit 1; }
  systemctl --user disable --now llama-embed.socket 2>/dev/null || true
  systemctl --user stop llama-embed-proxy.service llama-embed.service 2>/dev/null || true
  rm -f "$units/llama-embed.socket" "$units/llama-embed-proxy.service"
  cp "$bak" "$units/llama-embed.service"
  [ -f "$bin/mecha-embed-server.always-on.bak" ] && cp "$bin/mecha-embed-server.always-on.bak" "$bin/mecha-embed-server"
  systemctl --user daemon-reload
  systemctl --user enable --now llama-embed.service
  echo "restored the always-on llama-embed.service"
  exit 0
fi

if [ "${MECHA_EMBED_COLD_START_OK:-}" != 1 ]; then
  echo "refusing: install the mecha-graph build whose Embedder::available() waits out a cold start, then rerun with MECHA_EMBED_COLD_START_OK=1" >&2
  exit 1
fi

# Save what is running now, once, so --remove can put it back.
[ -f "$bak" ] || cp "$units/llama-embed.service" "$bak"
[ -f "$bin/mecha-embed-server.always-on.bak" ] || cp "$bin/mecha-embed-server" "$bin/mecha-embed-server.always-on.bak"

systemctl --user disable --now llama-embed.service
mkdir -p "$units/llama-embed.service.d"
install -m 0755 "$here/mecha-embed-server" "$bin/mecha-embed-server"
install -m 0755 "$here/mecha-wait-healthy" "$bin/mecha-wait-healthy"
install -m 0644 "$here/llama-embed.socket" "$units/llama-embed.socket"
install -m 0644 "$here/llama-embed-proxy.service" "$units/llama-embed-proxy.service"
install -m 0644 "$here/llama-embed.service" "$units/llama-embed.service"
install -m 0644 "$here/path.conf" "$units/llama-embed.service.d/path.conf"
systemctl --user daemon-reload
systemctl --user enable --now llama-embed.socket
systemctl --user --no-pager status llama-embed.socket | sed -n 1,5p
