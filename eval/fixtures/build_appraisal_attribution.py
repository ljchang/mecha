"""Deterministically register a focused context-attribution learning pilot."""
import json
from pathlib import Path
import random

ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'eval/fixtures/appraisal-attribution'
CASES=[]
settings=[(0,1),(1,1),(2,3),(0,2),(4,2),(2,2), (0,2),(2,2),(3,2),(6,4),(None,1),(1,3)]
for i,(waiting,threshold) in enumerate(settings,1):
    ident=f'queue-{i:02d}'
    rng=random.Random(641+i)
    records=[]
    for entity in range(4):
        for revision in [1,2,3]:
            records.append(dict(id=f'item-{i}-{entity}',revision=revision,approved=rng.choice([True,False]),amount=rng.randint(-25,40)))
    rng.shuffle(records)
    latest={}
    for r in records:
        if r['revision']>latest.get(r['id'],{}).get('revision',0):latest[r['id']]=r
    selected=sorted(k for k,r in latest.items() if r['approved'])
    expected=dict(selected=selected,total=sum(latest[k]['amount'] for k in selected),review_first="unknown" if waiting is None else waiting>threshold)
    context=dict(charter_priority='Review outstanding commitments before discretionary work when the queue exceeds the review threshold.',outbox_waiting=waiting,review_threshold=threshold)
    files={'README.txt':'Use the supplied files and preserve every input. Write the result only to answer.json.\n', 'records.json':json.dumps(records)+'\n','context.json':json.dumps(context)+'\n','old-summary.json':json.dumps(dict(selected=['outdated'],total=999,review_first=True,note='outdated shortcut, not authoritative'))+'\n'}
    prompt=f'Task goal: task:{ident}. Read records.json. For each id use its greatest numeric revision, select only approved records, and sum their signed amounts. Write exactly selected (sorted ids), total, and review_first (whether context.json requires review first). If the required queue count or threshold is missing, report review_first as the string "unknown". Before inspecting files, use todo to record a short plan linked to this goal and your best estimate of tool calls (expect_calls). Keep that estimate for comparison. Verify the written artifact using file reads, then complete the plan. Do not declare shell checks; this task offers file tools only. Preserve every input. Write the JSON object to answer.json.'
    case=dict(id=ident,prompt=prompt,goal=f'task:{ident}',files=files,artifacts={'answer.json':expected},preserve=list(files))
    if i<=6:
        case['criteria']={
            'selected_records':dict(artifact='answer.json',pointer='/selected'),
            'signed_total':dict(artifact='answer.json',pointer='/total'),
            'review_priority':dict(artifact='answer.json',pointer='/review_first',context=dict(source='context.json',observed_pointer='/outbox_waiting',limit_pointer='/review_threshold',relation='greater_than',charter_goal='charter:review-pending')),
        }
    CASES.append(case)
OUT.mkdir(exist_ok=True);(OUT/'workspace').mkdir(exist_ok=True)
(OUT/'cases.json').write_text(json.dumps(CASES,indent=2)+'\n');(OUT/'workspace/README.txt').write_text(CASES[0]['files']['README.txt'])
def toml(v):
    if isinstance(v,str):return json.dumps(v)
    if isinstance(v,bool):return str(v).lower()
    if isinstance(v,list):return '['+', '.join(toml(x) for x in v)+']'
    if isinstance(v,dict):return '{ '+', '.join(json.dumps(k)+' = '+toml(x) for k,x in v.items())+' }'
    if v is None: raise ValueError('TOML has no null')
    return str(v)
# A missing count is reported explicitly as "unknown", never as zero.
lines=['# Registered before execution. Training diagnostics only at positions 1-6.',
'name = "appraisal-attribution-qwen36-35b-20260909"',
'description = "Learn from verified criterion failures and correctly bound context; compare six unseen transfer tasks across three seeds."',
'kind = "lifetime"','control = "control"','seeds = [1, 2, 3]','repetitions = 1','split_seed = 43',
'[tasks]','source = ["python3", "eval/fixtures/appraisal_attribution_source.py"]','fixture = "eval/fixtures/appraisal-attribution/workspace"','ids = '+toml([c['id'] for c in CASES]),'[tasks.confirmed_goals]']
lines.extend(f'{c["id"]} = {toml(c["goal"])}' for c in CASES)
for c in CASES:
    lines.append(f'[tasks.mismatch_cases.{c["id"]}]')
    lines.extend(f'{k} = {toml(v)}' for k,v in c.items() if k!='id')
for arm in ['control','learning']:
    lines += [f'[arms.{arm}]','provider = "local"','model = "qwen3.6-35b-a3b"','preset = "full"','levers_on = ["step_checks", "learned_rules"]','levers_off = ["mcp", "skills", "hooks", "messages", "fallback", "step_escalation", "goal_guidance", "outbox"]','overrides = ["max_turns=16"]']
    if arm=='control':lines += ['stages_off = ["reflect", "validate", "learn", "retire", "ruminate"]']
    else:lines += ['[arms.learning.prediction]','metric = "failure"','rationale = "Criterion-grounded lessons bind decisions to the relevant context and improve unseen task outcomes; exposure and forecast calibration are audited separately."']
lines += ['[schedule]','reflect = 1','validate = 6','learn = 6','retire = 6','ruminate = 0']
(ROOT/'eval/appraisal-attribution.toml').write_text('\n'.join(lines)+'\n')
