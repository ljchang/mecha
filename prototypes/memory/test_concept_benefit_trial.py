"""Offline contract/oracle regressions; entirely synthetic and network-free."""
import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path
import concept_benefit_trial as t


class OracleTests(unittest.TestCase):
    def test_finite_world_targets_and_unknown_optima(self):
        fixture, gold = t.load_fixtures()
        self.assertEqual([t.oracle(c, gold)['expected'] for c in gold['cases']],
                         [['alternate'], ['primary'], ['insufficient_evidence'],
                          ['alternate'], ['repair_shared'], ['insufficient_evidence']])
        for c in gold['cases']:
            result = t.oracle(c, gold)
            self.assertEqual(result['completion_count'], 2 if None in c['observed'].values() else 1)
        self.assertEqual(t.oracle(gold['cases'][2], gold)['hidden_preferred'], ['alternate'])
        self.assertEqual(len(fixture['cases']), 6)

    def test_revised_cases_have_fresh_costs_and_shared_exception(self):
        fixture,gold=t.load_fixtures()
        self.assertNotEqual(fixture['cases'][0]['facts'][0]['text'],fixture['cases'][3]['facts'][0]['text'])
        self.assertEqual(gold['cases'][3]['costs'],dict(primary=4,alternate=6,repair_shared=11))
        self.assertTrue(gold['cases'][4]['observed']['primary_clear'])
        altered=copy.deepcopy(gold['cases'][3]);altered['costs']['repair_shared']=1
        self.assertEqual(t.oracle(altered,gold)['expected'],['repair_shared'])

    def test_ties_are_all_permitted(self):
        _, gold = t.load_fixtures()
        gold['cases'][1]['costs']['primary'] = gold['cases'][1]['costs']['alternate']
        self.assertEqual(t.oracle(gold['cases'][1], gold)['expected'], ['alternate', 'primary'])

    def test_fixture_gold_leaks_and_duplicate_ids_rejected(self):
        fixture, gold = t.load_fixtures()
        fixture['cases'][0]['expected'] = 'alternate'
        with self.assertRaises(ValueError): t.validate_fixture(fixture, gold)
        fixture, gold = t.load_fixtures()
        fixture['cases'][0]['facts'][0]['id'] = fixture['training'][0]['id']
        with self.assertRaises(ValueError): t.validate_fixture(fixture, gold)


class ScoringTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fixture, self.gold = t.load_fixtures()
        self.report = dict(version=1, protocol=t.PROTOCOL, fixture_sha256=None,
                           planned_calls=14, actual_calls=14, revision_outcome='revised',
                           stages=[], pairs=[], source_bindings={})
        self.put('fixture.json', self.fixture)
        self.report['fixture_sha256'] = self.hash('fixture.json')
        for source in self.fixture['training']+[self.fixture['counterexample']]+[f for c in self.fixture['cases'] for f in c['facts']]:
            self.report['source_bindings'][source['id']]=dict(object=source['id'],revision=source['id']+'_rev')
        for plan in t.planned_stages(self.fixture):
            row = dict(plan, status='withheld', admission=None, request_key=None,
                       request_path=None, request_sha256=None, response_path=None,
                       response_sha256=None, usage=None, elapsed_ms=None)
            if plan['kind'] == 'decision':
                case = next(c for c in self.fixture['cases'] if c['id'] == plan['case_id'])
                g = next(c for c in self.gold['cases'] if c['id'] == plan['case_id'])
                answer = dict(request_key=plan['id'], choice=t.oracle(g,self.gold)['expected'][0],
                              because=g['sufficient_premise_sets'][0], reason='Synthetic comparison.',
                              question='Is the unobserved prerequisite satisfied?' if None in g['observed'].values() else None)
                def bound(source):return dict(source,reference=self.report['source_bindings'][source['id']])
                evidence=dict(episodes=[bound(x) for x in self.fixture['training']+([self.fixture['counterexample']] if case['phase']=='revised' else [])],facts=[bound(x) for x in case['facts']])
                packet = dict(protocol='memory-concept-decision-v1', evidence=evidence, question=case['question'],
                              options=case['options'], provisional_concept=None if plan['arm']=='baseline' else dict(provisional='synthetic'))
                key=hashlib.sha256((plan['id']+':'+json.dumps(packet,separators=(',',':'),ensure_ascii=False)).encode()).hexdigest()
                packet['request_key']=key;answer['request_key']=key
                req = dict(messages=[dict(content=[dict(type='text', text=json.dumps(packet))])])
                row.update(status='accepted', admission='accepted', request_key=key,
                           request_path=plan['id']+'.request.json', response_path=plan['id']+'.response.json',
                           usage=dict(input_tokens=7,output_tokens=3), elapsed_ms=5)
                self.put(row['request_path'],req); self.put(row['response_path'],answer)
                row['request_sha256']=self.hash(row['request_path']);row['response_sha256']=self.hash(row['response_path'])
            self.report['stages'].append(row)
        for case in self.fixture['cases']:
            self.report['pairs'].append(dict(phase=case['phase'],case_id=case['id'],evidence_sha256=hashlib.sha256(json.dumps(json.loads(json.loads((self.root/f"{case['phase']}.{case['id']}.baseline.request.json").read_text())['messages'][0]['content'][0]['text'])['evidence'],separators=(',',':'),ensure_ascii=False).encode()).hexdigest(),
                baseline_stage=f"{case['phase']}.{case['id']}.baseline", concept_stage=f"{case['phase']}.{case['id']}.concept"))

    def put(self,path,value):
        (self.root/path).write_text(json.dumps(value,ensure_ascii=False),encoding='utf8')
    def hash(self,path): return hashlib.sha256((self.root/path).read_bytes()).hexdigest()
    def score(self): return t.score_report(self.report,self.root,self.fixture,self.gold)
    def first(self): return next(r for r in self.report['stages'] if r['kind']=='decision')
    def change(self, row, **updates):
        p=row['response_path']; a=json.loads((self.root/p).read_text());a.update(updates)
        self.put(p,a);row['response_sha256']=self.hash(p)

    def test_correct_choice_and_missing_premises_are_separate(self):
        self.assertEqual(self.score()['metrics']['accepted_correct'],12)
        self.change(self.first(),because=[])
        row=self.score()['decisions'][0]
        self.assertTrue(row['raw_choice_correct']);self.assertFalse(row['premise_coverage'])
        self.change(self.first(),choice='primary')
        self.assertFalse(self.score()['decisions'][0]['raw_choice_correct'])

    def test_false_certainty_can_match_hidden_optimum(self):
        row=next(r for r in self.report['stages'] if r['case_id']=='i_unknown')
        self.change(row,choice='alternate',question=None)
        scored=next(r for r in self.score()['decisions'] if r['id']==row['id'])
        self.assertTrue(scored['false_certainty']);self.assertTrue(scored['hidden_optimum_match'])
        self.assertFalse(scored['raw_choice_correct'])

    def test_absent_and_started_rows_preserve_denominator(self):
        self.report['stages'].remove(self.first())
        self.report['stages'][1]['status']='started'
        s=self.score();self.assertEqual(len(s['stages']),14)
        self.assertEqual(s['metrics']['decision_denominator'],12)
        self.assertEqual(s['metrics']['accepted_correct'],10)

    def test_tampered_artifact_never_scores(self):
        row=self.first();(self.root/row['response_path']).write_text('{}')
        score=self.score()['decisions'][0]
        self.assertFalse(score['artifact_integrity']);self.assertFalse(score['raw_choice_correct'])

    def test_rejected_answer_preserves_raw_but_not_accepted_score(self):
        self.first().update(status='failed',admission='rejected')
        row=self.score()['decisions'][0]
        self.assertTrue(row['raw_choice_correct']);self.assertFalse(row['accepted_correct'])

    def test_unchanged_and_abstained_cannot_claim_revision_benefit(self):
        for outcome in ('unchanged','abstained'):
            self.report['revision_outcome']=outcome
            s=self.score();self.assertEqual(s['follow_up'],'unchanged_concept_follow_up')
            self.assertFalse(s['revision_benefit_identified'])
        self.report['revision_outcome']='revised'
        s=self.score();self.assertEqual(s['follow_up'],'revised_concept_utility')
        self.assertFalse(s['revision_benefit_identified'])

    def test_duplicate_original_bindings_cannot_fake_independent_evidence(self):
        self.report['source_bindings']['train_south']=self.report['source_bindings']['train_north']
        row=self.first();request=json.loads((self.root/row['request_path']).read_text())
        packet=json.loads(request['messages'][0]['content'][0]['text'])
        packet['evidence']['episodes'][1]['reference']=self.report['source_bindings']['train_north']
        packet.pop('request_key');key=hashlib.sha256((row['id']+':'+json.dumps(packet,separators=(',',':'),ensure_ascii=False)).encode()).hexdigest()
        packet['request_key']=key;row['request_key']=key
        request['messages'][0]['content'][0]['text']=json.dumps(packet)
        self.put(row['request_path'],request);row['request_sha256']=self.hash(row['request_path']);self.change(row,request_key=key)
        self.assertFalse(self.score()['decisions'][0]['artifact_integrity'])

    def test_partial_report_remains_full_and_withheld(self):
        self.report['stages']=[];self.report['pairs']=[];self.report['revision_outcome']=None
        s=self.score();self.assertEqual(len(s['stages']),14)
        self.assertEqual(s['metrics']['accepted_correct'],0)
        self.assertEqual(s['follow_up'],'withheld')
        self.assertEqual(len(s['pairs']),6)
        self.assertTrue(all(p['accepted_correct_delta'] is None for p in s['pairs']))

    def test_tampered_pair_hash_preserves_individual_raw_but_disqualifies_pair(self):
        self.report['pairs'][0]['evidence_sha256']='0'*64
        scored=self.score()
        self.assertTrue(scored['decisions'][0]['raw_choice_correct'])
        self.assertFalse(scored['pairs'][0]['evidence_equal_and_bound'])
        self.assertEqual(scored['pair_denominator'],6)

    def test_request_evidence_and_identity_recomputed_independently(self):
        row=self.first();request=json.loads((self.root/row['request_path']).read_text())
        packet=json.loads(request['messages'][0]['content'][0]['text'])
        packet['evidence']['facts'][0]['text']='Invented rule'
        packet.pop('request_key');key=hashlib.sha256((row['id']+':'+json.dumps(packet,separators=(',',':'),ensure_ascii=False)).encode()).hexdigest()
        packet['request_key']=key;row['request_key']=key
        request['messages'][0]['content'][0]['text']=json.dumps(packet)
        self.put(row['request_path'],request);row['request_sha256']=self.hash(row['request_path'])
        self.change(row,request_key=key)
        self.assertFalse(self.score()['decisions'][0]['artifact_integrity'])

    def test_packet_key_is_not_trusted_as_host_assertion(self):
        row=self.first();row['request_key']='forged'
        request=json.loads((self.root/row['request_path']).read_text())
        packet=json.loads(request['messages'][0]['content'][0]['text']);packet['request_key']='forged'
        request['messages'][0]['content'][0]['text']=json.dumps(packet)
        self.put(row['request_path'],request);row['request_sha256']=self.hash(row['request_path'])
        self.change(row,request_key='forged')
        self.assertFalse(self.score()['decisions'][0]['artifact_integrity'])

    def test_paired_current_questions_must_match_frozen_fixture(self):
        row=self.first();request=json.loads((self.root/row['request_path']).read_text())
        packet=json.loads(request['messages'][0]['content'][0]['text']);packet['question']='Different decision'
        request['messages'][0]['content'][0]['text']=json.dumps(packet)
        self.put(row['request_path'],request);row['request_sha256']=self.hash(row['request_path'])
        self.assertFalse(self.score()['decisions'][0]['artifact_integrity'])

    def test_all_stage_artifacts_are_checked_and_costs_partitioned(self):
        row=self.report['stages'][0]
        row.update(request_path='bad.json',request_sha256='0'*64,status='failed')
        self.put('bad.json',{})
        scored=self.score()
        self.assertFalse(scored['stage_audit'][0]['request_integrity'])
        self.assertEqual(scored['costs_by_group']['induction']['unknown_usage_slots'],1)
        self.assertEqual(scored['by_phase_arm']['initial.baseline']['denominator'],3)

    def test_raw_malformed_json_and_duplicate_fields_are_not_repaired(self):
        row=self.first()
        for text in ('null','[]','{"choice":"primary","choice":"alternate"}','```json\n{}\n```'):
            with self.subTest(text=text):
                (self.root/row['response_path']).write_text(text)
                row['response_sha256']=self.hash(row['response_path'])
                scored=self.score()['decisions'][0]
                self.assertFalse(scored['contract_valid'])
                self.assertFalse(scored['raw_choice_correct'])

    def test_status_or_admission_alone_cannot_certify_response(self):
        row=self.first();self.change(row,reason=None)
        self.assertFalse(self.score()['decisions'][0]['accepted_correct'])

    def test_unknown_duplicate_foreign_and_missing_question_rejected(self):
        row=self.first()
        for updates in ({'reason':' '},{'reason':'x'*513},{'question':'x'*161},{'extra':1},{'because':['concept']},{'because':['train_north','train_north']},
                        {'choice':'insufficient_evidence','question':''},{'request_key':'other'}):
            with self.subTest(updates=updates):
                original=(self.root/row['response_path']).read_bytes()
                self.change(row,**updates)
                self.assertFalse(self.score()['decisions'][0]['contract_valid'])
                (self.root/row['response_path']).write_bytes(original)
                row['response_sha256']=self.hash(row['response_path'])

    def test_duplicate_or_unplanned_rows_and_fixture_tamper_rejected(self):
        self.report['stages'].append(copy.deepcopy(self.first()))
        with self.assertRaises(ValueError): self.score()
        self.report['stages'].pop();self.report['stages'][0]['id']='foreign'
        with self.assertRaises(ValueError): self.score()
        self.report['stages'][0]['id']='induction';self.put('fixture.json',{})
        with self.assertRaises(ValueError): self.score()

    def test_paths_cannot_escape_artifact_root(self):
        self.first()['response_path']='../outside.json'
        self.assertFalse(self.score()['decisions'][0]['artifact_integrity'])

    def test_prose_is_unreviewed_and_costs_are_distinct(self):
        s=self.score();self.assertEqual(s['prose_support'],'unreviewed')
        self.assertEqual(s['costs']['known_input_tokens'],84)
        self.assertEqual(s['costs']['unknown_usage_slots'],2)
        self.assertEqual(s['metrics']['stage_denominator'],14)


if __name__=='__main__': unittest.main()
