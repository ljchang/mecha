"""What the voice worker's journal may hold: measurements, never words.

The worker's stderr is the systemd journal, kept on disk. Until 2026-10-04 an
ordinary call (one not into an incognito chat) wrote both sides of itself
there: the first 80 characters of every typed turn and 100 of every
transcript (the worker's own lines), and, from pipecat's DEBUG lines, every
sentence the TTS was handed and the whole conversation sent to the model (up
to ~9,700 characters a line). The owner, on learning it: "Let's definitely
fix that". Incognito calls were already silent (`worker.Unlogged`).

The rules live here so they are tested without pipecat; CI runs this module's
tests with loguru installed and nothing else.

- `withheld`: a worker line that would carry words carries their length.
- `keeps`: below INFO, a pipecat record reaches the journal only from a
  family measured to carry no words (`WORDLESS_FAMILIES`). Measured over a
  week of this journal (2026-09-27 to 2026-10-04): the `services` family had
  a 3+-word phrase on 1,255 of 1,781 DEBUG lines (`tts_service`'s
  `Generating TTS [...]`, `whisper.base_stt`'s `Transcription: [...]`,
  `openai.base_llm`'s `Generating chat [...]`), and the seven families
  listed had none in ~6,800. An allowlist, not a cut-list of the text-bearing
  families: a family a pipecat upgrade adds or renames is unmeasured, and
  unmeasured is not clean, so its DEBUG lines are dropped until someone
  measures it (review of #547). Warnings and errors from every family stay,
  as does everything outside `pipecat`.

  The price: `services` lines that carried no words go too, among them
  `base_llm`'s function-call lines and `tts_service`'s interruption handling,
  and no log level brings them back, because `keeps` cuts below INFO
  whatever the handler's level. To see them while debugging a tool call or a
  barge-in, run the worker for that session with `install` skipped (or the
  family added to `WORDLESS_FAMILIES`) and put it back after.
- Tracebacks name the failing line but never the values on it: loguru's
  `diagnose` is off (see `install`).
"""

# Below this loguru level number a pipecat record must come from a wordless
# family to be kept.
INFO = 20

# The namespace this narrows. Nothing outside it is pipecat's, so nothing
# outside it is cut.
CUT = "pipecat"

# pipecat families whose DEBUG lines were measured to carry no words (see
# the module docstring): connection state, turn timing, pipeline lifecycle,
# metrics.
WORDLESS_FAMILIES = (
    "pipecat.audio",
    "pipecat.pipeline",
    "pipecat.processors",
    "pipecat.registry",
    "pipecat.transports",
    "pipecat.utils",
    "pipecat.workers",
)


def withheld(text: str) -> str:
    """A log line's stand-in for words: how many characters, never which."""
    return f"<{len(text)} chars>"


def _within(name: str, family: str) -> bool:
    return name == family or name.startswith(family + ".")


def keeps(name: str | None, level_no: int) -> bool:
    """Whether a log record from logger `name` at `level_no` may reach the
    journal."""
    if level_no >= INFO:
        return True
    name = name or ""
    if not _within(name, CUT):
        return True
    return any(_within(name, f) for f in WORDLESS_FAMILIES)


def install(logger, sink) -> int:
    """Make `sink` loguru's only handler, filtered by `keeps`, and return its
    id. Replaces every handler: pipecat's runner adds an unfiltered DEBUG
    sink in `main()` (`logger.remove()` then `logger.add(sys.stderr)`), so
    this must run after it, and the worker calls it at server start and
    again at the top of every call."""
    logger.remove()
    return logger.add(
        sink,
        level="DEBUG",
        # loguru defaults both to True, and a diagnosed traceback prints the
        # values of the locals on each line it shows: for pipecat's
        # frame_processor exception path, the frame being processed, which is
        # a whole turn, at a name no family cut covers (review of #547). The
        # traceback still names the file, line and exception.
        backtrace=False,
        diagnose=False,
        filter=lambda r: keeps(r["name"], r["level"].no),
    )


def lifespan(existing, logger, sink):
    """Wrap an app's lifespan so `install` runs once the wrapped one has
    started: ours is then the last word on loguru's handlers, whatever the
    inner lifespan added or removed. What the inner one yields is passed
    through, because Starlette merges a yielded mapping into each request's
    `scope["state"]` and a wrapper yielding `None` would drop it silently
    (review of #547)."""
    import contextlib

    @contextlib.asynccontextmanager
    async def wrapped(app):
        async with existing(app) as state:
            install(logger, sink)
            yield state

    return wrapped
