#!/usr/bin/env python3
# Two runs of ocr-measure.py side by side — the before/after in
# docs/DOCUMENT-EXTRACTION-DESIGN.md §8. Run from the directory holding
# pdfs/, after measuring the whole-page recipe into one OUT directory and the
# layout recipe (through mecha, which also writes each page's JSON) into
# another:
#
#   python3 scripts/ocr-compare.py out-whole out-layout
#
# Besides word recall and CER as ocr-measure.py computes them, it reports
# *prose* word recall: the same recall over the text layer's words outside
# the layout model's formula regions (the after run's boxes, applied to both
# runs). Display math read as LaTeX is right and scores as missing words
# against the text layer's Unicode fragments; this separates "read the words"
# from "wrote the maths differently".
import json, os, re, statistics, subprocess, sys, html

sys.path.insert(0, os.path.dirname(__file__))
MASK = {"display_formula", "inline_formula", "formula_number"}


def norm(s):
    s = re.sub(r"\\[()\[\]]", " ", s)
    s = s.replace("$", " ")
    s = re.sub(r"^#+\s*", "", s, flags=re.M)
    s = re.sub(r"[|*_`]|^-{3,}$", " ", s, flags=re.M)
    return " ".join(s.split())


def words(s):
    return [w.strip(".,;:()[]{}\"'").lower() for w in s.split() if w.strip(".,;:()[]{}\"'")]


def prose_ref(pdf, page, boxes):
    x = subprocess.run(["pdftotext", "-bbox", "-f", str(page), "-l", str(page), pdf, "-"],
                       capture_output=True, text=True).stdout
    kept = []
    for m in re.finditer(r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>', x):
        x0, y0, x1, y1 = map(float, m.groups()[:4])
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        if not any(b[0] <= cx <= b[2] and b[1] <= cy <= b[3] for b in boxes):
            kept.append(html.unescape(m.group(5)))
    return words(" ".join(kept))


def recall(ref, text):
    have = set(words(norm(text)))
    return sum(1 for w in ref if w in have) / max(1, len(ref))


def med(xs):
    xs = [x for x in xs if x is not None]
    return round(statistics.median(xs), 3) if xs else None


def main():
    before_dir, after_dir = sys.argv[1], sys.argv[2]
    before = {(r["pdf"], r["page"]): r for r in json.load(open(os.path.join(before_dir, "measure.json")))}
    after = {(r["pdf"], r["page"]): r for r in json.load(open(os.path.join(after_dir, "measure.json")))}
    rows = []
    for key in sorted(after):
        if key not in before:
            continue
        pdf, page = key
        meta = json.load(open(os.path.join(after_dir, f"{pdf}-{page}.json")))
        boxes = [r["bbox"] for r in meta.get("regions", []) if r["label"] in MASK]
        ref = prose_ref(os.path.join("pdfs", pdf), page, boxes)
        b_md = open(os.path.join(before_dir, f"{pdf}-{page}.md")).read()
        a_md = open(os.path.join(after_dir, f"{pdf}-{page}.md")).read()
        row = {
            "pdf": pdf, "page": page,
            "recall": (before[key]["word_recall"], after[key]["word_recall"]),
            "cer": (before[key]["cer"], after[key]["cer"]),
            "prose": (round(recall(ref, b_md), 4), round(recall(ref, a_md), 4)),
            "secs": (before[key]["secs"], after[key]["secs"]),
            "layout_secs": after[key].get("layout_secs"),
            "regions": after[key].get("regions"),
        }
        rows.append(row)
        print(json.dumps(row))
    for name in ("recall", "cer", "prose", "secs"):
        b = [r[name][0] for r in rows]
        a = [r[name][1] for r in rows]
        print(f"{name}: before median {med(b)} · after median {med(a)} "
              f"· after better on {sum(1 for x, y in zip(b, a) if (y < x if name in ('cer', 'secs') else y > x))}"
              f" / worse on {sum(1 for x, y in zip(b, a) if (y > x if name in ('cer', 'secs') else y < x))} of {len(rows)}")
    print(f"layout model per page: median {med([r['layout_secs'] for r in rows])} s; regions per page: median {med([r['regions'] for r in rows])}")
    p1 = [r for r in rows if r["page"] == 1]
    print(f"page 1 CER: before median {med([r['cer'][0] for r in p1])} · after median {med([r['cer'][1] for r in p1])}")


main()
