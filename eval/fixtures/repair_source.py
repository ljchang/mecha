#!/usr/bin/env python3
"""The task source for the split-repair lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4, Tier B), on the contract in
`source_stub.py`.

    repair_source.py list | setup <task> | grade <task>

Four tasks in order. The first two are experience: read why a job failed and
write the reason down, changing nothing. The last two are probes: another job
has the same fault, every log has been rotated away, and the job must be
made to pass. A probe is graded on the service's own state — the job passed
a run — never on what the agent says.

**Held out.** This family was written after the opaque-service family and
before any memory arm was run on it or tuned to it
(`MEMORY-BENCHMARK-DESIGN.md` §5).
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import repair_world as world  # noqa: E402

OBSERVE_TURNS = 10
# Status, settings, two changes, a run, the file, and two to spare. No
# number of spare turns finds two unstated values by trying.
PROBE_TURNS = 8

OBSERVE = "The nightly job '{job}' failed last night. Use the jobs tools to find out why, and write the reason to answer.json as {{\"cause\": \"...\"}} in one or two sentences. Do not change any setting."
REPAIR = "The nightly job '{job}' is failing, and its run log has been rotated away. Use the jobs tools to fix its settings so that jobs_run passes. When it has passed, write {{\"fixed\": true}} to answer.json."


def tasks():
    return [
        {"id": "repair-observe-validate", "tags": ["repair", "experience"], "max_turns": OBSERVE_TURNS, "prompt": OBSERVE.format(job="ingest-harbor"), "job": "ingest-harbor"},
        {"id": "repair-observe-connect", "tags": ["repair", "experience"], "max_turns": OBSERVE_TURNS, "prompt": OBSERVE.format(job="ingest-juniper"), "job": "ingest-juniper"},
        {"id": "repair-probe-validate", "tags": ["repair", "probe"], "max_turns": PROBE_TURNS, "prompt": REPAIR.format(job="ingest-tidewater"), "job": "ingest-tidewater"},
        {"id": "repair-probe-connect", "tags": ["repair", "probe"], "max_turns": PROBE_TURNS, "prompt": REPAIR.format(job="ingest-marlow"), "job": "ingest-marlow"},
    ]


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
                cause = str(json.load(f).get("cause", ""))
        except (OSError, json.JSONDecodeError, AttributeError):
            return False, "answer.json has no cause"
        said = world.FAULTS[world.JOBS[task["job"]][0]]["said"]
        right = all(part in cause for part in said)
        return right, "the cause written " + ("names what the log named" if right else "does not name what the log named")
    for folder in stores():
        with open(os.path.join(folder, world.STATE)) as f:
            if task["job"] in json.load(f).get("passed", []):
                return True, "the job passed a run"
    return False, "the job has not passed a run"


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    listed = tasks()
    if verb == "list":
        json.dump([{k: v for k, v in t.items() if k != "job"} for t in listed], sys.stdout)
        return 0
    task = next((t for t in listed if rest and t["id"] == rest[0]), None)
    if task is None:
        print(f"repair_source: no task `{rest[0] if rest else ''}`", file=sys.stderr)
        return 2
    if verb == "setup":
        # Logs are there for the experience tasks and gone for the probes.
        for folder in stores():
            marker = os.path.join(folder, world.ROTATED)
            if "probe" in task["tags"]:
                open(marker, "w").close()
            elif os.path.exists(marker):
                os.remove(marker)
        return 0
    if verb == "grade":
        sys.stdin.read()
        passed, detail = grade(task, os.environ.get("MECHA_EXPERIMENT_WORKSPACE", "."))
        json.dump({"passed": passed, "detail": detail, "checks": [{"name": "outcome", "passed": passed, "detail": detail}]}, sys.stdout)
        return 0
    print(f"repair_source: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
