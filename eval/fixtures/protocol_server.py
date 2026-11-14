#!/usr/bin/env python3
"""The fixture service for the far-analog lifetime (`protocol_world.py`).

One server, two faces. Which one it shows is read at start from a `world`
file in `$MECHA_FIXTURE_DIR`, written by the task source before each task, so
a task has one service's tools in front of it and never the other's. State
(every record's and berth's fields, and what is held) is kept beside it.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import copy
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import protocol_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def build(store, face):
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {
            "records": {ident: {**copy.deepcopy(fields), "locked": False} for ident, fields in world.REGISTRY["records"].values()},
            "berths": {ident: {**copy.deepcopy(fields), "held": False} for ident, fields in world.QUAY["berths"].values()},
        }

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    save(load())
    string = {"type": "string"}

    def number_if_was(old, value):
        # A model often sends a number as a string.
        if isinstance(old, int) and isinstance(value, str):
            try:
                return int(value)
            except ValueError:
                raise ToolError("the value must be a whole number")
        return value

    if face == "registry":
        def thing(state, args):
            ident = str(args.get("id", "")).strip()
            if ident not in state["records"]:
                raise ToolError(f"no record with id {ident!r}")
            return ident

        def help_(args):
            topic = str(args.get("topic", "")).strip().lower()
            if topic not in world.REGISTRY["topics"]:
                raise ToolError(f"no such topic: {topic!r}. {world.REGISTRY['topics']['']}")
            return world.REGISTRY["topics"][topic]

        def find(args):
            name = str(args.get("name", "")).strip()
            if name not in world.REGISTRY["records"]:
                raise ToolError(f"no record named {name!r}")
            ident = world.REGISTRY["records"][name][0]
            return json.dumps({"id": ident, **load()["records"][ident]})

        def lock(args):
            state = load()
            ident = thing(state, args)
            state["records"][ident]["locked"] = True
            save(state)
            return "locked"

        def unlock(args):
            state = load()
            ident = thing(state, args)
            state["records"][ident]["locked"] = False
            save(state)
            return "unlocked"

        def update(args):
            state = load()
            ident = thing(state, args)
            record = state["records"][ident]
            if not record["locked"]:
                raise ToolError("E4")
            field = str(args.get("field", ""))
            if field not in record or field == "locked":
                raise ToolError(f"no field named {field!r}")
            if "value" not in args:
                raise ToolError("a value is required")
            record[field] = number_if_was(record[field], args["value"])
            save(state)
            return f"{field} updated"

        one = {"type": "object", "properties": {"id": string}, "required": ["id"]}
        tools = [
            {"name": "reg_help", "description": "Read the registry's documentation by topic. An empty topic lists the topics.", "inputSchema": {"type": "object", "properties": {"topic": string}}},
            {"name": "reg_find", "description": "Find a record by its name.", "inputSchema": {"type": "object", "properties": {"name": string}, "required": ["name"]}},
            {"name": "reg_lock", "description": "Lock a record.", "inputSchema": one},
            {"name": "reg_update", "description": "Change one field of a record.", "inputSchema": {"type": "object", "properties": {"id": string, "field": string, "value": {}}, "required": ["id", "field", "value"]}},
            {"name": "reg_unlock", "description": "Unlock a record.", "inputSchema": one},
        ]
        return tools, {"reg_help": help_, "reg_find": find, "reg_lock": lock, "reg_update": update, "reg_unlock": unlock}

    def berth_of(state, args):
        ident = str(args.get("berth", "")).strip()
        if ident not in state["berths"]:
            raise ToolError(f"no berth {ident!r}")
        return ident

    def find(args):
        name = str(args.get("name", "")).strip()
        if name not in world.QUAY["berths"]:
            raise ToolError(f"no berth named {name!r}")
        ident = world.QUAY["berths"][name][0]
        return json.dumps({"berth": ident, **load()["berths"][ident]})

    def op(name):
        def run(args):
            state = load()
            ident = berth_of(state, args)
            if name == world.QUAY["hold"]:
                state["berths"][ident]["held"] = True
            elif name == world.QUAY["free"]:
                state["berths"][ident]["held"] = False
            save(state)
            return "done"
        return run

    def set_(args):
        state = load()
        ident = berth_of(state, args)
        berth = state["berths"][ident]
        if not berth["held"]:
            raise ToolError("refused: the berth is not held")
        key = str(args.get("key", ""))
        if key not in berth or key == "held":
            raise ToolError(f"no setting named {key!r}")
        if "value" not in args:
            raise ToolError("a value is required")
        berth[key] = number_if_was(berth[key], args["value"])
        save(state)
        return f"{key} set"

    one = {"type": "object", "properties": {"berth": string}, "required": ["berth"]}
    tools = [{"name": "quay_find", "description": "Find a berth by its name.", "inputSchema": {"type": "object", "properties": {"name": string}, "required": ["name"]}}]
    tools += [{"name": name, "description": f"Operation {name[-1].upper()} on a berth.", "inputSchema": one} for name in world.QUAY["ops"]]
    tools.append({"name": "quay_set", "description": "Set one value of a berth.", "inputSchema": {"type": "object", "properties": {"berth": string, "key": string, "value": {}}, "required": ["berth", "key", "value"]}})
    handlers = {"quay_find": find, "quay_set": set_, **{name: op(name) for name in world.QUAY["ops"]}}
    return tools, handlers


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        print("protocol_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it the service has no state and no face.", file=sys.stderr)
        return 2
    try:
        with open(os.path.join(store, world.WORLD)) as f:
            face = f.read().strip()
    except OSError:
        face = ""
    if face not in ("registry", "quay"):
        print("protocol_server.py: the store names no world. The task source writes it before each task.", file=sys.stderr)
        return 2
    tools, handlers = build(store, face)
    serve(tools, handlers, face)
    return 0


if __name__ == "__main__":
    sys.exit(main())
