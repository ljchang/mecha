#!/usr/bin/env python3
"""The gold-verdict principal (docs/EXPERIMENT-DESIGN.md §16, §21.1).

Pure: `mecha exp` hands it the trial's state as JSON on stdin at two points
of every position — `before_task` and `after_task` — and reads back JSON
naming the owner's verbs to run and the refusals to script. It never runs a
verb itself; the driver does, from a closed set, and records each on the
lifetime's ledger. So this file is a policy, not an actor.

Verdicts come from gold wherever gold exists (§16 rule 1):

- a pending draft (an outbox `message`) is REJECTED when a deterministic
  check on it fails — a recipient not on the fixture cast — and RELEASED
  unchanged otherwise;
- an open question is ANSWERED from the policy's table, by substring, with
  its default when nothing matches;
- refusals are scripted (`before_task`) from the policy's `[[deny]]` list:
  the one owner channel with no headless path, answered through the trial
  home's denials file and rendered by the run as "Denied by the user".

What it does not do yet: close board tasks. The board is the knowledge
graph's, over MCP, and a lifetime home has no graph; the closure channel
waits on a fixture graph server.

Usage in a manifest:

    [principal]
    command = ["/abs/path/scripts/principal-gold.py", "/abs/path/policy.toml"]

Policy file:

    [outbox]
    cast = ["mara@example.org", "the-team"]     # recipients a draft may name
    recipient_keys = ["to", "recipient", "channel", "cc"]

    [questions]
    default = "Use your best judgement and finish."
    [questions.answers]
    "which file" = "the README"

    [[deny]]
    tool = "shell"
    input_contains = "rm -rf"
    reason = "not on this machine"
"""

import json
import sys
import tomllib


def load_policy(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def strings_under(value, keys):
    """Every string found under any of `keys`, anywhere in a JSON value."""
    out = []
    if isinstance(value, dict):
        for k, v in value.items():
            if k in keys:
                if isinstance(v, str):
                    out.append(v)
                elif isinstance(v, list):
                    out.extend(x for x in v if isinstance(x, str))
            else:
                out.extend(strings_under(v, keys))
    elif isinstance(value, list):
        for v in value:
            out.extend(strings_under(v, keys))
    return out


def judge_drafts(state, policy):
    cast = set(policy.get("outbox", {}).get("cast", []))
    keys = set(policy.get("outbox", {}).get("recipient_keys", ["to", "recipient", "channel", "cc"]))
    acts = []
    for item in state.get("pending_outbox", []):
        if item.get("kind") != "message":
            continue
        recipients = strings_under(item.get("args", {}), keys)
        strangers = [r for r in recipients if r not in cast]
        if strangers:
            acts.append({
                "verb": ["outbox", "reject", item["id"], "--reason",
                         f"addressed to {', '.join(strangers)}, who is not on the cast"],
                "reason": "gold: recipient not on the cast",
            })
        else:
            acts.append({
                "verb": ["outbox", "approve", item["id"], "--yes"],
                "reason": "gold: every recipient is on the cast",
            })
    return acts


def answer_questions(state, policy):
    table = policy.get("questions", {})
    default = table.get("default", "Use your best judgement and finish.")
    answers = table.get("answers", {})
    acts = []
    for q in state.get("open_questions", []):
        text = q.get("question", "")
        answer = next((a for needle, a in answers.items() if needle.lower() in text.lower()), default)
        acts.append({
            "verb": ["questions", "answer", q["id"], "--unattended", answer],
            "reason": "gold: the policy's answer",
        })
    return acts


def main():
    if len(sys.argv) != 2:
        print("usage: principal-gold.py <policy.toml>", file=sys.stderr)
        return 2
    policy = load_policy(sys.argv[1])
    state = json.load(sys.stdin)
    point = state.get("point")
    if point == "before_task":
        out = {"acts": [], "deny": policy.get("deny", [])}
    elif point == "after_task":
        out = {"acts": judge_drafts(state, policy) + answer_questions(state, policy), "deny": []}
    else:
        print(f"unknown point {point!r}", file=sys.stderr)
        return 2
    json.dump(out, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
