#!/usr/bin/env python3
"""A fixture mail-and-calendar server: `mecha-mail`'s unified surface over a
seeded mailbox, with every send recorded and nothing delivered.

The real server sends from a real account. Inside a trial home that is the
one effect isolation cannot contain, which is why the principal could never
release a draft (docs/EXPERIMENT-DESIGN.md §16, §21.1). This server exposes
the same tools with the same argument shapes and answer texts — `mail_search`
rows, `mail_get_thread`'s rendering, `mail_send`'s "sent (message id …)" —
so a run and its staged drafts look exactly as they would against the real
one, and a release lands in `sent.jsonl` under the trial home instead of in
somebody's inbox.

State lives in `$MECHA_FIXTURE_DIR` (or `--store`), seeded once by `mecha exp`:

    mailbox.json    {"v": 1, "accounts": [...], "threads": [...]}
    calendar.json   {"v": 1, "events": [...]}
    sent.jsonl      one line per mail_send / mail_reply / calendar mutation
    triage.jsonl    one line per mail_triage

A seed message may carry `days_ago` and an event `days_ahead` (plus
`start_hour`/`minutes`) instead of absolute times; they are resolved against
the clock the first time the store is read, so a seeded mailbox is *recent*
relative to the run. Accounts: `mail_send` with no `account` uses the one
marked `"default": true`, else the only account, else fails and says to ask —
the real server's rule, which is a case the appraisal cares about.

Fail-closed: no store directory is a refusal to start. Newline-delimited
JSON-RPC, stdlib only, fictional cast only — this file is public.
"""

import argparse
import datetime as dt
import json
import os
import sys
import tempfile

TRIAGE_ACTIONS = ["archive", "read", "unread", "spam", "trash"]


def now_dt():
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0)


def iso(d):
    return d.isoformat().replace("+00:00", "Z")


def write_atomic(path, value):
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-", suffix=".json")
    with os.fdopen(fd, "w") as f:
        json.dump(value, f, indent=2, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


class ToolError(Exception):
    pass


class Store:
    def __init__(self, root, ephemeral):
        self.root = root
        self.ephemeral = ephemeral
        self.mailbox_path = os.path.join(root, "mailbox.json")
        self.calendar_path = os.path.join(root, "calendar.json")
        self.sent_path = os.path.join(root, "sent.jsonl")
        self.triage_path = os.path.join(root, "triage.jsonl")
        self.mailbox = self._load(self.mailbox_path, {"v": 1, "accounts": [], "threads": [], "next": 1})
        self.calendar = self._load(self.calendar_path, {"v": 1, "events": [], "next": 1})
        if "next" not in self.mailbox:
            self.mailbox["next"] = 1
        if "next" not in self.calendar:
            self.calendar["next"] = 1
        if self._resolve_relative_times():
            self.save()

    def _load(self, path, default):
        if not os.path.exists(path):
            return default
        with open(path) as f:
            return json.load(f)

    def _resolve_relative_times(self):
        changed = False
        base = now_dt()
        for t in self.mailbox["threads"]:
            for key, default in (("archived", False), ("trashed", False), ("spam", False)):
                if key not in t:
                    t[key] = default
                    changed = True
            for m in t.get("messages", []):
                if "days_ago" in m:
                    hour = int(m.pop("hour", 9))
                    when = (base - dt.timedelta(days=int(m.pop("days_ago")))).replace(
                        hour=hour, minute=int(m.pop("minute", 0)), second=0
                    )
                    m["date"] = iso(when)
                    changed = True
                for key, default in (("unread", True), ("to", []), ("cc", []), ("bulk", False)):
                    if key not in m:
                        m[key] = default
                        changed = True
        for e in self.calendar["events"]:
            if "days_ahead" in e:
                day = (base + dt.timedelta(days=int(e.pop("days_ahead")))).replace(
                    hour=int(e.pop("start_hour", 10)), minute=0, second=0
                )
                minutes = int(e.pop("minutes", 60))
                e["start"] = iso(day)
                e["end"] = iso(day + dt.timedelta(minutes=minutes))
                changed = True
        return changed

    def save(self):
        if self.ephemeral:
            return
        write_atomic(self.mailbox_path, self.mailbox)
        write_atomic(self.calendar_path, self.calendar)

    def append(self, path, record):
        if self.ephemeral:
            return
        with open(path, "a") as f:
            f.write(json.dumps(record, sort_keys=True) + "\n")

    def record_send(self, tool, account, args, produced):
        self.append(
            self.sent_path,
            {"at": iso(now_dt()), "tool": tool, "account": account, "args": args, "produced": produced},
        )

    # -- accounts --------------------------------------------------------------

    def account(self, name):
        for a in self.mailbox["accounts"]:
            if a["name"] == name:
                return a
        return None

    def pick_read(self, name):
        """Every account, or the named one."""
        if name:
            a = self.account(name)
            if a is None:
                raise ToolError(f"no account `{name}`; configured: {', '.join(x['name'] for x in self.mailbox['accounts'])}")
            return [a]
        return list(self.mailbox["accounts"])

    def pick_item(self, name):
        """An id-carrying call: the named account, or the only one."""
        if name:
            return self.pick_read(name)[0]
        accounts = self.mailbox["accounts"]
        if len(accounts) == 1:
            return accounts[0]
        raise ToolError(
            "several accounts are configured; pass the `account` the id came from (it is in every search row)"
        )

    def pick_send(self, name):
        """A create: the named account, the default, or the only one — else
        fail and say to ask, exactly as the real server does."""
        if name:
            return self.pick_read(name)[0]
        accounts = self.mailbox["accounts"]
        defaults = [a for a in accounts if a.get("default")]
        if len(defaults) == 1:
            return defaults[0]
        if len(accounts) == 1:
            return accounts[0]
        raise ToolError(
            "no default account is set and several are configured — ask the user which account to send from, then pass `account`"
        )

    def thread(self, account, thread_id):
        for t in self.mailbox["threads"]:
            if t["id"] == thread_id and t["account"] == account["name"]:
                return t
        raise ToolError(f"no thread `{thread_id}` in account `{account['name']}`")

    def next_message_id(self):
        n = self.mailbox["next"]
        self.mailbox["next"] = n + 1
        return f"fx-{n:04d}"

    def next_event_id(self):
        n = self.calendar["next"]
        self.calendar["next"] = n + 1
        return f"ev-fx{n:04d}"


# --- rendering, on the real server's shapes ------------------------------------------


def row(account, thread, m):
    return {
        "account": account,
        "thread_id": thread["id"],
        "message_id": m["id"],
        "from": f"{m['from_name']} <{m['from_address']}>",
        "subject": thread["subject"],
        "date": m["date"],
        "snippet": (m.get("body", "").strip().splitlines() or [""])[0][:160],
        "unread": bool(m.get("unread", False)),
        "has_attachments": bool(m.get("attachments")),
        "bulk": bool(m.get("bulk", False)),
    }


def render_rows(rows):
    if not rows:
        return "no matching messages"
    rows.sort(key=lambda r: r["date"], reverse=True)
    return json.dumps(rows, indent=2)


def render_thread(account, thread):
    parts = []
    for m in thread["messages"]:
        parts.append(
            f"--- [{account}] From: {m['from_name']} <{m['from_address']}> · {m['date']}\n"
            f"Subject: {thread['subject']}\n"
            f"Message id (for mail_reply): {m['id']}\n\n{m.get('body', '').strip()}"
        )
    return "\n\n".join(parts)


def str_arg(args, key):
    v = args.get(key)
    return v if isinstance(v, str) and v != "" else None


def visible(thread, inbox_only):
    if thread.get("trashed") or thread.get("spam"):
        return False
    if inbox_only and thread.get("archived"):
        return False
    return True


# --- mail ----------------------------------------------------------------------------


def mail_search(store, args):
    query = str_arg(args, "query")
    if query is None:
        raise ToolError("mail_search needs query")
    limit = max(1, min(int(args.get("max_results", 10) or 10), 50))
    tokens = [t.lower() for t in query.split() if t and not t.lower().startswith(("after:", "received"))]
    filters = {}
    for t in list(tokens):
        for key in ("from:", "to:", "subject:"):
            if t.startswith(key):
                filters[key[:-1]] = t[len(key):]
                tokens.remove(t)
    rows = []
    for a in store.pick_read(str_arg(args, "account")):
        for th in store.mailbox["threads"]:
            if th["account"] != a["name"] or not visible(th, inbox_only=False):
                continue
            for m in th["messages"]:
                hay = " ".join([th["subject"], m["from_name"], m["from_address"], m.get("body", ""), " ".join(m.get("to", []))]).lower()
                if "from" in filters and filters["from"] not in (m["from_address"] + " " + m["from_name"]).lower():
                    continue
                if "to" in filters and not any(filters["to"] in x.lower() for x in m.get("to", [])):
                    continue
                if "subject" in filters and filters["subject"] not in th["subject"].lower():
                    continue
                if tokens and not all(t in hay for t in tokens):
                    continue
                rows.append(row(a["name"], th, m))
    # Newest first, then the cap — a cap before the sort returned whichever
    # matches came first in the file, not the most recent (found on review).
    rows.sort(key=lambda r: r["date"], reverse=True)
    return render_rows(rows[:limit])


def mail_recent(store, args):
    limit = max(1, min(int(args.get("max_results", 10) or 10), 50))
    rows = []
    for a in store.pick_read(str_arg(args, "account")):
        for th in store.mailbox["threads"]:
            if th["account"] != a["name"] or not visible(th, inbox_only=True):
                continue
            for m in th["messages"]:
                rows.append(row(a["name"], th, m))
    rows.sort(key=lambda r: r["date"], reverse=True)
    return render_rows(rows[:limit])


def mail_get_thread(store, args):
    thread_id = str_arg(args, "thread_id")
    if thread_id is None:
        raise ToolError("mail_get_thread needs thread_id")
    a = store.pick_item(str_arg(args, "account"))
    th = store.thread(a, thread_id)
    for m in th["messages"]:
        m["unread"] = False
    store.save()
    return render_thread(a["name"], th)


def mail_send(store, args):
    to, subject, body = str_arg(args, "to"), str_arg(args, "subject"), str_arg(args, "body_markdown")
    if not (to and subject and body):
        raise ToolError("mail_send needs to, subject, and body_markdown")
    a = store.pick_send(str_arg(args, "account"))
    mid = store.next_message_id()
    recipients = [x.strip() for x in to.split(",") if x.strip()]
    thread = {
        "id": f"t-{mid}",
        "account": a["name"],
        "subject": subject,
        "archived": False,
        "trashed": False,
        "spam": False,
        "messages": [
            {
                "id": mid,
                "from_name": a.get("display_name", a["name"]),
                "from_address": a["address"],
                "to": recipients,
                "cc": [x.strip() for x in (str_arg(args, "cc") or "").split(",") if x.strip()],
                "date": iso(now_dt()),
                "body": body,
                "unread": False,
                "bulk": False,
            }
        ],
    }
    store.mailbox["threads"].append(thread)
    store.save()
    store.record_send("mail_send", a["name"], args, {"message_id": mid, "thread_id": thread["id"]})
    return f"sent (message id {mid}) from `{a['name']}` to {to}"


def mail_reply(store, args):
    thread_id, body = str_arg(args, "thread_id"), str_arg(args, "body_markdown")
    if not (thread_id and body):
        raise ToolError("mail_reply needs thread_id and body_markdown")
    a = store.pick_item(str_arg(args, "account"))
    th = store.thread(a, thread_id)
    if not th["messages"]:
        raise ToolError(f"thread {thread_id} has no messages")
    wanted = str_arg(args, "message_id")
    target = None
    if wanted:
        target = next((m for m in th["messages"] if m["id"] == wanted), None)
        if target is None:
            raise ToolError(f"no message `{wanted}` in thread {thread_id}")
    else:
        target = th["messages"][-1]
    reply_all = bool(args.get("reply_all", False))
    to = [target["from_address"]] if target["from_address"] != a["address"] else list(target.get("to", []))
    cc = []
    if reply_all:
        cc = [x for x in target.get("to", []) + target.get("cc", []) if x not in to and x != a["address"]]
    mid = store.next_message_id()
    th["messages"].append(
        {
            "id": mid,
            "from_name": a.get("display_name", a["name"]),
            "from_address": a["address"],
            "to": to,
            "cc": cc,
            "date": iso(now_dt()),
            "body": body,
            "unread": False,
            "bulk": False,
        }
    )
    store.save()
    store.record_send("mail_reply", a["name"], args, {"message_id": mid, "thread_id": thread_id, "to": to, "cc": cc})
    return f"replied (message id {mid}) in thread {thread_id} from `{a['name']}` to {', '.join(to)}"


def mail_triage(store, args):
    thread_id, raw = str_arg(args, "thread_id"), str_arg(args, "action")
    if thread_id is None:
        raise ToolError("mail_triage needs thread_id")
    if raw is None:
        raise ToolError("mail_triage needs action")
    if raw not in TRIAGE_ACTIONS:
        raise ToolError(f"unknown action `{raw}`; expected one of: {', '.join(TRIAGE_ACTIONS)}")
    a = store.pick_item(str_arg(args, "account"))
    th = store.thread(a, thread_id)
    if raw == "archive":
        th["archived"] = True
    elif raw == "read":
        for m in th["messages"]:
            m["unread"] = False
    elif raw == "unread":
        for m in th["messages"]:
            m["unread"] = True
    elif raw == "spam":
        th["spam"] = True
    elif raw == "trash":
        th["trashed"] = True
    store.save()
    store.append(store.triage_path, {"at": iso(now_dt()), "account": a["name"], "thread_id": thread_id, "action": raw})
    return f"{raw}: thread {thread_id} in `{a['name']}`"


# --- calendar ------------------------------------------------------------------------


def parse_time(raw, what):
    if not isinstance(raw, str) or not raw:
        raise ToolError(f"{what} must be RFC 3339 (or YYYY-MM-DD with all_day)")
    try:
        if len(raw) == 10:
            return dt.datetime.fromisoformat(raw).replace(tzinfo=dt.timezone.utc)
        d = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
        if d.tzinfo is None:
            d = d.replace(tzinfo=dt.timezone.utc)
        return d.astimezone(dt.timezone.utc)
    except ValueError:
        raise ToolError(f"{what} `{raw}` is not RFC 3339")


def event_json(e):
    return {
        "account": e["account"],
        "id": e["id"],
        "calendar_id": e.get("calendar_id", "primary"),
        "title": e["title"],
        "start": e["start"],
        "end": e["end"],
        "all_day": bool(e.get("all_day", False)),
        "location": e.get("location"),
        "description": e.get("description"),
        "attendees": e.get("attendees", []),
    }


def calendar_list(store, args):
    return json.dumps(
        [{"account": a["name"], "id": "primary", "name": "Calendar", "writable": True} for a in store.pick_read(str_arg(args, "account"))],
        indent=2,
    )


def window(args):
    base = now_dt()
    tmin = parse_time(str_arg(args, "time_min") or iso(base), "time_min")
    tmax = parse_time(str_arg(args, "time_max") or iso(base + dt.timedelta(days=7)), "time_max")
    return tmin, tmax


def calendar_list_events(store, args):
    tmin, tmax = window(args)
    accounts = {a["name"] for a in store.pick_read(str_arg(args, "account"))}
    events = [
        event_json(e)
        for e in store.calendar["events"]
        if e["account"] in accounts and parse_time(e["start"], "start") < tmax and parse_time(e["end"], "end") > tmin
    ]
    if not events:
        return f"no events between {iso(tmin)} and {iso(tmax)}"
    events.sort(key=lambda e: e["start"])
    return json.dumps(events, indent=2)


def calendar_freebusy(store, args):
    tmin, tmax = window(args)
    accounts = {a["name"] for a in store.pick_read(str_arg(args, "account"))}
    busy = sorted(
        (
            {"start": e["start"], "end": e["end"]}
            for e in store.calendar["events"]
            if e["account"] in accounts and parse_time(e["start"], "start") < tmax and parse_time(e["end"], "end") > tmin
        ),
        key=lambda b: b["start"],
    )
    return json.dumps({"time_min": iso(tmin), "time_max": iso(tmax), "busy": busy}, indent=2)


def calendar_create_event(store, args):
    title = str_arg(args, "title")
    if title is None:
        raise ToolError("calendar_create_event needs title, start_time and end_time")
    start = parse_time(args.get("start_time"), "start_time")
    end = parse_time(args.get("end_time"), "end_time")
    if end <= start:
        raise ToolError("end_time must be after start_time")
    a = store.pick_send(str_arg(args, "account"))
    attendees = args.get("attendees") or []
    if not isinstance(attendees, list) or not all(isinstance(x, str) for x in attendees):
        raise ToolError("attendees must be a list of addresses")
    e = {
        "id": store.next_event_id(),
        "account": a["name"],
        "calendar_id": str_arg(args, "calendar_id") or "primary",
        "title": title,
        "start": iso(start),
        "end": iso(end),
        "all_day": bool(args.get("all_day", False)),
        "location": str_arg(args, "location"),
        "description": str_arg(args, "description"),
        "attendees": attendees,
    }
    store.calendar["events"].append(e)
    store.save()
    store.record_send("calendar_create_event", a["name"], args, {"event_id": e["id"]})
    return f"created event {e['id']} on `{a['name']}`: {title} {e['start']} – {e['end']}" + (
        f"; invitations to {', '.join(attendees)}" if attendees else ""
    )


def find_event(store, args):
    event_id = str_arg(args, "event_id")
    if event_id is None:
        raise ToolError("event_id is required")
    a = store.pick_item(str_arg(args, "account"))
    for e in store.calendar["events"]:
        if e["id"] == event_id and e["account"] == a["name"]:
            return a, e
    raise ToolError(f"no event `{event_id}` in account `{a['name']}`")


def calendar_update_event(store, args):
    a, e = find_event(store, args)
    if str_arg(args, "title"):
        e["title"] = args["title"]
    if str_arg(args, "start_time"):
        e["start"] = iso(parse_time(args["start_time"], "start_time"))
    if str_arg(args, "end_time"):
        e["end"] = iso(parse_time(args["end_time"], "end_time"))
    for key in ("description", "location"):
        if isinstance(args.get(key), str):
            e[key] = args[key] or None
    if isinstance(args.get("attendees"), list):
        e["attendees"] = [x for x in args["attendees"] if isinstance(x, str)]
    if isinstance(args.get("all_day"), bool):
        e["all_day"] = args["all_day"]
    store.save()
    store.record_send("calendar_update_event", a["name"], args, {"event_id": e["id"]})
    return f"updated event {e['id']} on `{a['name']}`"


def calendar_delete_event(store, args):
    a, e = find_event(store, args)
    store.calendar["events"] = [x for x in store.calendar["events"] if x is not e]
    store.save()
    store.record_send("calendar_delete_event", a["name"], args, {"event_id": e["id"]})
    return f"deleted event {e['id']} on `{a['name']}`; attendees notified"


ACCOUNT = {"type": "string", "description": "The account, by name. Omit for every account (reads) or the default (sends)."}

TOOLS = [
    {
        "name": "mail_search",
        "description": "Search mail. With no `account`, every configured account is searched and each result row is tagged with the account it came from. from:/to:/subject: filters work. Returns metadata and snippets; use mail_get_thread to read full messages.",
        "inputSchema": {
            "type": "object",
            "properties": {"query": {"type": "string"}, "account": ACCOUNT, "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}},
            "required": ["query"],
        },
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "mail_recent",
        "description": "The most recent messages, newest first, across every account (or one, when `account` is given). Use when the user asks what just came in rather than for a specific search.",
        "inputSchema": {"type": "object", "properties": {"account": ACCOUNT, "max_results": {"type": "integer", "minimum": 1, "maximum": 50, "default": 10}}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "mail_get_thread",
        "description": "Read a whole conversation by thread_id, oldest first, with clean text bodies. thread_ids are account-scoped: pass the `account` from the search row the id came from.",
        "inputSchema": {"type": "object", "properties": {"thread_id": {"type": "string"}, "account": ACCOUNT}, "required": ["thread_id"]},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "mail_send",
        "description": "Send a NEW email. body_markdown is converted to HTML. To answer an existing message use mail_reply instead, so it threads. With no `account` the default account is used; if none is set, the call fails and you should ask the user which account to send from.",
        "inputSchema": {
            "type": "object",
            "properties": {"to": {"type": "string"}, "subject": {"type": "string"}, "body_markdown": {"type": "string"}, "account": ACCOUNT, "cc": {"type": "string"}, "bcc": {"type": "string"}},
            "required": ["to", "subject", "body_markdown"],
        },
        "annotations": {"openWorldHint": True},
    },
    {
        "name": "mail_reply",
        "description": "Reply within an existing conversation so it threads. Pass the thread_id and its `account` (both are in every search row). Replies to the newest message in the thread unless message_id names one. Set reply_all to include everyone on the original.",
        "inputSchema": {
            "type": "object",
            "properties": {"thread_id": {"type": "string"}, "body_markdown": {"type": "string"}, "account": ACCOUNT, "message_id": {"type": "string"}, "reply_all": {"type": "boolean", "default": False}},
            "required": ["thread_id", "body_markdown"],
        },
        "annotations": {"openWorldHint": True},
    },
    {
        "name": "mail_triage",
        "description": "Clear a conversation out of the inbox: archive it, mark it read or unread, report it as spam, or move it to the trash. Acts on the WHOLE thread. Nothing here leaves the mailbox or reaches anyone else. Use archive for anything dealt with; use spam only for genuine junk.",
        "inputSchema": {
            "type": "object",
            "properties": {"thread_id": {"type": "string"}, "action": {"type": "string", "enum": TRIAGE_ACTIONS}, "account": ACCOUNT},
            "required": ["thread_id", "action"],
        },
        "annotations": {"destructiveHint": True},
    },
    {
        "name": "calendar_list",
        "description": "List the calendars in every configured account (or one, when `account` is given), with write access noted.",
        "inputSchema": {"type": "object", "properties": {"account": ACCOUNT}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "calendar_list_events",
        "description": "List events in a time window across every account, merged in time order and tagged by account. Times are RFC 3339; omit both to get the next 7 days.",
        "inputSchema": {"type": "object", "properties": {"time_min": {"type": "string"}, "time_max": {"type": "string"}, "account": ACCOUNT, "calendar_id": {"type": "string", "default": "primary"}}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "calendar_freebusy",
        "description": "Busy intervals merged across every account (or one, when `account` is given) — when the user is busy, with no event details. Times are RFC 3339; omit both bounds for the next 7 days.",
        "inputSchema": {"type": "object", "properties": {"time_min": {"type": "string"}, "time_max": {"type": "string"}, "account": ACCOUNT}},
        "annotations": {"readOnlyHint": True},
    },
    {
        "name": "calendar_create_event",
        "description": "Create a calendar event; attendees receive invitations. Times are RFC 3339 (or YYYY-MM-DD with all_day). With no `account` the default account's calendar is used; if none is set, the call fails and you should ask the user which calendar.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string"}, "start_time": {"type": "string"}, "end_time": {"type": "string"}, "account": ACCOUNT,
                "description": {"type": "string"}, "location": {"type": "string"}, "attendees": {"type": "array", "items": {"type": "string"}},
                "all_day": {"type": "boolean", "default": False}, "timezone": {"type": "string"}, "calendar_id": {"type": "string", "default": "primary"},
            },
            "required": ["title", "start_time", "end_time"],
        },
        "annotations": {"openWorldHint": True},
    },
    {
        "name": "calendar_update_event",
        "description": "Update fields of an existing event by event_id in the `account` it lives in. Only the fields provided change; attendees are notified.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "event_id": {"type": "string"}, "account": ACCOUNT, "calendar_id": {"type": "string", "default": "primary"}, "title": {"type": "string"},
                "start_time": {"type": "string"}, "end_time": {"type": "string"}, "description": {"type": "string"}, "location": {"type": "string"},
                "attendees": {"type": "array", "items": {"type": "string"}}, "all_day": {"type": "boolean"}, "timezone": {"type": "string"},
            },
            "required": ["event_id"],
        },
        "annotations": {"openWorldHint": True, "destructiveHint": True},
    },
    {
        "name": "calendar_delete_event",
        "description": "Delete an event by event_id in the `account` it lives in. Attendees are notified of the cancellation.",
        "inputSchema": {"type": "object", "properties": {"event_id": {"type": "string"}, "account": ACCOUNT, "calendar_id": {"type": "string", "default": "primary"}}, "required": ["event_id"]},
        "annotations": {"openWorldHint": True, "destructiveHint": True},
    },
]

HANDLERS = {
    "mail_search": mail_search,
    "mail_recent": mail_recent,
    "mail_get_thread": mail_get_thread,
    "mail_send": mail_send,
    "mail_reply": mail_reply,
    "mail_triage": mail_triage,
    "calendar_list": calendar_list,
    "calendar_list_events": calendar_list_events,
    "calendar_freebusy": calendar_freebusy,
    "calendar_create_event": calendar_create_event,
    "calendar_update_event": calendar_update_event,
    "calendar_delete_event": calendar_delete_event,
}


# --- the wire ---------------------------------------------------------------------------


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
                    "serverInfo": {"name": "mail-fixture", "version": "0"},
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
                reply(request_id, text_result(handler(store, params.get("arguments") or {})))
            except ToolError as e:
                reply(request_id, text_result(str(e), is_error=True))
            except Exception as e:  # noqa: BLE001 — a fixture absorbs whatever a model sends
                # The repo's `Ok(ToolOutput { is_error: true })` convention: a
                # malformed argument is the model's error to recover from,
                # never a dead server for the rest of the run.
                reply(request_id, text_result(f"{type(e).__name__}: {e}", is_error=True))
        else:
            fail(request_id, f"unsupported method: {method}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", help="the state directory (default: $MECHA_FIXTURE_DIR)")
    parser.add_argument("--ephemeral", action="store_true", help="read the store, never write it")
    opts = parser.parse_args()
    root = opts.store or os.environ.get("MECHA_FIXTURE_DIR")
    if not root:
        print(
            "mail_server.py: no store — set MECHA_FIXTURE_DIR (mecha exp does) or pass --store; "
            "a mailbox that forgets what was sent is not a fixture",
            file=sys.stderr,
        )
        return 2
    if not os.path.isdir(root):
        if opts.ephemeral:
            print(f"mail_server.py: {root} is not a directory", file=sys.stderr)
            return 2
        os.makedirs(root, exist_ok=True)
    serve(Store(root, opts.ephemeral))
    return 0


if __name__ == "__main__":
    sys.exit(main())
