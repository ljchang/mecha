"""What the voice worker's journal may hold: measurements, never words.

The worker's stderr is the systemd journal, kept on disk. Until 2026-10-04 an
ordinary call (one not into an incognito chat) wrote both sides of itself
there: the first 80 characters of every typed turn and 100 of every
transcript (the worker's own lines), and, from pipecat's DEBUG lines, every
sentence the TTS was handed and the whole conversation sent to the model (up
to ~9,700 characters a line). The owner, on learning it: "Let's definitely
fix that". Incognito calls were already silent (`worker.Unlogged`).

Two rules, both here so they are tested without pipecat or loguru installed
(CI runs the voice tests on a bare python3):

- `withheld`: a worker line that would carry words carries their length.
- `keeps`: pipecat's `services` family logs text below INFO (measured
  2026-10-04 over two days of this journal: `tts_service._push_tts_frames`,
  `whisper.base_stt.run_stt` and `openai.base_llm.get_chat_completions` held
  a phrase on 903 of 1,196 lines; no other pipecat DEBUG line held one in
  ~4,000). The whole family is cut below INFO rather than those three
  functions, so a service added or upgraded later cannot start writing words
  through a door nobody listed. Its warnings and errors stay, as do every
  other module's DEBUG lines (connection state, turn timing, metrics).
"""

# Below this loguru level number a record from a text family is dropped.
INFO = 20

# Logger-name families whose lines below INFO carry what was said.
TEXT_FAMILIES = ("pipecat.services",)


def withheld(text: str) -> str:
    """A log line's stand-in for words: how many characters, never which."""
    return f"<{len(text)} chars>"


def keeps(name: str | None, level_no: int) -> bool:
    """Whether a log record from logger `name` at `level_no` may reach the
    journal."""
    if level_no >= INFO:
        return True
    name = name or ""
    return not any(name == f or name.startswith(f + ".") for f in TEXT_FAMILIES)


def install(logger, sink) -> int:
    """Make `sink` loguru's only handler, filtered by `keeps`, and return its
    id. Replaces every handler: pipecat's runner adds an unfiltered DEBUG
    sink in `main()` (`logger.remove()` then `logger.add(sys.stderr)`), so
    this must run after it, and the worker calls it at server start and
    again at the top of every call."""
    logger.remove()
    return logger.add(sink, level="DEBUG", filter=lambda r: keeps(r["name"], r["level"].no))


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
