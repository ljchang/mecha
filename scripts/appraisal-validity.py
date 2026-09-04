#!/usr/bin/env python3
"""Step 0 of the appraisal experiments: can the readout tell a failed run from a passed one?

`docs/EXPERIMENT-DESIGN.md` Part II (PR #156 until it lands) asks it as Q1 —
is the instrument valid? — and its build order makes it the first thing to
do, before any runner: a readout that cannot separate pass from fail cannot
improve anything downstream. The dataset is the first entry of Part II's
*Datasets* section, the kept Terminal-Bench sessions under
`jobs/mecha-arm64-subset/<run>/<task>__<id>/agent/sessions/`, with Harbor's
per-trial verdict beside each in `result.json`. No model is called and no
harness code is written: this joins what `mecha sessions appraise` already
reads to what Harbor already recorded, and reports discrimination.

Run from the repo root, against the checkout that holds the (gitignored)
jobs directory:

    python3 scripts/appraisal-validity.py --jobs jobs/mecha-arm64-subset

**The finding the script had to be shaped around: none of these sessions
carries an `outcome` record.** They were written by mecha 0.1.0–0.1.3, before
`Record::Outcome` existed, so `appraisal::for_transcript` returns `None` for
every one ("read, nothing to appraise") and the readout as shipped is silent
over the whole set. The script therefore reads in three layers, and the
report keeps them apart:

1. **The readout as-is** — the sessions copied verbatim into a scratch
   `MECHA_HOME`, `mecha sessions appraise --json` run over them. This is
   what the instrument says today, and the answer is a count of zero.
2. **The counters, reconstructed from the transcript** — what `RunStats`
   would have folded, taken from the same records the loop folded it from:
   tool calls and errored results from the message blocks, compactions
   from `rewrite` records, turns and usage from the `summary` record, and
   the stop cause from Harbor's own exception record (a timeout, a cancel,
   `mecha run`'s exit code 3 for no output) joined to the transcript's
   `max_turns`. Each is reported as its own predictor. A field that cannot
   be reconstructed (`malformed_tool_args`, the homeostat, `tool_denied`)
   is left at its default and named as such — never estimated.
3. **The real `of_session` over the reconstruction** — each session copied
   again with one synthesised `outcome` record appended, and the real CLI
   run over it one session at a time (its `--json` is an aggregate, so one
   session per store *is* the per-session row). This exercises the shipped
   derivation rather than a re-typing of its rules, and the synthesised
   record is labelled synthetic wherever its numbers appear.

Discrimination is AUROC against Harbor's verdict (fail = reward 0), oriented
so that higher predictor = more likely to fail, with a seeded bootstrap
interval; binary predictors also get their rate among failures and among
passes, which is what a reader needs to decide whether a flag is worth
gating on, and the others their per-class median, so every number the
research doc argues from is in this one table. Trials with no verdict (the verifier never ran) and trials with
no session file are counted and excluded, never folded in as either class.

`--appraise` adds the one paid pass: the quarantined appraiser
(`mecha sessions appraise --appraise`, §3.10 of the appraisal review) driven
once per session over the same synthesised store, against the local model
the scratch home's config names. Its evidence is numbers only — the signed
error counts, the label, whether a goal was named, the homeostat's two
readings — never the transcript, so this measures whether a model can add a
signed error the counters could not, on the counters alone. About fifteen
seconds a session on the local server.

The result belongs beside `docs/APPRAISAL-RESEARCH.md` §1's table.
"""

import argparse
import collections
import json
import os
import pathlib
import random
import re
import shutil
import statistics
import subprocess
import sys
import tempfile

# Enough that the sessions from 2026-08 admit through `Scan::in_window`
# regardless of when this is re-run; the filter exists for the live store.
DAYS = "3650"

STOP_CAUSES_EARLY = {"max_turns", "output_token_budget", "cost_budget", "loop", "no_output", "interrupted"}


# ─── The join ────────────────────────────────────────────────────────────────


class Trial:
    def __init__(self, run, name, result, session):
        self.run = run
        self.name = name
        self.result = result  # parsed result.json, or None
        self.session = session  # path to the one session file, or None
        self.task = name.rsplit("__", 1)[0]

    @property
    def reward(self):
        vr = (self.result or {}).get("verifier_result") or {}
        rewards = vr.get("rewards") or {}
        return rewards.get("reward")

    @property
    def failed(self):
        r = self.reward
        return None if r is None else (r < 1.0)

    @property
    def exception(self):
        e = (self.result or {}).get("exception_info") or {}
        kind = e.get("exception_type")
        m = re.search(r"exit (\d+)", e.get("exception_message") or "")
        return kind, (int(m.group(1)) if m else None)

    @property
    def wall_seconds(self):
        ex = (self.result or {}).get("agent_execution") or {}
        try:
            from datetime import datetime

            a = datetime.fromisoformat(ex["started_at"].replace("Z", "+00:00"))
            b = datetime.fromisoformat(ex["finished_at"].replace("Z", "+00:00"))
            return (b - a).total_seconds()
        except (KeyError, ValueError, TypeError):
            return None

    @property
    def mecha_version(self):
        return ((self.result or {}).get("agent_info") or {}).get("version")


def load_trials(jobs):
    trials, no_result, no_session, many_sessions = [], 0, 0, 0
    for run in sorted(p for p in jobs.iterdir() if p.is_dir()):
        for trial in sorted(p for p in run.iterdir() if p.is_dir()):
            result = None
            if (trial / "result.json").exists():
                with open(trial / "result.json") as f:
                    result = json.load(f)
            else:
                no_result += 1
            sessions = sorted((trial / "agent" / "sessions").glob("*.jsonl"))
            if not sessions:
                no_session += 1
                session = None
            else:
                if len(sessions) > 1:
                    many_sessions += 1
                session = sessions[0]
            trials.append(Trial(run.name, trial.name, result, session))
    return trials, {"no_result": no_result, "no_session": no_session, "many_sessions": many_sessions}


# ─── Layer 2: the counters, reconstructed ────────────────────────────────────


def read_records(path):
    records, bad = [], 0
    with open(path) as f:
        for line in f:
            if not line.strip():
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                bad += 1
    return records, bad


def reconstruct(trial):
    """What `RunStats` would have held, from the records the loop would have folded.

    Returns (stats-as-dict-for-the-outcome-record, extras-for-the-report)."""
    records, bad_lines = read_records(trial.session)
    config = next((r for r in records if r.get("record") == "config"), {})
    summaries = [r for r in records if r.get("record") == "summary"]
    taints = [r for r in records if r.get("record") == "taint"]
    rewrites = sum(1 for r in records if r.get("record") == "rewrite")
    messages = [r for r in records if r.get("record") == "message"]

    tool_calls = 0
    results = []  # is_error per tool_result, in order
    assistant_turns = 0
    for m in messages:
        blocks = m.get("content") or []
        if m.get("role") == "assistant":
            assistant_turns += 1
            tool_calls += sum(1 for b in blocks if b.get("type") == "tool_use")
        else:
            results.extend(bool(b.get("is_error")) for b in blocks if b.get("type") == "tool_result")
    tool_errors = sum(results)

    summary = summaries[-1] if summaries else None
    turns = summary["turns"] if summary else assistant_turns
    usage = (summary or {}).get("usage") or {}
    max_turns = config.get("max_turns")

    kind, code = trial.exception
    if kind in ("AgentTimeoutError", "CancelledError"):
        stop_cause = "interrupted"
        stop_source = "harbor:" + kind
    elif code == 3:
        stop_cause = "no_output"
        stop_source = "exit 3"
    elif code == 2:
        stop_cause = None  # a refusal is a StopReason, not a StopCause
        stop_source = "exit 2 (refusal)"
    elif code is not None:
        stop_cause = None
        stop_source = f"exit {code}"
    elif summary is None:
        stop_cause = None
        stop_source = "no summary record"
    elif max_turns and turns >= max_turns:
        stop_cause = "max_turns"
        stop_source = "turns >= config.max_turns"
    else:
        stop_cause = "completed"
        stop_source = "summary present, under max_turns"

    # `RunOutcome`'s own rule: only a `Completed` run can set it, and it is
    # the *last* call that counts.
    ended_on_failed_call = stop_cause == "completed" and bool(results) and results[-1]

    stats = {
        "record": "outcome",
        "turns": turns,
        "usage": {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "cache_creation_input_tokens": usage.get("cache_creation_input_tokens", 0),
            "cache_read_input_tokens": usage.get("cache_read_input_tokens", 0),
        },
        "usage_complete": summary is not None,
        "exhausted": stop_cause == "max_turns",
        "ended_on_failed_call": ended_on_failed_call,
        "tool_calls": tool_calls,
        "tool_errors": tool_errors,
        # Not reconstructible from a transcript; left at the record's default
        # and named in the report as such.
        "tool_denied": 0,
        "tool_staged": 0,
        "malformed_tool_args": 0,
        "blocked_sends": 0,
        "compactions": rewrites,
        "taint": taints[-1] and {"private": taints[-1].get("private", False), "untrusted": taints[-1].get("untrusted", False)}
        if taints
        else {"private": False, "untrusted": False},
    }
    if stop_cause is not None:
        stats["stop_cause"] = stop_cause
    meta = next((r for r in records if r.get("record") == "meta"), {})
    extras = {
        # The id the CLI prints its reasoning line under is the transcript's
        # own `meta.id`, not the filename; they agree for anything mecha
        # wrote, and reading it from the record keeps a copied file whose
        # name drifted from turning "no reasoning" into a silent None.
        "meta_id": meta.get("id"),
        "bad_lines": bad_lines,
        "has_summary": summary is not None,
        "assistant_turns": assistant_turns,
        "max_turns": max_turns,
        "stop_source": stop_source,
        "mecha_version": config.get("mecha_version"),
    }
    return stats, extras


# ─── Layers 1 and 3: the real reader ─────────────────────────────────────────


def run_appraise(mecha, home, session_dir, appraise=False, session_id=None, model=None):
    """The free readout, or with `appraise` the paid pass too (one appraisal,
    since the store holds one session). Returns the CLI's JSON plus, for the
    paid pass, the appraiser's own reasoning line off stderr as `reasoning`
    — the model's words, kept as data beside the verdict."""
    env = dict(os.environ, MECHA_HOME=str(home), MECHA_SESSION_DIR=str(session_dir))
    # The operator's shell must not reach the readout. `MECHA_SESSION_KIND`
    # would mark nothing here (it only marks writes) but is popped on
    # principle; `MECHA_LOG` writes trace lines to stderr *after* the
    # appraiser's reasoning line, which is read to the end of stderr; and
    # `MECHA_PROVIDER` / `MECHA_MODEL` merge *above* the scratch config, so
    # the model that answered could differ from the one `/props` named and
    # `--out` recorded (found on review). The scratch home is also the cwd,
    # so no `mecha.toml` in the operator's directory layers in either.
    for var in ("MECHA_SESSION_KIND", "MECHA_LOG", "MECHA_PROVIDER", "MECHA_MODEL"):
        env.pop(var, None)
    cmd = [mecha, "sessions", "appraise", "--days", DAYS, "--json"]
    if appraise:
        # Pinned on the command line as well as in the scratch config: flags
        # sit above every layer, so this is the one spelling nothing can
        # override, and it names the alias `/props` answered.
        cmd += ["--appraise", "--max-appraisals", "1", "--provider", "local", "--model", model]
    p = subprocess.run(cmd, env=env, cwd=str(home), capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"{mecha} sessions appraise failed over {session_dir}:\n{p.stderr}")
    start = p.stdout.find("{")
    if start < 0:
        sys.exit(f"no JSON from {mecha} sessions appraise over {session_dir}:\n{p.stdout}\n{p.stderr}")
    out = json.loads(p.stdout[start:])
    if appraise:
        # The `· <session>: ` prefix is shared with the harness's own failure
        # line ("appraiser call failed: …"), so a failed pass keeps `None`
        # here rather than filing our error text as the model's words. The
        # reasoning runs from its prefix line to the end of stderr, since a
        # reply that spans lines carries the prefix only on the first.
        # Anchored on this session's own prefix and the *first* match: with
        # one appraisal per store there is exactly one such line, and any
        # later `· ` is inside the model's reply (a bulleted line).
        out["reasoning"] = None
        if out["appraiser"]["failed"] == 0:
            prefix = f"\u00b7 {session_id}: "
            i = p.stderr.find(prefix)
            if i >= 0:
                out["reasoning"] = p.stderr[i + len(prefix):].strip() or None
    return out


def write_scratch_config(home, base_url, model):
    """The paid pass needs a provider; the free one must not see this
    machine's config at all, so the scratch home gets exactly one local
    provider and nothing else."""
    (home / "config.toml").write_text(
        "default_provider = \"local\"\n\n[providers.local]\nkind = \"local\"\n"
        f"base_url = \"{base_url}\"\nmodel = \"{model}\"\n"
    )


def served_model(base_url):
    """Ask the server what it serves (`/props` → `model_alias`); llama-server
    ignores the request's `model` field, so asserting one would be a guess."""
    import urllib.request

    try:
        with urllib.request.urlopen(f"{base_url}/props", timeout=5) as r:
            alias = json.load(r).get("model_alias")
    except Exception as e:  # noqa: BLE001 — any failure is "not reachable"
        sys.exit(f"--appraise needs a reachable local server at {base_url}: {e}")
    # `model_alias` is optional on the wire; a run recorded against `None`
    # would be the assert-what-is-served failure this function exists to
    # prevent, arriving quietly (found on review).
    if not alias:
        sys.exit(f"{base_url}/props names no model_alias; --appraise will not guess one")
    return alias


# ─── Discrimination ──────────────────────────────────────────────────────────


def auroc(pairs):
    """AUROC of `score` for `positive`, from (score, positive) pairs; ties count half.

    Rank-based (Mann–Whitney), so a bootstrap over it is cheap."""
    pos = sum(1 for _, y in pairs if y)
    neg = len(pairs) - pos
    if pos == 0 or neg == 0:
        return None
    ordered = sorted(pairs, key=lambda t: t[0])
    ranks, i = [0.0] * len(ordered), 0
    while i < len(ordered):
        j = i
        while j + 1 < len(ordered) and ordered[j + 1][0] == ordered[i][0]:
            j += 1
        avg = (i + j) / 2 + 1  # 1-based average rank across the tie
        for k in range(i, j + 1):
            ranks[k] = avg
        i = j + 1
    rank_sum = sum(r for r, (_, y) in zip(ranks, ordered) if y)
    return (rank_sum - pos * (pos + 1) / 2) / (pos * neg)


def bootstrap_ci(pairs, draws, seed):
    rng = random.Random(seed)
    values = []
    for _ in range(draws):
        sample = [pairs[rng.randrange(len(pairs))] for _ in pairs]
        a = auroc(sample)
        if a is not None:
            values.append(a)
    if not values:
        return None, None
    values.sort()
    lo = values[int(0.025 * (len(values) - 1))]
    hi = values[int(0.975 * (len(values) - 1))]
    return lo, hi


def discrimination(rows, key, draws, seed):
    """One predictor's line: n, AUROC with interval, and for a 0/1 predictor its
    rate in each class. `key` returns None to exclude a row (a rate over no
    calls is None, never zero)."""
    pairs = [(v, r["failed"]) for r in rows if (v := key(r)) is not None]
    if not pairs:
        return None
    a = auroc(pairs)
    lo, hi = bootstrap_ci(pairs, draws, seed) if a is not None else (None, None)
    out = {"n": len(pairs), "auroc": a, "ci_low": lo, "ci_high": hi}
    fails = [v for v, y in pairs if y]
    passes = [v for v, y in pairs if not y]
    if all(v in (0, 1, True, False) for v, _ in pairs):
        out["rate_in_fail"] = sum(map(int, fails)) / len(fails) if fails else None
        out["rate_in_pass"] = sum(map(int, passes)) / len(passes) if passes else None
    else:
        # The per-class centre for a count or a duration, so the prose that
        # argues from "passes made more errored calls" is reading this table
        # and not a number typed in from somewhere else (found on review).
        out["median_in_fail"] = statistics.median(fails) if fails else None
        out["median_in_pass"] = statistics.median(passes) if passes else None
    return out


# ─── Report ──────────────────────────────────────────────────────────────────


def fmt(x, digits=2):
    if x is None:
        return "—"
    if isinstance(x, bool):
        return "yes" if x else "no"
    if isinstance(x, float):
        return f"{x:.{digits}f}"
    return str(x)


def median(x):
    """`1` and `3`, not `1.0` and `3` — `statistics.median` returns whichever
    the sample had, and a table should not look like two precisions."""
    return "—" if x is None else f"{float(x):g}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--jobs", type=pathlib.Path, default=pathlib.Path("jobs/mecha-arm64-subset"))
    ap.add_argument("--mecha", default=shutil.which("mecha") or "mecha", help="the binary whose readout is measured")
    ap.add_argument("--out", type=pathlib.Path, help="write the full per-session join and every figure as JSON")
    ap.add_argument("--draws", type=int, default=2000, help="bootstrap resamples for the AUROC interval")
    ap.add_argument("--seed", type=int, default=20260903)
    ap.add_argument("--keep", action="store_true", help="leave the scratch stores on disk and print where")
    ap.add_argument("--appraise", action="store_true", help="also drive the quarantined appraiser once per session (paid; ~15 s each)")
    ap.add_argument("--base-url", default="http://127.0.0.1:8080", help="the local server the appraiser runs against")
    args = ap.parse_args()
    # The readout runs with the scratch home as cwd, and a relative executable
    # would be resolved against *that* — `--mecha target/release/mecha` would
    # die naming a path nobody typed. Resolve it here, before any cwd changes.
    if os.sep in args.mecha:
        args.mecha = str(pathlib.Path(args.mecha).resolve())

    if not args.jobs.is_dir():
        sys.exit(f"{args.jobs} is not a directory (it is gitignored; run against the checkout that holds it)")
    version = subprocess.run([args.mecha, "--version"], capture_output=True, text=True).stdout.strip()

    trials, listing = load_trials(args.jobs)
    scratch = pathlib.Path(tempfile.mkdtemp(prefix="appraisal-validity-"))
    home = scratch / "home"
    home.mkdir()
    appraiser_model = None
    if args.appraise:
        appraiser_model = served_model(args.base_url)
        write_scratch_config(home, args.base_url, appraiser_model)
        print(f"appraiser: {appraiser_model} at {args.base_url}", file=sys.stderr)

    # Layer 1: as-is.
    verbatim = scratch / "verbatim"
    verbatim.mkdir()
    for t in trials:
        if t.session:
            shutil.copy(t.session, verbatim / t.session.name)
    as_is = run_appraise(args.mecha, home, verbatim)

    # Layers 2 and 3, per session.
    rows = []
    excluded = collections.Counter()
    for t in trials:
        if t.session is None:
            continue
        if t.failed is None:
            excluded["no verdict"] += 1
            continue
        stats, extras = reconstruct(t)
        one = scratch / "synth" / t.name / "sessions"
        one.mkdir(parents=True)
        with open(t.session) as src, open(one / t.session.name, "w") as dst:
            text = src.read()
            dst.write(text if text.endswith("\n") else text + "\n")
            dst.write(json.dumps(stats) + "\n")
        readout = run_appraise(args.mecha, home, one)
        if readout["appraised"] != 1:
            sys.exit(f"{t.name}: expected one appraisal over the synthesised outcome, got {readout}")
        v = readout["valence"]
        appraiser = None
        if args.appraise:
            paid = run_appraise(
                args.mecha,
                home,
                one,
                appraise=True,
                session_id=extras["meta_id"] or t.session.stem,
                model=appraiser_model,
            )
            tally = paid["appraiser"]
            # The pass must have been driven, once: "the model looked and
            # found nothing" and "no appraisal ran" are opposite findings,
            # and a `sign` derived by elimination would fold them (found on
            # review). So the answer comes from the counter that means it.
            if tally["driven"] != 1 or tally["over_budget"]:
                sys.exit(f"{t.name}: expected one driven appraisal, got {tally}")
            sign = (
                -1 if tally["found_negative"]
                else 1 if tally["found_positive"]
                else 0 if tally["found_nothing"]
                else None
            )
            appraiser = {
                "driven": tally["driven"],
                "failed": tally["failed"],
                # The appraiser's own signed error, oriented like the rest:
                # higher = worse. `None` when the pass failed (a malformed
                # reply twice), which is "not answered", never "nothing".
                "sign": None if tally["failed"] or sign is None else -sign,
                # `None` on a failed pass, like `sign`: the appraiser never
                # ran `apply_appraiser`, so the valence here would be the free
                # readout's own number filed as "the appraiser added zero".
                "negative_with_appraiser": None if tally["failed"] else paid["valence"]["negative"],
                "reasoning": paid.get("reasoning"),
            }
        rows.append(
            {
                "run": t.run,
                "trial": t.name,
                "task": t.task,
                "session": t.session.name,
                "mecha_version": extras["mecha_version"] or t.mecha_version,
                "reward": t.reward,
                "failed": t.failed,
                "harbor_exception": t.exception[0],
                "harbor_exit_code": t.exception[1],
                "wall_seconds": t.wall_seconds,
                "reconstructed": {k: v_ for k, v_ in stats.items() if k != "record"},
                "reconstruction": extras,
                "readout": {
                    "labels": readout["labels"],
                    "channels": readout["channels"],
                    "negative": v["negative"],
                    "negatives": v["negatives"],
                    "positive": v["positive"],
                    "positives": v["positives"],
                    "signed": v["signed_sessions"] == 1,
                    "partial": v["partial"],
                },
                "appraiser": appraiser,
            }
        )

    # ── Predictors, all oriented "higher = worse" ──
    rc = lambda name: (lambda r: r["reconstructed"][name])
    predictors = [
        ("readout", "valence.negative (of_session over the synthesised outcome)", lambda r: r["readout"]["negative"]),
        ("readout", "any signed error", lambda r: int(r["readout"]["signed"])),
        ("readout", "label ≠ neutral", lambda r: int(r["readout"]["labels"].get("neutral", 0) == 0)),
        ("counter", "stop_cause early (not completed; None excluded)", lambda r: None if r["reconstructed"].get("stop_cause") is None else int(r["reconstructed"]["stop_cause"] in STOP_CAUSES_EARLY)),
        # The matched baseline for the `Interrupted` split (§3.3): the same
        # predictor with `interrupted` left out, so the pair above/below is
        # the split's own effect and not every unsigned cause folded in.
        ("counter", "stop_cause early, interrupted excluded (the readout's own rule)", lambda r: None if r["reconstructed"].get("stop_cause") is None else int(r["reconstructed"]["stop_cause"] in STOP_CAUSES_EARLY - {"interrupted"})),
        ("counter", "exhausted (max_turns)", lambda r: int(r["reconstructed"]["exhausted"])),
        ("counter", "ended_on_failed_call", lambda r: int(r["reconstructed"]["ended_on_failed_call"])),
        ("counter", "tool_errors", rc("tool_errors")),
        ("counter", "tool_error_rate (no calls → excluded)", lambda r: (r["reconstructed"]["tool_errors"] / r["reconstructed"]["tool_calls"]) if r["reconstructed"]["tool_calls"] else None),
        ("counter", "tool_calls", rc("tool_calls")),
        ("counter", "turns", rc("turns")),
        ("counter", "compactions", rc("compactions")),
        ("counter", "output_tokens (summary; 0 when none)", lambda r: r["reconstructed"]["usage"]["output_tokens"]),
        ("counter", "input_tokens (summary; 0 when none)", lambda r: r["reconstructed"]["usage"]["input_tokens"]),
        ("readout", "valence.negative, completed runs only (the clean-but-wrong regime)", lambda r: r["readout"]["negative"] if r["reconstructed"].get("stop_cause") == "completed" else None),
        ("counter", "stop_cause interrupted (Harbor timeout/cancel; skipped by of_session)", lambda r: int(r["reconstructed"].get("stop_cause") == "interrupted")),
        ("harbor", "wall-clock seconds", lambda r: r["wall_seconds"]),
    ]
    if args.appraise:
        predictors += [
            ("appraiser", "appraiser's signed error (−1 found positive · 0 nothing · +1 found negative)", lambda r: (r["appraiser"] or {}).get("sign")),
            ("appraiser", "found a negative error", lambda r: None if (r["appraiser"] or {}).get("sign") is None else int(r["appraiser"]["sign"] > 0)),
            ("appraiser", "valence.negative with the appraiser's error added", lambda r: (r["appraiser"] or {}).get("negative_with_appraiser")),
            ("appraiser", "… completed runs only", lambda r: (r["appraiser"] or {}).get("negative_with_appraiser") if r["reconstructed"].get("stop_cause") == "completed" else None),
        ]
    predictors += [
        ("harbor", "no summary record (process died)", lambda r: int(not r["reconstruction"]["has_summary"])),
    ]
    table = []
    for group, name, key in predictors:
        d = discrimination(rows, key, args.draws, args.seed)
        table.append({"group": group, "predictor": name, **(d or {"n": 0})})

    # ── The silent failures: what the readout cannot see ──
    fails = [r for r in rows if r["failed"]]
    passes = [r for r in rows if not r["failed"]]
    silent_fails = [r for r in fails if not r["readout"]["signed"]]
    silent_completed_fails = [
        r for r in silent_fails if r["reconstructed"].get("stop_cause") == "completed"
    ]
    by_stop = collections.Counter(
        (r["reconstructed"].get("stop_cause") or "unknown", r["failed"]) for r in rows
    )
    by_run = collections.OrderedDict()
    for r in rows:
        b = by_run.setdefault(r["run"], {"version": r["mecha_version"], "n": 0, "fail": 0, "rows": []})
        b["n"] += 1
        b["fail"] += int(r["failed"])
        b["rows"].append(r)
    for b in by_run.values():
        d = discrimination(b["rows"], lambda r: r["readout"]["negative"], args.draws, args.seed)
        b["auroc_negative"] = d and d["auroc"]
        del b["rows"]

    # ── Print ──
    print(f"# Appraisal validity over {args.jobs}\n")
    print(f"readout binary: `{version}`  ·  sessions: {sum(1 for t in trials if t.session)} of {len(trials)} trials")
    print(
        f"trials without result.json: {listing['no_result']}  ·  without a session file: {listing['no_session']}"
        f"  ·  with more than one session file (first by name taken): {listing['many_sessions']}"
        f"  ·  excluded, no verdict: {excluded['no verdict']}  ·  joined: {len(rows)} ({len(fails)} fail, {len(passes)} pass)\n"
    )
    print("## 1. The readout as-is\n")
    print(
        f"`mecha sessions appraise --json` over the {as_is['sessions_read']} sessions verbatim: "
        f"**appraised {as_is['appraised']}**, unreadable {as_is['sessions_unreadable']}. "
        "No session carries an `outcome` record (mecha 0.1.0–0.1.3), so `for_transcript` answers "
        "\"read, nothing to appraise\" for every one.\n"
    )
    print("## 2. Discrimination against Harbor's verdict (fail = reward 0)\n")
    print("| source | predictor (higher = worse) | n | AUROC | 95% CI | in fail | in pass |")
    print("|---|---|---|---|---|---|---|")
    print("| | *(binary: rate · otherwise: median)* | | | | | |")
    for t in table:
        ci = f"{fmt(t.get('ci_low'))}–{fmt(t.get('ci_high'))}" if t.get("ci_low") is not None else "—"
        if "rate_in_fail" in t:
            in_fail, in_pass = fmt(t["rate_in_fail"]), fmt(t["rate_in_pass"])
        else:
            in_fail = f"median {median(t.get('median_in_fail'))}"
            in_pass = f"median {median(t.get('median_in_pass'))}"
        print(
            f"| {t['group']} | {t['predictor']} | {t['n']} | {fmt(t.get('auroc'))} | {ci} "
            f"| {in_fail} | {in_pass} |"
        )
    print("\n## 3. What the readout cannot see\n")
    print(f"- failures silent to the readout (no signed error): **{len(silent_fails)} of {len(fails)}**")
    print(
        f"- of those, runs that `completed` under budget with no failed final call: "
        f"**{len(silent_completed_fails)}** — the case the design says only a verdict can reach"
    )
    print(f"- passes with a signed negative: {sum(1 for r in passes if r['readout']['signed'])} of {len(passes)}\n")
    print("| stop cause (reconstructed) | fail | pass |")
    print("|---|---|---|")
    for cause in sorted({c for c, _ in by_stop}):
        print(f"| {cause} | {by_stop[(cause, True)]} | {by_stop[(cause, False)]} |")
    print("\n## 4. By Harbor run\n")
    print("| run | mecha | n | fail | AUROC of valence.negative |")
    print("|---|---|---|---|---|")
    for run, b in by_run.items():
        print(f"| {run} | {b['version']} | {b['n']} | {b['fail']} | {fmt(b['auroc_negative'])} |")
    if args.appraise:
        tally = collections.Counter()
        for r in rows:
            a = r["appraiser"] or {}
            tally["failed" if a.get("failed") else {-1: "found positive", 0: "found nothing", 1: "found negative"}[a.get("sign", 0)]] += 1
        found_neg = [r for r in rows if (r["appraiser"] or {}).get("sign") == 1]
        print(f"\n## 5. The appraiser's marginal yield (§3.10)\n")
        print(f"model: `{appraiser_model}`  ·  driven: {len(rows)}  ·  {dict(tally)}")
        print(
            f"- of the {len(silent_fails)} failures silent to the readout, the appraiser signed "
            f"**{sum(1 for r in silent_fails if (r['appraiser'] or {}).get('sign') == 1)}** negative"
        )
        print(
            f"- of the {len(found_neg)} runs it signed negative, {sum(1 for r in found_neg if r['failed'])} failed "
            f"and {sum(1 for r in found_neg if not r['failed'])} passed"
        )
    channels = collections.Counter()
    for r in rows:
        for c, n in r["readout"]["channels"].items():
            channels[c] += n
    labels = collections.Counter()
    for r in rows:
        for l, n in r["readout"]["labels"].items():
            labels[l] += n
    print(f"\nsigned errors by channel over the joined set: {dict(channels) or 'none'}  ·  labels: {dict(labels)}")
    print(
        "\nnot reconstructible, left at default: tool_denied, tool_staged, malformed_tool_args, "
        "blocked_sends, the homeostat, context_overflows, boredom/step-escalation/check counters."
    )

    if args.out:
        with open(args.out, "w") as f:
            json.dump(
                {
                    "jobs": str(args.jobs),
                    "readout_binary": version,
                    "listing": listing,
                    "as_is": as_is,
                    "excluded": dict(excluded),
                    "discrimination": table,
                    "silent_failures": len(silent_fails),
                    "silent_completed_failures": len(silent_completed_fails),
                    "by_stop_cause": {f"{c}/{'fail' if y else 'pass'}": n for (c, y), n in by_stop.items()},
                    "by_run": by_run,
                    "channels": dict(channels),
                    "labels": dict(labels),
                    "appraiser_model": appraiser_model,
                    "rows": rows,
                },
                f,
                indent=2,
            )
        print(f"\nwrote {args.out}")

    if args.keep:
        print(f"scratch stores kept at {scratch}")
    else:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    main()
