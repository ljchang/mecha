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
    a = ap.parse_args()
    sys.exit(asyncio.run(call(a.wav, a.offer, a.seconds, a.uplink, a.stall_at, a.stall_secs, a.deaf)))
