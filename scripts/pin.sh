# Sourced by ruminate.sh, frontdoor.sh and learn-live.sh: the provider a
# scheduled script passes, as the array PIN (empty means no `-p`).
#
#   scheduled_pin "$MECHA_<SCRIPT>_PROVIDER"
#
# - **Set:** that entry. The operator chose it, and it pins.
# - **Unset, and the config's default provider follows the router**
#   (`follow_loaded`): no `-p`, so the run takes whatever the router has
#   loaded and the record names it (owner's ruling, REMOTE-SURFACE-DESIGN
#   §14). On a router a pin is a load: on 2026-09-27 a `-p local` default
#   pulled production over the comparison arm at 03:30:14.
# - **Unset otherwise:** `-p local`, as before the router. A default that is
#   a paid API must not start billing nightly, hourly and at every session
#   close because this line changed (found on review of #346); an install
#   with no `local` entry fails loudly, as it always did.
#
# The config is read from `/`, where no project `mecha.toml` can layer in —
# learn-live.sh starts in the closing session's workspace. A config that
# cannot be read, or no python3, reads as "does not follow": the old pin,
# which runs locally or fails loudly, never on the default.
#
# Needs $MECHA set. Callers expand PIN as ${PIN[@]+"${PIN[@]}"}: an empty
# array under `set -u` is an unbound variable on bash older than 4.4.
scheduled_pin() {
    PIN=()
    if [ -n "${1:-}" ]; then
        PIN=(-p "$1")
        return 0
    fi
    if (cd / && "$MECHA" config show 2>/dev/null) | python3 -c '
import sys, tomllib
c = tomllib.loads(sys.stdin.read())
entry = c.get("providers", {}).get(c.get("default_provider", ""), {})
sys.exit(0 if entry.get("follow_loaded") is True else 1)
' 2>/dev/null; then
        return 0
    fi
    PIN=(-p local)
}
