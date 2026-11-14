#!/usr/bin/env python3
"""Actual authenticated web-memory qualification with an authored local provider.

No real model, private graph, or production service. --browser leaves an isolated
fixture listening for a bounded interactive browser check; SIGTERM drains it.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import threading
import time
import urllib.error
import urllib.request

from chat_cli_smoke import AuthoredTransport, final, last_tool, records, tool, write_private

OWNER = 'fixture-owner@example.invalid'
OBSERVATION = 'The authored harbor marker is violet and its inspection window starts at 15:00.'
TEXTS = ['  Read observation.txt and retain the authored harbor observation.\n',
         'Change the goal: prioritize the inspection window; recall the violet harbor observation.',
         'Resume the inspection objective and check the violet harbor observation.']
REPLIES = ['Authored web observation retained.', 'Authored web goal change checked.', 'Authored web resume checked.']


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def wait_for(fn, seconds=35):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = fn()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError('authored web fixture timed out')


def tree_snapshot(*roots):
    """Capture authored file bytes and directory creation, excluding access times."""
    snapshot = {}
    for index, root in enumerate(roots):
        if not root.exists():
            snapshot[(index, '.')] = 'absent'
            continue
        for path in [root, *sorted(root.rglob('*'))]:
            relative = str(path.relative_to(root))
            if path.is_symlink():
                value = ('symlink', os.readlink(path))
            elif path.is_dir():
                value = 'directory'
            else:
                value = ('file', hashlib.sha256(path.read_bytes()).hexdigest())
            snapshot[(index, relative)] = value
    return snapshot


class Fixture:
    def __init__(self, binary, root, assets=None):
        self.binary, self.root, self.assets = binary, root, assets
        root.mkdir(mode=0o700)
        self.home, self.mecha, self.work, self.storage, self.keys = [root / p for p in ('home', 'mecha', 'work', 'storage', 'keys')]
        for directory in (self.home, self.mecha, self.work, self.storage, self.keys):
            directory.mkdir(mode=0o700)
        self.transport = AuthoredTransport(root)
        self.proc, self.log, self.launches = None, None, 0
        self.env = {'HOME':str(self.home), 'MECHA_HOME':str(self.mecha), 'PATH':os.environ.get('PATH','/usr/bin:/bin'), 'LANG':'C.UTF-8', 'TERM':'dumb'}
        self.manifest = root / 'request.json'
        self.authority = self.storage / 'authority'
        write_private(self.manifest, json.dumps({'protocol':'ordinary-session-memory-v1', 'db':str(self.storage / 'fixture.memory-prototype.sqlite'), 'key_file':str(self.keys / 'fixture.key'), 'authority':str(self.authority), 'domain':'fixture', 'account':'fixture', 'scopes':[]}))
        write_private(self.mecha / 'charter.toml', '[[line]]\nid="fixture"\ntext="Check the authored harbor observation."\n')
        write_private(self.mecha / 'config.toml', f'''default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
model = "fixture"
base_url = "http://127.0.0.1:{self.transport.server.server_port}"
max_retries = 0
[tools]
enabled = ["goal_context", "fs_read"]
permission_mode = "read-only"
[features]
web = true
[web]
owner_login = "{OWNER}"
''')
        with (root / 'init.log').open('w') as out:
            subprocess.run([str(binary),'memory-session','--request',str(self.manifest),'init'], cwd=self.work, env=self.env, stdout=out, stderr=out, check=True, timeout=35)

    def launch(self, enabled=True):
        assert self.proc is None
        self.port = port()
        self.launches += 1
        self.log = (self.root / f'serve-{self.launches}.log').open('w')
        args = [str(self.binary), 'serve','--port',str(self.port),'--voice-port','0','--offer-target','']
        if self.assets:
            args += ['--assets',str(self.assets)]
        if enabled:
            args += ['--dynamic-goals','--episodic-memory',str(self.manifest),'--memory-provider','fixture','--memory-tool','goal_context','--memory-tool','fs_read']
        self.proc = subprocess.Popen(args, cwd=self.work, env=self.env, stdout=self.log, stderr=self.log)
        def ready():
            assert self.proc.poll() is None, f'serve exited: inspect {self.log.name}'
            return self.request('/api/ping')[0] == 200
        wait_for(ready)

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill(); self.proc.wait()
                raise AssertionError('web fixture did not drain')
            finally:
                self.proc = None
                self.log.close()

    def request(self, path, body=None, *, owner=True, intent=True, method=None):
        headers = {'Content-Type':'application/json'}
        if owner: headers['Tailscale-User-Login'] = OWNER
        if intent: headers['X-Mecha-Request'] = '1'
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=10) as response:
                return response.status, response.read().decode()
        except urllib.error.HTTPError as error:
            return error.code, error.read().decode()
        except urllib.error.URLError:
            return 0, ''

    def json(self, path, body=None):
        status, text = self.request(path, body)
        assert status == 200, (path,status,text)
        return json.loads(text)

    def script(self):
        self.transport.script('web-memory', [tool('fs_read',{'path':'observation.txt'},'web_read'),final(REPLIES[0]),tool('goal_context',{'memory':{'query':'violet harbor'}},'web_recall_2'),final(REPLIES[1]),tool('goal_context',{'memory':{'query':'violet harbor'}},'web_recall_3'),final(REPLIES[2])])

    def retained(self, key, ordinal):
        def read():
            snap = self.json(f'/api/chat/{key}')
            memory = snap.get('memory',{})
            report = memory.get('last_report') or {}
            if report.get('turn_ordinal') == ordinal:
                assert report['status'] == 'retained', memory
                assert report['semantic_assessment'] == 'not_evaluated', report
                assert 'receipt' not in report, 'raw receipt leaked through browser DTO'
                assert memory['phase'] == 'ready' and memory['next_turn_ordinal'] == ordinal + 1, memory
                return snap
            return None
        return wait_for(read)

    def close(self):
        try: self.stop()
        finally: self.transport.close()


def smoke(f):
    f.launch()
    assert f.json('/api/summary')['chat_memory']['available'] is True
    path = '/api/chat/authored-memory'
    protected = (f.mecha / 'sessions', f.authority, f.mecha / 'work', f.work)
    before = tree_snapshot(*protected)
    for owner, intent in [(False,True),(True,False)]:
        assert f.request(path,{'memory':'episodic'},owner=owner,intent=intent)[0] == 403
        assert tree_snapshot(*protected) == before, 'unauthenticated open changed native/authority/work files'
        assert not f.transport.requests, 'unauthenticated open dispatched a provider request'
    opened = f.json(path, {'memory':'episodic'})
    session = opened['session']; key = opened['key']
    native = f.mecha / 'sessions' / f'{session}.jsonl'
    rows = records(native)
    workspace = Path(rows[0]['workspace'])
    write_private(workspace / 'observation.txt', OBSERVATION+'\n')
    assert opened['memory']['next_turn_ordinal'] == 1
    snapshot = native.read_bytes()
    protected_before = tree_snapshot(*protected)
    for label, route, body in [
        ('wrong-mode',path,{}),
        ('no-ordinal',path+'/send',{'text':'must not execute','request_id':'invalid-no-ordinal'}),
        ('old-ordinal',path+'/send',{'text':'must not execute','request_id':'invalid-old-ordinal','memory_turn':0}),
        ('attachment',path+'/send',{'text':'must not execute','request_id':'invalid-attachment','memory_turn':1,'attachments':['inbox/nope']}),
        ('source-spoof',path+'/send',{'text':'must not execute','request_id':'invalid-source','memory_turn':1,'source':'authenticated_web'}),
        ('permission-change',path+'/mode',{'mode':'allow'}),
    ]:
        status, text = f.request(route,body)
        assert status in (400,409,422), (label,status,text)
        assert native.read_bytes() == snapshot and not f.transport.requests, label
        assert tree_snapshot(*protected) == protected_before, label
    reports = []
    f.script()
    for i in range(2):
        if i:
            write_private(f.mecha / 'charter.toml','[[line]]\nid="fixture"\ntext="Prioritize the authored inspection window."\n')
        answer = f.json(path+'/send', {'text':TEXTS[i],'request_id':f'web-{i}','memory_turn':i+1})
        assert answer['started'] is True, answer
        reports.append(f.retained(key,i+1)['memory']['last_report'])
    rows = records(native)
    decisions = [r for r in rows if r['record'] == 'goal_context_decision']
    latest = [v for r in decisions for v in r['versions']][-1]
    assert [v['text'] for v in latest['owner_inputs']] == TEXTS[:2]
    assert all(v['source'] == 'authenticated_web' for v in latest['owner_inputs'])
    assert 'Prioritize the authored inspection window.' in json.dumps(latest['charter'])
    snapshot = native.read_bytes(); count = len(f.transport.requests)
    assert f.request(path+'/send',{'text':TEXTS[1],'request_id':'late-duplicate','memory_turn':2})[0] == 409
    assert native.read_bytes() == snapshot and len(f.transport.requests) == count
    f.stop(); f.launch()
    assert f.request(path)[0] in (404,409), 'restart silently recreated memory key'
    assert f.request('/api/resume',{'id':session})[0] in (400,409,422)
    assert native.read_bytes() == snapshot and len(f.transport.requests) == count
    resumed = f.json('/api/resume',{'id':session,'memory':'episodic'})
    key = resumed['key']; path=f'/api/chat/{key}'
    assert resumed['session'] == session and resumed['memory']['next_turn_ordinal'] == 3
    assert resumed['memory']['last_report'] is None
    answer = f.json(path+'/send',{'text':TEXTS[2],'request_id':'web-2','memory_turn':3})
    assert answer['started'] is True, answer
    reports.append(f.retained(key,3)['memory']['last_report'])
    f.transport.assert_consumed()
    assert len(f.transport.requests) == 6
    rows = records(native)
    markers = [row['marker'] for row in rows if row['record'] == 'chat_memory']
    assert len(markers) == 1
    assert markers[0]['protocol'] == 'ordinary-chat-memory-v3'
    assert markers[0]['conversation_id'] == session
    decisions = [row for row in rows if row['record'] == 'goal_context_decision']
    assert len(decisions) == 6
    latest = [version for row in decisions for version in row['versions']][-1]
    assert [owner['text'] for owner in latest['owner_inputs']] == TEXTS
    assert all(owner['source'] == 'authenticated_web' for owner in latest['owner_inputs'])
    assert len(list(f.authority.glob('*.chat-seal'))) == 3
    native_bytes = native.read_bytes()
    previous_run, previous_receipt, previous_length = None, None, 0
    source_identities, original_references = set(), set()
    observation, observation_reference = None, None
    for ordinal, report in enumerate(reports, 1):
        assert report['protocol'] == 'ordinary-chat-memory-v3'
        assert report['conversation_id'] == session and report['turn_ordinal'] == ordinal
        assert report['reason'] is None
        run_id = report['run_id']
        assert len(run_id) == 64 and all(char in '0123456789abcdef' for char in run_id)
        begin_bytes = (f.authority / f'{run_id}.chat-begin').read_bytes()
        assert hashlib.sha256(begin_bytes).hexdigest() == run_id
        begin = json.loads(begin_bytes)
        seal_bytes = (f.authority / f'{run_id}.chat-seal').read_bytes()
        assert hashlib.sha256(seal_bytes).hexdigest() == (f.authority / f'{run_id}.chat-seal-hash').read_text()
        seal = json.loads(seal_bytes)
        receipt = json.loads((f.authority / f'{run_id}.chat-retained').read_text())
        assert report['receipt_id'] == receipt['id'], 'browser receipt does not identify the retained capture'
        assert seal['turn'] == run_id and seal['length'] > previous_length
        assert hashlib.sha256(native_bytes[:seal['length']]).hexdigest() == seal['digest']
        payload = seal['payload']
        assert payload['identity']['check_version'] == 'ordinary-chat-observation-v2'
        assert payload['identity']['evaluator_version'] == 'ordinary-chat-recorder-v2'
        assert payload.get('criterion') is None, 'observation manufactured a confirmed criterion'
        context = payload['chat_context']
        assert context['conversation_id'] == session and context['turn_ordinal'] == ordinal
        assert context['previous_turn'] == previous_run and begin['previous'] == previous_run
        assert context['prefix_start'] == previous_length and begin['prefix_start'] == previous_length
        assert context['prefix_length'] == seal['length'] and context['prefix_digest'] == seal['digest']
        assert len(context['decisions']) == 2 and len(context['owner_messages']) == 1
        for version in context['versions']:
            assert [owner['text'] for owner in version['owner_inputs']] == TEXTS[:ordinal]
            assert all(owner['source'] == 'authenticated_web' for owner in version['owner_inputs'])
        if previous_receipt:
            assert previous_receipt['projection'] in payload['consulted'], 'predecessor projection lost from capture lineage'
            assert previous_receipt['projection'] in begin['dependencies'], 'predecessor projection lost from begin authority'
        originals = payload['originals']
        assert len(originals) == len(receipt['originals']) and originals
        owners = [original for original in originals if original['kind'] == 'UserRequest']
        assert len(owners) == 1 and owners[0]['text'] == TEXTS[ordinal-1]
        if ordinal > 1:
            assert not any(original['kind'] == 'ToolResult' for original in originals), 'recalled evidence was recaptured as new original evidence'
            assert not any(original['text'] in TEXTS[:ordinal-1] for original in originals), 'old owner text was recaptured'
        for original, reference in zip(originals, receipt['originals']):
            expected_connector = 'host-action-invocation-v1' if original['kind'] == 'ActionInvocation' else 'host-session-field-v1'
            assert original['source'] == {'connector':expected_connector, 'account':'fixture',
                'id':f"{session}:{original['line']}:{original['pointer']}"}, 'original source identity is not the native locator'
            identity = tuple(original['source'][field] for field in ('connector','account','id'))
            assert identity not in source_identities, 'separate turns reused an original source identity'
            source_identities.add(identity)
            revision = (reference['object'],reference['revision'])
            assert revision not in original_references, 'old evidence was retained again as a new original'
            original_references.add(revision)
            line = rows[original['line']-1]
            value = line
            for part in original['pointer'].split('/')[1:]:
                part = part.replace('~1','/').replace('~0','~')
                value = value[int(part)] if isinstance(value,list) else value[part]
            if original['kind'] == 'ActionInvocation':
                action = json.loads(original['text'])
                block = line['content'][int(original['pointer'].split('/')[2])]
                assert action['call_id'] == block['id'] and action['tool_name'] == block['name']
                assert action['input_status'] == 'Complete' and action['input'] == value
                # Hash the native object's preserved serialization order, as
                # workspace serde_json does; sorting would test different bytes.
                encoded = json.dumps(value,separators=(',',':'),ensure_ascii=False).encode()
                assert action['input_digest'] == hashlib.sha256(encoded).hexdigest()
            else:
                assert value == original['text'], 'source pointer did not resolve exactly'
            if ordinal == 1 and original['kind'] == 'ToolResult' and OBSERVATION in original['text']:
                assert observation is None, 'ambiguous authored source observation'
                observation, observation_reference = original, reference
        previous_run, previous_receipt, previous_length = run_id, receipt, seal['length']
    assert observation is not None, 'first-turn file observation was not retained'
    for index in (3,5):
        result = last_tool(f.transport.requests[index]['request'])
        assert result.get('confirmed_goal') is None, 'recall manufactured a confirmed goal'
        recalled = result['memory']['records']
        assert any(hit['reference'] == observation_reference
                   and hit['record']['kind'] == 'evidence'
                   and hit['record']['data'].get('text') == observation['text']
                   and hit['record']['data'].get('source') == observation['source']
                   for hit in recalled), 'recall lost the exact original evidence/reference/source'
    f.stop(); f.launch(False)
    assert f.json('/api/summary')['chat_memory']['available'] is False
    assert f.request('/api/resume',{'id':session,'memory':'episodic'})[0] == 503
    summary={'status':'pass','retained_turns':3,'authored_provider_requests':6,'source':'authenticated_web','distinct_original_sources':len(source_identities),'checks':['owner-and-intent-no-filesystem-effects','strict-opt-in','exact-input','versioned-charter','exact-recall-source','delayed-ordinal','restart-explicit-resume','v3-marker-v2-observation-recorder','exact-source-pointers','predecessor-lineage','receipt-identity','no-recapture','profile-off']}
    write_private(f.root/'summary.json',json.dumps(summary,indent=2)+'\n')
    print(json.dumps(summary))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('binary',type=Path);p.add_argument('output',type=Path)
    p.add_argument('--assets',type=Path);p.add_argument('--browser',action='store_true')
    args=p.parse_args()
    f=Fixture(args.binary.resolve(strict=True),args.output.resolve(),args.assets.resolve(strict=True) if args.assets else None)
    try:
        if not args.browser:
            smoke(f)
        else:
            f.script();f.launch()
            # Browser creates the conversation. The fixture supplies only the
            # authored observation file once the native workspace exists.
            stopped=threading.Event()
            signal.signal(signal.SIGTERM,lambda *_:stopped.set())
            print(json.dumps({'port':f.port,'owner':OWNER,'output':str(f.root)}),flush=True)
            deadline=time.monotonic()+600
            while not stopped.wait(.1) and time.monotonic()<deadline:
                for native in (f.mecha/'sessions').glob('*.jsonl'):
                    try: rows=records(native)
                    except json.JSONDecodeError: continue
                    if rows and rows[0].get('workspace'):
                        file=Path(rows[0]['workspace'])/'observation.txt'
                        if not file.exists():write_private(file,OBSERVATION+'\n')
            f.transport.save()
    finally:
        f.close()

if __name__ == '__main__':main()
