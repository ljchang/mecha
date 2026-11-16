#!/usr/bin/env python3
"""The task source for the yard lifetime (`docs/MEMORY-RETENTION-DESIGN.md`
§11.4), on the contract in `source_stub.py`.

    yard_source.py list | setup <task> | grade <task>

The surveys are experience: read some berth records and write down a routine
figure. A notice rides in each record and the task needs none of them. The
probes place a vessel after the board has been cleared, by a rule whose
plain reading picks a berth a notice had ruled out. A probe is graded on the
service's own state, the berth the vessel is assigned to, never on what the
agent says.

Two lifetimes are cut from these tasks. The plain one (`memory-yard.toml`):
two surveys, then two probes. The superseded one
(`memory-yard-superseded.toml`): a recheck after the surveys reads a second
board on which one notice has reversed, and the reefer probe's right berth
is the one the second board names.

**Held out.** Written before the stored index it is meant to measure
existed, and before any memory arm was run on it.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import yard_world as world  # noqa: E402

SURVEY_TURNS = 10
# The vessel, the berths, a record or two, the assignment, the file, and two
# to spare. No number of spare turns brings a cleared notice back.
PROBE_TURNS = 8

LENGTHS = "For the quay register, look up berths {berths} with the yard tools and write their lengths to answer.json as {{\"lengths\": {{\"<berth>\": <metres>, ...}}}}. Change nothing."
BOLLARDS = "For the mooring audit, look up berths {berths} with the yard tools and write how many bollards each has to answer.json as {{\"bollards\": {{\"<berth>\": <count>, ...}}}}. Change nothing."
PLACE = "The vessel '{vessel}' arrives tonight. Look it up with the yard tools and assign it to the first berth in letter order that can take it, given what it carries and its length. Then write {{\"assigned\": \"<berth>\"}} to answer.json."


def tasks():
    return [
        {"id": "yard-survey-lengths", "tags": ["yard", "experience"], "max_turns": SURVEY_TURNS, "prompt": LENGTHS.format(berths="A, B and C"), "board": "first", "field": "lengths", "berths": "ABC", "column": 0},
        {"id": "yard-survey-bollards", "tags": ["yard", "experience"], "max_turns": SURVEY_TURNS, "prompt": BOLLARDS.format(berths="D, E and F"), "board": "first", "field": "bollards", "berths": "DEF", "column": 1},
        {"id": "yard-recheck-bollards", "tags": ["yard", "experience"], "max_turns": SURVEY_TURNS, "prompt": BOLLARDS.format(berths="B and D"), "board": "second", "field": "bollards", "berths": "BD", "column": 1},
        {"id": "yard-probe-reefer", "tags": ["yard", "probe"], "max_turns": PROBE_TURNS, "prompt": PLACE.format(vessel="Corvid"), "board": "cleared", "vessel": "Corvid", "berth": "B"},
        {"id": "yard-probe-length", "tags": ["yard", "probe"], "max_turns": PROBE_TURNS, "prompt": PLACE.format(vessel="Halcyon"), "board": "cleared", "vessel": "Halcyon", "berth": "E"},
        {"id": "yard-probe-reefer-after", "tags": ["yard", "probe"], "max_turns": PROBE_TURNS, "prompt": PLACE.format(vessel="Petrel"), "board": "cleared", "vessel": "Petrel", "berth": "D"},
    ]


PRIVATE = ("board", "field", "berths", "column", "vessel", "berth")


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
        want = {b: world.BERTHS[b][task["column"]] for b in task["berths"]}
        try:
            got = {str(k).strip().upper(): int(v) for k, v in wrote.items()}
        except (AttributeError, TypeError, ValueError):
            return False, "the figures written are not numbers by berth"
        right = got == want
        return right, "the figures written " + ("are the records'" if right else "are not the records'")
    for folder in stores():
        with open(os.path.join(folder, world.STATE)) as f:
            where = json.load(f).get("assigned", {}).get(task["vessel"])
        if where is not None:
            right = where == task["berth"]
            return right, f"assigned to berth {where}, " + ("which the notices allowed" if right else "which a notice had ruled out or which is not the first allowed")
    return False, "the vessel was not assigned"


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
        print(f"yard_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # The board each task meets: up for the surveys, cleared for a probe.
        # Before a lifetime's first task no store exists yet and nothing is
        # written; the server takes that as the first board
        # (`yard_server.py`), so the first task of a lifetime must be one
        # that meets it.
        for folder in stores():
            with open(os.path.join(folder, world.BOARD), "w") as f:
                f.write(task["board"])
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task, os.environ.get("MECHA_EXPERIMENT_WORKSPACE", "."))
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"yard_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
