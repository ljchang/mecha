"""A line typed into a live call becomes a user turn and nothing else
(the owner's ask, 2026-10-01): trimmed, capped, and never a field or a
setting. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_typed_turn.py`."""

import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import asyncio  # noqa: E402
import time  # noqa: E402
from types import SimpleNamespace  # noqa: E402

from worker import MAX_TYPED_CHARS, UNLOGGED, Answering, TypedTurns, answer_pending, typed_turn  # noqa: E402


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


class AnsweringFrames(unittest.TestCase):
    """The model's answer, as the consumer sees it: busy from an LLM
    response's start frame to its end, every frame passed on - so an answer
    that speaks nothing still ends (review of #499, pass 3)."""

    def test_busy_from_start_to_end_and_frames_pass(self):
        from pipecat.frames.frames import LLMFullResponseEndFrame, LLMFullResponseStartFrame
        from pipecat.processors.frame_processor import FrameDirection

        async def main():
            a = Answering()
            passed = []

            async def push(frame, direction=FrameDirection.DOWNSTREAM):
                passed.append(frame)

            a.push_frame = push
            states = [a.busy]
            await a.process_frame(LLMFullResponseStartFrame(), FrameDirection.DOWNSTREAM)
            states.append(a.busy)
            await a.process_frame(LLMFullResponseEndFrame(), FrameDirection.DOWNSTREAM)
            states.append(a.busy)
            return states, len(passed)

        states, passed = asyncio.run(main())
        self.assertEqual(states, [False, True, False])
        self.assertEqual(passed, 2, "a frame was swallowed")

    def test_an_interruption_ends_the_answer(self):
        from pipecat.frames.frames import InterruptionFrame, LLMFullResponseStartFrame
        from pipecat.processors.frame_processor import FrameDirection

        async def main():
            a = Answering()

            async def push(frame, direction=FrameDirection.DOWNSTREAM):
                pass

            a.push_frame = push
            await a.process_frame(LLMFullResponseStartFrame(), FrameDirection.DOWNSTREAM)
            await a.process_frame(InterruptionFrame(), FrameDirection.DOWNSTREAM)
            return a.busy, a.ended_at

        busy, ended_at = asyncio.run(main())
        self.assertFalse(busy, "an interrupted answer latched busy")
        self.assertIsNotNone(ended_at, "an interrupted answer has no end for the settle window")


class Gap(unittest.TestCase):
    """The model's end frame and its first spoken audio are not adjacent;
    `answer_pending` keeps the gap busy, timestamped where the answer ends,
    so whichever turn the answer was to (reviews of #499, passes 4 and 5)."""

    def call(self, settle=0.6):
        model = SimpleNamespace(busy=False, ended_at=None)
        speaking = {"now": False}
        events = {"ended": []}

        async def answer(think=0.2, gap=0.4, speak=0.3, lead=0.1):
            await asyncio.sleep(lead)
            model.busy = True    # the model answering
            await asyncio.sleep(think)
            model.busy = False   # done; no audio yet
            model.ended_at = time.monotonic()
            await asyncio.sleep(gap)
            speaking["now"] = True   # the speech
            await asyncio.sleep(speak)
            speaking["now"] = False
            events["ended"].append(asyncio.get_running_loop().time())

        busy = lambda: answer_pending(model, lambda: speaking["now"], settle=settle)  # noqa: E731
        return busy, answer, events

    def test_the_quiet_between_the_model_and_its_speech_is_not_the_end(self):
        busy, answer, events = self.call()
        pushed = []

        async def push(text):
            pushed.append((text, asyncio.get_running_loop().time()))
            asyncio.get_running_loop().create_task(answer())

        async def main():
            turns = TypedTurns(push, busy, log=lambda _: None, start_secs=2.0, end_secs=3.0)
            turns.put({"text": "one"})
            turns.put({"text": "two"})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(2.2)
            task.cancel()

        asyncio.run(main())
        self.assertEqual([t for t, _ in pushed], ["one", "two"])
        self.assertTrue(events["ended"])
        self.assertGreaterEqual(pushed[1][1], events["ended"][0], "the second line went in the gap, before the speech")

    def test_a_line_typed_as_a_spoken_answer_finishes_waits_for_its_speech(self):
        """The answer is to something said aloud, not typed: a line typed
        just after the model finishes still waits for the speech (pass 5)."""
        busy, answer, events = self.call()
        pushed = []

        async def push(text):
            pushed.append((text, asyncio.get_running_loop().time()))

        async def main():
            spoken = asyncio.create_task(answer(lead=0.0, think=0.1))
            await asyncio.sleep(0.15)  # the model is done; nothing spoken yet
            turns = TypedTurns(push, busy, log=lambda _: None, start_secs=0.5, end_secs=3.0)
            turns.put({"text": "and another thing"})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(1.5)
            task.cancel()
            await spoken

        asyncio.run(main())
        self.assertEqual([t for t, _ in pushed], ["and another thing"])
        self.assertGreaterEqual(pushed[0][1], events["ended"][0], "the line went in the gap of a spoken answer")

    def test_a_late_turn_handed_over_mid_answer_waits_for_its_speech(self):
        """A late span transcribed while a spoken answer is under way goes
        after that answer's speech, not in the gap before it."""
        busy, answer, events = self.call()
        sent = []

        async def late():
            sent.append(("late", asyncio.get_running_loop().time()))

        async def main():
            spoken = asyncio.create_task(answer(lead=0.0, think=0.1))
            await asyncio.sleep(0.15)
            turns = TypedTurns(None, busy, log=lambda _: None, start_secs=0.5, end_secs=3.0)
            turns.deliver(late)
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(1.5)
            task.cancel()
            await spoken

        asyncio.run(main())
        self.assertEqual([t for t, _ in sent], ["late"])
        self.assertGreaterEqual(sent[0][1], events["ended"][0], "the late turn went in the gap of a spoken answer")

    def test_a_late_turn_waits_for_a_typed_turns_answer(self):
        """One queue for every turn outside live audio: a late span handed
        over while a typed line is being answered goes after that answer,
        never into it (pass 5)."""
        busy, answer, events = self.call()
        sent = []

        async def push(text):
            sent.append((text, asyncio.get_running_loop().time()))
            asyncio.get_running_loop().create_task(answer())

        async def late():
            sent.append(("late", asyncio.get_running_loop().time()))

        async def main():
            turns = TypedTurns(push, busy, log=lambda _: None, start_secs=2.0, end_secs=3.0)
            turns.put({"text": "typed"})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(0.05)
            turns.deliver(late)
            await asyncio.sleep(2.0)
            task.cancel()

        asyncio.run(main())
        self.assertEqual([t for t, _ in sent], ["typed", "late"])
        self.assertGreaterEqual(sent[1][1], events["ended"][0], "the late turn landed inside the typed turn's answer")


class LateFirst(unittest.TestCase):
    """The uplink's live audio waits behind a late turn's delivery (§2.5),
    so a late turn goes ahead of typed lines and waits least (review of
    #499, pass 6)."""

    def test_a_late_turn_jumps_the_typed_lines_waiting(self):
        sent = []

        async def push(text):
            sent.append(text)

        async def late():
            sent.append("late")

        async def main():
            turns = TypedTurns(push, lambda: False, log=lambda _: None, start_secs=0.1, end_secs=0.5)
            for line in ("one", "two"):
                turns.put({"text": line})
            done = turns.deliver(late)
            task = asyncio.create_task(turns.run())
            await asyncio.wait_for(done, 2.0)
            await asyncio.sleep(0.6)
            task.cancel()

        asyncio.run(main())
        self.assertEqual(sent, ["late", "one", "two"])

    def test_a_late_turn_does_not_wait_out_a_typed_lines_bound(self):
        """A typed line is waiting on a long answer: the late turn arriving
        meanwhile goes at its own short bound, not the typed line's three
        minutes - live speech is queued behind it."""
        sent = []

        async def late():
            sent.append(("late", asyncio.get_running_loop().time()))

        async def main():
            loop = asyncio.get_running_loop()
            t0 = loop.time()
            turns = TypedTurns(None, lambda: True, log=lambda _: None, start_secs=1.0, end_secs=180.0,
                               late_secs=0.4)
            turns.put({"text": "waits on the answer"})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(0.2)
            done = turns.deliver(late)
            await asyncio.wait_for(done, 3.0)
            task.cancel()
            return t0

        t0 = asyncio.run(main())
        self.assertEqual([k for k, _ in sent], ["late"])
        self.assertLess(sent[0][1] - t0, 1.5, "the late turn waited behind the typed line's bound")

    def test_delivered_means_sent(self):
        order = []

        async def late():
            await asyncio.sleep(0.2)
            order.append("sent")

        async def main():
            turns = TypedTurns(None, lambda: False, log=lambda _: None)
            task = asyncio.create_task(turns.run())
            done = turns.deliver(late)
            await done
            order.append("awaited")
            task.cancel()

        asyncio.run(main())
        self.assertEqual(order, ["sent", "awaited"])


class LateSurvives(unittest.TestCase):
    def test_a_failure_waiting_on_a_late_turns_answer_does_not_end_the_queue(self):
        """The wait after a late turn is sent can raise too: the next turn
        must still go, and the uplink must have been released at the send,
        not held behind the answer (review of #499, pass 8)."""
        sent, logged = [], []
        calls = {"n": 0}

        def busy():
            calls["n"] += 1
            if sent == ["late"] and calls["n"] < 1000:
                calls["n"] = 1000
                raise RuntimeError("the pipeline is going away")
            return False

        async def push(text):
            sent.append(text)

        async def late():
            sent.append("late")

        async def main():
            turns = TypedTurns(push, busy, log=logged.append, start_secs=0.3, end_secs=0.3)
            done = turns.deliver(late)
            turns.put({"text": "after"})
            task = asyncio.create_task(turns.run())
            await asyncio.wait_for(done, 2.0)
            await asyncio.sleep(1.0)
            task.cancel()

        asyncio.run(main())
        self.assertEqual(sent, ["late", "after"], "the consumer died after the late turn")
        self.assertTrue(any("late turn failed: RuntimeError" in l for l in logged), logged)

    def test_delivered_before_the_answer_is_over(self):
        """`done` resolves at the send: the live audio queued behind the
        flush must not wait for the whole answer."""
        times = {}

        async def late():
            times["sent"] = asyncio.get_running_loop().time()

        async def main():
            loop = asyncio.get_running_loop()
            answering = {"until": None}

            def busy():
                return answering["until"] is not None and loop.time() < answering["until"]

            async def send():
                await late()
                answering["until"] = loop.time() + 1.0  # a long answer begins

            turns = TypedTurns(None, busy, log=lambda _: None, start_secs=1.0, end_secs=3.0)
            task = asyncio.create_task(turns.run())
            await turns.deliver(send)
            times["done"] = loop.time()
            task.cancel()

        asyncio.run(main())
        self.assertLess(times["done"] - times["sent"], 0.5, "the flush was held behind the answer")


class Queue(unittest.TestCase):
    def test_put_says_why_a_line_was_not_queued(self):
        from worker import TYPED_QUEUE_MAX

        async def main():
            turns = TypedTurns(None, lambda: False, log=lambda _: None)
            return [turns.put({"text": f"line {i}"}) for i in range(TYPED_QUEUE_MAX + 1)], turns.put({"text": " "})

        results, empty = asyncio.run(main())
        self.assertEqual(results[:-1], ["queued"] * TYPED_QUEUE_MAX)
        self.assertEqual(results[-1], "full")
        self.assertEqual(empty, "not-a-turn")


class SilentAnswer(unittest.TestCase):
    def test_an_answer_that_speaks_nothing_releases_the_next_line_promptly(self):
        """Busy (the model answering) and then not, with no speech at all:
        the next line goes when the answer ends, not at the start bound."""
        pushed = []
        model = SimpleNamespace(busy=False, ended_at=None)

        async def push(text):
            pushed.append((text, asyncio.get_running_loop().time()))

            async def answer():
                await asyncio.sleep(0.1)
                model.busy = True
                await asyncio.sleep(0.2)
                model.busy = False
                model.ended_at = time.monotonic()

            asyncio.get_running_loop().create_task(answer())

        async def main():
            busy = lambda: answer_pending(model, lambda: False, settle=0.3)  # noqa: E731
            turns = TypedTurns(push, busy, log=lambda _: None, start_secs=5.0, end_secs=5.0)
            turns.put({"text": "one"})
            turns.put({"text": "two"})
            task = asyncio.create_task(turns.run())
            await asyncio.sleep(1.0)
            task.cancel()

        asyncio.run(main())
        self.assertEqual([t for t, _ in pushed], ["one", "two"])
        self.assertLess(pushed[1][1] - pushed[0][1], 1.0, "the next line waited out the start bound")


if __name__ == "__main__":
    unittest.main()
