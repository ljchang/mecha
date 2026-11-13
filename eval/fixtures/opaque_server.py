#!/usr/bin/env python3
"""A fixture service whose calling rule and units are learned only by using
it (`opaque_world.py`). `--world ledger|scope` picks the surface. Stateless
apart from a count of refused checks, kept in `$MECHA_FIXTURE_DIR` so it
survives the per-task restart of the server within one lifetime.

Newline-delimited JSON-RPC, stdlib only.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import opaque_world  # noqa: E402


class ToolError(Exception):
    pass


def build(world_name, store):
    world = opaque_world.WORLDS[world_name]
    names = world["tools"]
    by_id = {ident: name for name, (ident, _) in world["accounts"].items()}

    def help_(args):
        topic = str(args.get("topic", "")).strip().lower()
        if topic not in world["topics"]:
            raise ToolError(f"no such topic: {topic!r}. {world['topics']['']}")
        return world["topics"][topic]

    def lookup(args):
        name = str(args.get("name", "")).strip()
        if name not in world["accounts"]:
            raise ToolError(f"no {world['noun']} named {name!r}")
        ident, cls = world["accounts"][name]
        return json.dumps({"id": ident, "class": cls})

    def parse(ident, tok):
        name = by_id.get(ident)
        if name is None:
            raise ToolError("E17: request rejected")
        for year in (2025, 2026, 2027):
            for period in range(1, 14):
                if opaque_world.token(world_name, name, year, period) == tok:
                    return name, year, period
        raise ToolError("E17: request rejected")

    def export(args):
        name, year, period = parse(str(args.get("id", "")), str(args.get("token", "")))
        return json.dumps({"rows": opaque_world.rows(world_name, name, year, period)})

    def check(args):
        name, year, period = parse(str(args.get("id", "")), str(args.get("token", "")))
        try:
            claimed = float(args.get("total"))
        except (TypeError, ValueError):
            raise ToolError("total must be a number")
        if abs(claimed - opaque_world.total_major(world_name, name, year, period)) < 1e-6:
            return "confirmed"
        path = os.path.join(store, "refused-checks") if store else None
        refused = 0
        if path:
            if os.path.exists(path):
                refused = int(open(path).read().strip() or 0)
            refused += 1
            with open(path, "w") as f:
                f.write(str(refused))
        if refused >= 2:
            raise ToolError(f"mismatch. Exported amounts are in {world['minor_words']}; a total is given in {world['major_words']}.")
        raise ToolError("mismatch")

    string = {"type": "string"}
    tools = [
        {"name": names["help"], "description": "Read the service's documentation by topic. An empty topic lists the topics.", "inputSchema": {"type": "object", "properties": {"topic": string}}},
        {"name": names["lookup"], "description": f"Find an {world['noun']} by its name.", "inputSchema": {"type": "object", "properties": {"name": string}, "required": ["name"]}},
        {"name": names["export"], "description": f"Fetch the rows for an {world['noun']} id and a {world['period_words']} token.", "inputSchema": {"type": "object", "properties": {"id": string, "token": string}, "required": ["id", "token"]}},
        {"name": names["check"], "description": f"Check a total for an {world['noun']} id and a {world['period_words']} token.", "inputSchema": {"type": "object", "properties": {"id": string, "token": string, "total": {"type": "number"}}, "required": ["id", "token", "total"]}},
    ]
    handlers = {names["help"]: help_, names["lookup"]: lookup, names["export"]: export, names["check"]: check}
    return tools, handlers


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def serve(tools, handlers, label):
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        rid, method = message.get("id"), message.get("method")
        if rid is None:
            continue
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": f"{label}-fixture", "version": "0"}}})
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": rid, "result": {"tools": tools}})
        elif method == "tools/call":
            params = message.get("params") or {}
            handler = handlers.get(params.get("name"))
            if handler is None:
                send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"no such tool: {params.get('name')}"}})
                continue
            try:
                text, err = handler(params.get("arguments") or {}), False
            except ToolError as e:
                text, err = str(e), True
            except Exception as e:  # noqa: BLE001 — a fixture absorbs whatever a model sends
                text, err = f"{type(e).__name__}: {e}", True
            result = {"content": [{"type": "text", "text": text}]}
            if err:
                result["isError"] = True
            send({"jsonrpc": "2.0", "id": rid, "result": result})
        else:
            send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"unsupported method: {method}"}})


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--world", required=True, choices=sorted(opaque_world.WORLDS))
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    store = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if store and not os.path.isdir(store):
        store = None
    tools, handlers = build(opts.world, store)
    serve(tools, handlers, opts.world)
    return 0


if __name__ == "__main__":
    sys.exit(main())
