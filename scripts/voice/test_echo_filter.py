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

    def test_agreeing_in_a_silent_room_is_not_an_echo(self):
        """The false positive that would matter most: a person answering a
        question using the words of the question, with nothing playing."""
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
            overlapped(now=100.0, segment_started_at=99.0, bot_speaking=True, bot_audible_until=0.0)
        )

    def test_a_segment_inside_the_tail_overlaps(self):
        """The client's jitter buffer is still playing after the worker's last
        sample, so a segment starting just after the bot 'stopped' is still
        hearing it."""
        self.assertTrue(
            overlapped(
                now=101.0, segment_started_at=100.5, bot_speaking=False, bot_audible_until=100.0
            )
        )

    def test_a_segment_well_after_the_tail_does_not(self):
        self.assertFalse(
            overlapped(
                now=105.0, segment_started_at=104.0, bot_speaking=False, bot_audible_until=100.0
            )
        )

    def test_an_unknown_segment_start_is_not_assumed_to_overlap(self):
        """The conservative answer is the wrong one here: it would silence a
        barge-in on the strength of a missing timestamp."""
        self.assertFalse(
            overlapped(
                now=100.0, segment_started_at=None, bot_speaking=False, bot_audible_until=99.9
            )
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
