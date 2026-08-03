#!/usr/bin/env python3
"""Generate the fixtures the harder eval cases read, and compute their answers.

The point of generating rather than hand-writing: a gold answer typed by hand
is a guess. One shipped in this case set was wrong ($2,450 for a total that is
actually $1,750) because a base rate got double-counted, and a wrong gold answer
measures nothing — every model fails it and the failure means nothing.

Run from the repo root:

    python3 scripts/build-eval-fixtures.py

It rewrites eval/workspace/{audit,reports,kata} and prints the values the cases
in eval/cases.jsonl must assert. If you change a fixture, re-run it and update
the case file to match what it prints.
"""

import hashlib
import os
import pathlib
import random
import shutil
import subprocess
import sys
import textwrap

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKSPACE = ROOT / "eval" / "workspace"

# A __pycache__ left in the fixture would be copied into every sandbox and shown
# to the model as part of the workspace.
NO_BYTECODE = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}


def clean(path):
    """Start from empty. Stale files from an older fixture are still readable by
    the agent, and a case would silently be measuring both."""
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)
    return path


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text).lstrip(), encoding="utf-8")


# --------------------------------------------------------------------------
# audit/ — a 16-link chain for the long-horizon cases.
#
# Each entry names the next one and nothing else does, so the only way to the
# end is one read at a time with the running total carried along. The decoys
# are the load-bearing part: without them a model could list the directory,
# read everything at once and sum it, which measures retrieval rather than
# long-horizon state.
# --------------------------------------------------------------------------
def build_audit():
    rng = random.Random(20260802)
    out = clean(WORKSPACE / "audit")

    chain_len = 16
    ids = [f"{rng.randrange(16**4):04x}" for _ in range(chain_len - 1)]
    assert len(set(ids)) == len(ids), "regenerate: id collision"
    names = ["START.md"] + [f"entry-{i}.md" for i in ids]
    amounts = [rng.randrange(11, 99) for _ in range(chain_len)]

    for i, (name, amount) in enumerate(zip(names, amounts)):
        nxt = names[i + 1] if i + 1 < chain_len else "END"
        write(
            out / name,
            f"""
            # Audit entry {i + 1:02d}

            amount: {amount}
            next: {nxt}
            """,
        )

    # Decoys: real files, real amounts, unreachable from START.
    decoys = [f"{rng.randrange(16**4):04x}" for _ in range(7)]
    for i in decoys:
        name = f"entry-{i}.md"
        assert name not in names, "regenerate: decoy collides with the chain"
        write(
            out / name,
            f"""
            # Audit entry (superseded)

            amount: {rng.randrange(101, 999)}
            next: VOID
            """,
        )

    largest = max(range(chain_len), key=lambda i: amounts[i])
    return {
        "chain length": chain_len,
        "chain total": sum(amounts),
        "largest amount": amounts[largest],
        "largest entry": names[largest],
        "sum of every file (the wrong answer)": "computed below",
        "decoy count": len(decoys),
    }


# --------------------------------------------------------------------------
# reports/ — six documents that partly disagree, for the synthesis cases.
#
# Two separate traps. The throughput question has a majority figure and one
# vendor number measured under different conditions; the latency question has a
# flattering old figure that a later report explicitly supersedes. Reading any
# one document gives a confident wrong answer to one of them.
# --------------------------------------------------------------------------
def build_reports():
    out = clean(WORKSPACE / "reports")

    write(
        out / "2026-01-vendor-sheet.md",
        """
        # Kestrel X100 — performance sheet

        Source: Kestrel Systems, sales collateral. Not independently verified.

        Sustained throughput: **3,600 tokens/sec**.

        Measured at batch size 64 with fp8 weights on the reference chassis.
        Single-stream figures are available on request.
        """,
    )
    write(
        out / "2026-02-early-draft.md",
        """
        # X100 evaluation — early draft

        Status: DRAFT. Superseded by the June internal retest.

        Throughput: 1,238 tokens/sec (batch 1, bf16).
        Latency p99: **38 ms**, comfortably inside our 40 ms target.

        Caveat from the author: this ran on a pre-release firmware and a single
        node. Do not quote the latency figure without a retest.
        """,
    )
    write(
        out / "2026-03-lab-bench.md",
        """
        # Lab bench — X100

        Throughput: 1,240 tokens/sec (batch 1, bf16).
        Latency p99: 49 ms.

        Ten runs, median reported. Hardware as shipped.
        """,
    )
    write(
        out / "2026-04-partner-eval.md",
        """
        # Partner evaluation: Kestrel X100

        We measured 1,240 tokens/sec on our own harness, batch 1, bf16.

        This matches the lab bench figure. We did not measure tail latency.
        """,
    )
    write(
        out / "2026-05-independent.md",
        """
        # Independent review — Kestrel X100

        Throughput came out at 1,255 tokens/sec (batch 1, bf16), within noise of
        the two 1,240 figures we were asked to check against.

        Latency p99: 51 ms.
        """,
    )
    write(
        out / "2026-06-internal-retest.md",
        """
        # Internal retest — X100

        This retest **supersedes the February early draft**, which ran on
        pre-release firmware.

        Throughput: 1,240 tokens/sec (batch 1, bf16).
        Latency p99: **52 ms**.

        Conclusion: the X100 does not meet our 40 ms p99 target.
        """,
    )

    return {
        "majority throughput": "1,240 tok/s (3 of 6 reports)",
        "throughput outlier": "2026-01-vendor-sheet.md at 3,600 (batch 64, fp8)",
        "current p99": "52 ms (2026-06 retest) — misses the 40 ms target",
        "superseded p99": "38 ms (2026-02 draft) — the trap",
    }


# --------------------------------------------------------------------------
# kata/ — a real test command, for the codegen cases. Sandboxed cases only.
# --------------------------------------------------------------------------
def build_kata():
    out = clean(WORKSPACE / "kata")

    write(
        out / "stats.py",
        '''
        """Statistics helpers used by the reporting daemon."""


        def mean(values):
            if not values:
                raise ValueError("mean of an empty sequence")
            return sum(values) / len(values)


        def median(values):
            """Return the median of `values`.

            `values` is not necessarily sorted. With an even number of values the
            median is the mean of the two middle ones. An empty sequence is an
            error, as it is for `mean`.
            """
            raise NotImplementedError("median is not implemented yet")
        ''',
    )
    write(
        out / "test_stats.py",
        '''
        """Run me with: python3 kata/test_stats.py"""

        from stats import mean, median


        def check(actual, expected, label):
            assert actual == expected, f"{label}: expected {expected!r}, got {actual!r}"


        check(median([3, 1, 2]), 2, "odd length, unsorted")
        check(median([1, 2, 3, 4]), 2.5, "even length")
        check(median([7]), 7, "single value")
        check(median([5, 5, 5, 5]), 5, "all equal")
        check(median([-3, -1, -2]), -2, "negatives")
        check(mean([1, 2, 3]), 2, "mean still works")

        try:
            median([])
        except ValueError:
            pass
        else:
            raise AssertionError("median([]) should raise ValueError")

        print("ok")
        ''',
    )

    # A single-line bug, discoverable by reading: the minutes branch returns
    # before the hours are ever added.
    write(
        out / "parse.py",
        '''
        """Duration parsing for the scheduler."""

        import re

        _PATTERN = re.compile(r"^(?:(\\d+)h)?(?:(\\d+)m)?$")


        def parse_duration(text):
            """Parse "1h30m", "45m" or "2h" into a whole number of minutes."""
            text = text.strip().lower()
            match = _PATTERN.match(text)
            if not match or not any(match.groups()):
                raise ValueError(f"cannot parse duration {text!r}")

            hours, minutes = match.group(1), match.group(2)
            if minutes is not None:
                return int(minutes)
            return int(hours) * 60
        ''',
    )
    write(
        out / "test_parse.py",
        '''
        """Run me with: python3 kata/test_parse.py"""

        from parse import parse_duration


        def check(actual, expected, label):
            assert actual == expected, f"{label}: expected {expected!r}, got {actual!r}"


        check(parse_duration("45m"), 45, "minutes only")
        check(parse_duration("2h"), 120, "hours only")
        check(parse_duration("1h30m"), 90, "hours and minutes")
        check(parse_duration(" 3H15M "), 195, "whitespace and case")

        for bad in ["", "soon", "90"]:
            try:
                parse_duration(bad)
            except ValueError:
                pass
            else:
                raise AssertionError(f"parse_duration({bad!r}) should raise ValueError")

        print("ok")
        ''',
    )

    return {}


def sha_prefix(path):
    """The first 16 hex characters, matching `sha256sum | cut -c1-16`."""
    return hashlib.sha256(path.read_bytes()).hexdigest()[:16]


def main():
    audit = build_audit()
    reports = build_reports()
    build_kata()

    # The wrong answer a model gets by reading every file instead of the chain.
    total_all = 0
    for f in (WORKSPACE / "audit").glob("*.md"):
        for line in f.read_text().splitlines():
            if line.startswith("amount:"):
                total_all += int(line.split(":")[1])
    audit["sum of every file (the wrong answer)"] = total_all

    print("audit/")
    for k, v in audit.items():
        print(f"  {k:42} {v}")

    print("\nreports/")
    for k, v in reports.items():
        print(f"  {k:42} {v}")

    print("\nkata/ — verify commands for the case file")
    for name, cmd in [
        ("test_stats.py", "python3 kata/test_stats.py"),
        ("test_parse.py", "python3 kata/test_parse.py"),
    ]:
        digest = sha_prefix(WORKSPACE / "kata" / name)
        print(
            f'  test "$(sha256sum kata/{name} | cut -c1-16)" = "{digest}" && {cmd}'
        )

    # Two things have to be true of a codegen fixture, and neither is obvious
    # by inspection: the tests must FAIL as shipped, or the case passes without
    # the model doing anything; and they must be PASSABLE, or every model fails
    # a case that measures nothing. Both are checked here rather than trusted.
    print("\nkata/ — fixtures fail as shipped (they must):")
    ok = True
    for name in ("test_stats.py", "test_parse.py"):
        result = subprocess.run(
            [sys.executable, f"kata/{name}"], cwd=WORKSPACE, capture_output=True, text=True, env=NO_BYTECODE
        )
        state = "fails (good)" if result.returncode else "PASSES — the case measures nothing"
        print(f"  {name:16} {state}")
        ok &= bool(result.returncode)

    print("\nkata/ — solvable by a reference fix (they must be):")
    for name, target, fix in (
        ("test_stats.py", "stats.py", REFERENCE_MEDIAN),
        ("test_parse.py", "parse.py", REFERENCE_PARSE),
    ):
        original = (WORKSPACE / "kata" / target).read_text()
        try:
            (WORKSPACE / "kata" / target).write_text(textwrap.dedent(fix).lstrip())
            result = subprocess.run(
                [sys.executable, f"kata/{name}"],
                cwd=WORKSPACE,
                capture_output=True,
                text=True,
                env=NO_BYTECODE,
            )
        finally:
            (WORKSPACE / "kata" / target).write_text(original)

        passed = result.returncode == 0
        detail = "" if passed else f"  <- {result.stdout}{result.stderr}".rstrip()
        print(f"  {name:16} {'passes (good)' if passed else 'UNSOLVABLE'}{detail}")
        ok &= passed

    return 0 if ok else 1


# Kept here, never in the fixture: these are what a correct answer looks like,
# used only to prove the tests can be satisfied.
REFERENCE_MEDIAN = '''
    """Statistics helpers used by the reporting daemon."""


    def mean(values):
        if not values:
            raise ValueError("mean of an empty sequence")
        return sum(values) / len(values)


    def median(values):
        if not values:
            raise ValueError("median of an empty sequence")
        ordered = sorted(values)
        middle = len(ordered) // 2
        if len(ordered) % 2:
            return ordered[middle]
        return (ordered[middle - 1] + ordered[middle]) / 2
    '''

REFERENCE_PARSE = '''
    """Duration parsing for the scheduler."""

    import re

    _PATTERN = re.compile(r"^(?:(\\d+)h)?(?:(\\d+)m)?$")


    def parse_duration(text):
        """Parse "1h30m", "45m" or "2h" into a whole number of minutes."""
        text = text.strip().lower()
        match = _PATTERN.match(text)
        if not match or not any(match.groups()):
            raise ValueError(f"cannot parse duration {text!r}")

        hours, minutes = match.group(1), match.group(2)
        return int(hours or 0) * 60 + int(minutes or 0)
    '''


if __name__ == "__main__":
    sys.exit(main())
