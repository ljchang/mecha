#!/usr/bin/env python3
"""The task source for the third far-analog pair
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4), on the contract in `source_stub.py`.

    grounds_source.py list | setup <task> | grade <task>

Two experience tasks in the greenhouse, then two probes at the switchboard
(`grounds_world.py`). Every task is graded on the service's own state: the
change was made **and** the thing was put back as it was found.

**Held out.** Written before the cue for an owed step was run on anything;
run once, not for tuning.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import grounds_world as world  # noqa: E402

LEARN_TURNS = 14
# A look, a refused change, up to seven keys each checked with a look, the
# change, and up to six more keys each checked.
PROBE_TURNS = 30

TASKS = [
    {"id": "greenhouse-bench", "tags": ["grounds", "experience"], "max_turns": LEARN_TURNS, "world": "greenhouse",
     "prompt": "Lay the greenhouse bed named 'North bench' out in 4 rows with the greenhouse tools. Their notes explain any refusal.",
     "kind": "beds", "thing": "B-3", "field": "rows", "value": 4, "flag": "isolated"},
    {"id": "greenhouse-long", "tags": ["grounds", "experience"], "max_turns": LEARN_TURNS, "world": "greenhouse",
     "prompt": "The greenhouse bed named 'Long bed' should have 6 rows. If a call is refused, the tools' notes say why.",
     "kind": "beds", "thing": "B-8", "field": "rows", "value": 6, "flag": "isolated"},
    {"id": "switchboard-harbour", "tags": ["grounds", "probe"], "max_turns": PROBE_TURNS, "world": "switchboard",
     "prompt": "The line labelled 'Harbour office' is to have its gain set to 7. Use the switchboard tools.",
     "kind": "lines", "thing": "L-14", "field": "gain", "value": 7, "flag": "parked"},
    {"id": "switchboard-pump", "tags": ["grounds", "probe"], "max_turns": PROBE_TURNS, "world": "switchboard",
     "prompt": "With the switchboard tools, set the gain of the line labelled 'Pump house' to 2.",
     "kind": "lines", "thing": "L-27", "field": "gain", "value": 2, "flag": "parked"},
]


def folders():
    root = os.environ.get("MECHA_FIXTURES")
    if root and os.path.isdir(root):
        for name in os.listdir(root):
            if os.path.isdir(os.path.join(root, name)):
                yield os.path.join(root, name)


def grade(task):
    for folder in folders():
        path = os.path.join(folder, world.STATE)
        if not os.path.exists(path):
            continue
        with open(path) as f:
            thing = json.load(f)[task["kind"]][task["thing"]]
        changed = thing[task["field"]] == task["value"]
        if changed and not thing[task["flag"]]:
            return True, "the change was made and the thing was put back"
        if changed:
            return False, "the change was made and the thing was left as the first step put it"
        return False, "the change was not made"
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
        print(f"grounds_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        for folder in folders():
            with open(os.path.join(folder, world.WORLD), "w") as f:
                f.write(task["world"])
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task)
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"grounds_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
