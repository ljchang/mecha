#!/usr/bin/env python3
"""Frozen 48-first-response constraint diagnostic; real inference is parent-only.

Create three fresh authored-HTTP fixture lineages with --create-fixtures, then
pass their fixtures.json to --fixtures for the read-only comparison. No retries,
resume, model loads, or direct insertion of actual observations are supported.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import subprocess
import threading
from http.server import ThreadingHTTPServer
import outcome_reappraisal_trial as old
import durable_action_trial as action

encode, decode, hash_text = old.encode, old.decode, old.hash_text
sha, write_json, arm_request = old.sha, old.write_json, old.arm_request
ARMS, METHODS, WORKLOAD = old.ARMS, old.METHODS, old.WORKLOAD
SEEDS = (31, 43)
PROTOCOL = 'constraint-abstention-pilot-v1'
CORE_PROTOCOL = 'outcome-reappraisal-v2'
TREATMENTS = ('choice-first-fast', 'feasibility-first-fast', 'feasibility-first-thinking2048')
SIMULATOR = dict(Nori=(4,9,8), Arden=(8,3,9), Quill=(6,5,6))


def require(condition, message='invalid frozen diagnostic'):
    if not condition:
        raise ValueError(message)


def frozen_cases():
    ab = dict(Nori=(4,2,8), Arden=(8,8,9), Quill=(6,5,7))
    c = dict(Nori=(4,9,8), Arden=(8,3,9), Quill=(6,5,9))
    d = dict(Nori=(4,9,8), Arden=(8,3,8), Quill=(6,5,9))
    return [dict(id=n, store=s, source=source, actual_choice=choice, actual=SIMULATOR[choice],
        contract=dict(workload=WORKLOAD, objective='minimize_minutes', energy_budget=b, minimum_quality=q))
        for n,s,source,choice,b,q in [('A','AB',ab,'Arden',4,9),('B','AB',ab,'Arden',2,9),
                                      ('C','C',c,'Quill',5,8),('D','D',d,'Quill',5,9)]]


def controls(treatment):
    require(treatment in TREATMENTS)
    thinking = treatment == TREATMENTS[2]
    return dict(old.MODEL_CONTROLS, thinking=thinking, thinking_budget=2048 if thinking else None)


def response_format(treatment):
    return 'choice_first' if treatment == TREATMENTS[0] else 'feasibility_first'


def planned_pairs():
    plans=[]
    for j,seed in enumerate(SEEDS):
        for i,case in enumerate(frozen_cases()):
            rotated=TREATMENTS[(i+j)%3:]+TREATMENTS[:(i+j)%3]
            for k,treatment in enumerate(rotated):
                plans.append(dict(case=case['id'], seed=seed, treatment=treatment,
                                  outcome_first=bool((i+j+k)%2)))
    return plans


def effective(case, arm, simulator=False):
    require(arm in ARMS)
    rows=dict(SIMULATOR if simulator else case['source'])
    if not simulator and arm == ARMS[1]:
        rows[case['actual_choice']]=case['actual']
    return rows


def expected(case, arm, simulator=False):
    contract=case['contract']
    rows={n:v for n,v in effective(case,arm,simulator).items()
          if v[1]<=contract['energy_budget'] and v[2]>=contract['minimum_quality']}
    return {n for n,v in rows.items() if v[0]==min(x[0] for x in rows.values())} if rows else set()


def expected_checks(case, arm, request):
    result={}; contract=case['contract']
    for i,(name,(minutes,energy,quality)) in enumerate(effective(case,arm).items()):
        index=3 if arm==ARMS[1] and name==case['actual_choice'] else i
        energy_ok=energy<=contract['energy_budget']; quality_ok=quality>=contract['minimum_quality']
        result[name]=dict(minutes=minutes,energy=energy,quality=quality,
            source=request['passages'][index]['id'],energy_ok=energy_ok,quality_ok=quality_ok,
            feasible=energy_ok and quality_ok)
    return result


def checks_correct(case, arm, request, checks):
    if not isinstance(checks,dict) or set(checks)!=set(METHODS):return False
    expected_rows=expected_checks(case,arm,request)
    for name,wanted in expected_rows.items():
        row=checks[name]
        if not isinstance(row,dict) or set(row)!=set(wanted):return False
        if any(type(row[k]) is not int or not 0<=row[k]<=2**32-1 for k in ('minutes','energy','quality')):return False
        if any(type(row[k]) is not bool for k in ('energy_ok','quality_ok','feasible')):return False
        if row!=wanted:return False
    return True


def validate_preparation(prepared, case, fixture, treatment):
    try:
        require(prepared['protocol']==CORE_PROTOCOL and prepared['policy']==old.POLICY)
        require(prepared['action_id']==fixture['action'] and prepared['contract']==case['contract'])
        require(prepared['response_format']==response_format(treatment) and prepared['reason_max_chars']==4000)
        require(prepared['model_controls']==controls(treatment))
        require(prepared['private_data'] is True and prepared['untrusted_input'] is True and prepared['executed'] is False)
        require(set(prepared['snapshot'])=={'instance','sequence'})
        q=prepared['recall_query']
        require(q['known_at']==prepared['snapshot'])
        require((q['text'],q['limit'],q['neighbors'],q['max_bytes'])==(case['actual_choice'],100,0,100000))
        for field in ('read','observation'):
            require(hash_text(prepared[field]['text'])==prepared[field]['sha256'])
        source=decode(prepared['read']['text'])
        require(source==dict(kind='host_authored_synthetic_measurements_v1',workload=WORKLOAD,complete=True,
            observations=[dict(choice=n,observed_minutes=m,energy=e,quality=q)
                for n,(m,e,q) in case['source'].items()]))
        observation=decode(prepared['observation']['text']); actual=observation['observation']
        require(observation['kind']=='host_action_observation' and observation['action_id']==fixture['action']
                and observation['cancelled_action'] is False)
        require((actual['kind'],actual['fixture'],actual['state'],actual['choice'])==
                ('host_observed_synthetic_action_v1','measurements_v1','baseline',case['actual_choice']))
        m,e,q=case['actual']; require(actual['outcome']==dict(status='observed',observed_minutes=m,energy=e,quality=q))
        require(any(h['anchor']['reference']==prepared['observation']['source'] for h in prepared['raw_recall']))
        require(len(prepared['arms'])==2 and {a['arm'] for a in prepared['arms']}==set(ARMS))
        first=arm_request(prepared,ARMS[0]);second=arm_request(prepared,ARMS[1])
        require(len(first['passages'])==3 and len(second['passages'])==4 and first['passages']==second['passages'][:3])
        require(second['passages'][3]['source']==prepared['observation']['source'])
        for request in (first,second):
            require(type(request['prompt']) is str and bool(request['prompt']))
            ids=[p['id'] for p in request['passages']];require(len(set(ids))==len(ids))
            require(set(request['response_schema']['properties']['citations']['items']['enum'])==set(ids))
            require(request['response_schema']['properties']['reason']['maxLength']==4000)
            for i,passage in enumerate(request['passages']):
                require(type(passage['text']) is str and bool(passage['text']))
                require(hash_text(passage['text'])==passage['sha256'])
                projection=passage['projection']
                require(hash_text(projection['text'])==projection['sha256'])
                if i<3:
                    name=METHODS[i];m,e,q=case['source'][name]
                    require(passage['text']==prepared['read']['text'] and passage['source']==prepared['read']['observation'])
                    wanted=dict(workload=WORKLOAD,outcome=dict(choice=name,observed_minutes=m,energy=e,quality=q,kind='source_expectation'))
                else:
                    m,e,q=case['actual']
                    require(passage['text']==prepared['observation']['text'] and passage['source']==prepared['observation']['source'])
                    wanted=dict(workload=WORKLOAD,kind='actual_observation',choice=case['actual_choice'],
                        outcome=dict(status='observed',observed_minutes=m,energy=e,quality=q))
                require(decode(projection['text'])==wanted)
    except (KeyError,TypeError,AttributeError,StopIteration,ValueError) as error:
        raise ValueError('invalid frozen preparation') from error


def score_row(case,row,prepared,model,treatment):
    score=dict(valid=False,raw_choice=None,evidence_appropriate=False,required_abstention=False,
        unnecessary_abstention=False,model_abstained=False,check_correctness=None,citation_coverage=False,
        simulator_optimal=False,host_withheld=(row.get('decision') or {}).get('disposition')=='withheld')
    try:
        validate_preparation(prepared,case,dict(action=prepared['action_id']),treatment)
        request=arm_request(prepared,row['arm']);call=row['call'];reply=call['reply'];answer=decode(reply['text'])
        # Preserve raw declared choice even when checks or envelope fail.
        if isinstance(answer,dict):score['raw_choice']=answer.get('choice')
        ids={p['id'] for p in request['passages']}
        fields={'choice','citations','reason'} | ({'candidate_checks'} if response_format(treatment)=='feasibility_first' else set())
        valid=(not row.get('error') and not call.get('error') and not reply.get('error') and reply['model']==model
            and call['prompt']==request['prompt'] and call['response_schema']==request['response_schema']
            and isinstance(answer,dict) and set(answer)==fields and answer['choice'] in (*METHODS,'abstain')
            and isinstance(answer['reason'],str) and bool(answer['reason'].strip()) and len(answer['reason'])<=4000
            and isinstance(answer['citations'],list) and all(type(c) is str for c in answer['citations'])
            and len(set(answer['citations']))==len(answer['citations']) and set(answer['citations'])<=ids)
        if not valid:return score
        gold=expected(case,row['arm']);hidden=expected(case,row['arm'],True);abstain=answer['choice']=='abstain'
        covered=set(answer['citations'])==ids
        score.update(valid=True,model_abstained=abstain,citation_coverage=covered,
            evidence_appropriate=(answer['choice'] in gold or (abstain and not gold)),
            required_abstention=abstain and not gold, unnecessary_abstention=abstain and bool(gold),
            simulator_optimal=answer['choice'] in hidden or (abstain and not hidden))
        if 'candidate_checks' in answer:
            score['check_correctness']=checks_correct(case,row['arm'],request,answer['candidate_checks'])
    except (KeyError,TypeError,ValueError,StopIteration):pass
    return score


def validate_report(pair,report,case,fixture,model):
    p=pair['preparation'];t=pair['treatment'];validate_preparation(p,case,fixture,t)
    order=list(reversed(ARMS)) if pair['outcome_first'] else list(ARMS)
    require(report['preparation']==p and report['protocol']==CORE_PROTOCOL and report['model']==model)
    require(report['seed']==pair['seed'] and report['model_controls']==controls(t) and report['arm_order']==order)
    require(report['response_format']==response_format(t) and report['reason_max_chars']==4000)
    require(report['executed'] is False and report['private_data'] is True and report['untrusted_input'] is True)
    require(len(report['rows'])==2 and [r['arm'] for r in report['rows']]==order)


def summarize(pairs,model):
    lookup={(p['case'],p['seed'],p['treatment']):p for p in pairs};require(len(lookup)==len(pairs))
    plans={(p['case'],p['seed'],p['treatment']):p for p in planned_pairs()}
    for key,pair in lookup.items():
        require(key in plans and all(pair[k]==v for k,v in plans[key].items()))
        if pair.get('report'):
            case=next(c for c in frozen_cases() if c['id']==pair['case'])
            validate_report(pair,pair['report'],case,dict(action=pair['preparation']['action_id']),model)
    matrix=[]
    for plan in planned_pairs():
        pair=lookup.get((plan['case'],plan['seed'],plan['treatment']),{})
        case=next(c for c in frozen_cases() if c['id']==plan['case'])
        for arm in ARMS:
            row=next((r for r in (pair.get('report') or {}).get('rows',[]) if r['arm']==arm),None)
            matrix.append(dict(**plan,arm=arm,status='recorded' if row else 'dispatch_unknown' if pair.get('started') else 'not_started',
                score=score_row(case,row,pair['preparation'],model,plan['treatment']) if row else None,
                error=pair.get('error'),call=row.get('call') if row else None,
                reasoning_char_count=row.get('reasoning_char_count') if row else None))
    require(set(lookup)<= {(p['case'],p['seed'],p['treatment']) for p in planned_pairs()})
    recorded=sum(r['status']=='recorded' for r in matrix)
    return dict(protocol=PROTOCOL,planned_calls=48,calls_recorded=recorded,matrix_complete=recorded==48,
        unknown_dispatch_slots=sum(r['status']=='dispatch_unknown' for r in matrix),matrix=matrix,
        complete=recorded==48 and not any(p.get('error') for p in pairs),
        limitations='Four authored development cases, two seeds, no holdout or generalization claim. Raw choices, visible-evidence arithmetic, citation coverage, host withholding and hidden simulator outcomes are separate. Free-form explanation entailment requires independent review. Failed dispatch counts remain unknown; no repair or adaptive rerun. Private untrusted exports are outside local erasure.')


def isolated_env(home):
    home.mkdir(mode=0o700)
    (home/'config.toml').write_text('not = [valid toml\n')
    env=os.environ.copy()
    for key in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT'):env.pop(key,None)
    env.update(MECHA_HOME=str(home),MECHA_SESSION_KIND='test',MECHA_LOG='warn',RUST_LOG='off')
    return env



def helper_artifacts():
    return {module.__name__:dict(path=str(Path(module.__file__).resolve()),sha256=sha(Path(module.__file__)))
            for module in (old,action)}

def create_fixtures(binary, output):
    binary=binary.resolve(strict=True);output.mkdir(parents=True,mode=0o700,exist_ok=False);root=output.resolve()
    env=isolated_env(root/'isolated-home');holds=root/'holds';holds.mkdir()
    server=ThreadingHTTPServer(('127.0.0.1',0),action.ScriptedProvider)
    server.requests=[];server.choice='Arden'
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    endpoint=f'http://127.0.0.1:{server.server_port}'
    fixtures={}
    try:
        for case in (frozen_cases()[0],frozen_cases()[2],frozen_cases()[3]):
            store=case['store'];directory=root/store;directory.mkdir();db=directory/'constraint.memory-prototype.sqlite'
            def command(*argv):
                return old.invoke_cli(['memory','--experimental','--db',str(db),*argv],env=env,cwd=directory,binary=binary)
            def workflow(kind,*argv):
                return command(kind,'--synthetic-input','--domain','synthetic-demo','--scope','fixture-mail',*argv)
            command('init-action','--synthetic-input','--domain','synthetic-demo')
            source=dict(kind='host_authored_synthetic_measurements_v1',workload=WORKLOAD,complete=True,
                observations=[dict(choice=n,observed_minutes=m,energy=e,quality=q) for n,(m,e,q) in case['source'].items()])
            source_path=directory/'measurements.json';source_path.write_text(encode(source))
            provenance=directory/'provenance.jsonl';provenance.write_text(
                encode(dict(record='meta',id='constraint-diagnostic-'+store,created_at='2026-01-01T00:00:00Z'))+
                encode(dict(record='message',role='user',content=[dict(type='text',text='Synthetic '+case['actual_choice']+' constraint diagnostic prerequisite.')])) )
            imported=command('import-session',str(provenance),'--account','constraint-diagnostic','--synthetic-input')
            reference=imported['records'][-1]
            read=workflow('durable-read','start','--workspace',str(directory),'--file','measurements.json',
                '--source',reference['object']+':'+reference['revision'],'--max-bytes','4096','--max-lines','4')
            read_done=workflow('durable-read','resume','--task',read['status']['id'],'--workspace',str(directory))
            contract=dict(case['contract'],energy_budget=8 if store=='AB' else 5,minimum_quality=9)
            model=workflow('durable-decide','bind','--read',read['status']['id'],'--contract',encode(contract),
                           '--model',action.MODEL,'--view','observed')
            model_id=model['status']['id'];server.choice=case['actual_choice']
            model_done=workflow('durable-decide','run','--task',model_id,'--endpoint',endpoint,'--holds-dir',str(holds))
            require(model_done['decision']['disposition']=='recommend','fixture authored response rejected')
            bound=workflow('durable-act','bind','--model',model_id,'--spec',encode(dict(fixture='measurements_v1',state='baseline')))
            action_id=bound['status']['id'];result=workflow('durable-act','run','--task',action_id)
            require(result['status']['phase']=='finalized','fixture action not finalized')
            m,e,q=case['actual'];require(result['observation']['observation']['outcome']==
                dict(status='observed',observed_minutes=m,energy=e,quality=q),'unexpected authenticated actual')
            fixtures[store]=dict(db=str(db),action=action_id,source_path=str(source_path),source_sha256=sha(source_path),
                provenance=imported,read=read_done,model=model_done,action_report=result,database=old.database_state(db))
            write_json(directory/'lineage.json',fixtures[store])
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)
    manifest=dict(protocol=PROTOCOL,fixtures=fixtures,new_real_model_inference_calls=0,
        scripted_transport_requests=len(server.requests),binary_sha256=sha(binary),runner_sha256=sha(Path(__file__)),helper_artifacts=helper_artifacts(),
        cases=frozen_cases(),reply_source='authored loopback HTTP replies; actuals solely durable-act baseline handler')
    require(len(server.requests)==3,'fixture creation unexpectedly used revision/retry calls')
    write_json(root/'fixtures.json',manifest)
    return manifest


def validate_fixtures(manifest):
    require(manifest['protocol']==PROTOCOL and set(manifest['fixtures'])=={'AB','C','D'})
    require(manifest['new_real_model_inference_calls']==0 and manifest['scripted_transport_requests']==3)
    dbs=set();actions=set()
    for fixture in manifest['fixtures'].values():
        path=Path(fixture['db']).resolve(strict=True);require(path not in dbs);dbs.add(path)
        require(fixture['action'] not in actions);actions.add(fixture['action'])
        require(sha(Path(fixture['source_path']))==fixture['source_sha256'])
    return manifest['fixtures']


def run_trial(*,binary,fixtures,endpoint,model,holds_dir,output,invoke=old.invoke_cli,
              inspect_model=old.model_state,inspect_database=old.database_state):
    fixture_manifest=decode(fixtures.read_text()) if isinstance(fixtures,Path) else fixtures
    stores=validate_fixtures(fixture_manifest)
    binary=binary.resolve(strict=True);holds_dir=holds_dir.resolve(strict=True)
    output.mkdir(parents=True,mode=0o700,exist_ok=False);root=output.resolve();env=isolated_env(root/'isolated-home')
    manifest=dict(protocol=PROTOCOL,private_data=True,untrusted_input=True,executed=False,planned_calls=48,
        binary=str(binary),binary_sha256=sha(binary),runner_sha256=sha(Path(__file__)),helper_artifacts=helper_artifacts(),fixture_manifest=fixture_manifest,
        endpoint=endpoint,model=model,cases=frozen_cases(),seeds=list(SEEDS),treatments={t:controls(t) for t in TREATMENTS},
        offline_rubric=dict(simulator=SIMULATOR),pairs=[],error=None)
    pairs=manifest['pairs'];before={};state=None;error=None;interruption=None;tb=None
    write_json(root/'manifest.json',manifest)
    try:
        before={key:inspect_database(Path(f['db'])) for key,f in stores.items()};manifest['database_before']=before
        state=inspect_model(endpoint,model);manifest['resident_before']=state
        frozen_by_store={}
        for plan in planned_pairs():
            case=next(c for c in frozen_cases() if c['id']==plan['case']);fixture=stores[case['store']];t=plan['treatment']
            argv=['memory','--experimental','--db',fixture['db'],'evaluate-outcome-recall','--synthetic-input',
                '--domain','synthetic-demo','--scope','fixture-mail','--action',fixture['action'],
                '--contract',encode(case['contract']),'--query',case['actual_choice'],'--model',model,
                '--seed',str(plan['seed']),'--response-format',response_format(t).replace('_','-')]
            if controls(t)['thinking']:argv+=['--thinking-budget','2048']
            first=frozen_by_store.get(case['store'])
            if first is not None:argv+=['--snapshot',encode(first['snapshot'])]
            prepared=invoke(argv+['--prepare-only'],env=env,cwd=root,binary=binary)
            validate_preparation(prepared,case,fixture,t)
            if first is None:
                frozen_by_store[case['store']]=prepared;argv+=['--snapshot',encode(prepared['snapshot'])]
            else:
                require(all(prepared[k]==first[k] for k in ('snapshot','raw_recall','read','observation')),'source preparation drift')
            pairs.append(dict(**plan,argv=argv,preparation=prepared,report=None,error=None,started=False,
                request_hashes={a['arm']:hash_text(encode(a['request'])) for a in prepared['arms']}))
            write_json(root/'manifest.json',manifest)
        # All 48 slots and their exact preparations precede the first inference dispatch.
        write_json(root/'freeze.json',manifest);manifest['freeze_sha256']=sha(root/'freeze.json')
        write_json(root/'manifest.json',manifest)
        for pair in pairs:
            require({key:inspect_database(Path(f['db'])) for key,f in stores.items()}==before,'database drift before pair')
            require(old.settings_identity(inspect_model(endpoint,model))==old.settings_identity(state),'resident settings drift')
            pair['started']=True;write_json(root/'manifest.json',manifest)
            directory=root/f"{pair['case']}-{pair['seed']}-{pair['treatment']}";directory.mkdir()
            write_json(directory/'progress.json',dict(status='started',dispatch_count=None))
            try:
                argv=pair['argv']+['--endpoint',endpoint,'--holds-dir',str(holds_dir)]
                if pair['outcome_first']:argv+=['--outcome-first']
                report=invoke(argv,env=env,cwd=root,binary=binary)
                case=next(c for c in frozen_cases() if c['id']==pair['case'])
                validate_report(pair,report,case,stores[case['store']],model)
                pair['report']=report;write_json(directory/'report.json',report)
                write_json(directory/'progress.json',dict(status='recorded',reported_calls=2))
            except BaseException as failure:
                pair['error']=f'{type(failure).__name__}: {failure}'
                write_json(directory/'progress.json',dict(status='failed' if isinstance(failure,Exception) else 'interrupted',
                    error=pair['error'],dispatch_count=None));raise
            write_json(root/'manifest.json',manifest)
    except BaseException as failure:
        error=f'{type(failure).__name__}: {failure}'
        if not isinstance(failure,Exception):interruption=failure;tb=failure.__traceback__
    finally:
        for name,operation in (
            ('database_after',lambda:{key:inspect_database(Path(f['db'])) for key,f in stores.items()}),
            ('resident_after',lambda:inspect_model(endpoint,model))):
            try:manifest[name]=operation()
            except BaseException as failure:
                error=(error+'; ' if error else '')+f'{name} audit: {type(failure).__name__}: {failure}'
                if interruption is None and not isinstance(failure,Exception):interruption=failure;tb=failure.__traceback__
        manifest['database_unchanged']=bool(before) and manifest.get('database_after')==before
        manifest['settings_unchanged']=state is not None and 'resident_after' in manifest and old.settings_identity(manifest['resident_after'])==old.settings_identity(state)
        if not manifest['database_unchanged'] or not manifest['settings_unchanged']:
            error=(error+'; ' if error else '')+'database/settings post-audit mismatch'
        manifest.update(error=error,interrupted=interruption is not None);write_json(root/'manifest.json',manifest)
    summary=summarize(pairs,model);summary.update(error=error,interrupted=interruption is not None,
        database_unchanged=manifest['database_unchanged'],settings_unchanged=manifest['settings_unchanged'])
    summary['complete'] &= error is None and manifest['database_unchanged'] and manifest['settings_unchanged']
    write_json(root/'summary.json',summary)
    if interruption is not None:raise interruption.with_traceback(tb)
    return summary




def expected_http_request(request,treatment,seed,model):
    settings=controls(treatment)
    body=dict(model=model,max_tokens=settings['max_tokens'],temperature=settings['temperature'],seed=seed,
        messages=[dict(role='user',content=request['prompt'])],
        response_format=dict(type='json_object',schema=request['response_schema']),
        chat_template_kwargs=dict(enable_thinking=settings['thinking']))
    if settings['thinking_budget'] is not None:body['reasoning_budget_tokens']=settings['thinking_budget']
    return body


def audit_http_requests(expected,recorded):
    require(len(recorded)==len(expected),'HTTP dispatch count differs from frozen requests')
    for index,(wanted,actual) in enumerate(zip(expected,recorded)):
        require(all(actual.get(key)==value for key,value in wanted.items()),f'HTTP request drift at slot {index}')
        require(('reasoning_budget_tokens' in actual)==('reasoning_budget_tokens' in wanted),
                f'HTTP thinking-budget presence drift at slot {index}')
        require(not actual.get('tools') and not actual.get('stream'),f'HTTP request surface drift at slot {index}')

class QualificationProvider(action.ScriptedProvider):
    """Authored queue, never a model; request bytes remain available for audit."""
    def do_GET(self):
        if self.path.startswith('/props'):
            self.send_json(dict(model_path='authored-scripted-no-model',default_generation_settings={}))
        else:
            super().do_GET()

    def do_POST(self):
        if self.path != '/v1/chat/completions':
            self.send_error(404);return
        request=decode(self.rfile.read(int(self.headers['Content-Length'])).decode())
        self.server.requests.append(request)
        if not self.server.answers:
            self.send_error(500,'authored queue exhausted');return
        answer=self.server.answers.pop(0)
        self.send_json(dict(id='authored-constraint-qualification',object='chat.completion',model=action.MODEL,
            choices=[dict(index=0,message=dict(role='assistant',content=encode(answer)),finish_reason='stop')],
            usage=dict(prompt_tokens=0,completion_tokens=0,total_tokens=0)))


def qualify_scripted(binary, fixtures, output):
    """Exercise all48 actual CLI slots with preauthored replies and zero inference."""
    output.mkdir(parents=True,mode=0o700,exist_ok=False);root=output.resolve();holds=root/'holds';holds.mkdir()
    server=ThreadingHTTPServer(('127.0.0.1',0),QualificationProvider);server.requests=[];server.answers=[]
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    endpoint=f'http://127.0.0.1:{server.server_port}';preparations={};expected_requests=[]
    def invoke(argv,**kwargs):
        key=tuple(argv[:argv.index('--endpoint')]) if '--endpoint' in argv else tuple(argv[:-1])
        if '--prepare-only' in argv:
            prepared=old.invoke_cli(argv,**kwargs)
            # First preparation gains --snapshot after this return; identify by case contract/settings.
            key=(prepared['action_id'],encode(prepared['contract']),prepared['response_format'],
                prepared['model_controls']['thinking_budget'])
            preparations[key]=prepared;return prepared
        contract=decode(argv[argv.index('--contract')+1]);action_id=argv[argv.index('--action')+1]
        treatment=TREATMENTS[2] if '--thinking-budget' in argv else TREATMENTS[0] if argv[argv.index('--response-format')+1]=='choice-first' else TREATMENTS[1]
        prepared=preparations[(action_id,encode(contract),response_format(treatment),controls(treatment)['thinking_budget'])]
        case=next(c for c in frozen_cases() if c['contract']==contract)
        order=list(reversed(ARMS)) if '--outcome-first' in argv else list(ARMS)
        require(not server.answers,'unconsumed authored response')
        for arm in order:
            request=arm_request(prepared,arm)
            expected_requests.append(expected_http_request(request,treatment,int(argv[argv.index('--seed')+1]),action.MODEL))
            answer=dict(choice=next(iter(expected(case,arm)),'abstain'),
                citations=[p['id'] for p in request['passages']],reason='Authored qualification response; no model inference.')
            if response_format(treatment)=='feasibility_first':answer['candidate_checks']=expected_checks(case,arm,request)
            server.answers.append(answer)
        report=old.invoke_cli(argv,**kwargs);require(not server.answers,'CLI did not consume both authored replies')
        return report
    try:
        summary=run_trial(binary=binary,fixtures=fixtures,endpoint=endpoint,model=action.MODEL,
            holds_dir=holds,output=root/'trial',invoke=invoke)
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)
        write_json(root/'http-requests.json',server.requests)
        write_json(root/'expected-http-requests.json',expected_requests)
    audit_http_requests(expected_requests,server.requests)
    require(summary['complete'] and len(server.requests)==48,'scripted CLI qualification incomplete')
    require(all(r['score']['valid'] and r['score']['evidence_appropriate'] and r['score']['citation_coverage']
                and r['score']['check_correctness'] is not False and not r['score']['host_withheld'] for r in summary['matrix']),
            'scripted CLI qualification failed independent rubric')
    result=dict(complete=True,new_real_model_inference_calls=0,scripted_transport_requests=48,
        http_request_audit=True,recorded_http_sha256=sha(root/'http-requests.json'),
        expected_http_sha256=sha(root/'expected-http-requests.json'),helper_artifacts=helper_artifacts(),summary=summary)
    write_json(root/'qualification.json',result);return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--create-fixtures',action='store_true')
    parser.add_argument('--qualify-scripted',action='store_true')
    for name in ('binary','output'):parser.add_argument('--'+name,type=Path,required=True)
    for name in ('fixtures','holds-dir'):parser.add_argument('--'+name,type=Path)
    for name in ('endpoint','model'):parser.add_argument('--'+name)
    args=vars(parser.parse_args());create=args.pop('create_fixtures');qualify=args.pop('qualify_scripted')
    require(not(create and qualify),'choose a single mode')
    if create:
        result=create_fixtures(args['binary'],args['output'])
    elif qualify:
        require(args['fixtures'] is not None,'qualification requires fixtures')
        result=qualify_scripted(args['binary'],args['fixtures'],args['output'])
    else:
        require(all(args[k] is not None for k in ('fixtures','holds_dir','endpoint','model')),'runtime requires fixtures, holds-dir, endpoint and model')
        result=run_trial(**args)
    print(json.dumps(result,indent=2))
    if result.get('complete') is False:raise SystemExit(1)


if __name__=='__main__':main()
