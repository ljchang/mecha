#!/usr/bin/env python3
"""The task source for the misleading-experience lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4), on the contract in `source_stub.py`.

    relay_source.py list | setup <task> | grade <task>

Four tasks in order: a fix that only looked like one (`relay_world.py`), a
real fix of the same fault, and two probes that meet the fault again with
every note cleared and turns for one attempt. Every task is graded on the
service's own state — the relay's settings are the ones that deliver, and
for a probe it passed a full test — never on what the agent says.

**Held out.** Written after the opaque-service and split-repair families and
after the episode arms were built; run once, and not for tuning
(`MEMORY-BENCHMARK-DESIGN.md` §5).
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import relay_world as world  # noqa: E402

FIX_TURNS = 12
# Status, settings, two changes, a test, the file, and one to spare: a wrong
# first attempt (a change and a failed test) leaves too few for the right one.
PROBE_TURNS = 7

FIX = "The relay '{relay}' is stalling. Use the relay tools to find out why, fix its settings, and confirm with relay_test. Then write {{\"fixed\": true}} to answer.json and say in a sentence what fixed it."
PROBE = "The relay '{relay}' is stalling, and its operator note has been cleared. Use the relay tools to fix its settings so that relay_test passes. Then write {{\"fixed\": true}} to answer.json."


def tasks():
    return [
        {"id": "relay-fix-guess", "tags": ["relay", "experience"], "max_turns": FIX_TURNS, "prompt": FIX.format(relay="relay-west"), "relay": "relay-west"},
        {"id": "relay-fix-vendor", "tags": ["relay", "experience"], "max_turns": FIX_TURNS, "prompt": FIX.format(relay="relay-east"), "relay": "relay-east"},
        {"id": "relay-probe-north", "tags": ["relay", "probe"], "max_turns": PROBE_TURNS, "prompt": PROBE.format(relay="relay-north"), "relay": "relay-north"},
        {"id": "relay-probe-south", "tags": ["relay", "probe"], "max_turns": PROBE_TURNS, "prompt": PROBE.format(relay="relay-south"), "relay": "relay-south"},
    ]


def stores():
    root = os.environ.get("MECHA_FIXTURES")
    if root and os.path.isdir(root):
        for folder, _, files in os.walk(root):
            if world.STATE in files:
                yield folder


def grade(task):
    for folder in stores():
        with open(os.path.join(folder, world.STATE)) as f:
            state = json.load(f)
        delivering = world.healthy(state["settings"].get(task["relay"], {}))
        if "probe" in task["tags"]:
            delivering = delivering and task["relay"] in state.get("passed", [])
        return delivering, "the relay is delivering" if delivering else "the relay is still stalled"
    return False, "the service kept no state"


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    listed = tasks()
    if verb == "list":
        json.dump([{k: v for k, v in t.items() if k != "relay"} for t in listed], sys.stdout)
        return 0
    task = next((t for t in listed if rest and t["id"] == rest[0]), None)
    if task is None:
        print(f"relay_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # The first task runs in the maintenance window; the probes run with
        # every note cleared. A store that does not exist yet is the first
        # task's, so the window is also marked wherever a store will be made.
        want = {world.MAINTENANCE: task["id"] == "relay-fix-guess", world.CLEARED: "probe" in task["tags"]}
        root = os.environ.get("MECHA_FIXTURES")
        folders = set(stores())
        if root and os.path.isdir(root):
            folders.update(os.path.join(root, d) for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)))
        for folder in folders:
            for marker, on in want.items():
                path = os.path.join(folder, marker)
                if on:
                    open(path, "w").close()
                elif os.path.exists(path):
                    os.remove(path)
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task)
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"relay_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
