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
    MECHA_VOICE_STT   Voxtral           (default http://127.0.0.1:8082/v1)
    MECHA_VOICE_TTS   Kokoro/Chatterbox (default http://127.0.0.1:8880/v1)
    MECHA_VOICE_TTS_VOICE  voice name for the TTS leg
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
STT_URL = os.environ.get("MECHA_VOICE_STT", "http://127.0.0.1:8082/v1")
TTS_URL = os.environ.get("MECHA_VOICE_TTS", "http://127.0.0.1:8880/v1")
TTS_VOICE = os.environ.get("MECHA_VOICE_TTS_VOICE", "af_heart")

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
        try:
            duration, rms = self._segment_stats(audio)
        except Exception:
            duration, rms = 1.0, 1.0  # unparseable: let the model try
        if duration < MIN_SEGMENT_SECONDS or rms < MIN_SEGMENT_RMS:
            return Transcription(text="")
        b64 = base64.b64encode(audio).decode()
        r = await self._client.chat.completions.create(
            model="voxtral",
            messages=[
                {
                    "role": "user",
                    "content": [
                        {"type": "input_audio", "input_audio": {"data": b64, "format": "wav"}},
                        {"type": "text", "text": TRANSCRIBE_PROMPT},
                    ],
                }
            ],
            temperature=0,
            max_tokens=500,
            extra_body={"cache_prompt": False},
        )
        text = (r.choices[0].message.content or "").strip().strip('"')
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
            text = ""
        return Transcription(text=text)


class LocalTTS(OpenAITTSService):
    """The stock service hard-validates `voice` against OpenAI's own list,
    which rejects every voice our local servers actually have. Same wire
    (POST /v1/audio/speech, pcm streaming), our voice names allowed."""

    async def run_tts(self, text: str, context_id: str):
        from pipecat.frames.frames import ErrorFrame, TTSAudioRawFrame

        try:
            create_params = {
                "input": text,
                "model": self._settings.model,
                "voice": self._settings.voice,
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
                async for chunk in r.iter_bytes(self.chunk_size):
                    if len(chunk) > 0:
                        await self.stop_ttfb_metrics()
                        yield TTSAudioRawFrame(chunk, self.sample_rate, 1, context_id=context_id)
        except Exception as e:
            yield ErrorFrame(error=f"TTS failed: {e}")


async def run_bot(transport: BaseTransport, runner_args: RunnerArguments):
    stt = VoxtralSTT(api_key="unused", base_url=STT_URL)
    tts = LocalTTS(
        api_key="unused",
        base_url=TTS_URL,
        settings=OpenAITTSService.Settings(voice=TTS_VOICE, model="tts"),
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
