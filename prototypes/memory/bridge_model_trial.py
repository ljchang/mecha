#!/usr/bin/env python3
"""Frozen 8-pair native bridge comparison. Synthetic inputs only; never retries.

Run `--preflight` first with the exact native example binary. Scripted HTTP
qualification is not real-model evidence. Every started/failed slot is retained.
"""
import argparse
import copy
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import threading
import time

PROTOCOL = 'connected-bridge-model-comparison-v1'
BUDGETS = dict(max_turns=8, max_tokens=4096, max_output_tokens=16384, timeout_secs=600)
CONTROLS = dict(temperature=0.6, fast=True, provider_timeout_secs=180, retries=0)
TOOLS = ['fs_read', 'fs_write', 'goal_context', 'todo']
ABSTENTION = [{'status': 'needs_information'}]
SCRIPTED_MODEL = 'synthetic-bridge-scripted-no-inference'
PARTIAL_CANARY = 'PARTIAL_STREAM_OBSERVED_628194' 


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False)+'\n'


def decode(text):
    def unique(pairs):
        result={}
        for key,value in pairs:
            if key in result: raise ValueError('duplicate JSON object key')
            result[key]=value
        return result
    return json.loads(text,object_pairs_hook=unique)


def digest(value):
    return hashlib.sha256(encode(value).encode()).hexdigest()


def file_hash(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    Path(path).write_text(encode(value))


def frozen_cases():
    common = ('Read input.csv and write only a JSON array of string-valued objects to output.json. '
              'Use the current criterion. Prior evidence may be consulted with goal_context. '
              'If a required calibration parameter is unavailable in authorized evidence, write '
              '[{"status":"needs_information"}]. Do not guess or copy old output rows.')
    def task(prompt, text, csv, gold):
        return dict(prompt=common+' '+prompt, criterion_text=text, files={'input.csv':csv}, expected_rows=gold)
    return [
        dict(id='calibration_suffix', requires_prior=True, withdraw=False,
             A=task('Calibrate the Larch label workflow. The calibration suffix is CAL-729.',
                    'For each row append the confirmed calibration suffix to label with a hyphen; preserve id.',
                    'id,label\na1,amber\na2,blue\n', [{'id':'a1','label':'amber-CAL-729'},{'id':'a2','label':'blue-CAL-729'}]),
             B=task('Continue the Larch label workflow on these new records using its prior confirmed calibration.',
                    'For each row append the prior confirmed calibration suffix to label with a hyphen; preserve id.',
                    'id,label\nb1,green\nb2,red\n', [{'id':'b1','label':'green-CAL-729'},{'id':'b2','label':'red-CAL-729'}])),
        dict(id='quoted_newline', requires_prior=False, withdraw=False,
             A=task('Process the Larch multiline export.', 'Include active=yes rows; preserve id and exact note text, including quoted newlines.',
                    'id,active,note\na1,yes,"first\nsecond"\na2,no,omit\n', [{'id':'a1','note':'first\nsecond'}]),
             B=task('Process a new Larch multiline export.', 'Include active=yes rows; preserve id and exact note text, including quoted newlines.',
                    'id,active,note\nb1,no,omit\nb2,yes,"third\nfourth"\nb3,yes,"a,b"\n',
                    [{'id':'b2','note':'third\nfourth'},{'id':'b3','note':'a,b'}])),
        dict(id='changed_criterion', requires_prior=False, withdraw=False,
             A=task('Process the Larch selection export.', 'Include active=yes rows; preserve id and lowercase label.',
                    'id,active,label\na1,yes,Amber\na2,no,Blue\n', [{'id':'a1','label':'amber'}]),
             B=task('The Larch selection rule has changed. Apply only the current criterion.',
                    'Include active=no rows; preserve id and uppercase label.',
                    'id,active,label\nb1,yes,Green\nb2,no,Red\n', [{'id':'b2','label':'RED'}])),
        dict(id='withdrawn_parameter', requires_prior=True, withdraw=True,
             A=task('Calibrate the Larch withdrawal workflow. Its suffix is WITHDRAW-853.',
                    'For each row append the confirmed calibration suffix to label with a hyphen; preserve id.',
                    'id,label\na1,amber\n', [{'id':'a1','label':'amber-WITHDRAW-853'}]),
             B=task('Continue the Larch withdrawal workflow. Prior calibration authorization has been withdrawn; '
                    'use no withdrawn evidence and request information if no authorized calibration remains.',
                    'Append an authorized confirmed calibration suffix to each label, preserving id. If none is available, return the needs_information status row.',
                    'id,label\nb1,green\n', ABSTENTION))]


def planned_pairs():
    return [dict(id=f'{case["id"]}-r{rep}', case=case['id'], replicate=rep, seed=70400+rep,
                 order=['memory','control'] if (2*i+rep)%2 == 0 else ['control','memory'])
            for i, case in enumerate(frozen_cases()) for rep in range(2)]


def make_request(case, stage, pair_id, source=None):
    task = case[stage]
    criterion = dict(goal='task:larch', goal_text='Transform the synthetic Larch export under its current criterion.',
                     criterion_text=task['criterion_text'], criterion_revision=digest(task['criterion_text']),
                     confirmation_digest=digest({'synthetic_confirmed_criterion': task['criterion_text']}))
    return dict(version=1, task_id=pair_id+'-'+stage, prompt=task['prompt'], criterion=criterion,
                expected_rows=copy.deepcopy(task['expected_rows']), files=copy.deepcopy(task['files']),
                source=source, withdraw_source=bool(source and stage == 'B' and case['withdraw']))


def source_hashes():
    repo=Path(__file__).resolve().parents[2]
    files=list((repo/'mecha-core'/'src').rglob('*.rs'))+list((repo/'mecha-core'/'examples').rglob('*.rs'))
    files += [repo/'Cargo.toml',repo/'Cargo.lock',repo/'mecha-core'/'Cargo.toml',repo/'rust-toolchain.toml']
    return {str(p.relative_to(repo)):file_hash(p) for p in sorted(files)}


def freeze(root, binary, endpoint, model, holds_dir, source_commit=None, scripted=False, server_info=None):
    root, binary = Path(root).resolve(), Path(binary).resolve(strict=True)
    root.mkdir(parents=True, exist_ok=False)
    cases = frozen_cases()
    write_json(root/'cases.json', cases)
    if source_commit is None:
        source_commit = subprocess.check_output(['git','rev-parse','HEAD'], cwd=Path(__file__).resolve().parents[2], text=True).strip()
    server_observation=None
    if server_info is not None:
        info_path=Path(server_info).resolve(strict=True)
        server_observation=dict(value=decode(info_path.read_text()),source=str(info_path),source_sha256=file_hash(info_path),
            evidence_boundary='Pre-run observed server metadata supplied by operator; not a guarantee of runtime defaults or model-weight identity for an alias.')
    manifest = dict(server_observation=server_observation,protocol=PROTOCOL, synthetic_only=True, scripted=scripted, source_commit=source_commit,
                    source_files_sha256=source_hashes(), binary=str(binary), binary_sha256=file_hash(binary), helper=str(Path(__file__).resolve()),
                    helper_sha256=file_hash(__file__), endpoint=endpoint, model=model,
                    holds_dir=str(Path(holds_dir).resolve()), model_controls=CONTROLS, budgets=BUDGETS,
                    tools=TOOLS, planned_pairs=planned_pairs(), planned_slots=24,
                    cases_sha256=file_hash(root/'cases.json'),
                    case_hashes={c['id']:{stage:dict(input_sha256=digest({k:c[stage][k] for k in ('prompt','criterion_text','files')}),
                                                            oracle_sha256=digest(c[stage]['expected_rows'])) for stage in ('A','B')} for c in cases})
    write_json(root/'manifest.json', manifest)
    (root/'manifest.sha256').write_text(file_hash(root/'manifest.json')+'\n')
    return manifest


def validate_frozen(root, manifest):
    if (json.loads((root/'manifest.json').read_text()) != manifest
            or file_hash(root/'manifest.json') != (root/'manifest.sha256').read_text().strip()
            or file_hash(root/'cases.json') != manifest['cases_sha256']
            or file_hash(manifest['binary']) != manifest['binary_sha256']
            or file_hash(__file__) != manifest['helper_sha256']
            or source_hashes() != manifest['source_files_sha256']
            or json.loads((root/'cases.json').read_text()) != frozen_cases()):
        raise ValueError('modified frozen inputs, manifest, binary or helper')


def invoke_native(manifest, request_file, trial_root, seed):
    argv = [manifest['binary'],'--synthetic-only',str(request_file),str(trial_root),
            '--endpoint',manifest['endpoint'],'--model',manifest['model'],
            '--holds-dir',manifest['holds_dir'],'--seed',str(seed)]
    env = os.environ.copy()
    for key in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT'):
        env.pop(key, None)
    env.update(MECHA_SESSION_KIND='test', MECHA_LOG='warn')
    started=time.monotonic()
    result=dict(status='failed', argv=argv, native=None, input_tokens=None, output_tokens=None,
                returncode=None, error=None, timed_out=False)
    write_json(request_file.with_suffix('.started.json'), {'argv':argv, 'started_unix':time.time()})
    try:
        call=subprocess.run(argv, cwd=request_file.parent, env=env, capture_output=True, timeout=615)
        result.update(returncode=call.returncode, status='recorded' if call.returncode==0 else 'failed')
        stdout, stderr=call.stdout, call.stderr
    except subprocess.TimeoutExpired as error:
        stdout,stderr=error.stdout or b'',error.stderr or b''
        result.update(error='coordinator timeout after native 600s bound plus 15s shutdown allowance',timed_out=True)
    except OSError as error:
        stdout,stderr=b'',b'';result['error']=str(error)
    request_file.with_suffix('.stdout').write_bytes(stdout)
    request_file.with_suffix('.stderr').write_bytes(stderr)
    result['wall_elapsed_ms']=round((time.monotonic()-started)*1000)
    report=trial_root/'report.json'
    if report.exists():
        try:
            native=json.loads(report.read_text())
            result.update(native=native,input_tokens=native.get('input_tokens'),output_tokens=native.get('output_tokens'))
            if native.get('status') != 'completed': result['status']='failed'
            if native.get('run_error'): result['error']=native['run_error']
        except (ValueError,OSError) as error:
            result['error']='malformed native report: '+str(error)
    else:
        result['error']=result['error'] or 'missing native report'
        result['status']='failed'
    return result


def snapshot(root):
    return {str(p.relative_to(root)):file_hash(p) for p in sorted(root.rglob('*')) if p.is_file()}


def score_artifact(case, stage, arm, actual):
    gold=case[stage]['expected_rows']
    valid=(isinstance(actual,list) and all(isinstance(r,dict) and all(isinstance(k,str) and isinstance(v,str) for k,v in r.items()) for r in actual))
    need=stage=='B' and (case['withdraw'] or (case['requires_prior'] and arm=='control'))
    return dict(artifact_valid=valid,task_correct=valid and actual==gold,
                abstained=valid and actual==ABSTENTION, appropriate_abstention=valid and actual==ABSTENTION and need,
                evidence_requires_abstention=need)


def request_contract(request):
    return {k:request[k] for k in ('task_id','prompt','criterion','expected_rows','files')}


def audit_contract(native):
    """Retain effective settings for every request; prompt/history stays in audit."""
    result={'runtime_config':native.get('config'), 'first_request':None, 'effective_settings':None, 'requests':[]}
    path=native.get('provider_audit')
    if path and Path(path).exists():
        for line in Path(path).read_text().splitlines():
            row=json.loads(line)
            if row.get('event') == 'request_started':
                body={k:v for k,v in (row.get('wire_body') or {}).items() if k != 'messages'}
                effective={k:(row.get('request') or {}).get(k) for k in ('local_only','think','think_budget')}
                result['requests'].append({'wire_body':body,'effective_settings':effective})
                if result['first_request'] is None:
                    result['first_request']=body
                    result['effective_settings']=effective
    return result


def controls_match(contract, manifest, seed):
    requests=contract.get('requests',[])
    if contract.get('runtime_config')!=manifest['budgets'] or not requests:
        return False
    for index,request in enumerate(requests):
        body=request['wire_body'];effective=request['effective_settings']
        tools=body.get('tools') or []
        maximum=body.get('max_tokens')
        if not (body.get('model')==manifest['model'] and body.get('temperature')==0.6
                and body.get('seed')==seed and isinstance(maximum,int) and 0<maximum<=4096
                and (index!=0 or maximum==4096)
                and body.get('chat_template_kwargs',{}).get('enable_thinking') is False
                and effective.get('local_only') is True and effective.get('think') is False
                and effective.get('think_budget') is None
                and sorted(t.get('function',{}).get('name','') for t in tools)==manifest['tools']):
            return False
    return True


def first_contract(contract):
    return {k:contract[k] for k in ('runtime_config','first_request','effective_settings')}


def execute(root, manifest, invoke=invoke_native):
    root=Path(root)
    validate_frozen(root,manifest)
    # An interrupted output directory is evidence; never resume or replace slots.
    (root/'execution.started').open('x').close()
    cases={c['id']:c for c in json.loads((root/'cases.json').read_text())}
    slots=[]; pair_audits=[]
    for pair in manifest['planned_pairs']:
        case=cases[pair['case']]; pair_root=root/pair['id']; pair_root.mkdir()
        originals=None; source=None; b_contracts=[]; runtime_contracts=[]
        pair_audit=dict(pair=pair['id'], originals_unchanged=None, matched_b_inputs=True, matched_runtime_config=None)
        for stage, arm in [('A','A')]+[('B',arm) for arm in pair['order']]:
            slot=dict(pair=pair['id'],case=case['id'],stage=stage,arm=arm,seed=pair['seed'],status='not_started',
                      native=None,input_tokens=None,output_tokens=None,error=None)
            if stage=='B' and source is None:
                slot['error']='A capture unavailable; planned slot retained'
            else:
                try:
                    validate_frozen(root,manifest)
                    request=make_request(case,stage,pair['id'],source if arm=='memory' else None)
                    request_file=pair_root/(arm+'.request.json'); write_json(request_file,request)
                    if stage=='B': b_contracts.append(digest(request_contract(request)))
                    trial_root=pair_root/arm
                    slot.update(invoke(manifest,request_file,trial_root,pair['seed']))
                    native=slot.get('native') or {}
                    actual=None
                    artifact=native.get('artifact')
                    if artifact:
                        try: actual=decode(Path(artifact).read_text())
                        except (OSError,ValueError): pass
                    slot['score']=score_artifact(case,stage,arm,actual)
                    slot['config_audit']=audit_contract(native)
                    slot['frozen_controls_match']=controls_match(slot['config_audit'],manifest,pair['seed'])
                    slot['configuration_hashes']=dict(
                        runtime_config=dict(sha256=digest(native.get('config')),source=str(trial_root/'report.json')+'#config'),
                        provider_settings_and_tools=dict(sha256=digest(slot['config_audit']['first_request']),
                            source=str(native.get('provider_audit'))+'#first-request_started/wire_body excluding messages'),
                        task_input_and_oracle=dict(sha256=digest(request_contract(request)),source=str(request_file)))
                    if stage=='B': runtime_contracts.append(slot['config_audit'])
                    if stage=='A' and native.get('capture_receipt') and not native.get('capture_error') and native.get('confirmation'):
                        source={key:native[key] for key in ('database','authority','confirmation')}
                        originals=snapshot(trial_root)
                        write_json(pair_root/'A.frozen-hashes.json', originals)
                        pair_audit['A_artifact_passed']=(native.get('artifact_check') or {}).get('verdict')=='Passed'
                except Exception as error:
                    slot.update(status='failed',error=f'{type(error).__name__}: {error}')
            slots.append(slot)
            write_json(pair_root/(arm+'.slot.json'),slot)
            write_json(root/'slots.partial.json',slots)
        if originals is not None:
            pair_audit['originals_unchanged']=snapshot(pair_root/'A')==originals
        pair_audit['matched_b_inputs']=len(set(b_contracts))<=1
        pair_audit['matched_runtime_config']=(len(runtime_contracts)==2 and first_contract(runtime_contracts[0])==first_contract(runtime_contracts[1])
            and all(controls_match(c,manifest,pair['seed']) for c in runtime_contracts))
        pair_audits.append(pair_audit)
    summary=summarize(slots,pair_audits,manifest)
    write_json(root/'summary.json',summary)
    return summary


def summarize(slots, pair_audits, manifest):
    result=dict(protocol=PROTOCOL,scripted=manifest['scripted'], planned_pairs=8,planned_slots=24,slots=slots,
                pair_audits=pair_audits,unknown_usage_slots=sum(s['status']!='not_started' and (s.get('input_tokens') is None or s.get('output_tokens') is None) for s in slots),
                arms={},complete=len(slots)==24 and all(s['status']=='recorded' for s in slots),
                limitations='Four authored synthetic development cases, two replicates; no generalization or significance claim. '
                'Information availability is not appraisal-specific causal benefit. Task correctness and evidence-appropriate '
                'abstention are distinct. No retrieval is a measured result. Unknown usage is not zero. '
                'Scripted preflight, if selected, is transport/integration evidence only.')
    for arm in ('A','memory','control'):
        rows=[s for s in slots if s['arm']==arm]
        result['arms'][arm]=dict(denominator=8,task_correct=sum(bool(s.get('score',{}).get('task_correct')) for s in rows),
            appropriate_abstention=sum(bool(s.get('score',{}).get('appropriate_abstention')) for s in rows),
            captures=sum(bool((s.get('native') or {}).get('capture_receipt')) for s in rows),
            failures=sum(s['status']=='failed' for s in rows),not_started=sum(s['status']=='not_started' for s in rows),
            no_memory_calls=sum((s.get('native') or {}).get('recall_calls')==0 for s in rows),
            recall_references=[(s.get('native') or {}).get('recall_references') for s in rows])
    result['integrity_passed']=all(p['originals_unchanged'] is not False and p['matched_b_inputs'] and p['matched_runtime_config'] for p in pair_audits) and all(s.get('frozen_controls_match') for s in slots if s['status']!='not_started')
    return result


class ScriptedHTTP(BaseHTTPRequestHandler):
    """Authored responses over the actual provider transport; no model inference."""
    def log_message(self, *_):
        pass

    def send_json(self, value, status=200):
        body=encode(value).encode()
        self.send_response(status)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == '/props': self.send_json({'role':'router'})
        elif self.path in ('/models','/v1/models'):
            self.send_json({'data':[{'id':SCRIPTED_MODEL,'status':{'value':'loaded'}}]})
        else: self.send_error(404)

    def do_POST(self):
        if self.path.split('?')[0] != '/v1/chat/completions':
            self.send_error(404); return
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.requests.append(body)
        mode=self.server.mode
        if mode=='partial_stream_error':
            chunk={'id':'partial','object':'chat.completion.chunk','model':SCRIPTED_MODEL,
                   'choices':[{'index':0,'delta':{'content':PARTIAL_CANARY},'finish_reason':None}]}
            wire=('data: '+encode(chunk).strip()+'\n\n').encode()
            self.send_response(200);self.send_header('Content-Type','text/event-stream')
            self.send_header('Content-Length',str(len(wire)))
            self.end_headers();self.wfile.write(wire)
            return
        if mode=='transport_error':
            self.send_json({'error':{'message':'authored failure'}},503); return
        step=self.server.step; self.server.step+=1
        script=self.server.script
        message={'role':'assistant','content':'Synthetic task finished; host must check.'}
        reason='stop'
        if step<len(script):
            name,args=script[step]
            message={'role':'assistant','content':None,'tool_calls':[{
                'id':f'call{step}','type':'function',
                'function':{'name':name,'arguments':encode(args)}}]}
            reason='tool_calls'
        reply={'id':'scripted','object':'chat.completion','model':SCRIPTED_MODEL,
               'choices':[{'index':0,'message':message,'finish_reason':reason}]}
        if mode!='unknown_usage': reply['usage']={'prompt_tokens':11,'completion_tokens':7,'total_tokens':18}
        if body.get('stream'):
            delta=dict(message);delta.pop('role',None)
            for index,tool in enumerate(delta.get('tool_calls',[])):tool['index']=index
            chunk=dict(reply);chunk['object']='chat.completion.chunk'
            chunk['choices']=[{'index':0,'delta':delta,'finish_reason':reason}]
            wire=('data: '+encode(chunk).strip()+'\n\ndata: [DONE]\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type','text/event-stream')
            self.send_header('Content-Length',str(len(wire)))
            self.end_headers();self.wfile.write(wire)
        else:self.send_json(reply)


def declared_check_script():
    item = dict(content='Verify the synthetic output', check='test -f output.json')
    return [('todo', {'items': [dict(item, status='pending')]}),
            ('todo', {'items': [dict(item, status='completed')]}),
            ('fs_write', {'path': 'output.json', 'content': '[]'})]


def declared_check_evidence(native, wire):
    path = native.get('transcript')
    records = [decode(line) for line in Path(path).read_text().splitlines()] if path and Path(path).exists() else []
    messages = [r for r in records if r.get('record') == 'message']
    blocks = [b for m in messages for b in m.get('content', [])]
    feedback = [m['planning'] for m in messages if m.get('planning')]
    steps = [step for f in feedback for step in f.get('steps', [])]
    decisions = [d for f in feedback for d in f.get('decisions', [])]
    text = encode(records) + encode(wire)
    wire_calls = [call for r in wire for m in r.get('messages', []) for call in m.get('tool_calls', [])]
    uses = {b.get('id') for b in blocks if b.get('type') == 'tool_use'}
    results = {b.get('tool_use_id') for b in blocks if b.get('type') == 'tool_result'}
    return dict(
        transcript_present=bool(records),
        no_orphan_results=uses == results,
        same_four_tools=bool(wire) and all(sorted(t['function']['name'] for t in r.get('tools', [])) == TOOLS for r in wire),
        no_shell_dispatch=not any(b.get('type') == 'tool_use' and
            (b.get('name') == 'shell' or b.get('id', '').startswith('mecha-check-')) for b in blocks) and not any(
            call.get('function', {}).get('name') == 'shell' or call.get('id', '').startswith('mecha-check-')
            for call in wire_calls),
        no_check_trace='step.check' not in text,
        no_unknown_shell=all(term not in text.lower() for term in ('unknown tool', 'no tool named `shell`')),
        explicit_unverified='A declared plan check was not run; completion remains unverified.' in text,
        skipped_step=len(steps) == 1 and steps[0].get('verification') == 'skipped',
        remains_unverified=bool(decisions) and decisions[-1].get('unverified_steps') == 1,
        never_passed=bool(steps) and all(step.get('verification') != 'passed' for step in steps))


def preflight(binary, root):
    root=Path(root).resolve()
    server=ThreadingHTTPServer(('127.0.0.1',0),ScriptedHTTP)
    server.requests=[]; server.step=0; server.script=[];server.mode='normal'
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        manifest=freeze(root,binary,f'http://127.0.0.1:{server.server_port}',SCRIPTED_MODEL,root/'holds',scripted=True)
        requests_by_slot={}
        def scripted_invoke(manifest,request_file,trial_root,seed):
            req=json.loads(request_file.read_text())
            case=next(c for c in frozen_cases() if req['task_id'].startswith(c['id']+'-'))
            stage='A' if req['task_id'].endswith('-A') else 'B'
            arm=request_file.stem.split('.')[0]
            rows=req['expected_rows']
            if stage=='B' and case['requires_prior'] and not req['source']: rows=ABSTENTION
            server.mode='normal';server.step=0
            server.script=[('fs_read',{'path':'input.csv'})]
            if stage=='B':server.script.append(('goal_context',{'memory':{'query':'Larch'}}))
            server.script.append(('fs_write',{'path':'output.json','content':encode(rows)}))
            before=len(server.requests)
            result=invoke_native(manifest,request_file,trial_root,seed)
            requests_by_slot[req['task_id']+'-'+arm]=server.requests[before:]
            return result
        result=execute(root,manifest,invoke=scripted_invoke)
        checks={}
        checks['all_native_captures']=all((s.get('native') or {}).get('capture_receipt') for s in result['slots'])
        checks['memory_has_references']=all((s.get('native') or {}).get('recall_references') for s in result['slots']
            if s['arm']=='memory' and s['case']!='withdrawn_parameter')
        checks['control_and_withdrawn_have_no_references']=all(not (s.get('native') or {}).get('recall_references') for s in result['slots']
            if s['arm']=='control' or (s['arm']=='memory' and s['case']=='withdrawn_parameter'))
        checks['original_A_unchanged']=all(p['originals_unchanged'] is True for p in result['pair_audits'])
        checks['same_B_runtime_contract']=all(p['matched_runtime_config'] for p in result['pair_audits'])
        checks['all_tools_present']=all(sorted(t['function']['name'] for t in r.get('tools',[]))==TOOLS for r in server.requests)
        checks['effective_controls']=all(r.get('temperature')==0.6 and r.get('max_tokens')==4096
            and r.get('seed') in (70400,70401)
            and r.get('chat_template_kwargs',{}).get('enable_thinking') is False for r in server.requests)
        # Probe host files using ordinary native dispatch. Neither the gold file,
        # request oracle, nor A originals are in this fresh workspace.
        probe_root=root/'probes';probe_root.mkdir()
        gold_canary='HOST_GOLD_ONLY_283941731'
        gold_file=probe_root/'host-gold.txt';gold_file.write_text(gold_canary)
        first=next((s for s in result['slots'] if s['arm']=='A' and (s.get('native') or {}).get('capture_receipt')),None)
        if first is None:
            checks['probes']=False
            result.update(preflight_checks=checks,probes=[],complete=False,integrity_passed=False,
                          new_real_model_inference_calls=0,preflight_error='No A capture available for native probes')
            write_json(root/'preflight-summary.json',result)
            return result
        original=first['native']
        probe_rows=[]
        for mode in ('jail','transport_error','partial_stream_error','missing_artifact','malformed_artifact','no_memory','unknown_usage','declared_check_unavailable'):
            req=make_request(frozen_cases()[0],'B','probe-'+mode)
            req['expected_rows']=[{'host_only':gold_canary}]
            req_file=probe_root/(mode+'.request.json');write_json(req_file,req)
            server.mode=mode;server.step=0;server.script=[]
            if mode=='jail':
                server.script=[('fs_read',{'path':str(p)}) for p in (gold_file,req_file,original['artifact'],original['authority'])]
            elif mode in ('no_memory','unknown_usage'):
                server.script=[('fs_write',{'path':'output.json','content':'[]'})]
            elif mode=='declared_check_unavailable':
                server.script=declared_check_script()
            elif mode=='malformed_artifact':
                server.script=[('fs_write',{'path':'output.json','content':'not json'})]
            before=len(server.requests)
            trial=invoke_native(manifest,req_file,probe_root/mode,70400)
            wire=server.requests[before:]
            write_json(probe_root/(mode+'.wire.json'),wire)
            native=trial.get('native') or {}
            passed=False
            if mode=='jail':
                tool_results=[m for r in wire for m in r['messages'] if m.get('role')=='tool']
                passed=(len(wire)==5 and len(tool_results)>=4 and gold_canary not in encode(wire)
                        and all('outside' in str(m.get('content','')).lower() or 'denied' in str(m.get('content','')).lower()
                                or 'permission' in str(m.get('content','')).lower() for m in tool_results))
            elif mode=='transport_error':passed=native.get('model_calls_started')==1 and bool(native.get('run_error'))
            elif mode=='partial_stream_error':
                audit_path=native.get('provider_audit')
                audit_text=Path(audit_path).read_text() if audit_path and Path(audit_path).exists() else ''
                passed=(native.get('model_calls_started')==1 and native.get('model_calls_completed')==0
                        and bool(native.get('run_error')) and native.get('input_tokens') is None
                        and native.get('output_tokens') is None and not native.get('capture_receipt')
                        and PARTIAL_CANARY in audit_text and 'error' in audit_text)
            elif mode=='missing_artifact':passed=(native.get('artifact_check') or {}).get('verdict')=='Unavailable'
            elif mode=='malformed_artifact':passed=(native.get('artifact_check') or {}).get('verdict')=='Failed'
            elif mode=='no_memory':passed=native.get('recall_calls')==0
            elif mode=='unknown_usage':passed=native.get('input_tokens') is None and native.get('output_tokens') is None
            evidence=None
            if mode=='declared_check_unavailable':
                evidence=declared_check_evidence(native,wire)
                evidence['artifact_written']=Path(native.get('artifact','')).is_file() and decode(Path(native['artifact']).read_text()) == []
                evidence['finished']=not native.get('run_error') and native.get('model_calls_completed') == 4
                passed=all(evidence.values())
            probe_rows.append(dict(mode=mode,passed=bool(passed),result=trial,**({'checks':evidence} if evidence else {})))
        checks['probes']=all(p['passed'] for p in probe_rows)
        result.update(preflight_checks=checks,probes=probe_rows,scripted_http_requests=len(server.requests),new_real_model_inference_calls=0)
        result['complete']=result['complete'] and all(checks.values())
        result['integrity_passed']=result['integrity_passed'] and all(checks.values())
        write_json(root/'preflight-wire.json',requests_by_slot)
        write_json(root/'preflight-summary.json',result)
        return result
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--endpoint')
    parser.add_argument('--model')
    parser.add_argument('--holds-dir',type=Path)
    parser.add_argument('--preflight',action='store_true')
    parser.add_argument('--server-info',type=Path,help='Host-only observed server props/models/build metadata JSON to freeze in manifest')
    args=parser.parse_args()
    if args.preflight:
        result=preflight(args.binary,args.output)
    else:
        if not all((args.endpoint,args.model,args.holds_dir)): parser.error('real run requires explicit endpoint, model and holds-dir')
        manifest=freeze(args.output,args.binary,args.endpoint,args.model,args.holds_dir,server_info=args.server_info)
        result=execute(args.output.resolve(),manifest)
    print(json.dumps({k:v for k,v in result.items() if k not in ('slots','pair_audits')},indent=2))
    return 0 if result['complete'] and result['integrity_passed'] else 1


if __name__=='__main__': raise SystemExit(main())
