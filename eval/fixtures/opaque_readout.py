#!/usr/bin/env python3
"""Read an opaque-service experiment back: per arm, what passed, what it cost,
and whether the rule a probe needed was in what the harness handed over.

    opaque_readout.py <experiment directory> [<experiment directory> ...]

A pass rate alone cannot tell recall from luck or from turns saved elsewhere
(`docs/MEMORY-BENCHMARK-DESIGN.md` §5: a probe counts as using memory only if
recall was delivered to it). So each trial is read three ways: the grader's
verdict from the trial row, the tokens and turns from the same row, and the
trial's own transcript for (a) a delivery in the owner's message, and whether
the token rule and the unit rule are in it, and (b) calls the model made to a
memory tool itself.

Reads only. The answer key is not opened: the rule sentences come from
`opaque_world`'s documentation strings, which are what the service says.
"""

import glob
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import opaque_world  # noqa: E402

STEMS = (
    "The harness searched your memory",
    "The harness searched the transcripts",
    "A call just failed. The harness searched your memory",
    "The harness looked up what your earlier runs found out",
)
MEMORY_TOOLS = ("goal_context", "transcript_search", "memory_search")


def strings(node):
    if isinstance(node, str):
        yield node
    elif isinstance(node, dict):
        for value in node.values():
            yield from strings(value)
    elif isinstance(node, list):
        for value in node:
            yield from strings(value)


def tool_uses(node):
    if isinstance(node, dict):
        if node.get("type") == "tool_use" and "name" in node:
            yield node["name"]
        for value in node.values():
            yield from tool_uses(value)
    elif isinstance(node, list):
        for value in node:
            yield from tool_uses(value)


def blocks(node, kind):
    if isinstance(node, dict):
        if node.get("type") == kind:
            yield node
        for value in node.values():
            yield from blocks(value, kind)
    elif isinstance(node, list):
        for value in node:
            yield from blocks(value, kind)


def transcript(home, session_id):
    path = os.path.join(home, "sessions", session_id + ".jsonl")
    try:
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]
    except OSError:
        return None


def read_trial(root, row):
    world_name = row["task"].split("-", 1)[0]
    world = opaque_world.WORLDS[world_name]
    token_rule = [t for t in world["topics"].values() if world["token_example"] in t][0]
    unit_rule = f"Exported amounts are in {world['minor_words']}"
    home = os.path.join(root, "homes", f"{row['arm']}__s{row['seed']}__r{row['repetition']}")
    stats = row.get("stats") or {}
    usage = stats.get("usage") or {}
    out = {
        "arm": row["arm"],
        "task": row["task"].split("-", 1)[1],
        "seed": row["seed"],
        "passed": row.get("passed"),
        "turns": stats.get("turns"),
        "tokens": sum(usage.get(k, 0) for k in ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens")),
        "delivered_bytes": None,
        "token_rule": None,
        "unit_rule": None,
        "own_recalls": None,
        "after_error": None,
    }
    records = transcript(home, row.get("session_id") or "")
    if records is None:
        return out
    # Everything memory put in front of the model: what the harness handed
    # over, at the start or after a failed call, and what the model's own
    # searches returned. Each distinct text once (a rewrite lists it again).
    handed = {s for r in records for s in strings(r) if s.startswith(STEMS)}
    own = {b["id"] for r in records for b in blocks(r, "tool_use") if b.get("name") in MEMORY_TOOLS}
    answers = {
        b.get("content") if isinstance(b.get("content"), str) else json.dumps(b.get("content"))
        for r in records
        for b in blocks(r, "tool_result")
        if b.get("tool_use_id") in own
    }
    out["own_recalls"] = len(own)
    out["after_error"] = sum(1 for s in handed if s.startswith(STEMS[2]))
    seen = handed | answers
    if seen:
        # The rule may be JSON-escaped inside a packet; compare unescaped.
        whole = "\n".join(sorted(seen))
        plain = whole.replace('\\"', '"').replace("\\'", "'")
        out["delivered_bytes"] = len(whole.encode())
        out["token_rule"] = token_rule in plain or token_rule in whole
        out["unit_rule"] = unit_rule in plain
    return out


def rate(rows):
    done = [r for r in rows if r["passed"] is not None]
    return f"{sum(1 for r in done if r['passed'])}/{len(done)}" if done else "—"


def mean(values):
    values = [v for v in values if v is not None]
    return f"{sum(values) / len(values):.0f}" if values else "—"


def main(roots):
    for root in roots:
        root = root.rstrip("/")
        rows = []
        for path in sorted(glob.glob(os.path.join(root, "trials", "*.json"))):
            with open(path) as f:
                row = json.load(f)
            if row.get("status") == "done":
                rows.append(read_trial(root, row))
        print(f"\n{os.path.basename(root)}: {len(rows)} finished trial(s)")
        tasks = ["learn-rule", "learn-units", "probe-rule", "probe-both"]
        print(f"{'arm':<18} " + " ".join(f"{t:>11}" for t in tasks) + f" {'probes':>7} {'tokens/task':>11} {'turns':>5}")
        for arm in sorted({r["arm"] for r in rows}):
            mine = [r for r in rows if r["arm"] == arm]
            cells = " ".join(f"{rate([r for r in mine if r['task'] == t]):>11}" for t in tasks)
            probes = rate([r for r in mine if r["task"].startswith("probe")])
            print(f"{arm:<18} {cells} {probes:>7} {mean([r['tokens'] for r in mine]):>11} {mean([r['turns'] for r in mine]):>5}")
        print("probes, by what was handed over (pass/of):")
        for arm in sorted({r["arm"] for r in rows}):
            probes = [r for r in rows if r["arm"] == arm and r["task"].startswith("probe")]
            handed = [r for r in probes if r["delivered_bytes"] is not None]
            line = f"  {arm:<18} memory reached {len(handed)}/{len(probes)}"
            if handed:
                line += f" · mean {mean([r['delivered_bytes'] for r in handed])} bytes"
                with_token = [r for r in handed if r["token_rule"]]
                line += f" · token rule in packet {len(with_token)}/{len(handed)} (passed {rate(with_token)})"
                both = [r for r in handed if r["task"] == "probe-both"]
                with_unit = [r for r in both if r["unit_rule"]]
                line += f" · unit rule in packet {len(with_unit)}/{len(both)} of probe-both (passed {rate(with_unit)})"
            line += f" · model's own memory calls {sum(r['own_recalls'] or 0 for r in probes)}"
            line += f" · harness recalls after a failure {sum(r['after_error'] or 0 for r in probes)}"
            print(line)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
