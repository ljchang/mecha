"""Frozen coordinator tests; native preflight is explicit and requires a built example."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

MODULE = Path(__file__).resolve().parents[1] / 'bridge_model_trial.py'


class CoordinatorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if MODULE.exists():
            spec = importlib.util.spec_from_file_location('bridge_model_trial', MODULE)
            cls.m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.m)
        else:
            cls.m = None

    def test_frozen_baseline_cases_inputs_and_order_are_unchanged(self):
        # Retained real baseline cases.json and manifest.json, verified 2026-11-07.
        cases = self.m.frozen_cases()
        self.assertEqual(self.m.PROTOCOL, 'connected-bridge-model-comparison-v1')
        self.assertEqual(self.m.BUDGETS, dict(max_turns=8, max_tokens=4096, max_output_tokens=16384, timeout_secs=600))
        self.assertEqual(self.m.CONTROLS, dict(temperature=0.6, fast=True, provider_timeout_secs=180, retries=0))
        self.assertEqual(self.m.TOOLS, ['fs_read', 'fs_write', 'goal_context', 'todo'])
        self.assertEqual(self.m.digest(cases), '69de948532183b646e4d106659db70cc2b4782f1b5b8be56d713113ae56f5279')
        self.assertEqual(self.m.digest(self.m.planned_pairs()), 'db259d7874b9b1370707154adb108ee6d624f40e6508c42b298b271e97458d7c')
        self.assertEqual(self.m.digest([self.m.make_request(c, stage, c['id'])
                                      for c in cases for stage in ('A', 'B')]),
                         '5f2831a53eb5130224e7c03b1c806a00441488365fafeda7b928a1d66212a4a5')

    def test_declared_check_probe_opens_completes_then_writes(self):
        script = self.m.declared_check_script()
        self.assertEqual([name for name, _ in script], ['todo', 'todo', 'fs_write'])
        pending, completed = [args['items'][0] for _, args in script[:2]]
        self.assertEqual(pending['status'], 'pending')
        self.assertEqual(completed['status'], 'completed')
        self.assertEqual(pending['check'], completed['check'])
        self.assertTrue(pending['check'])

    def test_declared_check_evidence_rejects_phantom_and_requires_feedback(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'transcript.jsonl'
            feedback = {'steps': [{'verification': 'skipped'}],
                        'decisions': [{'unverified_steps': 1}]}
            records = [{'record': 'message', 'planning': feedback, 'content': [
                {'type': 'text', 'text': 'A declared plan check was not run; completion remains unverified.'}]}]
            def check():
                path.write_text(''.join(self.m.encode(r) for r in records))
                return self.m.declared_check_evidence({'transcript': str(path)},
                    [{'tools': [{'function': {'name': name}} for name in self.m.TOOLS]}])
            self.assertTrue(all(check().values()))
            records[0]['content'].append({'type': 'tool_use', 'id': 'mecha-check-1', 'name': 'shell'})
            self.assertFalse(check()['no_shell_dispatch'])
            records[0]['content'] = [{'type': 'text', 'text': 'no tool named `shell`'}]
            self.assertFalse(check()['no_unknown_shell'])
            self.assertFalse(check()['explicit_unverified'])
            feedback['steps'][0]['verification'] = 'passed'
            self.assertFalse(check()['never_passed'])
            self.assertFalse(check()['skipped_step'])
            records.clear()
            self.assertFalse(all(check().values()))

    def test_coordinator_exists(self):
        self.assertIsNotNone(self.m, 'frozen paired coordinator is not implemented')

    def test_case_matrix_and_common_gold(self):
        if self.m is None: self.skipTest('not implemented')
        cases = self.m.frozen_cases()
        pairs = self.m.planned_pairs()
        self.assertEqual(len(cases), 4)
        self.assertEqual(len(pairs), 8)
        self.assertEqual(sum(p['order'][0] == 'memory' for p in pairs), 4)
        self.assertEqual([p['order'][0] for p in pairs],['memory','control']*4)
        for case in cases:
            memory = self.m.make_request(case, 'B', 'pair', {'database':'/d','authority':'/a','confirmation':'a'*64})
            control = self.m.make_request(case, 'B', 'pair', None)
            for key in ('prompt', 'files', 'criterion', 'expected_rows', 'task_id'):
                self.assertEqual(memory[key], control[key])
            self.assertNotIn('prior.json', control['files'])
            self.assertNotIn('CAL-729', control['criterion']['criterion_text'])
        self.assertEqual(cases[-1]['B']['expected_rows'], [{'status':'needs_information'}])

    def test_failed_A_keeps_all_slots_and_unknown_usage(self):
        if self.m is None: self.skipTest('not implemented')
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binary = base/'binary'; binary.write_text('synthetic binary')
            root = base/'run'
            manifest = self.m.freeze(root, binary, 'http://127.0.0.1:9', 'scripted', base/'holds', source_commit='0'*40)
            def failed(*args):
                return {'status':'failed', 'error':'transport', 'native':None, 'input_tokens':None, 'output_tokens':None}
            result = self.m.execute(root, manifest, invoke=failed)
            self.assertEqual(result['planned_slots'], 24)
            self.assertEqual(len(result['slots']), 24)
            self.assertEqual(sum(s['status']=='not_started' for s in result['slots']), 16)
            self.assertEqual(result['unknown_usage_slots'], 8)
            self.assertEqual(result['arms']['memory']['task_correct'], 0)
            self.assertEqual(result['arms']['memory']['denominator'], 8)
            with self.assertRaises(FileExistsError):
                self.m.freeze(root, binary, 'http://127.0.0.1:9', 'scripted', base/'holds', source_commit='0'*40)

    def test_modified_frozen_input_refuses_before_dispatch(self):
        if self.m is None: self.skipTest('not implemented')
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory); binary=base/'binary'; binary.write_text('binary')
            root=base/'run'
            manifest=self.m.freeze(root,binary,'http://127.0.0.1:9','scripted',base/'holds',source_commit='0'*40)
            (root/'cases.json').write_text('[]')
            with self.assertRaisesRegex(ValueError,'frozen'):
                self.m.execute(root,manifest,invoke=lambda *a:self.fail('must not dispatch'))

    def test_task_and_evidence_abstention_are_separate(self):
        if self.m is None: self.skipTest('not implemented')
        case=self.m.frozen_cases()[0]
        score=self.m.score_artifact(case,'B','control',[{'status':'needs_information'}])
        self.assertFalse(score['task_correct'])
        self.assertTrue(score['appropriate_abstention'])
        self.assertFalse(self.m.score_artifact(case,'B','memory',[{'status':'needs_information'}])['appropriate_abstention'])
        self.assertFalse(self.m.score_artifact(case,'B','memory',None)['task_correct'])

    def test_preflight_available_and_wire_contract_requires_actual_request(self):
        if self.m is None: self.skipTest('not implemented')
        self.assertTrue(callable(getattr(self.m, 'preflight', None)))
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'audit.jsonl'
            p.write_text(json.dumps({'event':'request_started','wire_body':{
                'model':'m','tools':[{'function':{'name':'goal_context'}}],
                'temperature':0.6,'seed':7,'max_tokens':4096}})+'\n')
            contract=self.m.audit_contract({'provider_audit':str(p),'config':self.m.BUDGETS})
            self.assertEqual(contract['first_request']['seed'],7)
            self.assertIsNone(self.m.audit_contract({})['first_request'])

    def test_missing_audit_does_not_certify_matched_configuration(self):
        if self.m is None: self.skipTest('not implemented')
        slots=[]
        for pair in self.m.planned_pairs():
            for arm in ('A','memory','control'):
                slots.append({'arm':arm,'status':'recorded','native':{},'input_tokens':None,'output_tokens':None})
        summary=self.m.summarize(slots,[{'originals_unchanged':True,'matched_b_inputs':True,
                                       'matched_runtime_config':False}],{'scripted':True})
        self.assertFalse(summary['integrity_passed'])

    def test_source_hashes_are_frozen(self):
        if self.m is None: self.skipTest('not implemented')
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);binary=base/'binary';binary.write_text('b')
            manifest=self.m.freeze(base/'run',binary,'http://127.0.0.1:9','scripted',base/'holds',source_commit='0'*40)
            self.assertIn('source_files_sha256',manifest)
            self.assertIn('mecha-core/src/memory_experience_bridge.rs',manifest['source_files_sha256'])

    def test_admitted_wrong_A_continues_once_with_shared_B_gold(self):
        if self.m is None: self.skipTest('not implemented')
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);binary=base/'binary';binary.write_text('b');root=base/'run'
            manifest=self.m.freeze(root,binary,'http://127.0.0.1:9','scripted',base/'holds',source_commit='0'*40)
            calls=[]
            def admitted(manifest,request_file,trial_root,seed):
                req=json.loads(request_file.read_text());calls.append(req)
                trial_root.mkdir(); artifact=trial_root/'output.json'
                artifact.write_text(json.dumps([] if req['task_id'].endswith('-A') else req['expected_rows']))
                database=trial_root/'memory.sqlite';database.write_text('synthetic callback fixture')
                authority=trial_root/'authority';authority.mkdir()
                audit=trial_root/'audit.jsonl'
                audit.write_text(json.dumps({'event':'request_started','request':{'local_only':True,'think':False,'think_budget':None},'wire_body':{'seed':seed,'model':'scripted','max_tokens':4096,'temperature':0.6,'chat_template_kwargs':{'enable_thinking':False},'tools':[{'function':{'name':n}} for n in self.m.TOOLS]}})+'\n')
                native={'capture_receipt':{'id':'admitted'},'capture_error':None,'confirmation':'a'*64,
                        'database':str(database),'authority':str(authority),'artifact':str(artifact),
                        'artifact_check':{'verdict':'Failed'},'config':self.m.BUDGETS,
                        'provider_audit':str(audit),'recall_calls':0,'recall_references':[]}
                return {'status':'recorded','native':native,'input_tokens':1,'output_tokens':1}
            result=self.m.execute(root,manifest,invoke=admitted)
            self.assertEqual(len(calls),24)
            self.assertEqual(result['arms']['A']['task_correct'],0)
            self.assertEqual(result['arms']['memory']['denominator'],8)
            self.assertTrue(result['integrity_passed'])
            self.assertTrue(all(p['originals_unchanged'] for p in result['pair_audits']))
            for offset in range(0,24,3):
                self.assertEqual(calls[offset+1]['expected_rows'],calls[offset+2]['expected_rows'])
                self.assertEqual(sum(r['source'] is not None for r in calls[offset+1:offset+3]),1)
            with self.assertRaises(FileExistsError):self.m.execute(root,manifest,invoke=admitted)

    def test_duplicate_artifact_keys_are_not_silently_repaired(self):
        if self.m is None: self.skipTest('not implemented')
        self.assertTrue(callable(getattr(self.m,'decode',None)))
        with self.assertRaises(ValueError):self.m.decode('[{"id":"wrong","id":"correct"}]')

    def test_identical_but_wrong_controls_do_not_match_frozen_manifest(self):
        if self.m is None: self.skipTest('not implemented')
        self.assertTrue(callable(getattr(self.m,'controls_match',None)))
        contract={'runtime_config':self.m.BUDGETS,'first_request':{'temperature':1.0},'effective_settings':{}}
        self.assertFalse(self.m.controls_match(contract,{'model':'m','tools':self.m.TOOLS,'budgets':self.m.BUDGETS},7))

    def test_later_request_cannot_change_controls_after_valid_first_request(self):
        if self.m is None: self.skipTest('not implemented')
        body={'model':'m','temperature':0.6,'seed':7,'max_tokens':4096,
              'chat_template_kwargs':{'enable_thinking':False},'tools':[{'function':{'name':n}} for n in self.m.TOOLS]}
        effective={'local_only':True,'think':False,'think_budget':None}
        contract={'runtime_config':self.m.BUDGETS,'first_request':body,'effective_settings':effective,
                  'requests':[{'wire_body':body,'effective_settings':effective},
                              {'wire_body':dict(body,temperature=1.0),'effective_settings':effective}]}
        self.assertFalse(self.m.controls_match(contract,{'model':'m','tools':self.m.TOOLS,'budgets':self.m.BUDGETS},7))

    def test_server_observation_is_frozen_with_origin_hash(self):
        if self.m is None: self.skipTest('not implemented')
        with tempfile.TemporaryDirectory() as directory:
            base=Path(directory);binary=base/'binary';binary.write_text('b')
            info=base/'server-info.json';info.write_text('{"props":{"model_alias":"m"},"observed":true}')
            original_hash=self.m.file_hash(info)
            manifest=self.m.freeze(base/'run',binary,'http://127.0.0.1:9','m',base/'holds',
                                   source_commit='0'*40,server_info=info)
            self.assertEqual(manifest['server_observation']['source_sha256'],original_hash)
            self.assertEqual(manifest['server_observation']['value']['props']['model_alias'],'m')
            info.write_text('{}')
            self.m.validate_frozen(base/'run',manifest)
            self.assertEqual(manifest['server_observation']['value']['props']['model_alias'],'m')


if __name__ == '__main__': unittest.main()
