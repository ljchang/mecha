"""Telling mecha's own speaker apart from the person holding the phone.

Without headphones the microphone hears the reply. Everything downstream is
faithful — the VAD faithfully finds speech in it, and a transducer faithfully
transcribes it — so the bot's words come back as the owner's, and the reply to
them interrupts the reply that caused them. `docs/VOICE-RESEARCH.md` §7 has the
first sighting (2026-08-24) and §6 item 10 has the structural fix that is not
built yet; this is the layer that stands in until it is.

Nothing here is acoustic. It is deliberately a **pure** module — no pipecat,
no audio, no clock of its own — because the alternative is a heuristic on the
untrusted-content path with no tests, which is what this was. `worker.py` owns
the frames and the clock and asks these questions.

Three defences, in the order they are cheap:

1. **Energy** — a segment that overlapped our own speaker must clear a higher
   floor than one in a silent room. Residual echo after browser echo
   cancellation is far quieter than a voice at the mic; a person who means to
   interrupt is not.
2. **Timing** — "did the speaker overlap this segment" needs the tail as well
   as the overlap: the client's jitter buffer and the room are still playing
   for some tenths of a second after the last sample leaves the worker.
3. **Text** — the one signal that survives every acoustic path. If the words
   are the words we just said, they are ours.

The bias is stated once, here: **a dropped turn costs a repeat, a false turn
costs an interruption mid-sentence and a reply to nothing.** But not at any
price — the person who says "no, stop" over a wrong answer is exactly who this
must not silence, so every threshold below is loosened when nothing was
playing, and the text test never judges one or two words unless we literally
said them.
"""

import re
import time

# How long a spoken phrase stays a candidate for having come back. Long enough
# to cover a reply still being played out when the segment ends.
ECHO_WINDOW_SECONDS = 20.0

# What the room and the jitter buffer are still playing after the worker's last
# sample. Purely additive slack on the overlap test — too short and the tail of
# a reply reads as a fresh turn, too long and a real answer to a finished
# question is treated as suspect.
BOT_TAIL_SECONDS = 1.2

# Fraction of a transcript's words that must also be in what we just said.
# Two bars, because the same words mean different things depending on whether
# the speaker was audible: over our own voice, agreeing in our own vocabulary
# is what echo looks like; in a silent room it is what conversation looks like.
ECHO_OVERLAP_OVER_SPEAKER = 0.6
ECHO_OVERLAP_IN_SILENCE = 0.85

# Below this a transcript is judged only by exact containment. "Stop", "wait",
# "no" are the whole vocabulary of a barge-in, and word overlap cannot tell
# them from an echo — so it does not try.
MIN_WORDS_FOR_OVERLAP = 3


def normalize(text: str) -> str:
    return re.sub(r"[^a-z0-9 ]", " ", text.lower())


def _words(text: str) -> list[str]:
    return normalize(text).split()


class BotSpeech:
    """What the bot said recently, and when.

    A window rather than a log: the question is only ever "could this have
    just come back", and an unbounded history would start matching a phrase
    from four turns ago.
    """

    def __init__(self, window_seconds: float = ECHO_WINDOW_SECONDS, clock=time.monotonic):
        self._window = window_seconds
        self._clock = clock
        self._said: list[tuple[float, str]] = []

    def note(self, text: str) -> None:
        self._said.append((self._clock(), " ".join(_words(text))))
        self._forget()

    def _forget(self) -> None:
        cutoff = self._clock() - self._window
        self._said = [(t, s) for t, s in self._said if t >= cutoff]

    def recent(self) -> str:
        """Everything still in the window, as one blob.

        Joined rather than compared phrase by phrase: TTS is handed a sentence
        at a time, so an echo of a reply that ran over a sentence boundary is
        contained in *no* single phrase and matched nothing before this.
        """
        self._forget()
        return " ".join(s for _, s in self._said)

    def is_probable_echo(self, transcript: str, *, bot_was_audible: bool = False) -> bool:
        """Are these our own words coming back?

        `bot_was_audible` is the caller's answer to "was the speaker playing
        while this segment was captured" ([`overlapped`]). It only ever
        *loosens* the bars — with nothing playing there was no echo to have,
        so a transcript that merely resembles the last reply is treated as a
        person agreeing, which is the thing it almost certainly is.
        """
        words = _words(transcript)
        if not words:
            return False
        blob = self.recent()
        if not blob:
            return False

        norm = " ".join(words)
        # The original test, unchanged: said verbatim, heard verbatim.
        if len(norm) >= 8 and norm in blob:
            return True
        if len(words) < MIN_WORDS_FOR_OVERLAP:
            return False
        if not bot_was_audible and len(norm) < 8:
            return False

        vocab = set(blob.split())
        overlap = sum(1 for w in words if w in vocab) / len(words)
        bar = ECHO_OVERLAP_OVER_SPEAKER if bot_was_audible else ECHO_OVERLAP_IN_SILENCE
        return overlap >= bar


def overlapped(
    *,
    now: float,
    segment_started_at: float | None,
    bot_speaking: bool,
    bot_audible_until: float,
    tail_seconds: float = BOT_TAIL_SECONDS,
) -> bool:
    """Was our own speaker playing at any point during this segment?

    `bot_audible_until` is when the *last* reply stopped being written out;
    the tail is added here rather than by the caller so the one decision about
    how long a room keeps ringing is made in one place.

    A segment with no recorded start is treated as overlapping whenever the
    bot is speaking now and not otherwise — the honest answer to a question
    that cannot be asked, and the conservative one is the wrong default here:
    it would silence a barge-in on the strength of a missing timestamp.
    """
    if bot_speaking:
        return True
    if segment_started_at is None:
        return False
    return segment_started_at <= bot_audible_until + tail_seconds
