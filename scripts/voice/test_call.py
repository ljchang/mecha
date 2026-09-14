#!/usr/bin/env python3
"""A headless phone call: stream a wav as the microphone through the full
WebRTC -> VAD -> STT -> facade -> TTS path and print what comes back.

The repeatable stand-in for "Luke tries it from his iPhone" - built the
night the STT leg went silent in production while every curl-level test
passed, because the only layer nothing exercised was the one between the
browser's mic and the worker's segments.

Usage: test_call.py [wav] [--offer URL] [--seconds N]
"""
import argparse
import asyncio
import json
import sys

import aiohttp
from aiortc import RTCPeerConnection, RTCSessionDescription
from aiortc.contrib.media import MediaPlayer


def opus_packets(wav: str):
    """The wav as 20 ms Opus packets at 48 kHz mono — what the page's tap
    hands it, frame for frame."""
    import av

    enc = av.CodecContext.create("libopus", "w")
    enc.sample_rate, enc.layout, enc.format = 48000, "mono", "s16"
    rs = av.AudioResampler(format="s16", layout="mono", rate=48000)
    out = []
    with av.open(wav) as f:
        for frame in f.decode(audio=0):
            for r in rs.resample(frame):
                r.pts = None
                out += [bytes(p) for p in enc.encode(r)]
    # Five seconds of encoded silence after the speech, as a live
    # microphone would keep sending: smart-turn ends a turn on 3 s of
    # silence in the *audio*, and a stream that simply stops — or stops at
    # exactly 3 s — leaves it to the 15 s wall-clock timeout, which
    # measures the client, not the worker.
    import numpy as np

    quiet = av.AudioFrame.from_ndarray(np.zeros((1, 960), dtype=np.int16), format="s16", layout="mono")
    quiet.sample_rate = 48000
    for _ in range(250):
        quiet.pts = None
        out += [bytes(p) for p in enc.encode(quiet)]
    out += [bytes(p) for p in enc.encode(None)]
    return out


async def uplink(dc, packets, stall_at: float, stall_secs: float, deaf: bool = False):
    """Speak the page's uplink protocol (docs/VOICE-LINK-DESIGN.md §2.2):
    `audio-start`, then 100 ms batches paced in real time — except across a
    stall, during which capture continues (the clock advances, nothing is
    sent) and after which the backlog is delivered as fast as the channel
    takes it, each batch saying how far behind the ring still is."""
    import base64
    import time

    def send(t, d):
        dc.send(json.dumps({"label": "rtvi-ai", "type": "client-message", "id": "test-call", "data": {"t": t, "d": d}}))

    send("audio-start", {"tz_offset_min": 0})
    last_beat = 0.0

    def heartbeat():
        # The page's proof of life on the channel, every two seconds — not
        # during a stall, which blocks the whole channel, heartbeats too.
        nonlocal last_beat
        if time.monotonic() - last_beat >= 2.0:
            last_beat = time.monotonic()
            send("heartbeat", {})

    if deaf:
        # A page whose tap never delivers: the channel declared and alive,
        # no batch ever sent. The worker's watchdog must read RTP after all.
        print("uplink: declared and deaf — heartbeats only, no batches", flush=True)
        while dc.readyState == "open":
            heartbeat()
            await asyncio.sleep(0.2)
        return
    t0 = time.monotonic()
    wall0 = int(time.time() * 1000)
    seq, i, stalled_until = 0, 0, None
    while i < len(packets):
        captured_ms = (time.monotonic() - t0) * 1000
        sent_ms = i * 20
        if stall_at is not None and sent_ms >= stall_at * 1000 and stalled_until is None:
            stalled_until = time.monotonic() + stall_secs
            print(f"uplink: stalling for {stall_secs}s at {sent_ms / 1000:.1f}s", flush=True)
        if stalled_until is not None and time.monotonic() < stalled_until:
            await asyncio.sleep(0.05)
            continue
        heartbeat()
        if captured_ms < sent_ms + 100:
            await asyncio.sleep(0.02)
            continue
        batch = packets[i : i + 5]
        backlog_ms = max(0, int(captured_ms - (sent_ms + len(batch) * 20)))
        send("audio", {"seq": seq, "ms": sent_ms, "wall_ms": wall0 + sent_ms, "backlog_ms": backlog_ms,
                       "frames": [[20, base64.b64encode(p).decode()] for p in batch]})
        seq += 1
        i += len(batch)
    print(f"uplink: {seq} batches sent", flush=True)


async def carried_uplink(dc, packets, from_ms: int, live_from_ms: int, wall0: int):
    """The second connection of a reconnect (docs/VOICE-LINK-DESIGN.md
    §4.2): `audio-start` names where this connection's own capture begins,
    the ring's carried audio goes first — late whatever its backlog — and
    the live tail follows, paced."""
    import base64
    import time

    def send(t, d):
        dc.send(json.dumps({"label": "rtvi-ai", "type": "client-message", "id": "test-call", "data": {"t": t, "d": d}}))

    send("audio-start", {"tz_offset_min": 0, "live_from_ms": live_from_ms})
    last_beat = 0.0
    seq = 0
    i = from_ms // 20
    t0 = time.monotonic() - live_from_ms / 1000  # the clock as if capture never stopped
    while i < len(packets):
        if time.monotonic() - last_beat >= 2.0:
            last_beat = time.monotonic()
            send("heartbeat", {})
        sent_ms = i * 20
        captured_ms = (time.monotonic() - t0) * 1000
        if sent_ms >= live_from_ms and captured_ms < sent_ms + 100:
            await asyncio.sleep(0.02)
            continue
        batch = packets[i : i + 5]
        backlog_ms = max(0, int(captured_ms - (sent_ms + len(batch) * 20)))
        send("audio", {"seq": seq, "ms": sent_ms, "wall_ms": wall0 + sent_ms, "backlog_ms": backlog_ms,
                       "frames": [[20, base64.b64encode(p).decode()] for p in batch]})
        seq += 1
        i += len(batch)
    print(f"uplink (reconnected): {seq} batches sent, carried from {from_ms} ms, live from {live_from_ms} ms", flush=True)


async def connect(offer_url: str, wav: str, use_uplink: bool, on_open, got: list[str]):
    pc = RTCPeerConnection()
    player = MediaPlayer(wav)
    pc.addTrack(player.audio)
    pc.addTransceiver("audio", direction="recvonly")
    dc = pc.createDataChannel("rtvi")

    @dc.on("open")
    def _open():
        dc.send(json.dumps({"label": "rtvi-ai", "type": "client-ready", "id": "test-call"}))
        on_open(dc)

    @dc.on("message")
    def _msg(m):
        try:
            ev = json.loads(m)
        except Exception:
            return
        t = ev.get("type", "")
        if t in ("user-transcription", "bot-transcription"):
            print(f"{t}: {ev['data'].get('text', '')!r} final={ev['data'].get('final')}", flush=True)
            if t == "user-transcription" and ev["data"].get("final"):
                got.append(ev["data"]["text"])
        elif t == "server-message" and isinstance(ev.get("data"), dict) and ev["data"].get("t") == "late-turn":
            print(f"late-turn: {ev['data'].get('text', '')!r}", flush=True)
            got.append(ev["data"].get("text", ""))

    offer = await pc.createOffer()
    await pc.setLocalDescription(offer)
    while pc.iceGatheringState != "complete":
        await asyncio.sleep(0.1)
    async with aiohttp.ClientSession() as http:
        async with http.post(
            offer_url,
            json={"sdp": pc.localDescription.sdp, "type": pc.localDescription.type,
                  **({"request_data": {"uplink": "channel"}} if use_uplink else {})},
        ) as resp:
            if resp.status != 200:
                print(f"offer refused: {resp.status}", flush=True)
                return None
            answer = await resp.json()
    await pc.setRemoteDescription(RTCSessionDescription(sdp=answer["sdp"], type=answer["type"]))
    return pc


async def reconnect_call(wav: str, offer_url: str, drop_at: float, gap: float, seconds: float) -> int:
    """A cellular drop: the first connection dies `drop_at` seconds in,
    capture continues for `gap` seconds with no connection at all, and a
    second connection delivers what the ring held before its own live
    audio. Two peer connections, one process, the ring carried across."""
    import time

    packets = opus_packets(wav)
    got: list[str] = []
    wall0 = int(time.time() * 1000)

    def first_open(dc):
        asyncio.ensure_future(uplink(dc, packets[: int(drop_at * 50)], None, 0.0))

    pc = await connect(offer_url, wav, True, first_open, got)
    if pc is None:
        return 2
    await asyncio.sleep(drop_at + 1.0)
    print(f"dropping the connection at {drop_at}s; capture continues for {gap}s", flush=True)
    await pc.close()
    await asyncio.sleep(gap)
    from_ms, live_from_ms = int(drop_at * 1000), int((drop_at + gap) * 1000)

    def second_open(dc):
        asyncio.ensure_future(carried_uplink(dc, packets, from_ms, live_from_ms, wall0))

    pc = await connect(offer_url, wav, True, second_open, got)
    if pc is None:
        return 2
    await asyncio.sleep(seconds)
    await pc.close()
    print(f"final user transcripts: {got}", flush=True)
    return 0 if got else 1


async def call(wav: str, offer_url: str, seconds: float, use_uplink: bool = False,
               stall_at: float | None = None, stall_secs: float = 6.0, deaf: bool = False) -> int:
    pc = RTCPeerConnection()
    player = MediaPlayer(wav)
    pc.addTrack(player.audio)
    pc.addTransceiver("audio", direction="recvonly")

    got: list[str] = []
    dc = pc.createDataChannel("rtvi")
    packets = opus_packets(wav) if use_uplink else None

    @dc.on("open")
    def _open():
        dc.send(json.dumps({"label": "rtvi-ai", "type": "client-ready", "id": "test-call"}))
        if use_uplink:
            asyncio.ensure_future(uplink(dc, packets, stall_at, stall_secs, deaf))

    @dc.on("message")
    def _msg(m):
        try:
            ev = json.loads(m)
        except Exception:
            return
        t = ev.get("type", "")
        if t in ("user-transcription", "bot-transcription"):
            line = f"{t}: {ev['data'].get('text', '')!r} final={ev['data'].get('final')}"
            print(line, flush=True)
            if t == "user-transcription" and ev["data"].get("final"):
                got.append(ev["data"]["text"])
        elif t in ("user-started-speaking", "user-stopped-speaking", "bot-llm-started",
                   "bot-started-speaking", "error"):
            print(f"event: {t}", flush=True)
        elif t == "server-message" and isinstance(ev.get("data"), dict) and ev["data"].get("t") == "late-turn":
            print(f"late-turn: {ev['data'].get('text', '')!r}", flush=True)
            got.append(ev["data"].get("text", ""))

    offer = await pc.createOffer()
    await pc.setLocalDescription(offer)
    while pc.iceGatheringState != "complete":
        await asyncio.sleep(0.1)
    async with aiohttp.ClientSession() as http:
        async with http.post(
            offer_url,
            json={"sdp": pc.localDescription.sdp, "type": pc.localDescription.type,
                  **({"request_data": {"uplink": "channel"}} if use_uplink else {})},
        ) as resp:
            if resp.status != 200:
                print(f"offer refused: {resp.status}", flush=True)
                return 2
            answer = await resp.json()
    await pc.setRemoteDescription(RTCSessionDescription(sdp=answer["sdp"], type=answer["type"]))
    await asyncio.sleep(seconds)
    await pc.close()
    print(f"final user transcripts: {got}", flush=True)
    return 0 if got else 1


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("wav", nargs="?", default="jfk-phone.wav")
    ap.add_argument("--offer", default="http://127.0.0.1:7860/api/offer")
    ap.add_argument("--seconds", type=float, default=25.0)
    ap.add_argument("--uplink", action="store_true", help="speak the buffered-uplink protocol")
    ap.add_argument("--stall-at", type=float, default=None, help="seconds into the wav to stall the uplink")
    ap.add_argument("--stall-secs", type=float, default=6.0)
    ap.add_argument("--deaf", action="store_true", help="declare the channel, then send no batches")
    ap.add_argument("--reconnect-at", type=float, default=None, help="drop the connection this many seconds in")
    ap.add_argument("--reconnect-gap", type=float, default=8.0, help="seconds with no connection before reconnecting")
    a = ap.parse_args()
    if a.reconnect_at is not None:
        sys.exit(asyncio.run(reconnect_call(a.wav, a.offer, a.reconnect_at, a.reconnect_gap, a.seconds)))
    sys.exit(asyncio.run(call(a.wav, a.offer, a.seconds, a.uplink, a.stall_at, a.stall_secs, a.deaf)))
