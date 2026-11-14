#!/usr/bin/env python3
"""The fixture service for the third far-analog pair (`grounds_world.py`).

One server, two faces, chosen at start by the `world` file the task source
writes into `$MECHA_FIXTURE_DIR`. State is kept beside it.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import copy
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import grounds_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def whole(args, key):
    value = args.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise ToolError(f"{key} must be a whole number")
    return value


def build(store, face):
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {
            "beds": {code: {**copy.deepcopy(fields), "isolated": False} for code, fields in world.GREENHOUSE["beds"].values()},
            "lines": {code: {**copy.deepcopy(fields), "parked": False} for code, fields in world.SWITCHBOARD["lines"].values()},
        }

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    save(load())
    string = {"type": "string"}
    number = {"type": "integer"}

    if face == "greenhouse":
        def bed_of(state, args):
            code = str(args.get("bed", "")).strip()
            if code not in state["beds"]:
                raise ToolError(f"no bed with code {code!r}")
            return code

        def notes(args):
            page = str(args.get("page", "")).strip().lower()
            if page not in world.GREENHOUSE["pages"]:
                raise ToolError(f"no such page: {page!r}. {world.GREENHOUSE['pages']['']}")
            return world.GREENHOUSE["pages"][page]

        def bed(args):
            name = str(args.get("name", "")).strip()
            if name not in world.GREENHOUSE["beds"]:
                raise ToolError(f"no bed named {name!r}")
            code = world.GREENHOUSE["beds"][name][0]
            return json.dumps({"bed": code, **load()["beds"][code]})

        def flag(value, said):
            def run(args):
                state = load()
                state["beds"][bed_of(state, args)]["isolated"] = value
                save(state)
                return said
            return run

        def rows(args):
            state = load()
            found = state["beds"][bed_of(state, args)]
            if not found["isolated"]:
                raise ToolError("R2")
            found["rows"] = whole(args, "rows")
            save(state)
            return "set"

        one = {"type": "object", "properties": {"bed": string}, "required": ["bed"]}
        tools = [
            {"name": "gh_notes", "description": "Read the greenhouse notes by page. An empty page lists the pages.", "inputSchema": {"type": "object", "properties": {"page": string}}},
            {"name": "gh_bed", "description": "Find a bed by its name.", "inputSchema": {"type": "object", "properties": {"name": string}, "required": ["name"]}},
            {"name": "gh_isolate", "description": "Isolate a bed.", "inputSchema": one},
            {"name": "gh_rows", "description": "Set a bed's row count.", "inputSchema": {"type": "object", "properties": {"bed": string, "rows": number}, "required": ["bed", "rows"]}},
            {"name": "gh_reconnect", "description": "Reconnect a bed.", "inputSchema": one},
        ]
        return tools, {"gh_notes": notes, "gh_bed": bed, "gh_isolate": flag(True, "isolated"), "gh_rows": rows, "gh_reconnect": flag(False, "reconnected")}

    def line_of(state, args):
        code = str(args.get("line", "")).strip()
        if code not in state["lines"]:
            raise ToolError(f"no line {code!r}")
        return code

    def line(args):
        label = str(args.get("label", "")).strip()
        if label not in world.SWITCHBOARD["lines"]:
            raise ToolError(f"no line labelled {label!r}")
        code = world.SWITCHBOARD["lines"][label][0]
        return json.dumps({"line": code, **load()["lines"][code]})

    def key(name):
        def run(args):
            state = load()
            code = line_of(state, args)
            if name == world.SWITCHBOARD["park"]:
                state["lines"][code]["parked"] = True
            elif name == world.SWITCHBOARD["restore"]:
                state["lines"][code]["parked"] = False
            save(state)
            return "ok"
        return run

    def gain(args):
        state = load()
        found = state["lines"][line_of(state, args)]
        if not found["parked"]:
            raise ToolError("refused: the line is not parked")
        found["gain"] = whole(args, "level")
        save(state)
        return "gain set"

    one = {"type": "object", "properties": {"line": string}, "required": ["line"]}
    tools = [{"name": "sw_line", "description": "Look up a line by its label.", "inputSchema": {"type": "object", "properties": {"label": string}, "required": ["label"]}}]
    tools += [{"name": name, "description": f"Key {name[-1].upper()} for a line.", "inputSchema": one} for name in world.SWITCHBOARD["keys"]]
    tools.append({"name": "sw_gain", "description": "Set a line's gain.", "inputSchema": {"type": "object", "properties": {"line": string, "level": number}, "required": ["line", "level"]}})
    return tools, {"sw_line": line, "sw_gain": gain, **{name: key(name) for name in world.SWITCHBOARD["keys"]}}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        print("grounds_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it the service has no state and no face.", file=sys.stderr)
        return 2
    try:
        with open(os.path.join(store, world.WORLD)) as f:
            face = f.read().strip()
    except OSError:
        face = ""
    if face not in ("greenhouse", "switchboard"):
        print("grounds_server.py: the store names no world. The task source writes it before each task.", file=sys.stderr)
        return 2
    tools, handlers = build(store, face)
    serve(tools, handlers, face)
    return 0


if __name__ == "__main__":
    sys.exit(main())
