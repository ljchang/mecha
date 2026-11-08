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





class GroundedWireTests(unittest.TestCase):
    def packet(self):
        key = 'a' * 64
        text = 'The observed artifact contains item p1.'
        source = {'id': 'source:a', 'source': {'object': 'a', 'revision': '1'},
                  'field': '/text', 'kind': 'ArtifactObservation', 'text': text}
        citation = dict(handle='ArtifactObservation:' + key + ':c0', source_id=source['id'], source=source['source'],
                        field=source['field'], kind=source['kind'], quote=text)
        return dict(protocol='situated-support-wire-v2', request_key=key,
                    criterion={'goal': 'task:test'}, sources=[source],
                    catalog={'request_key': key, 'citations': [citation]})

    def generation(self, packet):
        return dict(request_key=packet['request_key'], units=[dict(id='u1', kind='RecordedEvent',
                    text='The artifact contains item p1.', evidence=[packet['catalog']['citations'][0]['handle']])],
                    judgments=[dict(bearing='Good', because=['u1'])])

    def evaluation(self, packet):
        proposal = trial.expand_generation(packet, self.generation(packet))
        packet.update(proposal=proposal, targets=['unit:u1', 'judgment:0'])
        diag = dict(evidence=[packet['catalog']['citations'][0]['handle']], reason='Exact observed evidence.', questions=[])
        return dict(request_key=packet['request_key'], criterion=dict(diag, state='Met'),
                    units=[dict(diag, target='unit:u1', factual_premises='Supported', status='Supported')],
                    judgments=[dict(diag, target='judgment:0', premises_sufficient=True)])

    def test_host_handles_expand_exact_canonical_identity_and_goal(self):
        packet = self.packet()
        proposal = trial.expand_generation(packet, self.generation(packet))
        self.assertEqual(proposal['units'][0]['citations'], [{'source_id': 'source:a', 'quote': packet['sources'][0]['text']}])
        self.assertEqual(proposal['judgments'][0]['goal'], 'task:test')
        for mutate in (lambda p: p.update(request_key='b'*64), lambda p: p['units'][0].update(goal='wrong'),
                       lambda p: p['units'][0].update(evidence=['foreign']),
                       lambda p: p['units'][0].update(text='é'*257),
                       lambda p: p['units'][0].update(evidence=[packet['catalog']['citations'][0]['handle']]*2)):
            payload = self.generation(packet)
            mutate(payload)
            with self.assertRaises(ValueError):
                trial.expand_generation(packet, payload)

    def test_catalog_reconstructs_balanced_unicode_and_refuses_identity_tampering(self):
        packet = self.packet()
        packet['sources'][0]['text'] = 'é' * 301
        entry = packet['catalog']['citations'][0]
        entry['quote'] = 'é' * 151
        packet['catalog']['citations'].append(dict(entry, handle='ArtifactObservation:' + packet['request_key'] + ':c1', quote='é'*150))
        trial.validate_catalog(packet)
        packet['catalog']['citations'][1]['source']['revision'] = 2
        # Use an independent source locator so mutating the row cannot mutate source too.
        packet['sources'][0]['source'] = {'object': 'a', 'revision': '1'}
        with self.assertRaises(ValueError):
            trial.validate_catalog(packet)

    def test_evaluation_derivation_and_missing_premises_diagnostics(self):
        packet = self.packet()
        payload = self.evaluation(packet)
        self.assertEqual([c['status'] for c in trial.expand_evaluation(packet, payload)['checks']], ['Supported', 'Supported'])
        payload['judgments'][0].update(premises_sufficient=False, questions=['Which premise establishes the criterion?'])
        checks = trial.expand_evaluation(packet, payload)['checks']
        self.assertEqual(checks[1]['status'], 'Unknown')
        self.assertFalse(checks[1]['unsupported_factual_premise'])
        self.assertEqual(checks[1]['questions'], payload['judgments'][0]['questions'])
        payload['units'][0]['factual_premises'] = 'Unknown'
        with self.assertRaises(ValueError):
            trial.expand_evaluation(packet, payload)

    def test_all_bearing_states_and_target_coverage(self):
        for state in ('Met', 'NotMet', 'Unknown'):
            for bearing in ('Good', 'Bad', 'Unknown'):
                for enough in (False, True):
                    packet = self.packet()
                    payload = self.evaluation(packet)
                    packet['proposal']['judgments'][0]['bearing'] = bearing
                    payload['criterion']['state'] = state
                    if state == 'Unknown':
                        payload['criterion']['questions'] = ['What is the observed result?']
                    expected = ('Supported' if state == 'Unknown' else 'Contradicted') if bearing == 'Unknown' else (
                        'Unknown' if state == 'Unknown' else ('Supported' if enough else 'Unknown')
                        if (state, bearing) in (('Met', 'Good'), ('NotMet', 'Bad')) else 'Contradicted')
                    payload['judgments'][0].update(premises_sufficient=enough, questions=['What premise is missing?'] if expected == 'Unknown' else [])
                    self.assertEqual(trial.expand_evaluation(packet, payload)['checks'][1]['status'], expected)
        packet = self.packet()
        payload = self.evaluation(packet)
        payload['units'].append(copy.deepcopy(payload['units'][0]))
        with self.assertRaises(ValueError):
            trial.expand_evaluation(packet, payload)

    def test_false_supported_is_counted_even_when_store_blocks_admission(self):
        old = SupportTrialTests('test_complete_report_is_accepted_with_verified_source_hashes')
        old.setUp()
        self.addCleanup(old.doCleanups)
        report = old.report()
        old.change_probe(report, unit_id='false', status='Supported', admitted=False, withheld=True,
                         unsupported_factual_premise=True)
        old.write_report(report)
        validation = old.validate()
        self.assertTrue(validation['valid'])
        row = trial.score_case(old.case, validation, None, protocol='memory-factual-support-v2')
        self.assertEqual(row['false_facts_supported'], 1)
        self.assertEqual(row['false_facts_accepted'], 0)
        summary = trial.summarize([dict(row, case_id=str(i), manual_audit_complete=True, unsupported_generated=0) for i in range(8)], protocol='memory-factual-support-v2')
        self.assertEqual(summary['false_facts_supported'], 8)
        self.assertFalse(summary['gate_passed'])

    def request(self, packet):
        evidence = {'type': 'array', 'maxItems': 2, 'uniqueItems': True,
                    'items': {'type': 'string', 'enum': [c['handle'] for c in packet['catalog']['citations']]}}
        def obj(properties):
            return dict(type='object', additionalProperties=False, required=list(properties), properties=properties)
        identifier = dict(type='string', minLength=1, maxLength=32, pattern='^[A-Za-z0-9_-]{1,32}$')
        properties = {'request_key': {'type': 'string', 'enum': [packet['request_key']]}}
        if 'proposal' in packet:
            diagnostic = dict(evidence=evidence, reason=dict(type='string', minLength=1, maxLength=128),
                              questions=dict(type='array', maxItems=4, items=dict(type='string', minLength=1, maxLength=64)))
            known = obj(dict(diagnostic, state={'type': 'string', 'enum': ['Met', 'NotMet']},
                             evidence=dict(evidence, minItems=1), questions=dict(diagnostic['questions'], minItems=0, maxItems=0)))
            unknown = obj(dict(diagnostic, state={'type': 'string', 'enum': ['Unknown']},
                               evidence=dict(evidence, minItems=0), questions=dict(diagnostic['questions'], minItems=1)))
            properties['criterion'] = {'oneOf': [known, unknown]}
            for name, targets in [('units', ['unit:' + u['id'] for u in packet['proposal']['units']]),
                                  ('judgments', ['judgment:' + str(i) for i in range(len(packet['proposal']['judgments']))])]:
                item = dict(diagnostic, target={'type': 'string', 'enum': targets})
                if name == 'units':
                    item.update(factual_premises={'type': 'string', 'enum': ['None', 'Supported', 'Contradicted', 'Unknown']},
                                status={'type': 'string', 'enum': ['Supported', 'Contradicted', 'Unknown']})
                else:
                    item['premises_sufficient'] = {'type': 'boolean'}
                properties[name] = dict(type='array', minItems=len(targets), maxItems=len(targets), items=obj(item))
                if not targets:
                    properties[name]['items'] = {'type': 'null'}
        else:
            properties['units'] = dict(type='array', maxItems=8, items=obj(dict(id=identifier,
                kind={'type': 'string', 'enum': ['RecordedEvent', 'GoalAssessment', 'Hypothesis', 'Prediction']},
                text=dict(type='string', minLength=1, maxLength=128), evidence=evidence)))
            properties['judgments'] = dict(type='array', maxItems=8, items=obj(dict(
                bearing={'type': 'string', 'enum': ['Good', 'Bad', 'Unknown']},
                because=dict(type='array', maxItems=8, uniqueItems=True, items=identifier))))
        schema = {'type': 'object', 'additionalProperties': False, 'required': list(properties), 'properties': properties}
        text = '<support-input>\n' + json.dumps(packet, separators=(',', ':'), ensure_ascii=False) + '\n</support-input>'
        request = {'request': {'system': 'Synthetic frame.', 'messages': [{'role': 'user', 'content': [{'type': 'text', 'text': text}]}],
                               'response_schema': schema, 'tools': []},
                   'wire_body': {'response_format': {'type': 'json_object', 'schema': copy.deepcopy(schema)},
                                 'messages': [{'role': 'system', 'content': 'Synthetic frame.'}, {'role': 'user', 'content': text}],
                                 'chat_template_kwargs': {'enable_thinking': False}}}
        stage = {'packet_bytes': len('Synthetic frame.'.encode()) + len(text.encode()) + len(trial.compact_bytes(schema))}
        return request, stage

    def test_actual_schema_binding_catalog_completeness_and_full_budget(self):
        packet = self.packet()
        request, stage = self.request(packet)
        self.assertEqual(trial.wire_packet(request, stage), packet)
        for mutate in (lambda r: r['wire_body'].pop('response_format'),
                       lambda r: r['wire_body']['response_format']['schema'].update(additionalProperties=True),
                       lambda r: r['request']['response_schema']['properties']['request_key'].update(enum=['wrong']),
                       lambda r: r['wire_body']['messages'][1].update(content='different input')):
            changed = copy.deepcopy(request)
            mutate(changed)
            with self.assertRaises(ValueError):
                trial.wire_packet(changed, stage)
        with self.assertRaises(ValueError):
            trial.wire_packet(request, dict(packet_bytes=stage['packet_bytes']-1))

    def test_malformed_wire_shapes_refuse_without_crashing_report_scoring(self):
        for mutate in (lambda r: r['request']['messages'].__setitem__(0, None),
                       lambda r: r['wire_body'].update(chat_template_kwargs=None),
                       lambda r: r['request']['response_schema'].update(properties=[]),
                       lambda r: r['request']['response_schema']['properties']['units'].update(items=None)):
            request, stage = self.request(self.packet())
            mutate(request)
            request['wire_body']['response_format']['schema'] = copy.deepcopy(request['request']['response_schema'])
            if request['request']['messages'][0] is not None:
                stage['packet_bytes'] = len(request['request']['system'].encode()) + len(request['request']['messages'][0]['content'][0]['text'].encode()) + len(trial.compact_bytes(request['request']['response_schema']))
            with self.assertRaises(ValueError):
                trial.wire_packet(request, stage)

    def test_expansion_preserves_distinct_sources_and_rejects_repeated_canonical_quote(self):
        packet = self.packet()
        source = copy.deepcopy(packet['sources'][0])
        source.update(id='source:b', source={'object': 'b', 'revision': '2'})
        packet['sources'].append(source)
        packet['catalog']['citations'].append(dict(packet['catalog']['citations'][0], handle='ArtifactObservation:'+packet['request_key']+':c1',
                                                  source_id=source['id'], source=source['source']))
        payload = self.generation(packet)
        payload['units'][0]['evidence'].append('ArtifactObservation:'+packet['request_key']+':c1')
        self.assertEqual([c['source_id'] for c in trial.expand_generation(packet, payload)['units'][0]['citations']], ['source:a', 'source:b'])
        # Two positions with the same source and quote resolve to one canonical identity.
        packet = self.packet()
        packet['sources'][0]['text'] = 'x'*600
        entry = packet['catalog']['citations'][0]
        entry['quote'] = 'x'*300
        packet['catalog']['citations'].append(dict(entry, handle='ArtifactObservation:'+packet['request_key']+':c1'))
        payload = self.generation(packet)
        payload['units'][0]['evidence'].append('ArtifactObservation:'+packet['request_key']+':c1')
        with self.assertRaises(ValueError):
            trial.expand_generation(packet, payload)

    def test_expanded_unicode_limits_and_target_diagnostics_are_not_truncated(self):
        packet = self.packet()
        packet['sources'][0]['text'] = '🧪'*600
        entry = packet['catalog']['citations'][0]
        entry['quote'] = '🧪'*300
        # Distinct quote avoids duplicate citation rejection; every entry stays exact.
        packet['sources'][0]['text'] = '🧪'*300 + '🐝'*300
        packet['catalog']['citations'].append(dict(entry, handle='ArtifactObservation:'+packet['request_key']+':c1', quote='🐝'*300))
        payload = self.generation(packet)
        payload['units'][0]['evidence'].append('ArtifactObservation:'+packet['request_key']+':c1')
        payload['units'] = [dict(payload['units'][0], id='u'+str(i)) for i in range(8)]
        payload['judgments'] = []
        with self.assertRaises(ValueError):
            trial.expand_generation(packet, payload)
        packet = self.packet()
        payload = self.evaluation(packet)
        payload['criterion']['reason'] = 'é'*257
        with self.assertRaises(ValueError):
            trial.expand_evaluation(packet, payload)

    def grounded_report(self, generated_bearings=()):
        old = SupportTrialTests('test_complete_report_is_accepted_with_verified_source_hashes')
        old.setUp()
        self.addCleanup(old.doCleanups)
        for unit in old.case['probe']['units'][:2]:
            unit['citation']['quote'] = None
        report = old.report()
        report['protocol'] = 'memory-factual-support-v2'
        for stage_name in trial.MODEL_STAGES:
            packet = self.packet()
            criterion = old.case['native']['current_criterion']
            packet['criterion'] = dict(goal='task:synthetic-support', goal_text=old.case['native']['goal_text'], criterion_text=criterion,
                                       criterion_revision=hashlib.sha256(criterion.encode()).hexdigest(),
                                       confirmation_digest=hashlib.sha256(('synthetic-owner-confirmation:' + criterion).encode()).hexdigest())
            observed = old.case['native']['observed_output']
            packet['sources'][0]['text'] = json.dumps(dict(content_status='Complete', artifact_text=observed,
                artifact_digest=hashlib.sha256(observed.encode()).hexdigest(), verdict='Passed'), separators=(',', ':'))
            packet['sources'][0]['field'] = '/artifact_observation'
            packet['catalog']['citations'][0].update(quote=packet['sources'][0]['text'], field='/artifact_observation')
            second = dict(packet['sources'][0], id='source:input', source={'object': 'input', 'revision': '1'},
                          kind='Observation', text=old.case['native']['input_text'])
            packet['sources'].append(second)
            packet['catalog']['citations'].append(dict(handle='Observation:'+packet['request_key']+':c1', source_id=second['id'],
                                                       source=second['source'], field=second['field'], kind=second['kind'], quote=second['text']))
            proposal = {'units': [], 'judgments': []}
            if stage_name == 'evaluate_probe':
                proposal['units'] = [dict(id=u['id'], kind=u['kind'], text=u['text'], citations=[{
                    'source_id': 'source:input' if u['citation']['passage'] == 'input' else 'source:a',
                    'quote': u['citation']['quote'] if u['citation']['quote'] is not None else packet['sources'][0]['text']}]) for u in old.case['probe']['units']]
                proposal['judgments'] = [dict(j, goal='task:synthetic-support') for j in old.case['probe']['judgments']]
            if stage_name != 'evaluate_probe' and generated_bearings:
                proposal['units'] = [dict(id='g1', kind='RecordedEvent', text='The observed artifact lists item p1.',
                                          citations=[dict(source_id='source:a', quote=packet['sources'][0]['text'])])]
                proposal['judgments'] = [dict(goal='task:synthetic-support', bearing=bearing, because=['g1']) for bearing in generated_bearings]
            if stage_name == 'generate':
                payload = dict(request_key=packet['request_key'],
                    units=[dict(id=u['id'], kind=u['kind'], text=u['text'], evidence=[packet['catalog']['citations'][0]['handle']]) for u in proposal['units']],
                    judgments=[{k:j[k] for k in ('bearing', 'because')} for j in proposal['judgments']])
                (old.root/'generated-proposal.json').write_text(trial.encode(proposal))
            else:
                packet.update(proposal=proposal, targets=['unit:'+u['id'] for u in proposal['units']] + ['judgment:'+str(i) for i in range(len(proposal['judgments']))])
                if stage_name == 'evaluate_probe':
                    (old.root/'probe-proposal.json').write_text(trial.encode(proposal))
                diag = dict(evidence=[packet['catalog']['citations'][0]['handle']], reason='Exact observation.', questions=[])
                payload = dict(request_key=packet['request_key'], criterion=dict(diag, state='Met'), units=[], judgments=[])
                for unit in proposal['units']:
                    status = {'false': 'Contradicted', 'uncertain': 'Unknown'}.get(unit['id'], 'Supported')
                    payload['units'].append(dict(diag, target='unit:'+unit['id'], status=status, factual_premises=status,
                                                 questions=['Was approval recorded?'] if status == 'Unknown' else []))
                payload['judgments'] = [dict(diag, target='judgment:'+str(i), premises_sufficient=True) for i in range(len(proposal['judgments']))]
                if stage_name == 'evaluate_generated' and generated_bearings:
                    payload['criterion']['state'] = 'NotMet' if generated_bearings[0] == 'Bad' else 'Met'
            stage = report['stages'][stage_name]
            req = trial.decode((old.root/stage['request']['path']).read_text())
            wire, bounds = self.request(packet)
            req['request'].update(wire['request'])
            req['wire_body'].update(wire['wire_body'])
            req['wire_body'].pop('autoload', None)
            raw = trial.decode((old.root/stage['reply']['path']).read_text())
            raw['message']['content'][0]['text'] = trial.encode(payload)
            request_bytes, response_bytes = trial.encode(req).encode(), trial.encode(raw).encode()
            stage.update(bounds)
            stage['request'] = old.artifact(stage['request']['path'], request_bytes)
            stage['reply'] = old.artifact(stage['reply']['path'], response_bytes)
            stage['provider_audit'] = old.artifact(stage['provider_audit']['path'], request_bytes+response_bytes)
            if stage_name == 'evaluate_probe':
                descriptor = report['support_reports']['probe']
                support = trial.decode((old.root/descriptor['path']).read_text())
                support['units'][0]['unit'] = proposal['units'][0]
                for judgment in support['judgments']:
                    judgment['goal'] = 'task:synthetic-support'
                report['support_reports']['probe'] = old.artifact(descriptor['path'], trial.encode(support).encode())
                for unit in report['probe']['units']:
                    unit['unsupported_factual_premise'] = unit['id'] != 'true'
            if stage_name == 'evaluate_generated' and generated_bearings:
                descriptor = report['support_reports']['generated']
                support = trial.decode((old.root/descriptor['path']).read_text())
                support['units'] = [dict(disposition='Accepted', unit=u, status='Supported') for u in proposal['units']]
                support['judgments'] = [dict(index=i, goal=j['goal'], offered_bearing=j['bearing'],
                    bearing=j['bearing'] if i == 0 else 'Unknown', because=j['because'], status='Supported',
                    reason='Retained' if i == 0 else 'DuplicateGoal') for i,j in enumerate(proposal['judgments'])]
                report['support_reports']['generated'] = old.artifact(descriptor['path'], trial.encode(support).encode())
                descriptor = report['artifacts']['admitted_generated']
                report['artifacts']['admitted_generated'] = old.artifact(descriptor['path'], trial.encode([dict(u,status='Supported') for u in proposal['units']]).encode())
        old.write_report(report)
        return old, report

    def test_complete_v2_audits_raw_canonical_durable_and_probe(self):
        old, report = self.grounded_report()
        validation = trial.validate_report(old.root, old.case, 'resident-test', 'memory-factual-support-v2')
        self.assertTrue(validation['valid'], validation['errors'] + validation['probe_errors'])
        self.assertEqual(validation['probe_outcome']['criterion_state'], 'Met')
        row = trial.score_case(old.case, validation, None, 'memory-factual-support-v2')
        self.assertEqual(row['false_facts_supported'], 0)
        self.assertEqual(row['criterion_correct'], 1)
        descriptor = report['support_reports']['probe']
        support = trial.decode((old.root/descriptor['path']).read_text())
        support['judgments'][0]['reason'] = 'UnsupportedPremises'
        support['judgments'][0]['bearing'] = 'Unknown'
        report['support_reports']['probe'] = old.artifact(descriptor['path'], trial.encode(support).encode())
        old.write_report(report)
        changed = trial.validate_report(old.root, old.case, 'resident-test', 'memory-factual-support-v2')
        self.assertFalse(changed['valid'])
        self.assertIn('durable judgment derivation mismatch', changed['probe_errors'])

    def test_perfect_probes_cannot_hide_supported_or_retained_wrong_generated_bearing(self):
        old, report = self.grounded_report(('Bad', 'Bad'))
        validation = trial.validate_report(old.root, old.case, 'resident-test', trial.GROUNDED_PROTOCOL)
        self.assertTrue(validation['valid'], validation['errors'])
        audit = dict(reviewer_kind='external_manual', reviewer='authored-test-reviewer',
                     report_sha256=validation['report_sha256'], admitted_generated_sha256=report['artifacts']['admitted_generated']['sha256'],
                     inspected_unit_ids=['g1'], unsupported_unit_ids=[], notes='Authored factual unit is supported; generated bearing is wrong.')
        row = trial.score_case(old.case, validation, audit, trial.GROUNDED_PROTOCOL)
        self.assertEqual(row['wrong_bearings_supported'], 0)
        self.assertEqual(row['wrong_generated_bearings_supported'], 2)
        self.assertEqual(row['wrong_generated_bearings_retained'], 1)
        self.assertTrue(row['manual_audit_complete'])
        rows = [dict(row, case_id=str(i)) for i in range(8)]
        summary = trial.summarize(rows, trial.GROUNDED_PROTOCOL)
        self.assertTrue(summary['generated_metrics_complete'])
        self.assertEqual(summary['wrong_generated_bearings_supported'], 16)
        self.assertFalse(summary['gate_passed'])
        corrected = [dict(r, wrong_generated_bearings_supported=0, wrong_generated_bearings_retained=0) for r in rows]
        self.assertTrue(trial.summarize(corrected, trial.GROUNDED_PROTOCOL)['gate_passed'])

    def test_partial_generated_outcome_survives_probe_failure(self):
        old, report = self.grounded_report(('Bad',))
        report.update(status='failed', run_error='probe transport failed')
        report['stages']['evaluate_probe'].update(status='failed', error='probe transport failed')
        old.write_report(report)
        validation = trial.validate_report(old.root, old.case, 'resident-test', trial.GROUNDED_PROTOCOL)
        self.assertFalse(validation['valid'])
        self.assertIsNone(validation['probe_outcome'])
        self.assertIsNotNone(validation['generated_outcome'], validation['generated_errors'])
        row = trial.score_case(old.case, validation, None, trial.GROUNDED_PROTOCOL)
        self.assertEqual(row['wrong_generated_bearings_supported'], 1)
        self.assertEqual(row['wrong_generated_bearings_retained'], 1)
        summary = trial.summarize([row], trial.GROUNDED_PROTOCOL)
        self.assertIsNone(summary['wrong_generated_bearings_supported'])
        self.assertEqual(summary['observed_wrong_generated_bearings_supported'], 1)
        self.assertEqual(summary['unknown_generated_outcomes'], 7)
        self.assertFalse(summary['gate_passed'])

    def test_missing_generated_evaluation_stays_unknown_in_gate_counts(self):
        old, report = self.grounded_report()
        report.update(status='failed', run_error='generated evaluator failed')
        report['stages']['evaluate_generated'].update(status='failed', error='generated evaluator failed')
        old.write_report(report)
        validation = trial.validate_report(old.root, old.case, 'resident-test', trial.GROUNDED_PROTOCOL)
        row = trial.score_case(old.case, validation, None, trial.GROUNDED_PROTOCOL)
        self.assertFalse(row['generated_outcome_observed'])
        self.assertIsNone(row['wrong_generated_bearings_supported'])
        summary = trial.summarize([dict(row, case_id=str(i)) for i in range(8)], trial.GROUNDED_PROTOCOL)
        self.assertFalse(summary['generated_metrics_complete'])
        self.assertEqual(summary['unknown_generated_outcomes'], 8)
        self.assertIsNone(summary['wrong_generated_bearings_retained'])
        self.assertEqual(summary['observed_wrong_generated_bearings_retained'], 0)
        self.assertFalse(summary['gate_passed'])

    def rebind_schema(self, request, stage):
        schema = request['request']['response_schema']
        request['wire_body']['response_format']['schema'] = copy.deepcopy(schema)
        stage['packet_bytes'] = len(request['request']['system'].encode()) + len(request['request']['messages'][0]['content'][0]['text'].encode()) + len(trial.compact_bytes(schema))

    def test_role_prefixed_handles_reject_absent_or_mismatched_source_kind(self):
        packet = self.packet()
        entry = packet['catalog']['citations'][0]
        entry['handle'] = 'ArtifactObservation:' + packet['request_key'] + ':c0'
        trial.validate_catalog(packet)
        for prefix in ('', 'Observation:'):
            entry['handle'] = prefix + packet['request_key'] + ':c0'
            with self.assertRaises(ValueError):
                trial.validate_catalog(packet)

    def test_utf8_safe_grammar_rejects_previous_character_limits(self):
        packet = self.packet()
        request, stage = self.request(packet)
        request['request']['response_schema']['properties']['units']['items']['properties']['text']['maxLength'] = 512
        self.rebind_schema(request, stage)
        with self.assertRaises(ValueError):
            trial.wire_packet(request, stage)
        self.evaluation(packet)
        for field, old_limit in (('reason', 512), ('questions', 256)):
            request, stage = self.request(packet)
            properties = request['request']['response_schema']['properties']['units']['items']['properties']
            (properties[field]['items'] if field == 'questions' else properties[field])['maxLength'] = old_limit
            self.rebind_schema(request, stage)
            with self.assertRaises(ValueError):
                trial.wire_packet(request, stage)

    def test_criterion_grammar_requires_known_evidence_or_unknown_questions(self):
        packet = self.packet()
        self.evaluation(packet)
        request, stage = self.request(packet)
        trial.wire_packet(request, stage)
        criterion = request['request']['response_schema']['properties']['criterion']
        self.assertIn('oneOf', criterion)
        for branch, field, value in ((0, 'evidence', 0), (1, 'questions', 0)):
            changed = copy.deepcopy(request)
            changed['request']['response_schema']['properties']['criterion']['oneOf'][branch]['properties'][field]['minItems'] = value
            self.rebind_schema(changed, stage)
            with self.assertRaises(ValueError):
                trial.wire_packet(changed, stage)

    def test_v2_actual_body_omits_url_autoload_policy_and_audit_names_boundary(self):
        old, report = self.grounded_report()
        validation = trial.validate_report(old.root, old.case, 'resident-test', 'memory-factual-support-v2')
        self.assertTrue(validation['valid'], validation['errors'])
        boundary = validation['transport_audit_boundary']
        self.assertIn('URL', boundary)
        self.assertIn('not recorded', boundary)
        stage = report['stages']['generate']
        request = trial.decode((old.root/stage['request']['path']).read_text())
        request['wire_body']['autoload'] = False
        request_bytes = trial.encode(request).encode()
        stage['request'] = old.artifact(stage['request']['path'], request_bytes)
        stage['provider_audit'] = old.artifact(stage['provider_audit']['path'], request_bytes+(old.root/stage['reply']['path']).read_bytes())
        old.write_report(report)
        validation = trial.validate_report(old.root, old.case, 'resident-test', 'memory-factual-support-v2')
        self.assertFalse(validation['valid'])
        self.assertIn('unexpected body autoload field', validation['errors'])

    def test_v2_binds_every_stage_to_frozen_current_criterion_and_sources(self):
        for name in trial.MODEL_STAGES:
            for mutation in ('criterion_text', 'missing', 'goal', 'source_role'):
                old, report = self.grounded_report()
                stage = report['stages'][name]
                request = trial.decode((old.root/stage['request']['path']).read_text())
                block = request['request']['messages'][0]['content'][0]
                packet = trial.decode(block['text'][len('<support-input>\n'):-len('\n</support-input>')])
                if mutation == 'missing':
                    packet['criterion'].pop('goal_text', None)
                elif mutation == 'source_role':
                    packet['sources'][1]['kind'] = 'AssistantStatement'
                    packet['catalog']['citations'][1]['kind'] = 'AssistantStatement'
                else:
                    packet['criterion'][mutation] = 'historical or foreign goal'
                block['text'] = '<support-input>\n' + json.dumps(packet, separators=(',', ':')) + '\n</support-input>'
                request['wire_body']['messages'][1]['content'] = block['text']
                stage['packet_bytes'] = len(request['request']['system'].encode()) + len(block['text'].encode()) + len(trial.compact_bytes(request['request']['response_schema']))
                request_bytes = trial.encode(request).encode()
                response_bytes = (old.root/stage['reply']['path']).read_bytes()
                stage['request'] = old.artifact(stage['request']['path'], request_bytes)
                stage['provider_audit'] = old.artifact(stage['provider_audit']['path'], request_bytes+response_bytes)
                old.write_report(report)
                validation = trial.validate_report(old.root, old.case, 'resident-test', 'memory-factual-support-v2')
                self.assertFalse(validation['valid'], (name, mutation))
                if name == 'evaluate_probe':
                    self.assertIsNone(validation['probe_outcome'], mutation)

    def test_explicit_v2_fixture_freeze_preserves_v1_default(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binary = base / 'native'
            binary.write_text('#!/bin/sh\nexit 0\n')
            binary.chmod(0o700)
            fixture = base / 'selected.json'
            fixture.write_bytes(trial.FIXTURE.read_bytes())
            with patch.object(trial, 'source_hashes', return_value={}):
                manifest = trial.freeze(base/'trial', binary, 'http://127.0.0.1:1', 'test', base/'holds',
                                        protocol='memory-factual-support-v2', fixture=fixture)
                self.assertEqual(manifest['protocol'], 'memory-factual-support-v2')
                self.assertEqual(manifest['fixture_path'], str(fixture.resolve()))
                self.assertEqual(len(trial.validate_frozen(base/'trial', manifest)), 8)
                fixture.write_text('{}')
                with self.assertRaises(ValueError):
                    trial.validate_frozen(base/'trial', manifest)

if __name__ == '__main__':
    unittest.main()
