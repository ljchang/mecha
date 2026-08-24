#!/usr/bin/env python3
"""The mecha voice worker: the Pipecat pipeline between a browser and the
facade. Deliberately dumb - pipeline config and transport, no judgment;
everything with judgment lives behind `mecha voice-serve` (D2).

Run inside the voice-worker venv via the pipecat runner:
    python worker.py --host 127.0.0.1 --port 7860
then publish it inside the tailnet with `tailscale serve` (D1) - the
browser needs HTTPS before it will open a microphone.

The three legs are env-configurable base URLs (D6):
    MECHA_VOICE_LLM   the facade        (default http://127.0.0.1:8990/v1)
    MECHA_VOICE_STT_KIND  parakeet (default) | voxtral - picks the STT leg
    MECHA_VOICE_STT   the STT leg       (default follows STT_KIND:
                      :8992 parakeet, :8082 voxtral)
    MECHA_VOICE_TTS   Kokoro/Chatterbox (default http://127.0.0.1:8880/v1
                      Kokoro; the shipped unit overrides to :8881
                      Chatterbox Turbo, which is the launch voice)
    MECHA_VOICE_TTS_VOICE  voice name for the TTS leg (start value; the
                      page can change it per session)
    MECHA_VOICE_TTS_SPEED  speaking rate, 0.5-2.0 (start value, likewise)
"""

import base64
import os
import uuid

from openai.types.audio import Transcription

from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.worker import PipelineParams, PipelineWorker
from pipecat.processors.aggregators.llm_context import LLMContext
from pipecat.processors.aggregators.llm_response_universal import (
    LLMContextAggregatorPair,
    LLMUserAggregatorParams,
)
from pipecat.processors.frameworks.rtvi.observer import RTVIObserver
from pipecat.processors.frameworks.rtvi.processor import RTVIProcessor
from pipecat.runner.types import RunnerArguments
from pipecat.services.openai.llm import OpenAILLMService
from pipecat.services.openai.tts import OpenAITTSService
from pipecat.services.whisper.base_stt import BaseWhisperSTTService
from pipecat.transports.base_transport import BaseTransport, TransportParams
from pipecat.transports.smallwebrtc.connection import SmallWebRTCConnection
from pipecat.transports.smallwebrtc.transport import SmallWebRTCTransport

FACADE_URL = os.environ.get("MECHA_VOICE_LLM", "http://127.0.0.1:8990/v1")
# parakeet (default): the transducer - cannot answer, obey, or improvise.
# voxtral: the chat model, kept for audio-understanding turns; as a
# transcriber it answered question-shaped speech and obeyed spoken
# instructions ("say the word banana" -> "banana"), so it lost the seat.
STT_KIND = os.environ.get("MECHA_VOICE_STT_KIND", "parakeet")
STT_URL = os.environ.get(
    "MECHA_VOICE_STT",
    "http://127.0.0.1:8992/v1" if STT_KIND == "parakeet" else "http://127.0.0.1:8082/v1",
)
TTS_URL = os.environ.get("MECHA_VOICE_TTS", "http://127.0.0.1:8880/v1")
TTS_VOICE = os.environ.get("MECHA_VOICE_TTS_VOICE", "af_heart")
TTS_SPEED = float(os.environ.get("MECHA_VOICE_TTS_SPEED", "1.0"))
# Bounds mirror the TTS server's own. Duplicated rather than fetched
# because they gate a value before it is sent: a slider that can ask for
# 4x and learn it was refused mid-sentence is a worse control than one
# that cannot ask.
MIN_SPEED, MAX_SPEED = 0.5, 2.0

# Pinned per the build log (docs/VOICE-RESEARCH.md S7): this wording
# transcribes; "from beginning to end" phrasing makes the model refuse, and
# naming an escape token ("output NOSPEECH") anchors the model into
# answering it for real speech too - both found by testing. Treat any
# rewording as a change to test, not a paraphrase. The trailing sentence
# was tested against jfk.wav (unchanged) and silence/noise (degrades the
# reply to droppable fragments instead of chat).
TRANSCRIBE_PROMPT = (
    "Transcribe this audio exactly. Output only the transcription. "
    "If there are no words, output nothing."
)

# Segments quieter than this never reach the model. Voxtral is a chat
# model: handed silence or echo residue it stops transcribing and starts
# *answering* ("I'm an AI and don't have a calendar"), and that answer
# then rides into mecha as the owner's words - the observed 2026-08-24
# bug. Measured on this STT server: speech ~0.14 RMS, room noise ~0.009,
# silence 0.000; the gate sits in the gap.
MIN_SEGMENT_RMS = 0.010
MIN_SEGMENT_SECONDS = 0.3
# A reply longer than speech allows is a hallucination, not a transcript.
MAX_WORDS_PER_SECOND = 5.0


# What the bot said recently, normalized, for the echo filter: a phone on
# speaker hears its own TTS, and when client-side echo cancellation fails
# (a known WebKit trap once WebAudio taps the mic track), the bot's words
# come back as the owner's. Text is the one signal that survives every
# acoustic path: if the transcript is contained in what the bot just said,
# it is the speaker, not the speaker's owner.
import collections
import re
import time as _time

RECENT_BOT_SPEECH: collections.deque = collections.deque(maxlen=12)
ECHO_WINDOW_SECONDS = 20.0


def _normalize(text: str) -> str:
    return re.sub(r"[^a-z0-9 ]", "", text.lower()).strip()


def note_bot_speech(text: str) -> None:
    RECENT_BOT_SPEECH.append((_time.monotonic(), _normalize(text)))


def is_probable_echo(transcript: str) -> bool:
    norm = _normalize(transcript)
    if len(norm) < 8:
        return False
    now = _time.monotonic()
    for stamp, spoken in RECENT_BOT_SPEECH:
        if now - stamp < ECHO_WINDOW_SECONDS and norm in spoken:
            return True
    return False


class VoxtralSTT(BaseWhisperSTTService):
    """Voxtral behind llama-server takes chat-completions `input_audio`,
    not `/v1/audio/transcriptions` - and `cache_prompt` must be off: slot
    cache reuse splices mid-audio and presents as deafness (S7)."""

    @staticmethod
    def _segment_stats(audio: bytes):
        """Duration and RMS of a wav segment, from the bytes themselves."""
        import array
        import io
        import math
        import wave

        with wave.open(io.BytesIO(audio)) as w:
            frames = w.readframes(w.getnframes())
            rate = w.getframerate() or 16000
        samples = array.array("h", frames)
        if not samples:
            return 0.0, 0.0
        rms = math.sqrt(sum(x * x for x in samples) / len(samples)) / 32768
        return len(samples) / rate, rms

    async def _transcribe(self, audio: bytes) -> Transcription:
        from loguru import logger

        try:
            duration, rms = self._segment_stats(audio)
        except Exception as e:
            logger.debug(f"voxtral segment stats unparseable ({e}); letting the model try")
            duration, rms = 1.0, 1.0  # unparseable: let the model try
        if duration < MIN_SEGMENT_SECONDS or rms < MIN_SEGMENT_RMS:
            logger.debug(
                f"voxtral segment gated: duration={duration:.2f}s rms={rms:.4f} "
                f"(gates: {MIN_SEGMENT_SECONDS}s / {MIN_SEGMENT_RMS})"
            )
            return Transcription(text="")
        b64 = base64.b64encode(audio).decode()
        r = await self._client.chat.completions.create(
            model="voxtral",
            messages=[
                {
                    "role": "user",
                    # Instruction first, audio second - tested: with the
                    # audio first the model treats it as the message and
                    # answers question-shaped speech instead of writing it
                    # down. Order does not fix the obeys-spoken-commands
                    # class, which is why Parakeet holds the seat.
                    "content": [
                        {"type": "text", "text": TRANSCRIBE_PROMPT},
                        {"type": "input_audio", "input_audio": {"data": b64, "format": "wav"}},
                    ],
                }
            ],
            temperature=0,
            max_tokens=500,
            extra_body={"cache_prompt": False},
        )
        raw = r.choices[0].message.content or ""
        logger.debug(
            f"voxtral answered: duration={duration:.2f}s rms={rms:.4f} raw={raw[:120]!r}"
        )
        text = raw.strip().strip('"')
        # Output-side guards, layered behind the energy gate: an assistant
        # reply, a "no transcription" notice, punctuation confetti, or more
        # words than the audio could hold must never become a user turn.
        lowered = text.lower()
        if lowered.startswith(
            ("i'm sorry", "i am sorry", "i'm unable", "i can't", "i'm an ai",
             "as an ai", "no transcription", "there are no words")
        ):
            text = ""
        elif not any(c.isalnum() for c in text):
            text = ""
        elif len(text.split()) > max(3.0, duration * MAX_WORDS_PER_SECOND):
            logger.debug(f"voxtral word-rate cap: {len(text.split())} words in {duration:.2f}s")
            text = ""
        elif is_probable_echo(text):
            logger.debug(f"voxtral echo filter: transcript matches recent bot speech: {text[:60]!r}")
            text = ""
        return Transcription(text=text)


class ParakeetSTT(VoxtralSTT):
    """The structural transcriber: Parakeet TDT behind parakeet_server.py,
    whisper-style multipart. Inherits Voxtral's gates (energy, duration,
    echo, word-rate) - a faithful transcriber faithfully transcribes the
    bot's own speaker when echo cancellation fails, so the text filter
    stays load-bearing - but needs none of the prompt discipline, because
    there is no prompt."""

    async def _transcribe(self, audio: bytes) -> Transcription:
        from loguru import logger

        try:
            duration, rms = self._segment_stats(audio)
        except Exception:
            duration, rms = 1.0, 1.0
        if duration < MIN_SEGMENT_SECONDS or rms < MIN_SEGMENT_RMS:
            logger.debug(f"parakeet segment gated: duration={duration:.2f}s rms={rms:.4f}")
            return Transcription(text="")
        r = await self._client.audio.transcriptions.create(
            model="parakeet", file=("segment.wav", audio, "audio/wav")
        )
        text = (r.text or "").strip()
        logger.debug(f"parakeet: duration={duration:.2f}s rms={rms:.4f} text={text[:100]!r}")
        if is_probable_echo(text):
            logger.debug(f"parakeet echo filter: {text[:60]!r}")
            return Transcription(text="")
        return Transcription(text=text)


_voices_cache = None


def available_voices():
    """What the TTS server can actually speak as, asked once and cached.

    Asked rather than configured, for the reason `GET /props` is asked of
    llama-server: a list written down here is a claim about another
    process, and the failure it produces is the UI offering a voice that
    400s at the moment somebody wants to hear it. On failure this returns
    None - meaning "unknown", never an empty list, because a picker that
    renders no choices and a picker that could not ask are opposite
    findings and only one of them should hide the control.
    """
    global _voices_cache
    if _voices_cache is not None:
        return _voices_cache
    import json
    import urllib.request

    from loguru import logger

    try:
        with urllib.request.urlopen(f"{TTS_URL}/voices", timeout=5) as r:
            _voices_cache = json.load(r).get("voices") or None
    except Exception as e:  # noqa: BLE001 - any failure is the same answer
        logger.debug(f"voice list unavailable at {TTS_URL}/voices: {e}")
        _voices_cache = None
    return _voices_cache


class LocalTTS(OpenAITTSService):
    """The stock service hard-validates `voice` against OpenAI's own list,
    which rejects every voice our local servers actually have. Same wire
    (POST /v1/audio/speech, pcm streaming), our voice names allowed.

    Voice and speed are held here rather than read from the environment at
    call time, because the page can change them mid-call: the service
    instance is per-connection, so one listener's choice cannot reach
    another's, and there is deliberately no way to set them globally from
    a message."""

    def __init__(self, *args, speed: float = 1.0, **kwargs):
        super().__init__(*args, **kwargs)
        self._speed = speed

    @property
    def speed(self) -> float:
        return self._speed

    def set_speed(self, speed: float) -> bool:
        """Clamp-and-refuse rather than clamp-and-accept: a control that
        silently substitutes a different value than the one asked for
        leaves the UI showing a lie."""
        if not (MIN_SPEED <= speed <= MAX_SPEED):
            return False
        self._speed = speed
        return True

    def set_voice_name(self, voice: str) -> None:
        self._settings.voice = voice

    async def run_tts(self, text: str, context_id: str):
        from pipecat.frames.frames import ErrorFrame, TTSAudioRawFrame

        try:
            create_params = {
                "input": text,
                "model": self._settings.model,
                "voice": self._settings.voice,
                "speed": self._speed,
                "response_format": "pcm",
            }
            async with self._client.audio.speech.with_streaming_response.create(
                **create_params
            ) as r:
                if r.status_code != 200:
                    error = await r.text()
                    yield ErrorFrame(error=f"TTS error {r.status_code}: {error}")
                    return
                await self.start_tts_usage_metrics(text)
                note_bot_speech(text)
                async for chunk in r.iter_bytes(self.chunk_size):
                    if len(chunk) > 0:
                        await self.stop_ttfb_metrics()
                        yield TTSAudioRawFrame(chunk, self.sample_rate, 1, context_id=context_id)
        except Exception as e:
            yield ErrorFrame(error=f"TTS failed: {e}")


async def run_bot(transport: BaseTransport, runner_args: RunnerArguments):
    stt_cls = ParakeetSTT if STT_KIND == "parakeet" else VoxtralSTT
    stt = stt_cls(api_key="unused", base_url=STT_URL)
    tts = LocalTTS(
        api_key="unused",
        base_url=TTS_URL,
        settings=OpenAITTSService.Settings(voice=TTS_VOICE, model="tts"),
        speed=TTS_SPEED,
    )
    # The facade ignores the re-sent history (the Conversation is the
    # server's state) and the system prompt rides in mecha's cached prefix,
    # so neither is configured here. The session key (D3) travels as a
    # header - pipecat's service has no `user` field, and without a key
    # every call would share one eternal conversation and two clients
    # would barge-in on each other forever. Per connection: one call, one
    # taint slate, one transcript.
    session_key = f"webrtc-{uuid.uuid4().hex[:8]}"
    print(f"voice session key: {session_key}", flush=True)
    llm = OpenAILLMService(
        api_key="unused",
        base_url=FACADE_URL,
        model="mecha",
        default_headers={"X-Voice-Session": session_key},
    )

    context = LLMContext()
    user_aggregator, assistant_aggregator = LLMContextAggregatorPair(
        context,
        user_params=LLMUserAggregatorParams(vad_analyzer=SileroVADAnalyzer()),
    )

    # RTVI is how the page knows what is happening: transcripts both ways,
    # user/bot speaking edges, and bot-llm-started - which is exactly the
    # thinking-sound trigger (request in flight, no first token yet). The
    # worker observes; the client plays. docs/VOICE-RESEARCH.md D7/D9.
    rtvi = RTVIProcessor()

    pipeline = Pipeline(
        [
            transport.input(),
            rtvi,
            stt,
            user_aggregator,
            llm,
            tts,
            transport.output(),
            assistant_aggregator,
        ]
    )

    worker = PipelineWorker(
        pipeline,
        params=PipelineParams(enable_metrics=True),
        observers=[RTVIObserver(rtvi)],
    )

    from pipecat.workers.runner import WorkerRunner

    runner = WorkerRunner(handle_sigint=runner_args.handle_sigint)
    await runner.add_workers(worker)

    # The voice controls (D7's dock). One message type serves read and
    # write: `{}` asks what is available, a payload sets it, and both
    # answer with the same shape, so the page has exactly one code path
    # for "what is the voice now" and cannot render a control that
    # disagrees with the server.
    #
    # This is a *presentation* channel and nothing else: it can pick a
    # voice and a rate, and there is deliberately no field here that
    # reaches the agent, the workspace or the posture. The remote-control
    # rule - inbound text is a prompt, never a command - is what keeps a
    # data channel from becoming a control plane, and the way to keep it
    # true is that the only settable things are how the answer sounds.
    @rtvi.event_handler("on_client_message")
    async def on_client_message(rtvi, msg):
        if msg.type != "voice-config":
            return
        data = msg.data or {}
        applied, refused = {}, {}
        if "voice" in data:
            want = str(data["voice"])
            known = available_voices()
            # Unknown list means unknown, not permissive: refusing here
            # costs one unchanged voice, where guessing costs a 400 in
            # the middle of a spoken sentence.
            if known is not None and want in known:
                tts.set_voice_name(want)
                applied["voice"] = want
            else:
                refused["voice"] = want
        if "speed" in data:
            try:
                want = float(data["speed"])
            except (TypeError, ValueError):
                refused["speed"] = data["speed"]
            else:
                if tts.set_speed(want):
                    applied["speed"] = want
                else:
                    refused["speed"] = want
        if applied:
            print(f"voice config: {applied}", flush=True)
        await rtvi.send_server_message({
            "t": "voice-config",
            "voices": available_voices(),
            "voice": tts._settings.voice,
            "speed": tts.speed,
            "range": {"min": MIN_SPEED, "max": MAX_SPEED},
            "refused": refused or None,
        })

    @transport.event_handler("on_client_connected")
    async def on_client_connected(transport, client):
        print("voice client connected", flush=True)

    @transport.event_handler("on_client_disconnected")
    async def on_client_disconnected(transport, client):
        print("voice client disconnected", flush=True)
        await runner.cancel()

    await runner.run()


async def bot(runner_args: RunnerArguments):
    webrtc_connection: SmallWebRTCConnection = runner_args.webrtc_connection
    transport = SmallWebRTCTransport(
        webrtc_connection=webrtc_connection,
        params=TransportParams(
            audio_in_enabled=True,
            audio_out_enabled=True,
        ),
    )
    await run_bot(transport, runner_args)


if __name__ == "__main__":
    from pipecat.runner.run import main

    main()
