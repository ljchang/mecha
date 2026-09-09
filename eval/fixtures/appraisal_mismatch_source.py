#!/usr/bin/env python3
"""Owner-authored file-only tasks. Gold is outside every task workspace."""
import json
import os
from pathlib import Path
import sys
from appraisal_source import paths, read_regular, strict_json, equal, write_file

CASES = json.loads((Path(__file__).resolve().parent/'appraisal-mismatch/cases.json').read_text())

def tasks():
    return [dict(id=c['id'],prompt=c['prompt'],tags=['appraisal','mismatch',c['id'].split('-')[0]],max_turns=24,expect={'tools':['todo']}) for c in CASES]

def grade(c,workspace):
    checks=[]
    for name,expected in c['artifacts'].items():
        try: passed=equal(strict_json(read_regular(workspace/name)),expected)
        except (OSError,ValueError): passed=False
        checks.append(dict(name=f'artifact {name}',passed=passed,detail=''))
    for name in c['preserve']:
        try: passed=read_regular(workspace/name)==c['files'][name]
        except (OSError,ValueError): passed=False
        checks.append(dict(name=f'preserved {name}',passed=passed,detail=''))
    return dict(passed=all(x['passed'] for x in checks),checks=checks,detail='Independent exact JSON and preserved-input checks.')

def main():
    if sys.argv[1]=='list': json.dump(tasks(),sys.stdout);return
    c=next(c for c in CASES if c['id']==sys.argv[2])
    if os.environ.get('MECHA_EXPERIMENT_TASK')!=c['id']:raise ValueError('task mismatch')
    _,workspace=paths()
    if sys.argv[1]=='setup':
        for name,text in c['files'].items():write_file(workspace/name,text)
    elif sys.argv[1]=='grade':json.load(sys.stdin);json.dump(grade(c,workspace),sys.stdout)
    else:raise ValueError('unknown source verb')

if __name__=='__main__':main()
