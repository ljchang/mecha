"""An answered offer whose browser never connects ends at the connect
deadline, not the fifteen-minute idle timeout - and an incognito one lets
its log silence go with it. The deadline itself, then the real `bot()` on a
real peer connection nobody answers. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_connect_deadline.py`."""

import asyncio
import contextlib
import io
import sys
import time
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import worker  # noqa: E402
from worker import UNLOGGED, end_unless_connected, settle_deadline  # noqa: E402


class TheDeadline(unittest.TestCase):
    def test_a_call_that_never_connects_is_ended_and_says_why(self):
        ended = []

        async def go():
            async def end():
                ended.append(True)

            return await end_unless_connected(asyncio.Event(), 0.05, end, asyncio.Event())

        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertTrue(asyncio.run(go()))
        self.assertEqual(ended, [True])
        self.assertIn("never connected", out.getvalue())

    def test_a_call_that_connects_in_time_is_left_alone(self):
        ended = []

        async def go():
            connected = asyncio.Event()

            async def end():
                ended.append(True)

            asyncio.get_running_loop().call_later(0.01, connected.set)
            return await end_unless_connected(connected, 0.5, end, asyncio.Event())

        self.assertFalse(asyncio.run(go()))
        self.assertEqual(ended, [])


class TheTidyUp(unittest.TestCase):
    """What `run_bot`'s `finally` does with the deadline (review of #386)."""

    def test_a_fired_deadline_finishes_the_teardown_it_asked_for(self):
        # `runner.run()` can return while the deadline's `runner.cancel()` is
        # still running; the tidy-up must not cut it short.
        done = []

        async def go():
            fired = asyncio.Event()

            async def slow_end():
                await asyncio.sleep(0.1)
                done.append(True)

            task = asyncio.create_task(
                end_unless_connected(asyncio.Event(), 0.01, slow_end, fired)
            )
            await fired.wait()  # the deadline has fired and is mid-teardown
            await settle_deadline(task, fired)

        with contextlib.redirect_stdout(io.StringIO()):
            asyncio.run(go())
        self.assertEqual(done, [True], "the teardown was interrupted")

    def test_a_failed_teardown_is_raised_not_swallowed(self):
        async def go():
            fired = asyncio.Event()

            async def failing_end():
                raise RuntimeError("the cancel failed")

            task = asyncio.create_task(
                end_unless_connected(asyncio.Event(), 0.01, failing_end, fired)
            )
            await fired.wait()
            await settle_deadline(task, fired)

        with contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(RuntimeError, "the cancel failed"):
                asyncio.run(go())

    def test_an_unfired_deadline_is_cancelled(self):
        ended = []

        async def go():
            fired = asyncio.Event()

            async def end():
                ended.append(True)

            task = asyncio.create_task(
                end_unless_connected(asyncio.Event(), 30, end, fired)
            )
            await asyncio.sleep(0)
            started = time.monotonic()
            await settle_deadline(task, fired)
            return time.monotonic() - started, task.cancelled()

        took, cancelled = asyncio.run(go())
        self.assertLess(took, 1, "the tidy-up waited out a deadline that had not fired")
        self.assertTrue(cancelled)
        self.assertEqual(ended, [])


class TheRealBot(unittest.TestCase):
    """The failure as measured on 2026-09-28: an incognito offer answered,
    then nobody connects. Driven through the real `bot()` on a real
    `SmallWebRTCConnection` initialised from a dummy offer - the runner's
    own two calls - with the deadline cut to a second."""

    def test_an_unconnected_incognito_call_ends_and_releases_the_silence(self):
        from pipecat.runner.types import SmallWebRTCRunnerArguments
        from pipecat.transports.smallwebrtc.connection import SmallWebRTCConnection

        async def go():
            connection = SmallWebRTCConnection(ice_servers=[])
            await connection.initialize(sdp="v=0", type="offer")
            args = SmallWebRTCRunnerArguments(
                webrtc_connection=connection,
                body={"session": "incognito-0123456789abcdef012345"},
            )
            started = time.monotonic()
            try:
                await asyncio.wait_for(worker.bot(args), timeout=60)
            except asyncio.TimeoutError:
                return None
            return time.monotonic() - started

        saved = worker.CONNECT_DEADLINE_SECS
        worker.CONNECT_DEADLINE_SECS = 1.0
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                took = asyncio.run(go())
        finally:
            worker.CONNECT_DEADLINE_SECS = saved
        # The failure message has to be reachable: an unended call is a
        # timeout in `go()`, returned as None, not an error from `wait_for`.
        self.assertIsNotNone(took, "the call ran on past 60 s towards the idle timeout")
        self.assertLess(took, 10, "the call outlived its one-second deadline by far")
        self.assertFalse(UNLOGGED.active, "the silence outlived the call")


if __name__ == "__main__":
    unittest.main()
