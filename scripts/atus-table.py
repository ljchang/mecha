#!/usr/bin/env python3
"""Estimate the day sampler's table from the American Time Use Survey.

`docs/PERSONA-LIFE-DESIGN.md` §4.2: a semi-Markov chain over the persona
activity set, estimated once from the public ATUS files and shipped as a
table. This script writes that table; `mecha-core/src/persona/atus.rs` reads
it and owns every decision about how to use it (backoff, smoothing). The
table therefore holds counts, never probabilities.

Run from the repo root:

    python3 scripts/atus-table.py            # downloads into ~/.cache/mecha/atus
    python3 scripts/atus-table.py --cache DIR

It rewrites mecha-core/data/atus/table.tsv.gz (`zcat` it to read it). Every
input is pinned by sha256, a file that does not match its pin is refused, and
the table's text is pinned too (`TEXT_SHA256`, which `persona::atus` checks
against the shipped file), so a re-run says whether the pinned inputs still
give the shipped rows. The pin is on the text, not the gzip file, whose bytes
depend on which zlib wrote them. It is compressed because it is a megabyte of
numbers no diff should carry: this script and its pins are what review reads.

The activity mapping below is a copy of `Activity::atus` in
`mecha-core/src/persona/activity.rs`. The table records the activity it gave
every code it met (`M` rows), and a Rust test checks each against the Rust
mapping, so the two cannot drift apart unnoticed: change one, re-run this.

ATUS is public domain (https://www.bls.gov/opub/copyright-information.htm).
"""

import argparse
import collections
import csv
import gzip
import hashlib
import io
import pathlib
import sys
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "mecha-core" / "data" / "atus" / "table.tsv.gz"
BASE = "https://www.bls.gov/tus/datafiles/"
# BLS refuses requests without a browser-like agent.
AGENT = "Mozilla/5.0 (compatible; mecha atus-table.py)"

YEARS = (2023, 2024, 2025)
PINS = {
    "atusresp-2023.zip": "5b24084ffd4c618c14e096429f99f0efa87e1393d7902c6007f96dd8725ba90c",
    "atusresp-2024.zip": "0e9a9b0061ab0aafa763c23d3a64f7487ae17d0a4594e6445c0f79b8ce74e085",
    "atusresp-2025.zip": "77566d2864f1b96412a46ceeee9ea53709bc191b092f3573af04c17b2d409873",
    "atusact-2023.zip": "c7f497f8ac91254b9ddf771f8de475dded5bc7ea39806a019e8cdee920989b33",
    "atusact-2024.zip": "b9d4a06482f6ac3a248fee87740537df13b9129ceb799f0d098fb27886153d2a",
    "atusact-2025.zip": "664314e9f0c7373534f10bb1450a1dfc2b50d04c7932ecfea090ff4a1c65bca3",
    "atuswho-2023.zip": "48830d85aacd5ee5f267ac551d944f84c36e15878840c6ceedd9015ecc656402",
    "atuswho-2024.zip": "32d76fba40ec0df7e6a9a4dcaf2d155fcfa0a0beaf1658331dd4816d98495cd0",
    "atuswho-2025.zip": "a2f884f2eab95f7ed5ffff1e60f1b7cde0e6ea41527e4aec8b0209742021aa9f",
    "atusrost-2023.zip": "f482824dade5bc9436170fef1175d0b26cae36b90eff772b71b5c977ea62b08a",
    "atusrost-2024.zip": "126b373b481a6ca572d87809bdf3554a883a86283124c31a7cd46011a82ad84f",
    "atusrost-2025.zip": "b10ac3ee0d2fd6229d8c0a1978a4497412f30b4ae16b24e393adc57a658ddd89",
}

# sha256 of the table's text (before gzip) that the pins above produce.
TEXT_SHA256 = "47629ab8cdf2f255c20f440113a50594392f02b6b7e07ba999e6e25325dfca9e"

# `Activity::atus`, longest prefix wins.
ACTIVITIES = {
    "sleep": ["0101"],
    "personal_care": ["01"],
    "eating": ["11"],
    "cooking": ["0202"],
    "housework": ["02"],
    "shopping": ["07"],
    "errands": ["08", "09", "10", "0604"],
    "caring": ["03", "04"],
    "work": ["05"],
    "class": ["06"],
    "studying": ["0603"],
    "commute": ["1805", "1806"],
    "travel": ["18"],
    "socializing": ["1201", "1202", "16"],
    "tv": ["120303", "120304"],
    "music": ["120305", "120306"],
    "games": ["120307"],
    "computer": ["120308"],
    "hobbies": ["120309", "120310", "120311", "120313", "0602"],
    "reading": ["120312"],
    "relaxing": ["12"],
    "outing": ["1204"],
    "exercise": ["13"],
    "sports_event": ["1302", "130302", "130402"],
    "religion": ["14"],
    "volunteering": ["15"],
}

# `Company` in atus.rs, in its order: who was there, from TUWHO_CODE.
# 18 and 19 are both "alone" (the codebook draws no distinction).
COMPANY = [
    ("alone", {18, 19}),
    ("partner", {20, 21}),
    ("child", {22, 23, 27, 40}),
    ("parent", {24, 51}),
    ("family", {25, 26, 52, 53}),
    ("housemate", {28, 29, 30}),
    ("friend", {54}),
    ("acquaintance", {56}),
    ("colleague", {59, 60, 61}),
    ("public", {57, 58, 62}),
]
# A positive who code in none of the sets above is refused: counted, it would
# read as a stretch nobody shared.
KNOWN_WHO = set().union(*(codes for _, codes in COMPANY))

# Clock hours a duration is conditioned on, by when the stretch starts.
BANDS = [("night", 22, 6), ("morning", 6, 12), ("afternoon", 12, 17), ("evening", 17, 22)]

# Below this many stretches a cell's duration row is not written, and the
# reader backs off to the pooled cell's.
MIN_RUNS = 20


def activity_of(code):
    best = None
    for name, prefixes in ACTIVITIES.items():
        for p in prefixes:
            if code.startswith(p) and (best is None or len(p) > len(best[1])):
                best = (name, p)
    return best[0] if best else None


def band_of(hour):
    for name, lo, hi in BANDS:
        if (lo <= hour < hi) if lo < hi else (hour >= lo or hour < hi):
            return name
    raise AssertionError(hour)


def cell_of(r, age):
    """The demographic cell a respondent's diary is counted in, or None.

    High-school students are left out: a persona is an adult."""
    employed = r["TELFS"] in ("1", "2")
    if r["TESCHENR"] == "1" and r["TESCHLVL"] == "1":
        return None
    if r["TESCHENR"] == "1" and r["TESCHLVL"] == "2" and r["TESCHFT"] == "1":
        return "student"
    if employed and r["TRDPFTPT"] == "1":
        youngest = int(r["TRYHHCHILD"])
        return "full_time_parent" if 0 <= youngest < 13 else "full_time"
    if employed:
        return "part_time"
    return "retired" if age >= 65 else "at_home"


def fetch(cache, name):
    path = cache / name
    if not path.exists():
        cache.mkdir(parents=True, exist_ok=True)
        req = urllib.request.Request(BASE + name, headers={"User-Agent": AGENT})
        with urllib.request.urlopen(req, timeout=120) as resp:
            data = resp.read()
        path.with_suffix(".part").write_bytes(data)
        path.with_suffix(".part").rename(path)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != PINS[name]:
        path.unlink()
        sys.exit(f"{path}: sha256 {digest} is not the pin {PINS[name]}; refused and removed")
    return path


def rows(cache, kind, year):
    with zipfile.ZipFile(fetch(cache, f"atus{kind}-{year}.zip")) as z:
        member = f"atus{kind}_{year}.dat"
        with z.open(member) as f:
            yield from csv.DictReader(io.TextIOWrapper(f, encoding="latin-1"))


def wquantiles(pairs, steps=20):
    """Weighted quantiles at 0, 1/steps, ..., 1 of (value, weight) pairs."""
    pairs = sorted(pairs)
    total = sum(w for _, w in pairs)
    out, cum, i = [], 0.0, 0
    for k in range(steps + 1):
        target = total * k / steps
        while i < len(pairs) - 1 and cum + pairs[i][1] < target:
            cum += pairs[i][1]
            i += 1
        out.append(pairs[i][0])
    return out


def duration_rows(cell, day, durs):
    for (band, act), pairs in sorted(durs.items()):
        if len(pairs) >= MIN_RUNS:
            n = sum(w for _, w in pairs)
            qs = "\t".join(str(q) for q in wquantiles(pairs))
            yield f"D\t{cell}\t{day}\t{band}\t{act}\t{n:.2f}\t{qs}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--cache", type=pathlib.Path, default=pathlib.Path.home() / ".cache/mecha/atus")
    args = ap.parse_args()

    diaries = []  # (cell, day, weight, runs)
    codes = {}
    for year in YEARS:
        ages = {r["TUCASEID"]: int(r["TEAGE"]) for r in rows(args.cache, "rost", year) if r["TULINENO"] == "1"}
        resp = {}
        for r in rows(args.cache, "resp", year):
            if r["TRHOLIDAY"] == "1":
                continue
            cell = cell_of(r, ages[r["TUCASEID"]])
            if cell is None:
                continue
            day = "weekend" if r["TUDIARYDAY"] in ("1", "7") else "weekday"
            resp[r["TUCASEID"]] = (cell, day, float(r["TUFINLWGT"]))
        who = collections.defaultdict(set)
        for r in rows(args.cache, "who", year):
            code = int(r["TUWHO_CODE"])
            if code > 0:
                if code not in KNOWN_WHO:
                    sys.exit(f"{year}: TUWHO_CODE {code} is in no COMPANY set; map it there first")
                who[(r["TUCASEID"], r["TUACTIVITY_N"])].add(code)
        episodes = collections.defaultdict(list)
        for r in rows(args.cache, "act", year):
            if r["TUCASEID"] not in resp:
                continue
            code = r["TRCODE"].zfill(6)
            act = activity_of(code)
            codes[code] = act
            hh, mm, _ = r["TUSTARTTIM"].split(":")
            company = who.get((r["TUCASEID"], r["TUACTIVITY_N"]))
            episodes[r["TUCASEID"]].append(
                (int(r["TUACTIVITY_N"]), act, int(hh) % 24, int(r["TUACTDUR"]), company)
            )
        for case, eps in episodes.items():
            eps.sort()
            # Stretches: consecutive episodes of one activity merge, and a
            # data code (50xxxx) breaks the chain rather than joining it.
            runs = []
            for n, act, hour, dur, company in eps:
                if runs and act is not None and runs[-1]["act"] == act:
                    runs[-1]["dur"] += dur
                    if company is not None:
                        runs[-1]["who"] = (runs[-1]["who"] or set()) | company
                    continue
                runs.append({"act": act, "hour": hour, "dur": dur, "who": company, "first": n == 1})
            cell, day, weight = resp[case]
            diaries.append((cell, day, weight, runs))

    cells = ["student", "full_time", "full_time_parent", "part_time", "at_home", "retired"]
    # One scale per day type across every cell, so a cell's counts sum to the
    # pooled cell's and the reader can add them; quantiles cannot be added,
    # so durations are also written for the pool, as cell "all".
    mean = {
        day: sum(w for _, d, w, _ in diaries if d == day) / sum(1 for _, d, _, _ in diaries if d == day)
        for day in ("weekday", "weekend")
    }
    out = [
        "# The day sampler's table (docs/PERSONA-LIFE-DESIGN.md §4.2). Generated by",
        "# scripts/atus-table.py; do not edit. Source: U.S. Bureau of Labor Statistics,",
        f"# American Time Use Survey {YEARS[0]}-{YEARS[-1]}, public domain. Weights are",
        "# normalized to a mean of 1 per day type, so a count is in diaries.",
        "V\t1",
        "C\t" + "\t".join(name for name, _ in COMPANY),
    ]
    for code in sorted(codes):
        out.append(f"M\t{code}\t{codes[code] or '-'}")
    pooled_durs = collections.defaultdict(list)
    for cell in cells:
        for day in ("weekday", "weekend"):
            members = [(w / mean[day], runs) for c, d, w, runs in diaries if c == cell and d == day]
            out.append(f"N\t{cell}\t{day}\t{len(members)}")
            trans = collections.defaultdict(collections.Counter)
            durs = collections.defaultdict(list)
            company = collections.defaultdict(lambda: [0.0] * (len(COMPANY) + 1))
            for w, runs in members:
                for prev, run in zip([None] + runs, runs):
                    if run["act"] is None:
                        continue
                    if prev is not None and prev["act"] is not None:
                        trans[(run["hour"], prev["act"])][run["act"]] += w
                    if not run["first"]:
                        key = (band_of(run["hour"]), run["act"])
                        durs[key].append((min(run["dur"], 1440), w))
                        pooled_durs[(day,) + key].append((min(run["dur"], 1440), w))
                    if run["who"] is not None:
                        row = company[run["act"]]
                        row[0] += w
                        for i, (_, wanted) in enumerate(COMPANY):
                            if run["who"] & wanted:
                                row[i + 1] += w
            for (hour, a), tos in sorted(trans.items()):
                pairs = " ".join(f"{b}={n:.2f}" for b, n in sorted(tos.items()) if round(n, 2) > 0)
                if pairs:
                    out.append(f"T\t{cell}\t{day}\t{hour}\t{a}\t{pairs}")
            out.extend(duration_rows(cell, day, durs))
            for act, row in sorted(company.items()):
                counts = "\t".join(f"{x:.2f}" for x in row)
                out.append(f"W\t{cell}\t{day}\t{act}\t{counts}")
    for day in ("weekday", "weekend"):
        durs = {k[1:]: v for k, v in pooled_durs.items() if k[0] == day}
        out.extend(duration_rows("all", day, durs))

    OUT.parent.mkdir(parents=True, exist_ok=True)
    text = ("\n".join(out) + "\n").encode()
    OUT.write_bytes(gzip.compress(text, compresslevel=9, mtime=0))
    kinds = collections.Counter(line[0] for line in out if not line.startswith("#"))
    print(f"wrote {OUT.relative_to(ROOT)}: {len(diaries)} diaries, " + ", ".join(f"{k}={v}" for k, v in sorted(kinds.items())))
    digest = hashlib.sha256(text).hexdigest()
    if digest != TEXT_SHA256:
        print(f"the table's text changed: sha256 {digest}; set TEXT_SHA256 here and in persona::atus")


if __name__ == "__main__":
    main()
