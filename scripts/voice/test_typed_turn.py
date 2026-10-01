"""A line typed into a live call becomes a user turn and nothing else
(the owner's ask, 2026-10-01): trimmed, capped, and never a field or a
setting. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_typed_turn.py`."""

import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import asyncio  # noqa: E402

from worker import MAX_TYPED_CHARS, UNLOGGED, TypedTurns, typed_turn  # noqa: E402


class TypedTurn(unittest.TestCase):
    def test_a_line_is_the_turn_trimmed(self):
        self.assertEqual(typed_turn({"text": "  how was the dig?  "}), "how was the dig?")

    def test_nothing_worth_a_turn_is_none(self):
        for data in (None, {}, {"text": ""}, {"text": "   "}, {"text": 7}, "text", {"voice": "ada"}):
            self.assertIsNone(typed_turn(data), repr(data))

    def test_a_long_paste_is_capped(self):
        self.assertEqual(len(typed_turn({"text": "x" * (MAX_TYPED_CHARS + 50)})), MAX_TYPED_CHARS)



class Delivery(unittest.TestCase):
    """The one consumer (review of #499): a second line waits for the first
    answer to be spoken, and the journal keeps no words of an incognito
    call's lines."""

    def run_turns(self, lines, speaking_script, unlogged=False):
        pushed, logged, speaking = [], [], {"now": False}

        async def push(text):
            pushed.append(text)
            # The answer: speaking starts, then ends, on the script's steps.
            for state in speaking_script:
                await asyncio.sleep(0.15)
                speaking["now"] = state

        async def main():
            turns = TypedTurns(push, lambda: speaking["now"], log=logged.append,
                               start_secs=1.0, end_secs=1.0)
            for line in lines:
                turns.put({"text": line})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(0.05)
            first_alone = list(pushed)
            await asyncio.sleep(1.5)
            task.cancel()
            return first_alone

        with UNLOGGED.held(unlogged):
            first_alone = asyncio.run(main())
        return pushed, logged, first_alone

    def test_a_second_line_waits_for_the_first_answer(self):
        pushed, _, first_alone = self.run_turns(["one", "two"], [True, False])
        self.assertEqual(first_alone, ["one"], "the second line went before the first was answered")
        self.assertEqual(pushed, ["one", "two"], "both lines, in order")

    def test_an_incognito_calls_lines_leave_no_words_in_the_log(self):
        _, logged, _ = self.run_turns(["the secret plan"], [], unlogged=True)
        self.assertEqual(len(logged), 1)
        self.assertNotIn("secret", logged[0])

    def test_a_turn_that_never_speaks_does_not_hold_the_next_forever(self):
        pushed, _, _ = self.run_turns(["one", "two"], [])
        self.assertEqual(pushed, ["one", "two"])


if __name__ == "__main__":
    unittest.main()
