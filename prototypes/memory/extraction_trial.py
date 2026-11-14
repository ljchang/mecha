#!/usr/bin/env python3
"""Frozen synthetic extraction diagnostic. One tool-free CLI call per case.

Gold labels/rubrics grade outputs afterward; semantic fields remain ungraded until
independent source/artifact review. Matching structural status is not faithful
extraction. No actions execute and no live memory database is opened.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from planning_trial import resident


def model_input(case):
    return case['input']


def grade(value, exit_code, case, model, expected_protocol='source-linked-extraction-v2'):
    calls = value.get('calls', []) if isinstance(value, dict) else []
    valid_calls = isinstance(calls, list) and len(calls) == 1 and isinstance(calls[0], dict)
    extraction = value.get('extraction') if isinstance(value, dict) else None
    same_model = valid_calls and calls[0].get('model') == model
    valid = exit_code == 0 and same_model and isinstance(extraction, dict) and extraction.get('protocol') == expected_protocol
    if valid:
        coverage=extraction.get('coverage')
        notes=extraction.get('unresolved')
        valid=(isinstance(coverage,dict) and coverage.get('status') in
               ('covered','uncovered','unresolved','no_reachable_actions') and
               isinstance(notes,list) and all(isinstance(note,str) for note in notes))
    row = dict(case=case['id'], exit_code=exit_code, host_valid=valid, same_model=same_model,
               coverage_label_matches=False, faithful_extraction=None,
               recorded_calls=len(calls) if isinstance(calls,list) else None)
    for direction in ('input','output'):
        count=calls[0].get(direction+'_tokens') if valid_calls else None
        row[direction+'_tokens']=count if type(count) is int and count>=0 else None
    if valid:
        row['coverage']=extraction['coverage']['status']
        row['coverage_label_matches']=row['coverage']==case['expected_coverage']
        row['unresolved']=extraction['unresolved']
    else:
        row['error']=value.get('error','missing/invalid extraction or model mismatch') if isinstance(value,dict) else 'invalid JSON'
    return row


def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    p.add_argument('--model',required=True)
    p.add_argument('--holds-dir',type=Path,required=True)
    p.add_argument('--cases',type=Path,default=Path(__file__).parent/'fixtures/extraction_development.json')
    p.add_argument('--expected-protocol',choices=['source-linked-extraction-v1','source-linked-extraction-v2'],default='source-linked-extraction-v2')
    args=p.parse_args()
    binary=args.binary.resolve(strict=True); holds=args.holds_dir.resolve(strict=True)
    cases=json.loads(args.cases.read_text())
    ids=[c['id'] for c in cases]
    if len(set(ids))!=len(ids) or any(not s or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789-' for c in s) for s in ids):
        raise ValueError('invalid case IDs')
    endpoint='http://127.0.0.1:8080'
    resident(endpoint,args.model)
    args.output.mkdir(parents=True,exist_ok=False); root=args.output.resolve()
    home=root/'home'; home.mkdir()
    (home/'config.toml').write_text('not = [valid toml')
    env=os.environ.copy()
    for key in ['OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT']:
        env.pop(key,None)
    env.update(MECHA_HOME=str(home),MECHA_SESSION_KIND='test')
    manifest=dict(protocol='source-linked-extraction-trial-v2', expected_protocol=args.expected_protocol, model=args.model, seed=7,
                  endpoint=endpoint,temperature=.6,thinking=False,max_tokens=8192,retries=0,
                  binary_sha256=digest(binary),harness_sha256=digest(Path(__file__)),
                  cases_sha256=digest(args.cases),cases=cases,
                  limit='12 authored synthetic development cases, no holdout or matched V5 comparison. Semantic correctness requires independent review of original passages and raw output. Failed calls stay in denominator. Usage missing on transport failure/timeout remains null.')
    (root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    rows=[]; review=[]
    for case in cases:
        resident(endpoint,args.model)
        path=root/(case['id']+'-input.json')
        path.write_text(json.dumps(model_input(case),ensure_ascii=False,indent=2)+'\n')
        argv=[str(binary),'memory','--experimental','--db',str(root/'unused.memory-prototype.sqlite'),
              'extract-plan',str(path),'--synthetic-input','--endpoint',endpoint,'--model',args.model,
              '--holds-dir',str(holds),'--seed','7']
        start=time.monotonic()
        try:
            run=subprocess.run(argv,env=env,cwd=root,capture_output=True,text=True,timeout=210)
            stdout,stderr,code=run.stdout,run.stderr,run.returncode
        except subprocess.TimeoutExpired as exc:
            stdout=exc.stdout or b'';stderr=exc.stderr or b'';code=124
            if isinstance(stdout,bytes):stdout=stdout.decode(errors='replace')
            if isinstance(stderr,bytes):stderr=stderr.decode(errors='replace')
        (root/(case['id']+'-output.json')).write_text(stdout)
        (root/(case['id']+'-stderr.log')).write_text(stderr)
        try:value=json.loads(stdout)
        except ValueError:value=None
        row=grade(value,code,case,args.model,args.expected_protocol)
        row.update(elapsed_seconds=time.monotonic()-start,input_sha256=digest(path))
        rows.append(row)
        review.append(dict(case=case['id'],rubric=case['rubric'],actions_correct=None,
                           conditions_correct=None,requirements_correct=None,branches_correct=None,
                           citations_support_targets=None,faithful_extraction=None,reason=None))
        (root/'report.json').write_text(json.dumps(rows,indent=2)+'\n')
        (root/'review-template.json').write_text(json.dumps(review,indent=2)+'\n')
        print(json.dumps(row),flush=True)
    if (root/'unused.memory-prototype.sqlite').exists():
        raise RuntimeError('unexpected database creation')
    resident(endpoint,args.model)

if __name__=='__main__':main()
