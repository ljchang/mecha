#!/usr/bin/env python3
"""The fixture service for the second far-analog pair (`works_world.py`).

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
import works_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def build(store, face):
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return {
            "boxes": {code: {**copy.deepcopy(fields), "open": False} for code, fields in world.ARCHIVE["boxes"].values()},
            "pieces": {code: {**copy.deepcopy(fields), "racked": False} for code, fields in world.KILN["pieces"].values()},
        }

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    save(load())
    string = {"type": "string"}

    if face == "archive":
        def box_of(state, args):
            code = str(args.get("box", "")).strip()
            if code not in state["boxes"]:
                raise ToolError(f"no box with code {code!r}")
            return code

        def guide(args):
            section = str(args.get("section", "")).strip().lower()
            if section not in world.ARCHIVE["sections"]:
                raise ToolError(f"no such section: {section!r}. {world.ARCHIVE['sections']['']}")
            return world.ARCHIVE["sections"][section]

        def locate(args):
            title = str(args.get("title", "")).strip()
            if title not in world.ARCHIVE["boxes"]:
                raise ToolError(f"no box titled {title!r}")
            code = world.ARCHIVE["boxes"][title][0]
            return json.dumps({"box": code, **load()["boxes"][code]})

        def flag(value, said):
            def run(args):
                state = load()
                state["boxes"][box_of(state, args)]["open"] = value
                save(state)
                return said
            return run

        def tag(args):
            state = load()
            box = state["boxes"][box_of(state, args)]
            if not box["open"]:
                raise ToolError("E7")
            label = str(args.get("tag", "")).strip()
            if not label:
                raise ToolError("a tag is required")
            box["tag"] = label
            save(state)
            return "tagged"

        one = {"type": "object", "properties": {"box": string}, "required": ["box"]}
        tools = [
            {"name": "arc_guide", "description": "Read the archive's guide by section. An empty section lists the sections.", "inputSchema": {"type": "object", "properties": {"section": string}}},
            {"name": "arc_locate", "description": "Locate a box by its title.", "inputSchema": {"type": "object", "properties": {"title": string}, "required": ["title"]}},
            {"name": "arc_open", "description": "Open a box.", "inputSchema": one},
            {"name": "arc_tag", "description": "Put a tag on a box.", "inputSchema": {"type": "object", "properties": {"box": string, "tag": string}, "required": ["box", "tag"]}},
            {"name": "arc_close", "description": "Close a box.", "inputSchema": one},
        ]
        return tools, {"arc_guide": guide, "arc_locate": locate, "arc_open": flag(True, "opened"), "arc_tag": tag, "arc_close": flag(False, "closed")}

    def piece_of(state, args):
        code = str(args.get("piece", "")).strip()
        if code not in state["pieces"]:
            raise ToolError(f"no piece {code!r}")
        return code

    def look(args):
        title = str(args.get("called", "")).strip()
        if title not in world.KILN["pieces"]:
            raise ToolError(f"no piece called {title!r}")
        code = world.KILN["pieces"][title][0]
        return json.dumps({"piece": code, **load()["pieces"][code]})

    def step(name):
        def run(args):
            state = load()
            code = piece_of(state, args)
            if name == world.KILN["rack"]:
                state["pieces"][code]["racked"] = True
            elif name == world.KILN["unrack"]:
                state["pieces"][code]["racked"] = False
            save(state)
            return "ok"
        return run

    def glaze(args):
        state = load()
        piece = state["pieces"][piece_of(state, args)]
        if not piece["racked"]:
            raise ToolError("refused: the piece is not racked")
        colour = str(args.get("colour", "")).strip()
        if not colour:
            raise ToolError("a colour is required")
        piece["glaze"] = colour
        save(state)
        return "glazed"

    one = {"type": "object", "properties": {"piece": string}, "required": ["piece"]}
    tools = [{"name": "kiln_look", "description": "Look up a piece by what it is called.", "inputSchema": {"type": "object", "properties": {"called": string}, "required": ["called"]}}]
    tools += [{"name": name, "description": f"Step {name[-1]} for a piece.", "inputSchema": one} for name in world.KILN["steps"]]
    tools.append({"name": "kiln_glaze", "description": "Glaze a piece.", "inputSchema": {"type": "object", "properties": {"piece": string, "colour": string}, "required": ["piece", "colour"]}})
    return tools, {"kiln_look": look, "kiln_glaze": glaze, **{name: step(name) for name in world.KILN["steps"]}}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        print("works_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it the service has no state and no face.", file=sys.stderr)
        return 2
    try:
        with open(os.path.join(store, world.WORLD)) as f:
            face = f.read().strip()
    except OSError:
        face = ""
    if face not in ("archive", "kiln"):
        print("works_server.py: the store names no world. The task source writes it before each task.", file=sys.stderr)
        return 2
    tools, handlers = build(store, face)
    serve(tools, handlers, face)
    return 0


if __name__ == "__main__":
    sys.exit(main())
