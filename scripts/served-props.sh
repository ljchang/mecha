# Source this. Two questions about the llama-server at BASE, asked so that a
# router (REMOTE-SURFACE-DESIGN §14) answers them honestly:
#
#   served_model BASE          the model it is serving: a single-model server's
#                              `model_alias`, a router's one resident model
#   served_props BASE [MODEL]  that model's `/props` (MODEL, when named)
#
# Each returns non-zero, with a line on stderr, when it cannot say.
#
# The router traps these exist for (docs/LLAMA-SERVER.md §Router mode): a bare
# `/props` is a placeholder (`model_alias: "llama-server"`, `n_ctx: 0`, no
# `total_slots`); a `/props?model=…` without `autoload=false` *loads* that
# model, so a benchmark's preflight would swap out the owner's pick; and a
# request with no `model` field is refused outright.

# The server root, however the URL was spelled.
_served_base() {
    local base="${1%/}"
    printf '%s' "${base%/v1}"
}

# The bare /props answer, and whether it came from a router.
_served_bare() {
    curl -s -m 5 "$(_served_base "$1")/props"
}

_served_role() {
    python3 -c 'import json, sys; print(json.load(sys.stdin).get("role", ""))' 2>/dev/null
}

served_model() {
    local base props
    base="$(_served_base "$1")"
    props="$(_served_bare "$base")" || {
        echo "served_model: nothing answered at $base/props" >&2
        return 1
    }
    if [ "$(printf '%s' "$props" | _served_role)" != router ]; then
        # An empty name is not an answer: every request would carry it as the
        # model, and a scorecard would be filed under it (found on review).
        local alias
        alias="$(printf '%s' "$props" | python3 -c 'import json, sys; print(json.load(sys.stdin).get("model_alias") or "")' 2>/dev/null)"
        if [ -z "$alias" ]; then
            echo "served_model: $base/props names no model_alias — nothing to call it" >&2
            return 1
        fi
        printf '%s' "$alias"
        return 0
    fi
    # The resident model, only from a list this fully reads: "nothing loaded"
    # or "two loaded" is not an answer to "what is served". "loading" counts
    # as resident, as in `RouterModel::is_resident` and model-idle.sh — mid-
    # eviction (A sleeping, B loading) that is two, not A (found on review).
    curl -s -m 5 "$base/models" | python3 -c '
import json, sys
d = json.load(sys.stdin).get("data")
known = {"unloaded", "loading", "loaded", "sleeping", "downloading"}
if not isinstance(d, list) or not d or any(m.get("status", {}).get("value") not in known for m in d):
    raise SystemExit(2)
r = [m["id"] for m in d if m["status"]["value"] in ("loaded", "loading", "sleeping")]
if len(r) != 1:
    raise SystemExit(3)
print(r[0])
' 2>/dev/null || {
        echo "served_model: cannot tell which model the router at $base has loaded — \`mecha model list\`" >&2
        return 1
    }
}

served_props() {
    local base model="${2:-}" props
    base="$(_served_base "$1")"
    props="$(_served_bare "$base")" || {
        echo "served_props: nothing answered at $base/props" >&2
        return 1
    }
    if [ "$(printf '%s' "$props" | _served_role)" != router ]; then
        printf '%s' "$props"
        return 0
    fi
    if [ -z "$model" ]; then
        model="$(served_model "$base")" || return 1
    fi
    props="$(curl -s -m 5 -w '\n%{http_code}' -G "$base/props" \
        --data-urlencode "model=$model" --data-urlencode "autoload=false")"
    case "${props##*$'\n'}" in
        200) ;;
        000)
            echo "served_props: the router at $base did not answer" >&2
            return 1
            ;;
        *)
            echo "served_props: the router at $base is not serving $model — \`mecha model use $model\` first" >&2
            return 1
            ;;
    esac
    printf '%s' "${props%$'\n'*}"
}
