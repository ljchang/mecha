#!/usr/bin/env python3
"""A fixture knowledge-graph server with a board that *remembers*.

`graph_server.py` is deliberately stateless — an eval case must not leak into
the next — and that is exactly wrong for a lifetime (docs/EXPERIMENT-DESIGN.md
Part II §14, §17 item 4), where what one task's run left on the board is what
the next task and the principal start from. This server is the stateful
stand-in for the real `mecha-graph-mcp`: the GTD board over `kg_task_list` /
`kg_task_create` / `kg_task_update` with the real server's argument and answer
shapes (what `mecha tasks` parses), the canned-graph reads
(`kg_search` / `kg_entity` / `kg_related` / `kg_timeline`) over a seeded
entity set, and `kg_upsert`, which records what a run tried to put into memory
without letting it in — the review queue, as a file.

State lives in the directory `$MECHA_FIXTURE_DIR` names (or `--store`), which
`mecha exp` sets to `<trial home>/fixtures/<server name>/` and seeds once
from the manifest's `seed` directory:

    board.json      {"v": 1, "next": <int>, "tasks": [ ... ]}
    entities.json   {"entities": {...}, "facts": {...}, "related": {...}, "timeline": {...}}
    staged.jsonl    one line per kg_upsert

A seed task may carry `due_in_days` / `defer_in_days` instead of a date; they
are resolved against the clock the first time the store is read, so a seeded
board is overdue *relative to the run*, not to the day the seed was written.

Fail-closed: no store directory is a refusal to start, never a board that
forgets. Speaks the newline-delimited JSON-RPC dialect mecha's client uses.
Stdlib only. Fictional cast only — this file is public.
"""

import argparse
import datetime as dt
import json
import os
import sys
import tempfile

ACTIONABLE = ["next", "inbox", "scheduled", "waiting"]
STATUSES = ACTIONABLE + ["done", "dropped"]
CAPTURED_KINDS = {"mail", "frontdoor", "session"}
CAPTURED_KEYS = {"kind", "id", "account", "label", "at"}
AGENT = "mecha"
OWNER = "@owner"


# --- the store ---------------------------------------------------------------


def today():
    return dt.datetime.now(dt.timezone.utc).date().isoformat()


def now():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def write_atomic(path, value):
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-", suffix=".json")
    with os.fdopen(fd, "w") as f:
        json.dump(value, f, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


class Store:
    def __init__(self, root, ephemeral):
        self.root = root
        self.ephemeral = ephemeral
        self.board_path = os.path.join(root, "board.json")
        self.entities_path = os.path.join(root, "entities.json")
        self.staged_path = os.path.join(root, "staged.jsonl")
        self.board = self._load(self.board_path, {"v": 1, "next": 1, "tasks": []})
        self.graph = self._load(
            self.entities_path, {"entities": {}, "facts": {}, "related": {}, "timeline": {}}
        )
        if self._resolve_relative_dates():
            self.save_board()

    def _load(self, path, default):
        if not os.path.exists(path):
            return default
        with open(path) as f:
            return json.load(f)

    def _resolve_relative_dates(self):
        """A seed's `due_in_days` becomes a date the first time it is read."""
        changed = False
        base = dt.date.fromisoformat(today())
        for t in self.board["tasks"]:
            for rel, absolute in (("due_in_days", "due_at"), ("defer_in_days", "defer_until")):
                if rel in t:
                    t[absolute] = (base + dt.timedelta(days=int(t.pop(rel)))).isoformat()
                    changed = True
            for key, default in (
                ("status", "inbox"),
                ("due_at", None),
                ("defer_until", None),
                ("context", None),
                ("project", None),
                ("waiting_on", None),
                ("about", []),
                ("previously_waiting_on", None),
                ("unreadable_when", None),
                ("session", None),
                ("completed_at", None),
                ("captured_from", None),
            ):
                if key not in t:
                    t[key] = default
                    changed = True
            if "created_at" not in t:
                t["created_at"] = now()
                changed = True
        return changed

    def save_board(self):
        if self.ephemeral:
            return
        write_atomic(self.board_path, self.board)

    def stage(self, record):
        if self.ephemeral:
            return
        with open(self.staged_path, "a") as f:
            f.write(json.dumps(record, sort_keys=True) + "\n")

    # -- names -----------------------------------------------------------------

    def resolve_node(self, name):
        """An entity by id or name (case-insensitive), or None."""
        if not isinstance(name, str):
            return None
        wanted = name.strip().lower()
        if not wanted:
            return None
        for eid, e in self.graph["entities"].items():
            if eid.lower() == wanted or e.get("name", "").lower() == wanted:
                return e
        return None

    def resolve_about(self, name):
        """What `about`, `project` and `waiting_on` accept: a node, the
        owner sentinel, or the agent. Unknown is an error at the caller."""
        if name in (OWNER, AGENT):
            return {"id": name, "name": name}
        return self.resolve_node(name)

    def task(self, task_id):
        for t in self.board["tasks"]:
            if t["id"] == task_id:
                return t
        return None


class ToolError(Exception):
    pass


# --- dates ---------------------------------------------------------------------


def parse_due(raw):
    """YYYY-MM-DD, 'today', 'tomorrow', '+Nd'; '' clears (None)."""
    if raw is None:
        return None
    raw = raw.strip()
    if raw == "":
        return None
    base = dt.date.fromisoformat(today())
    if raw == "today":
        return base.isoformat()
    if raw == "tomorrow":
        return (base + dt.timedelta(days=1)).isoformat()
    if raw.startswith("+") and raw.endswith("d") and raw[1:-1].isdigit():
        return (base + dt.timedelta(days=int(raw[1:-1]))).isoformat()
    try:
        return dt.date.fromisoformat(raw).isoformat()
    except ValueError:
        raise ToolError(f"unreadable date `{raw}`: use YYYY-MM-DD, 'today', 'tomorrow' or '+Nd'")


# --- the board ------------------------------------------------------------------


def task_json(t, today_str):
    overdue = bool(t.get("due_at")) and t["due_at"] < today_str and t.get("completed_at") is None
    return {
        "id": t["id"],
        "name": t["name"],
        "status": t["status"],
        "due_at": t.get("due_at"),
        "defer_until": t.get("defer_until"),
        "context": t.get("context"),
        "project": t.get("project"),
        "waiting_on": t.get("waiting_on"),
        "about": t.get("about", []),
        "previously_waiting_on": t.get("previously_waiting_on"),
        "unreadable_when": t.get("unreadable_when"),
        "session": t.get("session"),
        "completed_at": t.get("completed_at"),
        "captured_from": t.get("captured_from"),
        "overdue": overdue,
    }


def order_key(t):
    # Actionable statuses first, in the real server's order, then by due date
    # (undated last), then by creation.
    rank = ACTIONABLE.index(t["status"]) if t["status"] in ACTIONABLE else len(ACTIONABLE)
    return (rank, t.get("due_at") or "9999-12-31", t.get("created_at", ""))


def kg_task_list(store, args):
    include_closed = bool(args.get("include_closed", False))
    entity = args.get("entity")
    node = None
    if isinstance(entity, str) and entity.strip():
        node = store.resolve_about(entity)
        if node is None:
            raise ToolError(
                f"no node matches '{entity}' — kg_entity resolves names, and an unknown one is not an empty task list"
            )
    tasks = [t for t in store.board["tasks"] if include_closed or t["status"] not in ("done", "dropped")]
    if node is not None:
        want = {node["id"].lower(), node["name"].lower()}

        def touches(t):
            names = {a.get("name", "").lower() for a in t.get("about", [])}
            if t.get("waiting_on"):
                names.add(t["waiting_on"].lower())
            if t.get("project"):
                names.add(t["project"].lower())
            return bool(names & want)

        tasks = [t for t in tasks if touches(t)]
    tasks.sort(key=order_key)
    td = today()
    out = {"v": 1, "items": [task_json(t, td) for t in tasks], "today": td, "truncated": False}
    if node is not None:
        out["entity"] = {"id": node["id"], "name": node["name"]}
    return out


def name_array(args, key):
    value = args.get(key)
    if value is None:
        return []
    if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
        raise ToolError(f"`{key}` must be a list of names")
    return value


def check_captured_from(value):
    if not isinstance(value, dict):
        raise ToolError("`captured_from` is an object: {kind, id, account?, label?, at?}")
    unknown = set(value) - CAPTURED_KEYS
    if unknown:
        raise ToolError(f"`captured_from` has keys this pointer does not carry: {sorted(unknown)}")
    if value.get("kind") not in CAPTURED_KINDS:
        raise ToolError(f"`captured_from.kind` must be one of {sorted(CAPTURED_KINDS)}")
    if not isinstance(value.get("id"), str) or not value["id"]:
        raise ToolError("`captured_from.id` names which one")
    if value["kind"] == "mail" and not value.get("account"):
        raise ToolError("`captured_from.account` is required for kind 'mail': thread ids are account-scoped")


def kg_task_create(store, args):
    name = args.get("name")
    if not isinstance(name, str) or not name.strip():
        raise ToolError("kg_task_create needs `name`")
    due = parse_due(args.get("due")) if args.get("due") is not None else None
    project = args.get("project")
    if project is not None:
        if store.resolve_node(project) is None:
            raise ToolError(f"project `{project}` names no node in the graph — no task was created")
    about = name_array(args, "about")
    for a in about:
        if store.resolve_about(a) is None:
            raise ToolError(f"`about` names `{a}`, which is not a node in the graph — no task was created")
    captured = args.get("captured_from")
    if captured is not None:
        check_captured_from(captured)
    n = store.board["next"]
    store.board["next"] = n + 1
    task = {
        "id": f"task-fx{n:04d}",
        "name": name.strip(),
        "status": "inbox",
        "due_at": due,
        "defer_until": None,
        "context": args.get("context"),
        "project": store.resolve_node(project)["name"] if project is not None else None,
        "waiting_on": None,
        "about": [{"name": store.resolve_about(a)["name"], "unreviewed": False} for a in about],
        "previously_waiting_on": None,
        "unreadable_when": None,
        "session": None,
        "completed_at": None,
        "captured_from": captured,
        "created_at": now(),
    }
    store.board["tasks"].append(task)
    store.save_board()
    return {"v": 1, "status": "created", "id": task["id"], "due_at": due, "about": task["about"]}


def kg_task_update(store, args):
    task_id = args.get("task")
    if not isinstance(task_id, str):
        raise ToolError("kg_task_update needs `task`")
    t = store.task(task_id)
    if t is None:
        raise ToolError(f"no such task: {task_id}")
    # Validate everything before the first write, so "nothing was changed" is
    # true on a refusal (the real server's rule).
    status = args.get("status")
    if status is not None and status not in STATUSES:
        raise ToolError(f"unknown status `{status}`; one of {STATUSES}")
    to_add = name_array(args, "about_add")
    to_remove = name_array(args, "about_remove")
    for a in to_add:
        if store.resolve_about(a) is None:
            raise ToolError(f"`about_add` names `{a}`, which is not a node in the graph — nothing was changed")
    who = args.get("waiting_on")
    if isinstance(who, str) and who.strip() and store.resolve_about(who) is None:
        raise ToolError(f"waiting_on `{who}` is nobody the graph knows — nothing was changed")
    due = parse_due(args["due"]) if isinstance(args.get("due"), str) else "untouched"
    defer = parse_due(args["defer"]) if isinstance(args.get("defer"), str) else "untouched"
    captured = args.get("captured_from")
    if isinstance(captured, dict):
        check_captured_from(captured)

    if status is not None:
        was_closed = t["status"] in ("done", "dropped")
        t["status"] = status
        if status in ("done", "dropped"):
            t["completed_at"] = now()
            if t.get("waiting_on"):
                t["previously_waiting_on"] = t["waiting_on"]
                t["waiting_on"] = None
        elif was_closed:
            t["completed_at"] = None
    if due != "untouched":
        t["due_at"] = due
    if defer != "untouched":
        t["defer_until"] = defer
    if isinstance(args.get("context"), str):
        t["context"] = args["context"].strip() or None
    if isinstance(who, str):
        t["waiting_on"] = store.resolve_about(who)["name"] if who.strip() else None
    if isinstance(args.get("session"), str):
        t["session"] = args["session"] or None
    for a in to_add:
        nm = store.resolve_about(a)["name"]
        if not any(x.get("name") == nm for x in t["about"]):
            t["about"].append({"name": nm, "unreviewed": False})
    for a in to_remove:
        t["about"] = [x for x in t["about"] if x.get("name", "").lower() != a.lower()]
    if captured is not None:
        t["captured_from"] = None if captured == "" else captured
    store.save_board()
    return {"v": 1, "status": "updated", "task": task_json(t, today())}


# --- the canned graph -------------------------------------------------------------


def entity_text(store, entity_id):
    e = store.graph["entities"].get(entity_id)
    if e is None:
        return None
    body = {k: e.get(k) for k in ("id", "kind", "name", "summary")}
    body["facts"] = store.graph["facts"].get(entity_id, [])
    return body


def kg_search(store, args):
    query = args.get("query", "") if isinstance(args.get("query"), str) else ""
    tokens = [t for t in query.lower().split() if len(t) > 2]
    hits = []
    for e in store.graph["entities"].values():
        haystack = [k.lower() for k in e.get("keywords", [])] + e.get("name", "").lower().split()
        if any(any(t in h for h in haystack) for t in tokens):
            hits.append(e)
    # Several people sharing a name token the query used, with nothing in the
    # query telling them apart: report the ambiguity rather than picking.
    if len(hits) > 1 and all(e.get("kind") == "person" for e in hits):
        shared = None
        for t in tokens:
            if all(t in e.get("name", "").lower().split() for e in hits):
                shared = t
        if shared is not None:
            return {
                "results": [],
                "ambiguous": [
                    {k: e.get(k) for k in ("id", "name", "summary")} for e in hits
                ],
            }
    if not hits:
        return {"results": [], "ambiguous": [], "note": "no results"}
    body = {
        "results": [{k: e.get(k) for k in ("id", "kind", "name", "summary")} for e in hits],
        "ambiguous": [],
    }
    flags = []
    for e in hits:
        by_pred = {}
        for f in store.graph["facts"].get(e["id"], []):
            if f.get("polarity", "positive") == "positive":
                by_pred.setdefault(f.get("predicate"), []).append(f)
        for pred, fs in by_pred.items():
            if pred == "due" and len(fs) > 1:
                flags.append(
                    {
                        "kind": "contradiction",
                        "subject_id": e["id"],
                        "predicate": pred,
                        "detail": f"{len(fs)} live values on single-valued '{pred}'",
                        "fact_uids": [f.get("uid") for f in fs],
                    }
                )
    if flags:
        body["flags"] = flags
    return body


def kg_entity(store, args):
    body = entity_text(store, args.get("id", ""))
    if body is None:
        return {"error": "no such entity", "known": sorted(store.graph["entities"])}
    return body


def kg_related(store, args):
    ids = store.graph["related"].get(args.get("id", ""))
    if ids is None:
        return {"error": "no such entity", "known": sorted(store.graph["entities"])}
    return {"related": [entity_text(store, i) for i in ids]}


def kg_timeline(store, args):
    episodes = store.graph["timeline"].get(args.get("id", ""))
    if episodes is None:
        return {"error": "no such entity", "known": sorted(store.graph["entities"])}
    return {"episodes": episodes}


def kg_upsert(store, args):
    record = {
        "at": now(),
        "kind": args.get("kind", "fact"),
        "content": args.get("content", ""),
        "source": args.get("source", ""),
    }
    store.stage(record)
    return {
        "staged": {k: record[k] for k in ("kind", "content", "source")},
        "note": "staged for the user's review; not yet in the graph",
    }


TOOLS = [
    {
        "name": "kg_task_list",
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
        "description": "The GTD board: every open task, actionable statuses first (next, inbox, scheduled, waiting), then by due date. Each task carries its status, due/defer dates, parent project, who it is waiting on, the entities it is `about`, and — when captured from something — a `captured_from` pointer. Use it to answer 'what should Ada do next', to check whether something is already tracked, and to find overdue items (due_at earlier than today). include_closed adds done/dropped history. `entity` narrows to one person, project or topic.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "include_closed": {"type": "boolean", "description": "Also return done/dropped tasks (default false)"},
                "entity": {"type": "string", "description": "Only tasks associated with this person, project or topic, by name or node id. An unknown name is an error, not an empty list."},
            },
        },
    },
    {
        "name": "kg_task_create",
        "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False},
        "description": "Capture a task. Lands in 'inbox' status. Check kg_task_list first so the board does not collect duplicates. `project` must name an existing graph node — an unknown name is an error, not an implicit node.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The task, phrased as an action"},
                "due": {"type": "string", "description": "YYYY-MM-DD, 'today', 'tomorrow', or '+Nd'"},
                "project": {"type": "string", "description": "Parent project/topic — must resolve to an existing node"},
                "context": {"type": "string", "description": "GTD context tag, e.g. '@email', '@lab'"},
                "about": {"type": "array", "items": {"type": "string"}, "description": "People, projects or topics this task concerns, by name. Each must already resolve to a node."},
                "captured_from": {
                    "type": "object",
                    "description": "What prompted this task — a pointer, never a copy.",
                    "properties": {
                        "kind": {"type": "string", "enum": ["mail", "frontdoor", "session"]},
                        "id": {"type": "string"},
                        "account": {"type": "string"},
                        "label": {"type": "string"},
                        "at": {"type": "string"},
                    },
                    "required": ["kind", "id"],
                },
            },
            "required": ["name"],
        },
    },
    {
        "name": "kg_task_update",
        "annotations": {"readOnlyHint": False, "destructiveHint": False, "openWorldHint": False},
        "description": "Move a task through its lifecycle (status: next|inbox|scheduled|waiting|done|dropped) and/or edit its scheduling. 'done'/'dropped' stamp completed_at; reopening clears it. For due/defer/context/waiting_on: omit the field to leave it untouched, pass \"\" to clear it. Takes the task's id from kg_task_list.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {"type": "string"},
                "status": {"type": "string", "enum": ["next", "inbox", "scheduled", "waiting", "done", "dropped"]},
                "due": {"type": "string"},
                "defer": {"type": "string"},
                "context": {"type": "string"},
                "waiting_on": {"type": "string", "description": "Who has the ball — a person or agent the graph already knows, by name; '@owner' means whoever this graph is about; \"\" clears."},
                "about_add": {"type": "array", "items": {"type": "string"}},
                "about_remove": {"type": "array", "items": {"type": "string"}},
                "session": {"type": "string", "description": "The agent conversation working this task. Set by the harness — do not invent a value; \"\" clears."},
                "captured_from": {"description": "Same object kg_task_create takes; \"\" clears."},
            },
            "required": ["task"],
        },
    },
    {
        "name": "kg_search",
        "description": "Search the personal knowledge graph for people, projects, facts and episodes.",
        "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "kg_entity",
        "description": "Fetch one entity by id.",
        "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "kg_related",
        "description": "Entities connected to the given entity id.",
        "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "kg_timeline",
        "description": "Episodes involving the given entity id, most recent first.",
        "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "kg_upsert",
        "description": "Stage a fact or alias for the user to review. It does not enter the graph until they accept it. Always pass source.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["fact", "alias"]},
                "content": {"type": "string"},
                "source": {"type": "string"},
            },
            "required": ["content"],
        },
    },
]

HANDLERS = {
    "kg_task_list": kg_task_list,
    "kg_task_create": kg_task_create,
    "kg_task_update": kg_task_update,
    "kg_search": kg_search,
    "kg_entity": kg_entity,
    "kg_related": kg_related,
    "kg_timeline": kg_timeline,
    "kg_upsert": kg_upsert,
}


# --- the wire ------------------------------------------------------------------------


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def reply(request_id, result):
    send({"jsonrpc": "2.0", "id": request_id, "result": result})


def fail(request_id, message):
    send({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": message}})


def text_result(text, is_error=False):
    out = {"content": [{"type": "text", "text": text}]}
    if is_error:
        out["isError"] = True
    return out


def serve(store):
    while True:
        line = sys.stdin.readline()
        if not line:
            return
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        request_id = message.get("id")
        method = message.get("method")
        if request_id is None:
            continue
        if method == "initialize":
            reply(
                request_id,
                {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "board-fixture", "version": "0"},
                },
            )
        elif method == "tools/list":
            reply(request_id, {"tools": TOOLS})
        elif method == "tools/call":
            params = message.get("params") or {}
            name = params.get("name")
            handler = HANDLERS.get(name)
            if handler is None:
                fail(request_id, f"no such tool: {name}")
                continue
            try:
                value = handler(store, params.get("arguments") or {})
                reply(request_id, text_result(json.dumps(value)))
            except ToolError as e:
                reply(request_id, text_result(str(e), is_error=True))
        else:
            fail(request_id, f"unsupported method: {method}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    parser.add_argument(
        "--ephemeral",
        action="store_true",
        help="read the store, never write it — for an eval whose cases must not leak into each other",
    )
    opts = parser.parse_args()
    root = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not root:
        print(
            "board_server.py: no store — set MECHA_FIXTURE_DIR (mecha exp does) or pass --store; "
            "a board that forgets is not a fixture",
            file=sys.stderr,
        )
        return 2
    if not os.path.isdir(root):
        if opts.ephemeral:
            print(f"board_server.py: {root} is not a directory", file=sys.stderr)
            return 2
        os.makedirs(root, exist_ok=True)
    serve(Store(root, opts.ephemeral))
    return 0


if __name__ == "__main__":
    sys.exit(main())
