#!/usr/bin/env python3
"""Tests for the turn-end logic the voice worker wraps around pipecat.

These need pipecat, so they run in the voice worker's venv and nowhere else:

    ~/models/voice-worker-venv/bin/python scripts/voice/test_turn_stop.py

Not in CI on purpose (the runner has no pipecat), which is why this file
refuses to *pass* without it: a test that skips reads exactly like one that
ran.

Each case is a real turn from the 2026-09-04 12:55 UTC call, replayed
against a scripted turn analyzer and a real clock.
"""
import asyncio
import sys
import time
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

try:
    from pipecat.audio.turn.base_turn_analyzer import (
        BaseTurnAnalyzer,
        BaseTurnParams,
        EndOfTurnState,
    )
    from pipecat.frames.frames import (
        STTMetadataFrame,
        TranscriptionFrame,
        VADUserStartedSpeakingFrame,
        VADUserStoppedSpeakingFrame,
    )
    from pipecat.turns.user_stop.turn_analyzer_user_turn_stop_strategy import (
        TurnAnalyzerUserTurnStopStrategy,
    )
    from pipecat.utils.asyncio.task_manager import TaskManager
except ImportError as e:  # pragma: no cover - the whole point is to be loud
    print(f"test_turn_stop: needs pipecat ({e}); run it in the voice venv", file=sys.stderr)
    sys.exit(2)

from openai.types.audio import Transcription  # noqa: E402

from worker import STT_TTFS_P99, ParakeetSTT, TranscriptStartedTurnStop  # noqa: E402

# Pipecat's own default (`DEFAULT_TTFS_P99`), which is what the worker ran
# with until STT_TTFS_P99 existed. The "old behaviour" cases use it so the
# 0.8s they demonstrate is the 0.8s the journal showed.
PIPECAT_DEFAULT_P99 = 1.0
VAD_STOP_SECS = 0.2


class ScriptedAnalyzer(BaseTurnAnalyzer):
    """A smart-turn that says what the test tells it to, in order."""

    def __init__(self, verdicts):
        super().__init__(sample_rate=16000)
        self.verdicts = list(verdicts)

    @property
    def speech_triggered(self) -> bool:
        return False

    @property
    def params(self) -> BaseTurnParams:
        return BaseTurnParams()

    def append_audio(self, buffer: bytes, is_speech: bool) -> EndOfTurnState:
        return EndOfTurnState.INCOMPLETE

    async def analyze_end_of_turn(self):
        return self.verdicts.pop(0), None

    def clear(self):
        pass


class Call:
    """One strategy under test, driven the way the aggregator drives it:
    VAD edges, then (for a transcript-started turn) the turn start, then the
    transcript. `stopped` records when the strategy ended the turn."""

    def __init__(self, strategy_cls, verdicts, p99):
        self.analyzer = ScriptedAnalyzer(verdicts)
        self.strategy = strategy_cls(turn_analyzer=self.analyzer)
        self.stopped: list[float] = []
        self.p99 = p99

    async def start(self):
        await self.strategy.setup(TaskManager())

        @self.strategy.event_handler("on_user_turn_stopped")
        async def _on_stopped(strategy, params):
            self.stopped.append(time.monotonic())
            # What the controller does next: tell every stop strategy the
            # turn is over.
            await strategy.handle_user_turn_stopped()

        await self.strategy.process_frame(
            STTMetadataFrame(service_name="parakeet", ttfs_p99_latency=self.p99)
        )

    async def speaks(self):
        await self.strategy.process_frame(VADUserStartedSpeakingFrame(start_secs=0.3))

    async def stops_speaking(self):
        await self.strategy.process_frame(
            VADUserStoppedSpeakingFrame(stop_secs=VAD_STOP_SECS, timestamp=time.time())
        )

    async def turn_starts(self):
        await self.strategy.handle_user_turn_started()

    async def transcript(self, text):
        await self.strategy.process_frame(
            TranscriptionFrame(text, "owner", "now", finalized=True)
        )

    async def close(self):
        await self.strategy.cleanup()


def run(coro):
    return asyncio.run(coro)


class TranscriptStartedTurns(unittest.TestCase):
    """The first segment of a turn, where the transcript starts the turn
    *after* the VAD stop and after smart-turn has ruled."""

    def test_incomplete_first_fragment_is_held_open(self):
        """"add a couple" — smart-turn said INCOMPLETE; the stock strategy
        ended the turn 0.8s after the transcript anyway."""

        async def scenario(cls, p99):
            call = Call(cls, [EndOfTurnState.INCOMPLETE, EndOfTurnState.COMPLETE], p99)
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            await call.transcript("add a couple")
            await asyncio.sleep(1.5)
            held = not call.stopped
            # The owner goes on: "...of to-dos", and this time smart-turn
            # says COMPLETE. The turn must end on that transcript.
            await call.speaks()
            await call.stops_speaking()
            await call.transcript("of to-dos.")
            await asyncio.sleep(0.05)
            ended = bool(call.stopped)
            await call.close()
            return held, ended

        held, ended = run(scenario(TranscriptStartedTurnStop, STT_TTFS_P99))
        self.assertTrue(held, "INCOMPLETE was overruled by a timer")
        self.assertTrue(ended, "COMPLETE with its transcript in hand did not end the turn")

        # Fails on the old behaviour: the stock strategy, with pipecat's
        # default latency, ends the turn ~0.8s after the first transcript.
        held, _ = run(scenario(TurnAnalyzerUserTurnStopStrategy, PIPECAT_DEFAULT_P99))
        self.assertFalse(held, "the stock strategy no longer shows the fault this guards")

    def test_complete_first_fragment_ends_on_its_transcript(self):
        """"Tell me about Jonathan Phillips." — smart-turn said COMPLETE
        before the transcript arrived. The turn should end the moment the
        text lands, not a timer later."""

        async def scenario(cls, p99):
            call = Call(cls, [EndOfTurnState.COMPLETE], p99)
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            t0 = time.monotonic()
            await call.transcript("Tell me about Jonathan Phillips.")
            await asyncio.sleep(1.2)
            await call.close()
            return [t - t0 for t in call.stopped]

        lag = run(scenario(TranscriptStartedTurnStop, STT_TTFS_P99))
        self.assertEqual(len(lag), 1)
        self.assertLess(lag[0], 0.2, f"the turn waited {lag[0]:.2f}s past its own transcript")

        lag = run(scenario(TurnAnalyzerUserTurnStopStrategy, PIPECAT_DEFAULT_P99))
        self.assertEqual(len(lag), 1)
        self.assertGreater(lag[0], 0.6, "the stock strategy used to wait out the timer here")

    def test_a_vad_started_turn_still_resets(self):
        """When no VAD stop is pending the override is the stock strategy:
        a stale ruling from an earlier turn must not survive the start."""

        async def scenario():
            call = Call(TranscriptStartedTurnStop, [EndOfTurnState.INCOMPLETE], STT_TTFS_P99)
            await call.start()
            s = call.strategy
            s._turn_complete = True
            s._text = "leftover"
            await call.turn_starts()
            cleared = (not s._turn_complete) and s._text == ""
            await call.close()
            return cleared

        self.assertTrue(run(scenario()))

    def test_a_transcript_landing_after_the_owner_resumed_is_not_a_timer(self):
        """The 2026-09-01 shape: "Can you research" [INCOMPLETE], and the
        owner is already saying "Um options for" when its transcript lands.
        The VAD start has cleared the pending stop, so the override defers
        to the stock reset - and the stock fallback timer must still not
        arm, because the owner is audibly speaking."""

        async def scenario(cls):
            call = Call(cls, [EndOfTurnState.INCOMPLETE, EndOfTurnState.COMPLETE], STT_TTFS_P99)
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.speaks()  # resumed before the transcript arrived
            await call.turn_starts()
            await call.transcript("Can you research")
            await asyncio.sleep(STT_TTFS_P99 + 0.3)  # past any timer it could arm
            held = not call.stopped
            await call.stops_speaking()
            await asyncio.sleep(0.05)
            # The second segment's words are still in flight here.
            ended_early = bool(call.stopped)
            await call.transcript("options for a Starlink for a car.")
            await asyncio.sleep(0.05)
            ended = len(call.stopped) == 1
            await call.close()
            return held, ended_early, ended

        held, early, ended = run(scenario(TranscriptStartedTurnStop))
        self.assertTrue(held, "a timer ended the turn while the owner was speaking")
        self.assertFalse(early, "the turn ended before the second segment's words arrived")
        self.assertTrue(ended)

        # Fails on the stock strategy and on the first revision of this
        # class: the earlier transcript was still "finalized" at the second
        # VAD stop, so the COMPLETE there ended the turn without the second
        # segment's words - which then opened a turn of their own.
        _, early, _ = run(scenario(TurnAnalyzerUserTurnStopStrategy))
        self.assertTrue(early, "the stock strategy no longer shows the fault this guards")

    def test_a_kept_ruling_does_not_outlive_its_turn(self):
        """The override skips the stock reset at turn *start* when a VAD stop
        is pending. The reset at turn *end* is untouched, so turn N+1 must
        begin with nothing of turn N — no text for an expired safety net to
        end it on, no ruling, no pending stop."""

        async def scenario():
            call = Call(
                TranscriptStartedTurnStop,
                [EndOfTurnState.COMPLETE, EndOfTurnState.INCOMPLETE],
                STT_TTFS_P99,
            )
            await call.start()
            s = call.strategy
            # Turn 1: a complete question, ended on its own transcript.
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            await call.transcript("Tell me about Jonathan Phillips.")
            await asyncio.sleep(0.05)
            ended_once = len(call.stopped) == 1
            # The harness has already relayed the controller's stop callback.
            # `_turn_complete` is not asserted: the stock fallback branch runs
            # *after* that callback, inside the same transcript handling, and
            # leaves it True with a timer armed - in pipecat itself too. It
            # cannot end anything without text, which is what is asserted.
            clean = (s._text, s._vad_stopped) == ("", False)
            # Turn 2 opens on an INCOMPLETE first fragment and must be held,
            # not ended by turn 1's leftovers.
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            await call.transcript("add a couple")
            await asyncio.sleep(1.5)
            held = len(call.stopped) == 1
            await call.close()
            return ended_once, clean, held

        ended_once, clean, held = run(scenario())
        self.assertTrue(ended_once)
        self.assertTrue(clean, "turn 1's state survived its own end")
        self.assertTrue(held, "turn 2 was ended on turn 1's leftovers")

    def test_the_private_it_reads_is_checked_at_construction(self):
        """A pipecat rename must refuse the call, not quietly degrade."""
        s = TranscriptStartedTurnStop(turn_analyzer=ScriptedAnalyzer([]))
        del s._vad_stopped
        with self.assertRaises(RuntimeError):
            s._refuse_if_pipecat_moved()


class Transcripts(unittest.TestCase):
    def test_every_parakeet_transcript_is_final(self):
        stt = ParakeetSTT(api_key="unused", base_url="http://127.0.0.1:1/v1")

        async def fake(audio):
            return Transcription(text="add a couple")

        stt._transcribe = fake

        async def scenario():
            frames = []
            async for f in stt.run_stt(b"wav"):
                frames.append(f)
            return frames

        frames = run(scenario())
        texts = [f for f in frames if isinstance(f, TranscriptionFrame)]
        self.assertEqual(len(texts), 1)
        self.assertTrue(texts[0].finalized)

    def test_the_stt_wait_reaches_the_pipeline(self):
        """Not the constructor argument - the frame the service broadcasts
        at start, which is the only way the number reaches the strategy."""
        stt = ParakeetSTT(api_key="unused", base_url="http://127.0.0.1:1/v1")
        frame = stt.service_metadata_frame()
        self.assertIsInstance(frame, STTMetadataFrame)
        self.assertEqual(frame.ttfs_p99_latency, STT_TTFS_P99)
        self.assertGreater(STT_TTFS_P99, VAD_STOP_SECS)


if __name__ == "__main__":
    unittest.main()
