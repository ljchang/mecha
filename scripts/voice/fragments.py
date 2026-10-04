"""Which pieces of a reply are not sentences of their own, and wait for the next.

Pipecat's splitter cuts a streaming reply into sentences at sentence marks,
and a spoken persona writes ellipses: on 2026-10-03 one Stella reply came out
of it as 84 "sentences", 42 of them a lone "." and half the rest four
characters or fewer ("Shhh..", "slow..", "in.."). Each piece went to the TTS on
its own and got its own delivery direction, so a whispered run was performed
as disconnected fragments, and a lone "." made Breeze invent speech (the
adapter's `speakable` guard, #531). A vocal-event tag alone, "(laugh)", does
the same: measured on Breeze, a lone tag came back once in three as invented
words.

`holds(text)` says whether a piece should wait and be joined to what follows:
- nothing speakable outside a tag (".", "…", "(laugh)", "(sigh) ."), or
- it trails off in an ellipsis ("Shhh..", "that's it..."),
unless it has grown to `MAX_HOLD` characters, so a long run of trailing-off
phrases still starts speaking. The worker's `JoiningAggregator` applies it to
pipecat's sentences; the end of a reply flushes whatever is held.

Pure and dependency-free, so CI tests it on a bare python3.
"""

import re

# The longest a held run grows before it is spoken anyway: about two short
# sentences, so a reply that opens with a string of ellipses is not silent
# for long.
MAX_HOLD = 200

# The vocal events Breeze performs, as the tags the voice prompt may write.
# Measured 2026-10-04 (`vctk_p297`): each added 0.6-1.4 s of non-word audio
# after the tag and was never spoken as a word. A closed set, not a shape: a
# real aside in parentheses, "(and his sister)" or "(she sighed)", is words to
# speak, and only a known tag alone made Breeze invent speech (review of #548).
EVENTS = ("laugh", "sigh", "clears throat", "cough", "gasp", "giggle", "moan")

# One of `EVENTS` in parentheses, any case.
TAG = re.compile(r"\((?:" + "|".join(re.escape(e) for e in EVENTS) + r")\)", re.IGNORECASE)

# A trailing ellipsis, with any closing quote or bracket after it.
TRAILING_ELLIPSIS = re.compile(r"(?:(?:\.\s*){2,}|…\s*)(?:[\"'”’)\]*_~]\s*)*$")


def speakable_core(text: str) -> str:
    """The text with stage tags removed: what would be spoken as words."""
    return TAG.sub(" ", text)


def holds(text: str) -> bool:
    """Whether `text` should wait for the next piece rather than be spoken."""
    text = text.strip()
    if not text or len(text) >= MAX_HOLD:
        return False
    if not any(c.isalnum() for c in speakable_core(text)):
        return True
    return TRAILING_ELLIPSIS.search(text) is not None


class Joiner:
    """The held text between pieces. `push` returns what to speak now, if
    anything; `flush` returns whatever is left at the end of a reply."""

    def __init__(self):
        self.held = ""

    def push(self, piece: str) -> str | None:
        piece = piece.strip()
        if self.held and not any(c.isalnum() for c in piece) and not TAG.search(piece):
            # Punctuation closes the held phrase where it stands: "Shhh.." and
            # "." are "Shhh...", not "Shhh.. .".
            combined = self.held + piece
        else:
            combined = f"{self.held} {piece}".strip() if self.held else piece
        if holds(combined):
            self.held = combined
            return None
        self.held = ""
        return combined or None

    def flush(self, rest: str = "") -> str | None:
        rest = rest.strip()
        if self.held and rest and not any(c.isalnum() for c in rest) and not TAG.search(rest):
            text = self.held + rest  # attached as in `push`: "Shhh.." + "." is "Shhh..."
        else:
            text = " ".join(t for t in (self.held, rest) if t)
        self.held = ""
        return text.strip() or None

    def clear(self):
        self.held = ""
