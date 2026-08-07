#!/usr/bin/env python3
"""Assert a harbor job is running exactly the oracle-calibrated task set.

    bench/check-subset.py jobs/mecha-arm64-subset/<job-name>

Why this exists. `harbor run -x <name>` takes a glob matched against the
*dataset-qualified* task name (`terminal-bench/foo`), and a `-x` that matches
nothing is not an error — harbor excludes nothing and says nothing. A run
launched with bare names therefore executes all 89 tasks while every artifact
still reads `mecha-arm64-subset`, and the resulting scorecard is indexed by a
denominator that was never true. That happened twice here before it was
noticed, once on 2026-08-05 and once on 2026-08-07.

lock.json is written when the job starts and before any trial runs, so this is
a preflight: run it within seconds of launching, and kill the job if it fails.
Comparing counts alone is not enough — 75 of the wrong 75 is still wrong — so
this compares the actual name sets and prints the difference in both
directions.

Exit 0 iff the planned distinct tasks are exactly bench/oracle-arm64-subset.txt.
"""

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def names(path: Path) -> set[str]:
    return {
        line.strip()
        for line in path.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    }


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    job = Path(sys.argv[1])
    lock = job / "lock.json"
    if not lock.is_file():
        # The distinction matters: not-yet-written is "wait", absent-forever is
        # "the job never started", and reporting the second as the first is how
        # you sit watching a run that died at launch.
        print(f"no lock.json at {lock} — job not started yet, or it died before writing one")
        return 2

    trials = json.loads(lock.read_text())["trials"]
    planned = {t["task"]["name"] for t in trials}
    want = names(HERE / "oracle-arm64-subset.txt")

    ran_anyway = sorted(planned - want)
    missing = sorted(want - planned)

    print(f"trials planned: {len(trials)}   distinct tasks: {len(planned)}   expected: {len(want)}")
    if ran_anyway:
        print(f"\n!! {len(ran_anyway)} task(s) planned that the oracle CANNOT solve here —")
        print("   the exclusion did not take. Kill the job.")
        for n in ran_anyway:
            print(f"     + {n}")
    if missing:
        print(f"\n!! {len(missing)} calibrated task(s) absent from the plan —")
        print("   over-filtered, or the dataset ref moved.")
        for n in missing:
            print(f"     - {n}")
    if ran_anyway or missing:
        return 1

    k = len(trials) // len(planned) if planned else 0
    print(f"\nOK: exactly the {len(want)} oracle-passing tasks" + (f", k={k}" if k > 1 else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
