#!/usr/bin/env python3
"""Frozen 48-first-response memory-benefit diagnostic; real inference is parent-only.

Create five fresh authored-HTTP fixture lineages with --create-fixtures, then
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
from urllib.parse import urlsplit
from http.server import ThreadingHTTPServer
import outcome_reappraisal_trial as old
import durable_action_trial as action

encode, decode, hash_text = old.encode, old.decode, old.hash_text
sha, write_json, arm_request = old.sha, old.write_json, old.arm_request
ARMS, METHODS, WORKLOAD = old.ARMS, old.METHODS, old.WORKLOAD
SEEDS = (71,83)
PROTOCOL = 'memory-benefit-pilot-v1'
FIXTURE_PROTOCOLS = (PROTOCOL,)
PLANNED_CALLS = 48
CORE_PROTOCOL = 'outcome-reappraisal-v4'
TREATMENTS = ('fast', 'thinking2048')
SIMULATOR = dict(Nori=(4,9,8), Arden=(8,3,9), Quill=(6,5,6))


def require(condition, message='invalid frozen diagnostic'):
    if not condition:
        raise ValueError(message)


def frozen_cases():
    specs = [
        ('boundaries', ((4,2,8),(8,7,9),(6,5,7)), 'minutes',3,9,'Arden'),
        ('energy-correction', ((4,2,8),(8,3,9),(6,5,7)), 'minutes',6,7,'Nori'),
        ('quality-alternative', ((4,9,8),(8,3,9),(6,5,10)), 'minutes',6,9,'Quill'),
        ('emptied', ((4,9,8),(8,3,8),(6,5,10)), 'minutes',5,9,'Quill'),
        ('unchanged', ((4,2,8),(8,7,9),(6,5,7)), 'minutes',6,7,'Arden'),
        ('energy-tie', ((4,4,8),(8,4,9),(6,5,8)), 'energy',5,8,'Arden'),
    ]
    return [dict(id=n,store='boundaries' if n=='unchanged' else n,
        source=dict(zip(METHODS,rows)),actual_choice=choice,actual=SIMULATOR[choice],
        contract=dict(workload=WORKLOAD,objective='minimize_'+objective,energy_budget=b,minimum_quality=q))
        for n,rows,objective,b,q,choice in specs]


def controls(treatment):
    require(treatment in TREATMENTS)
    thinking = treatment == 'thinking2048'
    return dict(old.MODEL_CONTROLS, thinking=thinking, thinking_budget=2048 if thinking else None)


def response_format(treatment):
    require(treatment in TREATMENTS)
    return 'feasibility_provenance'


def planned_pairs():
    plans=[]
    for j,seed in enumerate(SEEDS):
        for i,case in enumerate(frozen_cases()):
            rotated=TREATMENTS[(i+j)%2:]+TREATMENTS[:(i+j)%2]
            for k,treatment in enumerate(rotated):
                plans.append(dict(case=case['id'], seed=seed, treatment=treatment,
                                  outcome_first=bool((i+j)%2)))
    require(len(plans)*len(ARMS)==PLANNED_CALLS,'diagnostic ceiling mismatch')
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
    return {n for n,v in rows.items() if v[1 if contract['objective']=='minimize_energy' else 0]==min(x[1 if contract['objective']=='minimize_energy' else 0] for x in rows.values())} if rows else set()


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


def prerequisite_contract(case):
    # Select the designated source method without inserting an actual directly.
    index=1 if case['contract']['objective']=='minimize_energy' else 0
    chosen=case['source'][case['actual_choice']]
    contract=dict(workload=WORKLOAD,objective=case['contract']['objective'],
                  energy_budget=chosen[1],minimum_quality=chosen[2])
    feasible={n:v for n,v in case['source'].items() if v[1]<=chosen[1] and v[2]>=chosen[2]}
    require(case['actual_choice'] in {n for n,v in feasible.items() if v[index]==min(x[index] for x in feasible.values())},
            'prerequisite cannot select designated method')
    return contract


def effective_citations(case,arm,request):
    return {r['source'] for r in expected_checks(case,arm,request).values()}


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
            require(list(request['response_schema']['properties'])==schema_order(treatment),'schema treatment order mismatch')
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
        unnecessary_abstention=False,model_abstained=False,check_correctness=None,citation_coverage=False,reason=None,prose_status='unverified_model_prose',
        outcome_informed_optimal=False,simulator_optimal=False,host_withheld=(row.get('decision') or {}).get('disposition')=='withheld')
    try:
        validate_preparation(prepared,case,dict(action=prepared['action_id']),treatment)
        request=arm_request(prepared,row['arm']);call=row['call'];reply=call['reply'];answer=decode(reply['text'])
        # Preserve raw declared choice even when checks or envelope fail.
        if isinstance(answer,dict):
            score['raw_choice']=answer.get('choice')
            score['raw_response_field_order']=list(answer)
            score['response_order_matches_treatment']=list(answer)==schema_order(treatment)
        ids={p['id'] for p in request['passages']}
        fields={'choice','citations','reason','candidate_checks'}
        valid=(not row.get('error') and not call.get('error') and not reply.get('error') and reply['model']==model
            and call['prompt']==request['prompt'] and call['response_schema']==request['response_schema']
            and isinstance(answer,dict) and set(answer)==fields and answer['choice'] in (*METHODS,'abstain')
            and isinstance(answer['reason'],str) and bool(answer['reason'].strip()) and len(answer['reason'])<=4000
            and isinstance(answer['citations'],list) and all(type(c) is str for c in answer['citations'])
            and len(set(answer['citations']))==len(answer['citations']) and set(answer['citations'])<=ids)
        if not valid:return score
        gold=expected(case,row['arm']);hidden=expected(case,row['arm'],True);abstain=answer['choice']=='abstain'
        covered=effective_citations(case,row['arm'],request)<=set(answer['citations'])
        shared=expected(case,ARMS[1]); chosen=answer['choice']
        score.update(valid=True,model_abstained=abstain,citation_coverage=covered,reason=answer['reason'],
            outcome_informed_optimal=chosen in shared or (abstain and not shared),
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
    require(report['citation_policy']=='effective-measurements-v1')
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
            try:validate_report(pair,pair['report'],case,dict(action=pair['preparation']['action_id']),model)
            except (ValueError,KeyError,TypeError) as failure:pair['audit_error']=str(failure)
    matrix=[]
    for plan in planned_pairs():
        pair=lookup.get((plan['case'],plan['seed'],plan['treatment']),{})
        case=next(c for c in frozen_cases() if c['id']==plan['case'])
        for arm in ARMS:
            row=next((r for r in (pair.get('report') or {}).get('rows',[]) if isinstance(r,dict) and r.get('arm')==arm),None)
            raw_score=score_row(case,row,pair['preparation'],model,plan['treatment']) if row else None
            audit_error=pair.get('audit_error')
            matrix.append(dict(**plan,arm=arm,audit_error=audit_error,qualified_score=raw_score if not audit_error else None,status='recorded' if row else 'dispatch_unknown' if pair.get('started') else 'not_started',
                score=raw_score,
                error=pair.get('error'),call=row.get('call') if row else None,
                reasoning_char_count=row.get('reasoning_char_count') if row else None,
                first_response_error=row.get('error') if row else None,
                decision=row.get('decision') if row else None))
    require(set(lookup)<= {(p['case'],p['seed'],p['treatment']) for p in planned_pairs()})
    recorded=sum(r['status']=='recorded' for r in matrix)
    return dict(protocol=PROTOCOL,metrics=aggregate(matrix),planned_calls=PLANNED_CALLS,calls_recorded=recorded,matrix_complete=recorded==PLANNED_CALLS,
        unknown_dispatch_slots=sum(r['status']=='dispatch_unknown' for r in matrix),matrix=matrix,
        complete=recorded==PLANNED_CALLS and not any(p.get('error') or p.get('audit_error') for p in pairs),
        limitations='Six authored diagnostic cases and two seeds; no holdout or generalization claim. Raw choices, visible-evidence arithmetic, citation coverage, host withholding and hidden simulator outcomes are separate. Free-form explanation entailment requires independent review. Failed dispatch counts remain unknown; no repair or adaptive rerun. Private untrusted exports are outside local erasure.')


def aggregate(matrix):
    """All denominators include missing/uncertain slots; no silent exclusion."""
    metrics={}
    for arm in ARMS:
        rows=[r for r in matrix if r['arm']==arm]
        counts={key:sum(bool((r.get('qualified_score') or {}).get(key)) for r in rows)
                for key in ('valid','evidence_appropriate','outcome_informed_optimal','simulator_optimal',
                            'required_abstention','unnecessary_abstention','model_abstained',
                            'check_correctness','citation_coverage','host_withheld')}
        counts['rubric_pass']=sum(bool(r.get('qualified_score')) and all(r['qualified_score'].get(k) for k in
            ('valid','evidence_appropriate','check_correctness','citation_coverage')) for r in rows)
        counts['decision_joint_success']=sum(bool(r.get('qualified_score')) and all(r['qualified_score'].get(k) for k in
            ('valid','evidence_appropriate','check_correctness','citation_coverage')) and not r['qualified_score']['host_withheld'] for r in rows)
        counts['audit_invalid_slots']=sum(bool(r.get('audit_error')) for r in rows)
        counts['denominator']=len(rows)
        counts['recorded']=sum(r['status']=='recorded' for r in rows)
        counts['rates']={k:v/len(rows) if rows else None for k,v in counts.items() if k not in ('denominator','rates')}
        for field in ('input_tokens','output_tokens','millis'):
            values=[(r.get('call') or {}).get(field) if field=='millis' else
                    ((r.get('call') or {}).get('reply') or {}).get(field) for r in rows]
            observed=[v for v in values if type(v) in (int,float)]
            counts[field]=dict(sum=sum(observed),observed_slots=len(observed),missing_slots=len(rows)-len(observed))
        metrics[arm]=counts
    paired=dict(improvement=0,regression=0,unchanged_correct=0,unchanged_incorrect=0,unknown=0,denominator=len(planned_pairs()))
    for plan in planned_pairs():
        rows=[r for r in matrix if all(r[k]==plan[k] for k in ('case','seed','treatment'))]
        scores={r['arm']:r.get('qualified_score') for r in rows}
        if len(scores)!=2 or not all(s and s.get('valid') for s in scores.values()):paired['unknown']+=1;continue
        before=scores[ARMS[0]]['outcome_informed_optimal'];after=scores[ARMS[1]]['outcome_informed_optimal']
        key='improvement' if after and not before else 'regression' if before and not after else 'unchanged_correct' if after else 'unchanged_incorrect'
        paired[key]+=1
    metrics['paired_outcome_informed_choice']=paired
    return metrics


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
        for case in [c for c in frozen_cases() if c['id']!='unchanged']:
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
            contract=prerequisite_contract(case)
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
    require(len(server.requests)==5,'fixture creation unexpectedly used revision/retry calls')
    write_json(root/'fixtures.json',manifest)
    return manifest


def validate_fixtures(manifest):
    require(manifest['protocol'] in FIXTURE_PROTOCOLS and set(manifest['fixtures'])=={c['store'] for c in frozen_cases()})
    require(manifest['new_real_model_inference_calls']==0 and manifest['scripted_transport_requests']==5)
    dbs=set();actions=set()
    for fixture in manifest['fixtures'].values():
        path=Path(fixture['db']).resolve(strict=True);require(path not in dbs);dbs.add(path)
        require(fixture['action'] not in actions);actions.add(fixture['action'])
        require(sha(Path(fixture['source_path']))==fixture['source_sha256'])
    return manifest['fixtures']


def invoke_captured(argv, *, env, cwd, binary, artifact):
    """Retain successful initial transport output before parse/audit; never retry.

    Nonzero exit can signal revoked source access, so the core's fail-closed
    no-new-content boundary takes precedence over saving failed stdout.
    """
    try:
        result=subprocess.run([str(binary),*argv],env=env,cwd=cwd,text=True,capture_output=True,timeout=420)
    except subprocess.TimeoutExpired as failure:
        write_json(artifact,dict(error='timeout',dispatch_count=None))
        raise RuntimeError('CLI timeout; dispatch count unknown') from failure
    if result.returncode:
        write_json(artifact,dict(error='CLI nonzero exit',returncode=result.returncode,dispatch_count=None))
        raise RuntimeError(f'CLI exited {result.returncode}; dispatch count unknown')
    write_json(artifact,dict(stdout=result.stdout,stderr=result.stderr,returncode=0))
    return decode(result.stdout)


def run_trial(*,binary,fixtures,endpoint,model,holds_dir,output,invoke=None,
              inspect_model=old.model_state,inspect_database=old.database_state):
    fixture_manifest=decode(fixtures.read_text()) if isinstance(fixtures,Path) else fixtures
    stores=validate_fixtures(fixture_manifest)
    binary=binary.resolve(strict=True);holds_dir=holds_dir.resolve(strict=True)
    require(fixture_manifest['binary_sha256']==sha(binary),'fixture binary drift')
    require(fixture_manifest['runner_sha256']==sha(Path(__file__)),'fixture runner drift')
    require(fixture_manifest['helper_artifacts']==helper_artifacts(),'fixture helper drift')
    output.mkdir(parents=True,mode=0o700,exist_ok=False);root=output.resolve();env=isolated_env(root/'isolated-home')
    manifest=dict(protocol=PROTOCOL,private_data=True,untrusted_input=True,executed=False,planned_calls=PLANNED_CALLS,
        binary=str(binary),binary_sha256=sha(binary),runner_sha256=sha(Path(__file__)),helper_artifacts=helper_artifacts(),fixture_manifest=fixture_manifest,
        endpoint=endpoint,model=model,cases=frozen_cases(),seeds=list(SEEDS),treatments={t:controls(t) for t in TREATMENTS},
        offline_rubric=dict(simulator=SIMULATOR),pairs=[],error=None)
    pairs=manifest['pairs'];before={};state=None;error=None;interruption=None;tb=None
    write_json(root/'manifest.json',manifest)
    try:
        before={key:inspect_database(Path(f['db'])) for key,f in stores.items()};manifest['database_before']=before
        require(all(before[k]==f['database'] for k,f in stores.items()),'fixture database drift')
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
            prepared=(invoke or old.invoke_cli)(argv+['--prepare-only'],env=env,cwd=root,binary=binary)
            validate_preparation(prepared,case,fixture,t)
            if first is None:
                frozen_by_store[case['store']]=prepared;argv+=['--snapshot',encode(prepared['snapshot'])]
            else:
                require(all(prepared[k]==first[k] for k in ('snapshot','raw_recall','read','observation')),'source preparation drift')
            pairs.append(dict(**plan,argv=argv,preparation=prepared,report=None,error=None,started=False,
                request_hashes={a['arm']:hash_text(encode(a['request'])) for a in prepared['arms']},
                schema_property_order={a['arm']:list(a['request']['response_schema']['properties']) for a in prepared['arms']}))
            write_json(root/'manifest.json',manifest)
        # All 48 response slots and their exact preparations precede the first inference dispatch.
        write_json(root/'freeze.json',manifest);manifest['freeze_sha256']=sha(root/'freeze.json')
        write_json(root/'manifest.json',manifest)
        for pair in pairs:
            require(sha(binary)==manifest['binary_sha256'] and sha(Path(__file__))==manifest['runner_sha256'] and helper_artifacts()==manifest['helper_artifacts'],'executable/helper drift')
            validate_fixtures(fixture_manifest)
            require({key:inspect_database(Path(f['db'])) for key,f in stores.items()}==before,'database drift before pair')
            require(old.settings_identity(inspect_model(endpoint,model))==old.settings_identity(state),'resident settings drift')
            pair['started']=True;write_json(root/'manifest.json',manifest)
            directory=root/f"{pair['case']}-{pair['seed']}-{pair['treatment']}";directory.mkdir()
            write_json(directory/'progress.json',dict(status='started',dispatch_count=None))
            try:
                argv=pair['argv']+['--endpoint',endpoint,'--holds-dir',str(holds_dir)]
                if pair['outcome_first']:argv+=['--outcome-first']
                report=(invoke(argv,env=env,cwd=root,binary=binary) if invoke else
                    invoke_captured(argv,env=env,cwd=root,binary=binary,artifact=directory/'initial-cli-response.json'))
                case=next(c for c in frozen_cases() if c['id']==pair['case'])
                pair['report']=report;write_json(directory/'report.json',report)
                validate_report(pair,report,case,stores[case['store']],model)
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
        try:
            manifest['artifacts_unchanged']=sha(binary)==manifest['binary_sha256'] and sha(Path(__file__))==manifest['runner_sha256'] and helper_artifacts()==manifest['helper_artifacts']
            validate_fixtures(fixture_manifest)
        except (ValueError,OSError):manifest['artifacts_unchanged']=False
        manifest['database_unchanged']=bool(before) and manifest.get('database_after')==before
        manifest['settings_unchanged']=state is not None and 'resident_after' in manifest and old.settings_identity(manifest['resident_after'])==old.settings_identity(state)
        if not manifest['database_unchanged'] or not manifest['settings_unchanged'] or not manifest['artifacts_unchanged']:
            error=(error+'; ' if error else '')+'database/settings post-audit mismatch'
        manifest.update(error=error,interrupted=interruption is not None);write_json(root/'manifest.json',manifest)
    summary=summarize(pairs,model);summary.update(error=error,interrupted=interruption is not None,
        database_unchanged=manifest['database_unchanged'],settings_unchanged=manifest['settings_unchanged'],artifacts_unchanged=manifest['artifacts_unchanged'])
    summary['complete'] &= error is None and manifest['database_unchanged'] and manifest['settings_unchanged'] and manifest['artifacts_unchanged']
    write_json(root/'summary.json',summary)
    if interruption is not None:raise interruption.with_traceback(tb)
    return summary





def schema_order(treatment):
    return ['candidate_checks']+['choice','citations','reason']

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
        require(list(actual['response_format']['schema']['properties'])==list(wanted['response_format']['schema']['properties']),
                f'HTTP schema property order drift at slot {index}')
        require(('reasoning_budget_tokens' in actual)==('reasoning_budget_tokens' in wanted),
                f'HTTP thinking-budget presence drift at slot {index}')
        require(not actual.get('tools') and not actual.get('stream'),f'HTTP request surface drift at slot {index}')

class QualificationProvider(action.ScriptedProvider):
    """Authored queue, never a model; request bytes remain available for audit."""
    def do_GET(self):
        if self.path.startswith('/props?'):
            self.send_json(dict(model_path='authored-scripted-no-model',default_generation_settings={}))
        else:
            super().do_GET()

    def do_POST(self):
        if urlsplit(self.path).path != '/v1/chat/completions':
            self.send_error(404);return
        raw=self.rfile.read(int(self.headers['Content-Length'])).decode()
        request=decode(raw)
        self.server.raw_requests.append(raw)
        self.server.schema_orders.append(list(request['response_format']['schema']['properties']))
        self.server.requests.append(request)
        if not self.server.answers:
            self.send_error(500,'authored queue exhausted');return
        answer=self.server.answers.pop(0)
        self.send_json(dict(id='authored-memory-benefit-qualification',object='chat.completion',model=action.MODEL,
            choices=[dict(index=0,message=dict(role='assistant',content=json.dumps(answer,ensure_ascii=False,sort_keys=False)),finish_reason='stop')],
            usage=dict(prompt_tokens=0,completion_tokens=0,total_tokens=0)))



def check_authored_queue(report,answers,root):
    if answers:
        # This report was actually returned by the CLI. A source invalidation
        # exits without one and cannot reach this branch or expose new content.
        write_json(root/'failed-pair-report.json',report)
        raise ValueError('CLI did not consume both authored replies; see failed-pair-report.json')

def qualify_scripted(binary, fixtures, output):
    """Exercise all 48 actual CLI slots with preauthored replies and zero inference."""
    output.mkdir(parents=True,mode=0o700,exist_ok=False);root=output.resolve();holds=root/'holds';holds.mkdir()
    server=ThreadingHTTPServer(('127.0.0.1',0),QualificationProvider);server.requests=[];server.answers=[];server.raw_requests=[];server.schema_orders=[]
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    endpoint=f'http://127.0.0.1:{server.server_port}';preparations={};expected_requests=[]
    fixture_manifest=decode(fixtures.read_text()) if isinstance(fixtures,Path) else fixtures
    stores=validate_fixtures(fixture_manifest)
    def invoke(argv,**kwargs):
        if '--prepare-only' in argv:
            prepared=old.invoke_cli(argv,**kwargs)
            # First preparation gains --snapshot after this return; identify by case contract/settings.
            key=(prepared['action_id'],encode(prepared['contract']),prepared['response_format'],
                prepared['model_controls']['thinking_budget'])
            preparations[key]=prepared;return prepared
        contract=decode(argv[argv.index('--contract')+1]);action_id=argv[argv.index('--action')+1]
        treatment='thinking2048' if '--thinking-budget' in argv else 'fast'
        prepared=preparations[(action_id,encode(contract),response_format(treatment),controls(treatment)['thinking_budget'])]
        case=next(c for c in frozen_cases() if c['contract']==contract and stores[c['store']]['action']==action_id)
        order=list(reversed(ARMS)) if '--outcome-first' in argv else list(ARMS)
        require(not server.answers,'unconsumed authored response')
        for arm in order:
            request=arm_request(prepared,arm)
            expected_requests.append(expected_http_request(request,treatment,int(argv[argv.index('--seed')+1]),action.MODEL))
            answer=dict(choice=next(iter(sorted(expected(case,arm))),'abstain'),
                citations=sorted(effective_citations(case,arm,request)),reason='Authored qualification response; no model inference.')
            if response_format(treatment)=='feasibility_provenance':answer=dict(candidate_checks=expected_checks(case,arm,request),**answer)
            server.answers.append(answer)
        report=invoke_captured(argv,artifact=root/('initial-cli-response-'+str(len(expected_requests))+'.json'),**kwargs)
        check_authored_queue(report,server.answers,root)
        return report
    try:
        summary=run_trial(binary=binary,fixtures=fixtures,endpoint=endpoint,model=action.MODEL,
            holds_dir=holds,output=root/'trial',invoke=invoke)
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)
        write_json(root/'http-requests.json',server.requests)
        write_json(root/'http-raw-requests.json',server.raw_requests)
        write_json(root/'http-schema-property-orders.json',server.schema_orders)
        write_json(root/'expected-http-requests.json',expected_requests)
    audit_http_requests(expected_requests,server.requests)
    require(summary['complete'] and len(server.requests)==PLANNED_CALLS,'scripted CLI qualification incomplete')
    require(all(r['score']['valid'] and r['score']['evidence_appropriate'] and r['score']['citation_coverage']
                and r['score']['check_correctness'] is not False and r['score']['response_order_matches_treatment']
                and not r['score']['host_withheld'] for r in summary['matrix']),
            'scripted CLI qualification failed independent rubric')
    result=dict(complete=True,new_real_model_inference_calls=0,scripted_transport_requests=PLANNED_CALLS,
        http_request_audit=True,recorded_http_sha256=sha(root/'http-requests.json'),
        expected_http_sha256=sha(root/'expected-http-requests.json'),
        raw_http_sha256=sha(root/'http-raw-requests.json'),schema_property_orders=server.schema_orders,helper_artifacts=helper_artifacts(),summary=summary)
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
