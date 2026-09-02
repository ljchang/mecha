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

# How many of a transcript's words must be our own, *in our own order*, before
# it is called an echo. A count, never a fraction — see `MAX_UNMATCHED_WORDS`
# for why every fraction that was tried silenced a correction.
#
# The count is what protects a barge-in, and an unweighted bag of words did
# not: the vocabulary of a 20-second window is every word in it, function
# words included, so a long reply makes almost any short sentence clear a
# ratio bar. "No, cancel it" over "…or would you rather I cancel it?" scored
# 0.667 against a 0.6 bar and was silenced — a three-word confirmation is the
# commonest legitimate barge-in there is, and because turn-start is
# transcription-based a gated transcript is not a degraded turn but no turn
# at all.
#
# Ordered matching (a longest common subsequence, not a substring) is what
# keeps the count honest in the other direction: real echo arrives with words
# dropped in the middle, so contiguity is too strict, while order still costs
# a coincidental match almost everything. "Actually cancel that" cannot match
# a window that says "that" before "cancel".
MIN_ECHO_MATCHED_WORDS = 4

# How many words a transcript may contain that we did not say, and the length
# at which it may contain one at all.
#
# A *ratio* was the wrong instrument, and raising it only moved the failure up
# the scale: at 0.6, "no cancel it" was silenced; at four-words-and-0.6, "can
# you move it to Friday" was; at 0.8, "book the small room for Tuesday" is —
# each of them a correction, and each sharing most of its words with the offer
# it corrects, because that is what correcting an offer sounds like. Order is
# no defence either: a counter-instruction reuses the offer's word order.
#
# So the test is not "how much of this was ours" but "**is any of it not
# ours**". An echo is our own sentence coming back; a person saying something
# is saying something we did not say, and one new word is the whole signal —
# "small", "Friday". The allowance exists only because recognition across a
# room mangles a word now and then, and it is granted only where one word is
# plausibly noise rather than content: in a long utterance. At six words a
# single insertion is the point of the sentence.
#
# What no bar over text can separate, and it is worth stating rather than
# tuning at: a person repeating our own proposal back over the speaker ("move
# it to Thursday please") is, as text, our sentence. That band belongs to the
# energy floor in `worker.py`, which is why that floor is the load-bearing
# defence and this is depth behind it.
MAX_UNMATCHED_WORDS = 1
LONG_ENOUGH_FOR_ONE_SLIP = 8

# The verbatim arm's floor when nothing was playing.
#
# That arm runs whether or not the speaker was audible, because the timing
# layer can be wrong and this is the fallback for when it is — but a fallback
# only ever needs to catch a *long* verbatim echo, and four words is a plain
# instruction. "Move it to Thursday" is a contiguous span of the offer that
# prompted it, and in a silent room nothing else is armed to let it through:
# the energy floor behind it is the ordinary `MIN_SEGMENT_RMS`, not the raised
# one, because the timing layer correctly said no speaker was playing. Same
# for "put it in the calendar", "add a note to the entry" — each of them a
# span of the sentence that offered it, and each the plainest way to accept.
#
# The surface grew this branch as well: `recent()` joins the window, so the
# substring test spans every cross-phrase boundary in twenty seconds where it
# used to run per phrase.
MIN_VERBATIM_WORDS_IN_SILENCE = 8

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

        norm = " ".join(words)
        # Said verbatim, heard verbatim — under the same word count the fuzzy
        # arm uses, and that guard is not decoration.
        #
        # This arm was a character count (>= 8), which is two short words:
        # "go ahead" is 8, "cancel it" and "delete it" are 9, and each is a
        # substring of the reply that just offered it. So the shortest, most
        # natural confirmations in the language were dropped as echo — and
        # dropped in a *silent room* too, since this arm runs whether or not
        # the speaker was playing. That is the same failure the fuzzy arm's
        # floor exists to prevent, arriving through the other door, and
        # joining the window widened the surface it arrives on: the substring
        # test now spans phrase boundaries where it used to run per phrase.
        #
        # A real verbatim echo of a spoken sentence is far longer than three
        # words, so the guard costs this arm nothing it is for — and in a
        # silent room, where this arm is a fallback rather than a defence, it
        # is far longer than four either.
        floor = MIN_ECHO_MATCHED_WORDS if bot_was_audible else MIN_VERBATIM_WORDS_IN_SILENCE
        if len(words) >= floor and norm in blob:
            return True
        if not bot_was_audible:
            return False

        matched, spread = _aligned(words, blob.split())
        if matched < MIN_ECHO_MATCHED_WORDS:
            return False
        # Nothing of ours left out of the match, and nothing of theirs left
        # over. The two guards answer different questions and neither implies
        # the other: a follow-up can leave nothing over and still be gathered
        # from across the whole window, and a correction can be perfectly
        # contiguous and still say one thing we never did.
        if spread > matched + MAX_MATCH_SPREAD:
            return False
        unmatched = len(words) - matched
        allowed = MAX_UNMATCHED_WORDS if len(words) >= LONG_ENOUGH_FOR_ONE_SLIP else 0
        return unmatched <= allowed


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
