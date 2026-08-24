#!/usr/bin/env python3
"""Build Chatterbox cloning references out of Kokoro's preset voices.

    python3 scripts/voice/make-voices.py            # the curated set
    python3 scripts/voice/make-voices.py af_sky bm_fable   # named ones
    python3 scripts/voice/make-voices.py --list     # what Kokoro offers

Chatterbox Turbo clones from a few seconds of reference audio, so
"pick a voice" means "have a reference wav on disk". The references
here are *synthesized by Kokoro* rather than cut from recordings of
real people, which is the whole reason this script exists: Kokoro is
Apache 2.0 and its voices are nobody's identity, so a voice can be
added, renamed or deleted without anyone's consent being the thing
that made it legal. Public-domain corpus clips (LJSpeech, VCTK) are
the fallback if a synthesized reference clones badly - it is a real
risk, since the reference has already been through a vocoder once.

Both servers are already running; this only talks to them over HTTP.
Nothing here is on the voice path at run time.
"""
import argparse
import os
import sys
import urllib.error
import urllib.request

KOKORO = os.environ.get("KOKORO_URL", "http://127.0.0.1:8880")
VOICES_DIR = os.path.expanduser(os.environ.get("VOICES_DIR", "~/models/voices"))

# Six, spanning accent and register, so the picker is a real choice and
# not a wall of near-identical American women. Kokoro's own ids are kept
# verbatim as the filenames: the prefix encodes accent and gender
# (a/b = American/British, f/m), the name is traceable back to its
# source, and a mapping table to prettier names would be a second source
# of truth that drifts the first time someone adds a seventh.
CURATED = [
    "af_heart",    # American female, warm - Kokoro's highest-graded voice
    "af_bella",    # American female, brighter
    "am_michael",  # American male, even
    "am_puck",     # American male, livelier
    "bf_emma",     # British female
    "bm_george",   # British male
]

# ~12 seconds, phonetically broad, and deliberately about nothing: a
# reference clip is a sample of a *voice*, and content that means
# something invites listening to the wrong thing when auditioning.
REFERENCE_TEXT = (
    "The quick brown fox jumps over the lazy dog while the sun sets behind "
    "the hills. She sells seashells by the shore, and the ship's crew "
    "gathered around to watch. Numbers like seventeen, forty-two and three "
    "hundred come up often enough to matter. Thursday's meeting moved to "
    "half past nine, which suits almost everyone involved."
)


def kokoro_voices():
    with urllib.request.urlopen(f"{KOKORO}/v1/audio/voices", timeout=10) as r:
        import json

        return [v["id"] for v in json.load(r)["voices"]]


def synth(voice: str, path: str) -> int:
    import json

    body = json.dumps({
        "model": "kokoro",
        "voice": voice,
        "input": REFERENCE_TEXT,
        "response_format": "wav",
    }).encode()
    req = urllib.request.Request(
        f"{KOKORO}/v1/audio/speech", data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=180) as r:
        data = r.read()
    # Write via a temp sibling and rename, the store convention: a
    # half-written wav in the voices dir is a voice the server will
    # offer and then fail to clone from.
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)
    return len(data)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("voices", nargs="*", help="Kokoro voice ids (default: the curated six)")
    ap.add_argument("--list", action="store_true", help="print Kokoro's voices and exit")
    args = ap.parse_args()

    try:
        available = kokoro_voices()
    except (urllib.error.URLError, OSError) as e:
        sys.exit(f"cannot reach Kokoro at {KOKORO}: {e}\n"
                 f"start it first - it is the source of the references.")

    if args.list:
        for v in available:
            print(v)
        return

    wanted = args.voices or CURATED
    unknown = [v for v in wanted if v not in available]
    if unknown:
        sys.exit(f"Kokoro has no such voice: {', '.join(unknown)}\n"
                 f"try --list")

    os.makedirs(VOICES_DIR, exist_ok=True)
    for v in wanted:
        path = os.path.join(VOICES_DIR, f"{v}.wav")
        n = synth(v, path)
        print(f"  {v:<12} {n/1024:7.0f} KiB  {path}")
    print(f"\n{len(wanted)} reference(s) in {VOICES_DIR}.")
    print("Chatterbox reads this directory live - GET :8881/v1/voices to confirm.")


if __name__ == "__main__":
    main()
