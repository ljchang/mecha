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
    """The one consumer (reviews of #499): a second line waits for the first
    answer to be spoken, a failing push does not end the call's typing, and
    the journal keeps no words of an incognito call's lines."""

    def run_turns(self, lines, answer=(0.2, 0.3), fail_first=False, unlogged=False, wait=1.6):
        """`answer`: how long after a push the answer starts being spoken,
        and for how long — asynchronously, after `push` has returned, as a
        real answer is (pass 2: a fake that answered inside `push` measured
        nothing)."""
        pushed, logged, ended = [], [], []
        speaking = {"now": False}
        loop_time = lambda: asyncio.get_running_loop().time()  # noqa: E731

        async def speak_answer():
            start, length = answer
            await asyncio.sleep(start)
            speaking["now"] = True
            await asyncio.sleep(length)
            speaking["now"] = False
            ended.append(loop_time())

        async def push(text):
            if fail_first and not pushed:
                pushed.append((text, loop_time()))
                raise RuntimeError("the pipeline refused it")
            pushed.append((text, loop_time()))
            if answer:
                asyncio.get_running_loop().create_task(speak_answer())

        async def main():
            turns = TypedTurns(push, lambda: speaking["now"], log=logged.append,
                               start_secs=1.0, end_secs=2.0)
            for line in lines:
                turns.put({"text": line})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(wait)
            task.cancel()

        with UNLOGGED.held(unlogged):
            asyncio.run(main())
        return pushed, logged, ended

    def test_a_second_line_waits_until_the_first_answer_has_been_spoken(self):
        pushed, _, ended = self.run_turns(["one", "two"])
        self.assertEqual([t for t, _ in pushed], ["one", "two"])
        self.assertTrue(ended, "the first answer never finished")
        # The second line went only after the first answer ended — remove
        # the wait and it goes ~0.2 s after the first push, while it plays.
        self.assertGreaterEqual(pushed[1][1], ended[0])

    def test_a_failing_push_does_not_end_the_calls_typing(self):
        pushed, logged, _ = self.run_turns(["one", "two"], answer=None, fail_first=True)
        self.assertEqual([t for t, _ in pushed], ["one", "two"], "the consumer died on the first failure")
        self.assertTrue(any("failed: RuntimeError" in l for l in logged), logged)

    def test_an_incognito_calls_lines_leave_no_words_in_the_log(self):
        _, logged, _ = self.run_turns(["the secret plan"], answer=None, unlogged=True, wait=0.3)
        self.assertTrue(logged)
        self.assertFalse(any("secret" in l for l in logged), logged)

    def test_a_turn_that_never_speaks_does_not_hold_the_next_forever(self):
        pushed, _, _ = self.run_turns(["one", "two"], answer=None)
        self.assertEqual([t for t, _ in pushed], ["one", "two"])


if __name__ == "__main__":
    unittest.main()
