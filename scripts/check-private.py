#!/usr/bin/env python3
"""Refuse to commit or push a persona chat's or a voice call's words.

The owner's ruling (2026-10-04): no persona chat or voice call text belongs in
this public repository — not in tests, docs or anywhere — only on the machine
that holds the conversations. Twice that rule was broken by hand: real lines
from a persona chat became test fixtures, and a call's transcript became a
research doc. This is the check that would have caught both.

It reads the conversations where they live (`~/.mecha`), never a list of
phrases kept in the repository, which would itself be the leak:

- every persona chat (`~/.mecha/personas/<name>/sessions/*.jsonl`): every
  message's text, thinking, tool calls and tool results, and each spoken
  sentence;
- every voice session (`~/.mecha/sessions/*.jsonl` with `kind = "voice"`);
- the voice worker's journal lines that carried words (until #547 stopped
  them).

Then it looks at the lines being added — the staged diff (`--staged`, the
pre-commit hook), a commit range (`--range A..B`, the pre-push hook) or a
commit message (`--message FILE`, the commit-msg hook) — for:

1. a run of `SHINGLE` words that was said in a conversation;
2. a quoted phrase of three or more words that was said in one;
3. a real persona or voice session id;
4. the name of one of the owner's own personas.

Text already in the repository before the change is never flagged: the harness's own
sentences (the call note, the outbox read-back) are spoken in conversations
too, and they come from the code. On a machine with no `~/.mecha` (CI, a
fresh clone) there is nothing to compare against, and it passes, saying so.
Nothing it finds is printed beyond the file, the line number and which check
fired: the words stay where they are.
"""

import glob
import json
import os
import re
import subprocess
import sys

SHINGLE = 6
HOME = os.path.expanduser("~")
MECHA = os.environ.get("MECHA_HOME", f"{HOME}/.mecha")
QUOTE = re.compile(r'"([^"\n]{6,240})"|“([^”\n]{6,240})”|\'([^\'\n]{12,240})\'')
SESSION_ID = re.compile(r"\b(\d{8}T\d{6}-[0-9a-f]{8})\b")


def words(text):
    return re.findall(r"[a-z0-9]+", text.lower().replace("’", "").replace("'", ""))


def grams(w, n):
    return {" ".join(w[i:i + n]) for i in range(len(w) - n + 1)}


def strings(o):
    """Every string with a space in it: words, never a base64 image or a
    signature."""
    if isinstance(o, str):
        if " " in o:
            yield o
    elif isinstance(o, dict):
        for k, v in o.items():
            if k not in ("data", "signature"):
                yield from strings(v)
    elif isinstance(o, list):
        for v in o:
            yield from strings(v)


def spoken(o):
    """Everything a conversation holds: every message's blocks at any depth
    (text, thinking, a tool call's arguments — a picture's prompt — and a
    tool's result, which can carry what a persona remembers about the owner;
    a compaction `rewrite` carries whole messages inside it) and each spoken
    direction's sentence."""
    if isinstance(o, dict):
        if o.get("record") == "spoken_direction":
            yield o.get("sentence", "")
        if o.get("role") in ("user", "assistant", "tool") and "content" in o:
            yield from strings(o["content"])
            return
        for v in o.values():
            yield from spoken(v)
    elif isinstance(o, list):
        for v in o:
            yield from spoken(v)


def read_journal():
    """The voice worker's journal lines that carried words, or nothing."""
    try:
        return subprocess.run(
            ["journalctl", "--user", "-u", "mecha-voice-worker", "--no-pager", "-o", "cat",
             "--grep", "Generating TTS|Transcription:"],
            capture_output=True, text=True, errors="replace", timeout=60).stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""


def corpus():
    """(shingles, short phrases, session ids, persona names) from ~/.mecha."""
    said = []
    ids = set()
    names = set()
    for d in glob.glob(f"{MECHA}/personas/*/"):
        if os.path.exists(f"{d}persona.toml"):
            names.add(os.path.basename(d.rstrip("/")).lower())
    files = glob.glob(f"{MECHA}/personas/*/sessions/*.jsonl")
    for f in glob.glob(f"{MECHA}/sessions/*.jsonl"):
        try:
            with open(f, errors="replace") as fh:
                if json.loads(fh.readline()).get("kind") == "voice":
                    files.append(f)
        except (OSError, ValueError):
            continue
    for f in files:
        ids.add(os.path.basename(f)[: -len(".jsonl")])
        with open(f, errors="replace") as fh:
            for line in fh:
                try:
                    said.extend(spoken(json.loads(line)))
                except ValueError:
                    continue
    # The worker journal is this machine's; a test of the check itself sets
    # CHECK_PRIVATE_JOURNAL=0 and reads only the store it built.
    if os.environ.get("CHECK_PRIVATE_JOURNAL", "1") == "0":
        journal = ""
    else:
        journal = read_journal()
    for line in journal.splitlines():
        m = re.search(r"(?:Generating TTS|Transcription:) \[(.*)\]", line)
        if m:
            said.append(m.group(1))
    shingles, phrases = set(), set()
    for text in said:
        w = words(text)
        shingles |= grams(w, SHINGLE)
        for n in range(3, SHINGLE):
            phrases |= grams(w, n)
    return shingles, phrases, ids, names, bool(files)


def added_lines(args):
    """(path, line number, text) for every line the change adds — or, with
    `--message FILE` (the commit-msg hook), every line of a commit message,
    which is text that goes public as surely as a diff (review of #557)."""
    if args and args[0] == "--message":
        with open(args[1], errors="replace") as fh:
            for num, line in enumerate(fh, 1):
                if not line.startswith("#"):  # git's own instructions
                    yield "commit message", num, line.rstrip("\n")
        return
    if args and args[0] == "--range":
        diff = ["git", "diff", "--unified=0", "--no-color", args[1]]
    else:
        diff = ["git", "diff", "--cached", "--unified=0", "--no-color"]
    r = subprocess.run(diff, capture_output=True, text=True, errors="replace")
    if r.returncode != 0:
        # A diff that could not be read is never "nothing added": refuse
        # rather than pass unread (review of #557 — the silently-degrading
        # guard).
        print(f"check-private: `{' '.join(diff)}` failed; refusing rather than passing unread.")
        print(r.stderr.strip())
        sys.exit(2)
    out = r.stdout
    path, num = None, 0
    for line in out.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@"):
            num = int(re.search(r"\+(\d+)", line).group(1))
        elif line.startswith("+") and path:
            yield path, num, line[1:]
            num += 1


BASE = ["HEAD"]


def in_head(normalised, head_cache=[]):
    """Whether the repository already says this before the change — the
    harness's own words. `HEAD` for a commit; the range's base for a push,
    whose `HEAD` is the change itself. Lines are kept apart by a separator
    no word can match: a run of words that exists only by straddling two
    lines, or two files, is not something the repository says (review of
    #557). The added side is read per line too, so the coverage is per line:
    a private phrase rewrapped across lines is caught where six of its words
    share one line, or by the quoted-phrase check."""
    if not head_cache:
        out = subprocess.run(["git", "grep", "-I", "-h", "-e", "", BASE[0], "--"],
                             capture_output=True, text=True, errors="replace").stdout
        head_cache.append(" | ".join(" ".join(words(line)) for line in out.splitlines()))
    return f" {normalised} " in f" {head_cache[0]} "


def main(args):
    if args and args[0] == "--range":
        BASE[0] = args[1].split("..")[0]
    shingles, phrases, ids, names, found = corpus()
    if not found:
        print("check-private: no conversations on this machine to compare against; passing.")
        return 0
    findings = []
    for path, num, text in added_lines(args):
        if path.endswith(("check-private.py",)):
            continue
        w = words(text)
        hit = None
        for g in grams(w, SHINGLE) & shingles:
            if not in_head(g):
                hit = "words said in a conversation"
                break
        if not hit:
            for m in QUOTE.finditer(text):
                q = " ".join(words(next(x for x in m.groups() if x)))
                if 3 <= len(q.split()) < SHINGLE and q in phrases and not in_head(q):
                    hit = "a quoted phrase said in a conversation"
                    break
        if not hit:
            for sid in SESSION_ID.findall(text):
                if sid in ids:
                    hit = "a real persona or voice session id"
                    break
        if not hit:
            for name in names:
                if re.search(rf"\b{re.escape(name)}\b", text, re.I) and not in_head(name):
                    hit = "the name of one of the owner's personas"
                    break
        if hit:
            findings.append((path, num, hit))
    if not findings:
        return 0
    print("check-private: refusing — this change carries conversation text (owner's ruling,")
    print("2026-10-04: no persona chat or voice call words in this repository).")
    for path, num, hit in findings[:40]:
        print(f"  {path}:{num}: {hit}")
    print("Replace it with made-up text of the same shape. The words are not printed here.")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
