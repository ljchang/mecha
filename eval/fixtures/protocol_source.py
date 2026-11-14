#!/usr/bin/env python3
"""The task source for the far-analog lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4), on the contract in `source_stub.py`.

    protocol_source.py list | setup <task> | grade <task>

Two experience tasks in the registry, where the documentation says a record
is locked before it is changed and unlocked after. Then two probes at the
quay, a service with other tools, other words and no documentation, where
the same arrangement holds and nothing says so (`protocol_world.py`). Every
task is graded on the service's own state: the value was changed **and** the
thing was given back.

**Held out for the concept work.** Written before any arm that matches by
structure was built, and before any arm was run on it.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import protocol_world as world  # noqa: E402

LEARN_TURNS = 14
# Find, a refused set, up to five operations each checked with a find, the
# set, and up to four more operations each checked: a model that knows the
# berth must be given back can find how. One that does not has no reason to.
PROBE_TURNS = 22

TASKS = [
    {"id": "registry-owner", "tags": ["protocol", "experience"], "max_turns": LEARN_TURNS, "world": "registry",
     "prompt": "Change the owner of the record named 'Marlow ledger' to 'Hale' with the registry tools. The tools have their own documentation; read it if a call is refused.",
     "thing": "R-7", "field": "owner", "value": "Hale"},
    {"id": "registry-tier", "tags": ["protocol", "experience"], "max_turns": LEARN_TURNS, "world": "registry",
     "prompt": "With the registry tools, the record named 'Juniper deed' should have tier 3. If a call is refused, the tools' documentation says why.",
     "thing": "R-12", "field": "tier", "value": 3},
    {"id": "quay-draft", "tags": ["protocol", "probe"], "max_turns": PROBE_TURNS, "world": "quay",
     "prompt": "Using the quay tools, the berth named 'North pier three' needs its draft at 9.",
     "thing": "B-3", "field": "draft", "value": 9},
    {"id": "quay-crane", "tags": ["protocol", "probe"], "max_turns": PROBE_TURNS, "world": "quay",
     "prompt": "The berth named 'South pier one' is to use the 'north' crane. Use the quay tools.",
     "thing": "B-1", "field": "crane", "value": "north"},
]


def folders():
    root = os.environ.get("MECHA_FIXTURES")
    if root and os.path.isdir(root):
        for name in os.listdir(root):
            if os.path.isdir(os.path.join(root, name)):
                yield os.path.join(root, name)


def grade(task):
    kind, flag = ("records", "locked") if task["world"] == "registry" else ("berths", "held")
    for folder in folders():
        path = os.path.join(folder, world.STATE)
        if not os.path.exists(path):
            continue
        with open(path) as f:
            thing = json.load(f)[kind][task["thing"]]
        changed = thing[task["field"]] == task["value"]
        returned = not thing[flag]
        if changed and returned:
            return True, "the value was changed and the thing was given back"
        if changed:
            return False, "the value was changed and the thing was left taken"
        return False, "the value was not changed"
    return False, "the service kept no state"


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    if verb == "list":
        json.dump([{k: t[k] for k in ("id", "tags", "max_turns", "prompt")} for t in TASKS], sys.stdout)
        return 0
    task = next((t for t in TASKS if rest and t["id"] == rest[0]), None)
    if task is None:
        print(f"protocol_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # The face the service shows for this task.
        for folder in folders():
            with open(os.path.join(folder, world.WORLD), "w") as f:
                f.write(task["world"])
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task)
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"protocol_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
