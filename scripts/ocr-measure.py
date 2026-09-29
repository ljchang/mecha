#!/usr/bin/env python3
# Whole-page OCR against each PDF's own text layer — the measurement behind
# docs/DOCUMENT-EXTRACTION-DESIGN.md §8. Nothing it reads or writes is
# committed: fetch public PDFs into a scratch directory first, e.g.
#
#   mkdir -p pdfs && for id in 1706.03762 1512.03385 1412.6980 1810.04805 \
#     1406.2661 2006.11239 1505.04597 2005.14165 1609.02907 2106.09685; do
#     curl -sL -o pdfs/$id.pdf https://arxiv.org/pdf/$id; sleep 1; done
#   mkdir -p png out
#   PORT=8085 uv run --with rapidfuzz python scripts/ocr-measure.py
#
# Per page: wall-clock seconds, tokens, character error rate and word recall
# of the transcript against `pdftotext` (reading order), and the share of the
# text layer's sentences (>= 8 words) that hold as one word run
# (grounding::holds' comparison) in `pdftotext -layout` and in the transcript.
# Pages: 1-4 and the middle page of each PDF. It talks to the server directly
# (the same request `document::OcrClient::page` sends), so it measures the
# model, not the cache.
import json, subprocess, sys, time, re, os, statistics, base64, urllib.request, math
from rapidfuzz.distance import Levenshtein
PORT = int(os.environ.get("PORT", "8085"))
MAXPX = 1003520
def pages_of(pdf):
    out = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True).stdout
    return int(re.search(r"Pages:\s+(\d+)", out).group(1))
def size_of(pdf, p):
    out = subprocess.run(["pdfinfo", "-f", str(p), "-l", str(p), pdf], capture_output=True, text=True).stdout
    m = re.search(r"Page\s+%d size:\s+([\d.]+) x ([\d.]+)" % p, out)
    return float(m.group(1)), float(m.group(2))
def dpi_for(w, h):
    return max(50, min(300, int(72 * math.sqrt(MAXPX / (w * h)))))
def text_layer(pdf, p, layout=False):
    a = ["pdftotext", "-f", str(p), "-l", str(p)] + (["-layout"] if layout else []) + [pdf, "-"]
    return subprocess.run(a, capture_output=True, text=True).stdout
def render(pdf, p, dpi, out):
    subprocess.run(["pdftoppm", "-r", str(dpi), "-f", str(p), "-l", str(p), "-singlefile", "-png", pdf, out], check=True)
    return out + ".png"
def ocr(png, prompt="OCR:"):
    img = base64.b64encode(open(png, "rb").read()).decode()
    body = {"model": "paddleocr-vl-1.6", "temperature": 0, "max_tokens": 8192, "messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:image/png;base64," + img}}, {"type": "text", "text": prompt}]}]}
    t = time.time()
    r = json.load(urllib.request.urlopen(urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/chat/completions", json.dumps(body).encode(), {"Content-Type": "application/json"}), timeout=600))
    return time.time() - t, r
def norm(s):
    s = re.sub(r"\\[()\[\]]", " ", s)            # LaTeX delimiters
    s = re.sub(r"^#+\s*", "", s, flags=re.M)      # headings
    s = re.sub(r"[|*_`]|^-{3,}$", " ", s, flags=re.M)
    return " ".join(s.split())
def words(s):
    return [w.strip(".,;:()[]{}\"'").lower() for w in s.split() if w.strip(".,;:()[]{}\"'")]
def holds(text, span):
    t, sp = words(text), words(span)
    n = len(sp)
    return n > 0 and any(t[i:i+n] == sp for i in range(len(t) - n + 1))
rows = []
for pdf in sorted(os.listdir("pdfs")):
    path = "pdfs/" + pdf
    n = pages_of(path)
    pick = sorted(set([1, 2, 3, 4, max(1, n // 2)]))
    for p in pick:
        w, h = size_of(path, p)
        dpi = dpi_for(w, h)
        png = render(path, p, dpi, f"png/m-{pdf}-{p}")
        ref = text_layer(path, p)
        lay = text_layer(path, p, layout=True)
        dt, r = ocr(png)
        out = r["choices"][0]["message"]["content"]
        a, b = norm(out), norm(ref)
        cer = Levenshtein.distance(a, b) / max(1, len(b))
        rw, ow = words(ref), set(words(out))
        recall = sum(1 for x in rw if x in ow) / max(1, len(rw))
        sents = [s for s in re.split(r"(?<=[.!?])\s+", " ".join(ref.split())) if len(s.split()) >= 8]
        held_layout = sum(holds(lay, s) for s in sents) / max(1, len(sents)) if sents else None
        held_ocr = sum(holds(out, s) for s in sents) / max(1, len(sents)) if sents else None
        row = dict(pdf=pdf, page=p, dpi=dpi, secs=round(dt, 2), prompt_tokens=r["usage"]["prompt_tokens"], out_tokens=r["usage"]["completion_tokens"], finish=r["choices"][0]["finish_reason"], ref_chars=len(b), cer=round(cer, 4), word_recall=round(recall, 4), sentences=len(sents), held_layout=held_layout, held_ocr=held_ocr)
        rows.append(row)
        open(f"out/{pdf}-{p}.md", "w").write(out)
        print(json.dumps(row), flush=True)
json.dump(rows, open("measure.json", "w"), indent=1)
