import unittest
import constraint_abstention_trial as t

class RubricTests(unittest.TestCase):
    def test_frozen_targets_and_matrix(self):
        self.assertEqual(len(t.planned_pairs()), 12)
        self.assertEqual(t.SEEDS, (59,))
        self.assertEqual(t.PROTOCOL,'constraint-abstention-pilot-v2')
        self.assertEqual(t.CORE_PROTOCOL,'outcome-reappraisal-v3')
        self.assertEqual(t.PLANNED_CALLS,24)
        self.assertEqual([(c['id'],c['contract']['energy_budget'],c['contract']['minimum_quality']) for c in t.frozen_cases()],
                         [('E',3,9),('F',1,9),('G',6,8),('H',6,9)])
        self.assertEqual([[t.expected(c,a) for a in t.ARMS] for c in t.frozen_cases()],
                         [[set(),{'Arden'}],[set(),set()],[{'Quill'},{'Arden'}],[{'Quill'},set()]])
    def test_boundary_ties_and_empty(self):
        c=t.frozen_cases()[0].copy(); c['source']={n:(4,3,9) for n in t.METHODS}
        self.assertEqual(t.expected(c,'source_only'),set(t.METHODS))
        c['contract']=dict(c['contract'],energy_budget=2)
        self.assertEqual(t.expected(c,'source_only'),set())
    def test_checks_reject_wrong_arithmetic_foreign_sources_and_bool_numbers(self):
        c=t.frozen_cases()[0]; p={'passages':[{'id':f'e{i+1}'} for i in range(4)]}
        checks=t.expected_checks(c,'source_and_outcome',p)
        self.assertTrue(t.checks_correct(c,'source_and_outcome',p,checks))
        for field,value in [('energy',8),('source','foreign'),('minutes',True),('feasible',False)]:
            import copy
            bad=copy.deepcopy(checks);bad['Arden'][field]=value
            self.assertFalse(t.checks_correct(c,'source_and_outcome',p,bad))


import copy
import json
import os
from pathlib import Path
import tempfile


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
        request=dict(prompt='frozen prompt '+arm,response_schema=dict(properties=dict(([('candidate_checks',{})] if t.response_format(treatment)=='feasibility_first' else []) + [('choice',{}),('citations',dict(items=dict(enum=[p['id'] for p in visible]))),('reason',dict(maxLength=4000))])),passages=visible)
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
        request=t.arm_request(p,arm);answer=dict(choice=next(iter(t.expected(case,arm)),'abstain'),
            citations=[x['id'] for x in request['passages']],reason='Authored comparison.')
        if t.response_format(treatment)=='feasibility_first':answer['candidate_checks']=t.expected_checks(case,arm,request)
        rows.append(dict(arm=arm,error=None,decision=dict(disposition='withheld'),reasoning_char_count=0,
            call=dict(prompt=request['prompt'],response_schema=request['response_schema'],error=None,
                reply=dict(text=json.dumps(answer),model=model,error=None,input_tokens=3,output_tokens=4),millis=1)))
    return dict(protocol=t.CORE_PROTOCOL,preparation=p,model=model,seed=pair['seed'],model_controls=t.controls(treatment),
        response_format=t.response_format(treatment),reason_max_chars=4000,arm_order=order,rows=rows,
        executed=False,private_data=True,untrusted_input=True)


class ScoringTests(unittest.TestCase):
    def test_raw_choice_host_withhold_checks_and_citations_are_independent(self):
        case=t.frozen_cases()[0];treatment=t.TREATMENTS[1];p=preparation(case,treatment)
        pair=dict(t.planned_pairs()[1],preparation=p);pair['treatment']=treatment
        row=next(r for r in report(pair)['rows'] if r['arm']==t.ARMS[1])
        score=t.score_row(case,row,p,'scripted',treatment)
        self.assertTrue(score['evidence_appropriate']);self.assertTrue(score['host_withheld']);self.assertTrue(score['check_correctness'])
        self.assertEqual(score['raw_response_field_order'],['choice','citations','reason','candidate_checks'])
        self.assertFalse(score['response_order_matches_treatment'])
        answer=json.loads(row['call']['reply']['text']);answer['candidate_checks']['Arden']['energy']=8
        row['call']['reply']['text']=json.dumps(answer);score=t.score_row(case,row,p,'scripted',treatment)
        self.assertEqual(score['raw_choice'],'Arden');self.assertTrue(score['evidence_appropriate']);self.assertFalse(score['check_correctness'])
        answer['choice']='abstain';answer['citations']=[];row['call']['reply']['text']=json.dumps(answer)
        score=t.score_row(case,row,p,'scripted',treatment)
        self.assertTrue(score['unnecessary_abstention']);self.assertFalse(score['citation_coverage'])
    def test_malformed_duplicate_wrong_model_and_reason_bound(self):
        c=t.frozen_cases()[0];p=preparation(c,t.TREATMENTS[0]);pair=dict(t.planned_pairs()[0],preparation=p)
        good=report(pair)['rows'][0]
        for text in ('null','[]','{"choice":"Nori","choice":"Arden"}',json.dumps(dict(choice='abstain',citations=[],reason='x'*4001))):
            row=copy.deepcopy(good);row['call']['reply']['text']=text
            self.assertFalse(t.score_row(c,row,p,'scripted',t.TREATMENTS[0])['valid'])
        row=copy.deepcopy(good);row['call']['reply']['model']='wrong'
        self.assertFalse(t.score_row(c,row,p,'scripted',t.TREATMENTS[0])['valid'])
    def test_reason_limit_counts_raw_unicode_characters(self):
        case=t.frozen_cases()[0];treatment=t.TREATMENTS[0];p=preparation(case,treatment)
        row=report(dict(t.planned_pairs()[0],preparation=p))['rows'][0]
        answer=json.loads(row['call']['reply']['text'])
        for reason,valid in [(' '*4001+'x',False),('x'+' '*4000,False),(' '*4000,False),('🌱'*4000,True)]:
            answer['reason']=reason;row['call']['reply']['text']=json.dumps(answer)
            self.assertEqual(t.score_row(case,row,p,'scripted',treatment)['valid'],valid)

    def test_old_choice_first_schema_rejected_for_feasibility_treatment(self):
        case=t.frozen_cases()[0];treatment=t.TREATMENTS[1];p=preparation(case,treatment)
        for arm in p['arms']:
            props=arm['request']['response_schema']['properties']
            arm['request']['response_schema']['properties']={k:props[k] for k in ('choice','citations','reason','candidate_checks')}
        with self.assertRaises(ValueError):t.validate_preparation(p,case,{'action':'action'},treatment)
        expected=t.expected_http_request(preparation(case,treatment)['arms'][0]['request'],treatment,59,'scripted')
        bad=copy.deepcopy(expected);props=bad['response_format']['schema']['properties']
        bad['response_format']['schema']['properties']={k:props[k] for k in ('choice','citations','reason','candidate_checks')}
        with self.assertRaises(ValueError):t.audit_http_requests([expected],[bad])

    def test_http_audit_checks_dispatch_order_and_all_frozen_controls(self):
        expected=[]
        for pair_plan in t.planned_pairs()[:3]:
            case=t.frozen_cases()[0];p=preparation(case,pair_plan['treatment'])
            for arm in (list(reversed(t.ARMS)) if pair_plan['outcome_first'] else t.ARMS):
                expected.append(t.expected_http_request(t.arm_request(p,arm),pair_plan['treatment'],pair_plan['seed'],'scripted'))
        t.audit_http_requests(expected,copy.deepcopy(expected))
        for mutate in (lambda x:x.__setitem__(slice(0,2),list(reversed(x[:2]))),lambda x:x[0].update(seed=99),lambda x:x[0].update(max_tokens=4096),
                       lambda x:x[0].update(reasoning_budget_tokens=2048),
                       lambda x:x[0].update(chat_template_kwargs={'enable_thinking':True}),
                       lambda x:x[0]['messages'][0].update(content='rewritten'),
                       lambda x:x[0]['response_format'].update(schema={})):
            bad=copy.deepcopy(expected);mutate(bad)
            with self.assertRaises(ValueError):t.audit_http_requests(expected,bad)

    def test_preparation_rejects_metadata_and_projection_drift(self):
        c=t.frozen_cases()[0];treatment=t.TREATMENTS[0];p=preparation(c,treatment)
        t.validate_preparation(p,c,dict(action='action'),treatment)
        for mutate in (lambda x:x.update(protocol='wrong'),lambda x:x.update(reason_max_chars=1000),
                       lambda x:x['model_controls'].update(thinking=True),
                       lambda x:x['arms'][0]['request']['passages'][0]['projection'].update(text='forged'),
                       lambda x:x['arms'][0]['request']['passages'][0].update(source={'object':'foreign','revision':'foreign'}),
                       lambda x:[a['request']['passages'][0]['projection'].update(text='forged') for a in x['arms']]):
            bad=copy.deepcopy(p);mutate(bad)
            with self.assertRaises(ValueError):t.validate_preparation(bad,c,dict(action='action'),treatment)



class QualificationHttpTests(unittest.TestCase):
    def test_authored_providers_accept_no_autoload_completion_query(self):
        import threading
        import urllib.request
        from http.server import ThreadingHTTPServer
        import durable_action_trial as action
        import memory_benefit_trial as benefit
        import test_outcome_reappraisal_trial as outcome
        for provider in (action.ScriptedProvider, t.QualificationProvider,
                         benefit.QualificationProvider, outcome.ScriptedProvider):
            with self.subTest(provider=provider):
                server=ThreadingHTTPServer(('127.0.0.1',0),provider)
                server.requests=[];server.raw_requests=[];server.schema_orders=[]
                server.choice='Arden';server.answers=[dict(choice='Arden')]
                worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
                try:
                    state=outcome.trial.frozen_cases()[0]['contract']
                    body=dict(messages=[dict(role='user',content='Current state: '+json.dumps(state)+'\nAvailable methods: authored')],
                              response_format=dict(schema=dict(properties=dict(choice=dict(type='string')))))
                    request=urllib.request.Request(
                        f'http://127.0.0.1:{server.server_port}/v1/chat/completions?autoload=false',
                        data=json.dumps(body).encode(),headers={'Content-Type':'application/json'})
                    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with opener.open(request,timeout=5) as response:reply=json.load(response)
                    self.assertIn('choice',json.loads(reply['choices'][0]['message']['content']))
                    self.assertEqual(server.requests,[body])
                finally:
                    server.shutdown();server.server_close();worker.join(timeout=5)

    def test_failed_returned_report_is_retained_before_queue_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);returned=dict(rows=[dict(call=dict(error='router discovery failed'))])
            with self.assertRaisesRegex(ValueError,'failed-pair-report.json'):
                t.check_authored_queue(returned,[{'choice':'Arden'}],root)
            self.assertEqual(t.decode((root/'failed-pair-report.json').read_text()),returned)
            t.check_authored_queue(returned,[],root)

    def test_router_discovery_and_model_specific_props_over_real_http(self):
        import threading
        import urllib.request
        from http.server import ThreadingHTTPServer
        server=ThreadingHTTPServer(('127.0.0.1',0),t.QualificationProvider)
        worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
        try:
            opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
            endpoint=f'http://127.0.0.1:{server.server_port}'
            with opener.open(endpoint+'/props',timeout=5) as response:bare=json.load(response)
            self.assertEqual(bare.get('role'),'router')
            with opener.open(endpoint+'/props?model=synthetic&autoload=false',timeout=5) as response:model=json.load(response)
            self.assertEqual(model['default_generation_settings'],{})
            self.assertEqual(model['model_path'],'authored-scripted-no-model')
        finally:
            server.shutdown();server.server_close();worker.join(timeout=5)

class RunnerTests(unittest.TestCase):
    def run_fake(self,root,interrupt=False,drift=False):
        binary=root/'binary';binary.write_text('binary');holds=root/'holds';holds.mkdir();fixtures={}
        for store in ('AB','C','D'):
            db=root/(store+'.db');db.write_text('sentinel');source=root/(store+'.json');source.write_text(store)
            fixtures[store]=dict(db=str(db),action='action-'+store,source_path=str(source),source_sha256=t.sha(source))
        manifest=dict(protocol='constraint-abstention-pilot-v1',fixtures=fixtures,new_real_model_inference_calls=0,scripted_transport_requests=3)
        calls=[]
        def invoke(argv,**kwargs):
            calls.append(argv);contract=t.decode(argv[argv.index('--contract')+1]);case=next(c for c in t.frozen_cases() if c['contract']==contract)
            treatment=t.TREATMENTS[2] if '--thinking-budget' in argv else t.TREATMENTS[0] if argv[argv.index('--response-format')+1]=='choice-first' else t.TREATMENTS[1]
            p=preparation(case,treatment,fixtures[case['store']]['action'])
            if '--prepare-only' in argv:return p
            if interrupt:raise KeyboardInterrupt('operator')
            pair=dict(case=case['id'],seed=int(argv[argv.index('--seed')+1]),treatment=treatment,
                outcome_first='--outcome-first' in argv,preparation=p)
            result=report(pair)
            if drift:result['model_controls']['temperature']=1
            return result
        try:
            summary=t.run_trial(binary=binary,fixtures=manifest,endpoint='http://127.0.0.1:1',model='scripted',holds_dir=holds,
                output=root/'output',invoke=invoke,inspect_database=lambda p:dict(bytes=p.read_text()),
                inspect_model=lambda *_:dict(props=dict(model_path='frozen')))
        except KeyboardInterrupt:
            summary=t.decode((root/'output'/'summary.json').read_text())
            self.assertTrue(summary['interrupted'])
        return summary,calls
    def test_full_frozen_matrix_and_interruption(self):
        for interrupt in (False,True):
            with self.subTest(interrupt=interrupt),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);summary,calls=self.run_fake(root,interrupt)
                self.assertEqual(sum('--prepare-only' in c for c in calls),12)
                self.assertEqual(len(summary['matrix']),24)
                self.assertEqual(summary['planned_calls'],24)
                self.assertEqual(summary['calls_recorded'],0 if interrupt else 24)
                self.assertEqual(summary['unknown_dispatch_slots'],2 if interrupt else 0)
                self.assertEqual(summary['complete'],not interrupt)
                freeze=t.decode((root/'output'/'freeze.json').read_text())
                self.assertTrue(all(not p['started'] and p['report'] is None for p in freeze['pairs']))
                for pair in freeze['pairs']:
                    self.assertEqual(pair['schema_property_order'],{arm:t.schema_order(pair['treatment']) for arm in t.ARMS})
                self.assertEqual(set(freeze['helper_artifacts']),{'outcome_reappraisal_trial','durable_action_trial'})
                for artifact in freeze['helper_artifacts'].values():self.assertEqual(artifact['sha256'],t.sha(Path(artifact['path'])))
                self.assertTrue(summary['database_unchanged']);self.assertTrue(summary['settings_unchanged'])
    def test_metadata_drift_stops_without_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            summary,calls=self.run_fake(Path(directory),drift=True)
            self.assertFalse(summary['complete']);self.assertEqual(len(calls),13)
            self.assertEqual(summary['unknown_dispatch_slots'],2)

if __name__=='__main__':unittest.main()
