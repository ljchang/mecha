#!/usr/bin/env python3
# Table fidelity, before and after the layout stage — the table numbers in
# docs/DOCUMENT-EXTRACTION-DESIGN.md §8. A cheap stand-in for TEDS that needs
# no hand-made ground truth: the PDF's own text layer is the truth for a
# born-digital table.
#
# For each table the layout model finds on a page (its box, from mecha's
# --json), the text layer's lines inside that box are the table's rows. Then:
#
#   rows held   a source row is held when every one of its tokens appears in
#               ONE line of the transcript — a Markdown table row after, any
#               line of the whole-page transcript before. A dropped column, a
#               table flattened to one cell per line, or a row label swapped
#               for its neighbour's all break it.
#   tokens      the share of the rows' tokens found anywhere in the reading.
#
# Usage (after ocr-measure.py has written both readings' Markdown):
#   MECHA=target/release/mecha MECHA_HOME=/tmp/h-layout \
#     python3 scripts/table-measure.py pdfs/1706.03762.pdf:8 ... \
#     --before out-whole
#
# Nothing it reads or writes is committed. LaTeX in the transcript is reduced
# to its characters (`10^{20}` -> `1020`), as the text layer has them.
import json, os, re, subprocess, sys, html


GREEK = {"alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ", "epsilon": "ε", "theta": "θ",
         "lambda": "λ", "mu": "μ", "sigma": "σ", "tau": "τ", "phi": "φ", "omega": "ω",
         "pm": "±", "times": "×", "cdot": "·", "sim": "∼", "approx": "≈", "leq": "≤", "geq": "≥"}


def tokens(s):
    # LaTeX reduced to the characters a text layer has: delimiters vanish,
    # symbols become their glyphs, `10^{20}` becomes `10 20` (the text layer
    # sets the exponent as its own word).
    s = s.replace("\\|", "|")
    for d in ("\\(", "\\)", "\\[", "\\]"):
        s = s.replace(d, "")
    s = re.sub(r"\\([a-zA-Z]+)", lambda m: GREEK.get(m.group(1).lower(), " "), s)
    s = re.sub(r"[{}^_]", " ", s)
    s = re.sub(r"[$|*`\\]", " ", s)
    s = s.replace("−", "-").replace("–", "-").replace("’", "'").replace("‘", "'")
    out = []
    for w in s.split():
        w = w.strip(".,;:()[]\"'").lower()
        if w and any(c.isalnum() for c in w):
            out.append(w)
    return out


def text_rows(pdf, page, box):
    # A row is the words sharing a baseline — poppler's own <line>s split a
    # table's columns apart, which would make every cell a "row".
    x = subprocess.run(["pdftotext", "-bbox", "-f", str(page), "-l", str(page), pdf, "-"],
                       capture_output=True, text=True).stdout
    ws = []
    for m in re.finditer(r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>', x):
        x0, y0, x1, y1 = map(float, m.groups()[:4])
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        if box[0] <= cx <= box[2] and box[1] <= cy <= box[3]:
            ws.append((y1, x0, html.unescape(m.group(5))))
    ws.sort()
    rows, cur, base = [], [], None
    for y1, x0, w in ws:
        if base is not None and abs(y1 - base) > 2.5:
            rows.append(cur)
            cur = []
        if not cur:
            base = y1
        cur.append((x0, w))
    if cur:
        rows.append(cur)
    out = []
    carry = []
    for r in rows:
        t = tokens(" ".join(w for _, w in sorted(r)))
        # A superscript sits on its own baseline above its row (`10` then a
        # raised `20`, `Adapter` then a raised `L`): fold it into the row.
        if t and len(t) <= 2 and all(len(w) <= 2 for w in t):
            carry += t
            continue
        if t:
            out.append(t + carry)
            carry = []
    if carry and out:
        out[-1] += carry
    return out


def held(row, lines):
    return any(all(w in ln for w in row) for ln in lines)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    before = sys.argv[sys.argv.index("--before") + 1] if "--before" in sys.argv else None
    if before in args:
        args.remove(before)
    mecha = os.environ["MECHA"]
    total = {"after": [0, 0, 0, 0], "before": [0, 0, 0, 0]}
    for item in args:
        pdf, page = item.rsplit(":", 1)
        r = subprocess.run([mecha, "document", "extract", "--mode", "ocr", "--json", "--pages", page, pdf],
                           capture_output=True, text=True)
        ocr = json.loads(r.stdout)["pages"][0]["ocr"]
        assert ocr["pipeline"] == "layout", ocr.get("fallback")
        prev = None
        if before:
            path = os.path.join(before, f"{os.path.basename(pdf)}-{page}.md")
            prev = open(path).read() if os.path.exists(path) else None
        for k, reg in enumerate(x for x in ocr["regions"] if x["label"] == "table"):
            rows = text_rows(pdf, page, reg["bbox"])
            md = reg.get("markdown") or ""
            after_lines = [set(tokens(l)) for l in md.splitlines() if l.startswith("|")]
            a_rows = sum(held(rw, after_lines) for rw in rows)
            a_tok = set(tokens(md))
            n_tok = sum(len(rw) for rw in rows)
            a_found = sum(1 for rw in rows for w in rw if w in a_tok)
            line = f"{os.path.basename(pdf)} p{page} table {k + 1}: {len(rows)} rows · after: rows held {a_rows}/{len(rows)}, tokens {a_found}/{n_tok}"
            total["after"] = [t + v for t, v in zip(total["after"], [a_rows, len(rows), a_found, n_tok])]
            if prev is not None:
                b_lines = [set(tokens(l)) for l in prev.splitlines()]
                b_rows = sum(held(rw, b_lines) for rw in rows)
                b_tok = set(tokens(prev))
                b_found = sum(1 for rw in rows for w in rw if w in b_tok)
                line += f" · before: rows held {b_rows}/{len(rows)}, tokens {b_found}/{n_tok}"
                total["before"] = [t + v for t, v in zip(total["before"], [b_rows, len(rows), b_found, n_tok])]
            print(line, flush=True)
    for k, (rh, rn, tf, tn) in total.items():
        if rn:
            print(f"{k}: rows held {rh}/{rn} ({rh / rn:.2f}), tokens {tf}/{tn} ({tf / max(1, tn):.2f})")


main()
