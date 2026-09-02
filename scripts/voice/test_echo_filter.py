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
        self.bot.note("Your first meeting tomorrow is at nine with the finance team.")
        self.assertTrue(
            self.bot.is_probable_echo(
                "your first meeting tomorrow is at nine with the finance team",
                bot_was_audible=True,
            )
        )

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
        # Eight words, one of them ours-with-a-slip — see
        # `test_one_slip_is_forgiven_only_where_it_is_plausibly_noise` for why
        # the same shape at six words is left alone.
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

    def test_a_correction_that_reuses_the_offer_is_not_an_echo(self):
        """**The band a count floor alone does not cover.**

        The four-word floor this file used to carry stopped "no cancel it"
        and not the same shape two words further up. "Can you move it to
        Friday" over "I can move it to Thursday" shares four words *in order*
        — and order is no help here, because a counter-instruction naturally
        reuses the word order of the offer it is correcting. It cleared a
        count of four and a 0.6 ratio, and it is not an echo, it is the
        correction. The floor is `MIN_ECHO_WORDS` now, which rules this out by
        length as well; the case is kept because the ordering argument is what
        would break first if the floor ever came down.
        """
        self.bot.note("I can move it to Thursday if you want me to.")
        for said in [
            "can you move it to friday",
            "actually can you move it to friday instead",
            "no not thursday can you do it on friday",
        ]:
            self.assertFalse(
                self.bot.is_probable_echo(said, bot_was_audible=True),
                f"{said!r} was silenced as an echo",
            )

    def test_below_the_floor_the_filter_says_nothing_in_any_state(self):
        """**The accept-the-offer band, and it must survive every state.**

        Each of these is a contiguous span of the sentence that offered it and
        the plainest way to say yes to it. The states do not separate them
        from an echo and only the length does — `bot_speaking` is a barge-in on
        headphones and an echo on speakers, and the 1.2 s tail is where a
        prompt answer to a question lands, not a speakerphone condition.

        Asserted at `bot_was_audible=True` as well, because the earlier split
        left that side unguarded: there is nothing behind this filter to catch
        the mistake. The energy floor runs *before* it and returns, so clearing
        the raised RMS bar does not exempt a transcript from this test — it
        only earns it the right to be killed by it.
        """
        self.bot.note(
            "I can move it to Thursday, or put it in the calendar, or add a "
            "note to the entry."
        )
        for said in ["move it to thursday", "put it in the calendar", "add a note to the entry"]:
            for audible in (False, True):
                self.assertFalse(
                    self.bot.is_probable_echo(said, bot_was_audible=audible),
                    f"{said!r} was silenced with bot_was_audible={audible}",
                )

    def test_one_new_word_above_the_floor_is_forgiven_as_a_slip(self):
        """And the other half: above the floor, a single word that is not ours
        is the recognition slip the allowance exists for, not a correction."""
        self.bot.note("Your first meeting tomorrow is at nine with the finance team.")
        self.assertTrue(
            self.bot.is_probable_echo(
                "your first meeting today is at nine with the finance team",
                bot_was_audible=True,
            )
        )

    def test_a_long_echo_with_two_slips_is_still_caught(self):
        """**The recall half**, and the case the branch opened with.

        Recognition error is roughly per-word, so a sixteen-word echo arrives
        with two words mangled about as often as an eight-word one arrives
        with one. A flat allowance of one made the arm weakest exactly where
        an echo is easiest to be certain about: fourteen of our sixteen words,
        in order, in one tight span, and it was the owner's turn.
        """
        self.bot.note(
            "Your first meeting tomorrow is at nine with the finance team in "
            "the small conference room."
        )
        self.assertTrue(
            self.bot.is_probable_echo(
                "your first meeting tomorrow is at nine with the finance team "
                "in a small conference groom",
                bot_was_audible=True,
            )
        )

    def test_the_allowance_grows_but_the_floor_does_not_move(self):
        """The growth is not the ratio this module rejected, and the
        difference is *where each is loosest*.

        A ratio is loosest at short lengths, which is exactly where
        corrections live. This is loosest at long lengths, where what it
        forgives is a mis-heard word rather than the point of the sentence.
        The same single substitution, below the floor and above it, is the
        cleanest way to say that.
        """
        short = BotSpeech(clock=FakeClock())
        short.note("Your first meeting tomorrow is at nine.")
        self.assertFalse(
            short.is_probable_echo("your first meeting today is at nine", bot_was_audible=True),
            "seven words with one word changed is a correction, not an echo",
        )

        long_ = BotSpeech(clock=FakeClock())
        long_.note("Your first meeting tomorrow is at nine with the finance team.")
        self.assertTrue(
            long_.is_probable_echo(
                "your first meeting today is at nine with the finance team",
                bot_was_audible=True,
            ),
            "ten words with one word changed is our sentence, mis-heard",
        )

    def test_a_longer_follow_up_is_still_not_an_echo(self):
        """The growing budget must not re-open the band the spread guard
        closed: a follow-up gathered from across a long reply stays a
        follow-up however much slack the length buys it."""
        self.bot.note(
            "I can add that to your calendar, and I can also add a note to the "
            "entry if you want, so that one is easy, and I can do the same for "
            "the others if it would help you keep track of them all."
        )
        self.assertFalse(
            self.bot.is_probable_echo(
                "can you also add a note to that one and to the others",
                bot_was_audible=True,
            )
        )

    def test_a_long_follow_up_against_a_long_reply_is_not_an_echo(self):
        """**The band the one-word allowance re-opened**, and the reason the
        match has to be *tight* as well as complete.

        The window is not one offer — it is every phrase spoken in the last
        twenty seconds joined together. A follow-up on the same topic can pick
        a whole sentence's worth of words out of twenty-five without repeating
        any phrase of it: this one matches eight of its nine words in order,
        leaving one over, which the allowance forgives at this length. What it
        does not have is contiguity, and an echo is a contiguous stretch of
        what we said.
        """
        self.bot.note(
            "I can add that to your calendar, and I can also add a note to the "
            "entry if you want, so that one is easy."
        )
        self.assertFalse(
            self.bot.is_probable_echo("can you also add a note to that one", bot_was_audible=True)
        )

    def test_the_allowance_does_not_grow_with_the_window(self):
        """The property behind the case above, stated so it cannot regress by
        someone raising a constant: lengthening the reply must not make a
        given turn easier to silence."""
        short_reply = "Shall I add a note to the entry?"
        long_reply = (
            "I can add that to your calendar, and I can also add a note to the "
            "entry if you want, so that one is easy, and I can do the same for "
            "the others if it would help you keep track of them."
        )
        said = "can you also add a note to that one"
        for reply in (short_reply, long_reply):
            bot = BotSpeech(clock=FakeClock())
            bot.note(reply)
            self.assertFalse(
                bot.is_probable_echo(said, bot_was_audible=True),
                f"silenced against a {len(reply.split())}-word reply",
            )

    def test_an_echo_with_a_word_dropped_in_the_middle_is_still_caught(self):
        """Why the match is ordered rather than contiguous: recognition of a
        speaker across a room drops words as readily as it mangles them, and a
        substring test cannot survive one falling out of the middle."""
        self.bot.note("Your first meeting tomorrow is at nine with the finance team.")
        self.assertTrue(
            self.bot.is_probable_echo(
                "your first meeting is at nine with the finance team", bot_was_audible=True
            )
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

    def test_a_two_word_confirmation_is_not_an_echo_of_the_offer(self):
        """The verbatim arm's own version of the short-barge-in failure, and
        it fires in a **silent room** — on headphones, where there is no echo
        to have at all.

        "go ahead" is 8 characters and a substring of the offer that prompted
        it, so the old character bar dropped it; and because turn-start is
        transcription-based the owner is not answered slowly, they are not
        answered at all, and they repeat themselves into a mic that keeps
        discarding them.
        """
        self.bot.note("Shall I go ahead?")
        for audible in (False, True):
            self.assertFalse(
                self.bot.is_probable_echo("go ahead", bot_was_audible=audible),
                f"silenced with bot_was_audible={audible}",
            )

    def test_short_verbs_the_reply_offered_are_not_echoes(self):
        """The same shape, and the reason it is worth a loop: every one of
        these is nine characters, a substring of the sentence that offered it,
        and the plainest way to say yes to it."""
        self.bot.note("I can cancel it, delete it, or do it now — which would you like?")
        for said in ["cancel it", "delete it", "do it now"]:
            self.assertFalse(
                self.bot.is_probable_echo(said, bot_was_audible=True),
                f"{said!r} was silenced as an echo",
            )

    def test_a_verbatim_echo_long_enough_to_mean_it_still_counts(self):
        """The arm still does its job with nothing playing — which is the
        point of keeping it unconditional: the timing layer can be wrong, and
        this is the fallback for when it is. What a fallback needs to catch is
        a whole sentence, not a phrase."""
        self.bot.note("I have moved the seminar to Thursday afternoon, as you asked.")
        self.assertTrue(
            self.bot.is_probable_echo(
                "i have moved the seminar to thursday afternoon as you asked",
                bot_was_audible=False,
            )
        )

    def test_a_plain_instruction_is_not_an_echo_of_the_offer(self):
        """**The verbatim arm's last short-turn failure**, and the worst of
        them: it fires with nothing playing.

        "Move it to Thursday" is a contiguous span of the offer that prompted
        it, and on headphones the timing layer correctly reports no speaker —
        so the energy floor behind it is the ordinary one, and the defence the
        module points at for this band is not armed. The turn does not degrade;
        it does not happen.
        """
        self.bot.note("I can move it to Thursday if you want me to.")
        for said in ["move it to thursday", "i can move it to thursday"]:
            self.assertFalse(
                self.bot.is_probable_echo(said, bot_was_audible=False),
                f"{said!r} was silenced on headphones",
            )

    def test_the_silent_room_case_does_not_rest_on_its_first_word(self):
        """`test_agreeing_in_a_silent_room_is_not_an_echo` passed only because
        of its leading "yes" — drop that and the phrase is a clean substring of
        the question. The bare form is the one a person actually says."""
        self.bot.note("Shall I move the seminar to Thursday?")
        self.assertFalse(
            self.bot.is_probable_echo("move the seminar to thursday", bot_was_audible=False)
        )

    def test_the_window_closes(self):
        # Audible, so the verdict turns on the window rather than on a floor.
        self.bot.note("Your first meeting tomorrow is at nine with the finance team.")
        self.clock.advance(30)
        self.assertFalse(
            self.bot.is_probable_echo(
                "your first meeting tomorrow is at nine with the finance team",
                bot_was_audible=True,
            )
        )

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
