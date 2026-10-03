#!/usr/bin/env python3
"""Build cloning references out of Kokoro's preset voices, for Breeze TTS 2
(the default TTS) or Chatterbox.

    python3 scripts/voice/make-voices.py            # the curated set
    python3 scripts/voice/make-voices.py af_sky bm_fable   # named ones
    python3 scripts/voice/make-voices.py --list     # what Kokoro offers

Both engines clone from a few seconds of reference audio, so "pick a
voice" means "have a reference wav on disk". Breeze also needs the clip's
exact words, so each `<name>.wav` gets a `<name>.txt` holding the text it
was synthesised from - exact, where the adapter would otherwise transcribe
it with Parakeet.

**The default voice.** Breeze has no built-in voice: `default` is a clip
like any other, `default.wav`, and without one every sentence in the default
voice is refused. So this also writes `default.wav` and `default.txt` from
one of the references (`--default`, af_heart unless named) - but only when
there is no default voice yet, because one already there is the owner's
choice (docs/VOICE-BREEZE-DESIGN.md §4). The references
here are *synthesized by Kokoro* rather than cut from recordings of
real people, which is the whole reason this script exists: Kokoro is
Apache 2.0 and its voices are nobody's identity, so a voice can be
added, renamed or deleted without anyone's consent being the thing
that made it legal. Public-domain corpus clips (LJSpeech, VCTK) are
the fallback if a synthesized reference clones badly - it is a real
risk, since the reference has already been through a vocoder once.

**Kokoro is not part of the running stack** — it was removed on 2026-08-25
once it was clear nothing failed over to it automatically. Start a container
before running this, and stop it afterwards; nothing else needs one. This
script is on nobody's voice path at run time.
"""
import argparse
import os
import shutil
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
    # The transcript after the clip, so it is never older than it: the
    # adapter re-transcribes a sidecar older than its wav.
    write_text(path[:-4] + ".txt", REFERENCE_TEXT)
    return len(data)


def write_text(path: str, text: str) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text + "\n")
    os.replace(tmp, path)


def ensure_default(source: str) -> str | None:
    """Copy reference `source` as the default voice when there is none yet.
    Returns the path written, or None when a default voice already exists."""
    target = os.path.join(VOICES_DIR, "default.wav")
    if os.path.exists(target):
        return None
    src = os.path.join(VOICES_DIR, f"{source}.wav")
    tmp = target + ".tmp"
    shutil.copyfile(src, tmp)
    os.replace(tmp, target)
    with open(src[:-4] + ".txt", encoding="utf-8") as f:
        write_text(target[:-4] + ".txt", f.read().strip())
    return target


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("voices", nargs="*", help="Kokoro voice ids (default: the curated six)")
    ap.add_argument("--list", action="store_true", help="print Kokoro's voices and exit")
    ap.add_argument("--default", dest="default_voice", default="af_heart",
                    help="the reference that becomes the default voice when there is none ("
                         "af_heart); synthesised too if not among the voices asked for")
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
    if args.default_voice not in wanted and not os.path.exists(os.path.join(VOICES_DIR, "default.wav")):
        wanted = wanted + [args.default_voice]
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
    written = ensure_default(args.default_voice)
    if written:
        print(f"The default voice is {args.default_voice}: {written}")
    else:
        print("A default voice was already there; left as it is.")
    print("The TTS reads this directory live - GET :8887/v1/voices to confirm.")


if __name__ == "__main__":
    main()
