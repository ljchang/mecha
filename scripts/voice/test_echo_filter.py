#!/usr/bin/env python3
"""Tests for the echo filter. Stdlib only, so they run anywhere:

    python3 scripts/voice/test_echo_filter.py

That is the whole reason `echo_filter.py` is a separate module. This heuristic
sits on the untrusted-content path and decides whether a person's turn happens
at all, and it had no test of any kind — importing it used to mean importing
pipecat, a GPU box and a WebRTC stack.

Each case below is a real failure or a real barge-in, named as one.
"""
import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from echo_filter import BotSpeech, overlapped  # noqa: E402


class FakeClock:
    def __init__(self):
        self.t = 1000.0

    def __call__(self):
        return self.t

    def advance(self, seconds):
        self.t += seconds


class TheTextFilter(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.bot = BotSpeech(clock=self.clock)

    def test_a_verbatim_echo_is_caught(self):
        self.bot.note("Your first meeting tomorrow is at nine.")
        self.assertTrue(self.bot.is_probable_echo("your first meeting tomorrow is at nine"))

    def test_an_echo_the_transcriber_got_slightly_wrong_is_caught(self):
        """The failure the exact-substring test could not see.

        Speech recognition of a speaker across a room is not verbatim: one
        word lands differently and `norm in spoken` is False, so the bot's own
        sentence arrived as the owner's turn and was answered.
        """
        self.bot.note("Your first meeting tomorrow is at nine.")
        self.assertFalse(
            "your first meeting tomorrow is at nine thirty"
            in " ".join(["your first meeting tomorrow is at nine"]),
            "precondition: this is not a substring match",
        )
        self.assertTrue(
            self.bot.is_probable_echo(
                "your first meeting tomorrow is at nine thirty", bot_was_audible=True
            )
        )

    def test_an_echo_spanning_two_spoken_sentences_is_caught(self):
        """TTS is handed a sentence at a time, so an echo that runs over the
        boundary is contained in neither phrase. Matching the joined window is
        what fixes it."""
        self.bot.note("That is on Thursday.")
        self.bot.note("Shall I put it in the calendar?")
        self.assertTrue(
            self.bot.is_probable_echo(
                "on thursday shall i put it in the calendar", bot_was_audible=True
            )
        )

    def test_a_barge_in_is_not_an_echo(self):
        self.bot.note("So the first option would be to move the Thursday seminar to the following week, which gives everyone more time.")
        for said in ["no stop", "wait", "that is wrong", "actually cancel that"]:
            self.assertFalse(
                self.bot.is_probable_echo(said, bot_was_audible=True),
                f"{said!r} was silenced as an echo",
            )

    def test_a_short_answer_over_the_speaker_is_not_an_echo(self):
        """**The case the unweighted word-overlap version got wrong**, and the
        one that costs the most: a three-word confirmation is the commonest
        legitimate barge-in there is, and because turn-start is
        transcription-based a gated transcript is not a degraded turn — the
        turn never happens at all.

        Both of these reuse the bot's own words to *disagree* with it, and
        both scored above the old 0.6 bag-of-words bar (0.667 and 1.0).
        """
        self.bot.note("Yes, I can do that.")
        self.bot.note("Do you want me to move it to Thursday, or would you rather I cancel it?")
        for said in ["no cancel it", "yes cancel it"]:
            self.assertFalse(
                self.bot.is_probable_echo(said, bot_was_audible=True),
                f"{said!r} was silenced as an echo",
            )

    def test_function_words_alone_do_not_carry_a_transcript_over_the_bar(self):
        """The mechanism behind the case above: a long reply puts most of
        English into the window, so any bar computed over an unordered bag of
        words falls with reply length rather than with resemblance."""
        self.bot.note(
            "I can do that, and it is the sort of thing you would want me to "
            "check on first, so let me have a look at what is there and I will "
            "tell you what I find when I am done with it."
        )
        self.assertFalse(
            self.bot.is_probable_echo("can you do it now", bot_was_audible=True)
        )

    def test_an_echo_with_a_word_dropped_in_the_middle_is_still_caught(self):
        """Why the match is ordered rather than contiguous: recognition of a
        speaker across a room drops words as readily as it mangles them, and a
        substring test cannot survive one falling out of the middle."""
        self.bot.note("Your first meeting tomorrow is at nine.")
        self.assertTrue(
            self.bot.is_probable_echo("your first meeting is at nine", bot_was_audible=True)
        )

    def test_agreeing_in_a_silent_room_is_not_an_echo(self):
        """The false positive that would matter most: a person answering a
        question using the words of the question, with nothing playing.

        It is not a near miss any more. With no speaker there was no echo to
        have, so the fuzzy arm does not run at all — the phrase used to score
        0.833 against a 0.85 bar, which is one word from silencing the plainest
        "yes" in the language.
        """
        self.bot.note("Shall I move the seminar to Thursday?")
        self.assertFalse(
            self.bot.is_probable_echo("yes move the seminar to thursday", bot_was_audible=False)
        )

    def test_the_window_closes(self):
        self.bot.note("Your first meeting tomorrow is at nine.")
        self.clock.advance(30)
        self.assertFalse(self.bot.is_probable_echo("your first meeting tomorrow is at nine"))

    def test_nothing_said_yet_is_never_an_echo(self):
        self.assertTrue(self.bot.recent() == "")
        self.assertFalse(self.bot.is_probable_echo("hello", bot_was_audible=True))
        self.assertFalse(self.bot.is_probable_echo("", bot_was_audible=True))


class TheOverlapTest(unittest.TestCase):
    def test_a_segment_while_the_bot_is_speaking_overlaps(self):
        self.assertTrue(
            overlapped(segment_started_at=99.0, bot_speaking=True, bot_audible_until=0.0)
        )

    def test_a_segment_inside_the_tail_overlaps(self):
        """The client's jitter buffer is still playing after the worker's last
        sample, so a segment starting just after the bot 'stopped' is still
        hearing it."""
        self.assertTrue(
            overlapped(
                segment_started_at=100.5, bot_speaking=False, bot_audible_until=100.0
            )
        )

    def test_a_segment_well_after_the_tail_does_not(self):
        self.assertFalse(
            overlapped(
                segment_started_at=104.0, bot_speaking=False, bot_audible_until=100.0
            )
        )

    def test_an_unknown_segment_start_is_not_assumed_to_overlap(self):
        """The conservative answer is the wrong one here: it would silence a
        barge-in on the strength of a missing timestamp."""
        self.assertFalse(
            overlapped(
                segment_started_at=None, bot_speaking=False, bot_audible_until=99.9
            )
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
