#!/usr/bin/env bash
# The drain loop: hold a request open on the box, process what lands, repeat.
#
# This is the "instant invite" half of the freshness story (the fast freebusy
# timer is the other). `drain --wait 25` long-polls `GET /v1/queue` — the box
# answers the moment a record lands, or empty at the cap — so a confirmed
# booking reaches home, becomes a calendar event, and mails its provider
# invite seconds after the visitor's click. The connection is initiated here
# every time: the box still never dials home, and this loop is nothing a
# crontab could not do slower.
#
# Deliberately a dumb loop, like the trigger daemon: each iteration is the
# same drain-then-sweep a hand run performs, the sweep is flock-guarded in
# `mecha-mail bookings` itself (the fifteen-minute timer also runs it), and
# the ledger makes re-runs idempotent. A drain failure sleeps and retries —
# the box being down must not turn this into a busy loop.
#
# Install: install -m 0755 scripts/drain-follow.sh ~/.cargo/bin/mecha-drain-follow
#          cp scripts/mecha-drain.service ~/.config/systemd/user/
#          systemctl --user daemon-reload
#          systemctl --user enable --now mecha-drain.service
set -u
PATH="$HOME/.cargo/bin:$PATH"

# Which calendar a booking's event lands on. Empty means "whatever
# `mecha-mail accounts` calls the default" — the answer that stays right
# when the default moves. This was hardcoded to one account, which
# outvoted the registry silently and landed every stranger's meeting on a
# calendar its owner never opens.
ACCOUNT="${MECHA_BOOKINGS_ACCOUNT:-}"
account_args=()
[ -n "$ACCOUNT" ] && account_args=(--account "$ACCOUNT")
POLICY="${MECHA_BOOK_POLICY:-$HOME/.mecha/instruments/book-policy.toml}"

while :; do
  # If the box answers an empty drain instantly — a binary that predates
  # `?wait=`, or a regression in the hold — this must degrade to a paced
  # poll, never a hot loop hammering the gate.
  began=$(date +%s)
  if out=$(factory-publish drain --wait 25 --json 2>&1); then
    drained=$(printf '%s' "$out" | jq -r '.drained // 0' 2>/dev/null || echo 0)
    if [ "${drained:-0}" -eq 0 ] && [ $(($(date +%s) - began)) -lt 5 ]; then
      sleep 5
    fi
    if [ "${drained:-0}" -gt 0 ]; then
      printf '%s\n' "$out"
      # The sweep: events for new bookings (re-verified against live
      # freebusy inside), withdrawals for cancellations. Its own flock
      # serialises this against the timer's copy.
      mecha-mail bookings ${account_args[@]+"${account_args[@]}"} || echo "bookings sweep failed" >&2
      # A created event changed the calendar, so the box's slot cache is
      # now wrong about the buffers around it — refresh immediately rather
      # than waiting out the timer. Failure here degrades to exactly the
      # timer's own staleness, so it only warns.
      mecha-mail freebusy --days 60 --json |
        factory-publish slots push book --policy "$POLICY" ||
        echo "slot refresh after drain failed; the timer will catch it" >&2
    fi
  else
    printf 'drain failed: %s\n' "$out" >&2
    sleep 10
  fi
done
