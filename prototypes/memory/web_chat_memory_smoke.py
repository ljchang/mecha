#!/usr/bin/env python3
"""Authored loopback qualification of memory on ORDINARY web chats. No real model.

Run with BINARY and a new OUTPUT directory. Drives the real `serve` binary with
`--chat-memory`: ordinary sends, ordinary tools, an ordinary resume after a
restart. Asserts each typed turn is sealed and retained under the open web
recorder, that a later turn recalls an earlier one, and that the store is not
needed for the chat to go on.
"""
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

if len(sys.argv) != 3:
    raise SystemExit('usage: web_chat_memory_smoke.py BINARY NEW_OUTPUT_DIR')
BINARY = str(Path(sys.argv[1]).resolve())
ROOT = Path(sys.argv[2]).resolve()
ROOT.mkdir(parents=True, exist_ok=False)
OWNER = 'fixture-owner@example.invalid'
LOCK = threading.Lock()
REQUESTS, QUEUE, ERRORS = [], [], []


def tool(name, arguments, call_id):
    return {'role': 'assistant', 'tool_calls': [{'index': 0, 'id': call_id, 'type': 'function', 'function': {'name': name, 'arguments': json.dumps(arguments)}}]}


def final(text):
    return {'role': 'assistant', 'content': text}


class Provider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['content-length'])))
        if not body.get('stream'):
            # A title request: never part of the scripted conversation.
            data = json.dumps({'model': body['model'], 'choices': [{'message': {'role': 'assistant', 'content': 'Fixture title'}, 'finish_reason': 'stop'}]}).encode()
            kind = 'application/json'
        else:
            with LOCK:
                REQUESTS.append(body)
                if not QUEUE:
                    ERRORS.append('unscripted request')
                    self.send_error(500)
                    return
                message = QUEUE.pop(0)
            finish = 'tool_calls' if 'tool_calls' in message else 'stop'
            chunk = {'id': 'fixture', 'model': body['model'], 'choices': [{'index': 0, 'delta': message, 'finish_reason': finish}], 'usage': {'prompt_tokens': 20, 'completion_tokens': 10, 'total_tokens': 30}}
            data = ('data: ' + json.dumps(chunk) + '\n\ndata: [DONE]\n\n').encode()
            kind = 'text/event-stream'
        self.send_response(200)
        self.send_header('Content-Type', kind)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


def private(path, text):
    path.write_text(text)
    path.chmod(0o600)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def wait_for(fn, seconds=35):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        answer = fn()
        if answer:
            return answer
        time.sleep(.05)
    raise AssertionError('timed out waiting for the authored fixture')


provider = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=provider.serve_forever, daemon=True).start()
HOME, MECHA, WORK, STORAGE, KEYS = [ROOT / p for p in ('home', 'mecha', 'work', 'storage', 'keys')]
for directory in (HOME, MECHA, WORK, STORAGE, KEYS):
    directory.mkdir(mode=0o700)
ENV = {'HOME': str(HOME), 'MECHA_HOME': str(MECHA), 'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'LANG': 'C.UTF-8', 'TERM': 'dumb'}
# An incognito room lives on a RAM-backed directory; without one the server
# refuses to open an incognito chat, and that step cannot be measured here.
if os.environ.get('XDG_RUNTIME_DIR'):
    ENV['XDG_RUNTIME_DIR'] = os.environ['XDG_RUNTIME_DIR']
MANIFEST = ROOT / 'request.json'
AUTHORITY = STORAGE / 'authority'
private(MANIFEST, json.dumps({'protocol': 'ordinary-session-memory-v1', 'db': str(STORAGE / 'fixture.memory-prototype.sqlite'), 'key_file': str(KEYS / 'fixture.key'), 'authority': str(AUTHORITY), 'domain': 'fixture', 'account': 'fixture', 'scopes': []}))
private(MECHA / 'charter.toml', '[[line]]\nid="fixture"\ntext="Keep the authored handoff notes."\n')
private(MECHA / 'config.toml', f'''default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
model = "fixture"
base_url = "http://127.0.0.1:{provider.server_port}"
max_retries = 0
[tools]
enabled = ["goal_context", "fs_read", "fs_list", "todo"]
permission_mode = "read-only"
[features]
web = true
[web]
owner_login = "{OWNER}"
''')
with (ROOT / 'init.log').open('w') as out:
    subprocess.run([BINARY, 'memory-session', '--request', str(MANIFEST), 'init'], cwd=WORK, env=ENV, stdout=out, stderr=out, check=True, timeout=35)


class Serve:
    def __init__(self, label, remembering=True):
        self.port = free_port()
        self.log = (ROOT / f'serve-{label}.log').open('w')
        args = [BINARY, 'serve', '--port', str(self.port), '--voice-port', '0', '--offer-target', '', '--dynamic-goals']
        if remembering:
            args += ['--chat-memory', str(MANIFEST)]
        self.proc = subprocess.Popen(args, cwd=WORK, env=ENV, stdout=self.log, stderr=self.log)

        def ready():
            assert self.proc.poll() is None, f'serve exited: inspect {self.log.name}'
            return self.request('/api/ping')[0] == 200
        wait_for(ready)

    def request(self, path, body=None, raw=None):
        headers = {'Content-Type': 'application/json', 'Tailscale-User-Login': OWNER, 'X-Mecha-Request': '1'}
        data = raw if raw is not None else (None if body is None else json.dumps(body).encode())
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=5) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as err:
            return err.code, err.read().decode()
        except urllib.error.URLError:
            return 0, ''

    def stop(self):
        self.proc.send_signal(signal.SIGTERM)
        try:
            self.proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
            raise AssertionError('serve did not drain')
        self.log.close()


def transcripts():
    return sorted((MECHA / 'sessions').glob('*.jsonl'))


def rows(path):
    try:
        return [json.loads(line) for line in path.read_text().splitlines()]
    except json.JSONDecodeError:
        return []


def retained():
    return sorted(AUTHORITY.glob('*.chat-retained'))


def seals():
    found = []
    for receipt in retained():
        run_id = receipt.name.split('.')[0]
        found.append(json.loads((AUTHORITY / f'{run_id}.chat-seal').read_text())['payload'])
    return sorted(found, key=lambda p: p['chat_context']['turn_ordinal'])


def turn(serve, key, text, script, expect):
    with LOCK:
        assert not QUEUE, 'previous script was not consumed'
        QUEUE.extend(script)
    status, body = serve.request(f'/api/chat/{key}/send', {'text': text, 'request_id': f'r{expect}'})
    assert status == 200, (status, body)
    wait_for(lambda: len(retained()) == expect)
    with LOCK:
        assert not QUEUE and not ERRORS, (QUEUE, ERRORS)


FIRST = 'Remember that the authored handoff code is coral.'
SECOND = 'What was the handoff code? Check memory.'
THIRD = 'And once more after the restart.'
try:
    serve = Serve('first')
    try:
        turn(serve, 'fixture', FIRST, [tool('fs_list', {'path': '.'}, 'list_1'), final('Noted: coral.')], 1)
        turn(serve, 'fixture', SECOND, [tool('goal_context', {'memory': {'query': 'coral'}}, 'recall_2'), final('The code is coral.')], 2)
        [transcript] = transcripts()
        recorded = rows(transcript)
        session_id = recorded[0]['id']
        markers = [r for r in recorded if r['record'] == 'chat_memory']
        assert len(markers) == 1 and markers[0]['marker']['protocol'] == 'ordinary-chat-memory-v5', markers
        assert recorded[1]['record'] == 'chat_memory', 'memory attaches before the configuration record'
        first, second = seals()
        for index, payload in enumerate((first, second)):
            assert payload['identity']['check_version'] == 'ordinary-chat-observation-v5', payload['identity']
            assert payload['chat_context']['turn_ordinal'] == index + 1
            owners = [o for o in payload['originals'] if o['kind'] == 'UserRequest']
            assert [o['text'] for o in owners] == [(FIRST, SECOND)[index]] and owners[0]['external'] is False
        # An ordinary tool, not one of the closed four the memory mode pins.
        assert any(o['kind'] == 'ActionInvocation' and '"fs_list"' in o['text'] for o in first['originals'])
        assert {'call_id': 'list_1', 'tool_name': 'fs_list', 'status': 'Succeeded'} in first['receipts']
        assert second['chat_context']['previous_turn'] is not None
        # The second turn's recall reached the model with the first turn's words.
        recalled = [m for r in REQUESTS for m in r['messages'] if m['role'] == 'tool' and 'coral' in json.dumps(m)]
        assert recalled, 'recall did not return the earlier turn'
        assert not any(o['kind'] == 'ToolResult' and 'coral' in o['text'] for o in second['originals']), 'recalled evidence was captured again as new evidence'
        status, body = serve.request('/api/sessions')
        assert status == 200, (status, body)
        listed = [s for s in json.loads(body)['sessions'] if s['id'] == session_id]
        assert listed and 'memory' not in listed[0], listed
    finally:
        serve.stop()

    serve = Serve('second')
    try:
        status, body = serve.request('/api/resume', {'id': session_id})
        assert status == 200, (status, body)
        key = json.loads(body)['key']
        turn(serve, key, THIRD, [final('Still coral.')], 3)
        third = seals()[2]
        assert third['chat_context']['turn_ordinal'] == 3
        assert [o['text'] for o in third['originals'] if o['kind'] == 'UserRequest'] == [THIRD]
        assert len(transcripts()) == 1, 'resume created a second conversation'
        # A new chat under the old key gets the directory the resumed one
        # was recorded in. Each still recalls through its own reader: the
        # recall is routed by conversation, never by workspace.
        turn(serve, 'fixture', 'A new chat in the old directory.', [tool('goal_context', {'memory': {'query': 'coral'}}, 'recall_4'), final('Hello.')], 4)
        turn(serve, key, 'Back to the resumed one.', [tool('goal_context', {'memory': {'query': 'coral'}}, 'recall_5'), final('Coral again.')], 5)
        newer = [rows(t) for t in transcripts() if rows(t)[0]['id'] != session_id]
        assert len(newer) == 1 and newer[0][0]['workspace'] == recorded[0]['workspace'], 'the two chats do not share a workspace'
        by_session = {}
        for payload in seals():
            by_session.setdefault(payload['identity']['session_id'], []).append(payload['chat_context']['turn_ordinal'])
        assert sorted(by_session.values()) == [[1], [1, 2, 3, 4]], by_session
        known = {session_id, newer[0][0]['id']}
        # An incognito chat on a server that remembers chats: nothing is
        # begun, sealed or retained for it, no transcript is written, and
        # a recall it attempts is refused. (The tool stays in the list the
        # model is shown, as every withheld tool does, so the cached prefix
        # is the same for every chat; what is withheld is the dispatch.)
        before_files = sorted(p.name for p in AUTHORITY.iterdir())
        before_requests = len(REQUESTS)
        assert 'XDG_RUNTIME_DIR' in ENV, 'no RAM-backed runtime directory: the incognito step cannot run, and a skipped check is not a passed one'
        status, body = serve.request('/api/incognito', raw=b'')
        assert status == 200, (status, body)
        private_key = json.loads(body)['key']
        with LOCK:
            QUEUE.extend([tool('goal_context', {'memory': {'query': 'coral'}}, 'recall_private'), final('Said in private.')])
        status, body = serve.request(f'/api/chat/{private_key}/send', {'text': 'The private handoff code is umber.', 'request_id': 'private'})
        assert status == 200, (status, body)
        wait_for(lambda: len(REQUESTS) == before_requests + 2 and not QUEUE)
        answers = [m for m in REQUESTS[-1]['messages'] if m['role'] == 'tool']
        assert len(answers) == 1, answers
        assert 'coral' not in json.dumps(answers[0]) and FIRST not in json.dumps(answers[0]), 'an incognito chat read what another chat stored'
        (ROOT / 'incognito-recall-answer.json').write_text(json.dumps(answers[0], indent=2))
        status, body = serve.request(f'/api/incognito/{private_key}/end', raw=b'')
        assert status in (200, 204), (status, body)
        assert sorted(p.name for p in AUTHORITY.iterdir()) == before_files, 'an incognito turn left something in the memory authority'
        assert len(transcripts()) == 2 and not any('umber' in t.read_text() for t in transcripts())
        assert not any('umber' in json.dumps(payload) for payload in seals())
    finally:
        serve.stop()

    # A server that does not remember chats leaves a remembered one alone.
    serve = Serve('third', remembering=False)
    try:
        status, body = serve.request('/api/resume', {'id': session_id})
        assert status == 409, (status, body)
        with LOCK:
            QUEUE.extend([final('An unremembered reply.')])
        status, body = serve.request('/api/chat/plain/send', {'text': 'A chat with no memory', 'request_id': 'plain'})
        assert status == 200, (status, body)
        wait_for(lambda: len(transcripts()) == 3 and any(r['record'] == 'outcome' for t in transcripts() for r in rows(t) if rows(t) and rows(t)[0]['id'] not in known))
        plain = next(t for t in transcripts() if rows(t)[0]['id'] not in known)
        assert not any(r['record'] == 'chat_memory' for r in rows(plain))
        assert len(retained()) == 5
    finally:
        serve.stop()
    assert not ERRORS, ERRORS
    summary = {'status': 'pass', 'retained_turns': 5, 'provider_requests': len(REQUESTS), 'checks': ['attach-before-config', 'open-web-recorder', 'ordinary-tool', 'recall-earlier-turn', 'no-recapture', 'ordinary-listing', 'ordinary-resume', 'chain-across-restart', 'two-chats-one-workspace', 'incognito-not-remembered', 'memory-off-refuses-resume', 'memory-off-plain-chat']}
    (ROOT / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary))
finally:
    provider.shutdown()
