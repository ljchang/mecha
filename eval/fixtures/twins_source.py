#!/usr/bin/env python3
"""The task source for the linked-change lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4), on the contract in `source_stub.py`.

    twins_source.py list | setup <task> | grade <task>

Two pairs (`twins_world.py`); a manifest names the four tasks of one. Every
task is graded on the service's own state: the change was made on the thing
**and** on its twin.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import twins_world as world  # noqa: E402

LEARN_TURNS = 12
PROBE_TURNS = 14


def task(ident, face, title, value, prompt, probe):
    code, twin, _ = world.FACES[face]["things"][title]
    return {"id": ident, "tags": ["twins", "probe" if probe else "experience"], "max_turns": PROBE_TURNS if probe else LEARN_TURNS,
            "prompt": prompt, "world": face, "thing": code, "twin": twin, "value": value}


TASKS = [
    task("signs-bridge", "signs", "Bridge approach", "Closed for repairs",
         "Make the sign named 'Bridge approach' read 'Closed for repairs' with the sign tools. Their manual explains any code they answer with.", False),
    task("signs-mill", "signs", "Mill lane", "Market day",
         "The sign named 'Mill lane' should read 'Market day'. If a tool answers with a code, the tools' manual says what it means.", False),
    task("valves-north", "valves", "North feed", "slow",
         "The valve tagged 'North feed' is to be put in mode 'slow'. Use the valve tools.", True),
    task("valves-yard", "valves", "Yard return", "shut",
         "With the valve tools, put the valve tagged 'Yard return' in mode 'shut'.", True),
    task("accounts-harbour", "accounts", "Harbour dues", 800,
         "Set the limit of the account titled 'Harbour dues' to 800 with the account tools. Their help explains any code they answer with.", False),
    task("accounts-mill", "accounts", "Mill rents", 450,
         "The account titled 'Mill rents' should have a limit of 450. If a tool answers with a code, the tools' help says what it means.", False),
    task("lamps-quay", "lamps", "Quay steps", 5,
         "The lamp labelled 'Quay steps' is to be set to level 5. Use the lamp tools.", True),
    task("lamps-ferry", "lamps", "Ferry gate", 1,
         "With the lamp tools, set the lamp labelled 'Ferry gate' to level 1.", True),
]


def folders():
    root = os.environ.get("MECHA_FIXTURES")
    if root and os.path.isdir(root):
        for name in os.listdir(root):
            if os.path.isdir(os.path.join(root, name)):
                yield os.path.join(root, name)


def grade(task):
    face = world.FACES[task["world"]]
    for folder in folders():
        path = os.path.join(folder, world.STATE)
        if not os.path.exists(path):
            continue
        with open(path) as f:
            things = json.load(f)[task["world"]]
        first = things[task["thing"]][face["value"]] == task["value"]
        second = things[task["twin"]][face["value"]] == task["value"]
        if first and second:
            return True, "the change was made on the thing and on its twin"
        if first:
            return False, "the change was made on the thing and not on its twin"
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
        print(f"twins_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
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
    print(f"twins_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
