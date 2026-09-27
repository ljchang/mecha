#!/usr/bin/env python3
"""What one image costs a vision model on the router, and how well it grounds.

    python3 scripts/vision-probe.py [--base http://127.0.0.1:8080] [--out FILE]

Draws a synthetic UI — labelled boxes at known positions, so the ground truth
is exact rather than hand-labelled — at three sizes, and for each asks the
router's *resident* model for every box's `bbox_2d` (0–1000 relative, the
Qwen-VL convention). Reports per size:

- `image_tokens`: `prompt_tokens` with the image minus the same request
  without it — what the image itself cost, measured, not derived;
- `mean_iou` and `hits` (IoU >= 0.5) against the drawn boxes.

Why it exists: llama.cpp warns that Qwen-VL needs at least 1024 image tokens
for grounding (#16842), and `start-router.sh` sets `image-min-tokens = 1024`
on the Qwen presets. Run this before and after that change: the small images
should cost more and ground better, and the 1568 px control should not move.

It names only the model already resident (from `/models`), so it never loads
one — a request naming another model on the router is a swap. It refuses when
nothing, or more than one model, is resident. Standard library plus Pillow.
"""
import argparse
import base64
import io
import json
import random
import sys
import urllib.request

from PIL import Image, ImageDraw, ImageFont

LABELS = ["Save", "Cancel", "Search", "Settings", "Profile", "Delete", "Upload", "Help"]
SIZES = [(320, 240), (800, 600), (1568, 980)]


def get(url):
    with urllib.request.urlopen(url, timeout=10) as r:
        return json.load(r)


def post(url, body, timeout=600):
    req = urllib.request.Request(
        url, data=json.dumps(body).encode(), headers={"content-type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def resident(base):
    """The one loaded model's id, or exit: naming any other would load it."""
    data = get(f"{base}/models").get("data", [])
    loaded = [m["id"] for m in data if m.get("status", {}).get("value") in ("loaded", "sleeping")]
    if len(loaded) != 1:
        sys.exit(f"vision-probe: need exactly one resident model, found {loaded or 'none'}; refusing to name one")
    return loaded[0]


def draw(w, h, seed=7):
    """A white canvas with LABELS as outlined buttons; returns (png, truth)."""
    rng = random.Random(seed)
    img = Image.new("RGB", (w, h), "white")
    d = ImageDraw.Draw(img)
    font = ImageFont.load_default(size=max(10, h // 24))
    truth = {}
    cols, rows = 4, 2
    cw, ch = w // cols, h // rows
    for i, label in enumerate(LABELS):
        c, r = i % cols, i // cols
        bw, bh = int(cw * rng.uniform(0.45, 0.8)), int(ch * rng.uniform(0.2, 0.4))
        x0 = c * cw + rng.randint(4, max(5, cw - bw - 4))
        y0 = r * ch + rng.randint(4, max(5, ch - bh - 4))
        box = (x0, y0, x0 + bw, y0 + bh)
        d.rectangle(box, outline="black", width=max(1, h // 200), fill=(230, 236, 245))
        d.text((x0 + bw / 2, y0 + bh / 2), label, fill="black", font=font, anchor="mm")
        truth[label] = [round(v) for v in (box[0] * 1000 / w, box[1] * 1000 / h, box[2] * 1000 / w, box[3] * 1000 / h)]
    buf = io.BytesIO()
    img.save(buf, "PNG")
    return buf.getvalue(), truth


PROMPT = (
    "The image shows labelled buttons: " + ", ".join(LABELS) + ". "
    "Locate each button. Answer with only a JSON list, one object per label: "
    '{"label": "<text>", "bbox_2d": [x1, y1, x2, y2]}, coordinates relative 0-1000.'
)


def ask(base, model, png=None):
    content = [{"type": "text", "text": PROMPT}]
    if png is not None:
        uri = "data:image/png;base64," + base64.b64encode(png).decode()
        content.append({"type": "image_url", "image_url": {"url": uri}})
    body = {
        "model": model,
        "messages": [{"role": "user", "content": content}],
        "temperature": 0,
        "max_tokens": 2048,
        "chat_template_kwargs": {"enable_thinking": False},
    }
    return post(f"{base}/v1/chat/completions", body)


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def parse(text):
    t = text.strip()
    if t.startswith("```"):
        t = t.split("\n", 1)[1].rsplit("```", 1)[0]
    try:
        items = json.loads(t)
    except json.JSONDecodeError:
        return {}
    out = {}
    for it in items if isinstance(items, list) else []:
        if isinstance(it, dict) and isinstance(it.get("bbox_2d"), list) and len(it["bbox_2d"]) == 4:
            out[str(it.get("label", ""))] = [float(v) for v in it["bbox_2d"]]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8080")
    ap.add_argument("--out")
    args = ap.parse_args()
    model = resident(args.base)
    text_only = ask(args.base, model)["usage"]["prompt_tokens"]
    rows = []
    for w, h in SIZES:
        png, truth = draw(w, h)
        r = ask(args.base, model, png)
        got = parse(r["choices"][0]["message"].get("content") or "")
        scores = [iou(truth[k], got[k]) if k in got else 0.0 for k in LABELS]
        rows.append(
            {
                "size": f"{w}x{h}",
                "image_tokens": r["usage"]["prompt_tokens"] - text_only,
                "mean_iou": round(sum(scores) / len(scores), 3),
                "hits": sum(s >= 0.5 for s in scores),
                "of": len(LABELS),
                "answered": len(got),
            }
        )
    report = {"model": model, "text_only_prompt_tokens": text_only, "rows": rows}
    print(json.dumps(report, indent=1))
    if args.out:
        with open(args.out, "w") as f:
            json.dump(report, f, indent=1)


if __name__ == "__main__":
    main()
