#!/usr/bin/env python3
"""Descriptive paired report from `mecha exp export appraisal-guidance` JSON.

Missing observations stay missing. This is not a promotion gate: exp judge's
held-out decision remains separate, and seeds of a task are not distinct tasks.
"""
import collections
import json
import math
from pathlib import Path
import sys

TASKS = {c["id"] for c in json.loads((Path(__file__).resolve().parents[1] /
                                   "eval/fixtures/appraisal/cases.json").read_text())}
METRICS = ("turns", "tool_calls", "duration_secs", "cost_usd", "step_reopens",
           "step_nulls", "step_completions", "step_measured", "goal_plan_writes",
           "goal_drift_writes", "goal_unnamed_writes", "checks_declared", "checks_passed")


def measured(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def report(data, task_ids=None):
    manifest = data["manifest"]
    if manifest.get("kind") != "single" or manifest.get("control") != "control" or set(manifest["arms"]) != {"control", "guided"}:
        raise ValueError("expected the control/guided single-run appraisal design")
    known_tasks = TASKS if task_ids is None else set(task_ids)
    selected = manifest["tasks"].get("ids") or sorted(known_tasks)
    if not set(selected) <= known_tasks or len(set(selected)) != len(selected):
        raise ValueError("unknown or duplicate task IDs")
    seeds = manifest.get("seeds") or [None]
    expected = {(task, seed, rep) for task in selected for seed in seeds
                for rep in range(1, manifest.get("repetitions", 1) + 1)}
    rows = {}
    for row in data["trials"]:
        key = (row["task"], row.get("seed"), row["repetition"])
        full = (row["arm"], key)
        if full in rows:
            raise ValueError("duplicate arm/task/seed/repetition")
        if key not in expected or row["arm"] not in {"control", "guided"}:
            raise ValueError("trial is outside the registered design")
        rows[full] = row
    def graded(row):
        return row is not None and row.get("status") == "done" and type(row.get("passed")) is bool
    paired = []
    for key in sorted(expected, key=str):
        left, right = rows.get(("control", key)), rows.get(("guided", key))
        if left and right and left.get("condition_hash") == right.get("condition_hash"):
            raise ValueError("paired arms have identical or absent condition hashes")
        if graded(left) and graded(right):
            paired.append((key, left, right))
    arms = {}
    for arm in ("control", "guided"):
        values = [row for (name, _), row in rows.items() if name == arm]
        eligible = [row for row in values if graded(row)]
        failures = sum(not row["passed"] for row in eligible)
        arms[arm] = dict(expected=len(expected), present=len(values), graded=len(eligible),
                         failures=failures, failure_rate=failures / len(eligible) if eligible else None,
                         status_counts=dict(collections.Counter(row.get("status", "unknown") for row in values)))
    improved = sum(not left["passed"] and right["passed"] for _, left, right in paired)
    regressed = sum(left["passed"] and not right["passed"] for _, left, right in paired)
    metrics = {}
    for metric in (*METRICS, "owner_actions"):
        observations = []
        for _, left, right in paired:
            a = left.get(metric) if metric == "owner_actions" else (left.get("stats") or {}).get(metric)
            b = right.get(metric) if metric == "owner_actions" else (right.get("stats") or {}).get(metric)
            complete_usage = metric != "cost_usd" or all(
                (row.get("stats") or {}).get("usage_complete") is True for row in (left, right))
            if measured(a) and measured(b) and complete_usage:
                observations.append((a, b))
        n = len(observations)
        metrics[metric] = dict(paired_n=n, missing_pairs=len(paired)-n,
                               control_mean=sum(a for a, _ in observations)/n if n else None,
                               guided_mean=sum(b for _, b in observations)/n if n else None,
                               guided_minus_control=sum(b-a for a, b in observations)/n if n else None)
    per_task = {}
    for task in sorted(selected):
        pairs = [(a,b) for (name,_,_),a,b in paired if name == task]
        per_task[task] = dict(paired_n=len(pairs), control_failures=sum(not a["passed"] for a,b in pairs),
                              guided_failures=sum(not b["passed"] for a,b in pairs))
    unreadable = data.get("unreadable_trials", 0)
    return dict(expected_pairs=len(expected), graded_pairs=len(paired), ungraded_or_missing_pairs=len(expected)-len(paired),
                unreadable_trials=unreadable, complete=len(paired)==len(expected) and unreadable==0,
                arms=arms, improved_pairs=improved, regressed_pairs=regressed,
                paired_failure_delta=(regressed-improved)/len(paired) if paired else None,
                metrics=metrics, tasks=per_task,
                note="Descriptive only. Negative failure delta favors guidance. Repeated seeds are not independent tasks; no learning or owner-policy efficacy is established.")


def main():
    if len(sys.argv) not in (2, 3):
        raise ValueError("usage: appraisal-report.py EXPORT.json [CASES.json] (or - for stdin)")
    with (sys.stdin if sys.argv[1] == "-" else open(sys.argv[1])) as stream:
        data = json.load(stream)
    task_ids = None if len(sys.argv) == 2 else [c["id"] for c in json.loads(Path(sys.argv[2]).read_text())]
    json.dump(report(data, task_ids), sys.stdout, indent=2, allow_nan=False)
    print()


if __name__ == "__main__":
    main()
