#!/usr/bin/env python3
"""Register/run a native gossip comparison; prepare an arm-blind citation audit."""
import argparse
import collections
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def write(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def props():
    with urllib.request.urlopen("http://127.0.0.1:8080/props", timeout=10) as response:
        p = json.load(response)
    result = dict(model=p.get("model_alias"), slots=p.get("total_slots"),
                  context=p.get("default_generation_settings", {}).get("n_ctx"),
                  generation=p.get("default_generation_settings", {}).get("params"))
    if result["model"] != "qwen3.6-35b-a3b" or result["context"] != 262144:
        raise ValueError(f"Unexpected served condition: {result}")
    return result


def key(case, claim):
    fields = [case, claim["statement"], claim["episode_id"], claim["quote"]]
    return hashlib.sha256(json.dumps(fields, ensure_ascii=True).encode()).hexdigest()[:20]


def audit(out):
    design = json.loads((out / "design.json").read_text())
    cases = {c["id"]: c for c in json.loads((out / "cases.json").read_text())}
    claims = {}
    outcomes = []
    for trial in design["trials"]:
        path = out / "trials" / trial["id"]
        result_file = path / "result.json"
        result = json.loads(result_file.read_text()) if result_file.exists() else {"status": "missing"}
        receipts = [json.loads(s) for s in (path / "provider.jsonl").read_text().splitlines()] if (path / "provider.jsonl").exists() else []
        requests = [r for r in receipts if r["event"] == "request"]
        responses = [r for r in receipts if r["event"] == "response"]
        searches = [json.loads(s) for s in (path / "search.jsonl").read_text().splitlines()] if (path / "search.jsonl").exists() else []
        problems = []
        if result["status"] != "complete": problems.append("incomplete exchange")
        if len(requests) != len(responses): problems.append("unanswered model request")
        if len(requests) > 32: problems.append("provider request ceiling exceeded")
        if any(s["error"] for s in searches): problems.append("search boundary/error")
        if any(r["model"] != "qwen3.6-35b-a3b" for r in responses): problems.append("wrong response model")
        if any(r["malformed_tool_args"] for r in responses): problems.append("malformed tool arguments")
        expected_tools = lambda r: ["kg_search"] if r["role"].endswith("_answer") else []
        if any([t["name"] for t in r["tools"]] != expected_tools(r) for r in requests): problems.append("unexpected tool surface")
        rows = []
        for round in result.get("exchange", {}).get("rounds", []):
            ids = []
            for claim in round["grounded"]:
                cid = key(trial["case"], claim)
                ids.append(cid)
                case = cases[trial["case"]]
                episode = next(e for e in case["episodes"] if e["id"] == claim["episode_id"])
                claims[cid] = dict(id=cid, case=trial["case"], entity=case["entity"],
                                   statement=claim["statement"], quote=claim["quote"],
                                   episode=episode, reference_facts=case["gold"])
            rows.append(dict(round=round["n"], claims=ids, abstentions=len(round["abstained"]),
                             ungrounded_answers=len(round["ungrounded"]), stalls=len(round.get("stalled", []))))
        outcomes.append(dict(**trial, problems=problems, requests=len(requests), searches=len(searches),
                             input_tokens=sum(r["usage"]["input_tokens"] + r["usage"]["cache_read_input_tokens"] + r["usage"]["cache_creation_input_tokens"] for r in responses),
                             output_tokens=sum(r["usage"]["output_tokens"] for r in responses), rounds=rows))
    # Stable hash order, with no mode/seed/round in the evaluator's packet.
    write(out / "audit-packet.json", [claims[k] for k in sorted(claims)])
    write(out / "outcomes.json", outcomes)
    return outcomes, claims


def report(out):
    outcomes, claims = audit(out)
    verdicts = json.loads((out / "audit-verdicts.json").read_text())
    verdicts = {v["id"]: v for v in verdicts}
    if set(verdicts) != set(claims):
        raise ValueError("Every unique carried claim must have exactly one reviewed verdict")
    for cid, v in verdicts.items():
        valid = {f["id"] for f in claims[cid]["reference_facts"]}
        if v["label"] not in ("supported", "contradicted", "unresolved") or not v["reason"]:
            raise ValueError("Invalid audit verdict")
        if not set(v["facts"]) <= valid or (v["label"] != "supported" and v["facts"]):
            raise ValueError("Invalid supported fact attribution")
    summary = {}
    for mode in ["own_evidence", "peer"]:
        trials = [r for r in outcomes if r["mode"] == mode]
        rounds = []
        for n in [1, 2, 3]:
            counts = collections.Counter()
            cumulative_facts = new_facts = 0
            for trial in trials:
                previous = set()
                for row in trial["rounds"][:n]:
                    now = set()
                    for cid in row["claims"]:
                        v = verdicts[cid]
                        now.update(v["facts"])
                        if row["round"] == n: counts[v["label"]] += 1
                    if row["round"] == n:
                        new_facts += len(now - previous)
                        for name in ["abstentions", "ungrounded_answers", "stalls"]: counts[name] += row[name]
                    previous.update(now)
                cumulative_facts += len(previous)
            rounds.append(dict(round=n, **counts, new_supported_facts=new_facts, cumulative_supported_facts=cumulative_facts))
        summary[mode] = dict(trials=len(trials), invalid_trials=sum(bool(t["problems"]) for t in trials),
                             requests=sum(t["requests"] for t in trials), searches=sum(t["searches"] for t in trials),
                             input_tokens=sum(t["input_tokens"] for t in trials), output_tokens=sum(t["output_tokens"] for t in trials), rounds=rounds)
    paired = []
    for control in [t for t in outcomes if t["mode"] == "own_evidence"]:
        treatment = next(t for t in outcomes if t["mode"] == "peer" and t["case"] == control["case"] and t["seed"] == control["seed"])
        def facts(t): return set(f for r in t["rounds"] for cid in r["claims"] for f in verdicts[cid]["facts"])
        paired.append(dict(case=control["case"], seed=control["seed"], control_facts=len(facts(control)), peer_facts=len(facts(treatment)),
                           peer_only=sorted(facts(treatment)-facts(control)), control_only=sorted(facts(control)-facts(treatment))))
    write(out / "scorecard.json", dict(summary=summary, paired=paired, audit_unique_claims=len(claims),
        complete=len(outcomes)==16 and all(not t["problems"] and len(t["rounds"])==3 for t in outcomes)
        and all(json.loads((out / "finish.json").read_text()).get(k) is True for k in ["server_matches", "input_hashes_match"])))


def run(out):
    out.mkdir(parents=True, exist_ok=False)
    (out / "trials").mkdir()
    sources = [ROOT / "eval/fixtures/gossip/cases.json", ROOT / "eval/fixtures/gossip/server.py",
               ROOT / "mecha-core/src/gossip.rs", ROOT / "mecha-core/examples/gossip_compare.rs", Path(__file__),
               ROOT / "target/debug/examples/gossip_compare"]
    for path in sources:
        if not path.is_file(): raise ValueError(f"Missing input {path}")
    (out / "cases.json").write_bytes(sources[0].read_bytes())
    (out / "server.py").write_bytes(sources[1].read_bytes())
    (out / "runner.py").write_bytes(Path(__file__).read_bytes())
    cases = json.loads(sources[0].read_text())
    pairs = [(c["id"], seed) for c in cases for seed in [1, 2]]
    random.Random(20260910).shuffle(pairs)
    trials = []
    for i, (case, seed) in enumerate(pairs):
        modes = ["own_evidence", "peer"] if i % 2 == 0 else ["peer", "own_evidence"]
        for mode in modes:
            trials.append(dict(id=f"{case}-{seed}-{mode}", case=case, seed=seed, mode=mode))
    hashes = {str(p.relative_to(ROOT)): digest(p) for p in sources}
    start = props()
    write(out / "design.json", dict(version=3, trials=trials, input_hashes=hashes, server_start=start,
        runtime_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        prediction="Peer questions recover more distinct supported source facts by round 3 than own-evidence followups without increasing contradicted claims.",
        allocation="Both arms: 3 rounds, 2 readers, max 4 turns per answer, 1 per asker plus one retry; 32 provider requests maximum. Actual tokens/calls are outcomes, not matched consumption.",
        grading="Arm-blind assistant review of every unique structurally admitted statement against its quoted episode and registered target/source/time facts. Citation presence alone does not pass. No model self-report, no acting-model judge. Identity contrasts are supported only when explicitly distinguished. Current-world assertions contradicted by later records fail unless source/time qualified. Count unique reference episode facts within each trial; partial coverage is not full proposition recall.",
        decision="A pilot only: retain default on mixed results, invalid pairs or increased contradictions. No significance/transfer claim from four synthetic cases and two seeds. No graph writes or config promotion."))
    for i, trial in enumerate(trials):
        print(f"{i+1}/16 {trial['id']}", flush=True)
        env = {k: os.environ[k] for k in ["PATH", "HOME", "LANG", "LC_ALL", "TZ"] if k in os.environ}
        env["MECHA_SESSION_KIND"] = "test"
        with (out / (trial["id"] + ".log")).open("w") as log:
            result = subprocess.run([str(sources[-1]), str(out / "cases.json"), str(out / "server.py"),
                                     trial["case"], trial["mode"], str(trial["seed"]), str(out / "trials" / trial["id"])],
                                    cwd=out, env=env, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print(f"Trial failed ({result.returncode}); preserving failure and continuing registered order.", flush=True)
    finish = props()
    write(out / "finish.json", dict(server_finish=finish, server_matches=start==finish,
        input_hashes_match=all(digest(ROOT / p)==h for p,h in hashes.items())))
    outcomes, _ = audit(out)
    if any(t["problems"] for t in outcomes) or start != finish or not all(digest(ROOT / p)==h for p,h in hashes.items()):
        raise SystemExit("Run preserved but incomplete or condition audit failed; do not treat it as a valid comparison.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--audit-only", action="store_true")
    parser.add_argument("--report-only", action="store_true")
    args = parser.parse_args()
    out = args.out.resolve()
    if args.report_only: report(out)
    elif args.audit_only: audit(out)
    else: run(out)
