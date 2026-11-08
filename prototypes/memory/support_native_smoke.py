#!/usr/bin/env python3
"""Authored HTTP contract smoke for the actual memory_support_trial binary.

This starts a local deterministic HTTP stub, never connects to a model, never
opens qualification fixtures, and does not constitute model qualification.
The explicit CLI requires an executable binary and audits its saved native
requests, SQLite authority and full retained exports with support_trial.py.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import threading
import urllib.parse

import support_trial as trial

MODEL = 'authored-support-contract'


def contract_case():
    return dict(id='authored-native-contract', category='correct',
        native=dict(input_text='item,keep\np1,yes\np2,no\n', requested_output='[{"item":"p1"}]',
                    observed_output='[{"item":"p1"}]', historical_criterion='Include only keep=yes items.',
                    current_criterion='Include only keep=yes items.', goal_text='Select recorded items.'),
        probe=dict(units=[
            dict(id='r1', kind='RecordedEvent', text='The observed artifact records p1.', citation=dict(passage='artifact', quote=None)),
            dict(id='r2', kind='RecordedEvent', text='The observed artifact records p9.', citation=dict(passage='artifact', quote=None)),
            dict(id='r3', kind='RecordedEvent', text='An independent reviewer approved the artifact.', citation=dict(passage='input', quote=None))],
            judgments=[dict(bearing='Good', because=['r1']), dict(bearing='Bad', because=['r1'])]),
        annotations=dict(true_unit='r1', false_unit='r2', uncertain_unit='r3', expected_bearing='Good', basis='Authored contract smoke only, never a fresh holdout or model score.'))


def authored_reply(packet):
    trial.validate_catalog(packet)
    entries = packet['catalog']['citations']
    def handle(kind):
        return next(row['handle'] for row in entries if row['kind'] == kind)
    artifact = handle('ArtifactObservation')
    if 'proposal' not in packet:
        return dict(request_key=packet['request_key'], units=[
            dict(id='one', kind='RecordedEvent', text='The observed artifact contains an item record.', evidence=[artifact]),
            dict(id='two', kind='RecordedEvent', text='The capture contains action and artifact records.', evidence=[handle('ActionInvocation'), artifact]),
            dict(id='zero', kind='Hypothesis', text='A future independent check might succeed.', evidence=[])],
            judgments=[dict(bearing='Unknown', because=[]), dict(bearing='Good', because=['one'])])
    diagnostic = dict(evidence=[artifact], reason='Authored deterministic contract response.', questions=[])
    units = []
    for unit in packet['proposal']['units']:
        status = {'r2': 'Contradicted', 'r3': 'Unknown', 'zero': 'Unknown'}.get(unit['id'], 'Supported')
        factual = 'None' if unit['id'] == 'zero' else status
        units.append(dict(diagnostic, evidence=[], target='unit:'+unit['id'], status=status, factual_premises=factual,
                          questions=['What independent evidence is missing?'] if status == 'Unknown' else [],
                          original_citations=[dict(citation_index=i, relation='DirectSupport' if status == 'Supported' else 'Contradiction' if status == 'Contradicted' else 'Uncertain') for i in range(len(unit['citations']))]))
    return dict(request_key=packet['request_key'], criterion=dict(diagnostic, state='Met'), units=units,
                judgments=[dict(diagnostic, target='judgment:'+str(i), premises_sufficient=bool(j['because'])) for i,j in enumerate(packet['proposal']['judgments'])])


def run(binary, root):
    binary = Path(binary).resolve(strict=True)
    trial.require(binary.is_file() and os.access(binary, os.X_OK), 'executable actual native example binary required')
    root = Path(root).absolute()
    root.mkdir(parents=True, exist_ok=False)
    case = contract_case()
    case_path = root / 'authored-contract-case.json'
    case_path.write_text(trial.encode(trial.trial_input(case)))
    calls, server_errors = [], []
    lock = threading.Lock()
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def reply(self, status, body):
            data = trial.encode(body).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        def do_GET(self):
            path = urllib.parse.urlsplit(self.path).path
            if path.endswith('/models'):
                self.reply(200, dict(data=[dict(id=MODEL, status=dict(value='loaded'))]))
            elif path == '/props':
                self.reply(200, dict(role='router'))
            else:
                self.reply(404, dict(error='unknown authored contract route'))
        def do_POST(self):
            try:
                parsed = urllib.parse.urlsplit(self.path)
                trial.require(parsed.path.endswith('/chat/completions') and urllib.parse.parse_qs(parsed.query).get('autoload') == ['false'], 'native provider missing autoload=false')
                length = int(self.headers.get('Content-Length', '0'))
                trial.require(0 < length <= 256 * 1024, 'native request over bound')
                body = trial.decode(self.rfile.read(length).decode())
                trial.require(body.get('model') == MODEL and body.get('max_tokens') == 4096 and not body.get('stream'), 'native request controls mismatch')
                text = body['messages'][-1]['content']
                trial.require(text.startswith('<support-input>\n') and text.endswith('\n</support-input>'), 'native support wrapper mismatch')
                packet = trial.decode(text[len('<support-input>\n'):-len('\n</support-input>')])
                payload = authored_reply(packet)
                with lock:
                    calls.append(dict(path=self.path, body=body, payload=payload))
                    trial.require(len(calls) <= 3, 'native contract exceeded three authored calls')
                self.reply(200, dict(model=MODEL, choices=[dict(message=dict(role='assistant', content=trial.encode(payload)), finish_reason='stop')], usage=dict(prompt_tokens=100, completion_tokens=100)))
            except Exception as error:
                with lock:
                    server_errors.append(str(error))
                self.reply(500, dict(error=str(error)))
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    environment = os.environ.copy()
    for key in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT'):
        environment.pop(key, None)
    environment.update(MECHA_SESSION_KIND='test', MECHA_LOG='warn')
    native_root = root / 'native'
    argv = [str(binary), '--evidence-v3', '--synthetic-only', str(native_root),
            'http://127.0.0.1:'+str(server.server_port), MODEL, str(root/'holds'), str(trial.SEED), str(case_path)]
    try:
        completed = subprocess.run(argv, capture_output=True, timeout=120, env=environment, stdin=subprocess.DEVNULL, cwd=root)
        (root/'native.stdout').write_bytes(completed.stdout)
        (root/'native.stderr').write_bytes(completed.stderr)
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)
        (root/'authored-http.json').write_text(trial.encode(calls))
    validation = trial.validate_report(native_root, case, MODEL, trial.EVIDENCE_PROTOCOL)
    errors = server_errors + validation['errors'] + validation.get('generated_errors', []) + validation.get('probe_errors', [])
    result = dict(protocol='authored-native-support-contract-v1', binary=str(binary), binary_sha256=trial.file_hash(binary),
                  model_inference=False, qualification=False, calls=len(calls), returncode=completed.returncode,
                  valid=completed.returncode == 0 and len(calls) == 3 and validation['valid'] and not server_errors,
                  errors=errors, command=argv)
    (root/'native-smoke-result.json').write_text(trial.encode(result))
    trial.require(result['valid'], 'actual native contract smoke failed: '+trial.encode(result))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--root', required=True, type=Path, help='New empty output directory')
    args = parser.parse_args()
    print(trial.encode(run(args.binary, args.root)), end='')


if __name__ == '__main__':
    main()
