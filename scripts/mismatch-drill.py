#!/usr/bin/env python3
"""Controlled artifact-regression drill. Synthetic mismatch; real model arms.

Run from the checkout root with a prepared bootstrap config path. No owner
rules/transcripts are seeded or changed. This proves the gate/retirement
mechanism and is deliberately separate from the natural learning pilot.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root=Path(__file__).resolve().parents[1]
binary=root/'target/debug/mecha';seed=root/'target/debug/examples/mismatch_drill_seed'
home=Path(tempfile.mkdtemp(prefix='mecha-mismatch-drill-'))
(home/'MISMATCH_DRILL').write_text('controlled synthetic validation test\n')
shutil.copyfile(Path(sys.argv[1]),home/'config.toml')
ws=home/'workspace';ws.mkdir();(ws/'input.json').write_text('[2,3]\n')
prompt='Read input.json, add its numbers, and write exactly {"sum": the sum} to answer.json. Preserve input.json.'
case=dict(prompt=prompt,goal='task:artifact-drill',files={'input.json':'[2,3]\n'},artifacts={'answer.json':{'sum':5}},preserve=['input.json'])
(home/'case.json').write_text(json.dumps(case))
env={**os.environ,'MECHA_HOME':str(home),'MECHA_SESSION_KIND':'test'}
for key in ['MECHA_SESSION_DIR','MECHA_LEARNING_DIR']:env.pop(key,None)
flags=['--no-mcp','--no-hooks','--no-skills','--no-messages','--no-fallback','--no-step-escalation','--no-goal-guidance','--no-outbox']
flags += [item for name in ['fs_edit','fs_list','fs_read','fs_write','todo'] for item in ['--tool',name]]
def run(args,log):
    with (home/log).open('w') as out:subprocess.run(args,cwd=ws,env=env,stdout=out,stderr=subprocess.STDOUT,check=True)
print(f'drill home: {home}',flush=True)
run([str(binary),'run','--yes','--json','--no-stream','--no-learned-rules',*flags,'--goal','task:artifact-drill','--mismatch-case',str(home/'case.json'),prompt],'record.log')
assert json.loads((ws/'answer.json').read_text())=={'sum':5}
session=next((home/'sessions').glob('*.jsonl')).stem
run([str(seed),str(home),session],'seed.log')
for attempt in [1,2]:
    run([str(binary),'validate','--yes',*flags,'--trigger','mismatch'],f'validate-{attempt}.log')
    records=[json.loads(l) for l in (home/'learning/validations.jsonl').read_text().splitlines()]
    convicted=[r for r in records if r['outcome']=='regressed' and r.get('attributed_rule_id')=='artifact-drill-bad']
    assert len(convicted)==attempt, f'expected {attempt} attributed artifact regressions: {records}'
    run([str(binary),'rules','propose-retirements','--apply'],f'retire-{attempt}.log')
    run([str(binary),'rules','list','--json'],f'rules-{attempt}.json')
    rules={r['id']:r for r in json.loads((home/f'rules-{attempt}.json').read_text())}
    assert rules['artifact-drill-bad']['active']==(attempt==1)
    assert rules['artifact-drill-bystander']['active']
    print(f'pass {attempt}: artifact regression attributed; bad rule active={attempt==1}; bystander active',flush=True)
assert len(list((home/'learning/artifact-probes').glob('*.json')))>=4
print('ARTIFACT DRILL PASSED',flush=True)
