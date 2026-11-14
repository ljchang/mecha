#!/usr/bin/env python3
"""The task source for the second far-analog pair
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4), on the contract in `source_stub.py`.

    works_source.py list | setup <task> | grade <task>

Two experience tasks in the archive, then two probes at the kiln
(`works_world.py`). Every task is graded on the service's own state: the
change was made **and** the thing was put back as it was found.

**Held out.** Run once, with the concept arm as it stood after the first
pair; not for tuning.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import works_world as world  # noqa: E402

LEARN_TURNS = 14
# A look, a refused glaze, up to six steps each checked with a look, the
# glaze, and up to five more steps each checked.
PROBE_TURNS = 26

TASKS = [
    {"id": "archive-survey", "tags": ["works", "experience"], "max_turns": LEARN_TURNS, "world": "archive",
     "prompt": "Tag the box titled 'Harbour survey 1911' as 'maps' with the archive tools. Their guide explains any refusal.",
     "kind": "boxes", "thing": "X-4", "field": "tag", "value": "maps", "flag": "open"},
    {"id": "archive-accounts", "tags": ["works", "experience"], "max_turns": LEARN_TURNS, "world": "archive",
     "prompt": "The archive box titled 'Mill accounts' should carry the tag 'ledgers'. If a call is refused, the tools' guide says why.",
     "kind": "boxes", "thing": "X-9", "field": "tag", "value": "ledgers", "flag": "open"},
    {"id": "kiln-bowl", "tags": ["works", "probe"], "max_turns": PROBE_TURNS, "world": "kiln",
     "prompt": "The piece called 'Blue bowl' is to be glazed 'celadon'. Use the kiln tools.",
     "kind": "pieces", "thing": "P-2", "field": "glaze", "value": "celadon", "flag": "racked"},
    {"id": "kiln-vase", "tags": ["works", "probe"], "max_turns": PROBE_TURNS, "world": "kiln",
     "prompt": "With the kiln tools, give the piece called 'Tall vase' an 'ash' glaze.",
     "kind": "pieces", "thing": "P-5", "field": "glaze", "value": "ash", "flag": "racked"},
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
        print(f"works_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
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
    print(f"works_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
