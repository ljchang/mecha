#!/usr/bin/env python3
"""The task source for the depot lifetime (`docs/MEMORY-RETENTION-DESIGN.md`
§11.4 and §11.6), on the contract in `source_stub.py`.

    depot_source.py list | setup <task> | grade <task>

The tallies are experience: read some bay records and write down a routine
figure. A handover note rides in each record and the task needs none of
them; the second tally's request carries a remark of the owner's that it
does not need either. The probes each need one of those things after the
notes are down, and no tool gives it back: a parcel for the cold locker
needs the locker's code, a courier's parcel needs her door, a recipient's
parcel needs his crate. A probe is graded on the service's own state.

**Held out.** Written on 2026-11-16 with the predictions in
`eval/memory-depot.toml`, before any arm ran on it. Not for tuning; a number
from a second run of it is development data and has to say so.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import depot_world as world  # noqa: E402

TALLY_TURNS = 10
# The parcel, the act, the file, and room for a wrong code or two. No number
# of spare turns brings a note back.
PROBE_TURNS = 8

PARCELS = "For the weekly tally, look up bays {bays} with the depot tools and write how many parcels each holds to answer.json as {{\"parcels\": {{\"<bay>\": <count>, ...}}}}. Change nothing."
WEIGHTS = "For the freight return, look up bays {bays} with the depot tools and write each one's total weight to answer.json as {{\"weight_kg\": {{\"<bay>\": <kilograms>, ...}}}}. Change nothing. " + world.ASIDE
COLD = "Parcel P-88 has just come in. Look it up with the depot tools and put it where it goes. Then write {\"done\": true} to answer.json."
COURIER = "The courier is here for parcel P-31. Look it up with the depot tools and put it out for collection where it should go. Then write {\"done\": true} to answer.json."
CRATE = "Parcel P-52 has just come in. Look it up with the depot tools and put it where it goes. Then write {\"done\": true} to answer.json."


def tasks():
    return [
        {"id": "depot-tally-parcels", "tags": ["depot", "experience"], "max_turns": TALLY_TURNS, "prompt": PARCELS.format(bays="1, 2 and 3"), "board": "shift", "field": "parcels", "bays": "123", "column": 0},
        {"id": "depot-tally-weights", "tags": ["depot", "experience"], "max_turns": TALLY_TURNS, "prompt": WEIGHTS.format(bays="4, 5 and 6"), "board": "shift", "field": "weight_kg", "bays": "456", "column": 1},
        {"id": "depot-probe-code", "tags": ["depot", "probe"], "max_turns": PROBE_TURNS, "prompt": COLD, "board": "cleared", "where": "stored", "parcel": "P-88", "want": True},
        {"id": "depot-probe-door", "tags": ["depot", "probe"], "max_turns": PROBE_TURNS, "prompt": COURIER, "board": "cleared", "where": "released", "parcel": "P-31", "want": "7"},
        {"id": "depot-probe-crate", "tags": ["depot", "probe"], "max_turns": PROBE_TURNS, "prompt": CRATE, "board": "cleared", "where": "crated", "parcel": "P-52", "want": "amber"},
    ]


PRIVATE = ("board", "field", "bays", "column", "where", "parcel", "want")


def stores():
    root = os.environ.get("MECHA_FIXTURES")
    if root and os.path.isdir(root):
        for folder, _, files in os.walk(root):
            if world.STATE in files:
                yield folder


def grade(task, workspace):
    if "experience" in task["tags"]:
        try:
            with open(os.path.join(workspace, "answer.json")) as f:
                wrote = json.load(f).get(task["field"], {})
        except (OSError, json.JSONDecodeError, AttributeError):
            return False, f"answer.json has no {task['field']}"
        want = {b: world.BAYS[b][task["column"]] for b in task["bays"]}
        try:
            got = {str(k).strip(): int(v) for k, v in wrote.items()}
        except (AttributeError, TypeError, ValueError):
            return False, "the figures written are not numbers by bay"
        right = got == want
        return right, "the figures written " + ("are the records'" if right else "are not the records'")
    found = list(stores())
    # One depot a trial: graded on the first of several would be graded on
    # whichever the walk happened to yield.
    if len(found) > 1:
        return False, f"{len(found)} depot states were found where one was expected"
    for folder in found:
        with open(os.path.join(folder, world.STATE)) as f:
            did = json.load(f).get(task["where"], {}).get(task["parcel"])
        # Each probe has its own parcel, so what an earlier task did is never
        # read as this one's: a parcel nothing was done to is a failure, and
        # is said as that.
        if did is None:
            return False, "nothing was done with the parcel"
        right = did == task["want"]
        return right, ("done as the note or the owner had said" if right else f"done wrongly ({did})")
    return False, "the depot's state was not found"


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    listed = tasks()
    if verb == "list":
        json.dump([{k: v for k, v in t.items() if k not in PRIVATE} for t in listed], sys.stdout)
        return 0
    task = next((t for t in listed if rest and t["id"] == rest[0]), None)
    if task is None:
        print(f"depot_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # Which notes each task meets: the shift's for the tallies, none for
        # a probe. Before a lifetime's first task no store exists yet; the
        # server takes that as the shift's notes (`depot_server.py`), so the
        # first task of a lifetime must be one that meets them.
        for folder in stores():
            with open(os.path.join(folder, world.BOARD), "w") as f:
                f.write(task["board"])
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task, os.environ.get("MECHA_EXPERIMENT_WORKSPACE", "."))
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"depot_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
