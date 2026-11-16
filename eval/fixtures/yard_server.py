#!/usr/bin/env python3
"""The fixture service for the yard lifetime (`yard_world.py`): berth
records, vessels, and an assignment.

A berth's record carries its notice while the board that holds one is up.
The task source names the board before each task by a `yard-board` file in
`$MECHA_FIXTURE_DIR`; assignments are kept beside it so the source can grade
on them and they survive the per-task restart of the server.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import yard_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def build(store):
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {"assigned": {}}

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    # Written at start, so the source finds the store before any call.
    save(load())

    def board():
        # With no board named, the first one is up. The source names a board
        # by a file in this store, and can only find the store once this
        # server has started in it: before a lifetime's first task there is
        # none to find. Read as "no notices", that made the first survey
        # read bare records, and nothing a later task needed was ever shown
        # (found in a scripted dry run, before any measured run).
        try:
            with open(os.path.join(store, world.BOARD)) as f:
                name = f.read().strip()
        except OSError:
            name = "first"
        return world.BOARDS.get(name, {})

    def berth_of(args):
        berth = str(args.get("berth", "")).strip().upper()
        if berth not in world.BERTHS:
            raise ToolError(f"no berth named {berth!r}; berths are {sorted(world.BERTHS)}")
        return berth

    def vessel_of(args):
        name = str(args.get("vessel", "")).strip()
        if name not in world.VESSELS:
            raise ToolError(f"no vessel named {name!r}")
        return name

    def berths(_args):
        return json.dumps(sorted(world.BERTHS))

    def berth(args):
        name = berth_of(args)
        length, bollards = world.BERTHS[name]
        record = {"berth": name, "length_m": length, "bollards": bollards}
        notice = board().get(name)
        if notice:
            record["notice"] = notice
        return json.dumps(record)

    def vessel(args):
        name = vessel_of(args)
        length, carries = world.VESSELS[name]
        return json.dumps({"vessel": name, "length_m": length, "carries": carries})

    def assign(args):
        name, where = vessel_of(args), berth_of(args)
        # Length is the one thing the service itself checks. What a notice
        # ruled out it never knew.
        if world.VESSELS[name][0] > world.BERTHS[where][0]:
            raise ToolError(f"{name} is longer than berth {where}")
        state = load()
        state["assigned"][name] = where
        save(state)
        return f"{name} assigned to berth {where}"

    one_berth = {"type": "object", "properties": {"berth": {"type": "string", "description": "The berth's letter."}}, "required": ["berth"]}
    one_vessel = {"type": "object", "properties": {"vessel": {"type": "string", "description": "The vessel's name."}}, "required": ["vessel"]}
    tools = [
        {"name": "yard_berths", "description": "The yard's berths, by letter.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "yard_berth", "description": "One berth's record.", "inputSchema": one_berth},
        {"name": "yard_vessel", "description": "One vessel's record.", "inputSchema": one_vessel},
        {
            "name": "yard_assign",
            "description": "Assign a vessel to a berth. A later assignment replaces an earlier one.",
            "inputSchema": {"type": "object", "properties": {"vessel": {"type": "string"}, "berth": {"type": "string"}}, "required": ["vessel", "berth"]},
        },
    ]
    return tools, {"yard_berths": berths, "yard_berth": berth, "yard_vessel": vessel, "yard_assign": assign}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        # A world with no state is a different measurement, not this one.
        print("yard_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it no board is ever cleared and every probe can read the notices.", file=sys.stderr)
        return 2
    tools, handlers = build(store)
    serve(tools, handlers, "yard")
    return 0


if __name__ == "__main__":
    sys.exit(main())
