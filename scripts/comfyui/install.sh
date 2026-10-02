#!/bin/bash
# Install the ComfyUI idle reset (docs/ARCHITECTURE.md §Image generation).
# Copies, never symlinks: a unit must not run a working tree, so what runs is
# what this script last copied (the llama-embed rule of 2026-08-20).
#
#   scripts/comfyui/install.sh          install/refresh, enable the timer
#   scripts/comfyui/install.sh --remove stop and remove what it installed
#
# It touches nothing of comfyui.service itself.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$HOME/.local/bin"
units="$HOME/.config/systemd/user"

if [ "${1:-}" = "--remove" ]; then
  systemctl --user disable --now mecha-comfyui-idle-reset.timer 2>/dev/null || true
  rm -f "$units/mecha-comfyui-idle-reset.timer" "$units/mecha-comfyui-idle-reset.service" \
        "$bin/comfyui-idle-reset"
  systemctl --user daemon-reload
  echo "removed mecha-comfyui-idle-reset"
  exit 0
fi

# The timer restarts `comfyui.service` as a *user* unit. On a box where
# ComfyUI is a system service it would say "not running" every minute, the
# same words as a stopped server, so a wrong install is refused here instead.
systemctl --user cat comfyui.service >/dev/null 2>&1 || {
  echo "comfyui.service is not a user unit here; the idle reset restarts it with systemctl --user" >&2
  exit 1
}

mkdir -p "$bin" "$units"
install -m 0755 "$here/comfyui-idle-reset" "$bin/comfyui-idle-reset"
install -m 0644 "$here/mecha-comfyui-idle-reset.service" "$units/mecha-comfyui-idle-reset.service"
install -m 0644 "$here/mecha-comfyui-idle-reset.timer" "$units/mecha-comfyui-idle-reset.timer"
systemctl --user daemon-reload
systemctl --user enable --now mecha-comfyui-idle-reset.timer
systemctl --user --no-pager list-timers mecha-comfyui-idle-reset.timer | sed -n 1,3p
