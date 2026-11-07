import copy
import json
from pathlib import Path
import tempfile
import unittest

import outcome_reappraisal_trial as trial


def preparation(case):
    passages = [dict(id=f'e{i+1}', text=name, source=dict(object=name, revision=name),
                     sha256=trial.hash_text(name)) for i, name in enumerate(trial.METHODS)]
    requests = []
    for arm in trial.ARMS:
        visible = copy.deepcopy(passages)
        if arm == 'source_and_outcome':
            visible.append(dict(id='e4', text='Actual Nori: 4 minutes, energy 9, quality 8.',
                                source=dict(object='actual', revision='actual'), sha256='actual'))
        requests.append(dict(arm=arm, request=dict(prompt='Evidence\n'+trial.encode(visible),
            response_schema=dict(type='object', properties=dict(citations=dict(items=dict(enum=[p['id'] for p in visible])))),
            passages=visible), request_sha256='request-'+arm))
    read = trial.encode(dict(kind='host_authored_synthetic_measurements_v1', workload=trial.WORKLOAD,
                           complete=True, observations=[dict(choice=n, observed_minutes=m, energy=e, quality=q)
                               for n, (m, e, q) in trial.SOURCE.items()]))
    actual = trial.encode(dict(kind='host_action_observation', action_id='action', cancelled_action=False,
        observation=dict(kind='host_observed_synthetic_action_v1', fixture='measurements_v1', state='baseline',
            choice='Nori', expected_from_evidence=dict(observed_minutes=4, energy=2, quality=8),
            outcome=dict(status='observed', observed_minutes=4, energy=9, quality=8))))
    return dict(protocol='outcome-reappraisal-v1', policy='same-workload-actual-over-expectation-v1',
        action_id='action', contract=case['contract'], snapshot=dict(instance='instance', sequence=42),
        recall_query=dict(text='Nori', known_at=dict(instance='instance', sequence=42), limit=100, neighbors=0, max_bytes=100000),
        raw_recall=[dict(anchor=dict(reference=dict(object='actual', revision='actual')))],
        read=dict(task={}, observation={}, text=read, sha256=trial.hash_text(read)),
        observation=dict(source=dict(object='actual', revision='actual'), text=actual, sha256=trial.hash_text(actual)),
        arms=requests, private_data=True, untrusted_input=True, executed=False)


def report(prepared, model='scripted', seed=7, outcome_first=False):
    case = next(c for c in trial.frozen_cases() if c['contract'] == prepared['contract'])
    rows = []
    arms = list(reversed(trial.ARMS)) if outcome_first else list(trial.ARMS)
    for arm in arms:
        request = next(a['request'] for a in prepared['arms'] if a['arm'] == arm)
        gold = trial.expected(case, arm)
        choice = next(iter(gold), 'abstain')
        answer = dict(choice=choice, citations=[p['id'] for p in request['passages']], reason='Scripted evidence comparison.')
        rows.append(dict(arm=arm, call=dict(prompt=request['prompt'], response_schema=request['response_schema'],
            reply=dict(text=json.dumps(answer), model=model, input_tokens=10, output_tokens=5, error=None),
            error=None, millis=12), decision=dict(proposed_choice=choice, citations=answer['citations'], reason=answer['reason'],
                disposition='abstain' if choice == 'abstain' else 'recommend', executed=False), error=None))
    return dict(protocol='outcome-reappraisal-v1', preparation=prepared, model=model, seed=seed,
                model_controls=trial.MODEL_CONTROLS, arm_order=arms, rows=rows,
                executed=False, private_data=True, untrusted_input=True)


class RubricTests(unittest.TestCase):
    def test_frozen_contrast_and_hidden_information_are_separate(self):
        cases = trial.frozen_cases()
        self.assertEqual(len(cases), 5)
        self.assertEqual(trial.SEEDS, (7, 19))
        self.assertEqual([trial.expected(c, 'source_only') for c in cases],
                         [{'Nori'}, {'Nori'}, {'Nori'}, {'Arden'}, {'Nori'}])
        self.assertEqual([trial.expected(c, 'source_and_outcome') for c in cases],
                         [{'Arden'}, {'Nori'}, set(), {'Arden'}, {'Quill'}])
        self.assertEqual(trial.expected(cases[-1], 'source_and_outcome', simulator=True), {'Arden'})
        self.assertEqual(trial.expected(cases[-1], 'source_only', simulator=True), {'Arden'})
        self.assertEqual(len(trial.planned_pairs()), 10)
        self.assertEqual(sum(p['outcome_first'] for p in trial.planned_pairs()), 5)

    def test_valid_choice_is_independent_of_host_withholding(self):
        case = trial.frozen_cases()[2]
        prepared = preparation(case)
        row = report(prepared)['rows'][1]
        row['decision']['disposition'] = 'withheld'
        scored = trial.score_row(case, row, prepared, 'scripted')
        self.assertTrue(scored['appropriate_model_abstention'])
        self.assertTrue(scored['host_withheld'])
        row['call']['reply']['text'] = json.dumps(dict(choice='Nori', citations=['e1','e2','e3','e4'], reason='Wrong.'))
        self.assertFalse(trial.score_row(case, row, prepared, 'scripted')['appropriate_model_abstention'])

    def test_uncited_model_abstention_is_not_appropriate_supported_abstention(self):
        case = trial.frozen_cases()[2]
        prepared = preparation(case)
        row = report(prepared)['rows'][1]
        row['call']['reply']['text'] = json.dumps(dict(choice='abstain', citations=[], reason='Nothing fits.'))
        row['decision']['disposition'] = 'withheld'
        score = trial.score_row(case, row, prepared, 'scripted')
        self.assertTrue(score['valid'])
        self.assertTrue(score['model_abstained'])
        self.assertTrue(score['host_withheld'])
        self.assertFalse(score['appropriate_model_abstention'])

    def test_bad_duplicate_truncated_wrong_model_and_missing_replies_fail(self):
        case = trial.frozen_cases()[0]
        prepared = preparation(case)
        good = report(prepared)['rows'][0]
        variants = []
        for text in ('{"choice":"Nori","choice":"Arden","citations":[],"reason":"x"}',
                     json.dumps(dict(choice='Nori', citations=['e1','e1'], reason='x')),
                     json.dumps(dict(choice='Nori', citations=None, reason='x')), '[]', 'null'):
            row = copy.deepcopy(good); row['call']['reply']['text'] = text; variants.append(row)
        row = copy.deepcopy(good); row['call']['reply']['error'] = 'max_tokens'; variants.append(row)
        row = copy.deepcopy(good); row['call']['reply']['error'] = ''; variants.append(row)
        row = copy.deepcopy(good); row['call']['reply']['model'] = 'other'; variants.append(row)
        row = copy.deepcopy(good); row['call']['reply'] = None; row['call']['error'] = 'offline'; variants.append(row)
        row = copy.deepcopy(good); row['error'] = 'host parse failed'; variants.append(row)
        for row in variants:
            with self.subTest(row=row):
                self.assertFalse(trial.score_row(case, row, prepared, 'scripted')['valid'])

    def test_hidden_diagnostic_has_visible_success_and_simulator_failure(self):
        case = trial.frozen_cases()[-1]
        prepared = preparation(case)
        score = trial.score_row(case, report(prepared)['rows'][1], prepared, 'scripted')
        self.assertTrue(score['supported_optimal'])
        self.assertFalse(score['simulator_optimal'])
        self.assertTrue(score['information_gap'])
        row = report(prepared)['rows'][1]
        row['call']['reply']['text'] = json.dumps(dict(choice='Quill', citations=['e3'], reason='Incomplete.'))
        self.assertFalse(trial.score_row(case, row, prepared, 'scripted')['supported_optimal'])

    def test_preparation_requires_same_source_actual_and_bounded_recall(self):
        case = trial.frozen_cases()[0]
        prepared = preparation(case)
        trial.validate_preparation(prepared, case, 'action')
        for field in ('snapshot', 'raw_recall', 'read', 'observation'):
            bad = copy.deepcopy(prepared); bad.pop(field)
            with self.subTest(field=field), self.assertRaises(ValueError):
                trial.validate_preparation(bad, case, 'action')
        bad = copy.deepcopy(prepared); bad['arms'][1]['request']['passages'][0]['text'] = 'rewritten'
        with self.assertRaises(ValueError): trial.validate_preparation(bad, case, 'action')

    def test_partial_matrix_missing_usage_and_duplicate_rows(self):
        pairs = []
        for plan in trial.planned_pairs():
            case = next(c for c in trial.frozen_cases() if c['id'] == plan['case'])
            prepared = preparation(case)
            pairs.append(dict(**plan, preparation=prepared,
                              report=report(prepared, seed=plan['seed'], outcome_first=plan['outcome_first']), error=None))
        pairs[0]['report']['rows'][0]['call']['reply']['input_tokens'] = None
        summary = trial.summarize(pairs, 'scripted')
        self.assertTrue(summary['complete'])
        self.assertEqual(summary['calls_recorded'], 20)
        self.assertEqual(summary['arms']['source_only']['input_tokens_unknown'], 1)
        self.assertFalse(trial.summarize(pairs[:-1], 'scripted')['complete'])
        with self.assertRaises(ValueError): trial.summarize(pairs+[pairs[0]], 'scripted')
        pairs[0]['report']['rows'].append(pairs[0]['report']['rows'][0])
        with self.assertRaises(ValueError): trial.summarize(pairs, 'scripted')


class RunnerTests(unittest.TestCase):
    def fake(self, fail_at=None, drift=False):
        self.calls = []
        def invoke(argv, **_):
            self.calls.append(argv)
            self.assertEqual(argv[4], 'evaluate-outcome-recall')
            contract = json.loads(argv[argv.index('--contract')+1])
            case = next(c for c in trial.frozen_cases() if c['contract'] == contract)
            prepared = preparation(case)
            if '--prepare-only' in argv: return prepared
            self.assertEqual(len([v for v in self.calls if '--prepare-only' in v]), 10)
            if fail_at is not None and len([v for v in self.calls if '--prepare-only' not in v]) == fail_at:
                raise RuntimeError('scripted failure')
            if drift: prepared['raw_recall'].append({'drift': True})
            return report(prepared, seed=int(argv[argv.index('--seed')+1]), outcome_first='--outcome-first' in argv)
        return invoke

    def run_fake(self, root, invoke):
        db = root/'db'; db.write_text('sentinel existing database')
        binary = root/'binary'; binary.write_text('sentinel existing binary')
        holds = root/'holds'; holds.mkdir()
        return trial.run_trial(binary=binary, db=db, action='action', endpoint='http://127.0.0.1:1', model='scripted',
            holds_dir=holds, output=root/'output', invoke=invoke,
            inspect_model=lambda *a: {'models': {'data': []}, 'props': {'n_ctx': 42}},
            inspect_database=lambda p: {'sha256':trial.sha(p)})

    def test_runner_freezes_all_requests_before_calls_and_never_writes_actions(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            summary = self.run_fake(root, self.fake())
            self.assertTrue(summary['complete'])
            self.assertEqual((root/'db').read_text(), 'sentinel existing database')
            self.assertEqual(len(self.calls), 20)
            manifest = json.loads((root/'output'/'manifest.json').read_text())
            self.assertEqual(len(manifest['pairs']), 10)
            self.assertTrue(all(p['preparation'] for p in manifest['pairs']))
            frozen = json.loads((root/'output'/'freeze.json').read_text())
            self.assertTrue(all(p['report'] is None and p['started'] is False for p in frozen['pairs']))
            self.assertEqual(manifest['freeze_sha256'], trial.sha(root/'output'/'freeze.json'))
            self.assertEqual([p['preparation'] for p in frozen['pairs']], [p['preparation'] for p in manifest['pairs']])
            for argv in self.calls[1:]: self.assertIn('--snapshot', argv)
            with self.assertRaises(FileExistsError):
                trial.run_trial(binary=root/'binary', db=root/'db', action='action', endpoint='http://127.0.0.1:1',
                    model='scripted', holds_dir=root/'holds', output=root/'output')

    def test_missing_preparation_stops_before_inference_and_writes_failure_matrix(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            calls = []
            def missing(argv, **kwargs):
                calls.append(argv)
                raise RuntimeError('missing retained observation')
            summary = self.run_fake(root, missing)
            self.assertFalse(summary['complete'])
            self.assertEqual(summary['calls_recorded'], 0)
            self.assertEqual(len(calls), 1)
            self.assertIn('--prepare-only', calls[0])
            self.assertTrue(all(r['status'] == 'not_started' for r in summary['matrix']))
            self.assertEqual(len(summary['matrix']), 20)

    def test_partial_failure_and_drift_preserve_incomplete_matrix_and_stop(self):
        for drift in (False, True):
            with self.subTest(drift=drift), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                summary = self.run_fake(root, self.fake(fail_at=None if drift else 2, drift=drift))
                self.assertFalse(summary['complete'])
                self.assertEqual(summary['calls_recorded'], 0 if drift else 2)
                self.assertEqual(len([v for v in self.calls if '--prepare-only' not in v]), 1 if drift else 2)
                saved = json.loads((root/'output'/'summary.json').read_text())
                self.assertFalse(saved['complete'])
                self.assertEqual(len(saved['matrix']), 20)
                self.assertIn('RuntimeError' if not drift else 'preparation drift', saved['error'])


# Opt-in actual CLI qualification uses a COPY of an explicit retained synthetic DB.
# Default discovery stays portable: no private records or source database are tracked.
# Parent/local command sets all three variables, and the test makes zero real inferences.
import os
import sqlite3
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class ScriptedProvider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, payload):
        data = trial.encode(payload).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == '/models':
            self.server.discovery_entered.set()
            if not self.server.discovery_release.wait(30):
                self.send_error(500)
                return
            self.respond(dict(data=[dict(id='scripted', status=dict(value='loaded'))]))
        elif self.path.startswith('/props'):
            self.respond(dict(role='router', default_generation_settings=dict(n_ctx=4096)))
        else:
            self.send_error(404)

    def do_POST(self):
        if self.path != '/v1/chat/completions':
            self.send_error(404)
            return
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.requests.append(request)
        prompt = request['messages'][-1]['content']
        if isinstance(prompt, list):
            prompt = ''.join(p.get('text', '') for p in prompt)
        state = json.loads(prompt.split('Current state: ', 1)[1].split('\nAvailable methods:', 1)[0])
        case = next(c for c in trial.frozen_cases() if c['contract']['energy_budget'] == state['energy_budget']
                    and c['contract']['minimum_quality'] == state['minimum_quality'])
        arm = 'source_and_outcome' if 'actual_observation' in prompt else 'source_only'
        gold = trial.expected(case, arm)
        answer = dict(choice=next(iter(gold), 'abstain'), citations=['e1', 'e2', 'e3']+(['e4'] if arm == 'source_and_outcome' else []),
                      reason='Authored synthetic transport reply; no inference.')
        self.respond(dict(id='scripted', object='chat.completion', model='scripted',
            choices=[dict(index=0, message=dict(role='assistant', content=json.dumps(answer)), finish_reason='stop')],
            usage=dict(prompt_tokens=0, completion_tokens=0, total_tokens=0)))


@unittest.skipUnless(all(os.environ.get(name) for name in (
    'MECHA_OUTCOME_QUALIFICATION_BINARY', 'MECHA_OUTCOME_QUALIFICATION_DB', 'MECHA_OUTCOME_QUALIFICATION_ACTION')),
    'opt-in actual CLI qualification requires explicit binary, retained synthetic DB and action')
class ActualCliQualification(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.db = self.root/'copied.memory-prototype.sqlite'
        original = Path(os.environ['MECHA_OUTCOME_QUALIFICATION_DB']).resolve(strict=True)
        self.original = original
        self.original_before = trial.database_state(original)
        with sqlite3.connect(original.as_uri()+'?mode=ro', uri=True) as source, sqlite3.connect(self.db) as target:
            source.backup(target)
        self.binary = Path(os.environ['MECHA_OUTCOME_QUALIFICATION_BINARY']).resolve(strict=True)
        self.action = os.environ['MECHA_OUTCOME_QUALIFICATION_ACTION']
        self.holds = self.root/'holds'; self.holds.mkdir()
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), ScriptedProvider)
        self.server.requests = []
        self.server.discovery_entered = threading.Event()
        self.server.discovery_release = threading.Event(); self.server.discovery_release.set()
        thread = threading.Thread(target=self.server.serve_forever, daemon=True); thread.start()
        def cleanup_server():
            self.server.discovery_release.set()
            self.server.shutdown(); self.server.server_close(); thread.join(5)
        self.addCleanup(cleanup_server)
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}'
        self.env = os.environ.copy()
        for key in ('OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT'):
            self.env.pop(key, None)
        home = self.root/'home'; home.mkdir()
        (home/'config.toml').write_text('not = [valid toml')
        self.env.update(MECHA_HOME=str(home), MECHA_SESSION_KIND='test', MECHA_LOG='warn', RUST_LOG='off')

    def tearDown(self):
        self.assertEqual(trial.database_state(self.original), self.original_before)

    def base(self):
        return [str(self.binary), 'memory', '--experimental', '--db', str(self.db), 'evaluate-outcome-recall',
            '--synthetic-input', '--domain', 'synthetic-demo', '--scope', 'fixture-mail', '--action', self.action,
            '--contract', trial.encode(trial.frozen_cases()[0]['contract']), '--query', 'Nori', '--model', 'scripted']

    def test_actual_runner_pairing_and_no_action_or_source_writes(self):
        before = trial.database_state(self.db)
        summary = trial.run_trial(binary=self.binary, db=self.db, action=self.action, endpoint=self.endpoint,
                                  model='scripted', holds_dir=self.holds, output=self.root/'pilot')
        self.assertTrue(summary['complete'], summary.get('error'))
        self.assertEqual(len(self.server.requests), 20)
        self.assertEqual(trial.database_state(self.db), before)
        manifest = trial.decode((self.root/'pilot'/'manifest.json').read_text())
        for pair in manifest['pairs']:
            self.assertEqual(pair['report']['preparation'], pair['preparation'])
            for row in pair['report']['rows']:
                self.assertFalse(row['decision']['executed'])
                self.assertFalse(row['error'])
                self.assertNotIn('simulator', row['call']['prompt'].lower())
                self.assertNotIn('historical_feedback', row['call']['prompt'])
                self.assertNotIn('"quality":6', row['call']['prompt'])
        self.assertEqual(summary['primary']['source_and_outcome']['evidence_appropriate'], 8)
        self.assertEqual(summary['incomplete_knowledge_diagnostic']['source_and_outcome']['evidence_appropriate'], 2)
        self.assertEqual(summary['incomplete_knowledge_diagnostic']['source_and_outcome']['simulator_optimal'], 0)
        self.assertEqual([r['seed'] for r in self.server.requests], [p['seed'] for p in trial.planned_pairs() for _ in range(2)])

    def test_source_revoked_during_models_await_has_zero_completions_and_no_report(self):
        prepared_process = subprocess.run(self.base()+['--prepare-only'], env=self.env, cwd=self.root,
                                          text=True, capture_output=True, timeout=30)
        self.assertEqual(prepared_process.returncode, 0, prepared_process.stderr)
        prepared = trial.decode(prepared_process.stdout)
        with sqlite3.connect(self.db) as c:
            before_actions = c.execute('SELECT * FROM durable_action ORDER BY id').fetchall()
        self.server.discovery_release.clear()
        process = subprocess.Popen(self.base()+['--snapshot', trial.encode(prepared['snapshot']),
            '--endpoint', self.endpoint, '--holds-dir', str(self.holds)], env=self.env, cwd=self.root,
            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            self.assertTrue(self.server.discovery_entered.wait(30), 'CLI never entered awaited resident discovery')
            with sqlite3.connect(self.db) as c:
                c.execute('INSERT INTO restriction VALUES (?)', (prepared['read']['observation']['object'],))
            self.server.discovery_release.set()
            stdout, stderr = process.communicate(timeout=30)
        finally:
            self.server.discovery_release.set()
            if process.poll() is None:
                process.kill(); process.communicate(timeout=5)
        self.assertNotEqual(process.returncode, 0, stderr)
        self.assertEqual(stdout.strip(), '', 'invalidation exposed source-bearing report/reply')
        self.assertEqual(self.server.requests, [], 'revoked source reached completion dispatch')
        with sqlite3.connect(self.db) as c:
            self.assertEqual(c.execute('SELECT * FROM durable_action ORDER BY id').fetchall(), before_actions)


if __name__ == '__main__':
    unittest.main()
