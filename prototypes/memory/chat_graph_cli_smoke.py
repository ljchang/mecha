#!/usr/bin/env python3
"""Actual opted-in CLI chat with an authored graph MCP and scripted transport.

No model or owner graph is queried. Exercises retain/recall/resume and rejects
changed launch/inventory before provider requests or native transcript writes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from chat_cli_smoke import AuthoredTransport, Terminal, TIMEOUT, final, last_tool, records, tool, write_private

SNAPSHOT = "Authored snapshot: project Cedar is active."
# Mechanics query targets returned text. The generic query "Cedar" can rank
# the three query invocations above their longer result passages at limit=3;
# that observed quality limitation is not a source-retention failure.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary, root = args.binary.resolve(strict=True), args.output.resolve()
    root.mkdir(mode=0o700)
    home, mecha, work, storage, keys = [root / p for p in ("home", "mecha", "work", "storage", "keys")]
    for directory in (home, mecha, work, storage, keys):
        directory.mkdir(mode=0o700)
    env = {"HOME": str(home), "MECHA_HOME": str(mecha), "TERM": "xterm", "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8"}
    fixture = root / "mecha-graph-mcp"
    source = Path(__file__).resolve().parents[2] / "mecha-core/tests/fixtures/selected_graph_mcp.py"
    # A fixed configured path lets live inventory change without changing launch
    # config, exactly the distinction the resume preflight must check.
    fixture.write_text(source.read_text().replace(
        'mode = os.environ.get("GRAPH_FIXTURE_MODE", "ok")',
        'mode = open(os.environ["GRAPH_MODE_FILE"], encoding="utf-8").read().strip()'))
    fixture.chmod(0o700)
    mode_file, spawn_file, other_spawn = [root / p for p in ("mode", "spawned", "unrelated-spawned")]
    write_private(mode_file, "ok")
    write_private(work / "observation.txt", "Authored local fixture for the project.\n")
    write_private(mecha / "charter.toml", '[[line]]\nid = "fixture"\ntext = "Check authored project evidence."\n')
    transport = AuthoredTransport(root)
    config_path = mecha / "config.toml"

    def config(extra=""):
        write_private(config_path, f'''default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
base_url = "http://127.0.0.1:{transport.server.server_port}"
model = "fixture"
max_retries = 0
[features]
graph = true
[tools]
permission_mode = "allow"
enabled = ["fs_read", "goal_context"]
[[mcp]]
name = "selected"
command = {json.dumps(str(fixture))}
[mcp.env]
GRAPH_MODE_FILE = {json.dumps(str(mode_file))}
GRAPH_FIXTURE_MARKER = {json.dumps(str(spawn_file))}
{extra}
[[mcp]]
name = "unrelated"
command = {json.dumps(str(fixture))}
[mcp.env]
GRAPH_MODE_FILE = {json.dumps(str(mode_file))}
GRAPH_FIXTURE_MARKER = {json.dumps(str(other_spawn))}
''')

    config()
    manifest = root / "request.json"
    authority = storage / "authority"
    write_private(manifest, json.dumps({"protocol": "ordinary-session-memory-v1", "db": str(storage / "fixture.memory-prototype.sqlite"), "key_file": str(keys / "fixture.key"), "authority": str(authority), "domain": "fixture", "account": "fixture", "scopes": []}))

    def run(label, tail):
        with (root / (label + ".stdout")).open("w") as out, (root / (label + ".stderr")).open("w") as err:
            return subprocess.run([str(binary), *tail], cwd=work, env=env, stdout=out, stderr=err, timeout=TIMEOUT).returncode

    restrictions = ["--no-code-tool", "--no-grants", "--no-learned-rules", "--no-past-appraisals", "--no-success-examples", "--no-messages", "--no-hooks", "--no-fallback", "--no-outbox"]
    names = ["fs_read", "goal_context", "selected__kg_search", "selected__kg_entity", "selected__kg_task_list"]
    command = [str(binary), "chat", *restrictions, "--dynamic-goals", "--episodic-memory", str(manifest), "--memory-graph-server", "selected"]
    command += [flag for name in names for flag in ("--tool", name)]
    rejected = []

    def reject(label, argv, transcript, may_spawn=False):
        before = transcript.read_bytes()
        count = len(transport.requests)
        spawns = spawn_file.read_text() if spawn_file.exists() else ""
        with Terminal(label, argv, root, work, env) as child:
            assert child.child.wait(timeout=TIMEOUT) != 0, label + " unexpectedly started"
        assert len(transport.requests) == count and not transport.errors, label + " reached provider"
        assert transcript.read_bytes() == before, label + " modified source"
        if not may_spawn:
            assert spawn_file.read_text() == spawns, label + " spawned a child"
        assert not other_spawn.exists(), "an unrelated configured server was started"
        rejected.append(label)

    try:
        assert run("initialize", ["memory-session", "--request", str(manifest), "init"]) == 0
        first = tool("fs_read", {"path": "observation.txt"}, "read_local")
        for index, name in enumerate(names[2:]):
            first["tool_calls"].extend(tool(name, {"query": "Cedar"}, "graph_" + str(index))["tool_calls"])
        transport.script("initial", [first, final("Authored graph observation complete."), tool("goal_context", {"memory": {"query": "Cedar active"}}, "recall_2"), final("Authored changed goal checked.")])
        with Terminal("initial", command, root, work, env) as child:
            child.await_text("/help for commands")
            transcript, = (mecha / "sessions").glob("*.jsonl")
            reject("duplicate-writer", command + ["--resume", transcript.stem], transcript)
            child.send("Read the authored local file and all three selected graph readers.")
            child.await_text("Authored graph observation complete.")
            child.send("Change the goal to evaluating Cedar's project status; recall the graph observation.")
            child.await_text("Authored changed goal checked.")
            child.send("/quit"); child.wait()
        transport.assert_consumed()
        resumed = command + ["--resume", transcript.stem]
        missing = list(resumed)
        i = missing.index("--memory-graph-server"); del missing[i:i + 2]
        missing.append("--no-mcp")
        reject("missing-graph-opt-in", missing, transcript)
        wrong = list(resumed); wrong[wrong.index("--memory-graph-server") + 1] = "unrelated"
        reject("wrong-server", wrong, transcript)
        config('AUTHORED_LAUNCH_CHANGE = "changed"')
        reject("changed-launch", resumed, transcript)
        config()
        for mode in ("description", "missing", "open", "destructive", "deferred", "duplicate", "nested_schema", "bad_annotations"):
            write_private(mode_file, mode)
            reject("inventory-" + mode, resumed, transcript, may_spawn=True)
        write_private(mode_file, "ok")
        transport.script("resumed", [tool("goal_context", {"memory": {"query": "Cedar active"}}, "recall_3"), final("Authored resumed recall complete.")])
        with Terminal("resumed", resumed, root, work, env) as child:
            child.await_text("/help for commands")
            child.send("Continue with the revised objective using the stored Cedar observation.")
            child.await_text("Authored resumed recall complete.")
            child.send("/quit"); child.wait()
        transport.assert_consumed()
        native = records(transcript)
        marker, = [r["marker"] for r in native if r.get("record") == "chat_memory"]
        assert marker["protocol"] == "ordinary-chat-memory-v2"
        # Locate the private attachment by the marker's authenticated content,
        # not an assumed basename. Other authority files are separate records.
        attachments = []
        for path in authority.iterdir():
            if path.is_file() and path.stat().st_size <= 65536:
                data = path.read_bytes()
                if hashlib.sha256(data).hexdigest() == marker["attachment"]:
                    attachments.append(json.loads(data))
        attachment, = attachments
        surface = attachment["graph_surface"]
        assert surface["server_name"] == "selected"
        assert surface["local_tools"] == ["fs_read", "goal_context"]
        assert [t["local_name"] for t in surface["graph_tools"]] == sorted(names[2:])
        assert all(t["private_data"] and t["untrusted_input"] and t["effect"] == "read_that_records" for t in surface["graph_tools"])
        reports = []
        for log in ("initial.stderr", "resumed.stderr"):
            for line in (root / log).read_text().splitlines():
                prefix = "mecha: episodic chat memory "
                if line.startswith(prefix):
                    reports.append(json.loads(line[len(prefix):]))
        assert len(reports) == 3, reports
        assert all(r["status"] == "retained" and r["semantic_assessment"] == "not_evaluated" and r["protocol"] == "ordinary-chat-memory-v2" for r in reports), reports
        first_seal = json.loads((authority / (reports[0]["run_id"] + ".chat-seal")).read_text())
        originals = first_seal["payload"]["originals"]
        snapshots = [(o, r) for o, r in zip(originals, reports[0]["receipt"]["originals"]) if o["kind"] == "ToolResult" and SNAPSHOT in o["text"]]
        assert len(snapshots) == 3
        delivered = [e["request"] for e in transport.requests]
        assert len(delivered) == 6
        for request in (delivered[3], delivered[5]):
            recalled = last_tool(request)["memory"]["records"]
            assert any(hit["reference"] == reference and hit["record"]["data"].get("text") == original["text"] and hit["record"]["data"].get("source") == original["source"] for original, reference in snapshots for hit in recalled), "graph snapshot or original locator not recalled"
        for original, _ in snapshots:
            record = native[original["line"] - 1]
            value = record
            for part in original["pointer"].split("/")[1:]:
                part = part.replace("~1", "/").replace("~0", "~")
                value = value[int(part)] if isinstance(value, list) else value[part]
            assert value == original["text"], "snapshot differs from native delivered evidence"
        # Native provenance must come from dispatch, not inferred by the adapter.
        messages = [r for r in native if r.get("record") == "message"]
        for i in range(3):
            assert any(m.get("tool_provenance", {}).get("graph_" + str(i)) is True for m in messages)
        assert not other_spawn.exists()
        # Transport/nontext failures remain external failed observations. They
        # are not clean host refusals and cannot attest successful graph facts.
        transport.script("failed-read", [
            tool("selected__kg_search", {"mode": "wire_error"}, "failed_graph"),
            final("Authored graph failure reported."),
        ])
        with Terminal("failed-read", resumed, root, work, env) as child:
            child.await_text("/help for commands")
            child.send("Try the authored failing graph query and report the failure.")
            child.await_text("Authored graph failure reported.")
            child.send("/quit"); child.wait()
        transport.assert_consumed()
        failed_report, = [json.loads(line.split("mecha: episodic chat memory ", 1)[1])
                         for line in (root / "failed-read.stderr").read_text().splitlines()
                         if line.startswith("mecha: episodic chat memory ")]
        assert failed_report["status"] == "retained", failed_report
        failed_seal = json.loads((authority / (failed_report["run_id"] + ".chat-seal")).read_text())
        assert {"call_id": "failed_graph", "tool_name": "selected__kg_search", "status": "Failed"} in failed_seal["payload"]["receipts"]
        failed_native = records(transcript)
        assert any(r.get("tool_provenance", {}).get("failed_graph") is True for r in failed_native)
        # Existing passage bounds stay intact. Oversized evidence visibly
        # withholds this turn and closes chat rather than inventing retention.
        transport.script("large-read", [
            tool("selected__kg_search", {"mode": "large"}, "large_graph"),
            final("Authored large graph response handled."),
        ])
        with Terminal("large-read", resumed, root, work, env) as child:
            child.await_text("/help for commands")
            child.send("Read the authored oversized graph result.")
            child.await_text("Authored large graph response handled.")
            child.wait()
        transport.assert_consumed()
        large_report, = [json.loads(line.split("mecha: episodic chat memory ", 1)[1])
                        for line in (root / "large-read.stderr").read_text().splitlines()
                        if line.startswith("mecha: episodic chat memory ")]
        assert large_report["status"] != "retained" and large_report["receipt"] is None, large_report
        reject("resume-after-withheld-result", resumed, transcript)
        summary = {"status": "passed", "retained_turns": 4, "withheld_large_result": True, "provider_requests": 10, "retained_graph_snapshots": 3, "startup_refusals": rejected, "unrelated_server_started": False, "semantic_benefit_measured": False}
        write_private(root / "summary.json", json.dumps(summary, indent=2))
        print(json.dumps(summary))
    finally:
        transport.close()


if __name__ == "__main__":
    main()
