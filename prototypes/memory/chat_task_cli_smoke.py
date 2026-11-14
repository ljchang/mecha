#!/usr/bin/env python3
"""Authored, encrypted task-chat qualification. No models or owner data are used.

The SQL reader is independent of the runtime API. A fresh process must recover
an open declaration omitted from the latest visible plan. This measures durable
mechanics, not whether a model chooses a good plan.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading
import time

from chat_cli_smoke import AuthoredTransport, Terminal, TIMEOUT, final, last_tool, records, system, tool, write_private

A, B, C = "fixtureplanamber", "fixtureplanbirch", "fixtureplancedar"
CHECK_A, CHECK_B = "printf fixture > check-a-executed", "printf fixture > check-b-executed"
OBSERVATION = "fixtureordinarycobalt: the authored transport window opens at 16:00."
DRAFT = {"title": "Synthetic durable task", "intent": "Keep the authored fixture work organized."}
REPORT_PREFIX = "mecha: episodic chat memory "
CHARTERS = ["Prepare the synthetic handoff.", "Prioritize the synthetic transport window."]


def item(content, status, check=None):
    value = {"content": content, "status": status}
    if check is not None:
        value["check"] = check
    return value


FIRST = {"items": [item(A, "in_progress", CHECK_A), item(B, "pending", CHECK_B), item(C, "pending")]}
SECOND = {"items": [item(A, "completed", CHECK_A), item(C, "in_progress")]}
THIRD = {"items": [item(A, "completed", CHECK_A), item(C, "in_progress"), item(B, "completed", "printf changed > check-b-executed")]}


class ParkedTransport(AuthoredTransport):
    def __init__(self, root):
        self.park_after = None
        self.parked, self.release = threading.Event(), threading.Event()
        super().__init__(root)

    def before_response(self, request):
        if self.park_after is not None and len(self.requests) == self.park_after:
            self.parked.set()
            if not self.release.wait(TIMEOUT):
                raise TimeoutError("driver did not release parked response")
            return False  # The driver reaped the intentionally killed CLI.


def sha(value):
    return hashlib.sha256(value).hexdigest()


def reports(path):
    return [json.loads(line[len(REPORT_PREFIX):]) for line in path.read_text().splitlines() if line.startswith(REPORT_PREFIX)]


def await_report(child, count):
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        found = reports(child.stderr_path)
        if len(found) >= count:
            assert found[-1]["status"] == "retained", found[-1]
            return found[-1]
        assert child.child.poll() is None, child.stderr_path
        time.sleep(0.025)
    raise TimeoutError(f"missing durable report: {child.stderr_path}")


def payloads(snapshot, kind):
    return [(r, json.loads(r["payload"])["data"]) for r in snapshot["revisions"] if r["kind"] == kind]


def verify_store(snapshot, expected_plans, pending=False):
    assert snapshot["schema_version"] == 10
    task_row, task = payloads(snapshot, "task")[0]
    assert len(payloads(snapshot, "task")) == 1
    assert {k: task[k] for k in DRAFT} == DRAFT and task["status"] == "open"
    assert len(snapshot["tasks"]) == 1 and snapshot["tasks"][0]["operation"] == "create"
    plans = payloads(snapshot, "plan")
    assert len(plans) == len(snapshot["plans"]) == expected_plans
    assert len({row["object_id"] for row, _ in plans}) <= 1
    previous = None
    for row, plan in plans:
        assert row["previous"] == previous
        assert plan["task"]["reference"] == {"object": task_row["object_id"], "revision": task_row["id"]}
        assert {"output": row["id"], "input": task_row["id"]} in snapshot["dependencies"]
        operation, = [o for o in snapshot["plans"] if o["after_revision"] == row["id"]]
        assert operation["state"] == ("pending" if pending else "audited")
        assert operation["before_revision"] == previous
        request = plan["request"]
        assert operation["turn_id"] == request["occurrence"]["turn_id"]
        assert operation["ordinal"] == request["occurrence"]["ordinal"]
        assert sha(request["feedback"]["text"].encode()) == request["feedback"]["digest"]
        feedback = json.loads(request["feedback"]["text"])
        assert len(feedback["decisions"]) == 1
        assert all(s["verification"] in ("not_declared", "skipped") and not s.get("criterion") for s in feedback["steps"])
        previous = row["id"]
    return plans


def verify_native(transcript, snapshot, turn_reports, authority, requests):
    native = records(transcript)
    markers = [r["marker"] for r in native if r.get("record") == "chat_memory"]
    assert [m["protocol"] for m in markers] == ["ordinary-chat-task-start-v1", "ordinary-chat-memory-v4"]
    attachment, = [json.loads(p.read_bytes()) for p in authority.iterdir() if p.is_file()
                   and p.stat().st_size <= 65536 and sha(p.read_bytes()) == markers[-1]["attachment"]]
    task_row, task = payloads(snapshot, "task")[0]
    assert task["account"] == "fixture-task-cli" and task["session_id"] == transcript.stem
    assert attachment["task"]["task"]["reference"] == {"object": task_row["object_id"], "revision": task_row["id"]}
    assert attachment["task"]["tools"] == ["fs_read", "goal_context", "todo"]
    assert attachment["task"]["startup_digest"] == markers[0]["attachment"]
    plans = {row["id"]: plan for row, plan in payloads(snapshot, "plan")}
    operations = {p["after_revision"]: p for p in snapshot["plans"]}
    seen = set()
    uses = {}
    # Decision bindings contain historical message copies. Audit canonical
    # message emissions only, preserving duplicate detection within that stream.
    def walk(value):
        if isinstance(value, dict):
            if value.get("type") == "tool_use":
                uses[value["id"]] = value
            if value.get("plan_results"):
                feedback = value["planning"]
                for audit in value["plan_results"]:
                    revision = audit["revision_id"]
                    assert revision not in seen
                    seen.add(revision)
                    request = plans[revision]["request"]
                    operation = operations[revision]
                    assert audit["receipt_id"] == operation["receipt_id"]
                    assert (audit["turn_id"], audit["ordinal"]) == (operation["turn_id"], operation["ordinal"])
                    result = value["content"][audit["result_index"]]
                    assert result["content"] == request["result_text"]
                    assert result["tool_use_id"] == request["provider_call_id"]
                    invocation = uses[request["provider_call_id"]]
                    assert invocation["name"] == "todo"
                    assert json.dumps(invocation["input"], ensure_ascii=False, separators=(",", ":")) == request["input_json"]
                    stored = json.loads(request["feedback"]["text"])
                    assert feedback["steps"][audit["steps_start"]:audit["steps_end"]] == stored["steps"]
                    assert feedback["decisions"][audit["decisions_start"]:audit["decisions_end"]] == stored["decisions"]
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    assert not any(r.get("record") == "rewrite" for r in native), "bounded task profile rewrote history"
    walk([r for r in native if r.get("record") == "message"])
    assert seen == set(plans)
    serialized_requests = json.dumps(requests)
    for operation in operations.values():
        assert operation["receipt_id"] not in serialized_requests
        assert operation["after_revision"] not in serialized_requests
    decisions = [r for r in native if r.get("record") == "goal_context_decision"]
    assert len(decisions) == len(requests) == 6
    versions = {v["revision"]: v for d in decisions for v in d["versions"]}
    for decision, request in zip(decisions, requests):
        assert decision["binding"]["system"] == system(request)
        assert versions[decision["binding"]["version"]]["owner_inputs"][-1]["source"] == "cli_chat"
    for _, plan in payloads(snapshot, "plan"):
        request = plan["request"]
        matches = [d for d in decisions if sha(json.dumps(d["binding"], ensure_ascii=False, separators=(",", ":")).encode()) == request["goal_decision_digest"]]
        assert len(matches) == 1
        version = versions[matches[0]["binding"]["version"]]
        assert sha(json.dumps(version, ensure_ascii=False, separators=(",", ":")).encode()) == request["goal_version_digest"]
    original_bytes = transcript.read_bytes()
    ordinary = []
    captures = [c for c in snapshot["captures"] if c["state"] == "active"]
    assert len(captures) == len(turn_reports) == 3
    for report in turn_reports:
        assert report["protocol"] == "ordinary-chat-memory-v4" and report["semantic_assessment"] == "not_evaluated"
        receipt = report["receipt"]
        capture, = [c for c in captures if c["receipt_id"] == receipt["id"]]
        assert {"object": capture["projection_object"], "revision": capture["projection_revision"]} == receipt["projection"]
        assert json.loads(capture["originals"]) == receipt["originals"]
        revision_ids = {r["id"] for r in snapshot["revisions"]}
        assert receipt["projection"]["revision"] in revision_ids
        assert all(r["revision"] in revision_ids for r in receipt["originals"])
        run_id = report["run_id"]
        raw = (authority / (run_id + ".chat-seal")).read_bytes()
        assert sha(raw) == (authority / (run_id + ".chat-seal-hash")).read_text()
        seal = json.loads(raw)
        assert sha(original_bytes[:seal["length"]]) == seal["digest"]
        originals = seal["payload"]["originals"]
        for original in originals:
            assert all(token not in original["text"] for token in (B, C, CHECK_A, CHECK_B))
            if A in original["text"]:
                # The independent negative-search invocation is ordinary action
                # evidence. It is not a second copy of the todo input or echo.
                assert original["kind"] == "ActionInvocation"
                invocation = json.loads(original["text"])
                assert invocation["tool_name"] == "goal_context"
                assert invocation["input"] == {"memory": {"query": A}}
        for original in originals:
            if original["kind"] == "ToolResult" and OBSERVATION in original["text"]:
                value = native[original["line"] - 1]
                for part in original["pointer"].split("/")[1:]:
                    part = part.replace("~1", "/").replace("~0", "~")
                    value = value[int(part)] if isinstance(value, list) else value[part]
                assert OBSERVATION in value
                ordinary.append(original)
    assert len(ordinary) == 1
    assert all(token not in json.dumps(snapshot["fts"]) for token in (B, C, CHECK_A, CHECK_B))
    for indexed in snapshot["fts"]:
        if A in indexed["text"]:
            invocation = json.loads(indexed["text"])
            assert invocation["tool_name"] == "goal_context" and invocation["input"] == {"memory": {"query": A}}
    assert OBSERVATION in json.dumps(snapshot["fts"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--inspect", required=True, type=Path)
    args = parser.parse_args()
    binary, inspector, root = args.binary.resolve(strict=True), args.inspect.resolve(strict=True), args.output.resolve()
    root.mkdir(mode=0o700)
    home, mecha, work, storage, keys = [root / n for n in ("home", "mecha", "work", "storage", "keys")]
    for directory in (home, mecha, work, storage, keys):
        directory.mkdir(mode=0o700)
    env = {"HOME": str(home), "MECHA_HOME": str(mecha), "TERM": "xterm", "PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8"}
    transport = ParkedTransport(root)
    manifest, draft = root / "request.json", root / "task.json"
    authority, db, key = storage / "authority", storage / "fixture.memory-prototype.sqlite", keys / "fixture.key"
    request = {"protocol": "ordinary-session-memory-v1", "db": str(db), "key_file": str(key), "authority": str(authority), "domain": "synthetic-demo", "account": "fixture-task-cli", "scopes": ["fixture-mail"]}
    write_private(manifest, json.dumps(request))
    write_private(draft, json.dumps(DRAFT))
    write_private(work / "observation.txt", OBSERVATION)
    graph_source = Path(__file__).resolve().parents[2] / "mecha-core/tests/fixtures/selected_graph_mcp.py"
    graph = root / "mecha-graph-mcp"
    write_private(graph, graph_source.read_text())
    graph.chmod(0o700)
    sentinel = root / "graph-started"
    write_private(mecha / "config.toml", f'''default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
base_url = "http://127.0.0.1:{transport.server.server_port}"
model = "fixture"
max_retries = 0
[features]
graph = true
[tools]
permission_mode = "allow"
enabled = ["fs_read", "goal_context", "todo"]
[[mcp]]
name = "selected"
command = {json.dumps(str(graph))}
[mcp.env]
GRAPH_FIXTURE_MARKER = {json.dumps(str(sentinel))}
''')
    command = [str(binary), "chat", "--no-mcp", "--no-code-tool", "--no-grants", "--no-learned-rules", "--no-past-appraisals", "--no-success-examples", "--no-messages", "--no-hooks", "--no-fallback", "--no-outbox"]
    command += [flag for name in ("fs_read", "goal_context", "todo") for flag in ("--tool", name)]
    enabled = command + ["--episodic-memory", str(manifest), "--dynamic-goals"]
    memory = ["memory-session", "--request", str(manifest)]

    def run(label, tail):
        result = subprocess.run([str(binary), *tail], cwd=work, env=env, capture_output=True, text=True, timeout=TIMEOUT)
        write_private(root / (label + ".stdout"), result.stdout)
        write_private(root / (label + ".stderr"), result.stderr)
        assert result.returncode == 0, (label, result.stderr)
        return json.loads(result.stdout)

    def inspect(label):
        result = subprocess.run([str(inspector), str(db), str(key)], env=env, capture_output=True, text=True, timeout=TIMEOUT)
        assert result.returncode == 0, result.stderr
        write_private(root / (label + ".sql.json"), result.stdout)
        return json.loads(result.stdout)

    def revise(index):
        temporary = mecha / "charter.tmp"
        write_private(temporary, '[[line]]\nid="fixture"\ntext=' + json.dumps(CHARTERS[index]) + "\n")
        temporary.replace(mecha / "charter.toml")

    def reject(label, argv, transcript=None):
        count = len(transport.requests)
        before = transcript.read_bytes() if transcript else None
        stored = inspect(label + "-before")
        with Terminal(label, argv, root, work, env) as child:
            assert child.child.wait(timeout=TIMEOUT) != 0, label
        assert len(transport.requests) == count and not transport.errors
        if transcript:
            assert transcript.read_bytes() == before
        assert stored == inspect(label + "-after"), "refused startup changed stored memory"
        assert not sentinel.exists(), "task profile started owner graph MCP"

    try:
        revise(0)
        assert run("init", memory + ["init"])["status"] == "initialized"
        assert inspect("v8")["schema_version"] == 8
        reject("requires-migration", enabled + ["--memory-task", str(draft)])
        assert inspect("still-v8")["schema_version"] == 8
        run("enable", memory + ["enable-planning"])
        before = inspect("enabled")
        assert before["schema_version"] == 10
        run("enable-again", memory + ["enable-planning"])
        assert before == inspect("enabled-again")
        wrong_key = keys / "wrong.key"
        write_private(wrong_key, "ab" * 32)
        result = subprocess.run([str(inspector), str(db), str(wrong_key)], env=env, capture_output=True, timeout=TIMEOUT)
        assert result.returncode != 0, "wrong SQLCipher key read fixture"
        prior_sessions = set((mecha / "sessions").glob("*.jsonl"))
        batch = tool("todo", FIRST, "fixture_write_1")
        batch["tool_calls"] += tool("todo", SECOND, "fixture_write_2")["tool_calls"]
        batch["tool_calls"] += tool("fs_read", {"path": "observation.txt"}, "fixture_read_1")["tool_calls"]
        recall_batch = tool("goal_context", {"memory": {"query": "fixtureordinarycobalt"}}, "fixture_recall_positive")
        recall_batch["tool_calls"] += tool("goal_context", {"memory": {"query": A}}, "fixture_recall_negative")["tool_calls"]
        transport.script("initial", [batch, final("Fixture plan stored."), recall_batch, final("Fixture goal changed.")])
        with Terminal("initial", enabled + ["--memory-task", str(draft)], root, work, env) as child:
            child.await_text("/help for commands")
            transcript, = set((mecha / "sessions").glob("*.jsonl")) - prior_sessions
            verify_store(inspect("created"), 0)
            child.send("Organize the synthetic work and read the authored observation.")
            child.await_text("Fixture plan stored.")
            first_report = await_report(child, 1)
            initial = inspect("two-writes")
            plans = verify_store(initial, 2)
            assert plans[-1][1]["request"]["snapshot"]["declarations"][B] == {"check": CHECK_B, "frozen": False}
            assert B not in [i["content"] for i in plans[-1][1]["request"]["snapshot"]["items"]]
            revise(1)
            child.send("Change direction: prioritize the transport window.")
            child.await_text("Fixture goal changed.")
            await_report(child, 2)
            assert initial["plans"] == inspect("zero-todo")["plans"]
            child.send("/quit")
            child.wait()
        transport.assert_consumed()
        reject("default-resume", command + ["--resume", transcript.stem], transcript)
        wrong = root / "wrong-request.json"
        write_private(wrong, json.dumps({**request, "authority": str(storage / "wrong-authority")}))
        reject("wrong-authority", command + ["--episodic-memory", str(wrong), "--dynamic-goals", "--memory-task-resume", "--resume", transcript.stem], transcript)
        reject("graph-profile", [v for v in enabled if v != "--no-mcp"] + ["--memory-graph-server", "selected", "--memory-task-resume", "--resume", transcript.stem], transcript)
        transport.script("resumed", [tool("todo", THIRD, "fixture_write_1"), final("Fixture resumed plan stored.")])
        with Terminal("resumed", enabled + ["--memory-task-resume", "--resume", transcript.stem], root, work, env) as child:
            child.await_text("/help for commands")
            child.send("Resume the synthetic work and update its plan.")
            child.await_text("Fixture resumed plan stored.")
            await_report(child, 1)
            child.send("/quit")
            child.wait()
        transport.assert_consumed()
        snapshot = inspect("resumed")
        plans = verify_store(snapshot, 3)
        first, second, last = [p["request"] for _, p in plans]
        assert first["occurrence"]["turn_id"] == second["occurrence"]["turn_id"]
        assert [first["occurrence"]["ordinal"], second["occurrence"]["ordinal"]] == [0, 1]
        assert first["provider_call_id"] == last["provider_call_id"] == "fixture_write_1"
        for saved, authored in zip((first, second, last), (FIRST, SECOND, THIRD)):
            expected_items = [{"expect": None, "check": None, "expect_calls": None, **entry} for entry in authored["items"]]
            if saved is last:
                expected_items[-1]["check"] = CHECK_B
            assert saved["snapshot"]["items"] == expected_items and saved["snapshot"]["serves"] is None
        for saved, frozen_a, frozen_b in ((first, False, False), (second, True, False), (last, True, True)):
            assert saved["snapshot"]["declarations"] == {A: {"check": CHECK_A, "frozen": frozen_a}, B: {"check": CHECK_B, "frozen": frozen_b}}
        assert json.loads(second["feedback"]["text"])["steps"][0]["actual_calls"] is None
        last = plans[-1][1]["request"]
        assert last["snapshot"]["declarations"][B] == {"check": CHECK_B, "frozen": True}
        assert next(i for i in last["snapshot"]["items"] if i["content"] == B)["check"] == CHECK_B
        feedback = json.loads(last["feedback"]["text"])
        assert any(s["step"] == B and s["check_tampered"] and s["verification"] == "skipped" for s in feedback["steps"])
        assert plans[0][1]["request"]["occurrence"]["turn_id"] != last["occurrence"]["turn_id"]
        assert all(not (work / name).exists() for name in ("check-a-executed", "check-b-executed"))
        live_requests = [r["request"] for r in transport.requests]
        for provider_request in live_requests:
            assert sorted(t["function"]["name"] for t in provider_request["tools"]) == ["fs_read", "goal_context", "todo"]
        results = [m for m in live_requests[1]["messages"] if m["role"] == "tool"]
        assert [m["tool_call_id"] for m in results] == ["fixture_write_1", "fixture_write_2", "fixture_read_1"]
        assert [m["content"] for m in results[:2]] == [first["result_text"], second["result_text"]]
        resumed_result = [m for m in live_requests[5]["messages"] if m["role"] == "tool"][-1]
        assert resumed_result["tool_call_id"] == "fixture_write_1" and resumed_result["content"] == last["result_text"]
        assert CHARTERS[0] in system(live_requests[0]) and CHARTERS[1] in system(live_requests[2])
        turn_reports = reports(root / "initial.stderr") + reports(root / "resumed.stderr")
        verify_native(transcript, snapshot, turn_reports, authority, live_requests)
        recalled = [m for m in live_requests[3]["messages"] if m["role"] == "tool"][-2:]
        assert [m["tool_call_id"] for m in recalled] == ["fixture_recall_positive", "fixture_recall_negative"]
        assert OBSERVATION in recalled[0]["content"], "authorized goal_context did not recall ordinary source"
        assert transcript.stem + ":" in recalled[0]["content"], "recalled source lost its conversation locator"
        # Compare returned records, not the query echoed in the reply envelope.
        assert last_tool(live_requests[3])["memory"]["records"] == [], "generic recall exposed private plan content"
        source_seal = json.loads((authority / (first_report["run_id"] + ".chat-seal")).read_text())
        original, reference = next((o, r) for o, r in zip(source_seal["payload"]["originals"], first_report["receipt"]["originals"])
                                   if o["kind"] == "ToolResult" and o["text"] == OBSERVATION)
        positive = last_tool({"messages": [recalled[0]]})["memory"]["records"]
        assert any(hit["reference"] == reference and hit["record"]["kind"] == "evidence"
                   and hit["record"]["data"]["source"] == original["source"]
                   and hit["record"]["data"]["text"] == original["text"] for hit in positive)
        # A bare memory reader has no external-source authority. Withholding the
        # same host evidence there is the required fail-closed control.
        recall = ["memory", "--experimental", "--db", str(db), "--key-file", str(key), "recall"]
        assert run("recall-without-authority", recall + ["fixtureordinarycobalt", "--account", "fixture-task-cli", "--neighbors", "0"]) == []
        original = transcript.read_bytes()
        retry = run("retry-retained", memory + ["retry", first_report["run_id"]])
        assert retry["status"] == "retained" and retry["receipt"] == first_report["receipt"]
        assert transcript.read_bytes() == original and snapshot == inspect("after-retry")
        assert len(transport.requests) == 6 and not transport.errors and not sentinel.exists()
        # The next provider request places a real process stop after SQL commit,
        # before the final native suffix has been prepared.
        sessions_before = set((mecha / "sessions").glob("*.jsonl"))
        transport.script("crash-after-commit", [tool("todo", FIRST, "fixture_crash_write"), final("This response must never be delivered.")])
        transport.park_after = len(transport.requests) + 2
        with Terminal("crash", enabled + ["--memory-task", str(draft)], root, work, env) as child:
            child.await_text("/help for commands")
            crashed, = set((mecha / "sessions").glob("*.jsonl")) - sessions_before
            child.send("Organize a separate synthetic task.")
            assert transport.parked.wait(TIMEOUT), "provider barrier was not reached"
            committed = inspect("crash-pending")
            pending, = [p for p in committed["plans"] if p["state"] == "pending"]
            assert len(committed["plans"]) == 4 and len(payloads(committed, "task")) == 2
            child.child.kill()
            assert child.child.wait(timeout=TIMEOUT) < 0
            transport.release.set()
        transport.assert_consumed()
        crash_bytes = crashed.read_bytes()
        recovery = run("retry-incomplete", memory + ["retry", pending["turn_id"]])
        assert recovery["status"] == "pending" and recovery["receipt"] is None
        assert recovery["reason"] == "recording_unavailable"
        assert committed == inspect("crash-after-retry") and crashed.read_bytes() == crash_bytes
        reject("pending-resume", enabled + ["--memory-task-resume", "--resume", crashed.stem], crashed)
        assert len(transport.requests) == 8 and not transport.errors and not sentinel.exists()
        assert all(not (work / name).exists() for name in ("check-a-executed", "check-b-executed"))
        summary = {"status": "passed", "happy_path_provider_requests": 6, "crash_provider_requests": 2, "happy_path_task_revisions": 1, "happy_path_plan_revisions": 3, "retained_turns": 3, "incomplete_crash_left_pending": True, "checks_executed": 0, "owner_graph_launches": 0, "semantic_benefit_measured": False}
        write_private(root / "summary.json", json.dumps(summary, indent=2))
        print(json.dumps(summary))
    finally:
        transport.release.set()
        transport.close()


if __name__ == "__main__":
    main()
