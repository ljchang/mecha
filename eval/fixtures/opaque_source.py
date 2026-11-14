#!/usr/bin/env python3
"""The task source for the opaque-service lifetimes
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4, Tier B: tool quirk and two lessons),
on the contract in `source_stub.py`.

    opaque_source.py <world> list | setup <task> | grade <task>

One lifetime per world, four tasks in order. The first two are experience,
with turns to spare: finding the token rule, then the unit rule. The last two
are probes on another account and period with too few turns to find either
rule again, so they pass only if what the first two showed is still there.

The answer key is computed here from `opaque_world`, which the agent cannot
read. A probe is graded on `answer.json` in the trial's workspace.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import opaque_world  # noqa: E402

ORDINAL = {5: "fifth", 9: "ninth", 11: "eleventh"}
# Turns a probe gets: recall, lookup, export, write, and one to spare. The
# help chain alone is four calls, so the rule cannot be found again inside it.
PROBE_TURNS = 6
LEARN_TURNS = 16


def tasks(world_name):
    world = opaque_world.WORLDS[world_name]
    names = list(world["accounts"])
    first, second = names[0], names[1]
    tools = world["tools"]
    period = world["period_words"]
    year = "FY26" if world_name == "ledger" else "2026"

    def where(name, n):
        return f"the {world['noun']} named '{name}', {ORDINAL[n]} {period} of {year}"

    count = "Write the number of rows to answer.json as {\"rows\": N}."
    total = f"Write the total in {world['major_words']} to answer.json as {{\"total\": X}}."
    return [
        {
            "id": f"{world_name}-learn-rule",
            "tags": ["opaque", world_name, "experience"],
            "max_turns": LEARN_TURNS,
            "prompt": f"Use the {world['server']} tools to fetch the rows for {where(first, 5)}. {count} The tools have their own documentation; read it if a call is rejected.",
            "gold": {"rows": len(opaque_world.rows(world_name, first, 2026, 5))},
        },
        {
            "id": f"{world_name}-learn-units",
            "tags": ["opaque", world_name, "experience"],
            "max_turns": LEARN_TURNS,
            "prompt": f"Use the {world['server']} tools to work out the total for {where(first, 5)}. Confirm it with {tools['check']} before you finish, and keep trying until it is confirmed. {total}",
            "gold": {"total": opaque_world.total_major(world_name, first, 2026, 5)},
        },
        {
            "id": f"{world_name}-probe-rule",
            "tags": ["opaque", world_name, "probe"],
            "max_turns": PROBE_TURNS,
            "prompt": f"Use the {world['server']} tools to fetch the rows for {where(second, 9)}. {count}",
            "gold": {"rows": len(opaque_world.rows(world_name, second, 2026, 9))},
        },
        {
            "id": f"{world_name}-probe-both",
            "tags": ["opaque", world_name, "probe"],
            "max_turns": PROBE_TURNS,
            "prompt": f"Use the {world['server']} tools to work out the total for {where(second, 11)}. {total}",
            "gold": {"total": opaque_world.total_major(world_name, second, 2026, 11)},
        },
    ]


def grade(task, workspace):
    path = os.path.join(workspace, "answer.json")
    key, want = next(iter(task["gold"].items()))
    try:
        with open(path) as f:
            answer = json.load(f)
        got = answer.get(key) if isinstance(answer, dict) else None
    except FileNotFoundError:
        return False, "answer.json was not written"
    except (json.JSONDecodeError, OSError) as e:
        return False, f"answer.json is unreadable: {e}"
    try:
        right = abs(float(got) - float(want)) < 1e-6
    except (TypeError, ValueError):
        right = False
    # The key itself is never put in a detail a later task could read back:
    # the row's detail says only whether the artifact matched.
    return right, f"answer.json {key!r} {'matches' if right else 'does not match'} the service's own figure"


def main(argv):
    if len(argv) < 2 or argv[0] not in opaque_world.WORLDS:
        print(__doc__, file=sys.stderr)
        return 2
    world_name, verb, rest = argv[0], argv[1], argv[2:]
    listed = tasks(world_name)
    if verb == "list":
        json.dump([{k: v for k, v in t.items() if k != "gold"} for t in listed], sys.stdout)
        return 0
    task = next((t for t in listed if rest and t["id"] == rest[0]), None)
    if task is None:
        print(f"opaque_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # The service says what its units are only after two refused checks.
        # The count starts again for every task, so a probe cannot learn the
        # unit rule on one refusal left over from the task that taught it.
        root = os.environ.get("MECHA_FIXTURES")
        if root and os.path.isdir(root):
            for folder, _, files in os.walk(root):
                if "refused-checks" in files:
                    os.remove(os.path.join(folder, "refused-checks"))
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task, os.environ.get("MECHA_EXPERIMENT_WORKSPACE", "."))
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "artifact", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"opaque_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
