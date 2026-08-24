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

from openai.types.audio import Transcription

from pipecat.audio.vad.silero import SileroVADAnalyzer
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.worker import PipelineParams, PipelineWorker
from pipecat.processors.aggregators.llm_context import LLMContext
from pipecat.processors.aggregators.llm_response_universal import (
    LLMContextAggregatorPair,
    LLMUserAggregatorParams,
)
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
# transcribes; "from beginning to end" phrasing makes the model refuse.
# Treat any rewording as a change to test, not a paraphrase.
TRANSCRIBE_PROMPT = "Transcribe this audio exactly. Output only the transcription."


class VoxtralSTT(BaseWhisperSTTService):
    """Voxtral behind llama-server takes chat-completions `input_audio`,
    not `/v1/audio/transcriptions` - and `cache_prompt` must be off: slot
    cache reuse splices mid-audio and presents as deafness (S7)."""

    async def _transcribe(self, audio: bytes) -> Transcription:
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
        # The model answers prose when it hears nothing intelligible; an
        # apology must not become a user turn.
        if text.lower().startswith(("i'm sorry", "i am sorry", "i'm unable", "i can't")):
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
    # so neither is configured here. `user` is the session key (D3).
    llm = OpenAILLMService(api_key="unused", base_url=FACADE_URL, model="mecha")

    context = LLMContext()
    user_aggregator, assistant_aggregator = LLMContextAggregatorPair(
        context,
        user_params=LLMUserAggregatorParams(vad_analyzer=SileroVADAnalyzer()),
    )

    pipeline = Pipeline(
        [
            transport.input(),
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
