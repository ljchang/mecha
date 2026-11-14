"""Opt-in v4 CLI qualification with existing synthetic fixture, zero inference."""
import copy
import json
import os
import subprocess
import unittest

import constraint_abstention_trial as transport
import outcome_reappraisal_trial as trial
import test_outcome_reappraisal_trial as existing


class ProvenanceProvider(transport.QualificationProvider):
    def do_GET(self):
        if self.path == '/models':
            self.server.discovery_entered.set()
            if not self.server.discovery_release.wait(30):
                self.send_error(500)
                return
        super().do_GET()


@unittest.skipUnless(all(os.environ.get(name) for name in (
    'MECHA_OUTCOME_QUALIFICATION_BINARY', 'MECHA_OUTCOME_QUALIFICATION_DB',
    'MECHA_OUTCOME_QUALIFICATION_ACTION')),
    'opt-in v4 CLI qualification requires explicit binary, retained synthetic DB and action')
class ComparisonProvenanceQualification(unittest.TestCase):
    def setUp(self):
        # Existing backup, isolated home, credential stripping and discovery barrier.
        existing.ActualCliQualification.setUp(self)
        self.server.RequestHandlerClass = ProvenanceProvider
        self.server.answers = []
        self.server.raw_requests = []
        self.server.schema_orders = []
        self.copy_before = trial.database_state(self.db)

    def tearDown(self):
        existing.ActualCliQualification.tearDown(self)

    def base(self):
        argv = existing.ActualCliQualification.base(self)
        argv[argv.index('--model')+1] = transport.action.MODEL
        return argv + ['--response-format', 'feasibility-provenance']

    def invoke(self, argv):
        result = subprocess.run(argv, env=self.env, cwd=self.root,
                                text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        return trial.decode(result.stdout)

    def assert_audit(self, decision, request, answer):
        passages = request['passages']
        actual = len(passages) == 4
        for passage in passages:
            self.assertEqual(trial.hash_text(passage['text']), passage['sha256'])
            self.assertEqual(trial.hash_text(passage['projection']['text']), passage['projection']['sha256'])
        effective = dict(Nori='e4' if actual else 'e1', Arden='e2', Quill='e3')
        wanted = dict(
            supplied_source_ids=[p['id'] for p in passages],
            sources=[dict(id=p['id'], source=p['source'], original_sha256=p['sha256'],
                          projection_sha256=p['projection']['sha256'],
                          role=('effective_actual' if p['id']=='e4' else
                                'superseded_expectation' if actual and p['id']=='e1' else
                                'effective_expectation')) for p in passages],
            method_effective_sources=effective,
            model_omitted_packet_ids=[p['id'] for p in passages if p['id'] not in answer['citations']],
            missing_effective_citation_ids=[p['id'] for p in passages
                if p['id'] in effective.values() and p['id'] not in answer['citations']])
        self.assertEqual(decision['comparison_provenance'], wanted)
        self.assertEqual(decision['reason_status'], 'unverified_model_prose')

    def test_actual_v4_positive_negatives_and_legacy_control(self):
        expected_http = []
        for scenario, response_format in (
            ('omit_superseded', 'feasibility-provenance'),
            ('missing_actual', 'feasibility-provenance'),
            ('false_check', 'feasibility-provenance'),
            ('legacy', 'feasibility-first')):
            with self.subTest(scenario=scenario):
                argv = self.base()
                argv[argv.index('--response-format')+1] = response_format
                prepared = self.invoke(argv+['--prepare-only'])
                v4 = scenario != 'legacy'
                self.assertEqual(prepared['protocol'], 'outcome-reappraisal-v4' if v4 else 'outcome-reappraisal-v3')
                self.assertEqual(prepared['response_format'], 'feasibility_provenance' if v4 else 'feasibility_first')
                source = trial.decode(prepared['read']['text'])
                self.assertEqual(source['observations'], [dict(choice=n, observed_minutes=m, energy=e, quality=q)
                    for n,(m,e,q) in trial.SOURCE.items()])
                observation = trial.decode(prepared['observation']['text'])
                self.assertEqual(observation['observation']['choice'], 'Nori')
                self.assertEqual(observation['observation']['outcome'], dict(status='observed',observed_minutes=4,energy=9,quality=8))
                answers = []
                for arm in trial.ARMS:
                    request = transport.arm_request(prepared, arm)
                    self.assertEqual([p['id'] for p in request['passages']],
                                     ['e1','e2','e3'] + (['e4'] if arm=='source_and_outcome' else []))
                    for passage in request['passages']:
                        self.assertEqual(passage['source'], prepared['observation']['source']
                            if passage['id']=='e4' else prepared['read']['observation'])
                    actual = arm == 'source_and_outcome'
                    measurements = dict(trial.SOURCE, Nori=(4,9,8)) if actual else trial.SOURCE
                    checks = {n:dict(source='e4' if actual and n=='Nori' else f'e{i+1}',
                        minutes=m, energy=e, quality=q, energy_ok=e<=4, quality_ok=q>=7, feasible=e<=4 and q>=7)
                        for i,(n,(m,e,q)) in enumerate(measurements.items())}
                    citations = ['e2','e3','e4'] if actual else ['e1','e2','e3']
                    if actual and scenario=='missing_actual': citations = ['e1','e2','e3']
                    if actual and scenario=='false_check': checks['Arden']['energy'] = 1
                    answer = dict(candidate_checks=checks, choice='Arden' if actual else 'Nori',
                                  citations=citations, reason='Authored qualification 🌱; no inference.')
                    answers.append(answer)
                    expected_http.append(transport.expected_http_request(request, transport.TREATMENTS[1], 59, transport.action.MODEL))
                self.server.answers.extend(copy.deepcopy(answers))
                report = self.invoke(argv+['--snapshot',trial.encode(prepared['snapshot']), '--endpoint',self.endpoint,
                    '--holds-dir',str(self.holds),'--seed','59'])
                self.assertEqual(self.server.answers, [], report)
                self.assertEqual(report['preparation'], prepared)
                self.assertEqual(report['protocol'], prepared['protocol'])
                self.assertEqual(report['arm_order'], list(trial.ARMS))
                self.assertEqual(len(report['rows']), 2)
                self.assertFalse(report['executed'])
                if v4: self.assertEqual(report['citation_policy'], 'effective-measurements-v1')
                else: self.assertNotIn('citation_policy', report)
                for row, answer in zip(report['rows'], answers):
                    self.assertIsNone(row['error'])
                    self.assertIsNone(row['call']['error'])
                    self.assertEqual(row['call']['reply']['text'], json.dumps(answer,ensure_ascii=False,sort_keys=False))
                    decision = row['decision']
                    self.assertEqual(decision['citations'], answer['citations'])
                    self.assertEqual(decision['candidate_checks'], answer['candidate_checks'])
                    self.assertEqual(decision['reason'], answer['reason'])
                    self.assertFalse(decision['executed'])
                    request = transport.arm_request(prepared,row['arm'])
                    self.assertEqual(row['call']['prompt'], request['prompt'])
                    self.assertEqual(row['call']['response_schema'],request['response_schema'])
                    self.assertEqual(decision['request_sha256'],next(a['request_sha256'] for a in prepared['arms'] if a['arm']==row['arm']))
                    if v4: self.assert_audit(decision, request, answer)
                    else:
                        self.assertNotIn('comparison_provenance', decision)
                        self.assertNotIn('reason_status', decision)
                    withheld = row['arm']=='source_and_outcome' and scenario!='omit_superseded'
                    self.assertEqual(decision['disposition'], 'withheld' if withheld else 'recommend')
                self.assertEqual(trial.database_state(self.db),self.copy_before)
        self.assertEqual(len(self.server.requests),8)
        transport.audit_http_requests(expected_http,self.server.requests)
        self.assertEqual(self.server.schema_orders,[['candidate_checks','choice','citations','reason']]*8)
        self.assertEqual([list(json.loads(raw)['response_format']['schema']['properties'])
                          for raw in self.server.raw_requests],self.server.schema_orders)

    def test_v4_revocation_before_dispatch(self):
        existing.ActualCliQualification.test_source_revoked_during_models_await_has_zero_completions_and_no_report(self)
        self.assertEqual(self.server.answers, [])
        self.assertEqual(self.server.raw_requests, [])


if __name__ == '__main__':
    unittest.main()
