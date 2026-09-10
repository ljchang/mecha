"""Derive stopping-prefix costs from native request receipts; no model calls.

Question-generation overhead belongs to the next answer round: stopping after
round 1 would skip the question generators that prepare round 2. Roles and fresh
conversation boundaries come from the recorded native actor, not generated prose.
"""
import argparse
import collections
import json
from pathlib import Path


def costs(receipts):
    round_no = 1
    previous_role = None
    pending = None
    totals = {n: collections.Counter() for n in (1, 2, 3)}
    started_answers = collections.Counter()
    for event in receipts:
        role = event["role"]
        if event["event"] == "request":
            if pending is not None:
                raise ValueError("overlapping or unanswered requests")
            fresh = len(event["messages"]) == 1
            if role == "slack_answer" and fresh and previous_role and previous_role.endswith("_ask"):
                round_no += 1
            assigned = round_no + 1 if role.endswith("_ask") else round_no
            if assigned not in totals:
                raise ValueError("unexpected round count")
            if role.endswith("_answer") and fresh:
                started_answers[(round_no, role)] += 1
            totals[assigned]["model_requests"] += 1
            pending = (assigned, role)
            previous_role = role
        elif event["event"] == "response":
            if pending is None or pending[1] != role:
                raise ValueError("response without matching request")
            assigned = pending[0]
            pending = None
            usage = event["usage"]
            totals[assigned]["input_tokens"] += sum(usage.get(k, 0) for k in
                ("input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"))
            totals[assigned]["output_tokens"] += usage["output_tokens"]
            totals[assigned]["requested_searches"] += sum(b["type"] == "tool_use" and b["name"] == "kg_search"
                for b in event["message"]["content"])
        else:
            raise ValueError("failed/unknown provider event")
    expected = {(n, role): 1 for n in (1, 2, 3) for role in ("slack_answer", "bee_answer")}
    if pending is not None or started_answers != expected:
        raise ValueError("missing or extra answer run")
    return [dict(round=n, **totals[n]) for n in (1, 2, 3)]


def main(run):
    design = json.loads((run / "design.json").read_text())
    trials = []
    aggregate = {mode: {n: collections.Counter() for n in (1, 2, 3)} for mode in ("peer", "own_evidence")}
    for trial in design["trials"]:
        folder = run / "trials" / trial["id"]
        receipts = [json.loads(line) for line in (folder / "provider.jsonl").read_text().splitlines()]
        rounds = costs(receipts)
        executed = [json.loads(line) for line in (folder / "search.jsonl").read_text().splitlines()]
        if any(e["error"] for e in executed) or sum(r["requested_searches"] for r in rounds) != len(executed):
            raise ValueError("requested searches do not match successful actual executions")
        for row in rounds:
            aggregate[trial["mode"]][row["round"]].update({k: v for k, v in row.items() if k != "round"})
        trials.append(dict(**trial, rounds=rounds))
    result = dict(method=__doc__, trials=trials,
                  aggregate={m: [dict(round=n, **rows[n]) for n in (1, 2, 3)] for m, rows in aggregate.items()})
    (run / "round-costs.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("run", type=Path)
    main(p.parse_args().run)
