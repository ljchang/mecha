#!/usr/bin/env python3
"""Add Chatterbox cloning references cut from the VCTK corpus.

    python3 scripts/voice/add-vctk-voices.py --list        # who is on offer
    python3 scripts/voice/add-vctk-voices.py               # the curated set
    python3 scripts/voice/add-vctk-voices.py p225 p234     # named speakers

The companion to `make-voices.py`, and the reason for a second script
rather than a flag: these references are recordings of **real people**,
where Kokoro's are synthesized. That difference is the whole point in
both directions. A real speaker carries prosody a preset TTS does not
have to give - which is what a clone actually copies, and why a voice
built from a synthesized reference sounds flat however high
`exaggeration` goes. And consent is what makes it legal, so it has to
be a property of the source rather than of anyone's good intentions:
VCTK is CC BY 4.0, recorded for this purpose, and redistributable with
attribution. Nothing here cuts audio off the internet, and there is
deliberately no argument that takes a URL.

Attribution, required by the licence and written beside the voices:
  CSTR VCTK Corpus (0.92), Centre for Speech Technology Research,
  University of Edinburgh. CC BY 4.0.

Additive by design: this writes new files into VOICES_DIR and never
touches an existing one. The default voice is Chatterbox's own built-in
and is not a file at all, so no run of this can change how mecha sounds
until somebody picks a new voice in the page.
"""
import argparse
import io
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import wave

DATASET = "sanchit-gandhi/vctk"
ROWS = "https://datasets-server.huggingface.co/rows"
VOICES_DIR = os.path.expanduser(os.environ.get("VOICES_DIR", "~/models/voices"))
INDEX = os.path.expanduser("~/.cache/mecha-vctk-index.json")

# ~12 seconds is what Chatterbox wants and one VCTK utterance is ~3, so a
# reference is several concatenated. Utterances 1-5 are the elicitation
# paragraph every speaker reads, which makes the references phonetically
# comparable across voices - auditioning is then a comparison of speakers
# rather than of whatever sentences they happened to draw.
TARGET_SECONDS = 12.0
MAX_UTTERANCES = 8

# The four kept after auditioning all 63 women in the corpus - chosen by
# ear, which is the only way this can be chosen: VCTK tags *region* and
# never register, so "which of these sounds posh" is not a query. p276 is
# the one speaker labelled Oxford rather than a broad region, and p362 is
# among the five oldest women in the corpus (29; the median is 23), which
# is what a lower, settled register needs and what the corpus mostly does
# not have - it is university students reading newspaper sentences.
#
# Pinned rather than left as an audition list so this script reproduces
# the installed set, and so the set is reviewable: a voice mecha speaks in
# is a decision, and one that lives only on somebody's disk is not one.
CURATED = ["p362", "p229", "p277", "p276"]


def get(url, tries=6):
    """datasets-server rate-limits hard; a 429 is a wait, not a failure."""
    delay = 2.0
    for attempt in range(tries):
        try:
            with urllib.request.urlopen(url, timeout=90) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code not in (429, 500, 502, 503) or attempt == tries - 1:
                raise
            sys.stderr.write(f"  {e.code}, waiting {delay:.0f}s\n")
            time.sleep(delay)
            delay = min(delay * 2, 60)
    raise RuntimeError("unreachable")


def rows(offset, length, columns=None):
    url = f"{ROWS}?dataset={urllib.parse.quote(DATASET)}&config=default&split=train&offset={offset}&length={length}"
    if columns:
        url += "&columns=" + ",".join(columns)
    return json.loads(get(url))


def build_index():
    """offset -> speaker, swept once and cached.

    Rows are grouped by speaker (~800 apiece), so a stride under the
    block size lands inside every block. Sequential and cached because
    the parallel version earned a 429 that outlasted the run it saved.
    """
    if os.path.exists(INDEX):
        return json.load(open(INDEX))
    meta = ["speaker_id", "gender", "accent", "region", "age"]
    total = rows(0, 1, meta)["num_rows_total"]
    seen = {}
    for off in range(0, total, 400):
        r = rows(off, 1, meta)["rows"][0]["row"]
        sid = r["speaker_id"]
        if sid not in seen:
            seen[sid] = {"offset": off, "gender": r["gender"],
                         "accent": r["accent"], "region": r["region"], "age": r["age"]}
            sys.stderr.write(f"  {sid} {r['gender']} {r['accent']} {r['region']}\n")
    os.makedirs(os.path.dirname(INDEX), exist_ok=True)
    json.dump(seen, open(INDEX, "w"), indent=1)
    return seen


# The corpus serves FLAC, and the reference has to be a wav the TTS
# server can open. ffmpeg does the decode rather than a Python audio
# dependency: this is a one-off tool, and `make-voices.py`'s rule is
# that nothing on the voice path at run time should grow a dependency
# for something run once.
RATE = 24000


def decode(flac: bytes) -> bytes:
    """FLAC -> raw s16le mono at RATE."""
    out = subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", "pipe:0",
         "-f", "s16le", "-ac", "1", "-ar", str(RATE), "pipe:1"],
        input=flac, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if out.returncode != 0 or not out.stdout:
        raise RuntimeError(f"ffmpeg failed: {out.stderr.decode()[:200]}")
    return out.stdout


def reference_wav(speaker, entry):
    """Concatenate utterances until ~12s, returned as one wav."""
    batch = rows(entry["offset"], MAX_UTTERANCES + 4)["rows"]
    chunks, total = [], 0.0
    for row in batch:
        r = row["row"]
        if r["speaker_id"] != speaker:
            continue
        pcm = decode(get(r["audio"][0]["src"]))
        chunks.append(pcm)
        total += len(pcm) / 2 / RATE
        if total >= TARGET_SECONDS or len(chunks) >= MAX_UTTERANCES:
            break
    if not chunks:
        raise RuntimeError(f"no audio for {speaker}")
    buf = io.BytesIO()
    with wave.open(buf, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        # A short gap between utterances: butt-joining two sentences
        # clones the join as a speaking habit.
        gap = b"\x00" * (int(RATE * 0.15) * 2)
        out.writeframes(gap.join(chunks))
    return buf.getvalue(), total


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("speakers", nargs="*", default=None)
    ap.add_argument("--list", action="store_true", help="print the speaker table and exit")
    ap.add_argument("--female-only", action="store_true", default=False)
    args = ap.parse_args()

    sys.stderr.write("indexing VCTK speakers (cached after the first run)\n")
    index = build_index()

    if args.list:
        for sid, e in sorted(index.items()):
            if args.female_only and e["gender"] != "F":
                continue
            print(f"{sid}  {e['gender']}  {e['accent']:14s} {e['region'] or '-':22s} age {e['age']}")
        return 0

    wanted = args.speakers or CURATED
    os.makedirs(VOICES_DIR, exist_ok=True)
    written = []
    for sid in wanted:
        if sid not in index:
            sys.stderr.write(f"unknown speaker {sid} - try --list\n")
            continue
        name = f"vctk_{sid}"
        path = os.path.join(VOICES_DIR, f"{name}.wav")
        if os.path.exists(path):
            sys.stderr.write(f"{name}: exists, left alone\n")
            continue
        data, secs = reference_wav(sid, index[sid])
        # Temp-sibling-and-rename: a half-written reference is a voice
        # the server will happily offer and fail to clone from.
        tmp = path + ".tmp"
        open(tmp, "wb").write(data)
        os.replace(tmp, path)
        e = index[sid]
        written.append((name, e, secs))
        print(f"{name}: {secs:.1f}s  {e['gender']} {e['accent']} {e['region']}")

    if written:
        with open(os.path.join(VOICES_DIR, "ATTRIBUTION.md"), "a") as f:
            if f.tell() == 0:
                f.write("# Voice reference sources\n\n"
                        "## CSTR VCTK Corpus (0.92)\n"
                        "Centre for Speech Technology Research, University of Edinburgh.\n"
                        "Licensed CC BY 4.0 <https://creativecommons.org/licenses/by/4.0/>.\n"
                        "Reference clips below are excerpts, unmodified except for\n"
                        "concatenation and silence padding.\n\n")
            for name, e, _ in written:
                f.write(f"- `{name}.wav` - VCTK speaker {name[5:]} "
                        f"({e['gender']}, {e['accent']}, {e['region'] or 'n/a'})\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
