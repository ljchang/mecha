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
    MECHA_VOICE_STT   Parakeet          (default http://127.0.0.1:8992/v1)
    MECHA_VOICE_TTS   Chatterbox Turbo  (default http://127.0.0.1:8881/v1)
    MECHA_VOICE_TTS_VOICE  voice name for the TTS leg (start value; the
                      page can change it per session)
    MECHA_VOICE_TTS_SPEED  speaking rate, 0.5-2.0 (start value, likewise)
    MECHA_VOICE_TTS_EXAGGERATION  emotion intensity, 0.0-1.0
    MECHA_VOICE_TTS_CFG_WEIGHT    guidance weight, 0.0-1.0 (lower = more
                      expressive pacing; it moves *against* exaggeration)
"""

import os
import uuid

from openai.types.audio import Transcription

from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.audio.vad.vad_analyzer import VADParams
from pipecat.turns.user_start.transcription_user_turn_start_strategy import (
    TranscriptionUserTurnStartStrategy,
)
from pipecat.turns.user_turn_strategies import UserTurnStrategies
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
STT_URL = os.environ.get("MECHA_VOICE_STT", "http://127.0.0.1:8992/v1")
TTS_URL = os.environ.get("MECHA_VOICE_TTS", "http://127.0.0.1:8881/v1")
TTS_VOICE = os.environ.get("MECHA_VOICE_TTS_VOICE", "default")
TTS_SPEED = float(os.environ.get("MECHA_VOICE_TTS_SPEED", "1.0"))
# Bounds mirror the TTS server's own. Duplicated rather than fetched
# because they gate a value before it is sent: a slider that can ask for
# 4x and learn it was refused mid-sentence is a worse control than one
# that cannot ask.
MIN_SPEED, MAX_SPEED = 0.5, 2.0

# Chatterbox's expressiveness pair, and the whole reason the voice sounded
# flat: the serving wrapper defaulted `exaggeration` to 0.0 - the monotone
# end of the scale, not the neutral middle - and nothing here sent the
# field at all, so every utterance since launch went out at the minimum.
# Resemble's expressive recipe raises exaggeration against a *lowered*
# cfg_weight, because a high one rushes the cadence. Sent on every request
# rather than left to the server's default: two places holding an opinion
# about how mecha sounds is one place too many, and the server's is now
# the library's neutral rather than this one.
TTS_EXAGGERATION = float(os.environ.get("MECHA_VOICE_TTS_EXAGGERATION", "0.8"))
TTS_CFG_WEIGHT = float(os.environ.get("MECHA_VOICE_TTS_CFG_WEIGHT", "0.3"))

# Pinned per the build log (docs/VOICE-RESEARCH.md S7): this wording
# transcribes; "from beginning to end" phrasing makes the model refuse, and
# naming an escape token ("output NOSPEECH") anchors the model into
# answering it for real speech too - both found by testing. Treat any
# rewording as a change to test, not a paraphrase. The trailing sentence
# was tested against jfk.wav (unchanged) and silence/noise (degrades the
# reply to droppable fragments instead of chat).
# Segments quieter than this never reach the model. The gate was measured
# against a chat-model transcriber, which handed silence or echo residue
# stopped transcribing and started *answering* ("I'm an AI and don't have
# a calendar") - and that answer rode into mecha as the owner's words, the
# observed 2026-08-24 bug. A transducer cannot do that, so the gate is now
# about cost and false turns rather than about fabrication; it stays
# because a half-second of room noise is not a turn whatever hears it.
# Measured on this STT server: speech ~0.14 RMS, room noise ~0.009,
# silence 0.000; the gate sits in the gap.
MIN_SEGMENT_RMS = 0.010
MIN_SEGMENT_SECONDS = 0.3


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


class SegmentGatedSTT(BaseWhisperSTTService):
    """The energy/duration gate every segment passes before any model sees
    it. Split out from the transcriber because it is a property of the
    *audio*, not of whichever model reads it: room noise and half-second
    breaths are not speech regardless of what is listening."""

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

class ParakeetSTT(SegmentGatedSTT):
    """The transcriber: Parakeet TDT behind parakeet_server.py,
    whisper-style multipart. Takes the segment gate (energy, duration) and
    the echo filter - a faithful transcriber faithfully transcribes the
    bot's own speaker when echo cancellation fails, so the text filter
    stays load-bearing - and needs no prompt discipline at all, because
    there is no prompt. The output-side guards a chat-model transcriber
    needed (refusal prefixes, a word-rate cap) went with it: a transducer
    cannot emit words it did not hear."""

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

    def __init__(self, *args, speed: float = 1.0,
                 exaggeration: float = TTS_EXAGGERATION,
                 cfg_weight: float = TTS_CFG_WEIGHT, **kwargs):
        super().__init__(*args, **kwargs)
        self._speed = speed
        self._exaggeration = exaggeration
        self._cfg_weight = cfg_weight

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
                # Not fields of OpenAI's speech API, so they ride in
                # extra_body rather than being silently dropped by the
                # typed client - which is how a knob gets plumbed to a
                # server that accepts it and still never arrives.
                "extra_body": {
                    "exaggeration": self._exaggeration,
                    "cfg_weight": self._cfg_weight,
                },
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
    stt = ParakeetSTT(api_key="unused", base_url=STT_URL)
    tts = LocalTTS(
        api_key="unused",
        base_url=TTS_URL,
        settings=OpenAITTSService.Settings(voice=TTS_VOICE, model="tts"),
        speed=TTS_SPEED,
        exaggeration=TTS_EXAGGERATION,
        cfg_weight=TTS_CFG_WEIGHT,
    )
    # The facade ignores the re-sent history (the Conversation is the
    # server's state) and the system prompt rides in mecha's cached prefix,
    # so neither is configured here. The session key travels as a header -
    # pipecat's service has no `user` field, and without a key every call
    # would share one eternal conversation and two clients would barge-in
    # on each other forever. Per connection: one call, one slot.
    session_key = f"webrtc-{uuid.uuid4().hex[:8]}"
    headers = {"X-Voice-Session": session_key}

    # D3: when the caller named a chat session in the WebRTC offer, the
    # facade speaks into *that* conversation instead of opening one of its
    # own - talking and typing become one transcript, one taint slate, one
    # workspace. A second header rather than overloading the first, because
    # the two mean different things and only one of them is ours to mint.
    #
    # Validated here as well as at the facade, and to the same rule
    # (mecha-cli's `valid_key`): this value becomes a directory name under
    # the producer root, it arrives from a browser, and a claim checked on
    # only one side of a seam is a claim nobody checks the day the other
    # side is reached directly. Refusing costs a call that is merely
    # unshared; passing it on costs whatever a bad name does.
    named = None
    body = runner_args.body if isinstance(runner_args.body, dict) else {}
    want = body.get("session")
    if isinstance(want, str):
        want = want.strip()
        if (
            0 < len(want) <= 32
            and want[0] not in "-_"
            and all(c.islower() or c.isdigit() or c in "-_" for c in want)
            and want.isascii()
        ):
            named = want
        elif want:
            print(f"voice: refusing malformed chat session {want!r}", flush=True)
    if named:
        headers["X-Chat-Session"] = named
    print(
        f"voice session key: {session_key}"
        + (f" (speaking into chat session {named!r})" if named else ""),
        flush=True,
    )
    llm = OpenAILLMService(
        api_key="unused",
        base_url=FACADE_URL,
        model="mecha",
        default_headers=headers,
    )

    context = LLMContext()
    # A user turn starts on a *transcription*, never on the VAD. Pipecat's
    # default is both (`[VADUserTurnStartStrategy, Transcription...]`) and
    # the VAD always wins the race, which is the bug: 200ms of anything
    # Silero scores as speech stops the bot mid-sentence, and a keyboard
    # clears that easily. Observed 2026-08-25 in a real call - 1.18s at
    # rms 0.0124 interrupted a reply and transcribed to '' - so the bot
    # stopped for a sound with no words in it.
    #
    # Dropping VAD from the *start* list fixes it structurally rather than
    # by tuning: `BaseWhisperSTTService.run_stt` emits no TranscriptionFrame
    # at all for empty text (`if text or self._push_empty_transcripts`),
    # so a wordless segment now reaches no strategy and the bot simply
    # keeps talking. This is "resume on an empty transcript" achieved by
    # never stopping, which needs no state to unwind.
    #
    # The VAD analyzer stays - it still *segments*, which is what hands the
    # STT an utterance. What changed is that a false segment now costs one
    # wasted 92ms transcription instead of an interruption, so the gate can
    # afford to stay sensitive. That matters here: the owner's measured
    # speech is ~0.024 RMS against a 0.14 tuning assumption, so raising VAD
    # thresholds to chase noise would start dropping quiet real speech.
    # start_secs 0.2 -> 0.3 only rejects the shortest transients; confidence
    # and min_volume are left alone deliberately.
    #
    # The cost, stated: Parakeet is offline, so a transcript arrives after
    # the utterance ends. Barge-in is therefore "finish the phrase and it
    # stops" rather than instant. A streaming STT would restore instant
    # barge-in via use_interim=True; with this one interim frames never
    # exist, so it is False rather than misleading.
    user_aggregator, assistant_aggregator = LLMContextAggregatorPair(
        context,
        user_params=LLMUserAggregatorParams(
            vad_analyzer=SileroVADAnalyzer(params=VADParams(start_secs=0.3)),
            user_turn_strategies=UserTurnStrategies(
                start=[TranscriptionUserTurnStartStrategy(use_interim=False)],
            ),
        ),
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

    # Pipecat cancels an "idle" pipeline - and the runner with it - after
    # `idle_timeout_secs`, where idle means neither side produced a speaking
    # frame. The default is 300s, which is a *conversational* silence on a
    # phone: think for five minutes, read something, put it in your pocket,
    # and the call dies with no message of any kind. The client sees only the
    # peer connection close, which is how "the call ends by itself" was
    # reported. Measured in production 2026-08-25: a call connected 11:36:23
    # ended 11:41:50 on exactly this timer.
    #
    # The timeout is kept rather than disabled, because an abandoned tab
    # otherwise holds VAD, turn detection, STT and TTS open forever on a box
    # with one GPU. It is raised to fifteen minutes - past any pause that is
    # still a conversation - and, more importantly, it now *says so* on the
    # way out (below): a component that stops has to be able to tell you why,
    # or the only symptom is a surface that silently stopped working.
    IDLE_SECS = 15 * 60
    worker = PipelineWorker(
        pipeline,
        params=PipelineParams(enable_metrics=True),
        observers=[RTVIObserver(rtvi)],
        idle_timeout_secs=IDLE_SECS,
    )

    @worker.event_handler("on_idle_timeout")
    async def on_idle_timeout(worker):
        # Fired before the cancel, so the data channel is still open and this
        # is the last chance to name the cause. Best-effort by design: a
        # failure here must not stop the teardown it is announcing.
        print(f"voice idle timeout after {IDLE_SECS}s - ending the call", flush=True)
        try:
            await rtvi.send_server_message({"t": "call-ending", "reason": "idle", "after_secs": IDLE_SECS})
        except Exception as e:  # noqa: BLE001 - an announcement is not load-bearing
            print(f"voice idle timeout: could not announce ({e})", flush=True)

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
