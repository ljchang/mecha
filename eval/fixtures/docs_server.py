#!/usr/bin/env python3
"""A fixture Google Docs server: `mecha-docs`' document tools over a seeded
store, with every write recorded and nothing reaching Google.

Same tool definitions and answer texts as the real server's document half —
`docs_create`, `docs_append`, `docs_replace`, `docs_read`, `docs_list`,
`docs_trash` — so a run and the drafts it stages look exactly as they would
against the real one. Both are measured, not promised: `mecha-mail`'s
`the_docs_fixture_serves_the_real_definitions` and
`the_docs_fixture_answers_in_the_real_sentences`. Sheets and slides are not served: no suite
uses them, and a tool a fixture answers wrongly is worse than one it lacks.

State lives in `$MECHA_FIXTURE_DIR` (or `--store`), seeded once by `mecha exp`:

    docs.json      {"v": 1, "next": 1, "docs": [{"id", "title", "body", "trashed"}]}
    writes.jsonl   one line per create / append / replace / trash

Fail-closed: no store directory is a refusal to start. Newline-delimited
JSON-RPC, stdlib only, fictional cast only — this file is public.
"""

import argparse
import json
import os
import sys
import tempfile


class ToolError(Exception):
    pass


def rust_debug(text):
    """A string as Rust's `{:?}` prints it, which is how the real answers
    quote a title or an anchor."""
    escaped = text.replace("\\", "\\\\").replace('"', '\\"')
    escaped = escaped.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
    return f'"{escaped}"'


def write_atomic(path, value):
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".docs-")
    with os.fdopen(fd, "w") as f:
        json.dump(value, f, indent=1, sort_keys=True)
    os.replace(tmp, path)


class Store:
    def __init__(self, root):
        self.path = os.path.join(root, "docs.json")
        self.writes = os.path.join(root, "writes.jsonl")
        if os.path.exists(self.path):
            with open(self.path) as f:
                self.data = json.load(f)
        else:
            self.data = {"v": 1, "next": 1, "docs": []}

    def save(self):
        write_atomic(self.path, self.data)

    def record(self, tool, args, produced):
        with open(self.writes, "a") as f:
            f.write(json.dumps({"tool": tool, "args": args, "produced": produced}, sort_keys=True) + "\n")

    def get(self, file_id):
        for d in self.data["docs"]:
            if d["id"] == file_id and not d.get("trashed"):
                return d
        raise ToolError(f"no document `{file_id}` (docs_list shows the ids)")


def str_arg(args, key, required=False):
    v = args.get(key)
    if v is None or (isinstance(v, str) and not v.strip()):
        if required:
            raise ToolError(f"`{key}` is required")
        return None
    if not isinstance(v, str):
        raise ToolError(f"`{key}` must be a string")
    return v


def docs_create(store, args):
    title = str_arg(args, "title", required=True)
    body = str_arg(args, "body") or ""
    n = store.data["next"]
    store.data["next"] = n + 1
    doc = {"id": f"doc-{n:04d}", "title": title, "body": body, "trashed": False}
    store.data["docs"].append(doc)
    store.save()
    store.record("docs_create", args, {"file_id": doc["id"]})
    return f"created {rust_debug(title)} [{doc['id']}]" + (" with its body" if body else "")


def docs_append(store, args):
    doc = store.get(str_arg(args, "file_id", required=True))
    text = str_arg(args, "text", required=True)
    doc["body"] = (doc["body"] + ("\n" if doc["body"] and not doc["body"].endswith("\n") else "") + text)
    store.save()
    store.record("docs_append", args, {"file_id": doc["id"]})
    return f"appended {len(text.encode())} characters"


def docs_replace(store, args):
    doc = store.get(str_arg(args, "file_id", required=True))
    find = str_arg(args, "find", required=True)
    replace = args.get("replace")
    if not isinstance(replace, str):
        raise ToolError("`replace` is required (an empty string deletes)")
    if args.get("match_case", True):
        count = doc["body"].count(find)
        doc["body"] = doc["body"].replace(find, replace)
    else:
        lower, needle = doc["body"].lower(), find.lower()
        count, out, i = 0, [], 0
        while True:
            j = lower.find(needle, i)
            if j < 0:
                break
            out.append(doc["body"][i:j] + replace)
            i = j + len(find)
            count += 1
        out.append(doc["body"][i:])
        doc["body"] = "".join(out)
    store.save()
    store.record("docs_replace", args, {"file_id": doc["id"], "replaced": count})
    if count == 0:
        # Zero is not success, as on the real server: a model told "ok" goes
        # on to report an edit that never happened.
        return f"no occurrences of {rust_debug(find)} found — nothing was changed. Read the document and quote its exact wording."
    return f"replaced {count} occurrence(s)"


def docs_read(store, args):
    doc = store.get(str_arg(args, "file_id", required=True))
    return f"# {doc['title']}\n\n{doc['body']}"


def docs_list(store, args):
    rows = [f"{'doc':7} {d['title']}  [{d['id']}]" for d in store.data["docs"] if not d.get("trashed")]
    if not rows:
        return "Nothing is in scope yet. Documents you create here become reachable automatically; existing ones must be added by the user with `mecha-docs pick`."
    return "\n".join(rows)


def docs_trash(store, args):
    doc = store.get(str_arg(args, "file_id", required=True))
    doc["trashed"] = True
    store.save()
    store.record("docs_trash", args, {"file_id": doc["id"]})
    return f"moved {doc['id']} to the Drive trash; it can be restored there"


# Verbatim from `mecha-mail/src/google/docs_server.rs::tool_definitions()`
# for the six docs tools; `mecha-mail`'s `the_docs_fixture_serves_the_real_definitions`
# fails on any drift, because a fixture described differently from the real
# server measures a harness nobody runs.
FILE_ID = {"type": "string"}
TOOLS = [
    {
        "name": "docs_list",
        "description": "List every Google Doc, Sheet, Slides deck and folder mecha can reach. This is the whole of what it can touch: files it created, plus files the user added with `mecha-docs pick`. A document not listed here is not reachable and cannot be made reachable from inside a run.",
        "inputSchema": {"type": "object", "properties": {}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "docs_read",
        "description": "Read a Google Doc's text by file id (from docs_list). Returns the title and the body as plain text, with tables flattened to pipe-separated rows.",
        "inputSchema": {"type": "object", "properties": {"file_id": FILE_ID}, "required": ["file_id"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "docs_create",
        "description": "Create a new Google Doc with a title, and optionally an initial body. Returns its file id. Anything mecha creates is reachable from then on with no further permission step.",
        "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}, "body": {"type": "string"}}, "required": ["title"]},
        "annotations": {"openWorldHint": False, "readOnlyHint": False},
    },
    {
        "name": "docs_append",
        "description": "Append text to the end of a Google Doc. Use for adding a section or a note; use docs_replace to change text that is already there.",
        "inputSchema": {"type": "object", "properties": {"file_id": FILE_ID, "text": {"type": "string"}}, "required": ["file_id", "text"]},
        "annotations": {"openWorldHint": True},
    },
    {
        "name": "docs_replace",
        "description": "Replace every occurrence of some text in a Google Doc. This is the surgical edit: quote enough of the surrounding wording in `find` to be unambiguous. Reports how many occurrences changed, and zero means the anchor text was not found and nothing was edited.",
        "inputSchema": {
            "type": "object",
            "properties": {"file_id": FILE_ID, "find": {"type": "string"}, "replace": {"type": "string"}, "match_case": {"type": "boolean", "default": True}},
            "required": ["file_id", "find", "replace"],
        },
        "annotations": {"openWorldHint": True},
    },
    {
        "name": "docs_trash",
        "description": "Move a file mecha can reach to the user's Drive trash, where they can restore it. There is deliberately no permanent-delete verb.",
        "inputSchema": {"type": "object", "properties": {"file_id": FILE_ID}, "required": ["file_id"]},
        "annotations": {"destructiveHint": True},
    },
]
HANDLERS = {
    "docs_create": docs_create,
    "docs_append": docs_append,
    "docs_replace": docs_replace,
    "docs_read": docs_read,
    "docs_list": docs_list,
    "docs_trash": docs_trash,
}


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def serve(store):
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
            send({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "docs-fixture", "version": "0"}}})
        elif method == "tools/list":
            send({"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}})
        elif method == "tools/call":
            params = message.get("params") or {}
            handler = HANDLERS.get(params.get("name"))
            if handler is None:
                send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"no such tool: {params.get('name')}"}})
                continue
            try:
                text, err = handler(store, params.get("arguments") or {}), False
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
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    opts = parser.parse_args()
    root = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not root or not os.path.isdir(root):
        print("docs_server.py: no store directory — `mecha exp` creates and seeds it; a docs store that forgets is not a fixture", file=sys.stderr)
        return 2
    serve(Store(root))
    return 0


if __name__ == "__main__":
    sys.exit(main())
