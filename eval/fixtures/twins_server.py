#!/usr/bin/env python3
"""The fixture service for the linked-change lifetime (`twins_world.py`).

One server, one face per start, chosen by the `world` file the task source
writes into `$MECHA_FIXTURE_DIR`. State is kept beside it.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import copy
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import twins_world as world  # noqa: E402
from opaque_server import ToolError, serve  # noqa: E402


def fresh():
    state = {}
    for name, face in world.FACES.items():
        things = {}
        for code, twin, fields in face["things"].values():
            things[code] = {**copy.deepcopy(fields), face["twin"]: twin}
            things[twin] = {**copy.deepcopy(fields), face["twin"]: code}
        state[name] = things
    return state


def build(store, name):
    face = world.FACES[name]
    path = os.path.join(store, world.STATE)

    def load():
        if os.path.exists(path):
            with open(path) as f:
                return json.load(f)
        return fresh()

    def save(state):
        with open(path, "w") as f:
            json.dump(state, f)

    save(load())
    string = {"type": "string"}
    lookup, lookup_arg, lookup_says = face["lookup"]
    change, thing_arg, value_arg, change_says = face["change"]

    def thing_of(state, args):
        code = str(args.get(thing_arg, "")).strip()
        if code not in state[name]:
            raise ToolError(f"no {face['id']} {code!r}")
        return code

    def look(args):
        title = str(args.get(lookup_arg, "")).strip()
        if title not in face["things"]:
            raise ToolError(f"nothing found for {title!r}")
        code = face["things"][title][0]
        return json.dumps({face["id"]: code, **load()[name][code]})

    def set_value(args):
        state = load()
        code = thing_of(state, args)
        value = args.get(value_arg)
        if face["kind"] == "integer":
            if isinstance(value, bool) or not isinstance(value, int):
                raise ToolError(f"{value_arg} must be a whole number")
        else:
            value = str(value or "").strip()
            if not value:
                raise ToolError(f"{value_arg} is required")
        thing = state[name][code]
        thing[face["value"]] = value
        save(state)
        if face["code"] and state[name][thing[face["twin"]]][face["value"]] != value:
            return f"set ({face['code']})"
        return "set" if face["code"] else "ok"

    def nothing(args):
        thing_of(load(), args)
        return "ok"

    one = {"type": "object", "properties": {thing_arg: string}, "required": [thing_arg]}
    tools = [
        {"name": lookup, "description": lookup_says, "inputSchema": {"type": "object", "properties": {lookup_arg: string}, "required": [lookup_arg]}},
        {"name": change, "description": change_says, "inputSchema": {"type": "object", "properties": {thing_arg: string, value_arg: {"type": face["kind"]}}, "required": [thing_arg, value_arg]}},
    ]
    handlers = {lookup: look, change: set_value}
    for extra in face["extra"]:
        tools.append({"name": extra, "description": f"{extra.rsplit('_', 2)[-2].capitalize()} {extra[-1].upper()} for a {face['id']}.", "inputSchema": one})
        handlers[extra] = nothing
    if face["docs"]:
        docs, docs_arg, docs_says, pages = face["docs"]

        def read(args):
            page = str(args.get(docs_arg, "")).strip().lower()
            if page not in pages:
                raise ToolError(f"no such {docs_arg}: {page!r}. {pages['']}")
            return pages[page]

        tools.insert(0, {"name": docs, "description": docs_says, "inputSchema": {"type": "object", "properties": {docs_arg: string}}})
        handlers[docs] = read
    return tools, handlers


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not store or not os.path.isdir(store):
        print("twins_server.py: no store directory. Set MECHA_FIXTURE_DIR (`mecha exp` does) or pass --store; without it the service has no state and no face.", file=sys.stderr)
        return 2
    try:
        with open(os.path.join(store, world.WORLD)) as f:
            face = f.read().strip()
    except OSError:
        face = ""
    if face not in world.FACES:
        print("twins_server.py: the store names no world. The task source writes it before each task.", file=sys.stderr)
        return 2
    tools, handlers = build(store, face)
    serve(tools, handlers, face)
    return 0


if __name__ == "__main__":
    sys.exit(main())
