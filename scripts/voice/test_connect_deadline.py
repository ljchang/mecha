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
from worker import UNLOGGED, end_unless_connected  # noqa: E402


class TheDeadline(unittest.TestCase):
    def test_a_call_that_never_connects_is_ended_and_says_why(self):
        ended = []

        async def go():
            async def end():
                ended.append(True)

            return await end_unless_connected(asyncio.Event(), 0.05, end)

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
            return await end_unless_connected(connected, 0.5, end)

        self.assertFalse(asyncio.run(go()))
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
            await asyncio.wait_for(worker.bot(args), timeout=60)
            return time.monotonic() - started

        saved = worker.CONNECT_DEADLINE_SECS
        worker.CONNECT_DEADLINE_SECS = 1.0
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                took = asyncio.run(go())
        finally:
            worker.CONNECT_DEADLINE_SECS = saved
        self.assertLess(took, 60, "the call ran on to the idle timeout")
        self.assertFalse(UNLOGGED.active, "the silence outlived the call")


if __name__ == "__main__":
    unittest.main()
