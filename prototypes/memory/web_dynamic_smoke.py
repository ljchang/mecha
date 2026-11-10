#!/usr/bin/env python3
"""Authored loopback-only qualification of actual serve CLI wiring. No real model."""
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
    raise SystemExit('usage: mecha-web-dynamic-smoke.py BINARY NEW_OUTPUT_DIR')
BINARY = str(Path(sys.argv[1]).resolve())
OUTPUT = Path(sys.argv[2]).resolve()
OUTPUT.mkdir(parents=True, exist_ok=False)
(OUTPUT / 'home').mkdir()
REQUEST_LOCK = threading.Lock()
OWNER = 'fixture-owner@example.invalid'
REQUESTS = []
FIXTURE_ERRORS = []

class Fixture(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['content-length'])))
        with REQUEST_LOCK:
            if len(REQUESTS) >= 16:
                self.send_error(429, 'fixture request bound exceeded')
                return
            REQUESTS.append(body)
            with (OUTPUT / 'fixture-requests.jsonl').open('a') as record:
                record.write(json.dumps(body) + '\n')
        if self.path != '/v1/chat/completions':
            with REQUEST_LOCK:
                FIXTURE_ERRORS.append(self.path)
                (OUTPUT / 'fixture-errors.json').write_text(json.dumps(FIXTURE_ERRORS))
            self.send_error(404, 'unexpected fixture route')
            return
        if body.get('stream'):
            chunks = [
                {'id': 'fixture', 'model': body['model'], 'choices': [{'index': 0, 'delta': {'content': 'Authored fixture response.'}, 'finish_reason': None}]},
                {'id': 'fixture', 'model': body['model'], 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]},
            ]
            data = ''.join('data: ' + json.dumps(c) + '\n\n' for c in chunks) + 'data: [DONE]\n\n'
            content_type = 'text/event-stream'
        else:
            data = json.dumps({'model': body['model'], 'choices': [{'message': {'role': 'assistant', 'content': 'Fixture title'}, 'finish_reason': 'stop'}]})
            content_type = 'application/json'
        data = data.encode()
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args):
        pass


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def wait_for(fn):
    end = time.monotonic() + 20
    while time.monotonic() < end:
        answer = fn()
        if answer:
            return answer
        time.sleep(.05)
    raise AssertionError('timed out waiting for authored fixture')


def request(port, path, body=None, owner=True, intent=True):
    headers = {'Content-Type': 'application/json'}
    if owner:
        headers['Tailscale-User-Login'] = OWNER
    if intent:
        headers['X-Mecha-Request'] = '1'
    req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=None if body is None else json.dumps(body).encode(), headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=3) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as err:
        return err.code, err.read().decode()
    except urllib.error.URLError:
        return 0, ''


def records(home):
    found = list((home / 'sessions').glob('*.jsonl'))
    if not found:
        return []
    assert len(found) == 1, found
    try:
        return [json.loads(s) for s in found[0].read_text().splitlines()]
    except json.JSONDecodeError:
        return []



def assert_completed(rows, expected):
    outcomes = [r for r in rows if r['record'] == 'outcome']
    assert len(outcomes) == expected and all(r.get('stop_cause') == 'completed' for r in outcomes), outcomes
    replies = [r for r in rows if r['record'] == 'message' and r.get('role') == 'assistant'
               and any(b.get('type') == 'text' and b.get('text') == 'Authored fixture response.' for b in r.get('content', []))]
    assert len(replies) == expected, rows
    assert not FIXTURE_ERRORS, FIXTURE_ERRORS

def config(home, endpoint, provider='first'):
    (home / 'config.toml').write_text(f'''default_provider = "{provider}"
[providers.first]
kind = "openai"
model = "fixture-first"
base_url = "{endpoint}"
[providers.second]
kind = "openai"
model = "fixture-second"
base_url = "{endpoint}"
[tools]
enabled = ["fs_list", "goal_context", "todo"]
permission_mode = "read-only"
[features]
web = true
[web]
owner_login = "{OWNER}"
''')


def stop(proc):
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
        raise AssertionError('serve did not drain')


def launch(home, dynamic, log):
    port = free_port()
    env = {
        'PATH': os.environ.get('PATH', '/usr/bin:/bin'),
        'HOME': str(OUTPUT / 'home'),
        'LANG': 'C.UTF-8',
        'TERM': 'dumb',
        'MECHA_HOME': str(home),
        'MECHA_SESSION_KIND': 'test',
    }
    args = [BINARY, 'serve', '--port', str(port), '--voice-port', '0', '--offer-target', '']
    if dynamic:
        # These unrelated flags must not pin or loosen the web setup.
        args += ['--dynamic-goals', '--yes', '--provider', 'unused', '--model', 'unused', '--workspace', '/unused', '--no-charter']
    proc = subprocess.Popen(args, env=env, cwd=home.parent, stdout=log, stderr=log)
    try:
        def ready():
            assert proc.poll() is None, f'serve exited early with {proc.returncode}'
            return request(port, '/api/ping')[0] == 200
        wait_for(ready)
    except BaseException:
        if proc.poll() is None:
            stop(proc)
        log.flush()
        log.seek(0)
        print(log.read(), file=sys.stderr)
        raise
    return proc, port


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
try:
    base = OUTPUT
    for enabled in [False, True]:
        home = base / ('enabled' if enabled else 'default')
        home.mkdir()
        config(home, endpoint)
        charter = home / 'charter.toml'
        charter.write_text("[[line]]\nid='fixture'\ntext='First authored priority'\n")
        with (base / f'serve-{enabled}.log').open('w+') as log:
            proc, port = launch(home, enabled, log)
            try:
                path = '/api/chat/fixture/send'
                body = {'text': 'First authored request', 'request_id': 'first'}
                assert request(port, path, body, owner=False)[0] == 403
                assert request(port, path, body, intent=False)[0] == 403
                assert not records(home)
                status, text = request(port, path, body)
                assert status == 200, (status, text)
                wait_for(lambda: sum(r['record'] == 'outcome' for r in records(home)) == 1)
                rows = records(home)
                assert_completed(rows, 1)
                decisions = [r for r in rows if r['record'] == 'goal_context_decision']
                assert bool(decisions) == enabled, decisions
                if enabled:
                    charter.write_text("[[line]]\nid='fixture'\ntext='Second authored priority'\n")
                    config(home, endpoint, 'second')
                    status, text = request(port, path, {'text': 'Second authored request', 'request_id': 'second'})
                    assert status == 200, (status, text)
                    wait_for(lambda: sum(r['record'] == 'outcome' for r in records(home)) == 2)
                    rows = records(home)
                    assert_completed(rows, 2)
                    decisions = [r for r in rows if r['record'] == 'goal_context_decision']
                    latest = [v for r in decisions for v in r['versions']][-1]
                    rebind = next(i for i, r in enumerate(rows) if r['record'] == 'config' and r['model'] == 'fixture-second')
                    second_decision = next(i for i, r in enumerate(rows) if r['record'] == 'goal_context_decision' and r['binding']['ordinal'] == 2)
                    assert rebind < second_decision
                    assert [i['source'] for i in latest['owner_inputs']] == ['authenticated_web'] * 2
                    assert [i['text'] for i in latest['owner_inputs']] == ['First authored request', 'Second authored request']
                    assert 'Second authored priority' in json.dumps(latest['charter'])
                    assert any(r['model'] == 'fixture-second' and 'Second authored priority' in json.dumps(r['messages']) for r in REQUESTS)
                    session_id = rows[0]['id']
            finally:
                stop(proc)
            if enabled:
                proc, port = launch(home, False, log)
                try:
                    status, text = request(port, '/api/resume', {'id': session_id})
                    assert status == 200, (status, text)
                    key = json.loads(text)['key']
                    before_requests = len(REQUESTS)
                    status, text = request(port, f'/api/chat/{key}/send', {'text': 'Must refuse continuation', 'request_id': 'off'})
                    assert status != 200 and '--dynamic-goals' in text, (status, text)
                    assert len(REQUESTS) == before_requests
                    assert 'Must refuse continuation' not in json.dumps(records(home))
                finally:
                    stop(proc)
    (OUTPUT / 'summary.json').write_text(json.dumps({'status': 'pass', 'request_count': len(REQUESTS), 'checks': ['default-off', 'opt-in', 'owner-guard', 'intent-guard', 'typed-context', 'charter-refresh', 'provider-rebind', 'disabled-continuation-refusal']}, indent=2) + '\n')
    print('PASS: actual serve default-off and opt-in; owner/intent guards; typed decision records; charter refresh; provider rebind; disabled continuation refusal')
finally:
    server.shutdown()
