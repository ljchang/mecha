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

# How much of a transcript must be our own words, *in our own order*, before
# it is called an echo — as a count and as a fraction, and it has to be both.
#
# The count is what protects a barge-in. An unweighted bag of words does not:
# `vocab` would be every word in a 20-second window, function words included,
# so a long reply makes almost any short sentence clear a ratio bar. "No,
# cancel it" over "…or would you rather I cancel it?" scored 0.667 against a
# 0.6 bar and was silenced — a three-word confirmation is the commonest
# legitimate barge-in there is, and because turn-start is transcription-based
# a gated transcript is not a degraded turn but no turn at all.
#
# Ordered matching (a longest common subsequence, not a substring) is what
# keeps the count honest in the other direction: real echo arrives with words
# dropped in the middle, so contiguity is too strict, while order still costs
# a coincidental match almost everything. "Actually cancel that" cannot match
# a window that says "that" before "cancel".
MIN_ECHO_MATCHED_WORDS = 4
ECHO_COVERAGE = 0.6


def normalize(text: str) -> str:
    return re.sub(r"[^a-z0-9 ]", " ", text.lower())


def _words(text: str) -> list[str]:
    return normalize(text).split()


def _matched_in_order(words: list[str], blob: list[str]) -> int:
    """How many of `words` appear in `blob`, in the same order.

    A longest common subsequence. Order is the whole point: it is what
    separates our own sentence coming back with a word misheard in the middle
    from a person reusing two of our words to disagree with us.
    """
    if not words or not blob:
        return 0
    prev = [0] * (len(blob) + 1)
    for w in words:
        cur = [0]
        for j, b in enumerate(blob):
            cur.append(prev[j] + 1 if w == b else max(cur[j], prev[j + 1]))
        prev = cur
    return prev[-1]


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
        while this segment was captured" ([`overlapped`]), and it gates the
        fuzzy arm entirely rather than merely loosening it: **with nothing
        playing there was no echo to have.** A transcript that resembles the
        last reply in a silent room is a person agreeing in the words of the
        question they were asked, which is what conversation sounds like. The
        verbatim arm still applies either way, because the timing layer can be
        wrong — a missing frame, an unrecorded segment start — and that is the
        one claim cheap enough to make unconditionally.
        """
        words = _words(transcript)
        if not words:
            return False
        blob = self.recent()
        if not blob:
            return False

        norm = " ".join(words)
        # Said verbatim, heard verbatim. The original test, unchanged.
        if len(norm) >= 8 and norm in blob:
            return True
        if not bot_was_audible:
            return False

        matched = _matched_in_order(words, blob.split())
        return matched >= MIN_ECHO_MATCHED_WORDS and matched / len(words) >= ECHO_COVERAGE


def overlapped(
    *,
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
