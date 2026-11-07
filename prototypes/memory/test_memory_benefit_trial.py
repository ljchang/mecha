import copy
import json
import tempfile
import unittest
from pathlib import Path
import memory_benefit_trial as t

def preparation(case,treatment,action='action'):
    read=t.encode(dict(kind='host_authored_synthetic_measurements_v1',workload=t.WORKLOAD,complete=True,
        observations=[dict(choice=n,observed_minutes=m,energy=e,quality=q) for n,(m,e,q) in case['source'].items()]))
    actual=t.encode(dict(kind='host_action_observation',action_id=action,cancelled_action=False,
        observation=dict(kind='host_observed_synthetic_action_v1',fixture='measurements_v1',state='baseline',
            choice=case['actual_choice'],outcome=dict(status='observed',**dict(zip(('observed_minutes','energy','quality'),case['actual']))))))
    passages=[]
    for i,(n,(m,e,q)) in enumerate(case['source'].items()):
        text=t.encode(dict(workload=t.WORKLOAD,outcome=dict(choice=n,observed_minutes=m,energy=e,quality=q,kind='source_expectation')))
        passages.append(dict(id=f'e{i+1}',text=read,sha256=t.hash_text(read),source={'object':'read','revision':'read'},
            projection=dict(text=text,sha256=t.hash_text(text))))
    arms=[]
    for arm in t.ARMS:
        visible=copy.deepcopy(passages)
        if arm==t.ARMS[1]:
            text=t.encode(dict(workload=t.WORKLOAD,kind='actual_observation',choice=case['actual_choice'],
                outcome=dict(status='observed',**dict(zip(('observed_minutes','energy','quality'),case['actual'])))))
            visible.append(dict(id='e4',text=actual,sha256=t.hash_text(actual),source={'object':'actual','revision':'actual'},
                projection=dict(text=text,sha256=t.hash_text(text))))
        request=dict(prompt='frozen prompt '+arm,response_schema=dict(properties=dict(([('candidate_checks',{})] if t.response_format(treatment)=='feasibility_provenance' else []) + [('choice',{}),('citations',dict(items=dict(enum=[p['id'] for p in visible]))),('reason',dict(maxLength=4000))])),passages=visible)
        arms.append(dict(arm=arm,request=request,request_sha256='core hash'))
    snapshot=dict(instance='instance',sequence=1)
    return dict(protocol=t.CORE_PROTOCOL,policy=t.old.POLICY,action_id=action,contract=case['contract'],snapshot=snapshot,
        response_format=t.response_format(treatment),reason_max_chars=4000,model_controls=t.controls(treatment),
        private_data=True,untrusted_input=True,executed=False,
        recall_query=dict(text=case['actual_choice'],known_at=snapshot,limit=100,neighbors=0,max_bytes=100000),
        raw_recall=[dict(anchor=dict(reference={'object':'actual','revision':'actual'}))],
        read=dict(text=read,sha256=t.hash_text(read),observation={'object':'read','revision':'read'}),
        observation=dict(text=actual,sha256=t.hash_text(actual),source={'object':'actual','revision':'actual'}),arms=arms)


def report(pair,model='scripted'):
    p=pair['preparation'];case=next(c for c in t.frozen_cases() if c['id']==pair['case']);treatment=pair['treatment']
    order=list(reversed(t.ARMS)) if pair['outcome_first'] else list(t.ARMS);rows=[]
    for arm in order:
        request=t.arm_request(p,arm);answer=dict(choice=next(iter(sorted(t.expected(case,arm))),'abstain'),
            citations=[x['id'] for x in request['passages']],reason='Authored comparison.')
        answer=dict(candidate_checks=t.expected_checks(case,arm,request),**answer)
        rows.append(dict(arm=arm,error=None,decision=dict(disposition='withheld'),reasoning_char_count=0,
            call=dict(prompt=request['prompt'],response_schema=request['response_schema'],error=None,
                reply=dict(text=json.dumps(answer),model=model,error=None,input_tokens=3,output_tokens=4),millis=1)))
    return dict(protocol=t.CORE_PROTOCOL,preparation=p,model=model,seed=pair['seed'],model_controls=t.controls(treatment),
        response_format=t.response_format(treatment),reason_max_chars=4000,arm_order=order,rows=rows,
        executed=False,private_data=True,untrusted_input=True,citation_policy='effective-measurements-v1')



class Oracles(unittest.TestCase):
    def test_exact_oracles(self):
        self.assertEqual([[t.expected(c,a) for a in t.ARMS] for c in t.frozen_cases()],
            [[set(),{'Arden'}],[{'Nori'},{'Quill'}],[{'Quill'},{'Arden'}],
             [{'Quill'},set()],[{'Nori'},{'Nori'}],[{'Nori','Arden'},{'Arden'}]])
        for c in t.frozen_cases():
            self.assertEqual(t.expected(c,t.ARMS[0],True),t.expected(c,t.ARMS[1],True))
            t.prerequisite_contract(c)
        c=t.frozen_cases()[1]
        self.assertEqual(t.expected(c,t.ARMS[1]),{'Quill'})
        self.assertEqual(t.expected(c,t.ARMS[1],True),{'Arden'})
    def test_schedule(self):
        plans=t.planned_pairs()
        self.assertEqual(len(plans),24)
        self.assertEqual(len({(p['case'],p['seed'],p['treatment']) for p in plans}),24)
        self.assertEqual(sum(p['outcome_first'] for p in plans),12)
        for treatment in t.TREATMENTS:
            self.assertEqual(sum(p['outcome_first'] for p in plans if p['treatment']==treatment),6)
        for case in t.frozen_cases():
            rows=[p for p in plans if p['case']==case['id']]
            self.assertEqual(sum(p['outcome_first'] for p in rows),2)
            self.assertNotEqual(rows[0]['treatment'],rows[2]['treatment'])
        self.assertEqual(t.summarize([], 'scripted')['unknown_dispatch_slots'],0)
        summary=t.summarize([], 'scripted')
        self.assertEqual(len(summary['matrix']),48)
        self.assertFalse(summary['complete'])
        self.assertEqual(summary['metrics'][t.ARMS[0]]['denominator'],24)
        self.assertEqual(summary['metrics']['paired_outcome_informed_choice']['unknown'],24)

class Scoring(unittest.TestCase):
    def row(self,index=0,arm=t.ARMS[1]):
        c=t.frozen_cases()[index];plan=next(p for p in t.planned_pairs() if p['case']==c['id'])
        p=preparation(c,plan['treatment']);pair=dict(plan,preparation=p)
        row=next(r for r in report(pair)['rows'] if r['arm']==arm)
        return c,plan,p,row
    def score(self,c,plan,p,row):return t.score_row(c,row,p,'scripted',plan['treatment'])
    def test_effective_citations_and_independent_checks(self):
        c,plan,p,row=self.row();answer=t.decode(row['call']['reply']['text'])
        req=t.arm_request(p,row['arm'])
        answer['citations']=sorted(t.effective_citations(c,row['arm'],req))
        row['call']['reply']['text']=t.encode(answer)
        self.assertTrue(self.score(c,plan,p,row)['citation_coverage'])
        self.assertEqual(len(answer['citations']),3)
        answer['citations'].remove('e4');row['call']['reply']['text']=t.encode(answer)
        score=self.score(c,plan,p,row)
        self.assertFalse(score['citation_coverage']);self.assertTrue(score['evidence_appropriate'])
        answer['candidate_checks']['Arden']['energy']=True
        row['call']['reply']['text']=t.encode(answer)
        self.assertFalse(self.score(c,plan,p,row)['check_correctness'])
    def test_tie_options_are_both_correct(self):
        c,plan,p,row=self.row(5,t.ARMS[0]);answer=t.decode(row['call']['reply']['text'])
        for choice in ('Nori','Arden'):
            answer['choice']=choice;row['call']['reply']['text']=t.encode(answer)
            self.assertTrue(self.score(c,plan,p,row)['evidence_appropriate'])
    def test_abstention_can_be_evidence_correct_and_lose_shared_reference(self):
        c,plan,p,row=self.row(0,t.ARMS[0]);score=self.score(c,plan,p,row)
        self.assertTrue(score['evidence_appropriate']);self.assertTrue(score['required_abstention'])
        self.assertFalse(score['outcome_informed_optimal'])
        self.assertTrue(score['host_withheld'])
    def test_malformed_and_false_checks(self):
        c,plan,p,row=self.row();good=copy.deepcopy(row)
        for text in ('null','[]','{"choice":"Nori","choice":"Arden"}'):
            row=copy.deepcopy(good);row['call']['reply']['text']=text
            self.assertFalse(self.score(c,plan,p,row)['valid'])
        for field,value in [('source','foreign'),('energy_ok',False),('feasible',False),('energy',True)]:
            row=copy.deepcopy(good);a=t.decode(row['call']['reply']['text']);a['candidate_checks']['Arden'][field]=value
            row['call']['reply']['text']=t.encode(a)
            self.assertFalse(self.score(c,plan,p,row)['check_correctness'])
        row=copy.deepcopy(good);a=t.decode(row['call']['reply']['text']);a['reason']='x'*4001
        row['call']['reply']['text']=t.encode(a);self.assertFalse(self.score(c,plan,p,row)['valid'])
    def test_http_audit(self):
        c,plan,p,row=self.row();req=t.expected_http_request(t.arm_request(p,row['arm']),plan['treatment'],71,'scripted')
        t.audit_http_requests([req],copy.deepcopy([req]))
        for key,value in [('seed',1),('max_tokens',1),('reasoning_budget_tokens',1)]:
            bad=copy.deepcopy(req);bad[key]=value
            with self.assertRaises(ValueError):t.audit_http_requests([req],[bad])

class Runner(unittest.TestCase):
    def run_fake(self,root,fault=None):
        binary=root/'binary';binary.write_text('binary');holds=root/'holds';holds.mkdir();fixtures={}
        state=lambda p:dict(bytes=p.read_text())
        for store in {c['store'] for c in t.frozen_cases()}:
            db=root/(store+'.db');db.write_text('sentinel');source=root/(store+'.json');source.write_text(store)
            fixtures[store]=dict(db=str(db),action='action-'+store,source_path=str(source),source_sha256=t.sha(source),database=state(db))
        manifest=dict(protocol=t.PROTOCOL,fixtures=fixtures,new_real_model_inference_calls=0,scripted_transport_requests=5,
            binary_sha256=t.sha(binary),runner_sha256=t.sha(Path(t.__file__)),helper_artifacts=t.helper_artifacts())
        calls=[]
        def invoke(argv,**kwargs):
            calls.append(argv);contract=t.decode(argv[argv.index('--contract')+1]);action_id=argv[argv.index('--action')+1]
            case=next(c for c in t.frozen_cases() if c['contract']==contract and fixtures[c['store']]['action']==action_id)
            treatment='thinking2048' if '--thinking-budget' in argv else 'fast'
            p=preparation(case,treatment,action_id)
            if '--prepare-only' in argv:return p
            if fault=='interrupt':raise KeyboardInterrupt('operator')
            pair=dict(case=case['id'],seed=int(argv[argv.index('--seed')+1]),treatment=treatment,
                outcome_first='--outcome-first' in argv,preparation=p)
            result=report(pair)
            if fault=='metadata':result['seed']=999
            if fault=='source':Path(fixtures[case['store']]['source_path']).write_text('drift')
            if fault=='partial':result['rows']=result['rows'][:1]
            return result
        try:
            summary=t.run_trial(binary=binary,fixtures=manifest,endpoint='http://127.0.0.1:1',model='scripted',holds_dir=holds,
                output=root/'output',invoke=invoke,inspect_database=state,
                inspect_model=lambda *_:dict(props=dict(model_path='frozen')))
        except KeyboardInterrupt:
            summary=t.decode((root/'output'/'summary.json').read_text())
        return summary,calls
    def test_initial_transport_output_retained_before_decoder_and_revocation_guard(self):
        from unittest.mock import patch
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            artifact=Path(directory)/'raw.json'
            with patch.object(t.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'malformed initial reply','warning')):
                with self.assertRaises(ValueError):
                    t.invoke_captured([],env={},cwd=Path(directory),binary=Path('/fake'),artifact=artifact)
            self.assertEqual(t.decode(artifact.read_text())['stdout'],'malformed initial reply')
            with patch.object(t.subprocess,'run',return_value=subprocess.CompletedProcess([],1,'revoked content','revoked content')):
                with self.assertRaises(RuntimeError):
                    t.invoke_captured([],env={},cwd=Path(directory),binary=Path('/fake'),artifact=artifact)
            saved=t.decode(artifact.read_text())
            self.assertEqual(saved['returncode'],1)
            self.assertNotIn('stdout',saved)
            self.assertNotIn('stderr',saved)

    def test_full_freeze_before_calls_and_paired_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);summary,calls=self.run_fake(root)
            self.assertTrue(summary['complete']);self.assertEqual(summary['calls_recorded'],48)
            self.assertTrue(all('--prepare-only' in c for c in calls[:24]))
            self.assertEqual(len(calls),48)
            freeze=t.decode((root/'output'/'freeze.json').read_text())
            self.assertTrue(all(not p['started'] and p['report'] is None for p in freeze['pairs']))
            counts=summary['metrics']['paired_outcome_informed_choice']
            self.assertEqual(counts['improvement'],16);self.assertEqual(counts['unchanged_correct'],8)
            with self.assertRaises(FileExistsError):self.run_fake_existing(root)
    def run_fake_existing(self,root):
        t.run_trial(binary=root/'binary',fixtures=t.decode((root/'output'/'manifest.json').read_text())['fixture_manifest'],
            endpoint='x',model='scripted',holds_dir=root/'holds',output=root/'output')
    def test_failures_preserved_never_retry_and_full_denominator(self):
        for fault,recorded,unknown in [('interrupt',0,2),('metadata',2,0),('partial',1,1),('source',2,0)]:
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);summary,calls=self.run_fake(root,fault)
                self.assertFalse(summary['complete']);self.assertEqual(len(summary['matrix']),48)
                self.assertEqual(summary['calls_recorded'],recorded);self.assertEqual(summary['unknown_dispatch_slots'],unknown)
                self.assertEqual(len(calls),25)
                if fault=='metadata':
                    recorded_rows=[r for r in summary['matrix'] if r['status']=='recorded']
                    self.assertTrue(all(r['audit_error'] and r['qualified_score'] is None and r['score'] for r in recorded_rows))
                    for arm in t.ARMS:
                        self.assertEqual(summary['metrics'][arm]['valid'],0)
                        self.assertEqual(summary['metrics'][arm]['rubric_pass'],0)
                        self.assertEqual(summary['metrics'][arm]['decision_joint_success'],0)
                    self.assertEqual(summary['metrics']['paired_outcome_informed_choice']['unknown'],24)
                if fault!='interrupt':
                    reports=list((root/'output').glob('*/report.json'));self.assertEqual(len(reports),1)
                    self.assertTrue(t.decode(reports[0].read_text())['rows'])

if __name__=='__main__':unittest.main()
