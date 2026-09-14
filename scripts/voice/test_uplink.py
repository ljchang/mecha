"""The reliable uplink (docs/VOICE-LINK-DESIGN.md), driven without a browser
or a pipeline: the caught-up rule on `LinkWatch`, the lane boundary, the
late prefix, and the injector end to end with real Opus packets and fakes
for the transport input and the STT server. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_uplink.py`."""

import asyncio
import base64
import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from worker import (  # noqa: E402
    BACKLOG_TALK_SECS,
    LINK_CAUGHT_UP_MS,
    LINK_RESUME_SETTLE_SECS,
    LINK_STALL_SECS,
    UPLINK_DEAF_SECS,
    LinkWatch,
    UplinkAudio,
    _wav16,
    deaf_verdict,
    lane_for,
    late_prefix,
)


def run(coro):
    return asyncio.run(coro)


async def _record(events, state, reason):
    events.append((state, reason))


def opus_batch(seq, ms, backlog_ms, n_frames=5, dropped=None, wall_ms=None):
    """`n_frames` 20 ms Opus packets of a 440 Hz tone, as the page sends them."""
    import av
    import numpy as np

    enc = av.CodecContext.create("libopus", "w")
    enc.sample_rate, enc.layout, enc.format = 48000, "mono", "s16"
    t = np.arange(0, 960 * (n_frames + 1)) / 48000.0
    pcm = (np.sin(2 * np.pi * 440 * t) * 8000).astype(np.int16)
    packets = []
    for i in range(0, len(pcm), 960):
        f = av.AudioFrame.from_ndarray(pcm[i : i + 960].reshape(1, -1), format="s16", layout="mono")
        f.sample_rate, f.pts = 48000, i
        packets += enc.encode(f)
    packets += enc.encode(None)
    frames = [[20, base64.b64encode(bytes(p)).decode()] for p in packets[:n_frames]]
    d = {"seq": seq, "ms": ms, "backlog_ms": backlog_ms, "frames": frames}
    d["wall_ms"] = WALL0 + ms if wall_ms is None else wall_ms
    if dropped:
        d["dropped"] = dropped
    return d


# 2026-09-14 12:22:00 UTC, the page's wall clock at media zero.
WALL0 = 1_789_388_520_000


class FakeInput:
    def __init__(self):
        self.audio, self.frames, self.order = [], [], []
        self.batches, self.rtp_fallback = 0, False

    def note_batch(self):
        self.batches += 1

    async def push_audio_frame(self, frame):
        self.audio.append(frame)
        self.order.append("audio")

    async def push_frame(self, frame, direction=None):
        self.frames.append(frame)
        self.order.append(type(frame).__name__)


class FakeSTT:
    """Only what the injector touches: the bot-speaking flag and the
    transcriptions client."""

    _bot_speaking = False

    def __init__(self, text="lab meeting at noon"):
        stt = self
        stt.requests = []

        class _Create:
            async def create(self, model, file):
                stt.requests.append(file[1])

                class R:
                    pass

                r = R()
                r.text = text
                return r

        class _Audio:
            transcriptions = _Create()

        class _Client:
            audio = _Audio()

        self._client = _Client()


class SlowSTT(FakeSTT):
    """Transcription that waits until the test lets it finish — the window
    in which a second late batch used to cancel the turn."""

    def __init__(self, text):
        super().__init__(text)
        self.gate = asyncio.Event()
        self.started = asyncio.Event()
        stt = self

        class _Create:
            async def create(self, model, file):
                stt.requests.append(file[1])
                stt.started.set()
                await stt.gate.wait()

                class R:
                    pass

                r = R()
                r.text = text
                return r

        class _Audio:
            transcriptions = _Create()

        class _Client:
            audio = _Audio()

        self._client = _Client()


class NothingIsCancelled(unittest.TestCase):
    """Review of #231: the settle timer used to *be* the flush after its
    sleep, so a late batch arriving during transcription cancelled the turn
    with the audio already moved into locals. Now the timer only enqueues,
    one consumer delivers in order, and a batch mid-flush starts the next
    span."""

    def test_a_late_batch_during_a_flush_does_not_lose_the_turn(self):
        async def scenario():
            inp, stt = FakeInput(), SlowSTT("first span")
            up = UplinkAudio(inp, LinkWatch(), stt)
            await up.on_start({"tz_offset_min": 0})
            await up.on_audio(opus_batch(0, 0, 130_000, n_frames=25))
            # The span closes as production closes it: a flush enqueued
            # (here directly, rather than by the settle timer's sleep).
            up._work.put_nowait(up.flush_late)
            await asyncio.wait_for(stt.started.wait(), 2.0)  # transcription is in flight
            # Mid-flush: another late batch. This used to cancel the flush.
            await up.on_audio(opus_batch(1, 500, 129_500, n_frames=25))
            stt.gate.set()
            up._work.put_nowait(up.flush_late)
            await asyncio.wait_for(up.drain(), 5.0)
            return inp, up

        inp, up = run(scenario())
        self.assertEqual(up.late_turns, 2, "a span was lost")
        self.assertEqual([f.text.endswith("first span") for f in inp.frames], [True, True])

    def test_live_audio_queues_behind_the_turn_it_closed(self):
        async def scenario():
            inp, stt = FakeInput(), SlowSTT("what I said before")
            up = UplinkAudio(inp, LinkWatch(), stt)
            await up.on_start({"tz_offset_min": 0})
            await up.on_audio(opus_batch(0, 0, 130_000, n_frames=25))
            await up.on_audio(opus_batch(1, 500, 0))  # live: enqueues flush, then audio
            await asyncio.wait_for(stt.started.wait(), 2.0)
            before = list(inp.order)
            stt.gate.set()
            await up.drain()
            return before, inp

        before, inp = run(scenario())
        self.assertEqual(before, [], "live audio was pushed while its predecessor was still transcribing")
        self.assertEqual(inp.order[0], "TranscriptionFrame")
        self.assertGreater(len(inp.audio), 0)


class ChunkTail(unittest.TestCase):
    """A span just past a chunk boundary keeps its last word."""

    def test_a_short_tail_joins_the_piece_before_it(self):
        async def scenario():
            inp, stt = FakeInput(), FakeSTT("words")
            up = UplinkAudio(inp, LinkWatch(), stt)
            long = b"\x00\x00" * int(16000 * 60.2)  # 60.2 s
            await up._transcribe(long)
            sizes = [len(r) - 44 for r in stt.requests]
            stt.requests.clear()
            await up._transcribe(b"\x00\x00" * int(16000 * 0.2))  # under the floor: nothing
            return sizes, len(stt.requests)

        sizes, short = run(scenario())
        self.assertEqual(sizes, [int(16000 * 60.2) * 2], "the 0.2 s tail was not joined to its chunk")
        self.assertEqual(short, 0)


class CaughtUp(unittest.TestCase):
    """§2.4: "resumed" means caught up. Flow alone must not lift the hold
    while the page still holds audio it has not sent."""

    def test_flow_with_a_backlog_stays_held_until_the_backlog_drains(self):
        async def scenario():
            events = []
            watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))

            async def push(frame, direction=None):
                pass

            watch.push_frame = push
            watch.note_uplink(0.0, 0)
            await watch._judge(0.0 + LINK_STALL_SECS + 0.05)  # nothing arrived: stalled
            self.assertEqual(events, [("paused", "audio")])
            # The drain: batches every 100 ms, the page still 20 s behind.
            t = 5.0
            while t < 5.0 + LINK_RESUME_SETTLE_SECS + 1.0:
                watch.note_uplink(t, 20_000)
                await watch._judge(t)
                t += 0.1
            still_held = watch.held
            # Caught up: the backlog is under the threshold, flow continues.
            while t < 10.0 + LINK_RESUME_SETTLE_SECS + 1.0:
                watch.note_uplink(t, LINK_CAUGHT_UP_MS)
                await watch._judge(t)
                t += 0.1
            return events, still_held, watch.held

        events, still_held, held_after = run(scenario())
        self.assertTrue(still_held, "flow with a 20 s backlog lifted the hold")
        self.assertFalse(held_after)
        self.assertEqual(events, [("paused", "audio"), ("ok", "audio")])

    def test_an_rtp_call_resumes_on_flow_alone(self):
        async def scenario():
            events = []
            watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))

            async def push(frame, direction=None):
                pass

            watch.push_frame = push
            watch._note_audio(0.0)
            await watch._judge(LINK_STALL_SECS + 0.05)
            t = 3.0
            while t < 3.0 + LINK_RESUME_SETTLE_SECS + 0.5:
                watch._note_audio(t)
                await watch._judge(t)
                t += 0.1
            return events

        self.assertEqual(run(scenario()), [("paused", "audio"), ("ok", "audio")])


class Lanes(unittest.TestCase):
    def test_the_boundary_is_the_constant(self):
        edge = int(BACKLOG_TALK_SECS * 1000)
        self.assertEqual(lane_for(edge - 1), "live")
        self.assertEqual(lane_for(edge), "late")
        self.assertEqual(lane_for(0), "live")


class LatePrefix(unittest.TestCase):
    def test_the_phones_own_clock_and_what_was_lost(self):
        # 2026-09-14 12:22:00 UTC, on a phone at UTC-4: 08:22 local.
        p = late_prefix(WALL0, WALL0 + 60_000, -240, 0)
        self.assertEqual(p, "(said 08:22–08:23, delivered late) ")
        p = late_prefix(WALL0, WALL0 + 60_000, -240, 45_000)
        self.assertIn("about 45 seconds before this were lost", p)
        self.assertTrue(p.endswith(") "))
        # No clock from the page: honest, not invented.
        self.assertEqual(late_prefix(None, None, 0, 0), "(said earlier–earlier, delivered late) ")


class Fallback(unittest.TestCase):
    """The watch that rescues a deaf call must not brick a live one
    (review of #231, three findings on one path)."""

    def test_the_witness_is_rtp_and_the_channel_alive_while_no_batch_arrives(self):
        # A stalled link stops everything: not deaf.
        self.assertFalse(deaf_verdict(False, False, UPLINK_DEAF_SECS + 5))
        # A head-of-line-blocked channel: RTP flows, nothing at all on the
        # channel — a lossy link, not a dead tap (third review of #231).
        self.assertFalse(deaf_verdict(True, False, UPLINK_DEAF_SECS + 5))
        # RTP flowing, channel alive, batches recent: not deaf.
        self.assertFalse(deaf_verdict(True, True, 1.0))
        # RTP flowing, heartbeats arriving, no audio for the window: deaf.
        self.assertTrue(deaf_verdict(True, True, UPLINK_DEAF_SECS))

    def test_a_late_lane_backlog_is_a_batch_for_the_witness(self):
        async def scenario():
            inp = FakeInput()
            up = UplinkAudio(inp, LinkWatch(), FakeSTT())
            await up.on_start({"tz_offset_min": 0})
            for seq in range(5):
                await up.on_audio(opus_batch(seq, seq * 100, 200_000))
            return inp.batches, len(inp.audio)

        batches, pushed = run(scenario())
        self.assertEqual(batches, 5, "late batches must count as arrivals or a reconnect trips the fallback")
        self.assertEqual(pushed, 0)

    def test_after_the_fallback_flow_alone_resumes_and_batches_are_ignored(self):
        async def scenario():
            events = []
            watch = LinkWatch(on_change=lambda s, r, a: _record(events, s, r))

            async def push(frame, direction=None):
                pass

            watch.push_frame = push
            inp = FakeInput()
            up = UplinkAudio(inp, watch, FakeSTT())
            # A backlog was reported, then the link went quiet: held.
            watch.note_uplink(0.0, 200_000)
            await watch._judge(LINK_STALL_SECS + 0.05)
            # The fallback engages: the sticky backlog must go with it.
            inp.rtp_fallback = True
            watch.forget_backlog()
            # RTP audio flows; nothing else ever updates the backlog.
            t = 3.0
            while t < 3.0 + LINK_RESUME_SETTLE_SECS + 0.5:
                watch._note_audio(t)
                await watch._judge(t)
                t += 0.1
            # Batches that still arrive are counted for the witness and ignored.
            before = watch._backlog_ms
            await up.on_audio(opus_batch(0, 0, 150_000))
            await up.drain()
            return events, inp.batches, len(inp.audio), before, watch._backlog_ms

        events, batches, pushed, before, after = run(scenario())
        self.assertEqual(events, [("paused", "audio"), ("ok", "audio")], "the fallback left the turn held")
        self.assertEqual(batches, 1)
        self.assertEqual(pushed, 0, "a batch after the fallback doubled the voice")
        self.assertIsNone(before)
        self.assertIsNone(after, "an ignored batch must not resurrect the backlog")


class AcrossAGap(unittest.TestCase):
    """The ring survives the end of a call; the prefix must place what it
    held at the time it was said, not at the reconnect."""

    def test_the_prefix_uses_the_frames_own_wall_clock(self):
        async def scenario():
            inp, stt = FakeInput(), FakeSTT("what I said before the gap")
            up = UplinkAudio(inp, LinkWatch(), stt)
            await up.on_start({"tz_offset_min": -240})
            # Captured at 08:19 local, delivered at a reconnect three
            # minutes later with the media clock still near zero.
            said_at = WALL0 - 180_000
            await up.on_audio(opus_batch(0, 0, 200_000, n_frames=25, wall_ms=said_at))
            await up.on_audio(opus_batch(1, 500, 199_500, n_frames=25, wall_ms=said_at + 500))
            await up.flush_late()
            await up.drain()
            return inp

        inp = run(scenario())
        self.assertEqual(len(inp.frames), 1)
        self.assertTrue(inp.frames[0].text.startswith("(said 08:19–08:19, delivered late) "), inp.frames[0].text)


class Injection(unittest.TestCase):
    """Real Opus packets through the real decoder into fakes on both ends."""

    def test_live_batches_become_20ms_frames_with_nothing_dropped(self):
        async def scenario():
            inp, stt = FakeInput(), FakeSTT()
            link = LinkWatch()
            up = UplinkAudio(inp, link, stt)
            await up.on_start({"tz_offset_min": 0})
            for seq in range(10):
                await up.on_audio(opus_batch(seq, seq * 100, 0))
            await up.drain()
            return inp, up

        inp, up = run(scenario())
        # Ten batches of 100 ms: fifty frames, give or take the resampler's
        # priming — the tail carry means never fewer than 48.
        self.assertGreaterEqual(len(inp.audio), 48, len(inp.audio))
        self.assertLessEqual(len(inp.audio), 50)
        self.assertTrue(all(len(f.audio) == 640 and f.sample_rate == 16000 for f in inp.audio))
        self.assertEqual(inp.frames, [])
        self.assertEqual(up.late_turns, 0)

    def test_a_late_span_is_one_turn_put_before_the_speech_that_ended_it(self):
        async def scenario():
            inp, stt = FakeInput(), FakeSTT("call the dentist tomorrow")
            link = LinkWatch()
            up = UplinkAudio(inp, link, stt)
            await up.on_start({"tz_offset_min": -240})
            # Twenty batches from two minutes behind: late, no frames pushed.
            for seq in range(20):
                await up.on_audio(opus_batch(seq, seq * 100, 130_000 - seq * 100))
            await up.drain()
            pushed_during_late = len(inp.audio)
            # A live batch: the span closes first, then the live audio flows.
            await up.on_audio(opus_batch(20, 2000, 0))
            await up.drain()
            return inp, stt, up, pushed_during_late

        inp, stt, up, pushed_during_late = run(scenario())
        self.assertEqual(pushed_during_late, 0, "late audio reached the live pipeline")
        self.assertEqual(up.late_turns, 1)
        self.assertEqual(len(inp.frames), 1)
        text = inp.frames[0].text
        self.assertTrue(text.startswith("(said 08:22–08:22, delivered late) "), text)
        self.assertTrue(text.endswith("call the dentist tomorrow"))
        self.assertTrue(inp.frames[0].finalized)
        self.assertEqual(inp.order[0], "TranscriptionFrame", "the late turn must precede the live audio")
        self.assertGreater(len(inp.audio), 0)
        self.assertEqual(len(stt.requests), 1)
        self.assertTrue(stt.requests[0].startswith(b"RIFF"))

    def test_a_dropped_span_is_named_on_the_turn_that_follows_it(self):
        async def scenario():
            inp, stt = FakeInput(), FakeSTT("and the second thing")
            up = UplinkAudio(inp, LinkWatch(), stt)
            await up.on_start({"tz_offset_min": 0})
            await up.on_audio(
                opus_batch(0, 300_000, 200_000, n_frames=25, dropped=[{"from_ms": 0, "to_ms": 45_000}])
            )
            await up.flush_late()
            await up.drain()
            return inp

        inp = run(scenario())
        self.assertEqual(len(inp.frames), 1)
        self.assertIn("about 45 seconds before this were lost", inp.frames[0].text)

    def test_the_wav_header_is_what_the_stt_server_reads(self):
        wav = _wav16(b"\x00\x00" * 160)
        self.assertEqual(wav[:4], b"RIFF")
        self.assertEqual(wav[8:12], b"WAVE")
        self.assertEqual(len(wav), 44 + 320)


if __name__ == "__main__":
    unittest.main()
