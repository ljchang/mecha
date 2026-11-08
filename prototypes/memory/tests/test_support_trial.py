"""Offline failure-contract tests; no endpoint/model calls."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

MODULE = Path(__file__).resolve().parents[1] / 'support_trial.py'
spec = importlib.util.spec_from_file_location('support_trial', MODULE)
trial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trial)


class SupportTrialTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        # Other lanes edit the shared Rust checkout concurrently. Freeze a real
        # test-owned source file at this external discovery boundary instead.
        source = self.root / 'source.rs'
        source.write_text('// synthetic qualification source\n')
        source_patch = patch.object(trial, 'source_hashes', side_effect=lambda: {'source.rs': trial.file_hash(source)})
        source_patch.start()
        self.addCleanup(source_patch.stop)
        self.case = {
            'id': 'correct', 'category': 'correct',
            'native': {'input_text': 'item,kept\np1,yes\n',
                       'requested_output': '[{"item":"p1"}]',
                       'observed_output': '[{"item":"p1"}]',
                       'historical_criterion': 'Include kept=yes items.',
                       'current_criterion': 'Include kept=yes items.',
                       'goal_text': 'Select manifest items.'},
            'probe': {'units': [
                {'id': 'true', 'kind': 'RecordedEvent', 'text': 'The observed output lists p1.',
                 'citation': {'passage': 'artifact', 'quote': '[{"item":"p1"}]'}},
                {'id': 'false', 'kind': 'RecordedEvent', 'text': 'The observed output lists z9.',
                 'citation': {'passage': 'artifact', 'quote': '[{"item":"p1"}]'}},
                {'id': 'uncertain', 'kind': 'RecordedEvent', 'text': 'A reviewer approved these items.',
                 'citation': {'passage': 'input', 'quote': 'item,kept\np1,yes'}}],
                'judgments': [{'bearing': 'Good', 'because': ['true']},
                              {'bearing': 'Bad', 'because': ['true']}]},
            'annotations': {'true_unit': 'true', 'false_unit': 'false', 'uncertain_unit': 'uncertain',
                            'expected_bearing': 'Good', 'basis': 'Direct bytes and criterion.'}}

    def artifact(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return {'path': name, 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)}

    def report(self):
        artifacts = {
            'initial_input': self.artifact('input.txt', self.case['native']['input_text'].encode()),
            'native_transcript': self.artifact('session.jsonl', b'{"synthetic":true}\n'),
            'observed_output': self.artifact('output.json', self.case['native']['observed_output'].encode()),
            'admitted_generated': self.artifact('admitted.json', b'[]\n')}
        stages = {name: {'status': 'completed', 'error': None} for name in
                  ['source', 'capture', 'generate', 'evaluate_generated', 'evaluate_probe', 'retain', 'recall', 'changed_goal']}
        stages['capture'].update(packet_bytes=900, withheld=False)
        for name in ['generate', 'evaluate_generated', 'evaluate_probe']:
            request = {'event': 'request_started', 'call': 1,
                       'request': {'model': 'resident-test', 'local_only': True, 'think': False, 'max_tokens': 4096},
                       'wire_body': {'model': 'resident-test', 'max_tokens': 4096, 'seed': 70510,
                                     'temperature': 0.6, 'autoload': False}}
            response = {'event': 'response', 'call': 1, 'model': 'resident-test', 'stop_reason': 'end_turn',
                        'refusal': None, 'input_tokens': 10, 'output_tokens': 5}
            request_bytes, response_bytes = [(json.dumps(e)+'\n').encode() for e in (request, response)]
            stages[name].update(request=self.artifact(name + '-request.json', request_bytes),
                                reply=self.artifact(name + '-reply.json', response_bytes),
                                provider_audit=self.artifact(name + '-provider.jsonl', request_bytes + response_bytes), model='resident-test',
                                input_tokens=10, output_tokens=5, stop_reason='end_turn', packet_bytes=1200)
        report = {'protocol': 'memory-factual-support-v1', 'case_id': self.case['id'],
                'case_sha256': trial.digest(self.case), 'model': 'resident-test', 'seed': 70510,
                'status': 'completed', 'run_error': None, 'stages': stages, 'artifacts': artifacts,
                'initial_hashes': {k: v['sha256'] for k, v in artifacts.items() if k != 'admitted_generated'},
                'probe': {'units': [
                    {'id': 'true', 'status': 'Supported', 'admitted': True, 'withheld': False},
                    {'id': 'false', 'status': 'Contradicted', 'admitted': False, 'withheld': True},
                    {'id': 'uncertain', 'status': 'Unknown', 'admitted': False, 'withheld': True}],
                    'judgments': [{'index': 0, 'status': 'Supported', 'bearing': 'Good', 'disposition': 'retained'},
                                  {'index': 1, 'status': 'Contradicted', 'bearing': 'Unknown', 'disposition': 'downgraded'}]},
                'input_tokens': 30, 'output_tokens': 15}
        report['source_seal'] = self.artifact('source-seal.json', trial.encode(report['initial_hashes']).encode())
        true_unit = self.case['probe']['units'][0]
        accepted = {k: true_unit[k] for k in ('id', 'kind', 'text')}
        accepted['citations'] = [{'source_id': 'source:test', 'quote': 'exact source quotation'}]
        base = dict(protocol='situated-factual-support-v1', generator_model='resident-test', evaluator_model='resident-test',
                    proposal_hash='a' * 64, request_hash='b' * 64)
        generated = dict(base, units=[], judgments=[])
        probe = dict(base, units=[{'disposition': 'Accepted', 'unit': accepted, 'status': 'Supported'},
                                 {'disposition': 'Rejected', 'id': 'false', 'status': 'Contradicted'},
                                 {'disposition': 'Rejected', 'id': 'uncertain', 'status': 'Unknown'}],
                     judgments=[dict(index=0, goal='goal:current', offered_bearing='Good', bearing='Good', because=['true'], status='Supported', reason='Retained'),
                                dict(index=1, goal='goal:current', offered_bearing='Bad', bearing='Unknown', because=['true'], status='Contradicted', reason='DuplicateGoal')])
        report['support_reports'] = {key: self.artifact(key + '-support.json', trial.encode(value).encode())
                                     for key, value in [('generated', generated), ('probe', probe)]}
        for unit in report['probe']['units']:
            unit['unsupported_factual_premise'] = False
        for index, judgment in enumerate(report['probe']['judgments']):
            judgment.update(offered_bearing=self.case['probe']['judgments'][index]['bearing'],
                            reason=probe['judgments'][index]['reason'], unsupported_factual_premise=False)
        report['probe']['judgments'][1]['disposition'] = 'dropped'
        for name in trial.MODEL_STAGES:
            checks = []
            if name == 'evaluate_probe':
                checks = [dict(target='unit:' + u['id'], status=u['status'], unsupported_factual_premise=False,
                               citations=accepted['citations'] if u['status'] == 'Supported' else [], reason='Authored diagnostic',
                               questions=['What evidence is missing?'] if u['status'] == 'Unknown' else []) for u in report['probe']['units']]
                checks += [dict(target='judgment:' + str(j['index']), status=j['status'], unsupported_factual_premise=False,
                                citations=accepted['citations'] if j['status'] == 'Supported' else [], reason='Authored diagnostic', questions=[])
                           for j in report['probe']['judgments']]
            payload = {'units': [], 'judgments': []} if name == 'generate' else {'checks': checks}
            stage = report['stages'][name]
            raw = trial.decode((self.root / stage['reply']['path']).read_text())
            raw['message'] = {'role': 'assistant', 'content': [{'type': 'text', 'text': trial.encode(payload)}]}
            response = (trial.encode(raw)).encode()
            stage['reply'] = self.artifact(stage['reply']['path'], response)
            request = (self.root / stage['request']['path']).read_bytes()
            stage['provider_audit'] = self.artifact(stage['provider_audit']['path'], request + response)
        return report

    def change_probe(self, report, unit_id=None, judgment_index=None, **changes):
        """Mutate actual raw and durable observations together, not only a summary."""
        descriptor = report['support_reports']['probe']
        support = trial.decode((self.root / descriptor['path']).read_text())
        stage = report['stages']['evaluate_probe']
        raw = trial.decode((self.root / stage['reply']['path']).read_text())
        payload = trial.decode(raw['message']['content'][0]['text'])
        if unit_id is not None:
            item = next(u for u in report['probe']['units'] if u['id'] == unit_id)
            item.update(changes)
            position = next(i for i, u in enumerate(support['units']) if u.get('id', u.get('unit', {}).get('id')) == unit_id)
            original = next(u for u in self.case['probe']['units'] if u['id'] == unit_id)
            unit = {k: original[k] for k in ('id', 'kind', 'text')}
            unit['citations'] = [{'source_id': 'source:test', 'quote': 'exact source quotation'}]
            support['units'][position] = dict(disposition='Accepted', unit=unit, status=item['status']) if item['admitted'] else dict(disposition='Rejected', id=unit_id, status=item['status'])
            target = 'unit:' + unit_id
        else:
            item = report['probe']['judgments'][judgment_index]
            item.update(changes)
            support['judgments'][judgment_index].update({k: item[k] for k in ('offered_bearing', 'status', 'bearing', 'reason')})
            target = 'judgment:' + str(judgment_index)
        check = next(c for c in payload['checks'] if c['target'] == target)
        check.update(status=item['status'], unsupported_factual_premise=item['unsupported_factual_premise'])
        check['citations'] = [{'source_id': 'source:test', 'quote': 'exact source quotation'}] if item['status'] == 'Supported' else []
        check['questions'] = ['What evidence is missing?'] if item['status'] == 'Unknown' else []
        raw['message']['content'][0]['text'] = trial.encode(payload)
        response = trial.encode(raw).encode()
        stage['reply'] = self.artifact(stage['reply']['path'], response)
        request = (self.root / stage['request']['path']).read_bytes()
        stage['provider_audit'] = self.artifact(stage['provider_audit']['path'], request + response)
        report['support_reports']['probe'] = self.artifact(descriptor['path'], trial.encode(support).encode())

    def write_report(self, report):
        (self.root / 'report.json').write_text(json.dumps(report))

    def validate(self):
        return trial.validate_report(self.root, self.case, 'resident-test')

    def test_missing_report_is_failed_attempt(self):
        result = self.validate()
        self.assertFalse(result['valid'])
        self.assertIn('missing', ' '.join(result['errors']))

    def test_truncated_report_is_failed_attempt(self):
        (self.root / 'report.json').write_text('{"status":')
        self.assertFalse(self.validate()['valid'])

    def test_complete_report_is_accepted_with_verified_source_hashes(self):
        self.write_report(self.report())
        self.assertTrue(self.validate()['valid'])

    def test_omitted_stage_fails_closed(self):
        report = self.report()
        del report['stages']['evaluate_probe']
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_false_complete_status_fails_closed(self):
        report = self.report()
        report['stages']['capture']['status'] = 'failed'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_pending_stage_cannot_be_counted_complete(self):
        report = self.report()
        report['stages']['changed_goal']['status'] = 'pending'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_served_model_mismatch_fails_closed(self):
        report = self.report()
        report['stages']['evaluate_probe']['model'] = 'other-model'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_max_tokens_reply_fails_closed(self):
        report = self.report()
        report['stages']['generate']['stop_reason'] = 'max_tokens'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_known_wrong_actual_artifact_fails_even_with_matching_descriptor(self):
        report = self.report()
        report['artifacts']['observed_output'] = self.artifact('output.json', b'[]')
        report['initial_hashes']['observed_output'] = report['artifacts']['observed_output']['sha256']
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_modified_native_transcript_fails(self):
        report = self.report()
        self.write_report(report)
        (self.root / 'session.jsonl').write_text('changed')
        self.assertFalse(self.validate()['valid'])

    def test_artifact_path_escape_and_symlink_refused(self):
        for path in ['../outside.json', '/tmp/outside.json']:
            report = self.report()
            report['artifacts']['native_transcript']['path'] = path
            self.write_report(report)
            self.assertFalse(self.validate()['valid'])
        report = self.report()
        (self.root / 'link.json').symlink_to(self.root / 'session.jsonl')
        report['artifacts']['native_transcript']['path'] = 'link.json'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_unknown_or_omitted_probe_targets_fail(self):
        for mutate in [lambda r: r['probe']['units'].pop(),
                       lambda r: r['probe']['units'].append(dict(r['probe']['units'][0]))]:
            report = self.report()
            mutate(report)
            self.write_report(report)
            self.assertFalse(self.validate()['valid'])

    def test_missing_audit_prevents_gate_even_when_probe_passes(self):
        report = self.report()
        self.write_report(report)
        result = trial.score_case(self.case, self.validate(), None)
        self.assertEqual(result['true_supported'], 1)
        self.assertEqual(result['uncertain_appropriate'], 1)
        self.assertFalse(result['manual_audit_complete'])

    def test_accepted_false_fact_and_wrong_bearing_are_counted(self):
        self.case['annotations']['expected_bearing'] = 'Bad'
        report = self.report()
        self.change_probe(report, unit_id='false', status='Supported', admitted=True, withheld=False)
        self.write_report(report)
        result = trial.score_case(self.case, self.validate(), None)
        self.assertEqual(result['false_facts_accepted'], 1)
        self.assertEqual(result['wrong_bearings_accepted'], 1)

    def test_failed_attempts_stay_in_fixed_eight_case_denominator(self):
        summary = trial.summarize([{'case_id': str(i), 'valid': False} for i in range(8)])
        self.assertEqual(summary['attempts'], 8)
        self.assertEqual(summary['true_supported'], {'count': 0, 'denominator': 8})
        self.assertFalse(summary['gate_passed'])

    def test_gate_requires_exactly_eight_unique_cases_and_all_audits(self):
        good = dict(valid=True, true_supported=1, uncertain_appropriate=1, false_facts_accepted=0,
                    wrong_bearings_accepted=0, wrong_bearings_supported=0, correct_bearing_supported=1,
                    probe_outcome_observed=True, manual_audit_complete=True, unsupported_generated=0)
        rows = [dict(good, case_id=str(i)) for i in range(8)]
        self.assertTrue(trial.summarize(rows)['gate_passed'])
        for changed in [rows[:7], rows + [dict(rows[0])], rows[:-1] + [dict(rows[0])]]:
            self.assertFalse(trial.summarize(changed)['gate_passed'])
        rows[0]['manual_audit_complete'] = False
        self.assertFalse(trial.summarize(rows)['gate_passed'])

    def test_fixture_has_eight_fresh_distinct_cases_and_annotation_free_native_input(self):
        cases = trial.load_cases()
        self.assertEqual({c['category'] for c in cases}, {
            'correct', 'empty', 'missing', 'request_output_mismatch', 'changed_goal',
            'unsupported_cause', 'injected_instructions', 'unknown_evidence'})
        self.assertEqual(len({c['id'] for c in cases}), 8)
        for case in cases:
            model_input = trial.native_input(case)
            self.assertNotIn('annotations', model_input)
            self.assertNotIn('probe', model_input)

    def test_runtime_probe_ids_and_order_do_not_reveal_host_labels(self):
        cases = trial.load_cases()
        true_positions, correct_judgment_positions = [], []
        for case in cases:
            runtime = trial.trial_input(case)
            ids = [u['id'] for u in runtime['probe']['units']]
            self.assertTrue(set(ids).isdisjoint({'true', 'false', 'uncertain', 'supported', 'contradicted', 'unknown'}))
            self.assertNotIn('annotations', runtime)
            true_positions.append(ids.index(case['annotations']['true_unit']))
            bearings = [j['bearing'] for j in runtime['probe']['judgments']]
            correct_judgment_positions.append(bearings.index(case['annotations']['expected_bearing']))
        self.assertEqual(set(true_positions), {0, 1, 2})
        self.assertLessEqual(max(true_positions.count(i) for i in range(3)) - min(true_positions.count(i) for i in range(3)), 1)
        self.assertEqual(correct_judgment_positions.count(0), 4)
        self.assertEqual(correct_judgment_positions.count(1), 4)

    def test_case_unknown_fields_and_duplicate_keys_refused(self):
        path = self.root / 'cases.json'
        case = copy.deepcopy(self.case)
        case['native']['expected_rows'] = []
        path.write_text(json.dumps({'version': 1, 'cases': [case] * 8}))
        with self.assertRaises(ValueError):
            trial.load_cases(path)
        path.write_text('{"version":1,"version":1,"cases":[]}')
        with self.assertRaises(ValueError):
            trial.load_cases(path)

    def test_raw_reply_model_mismatch_cannot_hide_behind_report_identity(self):
        report = self.report()
        stage = report['stages']['generate']
        raw = json.loads((self.root / stage['reply']['path']).read_text())
        raw['model'] = 'other-model'
        stage['reply'] = self.artifact('generate-reply.json', (json.dumps(raw)+'\n').encode())
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_raw_audit_additional_attempt_refused(self):
        report = self.report()
        stage = report['stages']['generate']
        data = (self.root / stage['provider_audit']['path']).read_bytes()
        stage['provider_audit'] = self.artifact('generate-provider.jsonl', data + data)
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_malformed_raw_event_is_failed_attempt_not_an_exception(self):
        report = self.report()
        report['stages']['generate']['request'] = self.artifact('generate-request.json', b'null\n')
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_missing_or_oversized_atomic_packet_accounting_refused(self):
        for name, limit in [('capture', 20000), ('generate', 32768), ('evaluate_generated', 49152), ('evaluate_probe', 49152)]:
            report = self.report()
            report['stages'][name]['packet_bytes'] = limit + 1
            self.write_report(report)
            self.assertFalse(self.validate()['valid'])
        report = self.report()
        del report['stages']['capture']['withheld']
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_downgraded_bearing_cannot_hide_a_good_or_bad_result(self):
        report = self.report()
        report['probe']['judgments'][1]['bearing'] = 'Bad'
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_manual_audit_must_cover_actual_admitted_ids(self):
        report = self.report()
        unit = dict(id='g1', kind='RecordedEvent', text='A fabricated event.',
                    citations=[dict(source_id='source:test', quote='exact source quotation')])
        report['artifacts']['admitted_generated'] = self.artifact('admitted.json', trial.encode([dict(unit, status='Supported')]).encode())
        descriptor = report['support_reports']['generated']
        support = trial.decode((self.root / descriptor['path']).read_text())
        support['units'] = [dict(disposition='Accepted', unit=unit, status='Supported')]
        report['support_reports']['generated'] = self.artifact(descriptor['path'], trial.encode(support).encode())
        for name, payload in [('generate', dict(units=[unit], judgments=[])),
                              ('evaluate_generated', dict(checks=[dict(target='unit:g1', status='Supported',
                                unsupported_factual_premise=False, citations=unit['citations'], reason='Authored diagnostic', questions=[])]))]:
            stage = report['stages'][name]
            raw = trial.decode((self.root / stage['reply']['path']).read_text())
            raw['message']['content'][0]['text'] = trial.encode(payload)
            response = trial.encode(raw).encode()
            stage['reply'] = self.artifact(stage['reply']['path'], response)
            request = (self.root / stage['request']['path']).read_bytes()
            stage['provider_audit'] = self.artifact(stage['provider_audit']['path'], request + response)
        self.write_report(report)
        validation = self.validate()
        audit = dict(reviewer_kind='external_manual', reviewer='test-reviewer',
                     report_sha256=validation['report_sha256'],
                     admitted_generated_sha256=report['artifacts']['admitted_generated']['sha256'],
                     inspected_unit_ids=[], admitted_unit_ids=[], unsupported_unit_ids=[], notes='No statements inspected.')
        self.assertFalse(trial.score_case(self.case, validation, audit)['manual_audit_complete'])
        audit.update(inspected_unit_ids=['g1'], admitted_unit_ids=['g1'], unsupported_unit_ids=['g1'])
        scored = trial.score_case(self.case, validation, audit)
        self.assertTrue(scored['manual_audit_complete'])
        self.assertEqual(scored['unsupported_generated'], 1)

    def test_freeze_records_all_slots_and_refuses_overwrite(self):
        binary = self.root / 'native'
        shutil.copyfile('/usr/bin/false', binary)
        binary.chmod(0o700)
        frozen = self.root / 'qualification'
        manifest = trial.freeze(frozen, binary, 'http://127.0.0.1:8080', 'resident-test', self.root / 'holds')
        self.assertEqual(len(manifest['attempts']), 8)
        self.assertEqual(manifest['budgets']['retries'], 0)
        native = json.loads((frozen / 'cases' / 'herbarium-correct.json').read_text())
        self.assertNotIn('annotations', native)
        with self.assertRaises(FileExistsError):
            trial.freeze(frozen, binary, 'http://127.0.0.1:8080', 'resident-test', self.root / 'holds')

    def test_changed_frozen_binary_prevents_first_attempt(self):
        binary = self.root / 'native'
        shutil.copyfile('/usr/bin/false', binary)
        binary.chmod(0o700)
        frozen = self.root / 'qualification'
        manifest = trial.freeze(frozen, binary, 'http://127.0.0.1:8080', 'resident-test', self.root / 'holds')
        binary.write_bytes(b'changed')
        with self.assertRaises(ValueError):
            trial.validate_frozen(frozen, manifest)

    def test_failed_native_processes_are_all_recorded_and_cannot_retry(self):
        binary = self.root / 'native'
        shutil.copyfile('/usr/bin/false', binary)
        binary.chmod(0o700)
        frozen = self.root / 'qualification'
        trial.freeze(frozen, binary, 'http://127.0.0.1:8080', 'resident-test', self.root / 'holds')
        result = trial.run(frozen)
        self.assertEqual(result['attempts'], 8)
        self.assertEqual(result['failed_attempts'], 8)
        ledger = json.loads((frozen / 'attempts.json').read_text())
        self.assertEqual(len(ledger), 8)
        self.assertTrue(all(a['status'] == 'failed' for a in ledger))
        with self.assertRaises(ValueError):
            trial.run(frozen)

    def test_runtime_drift_during_later_scoring_preserves_all_attempts(self):
        binary = self.root / 'native'
        shutil.copyfile('/usr/bin/false', binary)
        binary.chmod(0o700)
        frozen = self.root / 'qualification'
        trial.freeze(frozen, binary, 'http://127.0.0.1:8080', 'resident-test', self.root / 'holds')
        trial.run(frozen)
        (self.root / 'source.rs').write_text('// changed after run\n')
        result = trial.score_root(frozen)
        self.assertEqual(result['attempts'], 8)
        self.assertEqual(result['failed_attempts'], 8)
        self.assertFalse(result['gate_passed'])

    def test_supported_wrong_bearing_is_not_hidden_by_duplicate_goal_drop(self):
        report = self.report()
        self.change_probe(report, judgment_index=1, status='Supported')
        self.write_report(report)
        row = trial.score_case(self.case, self.validate(), None)
        self.assertEqual(row.get('wrong_bearings_supported', 0), 1)

    def test_unknown_correct_bearings_cannot_pass_the_coverage_gate(self):
        good = dict(valid=True, true_supported=1, uncertain_appropriate=1, false_facts_accepted=0,
                    wrong_bearings_accepted=0, wrong_bearings_supported=0, correct_bearing_supported=0,
                    manual_audit_complete=True, unsupported_generated=0)
        self.assertFalse(trial.summarize([dict(good, case_id=str(i)) for i in range(8)])['gate_passed'])

    def test_admitted_export_must_include_all_actual_durable_accepted_units(self):
        report = self.report()
        descriptor = report['support_reports']['generated']
        support = trial.decode((self.root / descriptor['path']).read_text())
        support['units'] = [dict(disposition='Accepted', unit=dict(id='g1', kind='RecordedEvent', text='Actual admitted statement', citations=[]), status='Supported')]
        report['support_reports']['generated'] = self.artifact(descriptor['path'], trial.encode(support).encode())
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_preinference_seal_cannot_be_replaced_by_matching_final_hash_claims(self):
        report = self.report()
        report['artifacts']['native_transcript'] = self.artifact('session.jsonl', b'changed after inference')
        report['initial_hashes']['native_transcript'] = report['artifacts']['native_transcript']['sha256']
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_later_recall_failure_does_not_erase_observed_false_admission(self):
        report = self.report()
        self.change_probe(report, unit_id='false', status='Supported', admitted=True, withheld=False)
        report['stages']['recall'].update(status='failed', error='later recall failure')
        self.write_report(report)
        row = trial.score_case(self.case, self.validate(), None)
        self.assertFalse(row['valid'])
        self.assertEqual(row['false_facts_accepted'], 1)

    def test_raw_evaluator_status_cannot_differ_from_durable_probe_and_summary(self):
        report = self.report()
        stage = report['stages']['evaluate_probe']
        raw = trial.decode((self.root / stage['reply']['path']).read_text())
        payload = trial.decode(raw['message']['content'][0]['text'])
        payload['checks'][0]['status'] = 'Unknown'
        raw['message']['content'][0]['text'] = trial.encode(payload)
        response = trial.encode(raw).encode()
        stage['reply'] = self.artifact(stage['reply']['path'], response)
        request = (self.root / stage['request']['path']).read_bytes()
        stage['provider_audit'] = self.artifact(stage['provider_audit']['path'], request + response)
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_missing_probe_outcome_is_unknown_even_when_attempt_also_fails(self):
        report = self.report()
        report['stages']['evaluate_probe'].update(status='failed', error='no model reply')
        self.write_report(report)
        row = trial.score_case(self.case, self.validate(), None)
        self.assertFalse(row['valid'])
        self.assertIsNone(row['false_facts_accepted'])
        self.assertIsNone(row['wrong_bearings_supported'])
        self.assertFalse(row['probe_outcome_observed'])
        self.assertEqual(trial.summarize([dict(row, case_id=str(i)) for i in range(8)])['unknown_probe_outcomes'], 8)

    def test_correct_second_offered_bearing_counts_even_when_duplicate_is_dropped(self):
        self.case['annotations']['expected_bearing'] = 'Bad'
        report = self.report()
        self.change_probe(report, judgment_index=0, status='Unknown', bearing='Unknown', disposition='downgraded', reason='UnsupportedJudgment')
        self.change_probe(report, judgment_index=1, status='Supported')
        self.write_report(report)
        row = trial.score_case(self.case, self.validate(), None)
        self.assertTrue(row['valid'])
        self.assertEqual(row['correct_bearing_supported'], 1)
        self.assertEqual(row['wrong_bearings_supported'], 0)
        self.assertEqual(row['wrong_bearings_accepted'], 0)

    def test_probe_summary_cannot_shed_the_raw_unsupported_premise_flag(self):
        report = self.report()
        report['probe']['units'][0]['unsupported_factual_premise'] = True
        self.write_report(report)
        self.assertFalse(self.validate()['valid'])

    def test_missing_durable_reports_or_source_seal_fails_closed(self):
        for field in ('support_reports', 'source_seal'):
            report = self.report()
            del report[field]
            self.write_report(report)
            self.assertFalse(self.validate()['valid'])

    def test_failed_late_pass_preserves_observed_partial_cost_without_claiming_complete_totals(self):
        report = self.report()
        report.update(status='failed', run_error='generation rejected after observed usage',
                      input_tokens=None, output_tokens=None, observed_input_tokens=19, observed_output_tokens=7)
        report['stages']['generate'].update(status='failed', error='max_tokens', input_tokens=19, output_tokens=7)
        for name in ('evaluate_generated', 'evaluate_probe'):
            report['stages'][name].update(status='withheld', error='upstream failed', input_tokens=None, output_tokens=None)
        self.write_report(report)
        row = trial.score_case(self.case, self.validate(), None)
        self.assertFalse(row['valid'])
        self.assertIsNone(row['input_tokens'])
        self.assertIsNone(row['output_tokens'])
        self.assertEqual(row.get('observed_input_tokens'), 19)
        self.assertEqual(row.get('observed_output_tokens'), 7)
        summary = trial.summarize([dict(row, case_id=str(i)) for i in range(8)])
        self.assertEqual(summary['reported_input_tokens'], 152)
        self.assertEqual(summary['reported_output_tokens'], 56)
        self.assertEqual(summary.get('observed_input_tokens'), 152)
        self.assertEqual(summary.get('observed_output_tokens'), 56)
        self.assertFalse(summary['cost_complete'])




if __name__ == '__main__':
    unittest.main()
