"""Harbor adapter: run mecha inside a Terminal-Bench task container.

The `BaseInstalledAgent` shape, deliberately: `install()` puts the mecha
binary *inside* the task container and `run()` executes it there, so mecha's
own loop, tools, path jail and budgets are what get measured — the
alternative (`BaseAgent`) makes Harbor the executor and mecha a planner,
which measures nothing this project cares about.

The benchmark posture mirrors what `mecha eval` forces: a minimal config
with one provider, no MCP, no hooks, no outbox, no learned rules, no
fallbacks, sandbox `none` (the task container is the boundary), permission
mode `allow`. The sampler is deliberately NOT pinned: the servers run
`-np 1`, so a pinned seed would make repeated rollouts of a task
token-identical — k samples of one draw dressed as a reliability
measurement.

Model routing: the task container cannot see the host's loopback, so
`bench/run.sh` starts a socat forwarder and `install()` writes the
container's own default-gateway IP into the config — compose networks get
different gateways per task, so it must be discovered from inside, not
assumed.

Usage (from the repo root — see bench/run.sh, which does all of this):

    PYTHONPATH=bench harbor run -d <terminal-bench dataset> \
        --agent-import-path mecha_agent:MechaAgent \
        -m local/qwen3.6-35b-a3b
"""

import json
import shlex
from pathlib import Path
from typing import override

from harbor.agents.installed.base import BaseInstalledAgent, CliFlag
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

# Where bench/run.sh forwards the host's llama-server to. Not 8080: the
# forwarder listens on every interface for the run's duration, and a
# distinct port makes it obvious in `ss` what it is and what to kill.
FORWARD_PORT = 18080

# Falls back to the repo-relative release binary; override with the
# MECHA_BENCH_BINARY env var (an absolute path).
DEFAULT_BINARY = Path(__file__).resolve().parent.parent / "target" / "release" / "mecha"

SESSION_DIR = "/tmp/mecha-bench-sessions"


class MechaAgent(BaseInstalledAgent):
    """mecha, installed and run inside the task container."""

    CLI_FLAGS = [
        CliFlag("max_turns", cli="--max-turns", type="int", default=40),
    ]

    @staticmethod
    @override
    def name() -> str:
        return "mecha"

    @override
    def get_version_command(self) -> str | None:
        return "/installed-agent/mecha --version"

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        import os

        binary = Path(os.environ.get("MECHA_BENCH_BINARY", DEFAULT_BINARY))
        if not binary.is_file():
            raise FileNotFoundError(
                f"mecha binary not found at {binary}; build it (cargo build "
                "--release) or set MECHA_BENCH_BINARY"
            )
        await environment.upload_file(binary, "/installed-agent/mecha")
        await self.exec_as_root(environment, "chmod +x /installed-agent/mecha")

        # The host, as seen from inside this task's network. Compose networks
        # get different gateways per task, so it is discovered here rather
        # than assumed; /proc/net/route is the fallback for images without
        # iproute2 (the gateway field is little-endian hex).
        # The fallback avoids every non-POSIX construct: `${var:6:2}` is bash
        # (slim images' /bin/sh is dash), and awk's strtonum is gawk (slim
        # images ship mawk). `cut` plus printf-with-hex works in dash and
        # busybox alike, and the images that need the fallback — the ones
        # without iproute2 — are exactly the slim ones.
        gateway = (
            await self.exec_as_agent(
                environment,
                "if command -v ip >/dev/null 2>&1; then "
                "ip route | awk '/^default/ {print $3; exit}'; "
                "else gw=$(awk '$2==\"00000000\" {print $3; exit}' /proc/net/route); "
                'printf "%d.%d.%d.%d" '
                '"0x$(echo "$gw" | cut -c7-8)" "0x$(echo "$gw" | cut -c5-6)" '
                '"0x$(echo "$gw" | cut -c3-4)" "0x$(echo "$gw" | cut -c1-2)"; '
                "fi",
            )
        ).stdout.strip()
        if not gateway:
            raise RuntimeError("could not determine the container's default gateway")

        # The benchmark posture. context_window is what derives the compaction
        # threshold; shell_timeout is generous because TB tasks build things.
        #
        # max_tokens has to leave room for an answer *after* the thinking. The
        # server caps thinking at 4096 (`--reasoning-budget`, in
        # scripts/start-moe-mtp.sh) because this model otherwise reasons
        # without terminating on hard tasks and returns empty content — see
        # that script for the measurements. 8192 leaves ~4k for the answer
        # after a full-budget think, inside a 32k window.
        #
        # Raising max_tokens alone does NOT fix it and makes it worse: at
        # 32768 with unbounded thinking, one turn ran 20 minutes and hit the
        # harness's agent timeout without finishing a single turn. The budget
        # is the fix; this number just has to be bigger than it. Raising the
        # server's -c to give thinking room was also a dead end — it cost a
        # 50x slowdown; see scripts/start-moe-mtp.sh.
        #
        # context_window tracks scripts/start-moe-mtp.sh's `-c`. Change them
        # together or the derived compaction threshold is a lie.
        config = f"""\
default_provider = "local"

[providers.local]
kind = "local"
base_url = "http://{gateway}:{FORWARD_PORT}"
model = "qwen3.6-35b-a3b"
context_window = 32768

[agent]
max_tokens = 8192

[tools]
permission_mode = "allow"
shell_timeout_secs = 600
"""
        await self.exec_as_agent(
            environment,
            'mkdir -p "$HOME/.mecha" && cat > "$HOME/.mecha/config.toml" << \'MECHA_EOF\'\n'
            + config
            + "MECHA_EOF",
        )

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        model = ""
        if self.model_name and "/" in self.model_name:
            # harbor names models provider/model; the config holds the
            # provider, mecha gets the model half.
            model = f"-m {shlex.quote(self.model_name.split('/', 1)[1])} "
        # The declared CLI_FLAGS (max_turns) — declaring them without applying
        # them here would make the kwargs silent no-ops.
        flags = self.build_cli_flags()
        flags = f"{flags} " if flags else ""

        try:
            # stderr goes to a file beside the transcript, not to Harbor's
            # capture: Harbor recorded `stderr: None` for every 2026-08-07
            # trial, which silently discarded the one channel carrying
            # mecha's compaction notices and tracing — the exact evidence
            # needed to diagnose the trials that died. MECHA_LOG=debug is
            # cheap here (one run per container) and the log downloads with
            # the sessions below.
            await self.exec_as_agent(
                environment,
                f"mkdir -p {SESSION_DIR} && "
                f"/installed-agent/mecha run --yes {model}{flags}{shlex.quote(instruction)} "
                f"2> {SESSION_DIR}/stderr.log",
                env={"MECHA_SESSION_DIR": SESSION_DIR, "MECHA_LOG": "debug"},
            )
        finally:
            # The transcript is the trajectory; pull it out even when the run
            # failed, because the failed runs are the ones worth reading.
            try:
                await environment.download_dir(SESSION_DIR, self.logs_dir / "sessions")
            except Exception:
                pass  # a run that died before its first turn has no sessions

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        sessions = sorted((self.logs_dir / "sessions").glob("*.jsonl"))
        if not sessions:
            return
        # One `mecha run` per trial, so the newest transcript is the trial's.
        usage, turns = None, None
        for line in sessions[-1].read_text().splitlines():
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue  # a torn final line is the normal crash artifact
            if record.get("record") == "summary":
                usage = record.get("usage") or {}
                turns = record.get("turns")
        if usage is None:
            return
        cache_read = usage.get("cache_read_input_tokens", 0)
        cache_write = usage.get("cache_creation_input_tokens", 0)
        context.n_input_tokens = usage.get("input_tokens", 0) + cache_read + cache_write
        context.n_cache_tokens = cache_read or None
        context.n_output_tokens = usage.get("output_tokens", 0)
        context.metadata = {"turns": turns, "session": sessions[-1].name}
