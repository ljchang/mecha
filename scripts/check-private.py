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
- every voice session (`~/.mecha/sessions/*.jsonl` of kind voice), whole,
  and the spoken turns of any other session a call spoke into (a web chat, a
  test session, one from before `kind` existed): each turn that opens with
  the voice block, and the reply that answers it;
- every persona's memory (`memory.db`): the facts and episodes written from
  its chats, which are paraphrases rather than quotations;
- the voice worker's journal lines that carried words (until #547 stopped
  them).

Then it looks at the text being added — each run of consecutive added lines
read as one passage, since prose here is wrapped and a sentence crosses lines
— in the staged diff (`--staged`, the
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
fired: the words stay where they are. It reads text only: a recording or other
binary file shows in a diff as "Binary files differ" and is never scanned —
`.gitignore` and the CI no-logs job hold that line.

On the machine that holds the conversations, set `CHECK_PRIVATE_REQUIRE=1`
in the environment the hooks run in: a missing or moved store then refuses
instead of passing.
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
# The opening of the voice block every spoken turn carries
# (`voice::VOICE_BLOCK` / `VOICE_BLOCK_STREAMING`).
VOICE_MARK = "Voice mode: everything you write is spoken aloud"
# Since 2026-10-06 a spoken turn carries its guidance as a run note
# (`voice::VOICE_NOTE`), recorded as a `notes` line ahead of the owner's
# message, and the message holds only what was said: the note's opening is
# what marks the turn as spoken now.
# A hand copy of the opening of `voice::VOICE_NOTE` and
# `VOICE_NOTE_STREAMING` in mecha-cli/src/voice/mod.rs, whose test pins the
# literal: reword one and reword the other.
VOICE_NOTE_STEM = "(From the harness: this turn is spoken."
# The calendar reference's opening (`date_context::REFERENCE_STEM`).
REFERENCE_STEM = "Calendar reference from the harness clock:"
# The header a compaction summary opens with (`compact::SUMMARY_HEADER`).
SUMMARY_HEADER = "[Earlier turns were compacted to fit the context window. What happened in them:]"
# Harness text that can ride in a user message of a chat that is not a
# persona's (persona chats are read whole, so the persona stems
# `title::is_derived` also matches are left out): the compaction sentinels
# (`compact::SUMMARY_HEADER`, `CARRIED_HEADER`), the tool picture caption
# (`agent::TOOL_IMAGE_STEM`), the calendar reference
# (`date_context::REFERENCE_STEM`), the situation brief (`brief::BRIEF_STEM`),
# and the loop's own voices in `agent::is_harness_voice` — boredom's notice,
# the plan step's escalation and check feedback, the criterion observation,
# the two nudges and the two plan-check sentences — plus a mailbox delivery
# (`mailbox::DELIVERY_STEM`). Matched as prefixes; a stale one fails toward
# refusing, never toward passing.
DERIVED_STEMS = (SUMMARY_HEADER, "[Live state, carried past the compaction", "[picture returned by ",
                 REFERENCE_STEM, "Situation brief from the harness",
                 "Nothing is being learned here:", "A second opinion on your plan:",
                 "Harness check feedback: ", "Harness observations of owner-bound task criteria.",
                 "You have used your entire tool budget", "Your previous turn ended without producing anything",
                 "A declared plan check was not run", "The declared plan check did not establish completion",
                 VOICE_NOTE_STEM)
DELIVERY_STEM = "another mecha agent on this machine, not the user"
# The opening both scene notes share (`scene::SCENE_STEM` and
# `UNTRUSTED_SCENE_STEM` in mecha-core/src/scene.rs; IMAGE-SCENE-DESIGN.md
# §5.5). Only the note's own sentence, up to its first ")", is the harness's;
# the place and the clothes after it come from the chat, and are read.
SCENE_NOTE_STEM = "(Where things stand in your pictures now"
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
    a compaction `rewrite` carries whole messages inside it), each spoken
    direction's sentence, and a run's notes."""
    if isinstance(o, dict):
        if o.get("record") == "spoken_direction":
            yield o.get("sentence", "")
        # A session's title is model-written from the owner's first words.
        if o.get("record") in ("meta", "title") and isinstance(o.get("title"), str):
            yield o["title"]
        # A run's notes (`Record::Notes`): what the harness told the model for
        # one run, kept out of the messages since #572. They carry what used to
        # ride the owner's message, among it the persona's `## Core`
        # (`safety::reanchor_text`), which no other source here covers.
        if o.get("record") == "notes":
            # Less the harness's own reading and guidance — the calendar
            # reference and a spoken turn's voice note, by their prefixes
            # alone: `derived()` also matches a delivery phrase anywhere in a
            # text, which would let a memory note quoting it go unread.
            for n in o.get("notes") or []:
                if not isinstance(n, str) or n.startswith((VOICE_NOTE_STEM, REFERENCE_STEM)):
                    continue
                # A scene note: its opening sentence is mecha's, so editing
                # that wording is never refused as chat text (#567's lesson);
                # what follows it is read.
                if n.startswith(SCENE_NOTE_STEM) and ")" in n:
                    n = n[n.index(")") + 1:]
                yield n
            return
        if o.get("role") in ("user", "assistant", "tool") and "content" in o:
            yield from strings(o["content"])
            return
        for v in o.values():
            yield from spoken(v)
    elif isinstance(o, list):
        for v in o:
            yield from spoken(v)


def read_journal():
    """The voice worker's journal lines that carried words, or nothing. Words
    from before #547 exist nowhere else, so a journal that cannot be read is
    refused rather than taken as empty (review of #557); a machine with no
    journalctl at all (CI, another OS) has no such journal and reads empty."""
    cmd = ["journalctl", "--user", "-u", "mecha-voice-worker", "--no-pager", "-o", "cat",
           "--grep", "Generating TTS|Transcription:"]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=60)
    except FileNotFoundError:
        return ""
    except subprocess.TimeoutExpired:
        print("check-private: the voice journal did not answer within 60 s; refusing rather than passing unread.")
        sys.exit(2)
    # journalctl exits 1 with nothing on stderr when no line matches --grep.
    if r.returncode != 0 and (r.returncode != 1 or r.stderr.strip()):
        print("check-private: the voice journal could not be read; refusing rather than passing unread.")
        print(r.stderr.strip())
        sys.exit(2)
    return r.stdout


def read_memory(db):
    """Every recallable fact and episode a persona's memory holds
    (`personas/<name>/memory.db`): model-written paraphrases of its chats,
    which need not appear verbatim in any transcript (review of #557).
    `recall_fts` indexes all of them; an older store without it is read
    column by column. A store that exists and cannot be read is refused."""
    import sqlite3
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        tables = [r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type = 'table'")]
        if "recall_fts" in tables:
            return [r[0] for r in con.execute("SELECT text FROM recall_fts") if isinstance(r[0], str)]
        out = []
        for t in tables:
            if t.startswith(("sqlite_", "recall_fts", "vectors")):
                continue
            for row in con.execute(f'SELECT * FROM "{t}"'):
                out.extend(v for v in row if isinstance(v, str) and " " in v)
        return out
    except sqlite3.Error as e:
        print(f"check-private: the persona memory {db} could not be read ({e}); refusing rather than passing unread.")
        sys.exit(2)


def without_voice_block(text):
    """An owner turn as said: a spoken turn recorded before 2026-10-06 is
    the voice block, a blank line, then the words, and the block is the
    harness's own text — read as said, it made every later edit near the
    block's wording look like conversation."""
    if text.lstrip().startswith(VOICE_MARK):
        return text.split("\n\n", 1)[1] if "\n\n" in text else ""
    return text


def derived(text):
    """Harness text that rides in a user message (`title::is_derived` and the
    folds it names): a compaction's summary or carried state, a tool's
    picture caption, a mailbox delivery, the calendar reference, the
    situation brief. Never the owner's words."""
    text = text.strip()
    return text.startswith(DERIVED_STEMS) or DELIVERY_STEM in text


def block_key(b):
    """A block as a replay compares it: what it says and which call it
    belongs to, never a picture's bytes or a tool result's body."""
    if not isinstance(b, dict):
        return str(b)
    return (b.get("type"), b.get("text"), b.get("id"), b.get("tool_use_id"))


def message_key(m):
    """A message as a replay compares it, less the harness text a door or a
    compaction folds into it — so the head `compact::rebuild` gave a summary
    and carried state is still the head the owner sent."""
    return (m.get("role"), tuple(block_key(b) for b in m.get("content") or []
                                 if not (isinstance(b, dict) and b.get("type") == "text"
                                         and derived(b.get("text", "")))))


def folded(before, after):
    """The text a door folded into the tail, or None: `after` is `before`
    with exactly one text block (and any pictures sent with it) appended to
    its last message, a user message, and nothing else changed. That is the
    shape `serve::chat::begin_turn` records as a rewrite when a turn arrives
    while the conversation already ends on a user message — a barge-in that
    cancelled a tool turn, or a message after a failed run. A compaction
    rewrites the head and never has this shape."""
    if not before or len(before) != len(after):
        return None
    old, new = before[-1], after[-1]
    if old.get("role") != "user" or new.get("role") != "user":
        return None
    if any(a.get("role") != b.get("role") or list(map(block_key, a.get("content") or [])) != list(map(block_key, b.get("content") or []))
           for a, b in zip(before[:-1], after[:-1])):
        return None
    was, now = old.get("content") or [], new.get("content") or []
    if len(now) <= len(was) or list(map(block_key, now[:len(was)])) != list(map(block_key, was)):
        return None
    head, *rest = now[len(was):]
    if not isinstance(head, dict) or head.get("type") != "text":
        return None
    if any(isinstance(b, dict) and b.get("type") == "text" for b in rest):
        return None
    return head.get("text", "")


def user_texts(records):
    """Every text block of every owner-side message, recorded or rewritten."""
    for r in records:
        msgs = [r] if r.get("record") == "message" else r.get("messages") or [] if r.get("record") == "rewrite" else []
        for m in msgs:
            if isinstance(m, dict) and m.get("role") == "user":
                for b in m.get("content") or []:
                    if isinstance(b, dict) and b.get("type") == "text":
                        yield b.get("text", "")


def spoken_turns(records):
    """What was said aloud in a chat a call spoke into (review of #559),
    replayed in order the way `Session::read` does:

    - every spoken direction's sentence — a direction is only ever written
      for a sentence that was spoken;
    - each spoken turn — one that carries the voice block (recorded before
      2026-10-06, the first turn of a stretch only) or follows a voice-note
      `notes` line (since): the owner's words, the assistant's replies and the arguments
      it passed to tools, through its tool turns, a turn folded into its
      tail (a barge-in) and any of it recorded only inside a compaction's
      rewrite — but
      not what a tool returned, which is a file or a page, not speech, nor
      harness text folded beside it;
    - each reply containing a directed sentence, and the owner's turn it
      answers — which is how a later turn of a call is found, since only the
      first turn of a spoken stretch carries the block;
    - every rename recorded after a turn read as spoken, and the header's
      title when the chat opened spoken.

    What stays out of reach, and is the trade for not reading the owner's
    typing as speech:

    - a later turn of a call whose reply was not directed — the owner's
      words in it and the reply both — and, even when it was directed,
      what was said into its tool turns. A direction is asked for only when the
      worker's TTS honours `instructions` (`voice::direct`), so on any other
      engine no direction is written and *every* turn of a call after its
      first is out of reach. The worker journal held those until #547
      stopped it carrying words; nothing does now;
    - a compaction summary, which paraphrases typed and spoken turns alike;
    - a Listen tap's reply, which was typed and only read aloud;
    - a sentence the owner typed into a spoken stretch's tool turn reads as
      spoken (over-inclusion, the safe direction)."""
    def text_of(content):
        return " ".join(b.get("text", "") for b in content or []
                        if isinstance(b, dict) and b.get("type") == "text")

    directed = set()
    for r in records:
        if r.get("record") == "spoken_direction" and r.get("sentence"):
            # A Listen tap is recorded as a direction too (`serve::listen`,
            # under `voice_direction::LISTEN_TURN`), on any reply of any
            # chat: a typed reply read aloud, not a call — and its sentence
            # is the whole reply. It binds nothing and is not speech here
            # (review of #559, pass 6); a persona's is read whole anyway.
            if str(r.get("turn", "")).startswith("listen:"):
                continue
            yield r["sentence"]
            sentence = " ".join(words(r["sentence"]))
            # Only a sentence long enough to tell replies apart marks one as
            # spoken: "Sure." is in half of them (review of #559, pass 3).
            if len(sentence.split()) >= 3:
                directed.add(sentence)
    # `heard`: has a turn been read as spoken yet — a title recorded after
    # one can have been written from it. `opened`: was the first owner turn
    # spoken — the header's title is written before any turn.
    state = {"in_stretch": False, "last_user": None, "heard": False, "opened": None,
             "spoken_next": False}

    def turn(m):
        """One message entering the conversation, as said or not."""
        text = text_of(m.get("content"))
        if m.get("role") == "user":
            # A tool-results turn goes on with the stretch: steering and a
            # queued sentence are folded into it beside the results, since
            # nothing can sit between a tool_use and its result
            # (`agent::is_plain_user_text`, the same distinction). Its folded
            # words were said; what the tool returned was not (a file or a
            # page), so only the text blocks count, less harness text.
            results = any(isinstance(b, dict) and b.get("type") == "tool_result"
                          for b in m.get("content") or [])
            if not results:
                # Any turn the owner sends decides the stretch, a picture
                # with no words included (review of #559, pass 4).
                state["in_stretch"] = VOICE_MARK in text or state["spoken_next"]
                state["spoken_next"] = False
                state["heard"] = state["heard"] or state["in_stretch"]
                if state["opened"] is None:
                    state["opened"] = state["in_stretch"]
                # What the owner said, less what a door or a compaction
                # folded beside it (review of #559, pass 6).
                state["last_user"] = " ".join(
                    without_voice_block(b.get("text", "")) for b in m.get("content") or []
                    if isinstance(b, dict) and b.get("type") == "text" and not derived(b.get("text", "")))
            if state["in_stretch"]:
                for b in m.get("content") or []:
                    if isinstance(b, dict) and b.get("type") == "text" and not derived(b.get("text", "")):
                        yield without_voice_block(b.get("text", ""))
        elif m.get("role") == "assistant":
            said = " ".join(words(text))
            if state["in_stretch"]:
                yield from spoken(m)
            elif said and any(f" {d} " in f" {said} " for d in directed):
                # A directed reply: it was spoken, and so was what it answers.
                state["heard"] = True
                yield from spoken(m)
                if state["last_user"] is not None:
                    yield state["last_user"]

    convo = []
    seen = set()
    header = []
    for r in records:
        kind = r.get("record")
        if kind == "meta" and isinstance(r.get("title"), str):
            header.append(r["title"])
        elif kind == "title" and isinstance(r.get("title"), str):
            # `title::due` renames at owner turns 1, 3 and 8 from the owner's
            # turns oldest-first, a spoken one included — so a rename after a
            # spoken turn can be written from it, whichever turn opened the
            # chat. One before any is typed words only, and a title runs to
            # 48 characters, enough for a shingle (review of #559, pass 7).
            if state["heard"]:
                yield r["title"]
        elif kind == "notes":
            # A spoken turn's voice note, recorded ahead of the owner's
            # message it guides: that message was said (2026-10-06).
            if any(isinstance(n, str) and n.startswith(VOICE_NOTE_STEM) for n in r.get("notes") or []):
                state["spoken_next"] = True
        elif kind == "extend":
            # Harness text folded onto the last message (`Record::Extend`):
            # kept for the replay, never read as said.
            index = r.get("index")
            if isinstance(index, int) and index + 1 == len(convo) and isinstance(r.get("blocks"), list):
                convo[-1] = dict(convo[-1], content=list(convo[-1].get("content") or []) + r["blocks"])
                seen.add(message_key(convo[-1]))
        elif kind == "rewrite":
            after = [m for m in r.get("messages") or [] if isinstance(m, dict)]
            text = folded(convo, after)
            if text is not None:
                # A fold goes on with the turn it joins, as steering does;
                # one that carries the voice block opens a stretch (review of
                # #559, pass 4: a compaction's new text is not a barge-in).
                if text.strip() and not derived(text):
                    state["in_stretch"] = state["in_stretch"] or VOICE_MARK in text or state["spoken_next"]
                    state["spoken_next"] = False
                    state["heard"] = state["heard"] or state["in_stretch"]
                    state["last_user"] = without_voice_block(text)
                    if state["in_stretch"]:
                        yield without_voice_block(text)
            else:
                # A compaction. `Session::record_run` writes a run that
                # compacted itself as the compacted state *plus* every turn
                # the loop produced after it, in this one record — those
                # turns are nowhere else (review of #559, pass 5). The head
                # it rebuilt (summary, carried state) is harness text, and
                # the turns it kept were read when they were recorded.
                for m in after:
                    k = message_key(m)
                    if k[1] and k not in seen:
                        yield from turn(m)
            convo = after
            seen.update(message_key(m) for m in after)
        elif kind == "message":
            convo.append(r)
            seen.add(message_key(r))
            yield from turn(r)
    if state["opened"]:
        yield from header


def corpus():
    """(shingles, short phrases, session ids, persona names) from ~/.mecha."""
    said = []
    ids = set()
    names = set()
    for d in glob.glob(f"{MECHA}/personas/*/"):
        toml = f"{d}persona.toml"
        if os.path.exists(toml):
            names.add(os.path.basename(d.rstrip("/")).lower())
            # How the persona is addressed and drawn, which is what a fixture
            # or a doc would spell (review of #557): `display` and its words,
            # and `character`.
            try:
                text = open(toml, errors="replace").read()
            except OSError:
                text = ""
            for key in ("display", "character"):
                m = re.search(rf'^\s*{key}\s*=\s*"([^"]*)"', text, re.M)
                if m and m.group(1).strip():
                    value = m.group(1).strip().lower()
                    names.add(value)
                    names.update(w for w in words(value) if len(w) >= 4)
    # Read whole: every persona chat, and every session that is a call.
    whole = glob.glob(f"{MECHA}/personas/*/sessions/*.jsonl")
    # Read for their spoken turns only: any other session a call spoke into
    # (a web chat, a test session, one from before `kind` existed). The rest
    # of such a chat is the owner typing — often mecha's own development —
    # and taking it whole made ordinary repository prose ("cargo test -p
    # mecha-core") read as conversation (review of #557, follow-up).
    partly = []
    for f in glob.glob(f"{MECHA}/sessions/*.jsonl"):
        try:
            with open(f, errors="replace") as fh:
                kind = json.loads(fh.readline()).get("kind")
                if kind == "voice":
                    whole.append(f)
                else:
                    rest = fh.read()
                    if VOICE_MARK in rest or VOICE_NOTE_STEM in rest:
                        partly.append(f)
        except (OSError, ValueError) as e:
            print(f"check-private: the session {f} could not be read ({e}); refusing rather than passing unread.")
            sys.exit(2)
    whole_set = set(whole)
    for f in whole + partly:
        ids.add(os.path.basename(f)[: -len(".jsonl")])
        with open(f, errors="replace") as fh:
            # Streamed when read whole: a transcript holds pictures, and only
            # the replay needs the records at once (review of #559, pass 6).
            records = []
            for line in fh:
                try:
                    r = json.loads(line)
                except ValueError:
                    continue
                if f in whole_set:
                    said.extend(spoken(r))
                elif isinstance(r, dict):
                    records.append(r)
        if f in whole_set:
            continue
        # The block in the system prompt and in no owner turn: a standalone
        # `mecha voice-serve`, whose kind reads "test" rather than "voice"
        # under `MECHA_SESSION_KIND=test`. Every turn of it was spoken, so it
        # is read whole rather than found empty (review of #559, pass 7) —
        # but only on that shape: the mark in a tool result or a reply is a
        # session that read the voice code or this guard (pass 8).
        voiced_prompt = any(r.get("record") == "config" and VOICE_MARK in str(r.get("system_prompt") or "")
                            for r in records)
        if voiced_prompt and not any(VOICE_MARK in text for text in user_texts(records)):
            for r in records:
                said.extend(spoken(r))
        else:
            said.extend(spoken_turns(records))
    for db in glob.glob(f"{MECHA}/personas/*/memory.db") + glob.glob(f"{MECHA}/personas/shared.db"):
        said.extend(read_memory(db))
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
    # Persona folders with no chats yet still have names to refuse.
    return shingles, phrases, ids, names, bool(whole or partly or names)


def run_git(cmd):
    """A git command's output; one that fails is refused, never read as
    "nothing added" (review of #557 — the silently-degrading guard)."""
    r = subprocess.run(cmd, capture_output=True, text=True, errors="replace")
    if r.returncode != 0:
        print(f"check-private: `{' '.join(cmd)}` failed; refusing rather than passing unread.")
        print(r.stderr.strip())
        sys.exit(2)
    return r.stdout


def diff_lines(out, where=""):
    """(path, line number, text) for every line a unified diff adds. Header
    lines are recognised only outside a hunk — inside one, an added line
    whose own text starts "++ " is content, not a header (review of #557). A
    `+++` header it cannot read is refused: only `/dev/null` means nothing
    added."""
    path, num, new_left, old_left = None, 0, 0, 0
    for line in out.splitlines():
        if new_left or old_left:
            if line.startswith("+"):
                if path:
                    yield path, num, line[1:]
                num += 1
                new_left -= 1
            elif line.startswith("-"):
                old_left -= 1
            elif line.startswith("\\"):
                pass  # "\ No newline at end of file"
            else:
                num += 1
                new_left -= 1
                old_left -= 1
            continue
        if line.startswith("+++ "):
            target = line[4:]
            if target.startswith("b/"):
                path = target[2:]
            elif target == "/dev/null":
                path = None
            else:
                print(f"check-private: cannot read the diff header {target!r}{where}; refusing.")
                sys.exit(2)
        elif line.startswith("@@"):
            m = re.match(r"@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", line)
            if not m:
                print(f"check-private: cannot read the hunk header {line!r}{where}; refusing.")
                sys.exit(2)
            old_left = int(m.group(1)) if m.group(1) is not None else 1
            num = int(m.group(2))
            new_left = int(m.group(3)) if m.group(3) is not None else 1


# Prefixes and helpers pinned, so no one's git config (diff.noprefix,
# diff.mnemonicPrefix, an external diff driver) changes what is parsed
# (review of #557).
DIFF = ["git", "-c", "core.quotepath=false", "diff", "--unified=0", "--no-color",
        "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/"]


def added_lines(args):
    """(path, line number, text) for every line the change adds: the staged
    diff; with `--message FILE` (the commit-msg hook), every line of a
    commit message, which goes public as surely as a diff; with `--range
    A..B` (the pre-push hook), every commit's own diff *and message*, one by
    one — an endpoint diff would never read text added and then rewritten
    inside the range, which is exactly a `--no-verify` WIP branch (review of
    #557)."""
    if args and args[0] == "--message":
        with open(args[1], errors="replace") as fh:
            for num, line in enumerate(fh, 1):
                if not line.startswith("#"):  # git's own instructions
                    yield "commit message", num, line.rstrip("\n")
        return
    if args and args[0] == "--range":
        commits = run_git(["git", "rev-list", "--reverse", args[1]]).split()
        for c in commits:
            short = c[:8]
            parents = run_git(["git", "rev-list", "--parents", "-n", "1", c]).split()[1:]
            base = parents[0] if parents else "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            # Each commit's lines are their own passages: a run that exists
            # only by joining two commits' lines is not what the push adds.
            for path, num, text in diff_lines(run_git(DIFF + [base, c]), f" in {short}"):
                yield f"{path} (in {short})", num, text
            message = run_git(["git", "log", "-n", "1", "--format=%B", c])
            for num, line in enumerate(message.splitlines(), 1):
                yield f"commit message of {short}", num, line
        return
    yield from diff_lines(run_git(DIFF + ["--cached"]))


BASE = ["HEAD"]


def in_head(normalised, head_cache=[]):
    """Whether the repository already says this before the change — the
    harness's own words. `HEAD` for a commit; the range's base for a push,
    whose `HEAD` is the change itself. Each file is read as one run of words
    — prose here is wrapped at ~80 columns, and a wrapped harness sentence is
    still the repository's — but files are kept apart by a separator no word
    can match: a run that only exists by straddling two files is not
    something the repository says (review of #557)."""
    if not head_cache:
        # Per file: `git grep -h` gives no file boundaries, so ask with them.
        listing = subprocess.run(["git", "grep", "-I", "-z", "-e", "", BASE[0], "--"],
                                 capture_output=True, text=True, errors="replace").stdout
        files = {}
        for rec in listing.split("\n"):
            # `<rev>:<path>\0<line>` with -z: path and text split on the NUL.
            if "\0" not in rec:
                continue
            path, text = rec.split("\0", 1)
            files.setdefault(path, []).append(text)
        head_cache.append(" | ".join(" ".join(words(" ".join(lines))) for lines in files.values()))
    return f" {normalised} " in f" {head_cache[0]} "


def main(args):
    if args and args[0] == "--range":
        BASE[0] = args[1].split("..")[0]
    shingles, phrases, ids, names, found = corpus()
    if not found:
        # On the machine that holds the conversations, set
        # CHECK_PRIVATE_REQUIRE=1: a mistyped MECHA_HOME or a hook run as
        # another user then refuses instead of passing (review of #557), as
        # MECHA_TEST_REQUIRE_BACKENDS does for skipped tests.
        if os.environ.get("CHECK_PRIVATE_REQUIRE") == "1":
            print(f"check-private: no conversations under {MECHA}, and CHECK_PRIVATE_REQUIRE=1; refusing.")
            return 2
        print("check-private: no conversations on this machine to compare against; passing.")
        return 0
    findings = []
    # The guard's own file, under whatever label the mode gives its path.
    lines = [(p, n, t) for p, n, t in added_lines(args)
             if not p.split(" (in ")[0].endswith("check-private.py")]
    # Passages: runs of consecutive added lines in one file, read as one text,
    # so a sentence wrapped across lines — which prose here always is — is
    # still a run of words and a quote (review of #557). Ids and names are
    # line-local and checked per line.
    passages = []
    for p, n, t in lines:
        if passages and passages[-1][0] == p and passages[-1][2] == n - 1:
            passages[-1][2] = n
            passages[-1][3].append(t)
        else:
            passages.append([p, n, n, [t]])
    for p, first, last, texts in passages:
        text = " ".join(texts)
        where = f"{first}" if first == last else f"{first}-{last}"
        hit = None
        for g in grams(words(text), SHINGLE) & shingles:
            if not in_head(g):
                hit = "words said in a conversation"
                break
        if not hit:
            for m in QUOTE.finditer(text):
                q = " ".join(words(next(x for x in m.groups() if x)))
                if 3 <= len(q.split()) < SHINGLE and q in phrases and not in_head(q):
                    hit = "a quoted phrase said in a conversation"
                    break
        if hit:
            findings.append((p, where, hit))
    for p, n, t in lines:
        hit = None
        for sid in SESSION_ID.findall(t):
            if sid in ids:
                hit = "a real persona or voice session id"
                break
        if not hit:
            for name in names:
                if re.search(rf"\b{re.escape(name)}\b", t, re.I) and not in_head(" ".join(words(name))):
                    hit = "the name of one of the owner's personas"
                    break
        if hit:
            findings.append((p, n, hit))
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
