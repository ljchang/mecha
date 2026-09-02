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

# How long a match must be before **text alone** may call something an echo.
#
# One floor, both arms, every overlap state — and the single number is the
# point, because three rounds of splitting it by circumstance kept silencing
# turns. The circumstances do not distinguish what they were supposed to:
#
#   - `bot_speaking` at transcribe time means the owner spoke *over* a reply
#     that is still playing. On speakers that is echo; on headphones it is a
#     barge-in. Nothing in the text tells them apart.
#   - The 1.2 s tail is not a speakerphone condition either. It is set when the
#     last sample is written out, and a person hears it a jitter buffer later
#     and answers within a second — so "inside the tail" is where a *prompt
#     answer to a question* lands, which is most of them.
#
# So at four words there is no state in which "move it to Thursday" is more
# likely our echo than the plainest way to accept the offer that prompted it.
# Below this floor the filter says nothing at all, and the energy floor in
# `worker.py` is the only defence — which is the honest description, because
# the two are **ANDed, not layered**: clearing the raised RMS bar does not
# exempt a transcript from this test, it only earns it the right to be killed
# by it. Anything this filter rejects is rejected finally, so it must only
# speak when a person is unlikely to have said exactly that.
#
# The cost, stated: a *short* echo that clears the raised RMS floor becomes a
# turn, and mecha answers something odd. That is recoverable in a way the
# other direction is not — turn-start is transcription-based, so a wrong
# suppression is not a degraded turn but no turn, and the owner repeats
# themselves into a mic that keeps discarding them.
MIN_ECHO_WORDS = 8

# How many words of a transcript this long may fail to be ours, and how fast
# that budget grows.
#
# It has to grow, because recognition error is roughly per-word: a sixteen-word
# echo comes back with two words mangled about as often as an eight-word one
# comes back with one, and a flat allowance made the filter weakest exactly
# where an echo is easiest to be sure about.
#
# This is not the ratio earlier rounds rejected, and the difference is where
# each is loosest. A ratio is loosest at short lengths, which is where
# corrections live — "book the small room for Tuesday" is five of six words
# ours. A floor plus a slow-growing allowance is loosest at long ones, where
# what it forgives is a mis-heard word rather than the point of the sentence.
MAX_UNMATCHED_WORDS = 1
WORDS_PER_EXTRA_SLIP = 16

# How much of `blob` the match may be spread across, over and above the words
# it matched. An echo is a *contiguous stretch* of what we said; one skipped
# word is the same recognition slip `MAX_UNMATCHED_WORDS` allows, seen from
# the other side.
#
# Without this the one-word allowance above re-opened, at eight words and up,
# the very failure the rest of this module removed — because the window is not
# the one offer the reasoning above imagines, it is every phrase spoken in the
# last twenty seconds joined together. "Can you also add a note to that one",
# over a reply that had said "I can add that to your calendar, and I can also
# add a note to the entry if you want, so that one is easy", matches eight of
# its nine words in order. Not one phrase of it: eight words gathered from
# across twenty-five. It is a follow-up sharing the reply's topic vocabulary,
# and the longer the reply the easier it gets — the same length-dependence
# that made the bag of words wrong.
MAX_MATCH_SPREAD = 1


def normalize(text: str) -> str:
    return re.sub(r"[^a-z0-9 ]", " ", text.lower())


def _words(text: str) -> list[str]:
    return normalize(text).split()


def _slips_allowed(n_words: int) -> int:
    """How many words of a transcript this long may fail to be ours.

    One, plus one more per `WORDS_PER_EXTRA_SLIP`. Only ever asked about a
    transcript that already cleared `MIN_ECHO_WORDS`, so there is no
    short-transcript case here — that one is answered by the floor.
    """
    return MAX_UNMATCHED_WORDS + n_words // WORDS_PER_EXTRA_SLIP


def _aligned(words: list[str], blob: list[str]) -> tuple[int, int]:
    """How many of `words` appear in `blob` in the same order, and **how far
    across `blob` the match is spread**.

    A longest common subsequence, plus the span it occupies. Order is what
    separates our own sentence coming back with a word misheard in the middle
    from a person reusing two of our words to disagree with us. The span is
    what separates it from a person reusing *eight*: the window is every
    phrase spoken in the last twenty seconds joined together, so a topical
    follow-up can pick a whole sentence's worth of words out of it without
    ever repeating a phrase. An echo is a contiguous stretch of what we said;
    a follow-up is words gathered from all over it.

    Where two alignments match the same number of words, the tighter one wins
    — a match is only evidence of echo at the place it is densest.
    """
    if not words or not blob:
        return 0, 0
    # (count, first index, last index); scored on count, then on tightness.
    def score(cell):
        count, first, last = cell
        return (count, -(last - first) if count else 0)

    prev = [(0, 0, 0)] * (len(blob) + 1)
    for w in words:
        cur = [(0, 0, 0)]
        for j, b in enumerate(blob):
            if w == b:
                count, first, last = prev[j]
                hit = (count + 1, first if count else j, j)
            else:
                hit = (0, 0, 0)
            cur.append(max(hit, cur[j], prev[j + 1], key=score))
        prev = cur
    count, first, last = prev[-1]
    return count, (last - first + 1) if count else 0


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
        wrong — a missing frame, an unrecorded segment start — but it applies
        at a *longer* floor when nothing was playing, since there it is a
        fallback rather than a defence and nothing else is armed behind it.
        Without any floor it was not cheap at all: at two short words against
        a joined window it was a bag of every phrase spoken in the last twenty
        seconds, and short confirmations quoting the assistant are the
        commonest legitimate turn there is.
        """
        words = _words(transcript)
        if not words:
            return False
        blob = self.recent()
        if not blob:
            return False

        # Too short for text to be evidence of anything — see
        # `MIN_ECHO_WORDS`. This is checked before either arm and in every
        # overlap state, because the states do not tell an echo from an
        # answer and only the length does.
        if len(words) < MIN_ECHO_WORDS:
            return False

        # Said verbatim, heard verbatim. Unconditional, because the timing
        # layer can be wrong and this is the fallback for when it is.
        if " ".join(words) in blob:
            return True
        if not bot_was_audible:
            return False

        matched, spread = _aligned(words, blob.split())
        slips = _slips_allowed(len(words))
        # Nothing of ours left out of the match, and nothing of theirs left
        # over. The two guards answer different questions and neither implies
        # the other: a follow-up can leave nothing over and still be gathered
        # from across the whole window, and a correction can be perfectly
        # contiguous and still say one thing we never did. One skipped word
        # inside the run is always allowed — the same recognition slip seen
        # from the blob's side — and beyond that the two share a budget, since
        # a long echo drops words as readily as it mangles them.
        if spread > matched + max(MAX_MATCH_SPREAD, slips):
            return False
        return len(words) - matched <= slips


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
