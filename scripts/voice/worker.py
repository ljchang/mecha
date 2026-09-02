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

import httpx
from openai.types.audio import Transcription

from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.audio.vad.vad_analyzer import VADParams
from pipecat.frames.frames import (
    BotStartedSpeakingFrame,
    BotStoppedSpeakingFrame,
    VADUserStartedSpeakingFrame,
)
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

# GOAL-SYSTEM-DESIGN.md §6.2's voice readout: a small side channel to the
# facade, symmetric with the TTS/STT legs above rather than piggybacking the
# OpenAI-compatible completion response - `OpenAILLMService` parses that
# through the real `openai` SDK's typed models, which drop an unrecognised
# top-level field before any pipecat frame processor ever sees it.
AFFECT_URL = f"{FACADE_URL}/mecha-affect"
# This poll happens once per spoken answer (`LocalTTS.on_turn_context_created`),
# so a hung (not merely refusing) facade costs this much once per answer, not
# per sentence. Loopback, same machine - a healthy answer takes low
# single-digit milliseconds.
AFFECT_POLL_TIMEOUT_SECONDS = 0.05
# A single, deliberately conservative nudge for whichever of the four labels
# reachable today (`Affect::reachable_today`, mecha-core/src/appraisal.rs)
# the harness names - not a full emotion-to-prosody mapping, because nothing
# here can validate one perceptually. Lower `cfg_weight` reads as a more
# measured, careful delivery (Resemble's own expressive recipe: cfg_weight
# moves *against* exaggeration). Extending this is free once rung 7's
# quarantined appraiser and counterfactual probe make more labels reachable
# - only this table grows, nothing else here needs to change.
AFFECT_CFG_WEIGHT_DELTA = -0.05

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


# The echo defence: a phone or a laptop on speaker hears its own TTS, and
# everything downstream is faithful about it - the VAD finds speech in it and
# a transducer transcribes it - so the bot's words come back as the owner's.
# The reasoning, the thresholds and the tests live in `echo_filter`, which is
# a pure module for exactly that reason; this file owns the frames and the
# clock that feed it.
import time as _time

from echo_filter import BotSpeech, overlapped

BOT_SPEECH = BotSpeech()


def note_bot_speech(text: str) -> None:
    BOT_SPEECH.note(text)


# The energy floor while our own speaker is playing. Echo that survives the
# browser's canceller is far quieter than a voice at the microphone, so this
# sits above the room-noise floor below and below the owner's measured speech
# (~0.024 RMS) - a person raising their voice over a reply clears it, a
# speaker across the room does not.
#
# Tunable from the environment because the right value is a property of *this
# room and this laptop*, not of the code: turn on MECHA_LOG=debug (or read the
# worker's journal) and every gated segment prints the RMS it was gated at,
# which is the measurement to set this from. A guessed constant with no way to
# check it would be the worse half of both worlds.
ECHO_SEGMENT_RMS = float(os.environ.get("MECHA_VOICE_ECHO_RMS", "0.020"))


class SegmentGatedSTT(BaseWhisperSTTService):
    """The energy/duration gate every segment passes before any model sees
    it. Split out from the transcriber because it is a property of the
    *audio*, not of whichever model reads it: room noise and half-second
    breaths are not speech regardless of what is listening.

    It also keeps the clock the echo defence needs: **was our own speaker
    playing while this segment was captured**. Kept here rather than in a
    processor of its own because pipecat pushes the bot-speaking edges
    *upstream* as well as downstream (`base_output.py`), so the STT service
    sees them where it stands, and because the question is asked in the same
    breath as the energy floor it changes.

    The edges are the transport's, not the TTS service's, and that is the
    point: `BotStartedSpeakingFrame` fires when audio starts being written
    out, which is when a room can hear it. TTS text is generated well before
    that and would put the window in the wrong place."""

    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self._segment_started_at = None
        self._bot_speaking = False
        # Never `0.0`: `time.monotonic()` is uptime on Linux, so a zero here
        # is "the bot stopped speaking at boot", which on a freshly started
        # box is inside the tail.
        self._bot_audible_until = float("-inf")

    async def process_frame(self, frame, direction):
        await super().process_frame(frame, direction)
        if isinstance(frame, VADUserStartedSpeakingFrame):
            self._segment_started_at = _time.monotonic()
        elif isinstance(frame, BotStartedSpeakingFrame):
            self._bot_speaking = True
        elif isinstance(frame, BotStoppedSpeakingFrame):
            self._bot_speaking = False
            self._bot_audible_until = _time.monotonic()

    def heard_the_speaker(self) -> bool:
        """Did our own reply overlap the segment about to be transcribed?"""
        return overlapped(
            now=_time.monotonic(),
            segment_started_at=self._segment_started_at,
            bot_speaking=self._bot_speaking,
            bot_audible_until=self._bot_audible_until,
        )

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
        # Whether our own speaker was playing changes what this segment has to
        # clear, and it is asked once here so the floor and the text filter
        # cannot disagree about it.
        echoey = self.heard_the_speaker()
        floor = ECHO_SEGMENT_RMS if echoey else MIN_SEGMENT_RMS
        if duration < MIN_SEGMENT_SECONDS or rms < floor:
            # The RMS is printed on the gated path too, and deliberately: it
            # is the only measurement of what this room's echo actually looks
            # like, and `MECHA_VOICE_ECHO_RMS` has to be set from something.
            logger.debug(
                f"parakeet segment gated: duration={duration:.2f}s rms={rms:.4f} "
                f"floor={floor:.4f} over_speaker={echoey}"
            )
            return Transcription(text="")
        r = await self._client.audio.transcriptions.create(
            model="parakeet", file=("segment.wav", audio, "audio/wav")
        )
        text = (r.text or "").strip()
        logger.debug(
            f"parakeet: duration={duration:.2f}s rms={rms:.4f} "
            f"over_speaker={echoey} text={text[:100]!r}"
        )
        if BOT_SPEECH.is_probable_echo(text, bot_was_audible=echoey):
            logger.debug(f"parakeet echo filter: {text[:60]!r} over_speaker={echoey}")
            return Transcription(text="")
        return Transcription(text=text)


_voices_cache = None


def available_voices(refresh=False):
    """What the TTS server can actually speak as, asked once and cached.

    Asked rather than configured, for the reason `GET /props` is asked of
    llama-server: a list written down here is a claim about another
    process, and the failure it produces is the UI offering a voice that
    400s at the moment somebody wants to hear it. On failure this returns
    None - meaning "unknown", never an empty list, because a picker that
    renders no choices and a picker that could not ask are opposite
    findings and only one of them should hide the control.

    `refresh` drops the cache first. The one caller that passes it is the
    voice-config handler on a *miss*: a voice can be cloned onto the box
    while this process runs (the settings page writes a WAV into the same
    directory the TTS lists), and a forever-cache would refuse the new name
    until a worker restart nobody was told to do. Refetching only on a miss
    keeps the happy path at zero extra requests - a known voice never pays.
    """
    global _voices_cache
    if refresh:
        _voices_cache = None
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
                 cfg_weight: float = TTS_CFG_WEIGHT,
                 affect_key: str | None = None, **kwargs):
        super().__init__(*args, **kwargs)
        self._speed = speed
        self._exaggeration = exaggeration
        self._cfg_weight = cfg_weight
        # §6.2: the same namespaced key `mecha-cli`'s facade uses internally
        # (`"chat:<id>"` for a hosted D3 session, `"voice:<slot>"` for the
        # facade's own) - the two must not collide, so the namespace has to
        # match on both sides of this poll. `None` when there is nothing to
        # poll for (should not happen in practice; `run_bot` always sets one).
        self._affect_key = affect_key
        # Loopback, same machine - a healthy facade answers in low single-
        # digit milliseconds. Worth naming as its own constant rather than
        # matching the TTS server's own timeouts: a hung facade must cost
        # this once per *answer* and no more (see `on_turn_context_created`),
        # not the seconds a normal network timeout would tolerate. Falls
        # back to the baseline on expiry, same as any other failure here.
        self._affect_client = httpx.AsyncClient(timeout=AFFECT_POLL_TIMEOUT_SECONDS)
        # Latched once per turn by `on_turn_context_created`, never re-polled
        # per sentence - see that method's docstring for why.
        self._affect_context_id: str | None = None
        self._affect_params: tuple[float, float] = (self._exaggeration, self._cfg_weight)

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

    def set_affect_key(self, key: str) -> None:
        """Set once `named`/`session_key` are known in `run_bot` - the same
        after-construction pattern `set_speed`/`set_voice_name` already use,
        since the facade's namespaced key is not known at construction
        time."""
        self._affect_key = key

    async def on_turn_context_created(self, context_id: str) -> None:
        """§6.2's voice readout, latched once per answer rather than polled
        per sentence.

        `context_id` is the base class's own turn boundary — with
        `reuse_context_id_within_turn` (the default, not overridden here),
        `create_context_id` reuses one id for every sentence of an answer,
        and this hook fires exactly once per new id, before any text reaches
        `run_tts`. Polling from `run_tts` instead polled per sentence against
        a set-and-overwrite cache, so a slow tail sentence - or the
        confirmation offer appended after it - could read the *next*
        answer's label once the facade's `turn.done` had already landed,
        switching `cfg_weight` mid-utterance instead of lagging by one clean
        turn.

        Lags by one turn, honestly rather than by accident: the facade can
        only cache a label once the turn that earned it has *finished*
        (`Affect` is a function of the completed `RunOutcome`), and this
        fires while that turn's own text is still streaming in - before the
        facade has had a chance to update the cache. So what this reads is
        the *previous* turn's mood, applied to the current turn's words.
        There is no way to close that gap without holding speech until the
        whole answer is known, which defeats the point of streaming - a
        deliberate trade-off, not a bug.

        **Logs every fire, unconditionally.** Whether this base-class hook is
        actually dispatched by the installed pipecat is not provable from
        this file alone (`run_tts`'s own `context_id` parameter proves TTS
        context ids exist, not that this particular hook name is part of the
        contract on every version). If it silently stopped firing, nothing
        here would error - `_affect_context_id` would stay `None` and
        `run_tts` would take the baseline every time, indistinguishable from
        every session simply carrying no affect, which is most of them
        (`Affect::Neutral` on 119 of 120 sessions in the rung 7 corpus). So
        the log line fires every time this method runs, not only when it
        changes the outgoing params - logging only the interesting case
        would read identically to the hook never firing at all on an
        ordinary, mostly-neutral day, which is exactly the silent-inertness
        failure this exists to catch. Debug level, once per answer: the same
        shape as the percent-encoding bug this module already shipped and
        fixed, which was also silently inert until someone went looking."""
        await super().on_turn_context_created(context_id)
        self._affect_context_id = context_id
        self._affect_params = await self._poll_affect_params()
        from loguru import logger

        logger.debug(
            f"voice affect latch: context={context_id} key={self._affect_key} "
            f"cfg_weight={self._affect_params[1]:.3f} (baseline "
            f"{self._cfg_weight:.3f})"
        )

    async def _poll_affect_params(self) -> tuple[float, float]:
        """One fetch of the cached label, applied to the caller's context
        only - never mutating `self._exaggeration`/`self._cfg_weight`, which
        stay the owner's configured baseline for every other session.

        Failure, timeout, or a `neutral`/absent label all fall back to the
        baseline silently - a harness hiccup must never make a call worse
        or slower than if this did not exist."""
        if not self._affect_key:
            return self._exaggeration, self._cfg_weight
        try:
            r = await self._affect_client.get(
                AFFECT_URL, params={"session": self._affect_key}
            )
            if r.status_code == 200:
                label = r.json().get("affect")
                if label and label != "neutral":
                    return (
                        self._exaggeration,
                        max(0.0, self._cfg_weight + AFFECT_CFG_WEIGHT_DELTA),
                    )
        except Exception:
            pass
        return self._exaggeration, self._cfg_weight

    async def run_tts(self, text: str, context_id: str):
        from pipecat.frames.frames import ErrorFrame, TTSAudioRawFrame

        try:
            # `on_turn_context_created` always fires before `run_tts` for a
            # given context per the base class's own contract, so the
            # mismatch arm below should be unreachable - the baseline is
            # always the safe answer if it somehow is not.
            exaggeration, cfg_weight = (
                self._affect_params
                if context_id == self._affect_context_id
                else (self._exaggeration, self._cfg_weight)
            )
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
                    "exaggeration": exaggeration,
                    "cfg_weight": cfg_weight,
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
    # §6.2: the same namespaced key `mecha-cli`'s facade keys its cache by
    # (`hosted_completion`'s `confirm_key`) - a hosted chat session and this
    # connection's own voice slot must not collide, so the namespace has to
    # match exactly on both sides of the poll.
    tts.set_affect_key(f"chat:{named}" if named else f"voice:{session_key}")
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
            if known is not None and want not in known:
                # A miss may be a voice cloned since this process last
                # asked - one revalidation before refusing, so a fresh
                # clone works on the next call rather than after a worker
                # restart. A genuinely unknown name costs one extra list
                # fetch and is then refused exactly as before.
                known = available_voices(refresh=True)
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
