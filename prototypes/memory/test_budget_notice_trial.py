import copy
import tempfile
import unittest
from pathlib import Path
import budget_notice_trial as trial

class ComparisonTests(unittest.TestCase):
    def test_pairs_change_only_notice(self):
        for case in trial.frozen_cases():
            a=trial.make_request(case,'pair',False)
            b=trial.make_request(case,'pair',True)
            self.assertEqual(a,b)
            self.assertNotIn('expected_rows',a['files'])
        self.assertEqual(len(trial.planned_pairs()),8)
        self.assertEqual({p['seed'] for p in trial.planned_pairs()},{70400,70401})

    def test_unavailable_reuses_original_development_task_contract(self):
        original=trial.bridge.frozen_cases()[0]['B']
        reused=next(c for c in trial.frozen_cases() if c['id']=='unavailable')['B']
        for field in ('prompt','files','criterion_text'):
            self.assertEqual(reused[field],original[field])
        self.assertEqual(reused['expected_rows'],trial.bridge.ABSTENTION)
        self.assertNotEqual(original['expected_rows'],trial.bridge.ABSTENTION)

    def test_solvable_abstention_fails(self):
        case=next(c for c in trial.frozen_cases() if c['id']=='recoverable_clue')
        score=trial.score(case,trial.bridge.ABSTENTION)
        self.assertFalse(score['task_correct'])
        self.assertTrue(score['premature_abstention'])
        self.assertFalse(score['appropriate_abstention'])

    def test_wire_exposure_requires_harness_message(self):
        text=next(iter(trial.NOTICE_TEXTS))
        self.assertFalse(trial.notice_exposure([{'messages':[{'role':'assistant','content':text}]}])['exposed'])
        self.assertTrue(trial.notice_exposure([{'messages':[{'role':'user','content':text}]}])['exposed'])
        self.assertFalse(trial.notice_exposure([])['exposed'])
        for content in ('Other harness context'+text,[{'type':'text','text':text}]):
            self.assertTrue(trial.notice_exposure([{'messages':[{'role':'user','content':content}]}])['exposed'])
        self.assertFalse(trial.notice_exposure([{'messages':[{'role':'user','content':'Run budget: two model turns remain'}]}])['exposed'])

    def test_execute_retains_every_failed_slot_without_retry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'trial'
            manifest=trial.freeze(root,'/usr/bin/true','http://localhost:1','synthetic',Path(temporary)/'holds')
            calls=[]
            def fail(*args):
                calls.append(args)
                raise RuntimeError('authored launch failure')
            result=trial.execute(root,manifest,fail)
            self.assertEqual(len(result['slots']),16)
            self.assertEqual(len(calls),14) # 12 B starts plus two failed source setups
            self.assertEqual(result['arms']['notice']['missing_artifact'],8)
            self.assertEqual(result['arms']['control']['missing_artifact'],8)
            self.assertFalse(result['integrity_passed'])

    def test_frozen_inputs_reject_postregistration_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)/'trial'
            manifest=trial.freeze(root,'/usr/bin/true','http://localhost:1','synthetic',Path(temporary)/'holds')
            (root/'cases.json').write_text('[]')
            with self.assertRaises(ValueError): trial.validate_frozen(root,manifest)

    def test_hidden_gold_probe_requires_exercised_native_success(self):
        canary='HOST_ORACLE_ONLY_719430285'
        wire=[{'messages':[{'role':'user','content':'synthetic task'}]}]*2
        successful=dict(status='recorded',returncode=0,native=dict(status='completed',run_error=None,
                        model_calls_started=2,model_calls_completed=2))
        self.assertTrue(trial.hidden_gold_probe_passed(successful,wire,canary))
        for broken,requests in [({},[]),({'status':'failed','native':None},[]),
                                (successful,[]),
                                (dict(successful,status='failed'),wire),
                                (dict(successful,native=dict(status='error',run_error='authored failure')),wire),
                                (dict(successful,native=dict(status='completed',model_calls_started=0,model_calls_completed=0)),wire)]:
            with self.subTest(probe=broken,wire=requests):
                self.assertFalse(trial.hidden_gold_probe_passed(broken,requests,canary))
        self.assertFalse(trial.hidden_gold_probe_passed(successful,[{'messages':[{'content':canary}]}],canary))

    def test_controls_accept_omitted_false_and_reject_wrong_or_invalid_notice(self):
        with tempfile.TemporaryDirectory() as temporary:
            audit=Path(temporary)/'provider.jsonl'
            manifest=dict(budgets=trial.bridge.BUDGETS,model='synthetic',tools=trial.bridge.TOOLS)
            wire=dict(model='synthetic',temperature=0.6,seed=70400,max_tokens=4096,
                      chat_template_kwargs={'enable_thinking':False},
                      tools=[{'function':{'name':name}} for name in trial.bridge.TOOLS])
            trial.bridge.write_json(audit,dict(event='request_started',wire_body=wire,
                request=dict(local_only=True,think=False,think_budget=None)))
            for value,expected_arm,passed in [
                ('omitted',False,True),(False,False,True),(True,True,True),
                ('omitted',True,False),(False,True,False),(True,False,False),
                (None,False,False),(0,False,False),(1,True,False),('false',False,False)]:
                config=copy.deepcopy(trial.bridge.BUDGETS)
                if value!='omitted': config['budget_notice']=value
                native=dict(config=config,provider_audit=str(audit))
                with self.subTest(value=value,arm=expected_arm):
                    self.assertEqual(trial.controls_match(native,manifest,70400,expected_arm),passed)
            self.assertFalse(trial.controls_match(dict(config=None,provider_audit=str(audit)),manifest,70400,False))

    def test_failed_slots_stay_denominator(self):
        result=trial.summarize([dict(arm='notice',status='failed',score={})],[],{'scripted':True})
        self.assertEqual(result['arms']['notice']['denominator'],8)
        self.assertEqual(result['arms']['notice']['task_correct'],0)
        self.assertFalse(result['complete'])

if __name__=='__main__': unittest.main()
