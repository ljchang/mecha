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
    # The same test as `provider::router::follows_here`: `follow_loaded`, a
    # `local` kind, and a loopback base URL — the flag on anything else is
    # ignored (and warned about) by mecha, so reading it alone here would
    # drop the pin for a run that then follows nothing (found on review).
    #
    # "Does not follow" is quiet: on an install without a router, `-p local`
    # is the ordinary answer. "Cannot tell" — `config show` failed, the TOML
    # will not parse, no python3 — says why on stderr, because on a router
    # box that fallback is the pin that loads a model over the owner's pick,
    # and a silent one reads like a decision (found on review of #346).
    #
    # Stdout alone is parsed: `config show` warns on stderr (an unreadable
    # harness override, `MECHA_LOG=debug`) while printing a good config, and
    # mixing that in reads a router box as unreadable — the load this exists
    # to report (found on review of #360). Stderr is read only on failure, to
    # say why.
    local shown verdict why
    if ! shown="$(cd / && "$MECHA" config show 2>/dev/null)"; then
        why="$(cd / && "$MECHA" config show 2>&1 >/dev/null | tail -n 1)"
        echo "pin.sh: \`mecha config show\` failed${why:+ ($why)}; cannot tell whether the default follows a router, so pinning -p local" >&2
        PIN=(-p local)
        return 0
    fi
    if ! verdict="$(printf '%s' "$shown" | python3 -c '
import ipaddress, sys, tomllib
from urllib.parse import urlsplit
c = tomllib.loads(sys.stdin.read())
entry = c.get("providers", {}).get(c.get("default_provider", ""), {})
def loopback(url):
    try:
        host = urlsplit(url).hostname or ""
    except ValueError:
        return False
    if host.lower() == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False
follows = (
    entry.get("follow_loaded") is True
    and entry.get("kind") == "local"
    and loopback(entry.get("base_url") or "")
)
print("follows" if follows else "no")
' 2>/dev/null)"; then
        why="$(printf '%s' "$shown" | python3 -c 'import sys, tomllib; tomllib.loads(sys.stdin.read())' 2>&1 | tail -n 1)"
        echo "pin.sh: cannot read the config (${why:-python3 missing or failing}); pinning -p local" >&2
        PIN=(-p local)
        return 0
    fi
    [ "$verdict" = follows ] && return 0
    PIN=(-p local)
}
