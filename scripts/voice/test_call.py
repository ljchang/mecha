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


async def call(wav: str, offer_url: str, seconds: float) -> int:
    pc = RTCPeerConnection()
    player = MediaPlayer(wav)
    pc.addTrack(player.audio)
    pc.addTransceiver("audio", direction="recvonly")

    got: list[str] = []
    dc = pc.createDataChannel("rtvi")

    @dc.on("open")
    def _open():
        dc.send(json.dumps({"label": "rtvi-ai", "type": "client-ready", "id": "test-call"}))

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
            json={"sdp": pc.localDescription.sdp, "type": pc.localDescription.type},
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
    a = ap.parse_args()
    sys.exit(asyncio.run(call(a.wav, a.offer, a.seconds)))
