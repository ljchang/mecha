#!/usr/bin/env python3
"""Bounded actual-CLI qualification with a scripted loopback reply, zero real inference.

This is persistence/action qualification, never a fresh model performance score.
No owner stores, services, SSH, model loads, or third-party packages are used.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import threading
from urllib.parse import urlsplit

PROTOCOL = 'durable-synthetic-action-qualification-v1'
MODEL = 'synthetic-scripted-action-no-inference'
WORKLOAD = 'durable-action-fixture-v1'
CASES = (('mismatch', 'baseline', 'Nori', 'failed'),
         ('passed', 'baseline', 'Nori', 'passed'),
         ('refused', 'refused', 'Nori', 'refused'),
         ('unknown', 'unknown', 'Nori', 'pending'),
         ('abstain', 'baseline', 'abstain', None))


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))+'\n'


def frozen_source():
    return dict(kind='host_authored_synthetic_measurements_v1', workload=WORKLOAD, complete=True,
                observations=[dict(choice=c, observed_minutes=m, energy=e, quality=q)
                              for c,m,e,q in [('Nori',4,2,8),('Arden',8,3,9),('Quill',6,5,7)]])


class ScriptedProvider(BaseHTTPRequestHandler):
    """Only returns authored JSON; implements the bounded transport under test."""
    def log_message(self, *_):
        pass

    def send_json(self, value):
        body=encode(value).encode()
        self.send_response(200)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == '/props':
            self.send_json(dict(role='router'))
            return
        if self.path != '/models':
            self.send_error(404)
            return
        self.send_json(dict(data=[dict(id=MODEL,status=dict(value='loaded'))]))

    def do_POST(self):
        if urlsplit(self.path).path != '/v1/chat/completions':
            self.send_error(404)
            return
        request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.requests.append(request)
        choice=self.server.choice
        answer=dict(choice=choice,citations=[] if choice=='abstain' else ['e1','e2','e3'],
                    reason='Authored synthetic transport reply; no inference.')
        self.send_json(dict(id='synthetic-scripted',object='chat.completion',model=MODEL,
            choices=[dict(index=0,message=dict(role='assistant',content=json.dumps(answer)),finish_reason='stop')],
            usage=dict(prompt_tokens=0,completion_tokens=0,total_tokens=0)))


def closure(conn, revision):
    return [r[0] for r in conn.execute('''WITH RECURSIVE ancestors(id) AS (
        SELECT ? UNION SELECT input FROM dependency JOIN ancestors ON output=ancestors.id)
        SELECT id FROM ancestors ORDER BY id''',(revision,))]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    binary=args.binary.resolve(strict=True)
    root=args.output.resolve()
    root.mkdir(parents=True,exist_ok=False)
    home=root/'isolated-home';home.mkdir()
    (home/'config.toml').write_text('not = [valid toml\n')
    holds=root/'holds';holds.mkdir()
    env=os.environ.copy()
    for key in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT'):
        env.pop(key,None)
    env.update(MECHA_HOME=str(home),MECHA_SESSION_KIND='test',MECHA_LOG='warn',RUST_LOG='off',
               HTTP_PROXY='http://127.0.0.1:1',HTTPS_PROXY='http://127.0.0.1:1')
    server=ThreadingHTTPServer(('127.0.0.1',0),ScriptedProvider)
    server.requests=[];server.choice='Nori'
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    endpoint=f'http://127.0.0.1:{server.server_port}'
    manifest=dict(protocol=PROTOCOL,input='synthetic',reply_source='authored scripted loopback HTTP server',
        new_real_model_inference_calls=0,frozen_model_artifact_used=False,
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=Path(__file__).resolve().parents[2],text=True).strip(),
        runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        source_sha256=hashlib.sha256(encode(frozen_source()).encode()).hexdigest(),
        cases=[dict(id=i,state=s,choice=c,expected_verification=v) for i,s,c,v in CASES])
    (root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    rows=[]
    try:
        for name,state,choice,verification in CASES:
            case=root/name;case.mkdir();db=case/'synthetic.memory-prototype.sqlite'
            def command(*argv, allow_failure=False):
                result=subprocess.run([str(binary),'memory','--experimental','--db',str(db),*argv],cwd=case,env=env,
                                      text=True,capture_output=True,timeout=30)
                with (root/'stderr.log').open('a') as log:log.write(result.stderr)
                if result.returncode:
                    if allow_failure:return None
                    raise RuntimeError(f'{name}: {argv[0]} exited {result.returncode}: {result.stderr[-1000:]}')
                return json.loads(result.stdout)
            def workflow(kind,*argv,**kwargs):
                return command(kind,'--synthetic-input','--domain','synthetic-demo','--scope','fixture-mail',*argv,**kwargs)
            row=dict(case=name,passed=False,new_real_model_inference_calls=0)
            before=len(server.requests)
            try:
                command('init-action','--synthetic-input','--domain','synthetic-demo')
                source=case/'measurements.json';source.write_text(encode(frozen_source()))
                provenance=case/'provenance.jsonl'
                provenance.write_text(encode(dict(record='meta',id='synthetic-action-'+name,created_at='2026-01-01T00:00:00Z'))+
                    encode(dict(record='message',role='user',content=[dict(type='text',text='Synthetic action prerequisite.')])) )
                imported=command('import-session',str(provenance),'--account','synthetic-action-trial','--synthetic-input')
                reference=imported['records'][-1]
                read=workflow('durable-read','start','--workspace',str(case),'--file','measurements.json',
                    '--source',reference['object']+':'+reference['revision'],'--max-bytes','4096','--max-lines','4')
                workflow('durable-read','resume','--task',read['status']['id'],'--workspace',str(case))
                contract=dict(workload=WORKLOAD,objective='minimize_minutes',
                    energy_budget=10 if name=='passed' else 5,minimum_quality=6)
                bound=workflow('durable-decide','bind','--read',read['status']['id'],'--contract',encode(contract),
                    '--model',MODEL,'--view','observed')
                model_id=bound['status']['id'];server.choice=choice
                model=workflow('durable-decide','run','--task',model_id,'--endpoint',endpoint,'--holds-dir',str(holds))
                (case/'model-report.json').write_text(json.dumps(model,indent=2)+'\n')
                row['model_disposition']=model['decision']['disposition']
                assert row['model_disposition']==('abstain' if choice=='abstain' else 'recommend')
                spec=encode(dict(fixture='measurements_v1',state=state))
                bound=workflow('durable-act','bind','--model',model_id,'--spec',spec,allow_failure=choice=='abstain')
                if choice=='abstain':
                    assert bound is None
                    with sqlite3.connect(db.as_uri()+'?mode=ro',uri=True) as conn:
                        assert conn.execute('SELECT count(*) FROM durable_action').fetchone()[0]==0
                    row.update(passed=True,action_reservations=0,retained_handler_observations=0)
                else:
                    action_id=bound['status']['id']
                    result=workflow('durable-act','run','--task',action_id)
                    assert result['status']['phase']=='finalized'
                    outcome=result['observation']['observation']['outcome']
                    feedback=result['feedback']
                    assert feedback['step_feedback']['verification']==verification
                    if state=='baseline':
                        assert {k:outcome[k] for k in ('observed_minutes','energy','quality')}==dict(observed_minutes=4,energy=9,quality=8)
                        assert feedback['resource_constraint_met']==(outcome['energy']<=contract['energy_budget'])
                        assert feedback['quality_constraint_met']==(outcome['quality']>=contract['minimum_quality'])
                        assert result['observation']['observation']['expected_from_evidence']['energy']==2
                    else:
                        assert feedback['observed'] is False
                        assert all(outcome.get(k) is None for k in ('observed_minutes','energy','quality'))
                        assert feedback['resource_constraint_met'] is None and feedback['quality_constraint_met'] is None
                    assert all(feedback['step_feedback'].get(k) is None for k in ('goal','expected_calls','actual_calls'))
                    source.unlink()
                    resumed=workflow('durable-act','run','--task',action_id)
                    assert resumed['observation']==result['observation'] and resumed['feedback']==feedback
                    recalled=command('recall','Nori','--limit','100','--neighbors','0','--max-bytes','100000')
                    observation_revision=result['status']['observation']['revision']
                    assert any(h['anchor']['reference']['revision']==observation_revision for h in recalled)
                    with sqlite3.connect(db.as_uri()+'?mode=ro',uri=True) as conn:
                        assert conn.execute('PRAGMA user_version').fetchone()[0]==5
                        ancestors=closure(conn,observation_revision)
                        expected_closure={reference['revision'],read['status']['task']['revision'],
                            model['status']['decision']['revision'],result['status']['spec']['revision']}
                        assert expected_closure.issubset(ancestors)
                        feedback_revision=result['status']['feedback']['revision']
                        feedback_closure=closure(conn,feedback_revision)
                        assert observation_revision in feedback_closure and reference['revision'] in feedback_closure
                        reservations=conn.execute('SELECT count(*) FROM durable_action').fetchone()[0]
                    row.update(passed=True,source_expected_energy=2,energy_budget=contract['energy_budget'],outcome=outcome,
                        verification=verification,action_reservations=reservations,retained_handler_observations=1,
                        recall_hit_revision=observation_revision,source_revision=reference['revision'],source_closure=ancestors,
                        source_closure_verified=True,final_reuse_equal=True,schema_version=5,
                        feedback_revision=feedback_revision,feedback_source_closure=feedback_closure,
                        count_basis='one finalized operational action row and its actual settled host observation; invocation counter is separate Rust process qualification')
                    (case/'report.json').write_text(json.dumps(dict(model=model,action=result,resume=resumed,recall=recalled),indent=2)+'\n')
            except Exception as error:
                row.update(passed=False,error=f'{type(error).__name__}: {error}')
            row['scripted_transport_requests']=len(server.requests)-before
            rows.append(row)
    finally:
        server.shutdown();server.server_close();thread.join(timeout=5)
    summary=dict(protocol=PROTOCOL,complete=all(r['passed'] for r in rows),cases_passed=sum(r['passed'] for r in rows),
        cases_failed=sum(not r['passed'] for r in rows),new_real_model_inference_calls=0,
        scripted_transport_requests=len(server.requests),rows=rows,
        limitations='Authored scripted replies measure persistence and closed host fixtures, never fresh model performance. '
            'Action reservations and retained observations are SQL audits; actual handler invocation/no-replay counts are measured by separate Rust child-process qualification. '
            'The runtime fixture is synthetic and deterministic, with no real-world execution or generalized skill learning. Exported reports are outside local erasure.')
    (root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps(summary,indent=2))
    if not summary['complete']:raise SystemExit(1)


if __name__=='__main__':main()
