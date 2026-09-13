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

from worker import (  # noqa: E402
    LINK_PAGE_HOLD_EXPIRY_SECS,
    LINK_RESUME_SETTLE_SECS,
    LINK_STALL_SECS,
    LOOP_LAG_WARN_SECS,
    STT_TTFS_P99,
    LinkResumedFrame,
    LinkStalledFrame,
    LinkWatch,
    LoopSampler,
    ParakeetSTT,
    SegmentDroppedFrame,
    TranscriptStartedTurnStop,
)

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

    async def dropped(self):
        """What the STT says about a segment the gate or echo filter ate."""
        await self.strategy.process_frame(SegmentDroppedFrame())

    async def link_stalls(self):
        """What the link watch says when the microphone audio stops arriving."""
        await self.strategy.process_frame(LinkStalledFrame(reason="audio"))

    async def link_resumes(self):
        await self.strategy.process_frame(LinkResumedFrame(after_secs=2.5))

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

    def test_echo_between_turns_leaves_nothing_outstanding(self):
        """On speakers every bot sentence is a VAD segment the gate or the
        echo filter drops, with no turn open. The owner's next transcript
        must still close its turn on its own COMPLETE, not on the safety
        net 1.8 s later."""

        async def scenario():
            call = Call(
                TranscriptStartedTurnStop,
                [EndOfTurnState.INCOMPLETE] * 3 + [EndOfTurnState.COMPLETE],
                STT_TTFS_P99,
            )
            await call.start()
            for _ in range(3):  # the bot's reply, heard by the mic
                await call.speaks()
                await call.stops_speaking()
                await call.dropped()
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            t0 = time.monotonic()
            await call.transcript("What's next on my list?")
            await asyncio.sleep(0.3)
            lag = [t - t0 for t in call.stopped]
            await call.close()
            return lag

        lag = run(scenario())
        self.assertEqual(len(lag), 1, "the owner's turn did not end on its transcript")
        self.assertLess(lag[0], 0.2)

    def test_a_dropped_tail_ends_a_complete_turn_at_once(self):
        """Mid-turn: "add it to my to-dos" then a breath the gate drops,
        which is where smart-turn says COMPLETE. The words are all in;
        the turn ends when the STT says the breath was nothing."""

        async def scenario():
            call = Call(
                TranscriptStartedTurnStop,
                [EndOfTurnState.INCOMPLETE, EndOfTurnState.COMPLETE],
                STT_TTFS_P99,
            )
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.turn_starts()
            await call.transcript("Okay. I just need you to add it to my to do's.")
            await call.speaks()
            await call.stops_speaking()
            await asyncio.sleep(0.05)
            before_drop = bool(call.stopped)
            t0 = time.monotonic()
            await call.dropped()
            await asyncio.sleep(0.05)
            lag = [t - t0 for t in call.stopped]
            await call.close()
            return before_drop, lag

        before_drop, lag = run(scenario())
        self.assertFalse(before_drop, "ended before the STT reported on the last segment")
        self.assertEqual(len(lag), 1)
        self.assertLess(lag[0], 0.1)

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


class LinkStalls(unittest.TestCase):
    """The 2026-09-12 shape, from a moving car: "Can you add" [COMPLETE]
    at the instant the audio stopped arriving, and the rest of the sentence
    in the gap. A gap in packets is not silence."""

    def test_a_ruling_made_on_the_gap_waits_for_the_link(self):
        """COMPLETE and the transcript both land while the link is paused.
        The turn must not end until the audio is back — and when what comes
        back is the owner still talking, it must not end at all."""

        async def scenario(cls):
            call = Call(cls, [EndOfTurnState.COMPLETE, EndOfTurnState.COMPLETE], STT_TTFS_P99)
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.link_stalls()
            await call.turn_starts()
            await call.transcript("Can you add")
            await asyncio.sleep(STT_TTFS_P99 + 0.3)  # past the safety net too
            held = not call.stopped
            # The audio resumes carrying the rest of the sentence: the VAD
            # start lands before the release, as the settle guarantees.
            await call.speaks()
            await call.link_resumes()
            await asyncio.sleep(0.05)
            still_held = not call.stopped
            await call.stops_speaking()
            await call.transcript("a reminder to call Suburban about the furnace.")
            await asyncio.sleep(0.05)
            ended = len(call.stopped) == 1
            await call.close()
            return held, still_held, ended

        held, still_held, ended = run(scenario(TranscriptStartedTurnStop))
        self.assertTrue(held, "the turn ended on a ruling made across a paused link")
        self.assertTrue(still_held, "the release ended the turn although the owner had resumed")
        self.assertTrue(ended, "the whole sentence did not end the turn once")

        # Fails on the stock strategy: it knows nothing of the link and ends
        # the turn on the fragment.
        held, _, _ = run(scenario(TurnAnalyzerUserTurnStopStrategy))
        self.assertFalse(held, "the stock strategy no longer shows the fault this guards")

    def test_a_resumed_link_with_nothing_more_ends_the_turn(self):
        """The owner really was done: the hold delays the answer by the
        stall and no more."""

        async def scenario():
            call = Call(TranscriptStartedTurnStop, [EndOfTurnState.COMPLETE], STT_TTFS_P99)
            await call.start()
            await call.speaks()
            await call.stops_speaking()
            await call.link_stalls()
            await call.turn_starts()
            await call.transcript("What is on my calendar today?")
            await asyncio.sleep(0.3)
            held = not call.stopped
            await call.link_resumes()
            await asyncio.sleep(0.05)
            ended = len(call.stopped) == 1
            await call.close()
            return held, ended

        held, ended = run(scenario())
        self.assertTrue(held)
        self.assertTrue(ended, "the release did not act on the ruling it had been holding")


class LinkWatchHolds(unittest.TestCase):
    """Two sources can pause; both must clear. Driven directly, with the
    pushes recorded, because the processor needs no pipeline to decide."""

    def test_both_sources_must_clear(self):
        async def scenario():
            events = []
            watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))
            pushed = []

            async def push(frame, direction=None):
                pushed.append(type(frame).__name__)

            watch.push_frame = push
            await watch.hold("mic", now=0.0)
            watch._audio_stalled = True
            await watch._settle("audio")
            await watch.release()  # the page recovered; the audio has not
            still = watch.held
            watch._audio_stalled = False
            await watch._settle("audio")
            return events, pushed, still, watch.held

        events, pushed, still, held_after = run(scenario())
        self.assertEqual(events, [("paused", "mic"), ("ok", "audio")])
        self.assertEqual(pushed, ["LinkStalledFrame", "LinkResumedFrame"])
        self.assertTrue(still, "one source clearing released a hold the other still owned")
        self.assertFalse(held_after)


async def _record(events, state, reason):
    events.append((state, reason))


class LinkWatchClock(unittest.TestCase):
    """The audio clock that decides the hold, driven with a clock the test
    owns: frames through `_note_audio`, verdicts through `_judge`."""

    def test_one_stray_frame_does_not_lift_the_hold(self):
        """Review of #226: the settle measured time since the *first* frame
        back, so one packet in a dead link released the hold 0.6 s later
        and the held COMPLETE shipped the fragment anyway."""

        async def scenario():
            events = []
            watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))
            pushed = []

            async def push(frame, direction=None):
                pushed.append(type(frame).__name__)

            watch.push_frame = push
            t = 0.0
            while t < 1.0:  # a second of ordinary flow
                watch._note_audio(t)
                t += 0.02
            await watch._judge(1.0 + LINK_STALL_SECS)
            paused = list(events)
            # One stray packet, then nothing: no settle can complete on it.
            watch._note_audio(2.5)
            await watch._judge(2.5 + LINK_RESUME_SETTLE_SECS)
            await watch._judge(2.5 + LINK_RESUME_SETTLE_SECS + 0.5)
            after_stray = list(events)
            # Real flow: frames every 20 ms for the settle's length and a
            # little past it (integer steps, so float drift cannot stop the
            # clock a frame short of the settle).
            for i in range(int(LINK_RESUME_SETTLE_SECS / 0.02) + 3):
                t = 4.0 + i * 0.02
                watch._note_audio(t)
                await watch._judge(t)
            return paused, after_stray, events, pushed

        paused, after_stray, events, pushed = run(scenario())
        self.assertEqual(paused, [("paused", "audio")])
        self.assertEqual(after_stray, paused, "a single frame lifted the hold")
        self.assertEqual(events, [("paused", "audio"), ("ok", "audio")])
        self.assertEqual(pushed, ["LinkStalledFrame", "LinkResumedFrame"])


class PageHoldExpiry(unittest.TestCase):
    """Sixth review of #226: a hold the page asked for, whose `ok` was lost
    on a channel that was not open, must not latch for the life of the
    call. Unbroken audio for `LINK_PAGE_HOLD_EXPIRY_SECS` lifts it; audio with a
    break in it restarts the count."""

    def _watch(self, events):
        watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))

        async def push(frame, direction=None):
            pass

        watch.push_frame = push
        return watch

    def test_unbroken_audio_expires_a_link_hold(self):
        async def scenario():
            events = []
            watch = self._watch(events)
            watch._note_audio(0.0)
            await watch.hold("link", now=0.0)
            t = 0.0
            while t < LINK_PAGE_HOLD_EXPIRY_SECS + 0.3:
                watch._note_audio(t)
                await watch._judge(t)
                t += 0.02
            return events

        events = run(scenario())
        self.assertEqual(events, [("paused", "link"), ("ok", "page-expired")])

    def test_a_mic_hold_survives_unbroken_audio_and_lifts_on_speech(self):
        """Seventh review: a muted iOS track keeps sending silence, so
        audio flowing is no witness that the microphone is back. Speech is."""

        async def scenario():
            events = []
            watch = self._watch(events)
            watch._note_audio(0.0)
            await watch.hold("mic", now=0.0)
            t = 0.0
            while t < LINK_PAGE_HOLD_EXPIRY_SECS + 2.0:
                watch._note_audio(t)
                await watch._judge(t)
                t += 0.02
            after_audio = list(events)
            await watch._note_speech(0.3)  # queued audio segmenting just after the mute: not proof
            after_early_speech = list(events)
            await watch._note_speech(LINK_PAGE_HOLD_EXPIRY_SECS + 3.0)
            return after_audio, after_early_speech, events

        after_audio, after_early, events = run(scenario())
        self.assertEqual(after_audio, [("paused", "mic")], "silence from a muted track lifted a mic hold")
        self.assertEqual(after_early, [("paused", "mic")], "speech inside the settling time lifted the hold")
        self.assertEqual(events, [("paused", "mic"), ("ok", "page-expired")])

    def test_a_link_hold_expiring_does_not_lift_a_mic_hold_beside_it(self):
        """Eighth review: the page pauses for the link, then the mic is
        muted. The link's hold expires on unbroken audio; the mic's must
        not go with it, because the audio is the muted track's silence."""

        async def scenario():
            events = []
            watch = self._watch(events)
            watch._note_audio(0.0)
            await watch.hold("link", now=0.0)
            await watch.hold("mic", now=1.0)
            t = 0.0
            while t < LINK_PAGE_HOLD_EXPIRY_SECS + 2.0:
                watch._note_audio(t)
                await watch._judge(t)
                t += 0.02
            still_held = watch.held
            after_audio = list(events)
            await watch._note_speech(LINK_PAGE_HOLD_EXPIRY_SECS + 3.0)
            return still_held, after_audio, events

        still_held, after_audio, events = run(scenario())
        self.assertTrue(still_held, "the link hold's expiry lifted the mic hold with it")
        self.assertEqual(after_audio, [("paused", "link")])
        self.assertEqual(events, [("paused", "link"), ("ok", "page-expired")])

    def test_the_page_releases_one_reason_at_a_time(self):
        async def scenario():
            events = []
            watch = self._watch(events)
            await watch.hold("link", now=0.0)
            await watch.hold("mic", now=0.0)
            await watch.release("link")
            held_after_one = watch.held
            await watch.release("mic")
            return held_after_one, watch.held, events

        held_after_one, held, events = run(scenario())
        self.assertTrue(held_after_one, "releasing one reason released the other")
        self.assertFalse(held)
        self.assertEqual(events, [("paused", "link"), ("ok", "page")])

    def test_a_break_in_the_audio_restarts_the_count(self):
        async def scenario():
            events = []
            watch = self._watch(events)
            watch._note_audio(0.0)
            await watch.hold("link", now=0.0)
            t = 0.0
            while t < LINK_PAGE_HOLD_EXPIRY_SECS + 0.3:
                if 5.0 < t < 5.5:  # half a second of nothing
                    await watch._judge(t)
                else:
                    watch._note_audio(t)
                    await watch._judge(t)
                t += 0.02
            return events

        events = run(scenario())
        self.assertEqual(events, [("paused", "link")], "a broken flow counted as unbroken")


class LoopStalls(unittest.TestCase):
    """The sampler names the frame the loop is stuck in, from a thread,
    while it is stuck — not after."""

    def test_a_blocked_loop_is_caught_with_its_stack(self):
        async def scenario():
            sampler = LoopSampler.start()
            await asyncio.sleep(0.3)  # let the heartbeat run
            time.sleep(LOOP_LAG_WARN_SECS + 0.4)  # the fault: blocking the loop
            await asyncio.sleep(0.4)  # the beat resumes; the duration line follows
            return sampler.stalls

        stalls = run(scenario())
        self.assertEqual(len(stalls), 1, f"expected one stall, saw {len(stalls)}")
        late, stack = stalls[0]
        self.assertGreaterEqual(late, LOOP_LAG_WARN_SECS)
        self.assertIn("time.sleep", stack, "the stack does not name the blocking call")
        self.assertIn("scenario", stack)


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
        self.assertFalse(any(isinstance(f, SegmentDroppedFrame) for f in frames))

    def test_a_segment_with_no_words_says_so(self):
        """The base service emits nothing for empty text; the turn logic
        needs the segment accounted for either way."""
        stt = ParakeetSTT(api_key="unused", base_url="http://127.0.0.1:1/v1")

        async def fake(audio):
            return Transcription(text="")

        stt._transcribe = fake

        async def scenario():
            return [f async for f in stt.run_stt(b"wav")]

        frames = run(scenario())
        self.assertFalse(any(isinstance(f, TranscriptionFrame) for f in frames))
        self.assertEqual(sum(isinstance(f, SegmentDroppedFrame) for f in frames), 1)

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
