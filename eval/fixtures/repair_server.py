#!/usr/bin/env python3
"""The fixture service for the split-repair lifetime (`repair_world.py`):
job status, the run log while it is retained, settings, and a run.

State (each job's settings and which jobs have passed a run) is kept in
`$MECHA_FIXTURE_DIR` so it survives the per-task restart of the server
within one lifetime, and so the task source can grade on it. A `logs-rotated`
file beside it, written by the source before a probe, takes every log away.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import copy
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import repair_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def build(store):
    path = os.path.join(store, world.STATE) if store else None

    def load():
        if path and os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {"settings": {job: copy.deepcopy(s) for job, (_, s) in world.JOBS.items()}, "passed": []}

    def save(state):
        if path:
            with open(path, "w") as f:
                json.dump(state, f)

    # Written at start, so the source finds the store before any call.
    save(load())

    def job_of(args):
        job = str(args.get("job", "")).strip()
        if job not in world.JOBS:
            raise ToolError(f"no job named {job!r}")
        return job

    def healthy(state, job):
        return world.FAULTS[world.JOBS[job][0]]["fixed"](state["settings"][job])

    def status(args):
        job, state = job_of(args), load()
        if healthy(state, job):
            return "last run passed" if job in state["passed"] else "settings changed since the last failure; not run yet"
        return f"last run failed at {world.FAULTS[world.JOBS[job][0]]['step']}. The run log has the reason."

    def log(args):
        job = job_of(args)
        if store and os.path.exists(os.path.join(store, world.ROTATED)):
            raise ToolError("no log retained for this job: logs are rotated after a day")
        return world.FAULTS[world.JOBS[job][0]]["log"]

    def settings(args):
        return json.dumps(load()["settings"][job_of(args)])

    def set_(args):
        job, state = job_of(args), load()
        key = str(args.get("key", ""))
        if key not in state["settings"][job]:
            raise ToolError(f"no setting named {key!r}; settings are {sorted(state['settings'][job])}")
        if "value" not in args:
            raise ToolError("a value is required")
        value = args["value"]
        # A model often sends a number as a string.
        if isinstance(value, str) and isinstance(world.JOBS[job][1][key], int):
            try:
                value = int(value)
            except ValueError:
                raise ToolError(f"{key} must be a whole number")
        state["settings"][job][key] = value
        if job in state["passed"]:
            state["passed"].remove(job)
        save(state)
        return f"{key} set"

    def run(args):
        job, state = job_of(args), load()
        if not healthy(state, job):
            raise ToolError(f"run failed at {world.FAULTS[world.JOBS[job][0]]['step']}")
        if job not in state["passed"]:
            state["passed"].append(job)
            save(state)
        return "run passed"

    job = {"type": "string", "description": "The job's name."}
    one = {"type": "object", "properties": {"job": job}, "required": ["job"]}
    tools = [
        {"name": "jobs_status", "description": "How a job's last run went.", "inputSchema": one},
        {"name": "jobs_log", "description": "The log of a job's last run, while it is retained.", "inputSchema": one},
        {"name": "jobs_settings", "description": "A job's current settings.", "inputSchema": one},
        {"name": "jobs_set", "description": "Change one setting of a job.", "inputSchema": {"type": "object", "properties": {"job": job, "key": {"type": "string"}, "value": {}}, "required": ["job", "key", "value"]}},
        {"name": "jobs_run", "description": "Run a job now.", "inputSchema": one},
    ]
    return tools, {"jobs_status": status, "jobs_log": log, "jobs_settings": settings, "jobs_set": set_, "jobs_run": run}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        # A world with no state is a different measurement, not this one.
        print("repair_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it no log is ever rotated away and every probe can read the cause.", file=sys.stderr)
        return 2
    tools, handlers = build(store)
    serve(tools, handlers, "jobs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
