#!/usr/bin/env python3
"""The fixture service for the depot lifetime (`depot_world.py`): bay
records, parcels, and the three things a parcel can have done to it.

A bay's record carries its handover note while the shift's notes are up. The
task source names which notes are up before each task by a `depot-board`
file in `$MECHA_FIXTURE_DIR`. Nothing here returns the locker's code, a
courier's door or a recipient's crate: each is an argument the caller has to
bring.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import depot_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def build(store):
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {"stored": {}, "released": {}, "crated": {}, "wrong_codes": 0}

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    # Written at start, so the source finds the store before any call.
    save(load())

    def notes():
        # With nothing named, the shift's notes are up: before a lifetime's
        # first task the source has no store to name anything in (the yard
        # lifetime's first form read that as no notes at all).
        try:
            with open(os.path.join(store, world.BOARD)) as f:
                name = f.read().strip()
        except OSError:
            name = "shift"
        return world.NOTES.get(name, {})

    def parcel_of(args):
        name = str(args.get("parcel", "")).strip().upper()
        if name not in world.PARCELS:
            raise ToolError(f"no parcel {name!r} is in the depot")
        return name

    def bays(_args):
        return json.dumps(sorted(world.BAYS))

    def bay(args):
        name = str(args.get("bay", "")).strip()
        if name not in world.BAYS:
            raise ToolError(f"no bay {name!r}; bays are {sorted(world.BAYS)}")
        parcels, weight = world.BAYS[name]
        record = {"bay": name, "parcels": parcels, "weight_kg": weight}
        note = notes().get(name)
        if note:
            record["handover_note"] = note
        return json.dumps(record)

    def parcel(args):
        name = parcel_of(args)
        return json.dumps({"parcel": name, **world.PARCELS[name]})

    def store_cold(args):
        name = parcel_of(args)
        code = str(args.get("code", "")).strip()
        state = load()
        if state["wrong_codes"] >= world.TRIES:
            raise ToolError("the cold locker is locked for the day after too many wrong codes")
        if code != world.COLD_CODE:
            state["wrong_codes"] += 1
            save(state)
            raise ToolError("the cold locker did not open: wrong code")
        state["stored"][name] = True
        save(state)
        return f"{name} is in the cold locker"

    def release(args):
        name = parcel_of(args)
        door = str(args.get("door", "")).strip()
        if door not in world.DOORS:
            raise ToolError(f"no dock door {door!r}; doors are 1 to 9")
        # The service opens whichever door it is told to. Whose pass opens
        # which it never knew.
        state = load()
        state["released"][name] = door
        save(state)
        return f"{name} is waiting at dock door {door}"

    def crate(args):
        name = parcel_of(args)
        colour = str(args.get("crate", "")).strip().lower()
        if colour not in world.CRATES:
            raise ToolError(f"no {colour!r} crate; crates are {world.CRATES}")
        state = load()
        state["crated"][name] = colour
        save(state)
        return f"{name} is in the {colour} crate"

    one_parcel = {"type": "string", "description": "The parcel's number."}
    tools = [
        {"name": "depot_bays", "description": "The depot's bays, by number.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "depot_bay", "description": "One bay's record.", "inputSchema": {"type": "object", "properties": {"bay": {"type": "string", "description": "The bay's number."}}, "required": ["bay"]}},
        {"name": "depot_parcel", "description": "One parcel's record.", "inputSchema": {"type": "object", "properties": {"parcel": one_parcel}, "required": ["parcel"]}},
        {"name": "depot_store_cold", "description": "Put a parcel in the cold locker. The locker opens with its code and locks for the day after three wrong ones.", "inputSchema": {"type": "object", "properties": {"parcel": one_parcel, "code": {"type": "string", "description": "The locker's code."}}, "required": ["parcel", "code"]}},
        {"name": "depot_release", "description": "Put a parcel out at a dock door for its courier. A later release replaces an earlier one.", "inputSchema": {"type": "object", "properties": {"parcel": one_parcel, "door": {"type": "string", "description": "The dock door, 1 to 9."}}, "required": ["parcel", "door"]}},
        {"name": "depot_crate", "description": "Put a parcel in a crate, by the crate's colour. A later crate replaces an earlier one.", "inputSchema": {"type": "object", "properties": {"parcel": one_parcel, "crate": {"type": "string", "description": "The crate's colour."}}, "required": ["parcel", "crate"]}},
    ]
    return tools, {"depot_bays": bays, "depot_bay": bay, "depot_parcel": parcel, "depot_store_cold": store_cold, "depot_release": release, "depot_crate": crate}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        # A world with no state is a different measurement, not this one.
        print("depot_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it the shift's notes are never taken down and every probe can read them.", file=sys.stderr)
        return 2
    tools, handlers = build(store)
    serve(tools, handlers, "depot")
    return 0


if __name__ == "__main__":
    sys.exit(main())
