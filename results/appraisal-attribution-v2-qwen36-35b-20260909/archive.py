import collections, datetime, hashlib, json, pathlib, shutil, sys, tomllib, urllib.request
repo=pathlib.Path(__file__).resolve().parents[2]
root=pathlib.Path(sys.argv[1])
out=repo/'results'/root.name
sys.path.insert(0,str(repo/'eval/fixtures'))
from appraisal_mismatch_source import grade
cases={c['id']:c for c in json.loads((repo/'eval/fixtures/appraisal-attribution/cases.json').read_text())}
def save(name,data):
 p=out/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(data,indent=2)+'\n')
def lines(p):return [json.loads(l) for l in p.read_text().splitlines() if l.strip()] if p.exists() else []
trials=[json.loads(p.read_text()) for p in sorted((root/'trials').glob('*.json'))]
assert len(trials)==72 and all(t['status']=='done' for t in trials),'pilot incomplete'
feedback=[]; scores=[]; configurations=[]; resources=[]
for t in trials:
 w=root/'trials'/t['id']/'workspace';c=cases[t['task']]
 artifact=grade(c,w)
 a=w/'answer.json'
 if a.is_file() and not a.is_symlink():
  dest=out/'artifacts'/t['id']/'answer.json';dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(a,dest)
 records=lines(root/'homes'/t['lifetime']/'sessions'/(t['session_id']+'.jsonl'))
 save(pathlib.Path('transcripts')/(t['id']+'.json'),records)
 config=[r.get('config',r) for r in records if r.get('record')=='config'][-1]
 assert config['max_turns']==16 and config['model']=='qwen3.6-35b-a3b'
 assert set(config['tools'])=={'fs_edit','fs_list','fs_read','fs_write','todo'}
 assert bool(config['mismatch_case'].get('criteria')) == (t['position']<=6)
 configurations.append({'trial':t['id'],'max_turns':config['max_turns'],'tools_hash':config['tools_hash'],'rule_ids':config.get('rule_ids',[]),'rules_hash':config.get('rules_hash'),'seed':config.get('seed')})
 steps=[s for r in records if r.get('record')=='message' for s in (r.get('message',r).get('planning') or {}).get('steps',[])]
 assert t['position']<=6 or not any(s.get('criterion') for s in steps)
 for s in steps: feedback.append({'trial':t['id'],'arm':t['arm'],'seed':t['seed'],'position':t['position'],**s})
 scores.append({'trial':t['id'],'arm':t['arm'],'seed':t['seed'],'position':t['position'],'overall_pass':t['passed'],'artifact_pass':artifact['passed'],'checks':artifact['checks']})
 resources.append({'trial':t['id'],'arm':t['arm'],'seed':t['seed'],'stats':t['stats']})
summary={}
for arm in ['control','learning']:
 summary[arm]={}
 for seed in [None,1,2,3]:
  byseed={}
  for phase in ['initial','transfer','all']:
   rows=[r for r in scores if r['arm']==arm and (seed is None or r['seed']==seed) and (phase=='all' or (r['position']<=6)==(phase=='initial'))]
   byseed[phase]={'n':len(rows),'overall_pass':sum(r['overall_pass'] for r in rows),'artifact_pass':sum(r['artifact_pass'] for r in rows)}
  summary[arm]['all_seeds' if seed is None else str(seed)]=byseed
save('artifact-scores.json',summary);save('artifact-trials.json',scores);save('step-feedback.json',feedback);save('config-audit.json',configurations);save('resources.json',resources)
calibration={}
for arm in ['control','learning']:
 rows=[s for s in feedback if s['arm']==arm and not s.get('criterion')]
 measured=[s for s in rows if s.get('expected_calls') is not None and s.get('actual_calls') is not None]
 misses=[s for s in measured if s['actual_calls']>=max(s['expected_calls']*3,6)]
 criteria=[s for s in feedback if s['arm']==arm and s.get('criterion')]
 calibration[arm]={'step_observations':len(rows),'paired_call_estimates':len(measured),'missing_call_estimate_or_span':len(rows)-len(measured),'forecast_misses':len(misses),'misses_with_batched_completion':sum((s.get('completion_batch') or 0)>1 for s in misses),'mean_signed_call_error':sum(s['actual_calls']-s['expected_calls'] for s in measured)/len(measured) if measured else None,'mean_absolute_call_error':sum(abs(s['actual_calls']-s['expected_calls']) for s in measured)/len(measured) if measured else None,'criterion_verdicts':dict(collections.Counter(s['verification'] for s in criteria)),'criterion_failures_by_id':dict(collections.Counter(s['criterion']['id'] for s in criteria if s['verification']=='failed')),'note':'Step estimates and spans use varying step boundaries. These are descriptive observations, not comparable measures of wasted work or evidence of causal improvement.'}
save('calibration.json',calibration)

inventory={};roster={}
for home in sorted((root/'homes').iterdir()):
 src=home/'learning';dest=out/'learning-evidence'/home.name;hashes={}
 if src.exists():
  for p in sorted(src.rglob('*')):
   rel=p.relative_to(src)
   if not p.is_file() or '.git' in rel.parts or p.name.endswith('.lock') or p.name=='.gitignore':continue
   hashes[str(rel)]=hashlib.sha256(p.read_bytes()).hexdigest()
   q=dest/rel;q.parent.mkdir(parents=True,exist_ok=True)
   if p.suffix=='.jsonl':
    if p.stem.startswith('mined'):q.with_suffix('.txt').write_text(p.read_text())
    else:q.with_suffix('.json').write_text(json.dumps(lines(p),indent=2)+'\n')
   else:shutil.copyfile(p,q)
 refs=lines(src/'reflections.jsonl');valid=lines(src/'validations.jsonl')
 assert all(r.get('trigger')=='mismatch' for r in refs), 'unexpected non-mismatch reflection in the registered treatment'
 inventory[home.name]={'reflections':len(refs),'triggers':dict(collections.Counter(r.get('trigger','unknown') for r in refs)),'origins':dict(collections.Counter(r.get('origin','unknown') for r in refs)),'evidence':dict(collections.Counter(r.get('evidence','unknown') for r in refs)),'processed':sum(bool(r.get('is_processed')) for r in refs),'goals':sorted({g for r in refs for g in r.get('goals',[])}),'validation_records':len(valid),'validation_outcomes':dict(collections.Counter(str(r.get('outcome','unknown')) for r in valid)),'store_file_sha256':hashes}
 roster[home.name]={str(p.relative_to(src)):tomllib.loads(p.read_text()) for p in sorted((src/'rules').glob('*.toml'))}
save('learning-evidence.json',inventory);save('final-rules.json',roster)
for p in root.rglob('*.log'):
 if 'stages' in p.parts:
  dest=out/'stage-logs'/p.relative_to(root/'stages');dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(p,dest)
logs={'/tmp/mecha-attribution-v2-pilot.log':'runner.log','/tmp/mecha-attribution-v2-build.log':'verification/build.log','/tmp/mecha-attribution-v2-clippy.log':'verification/clippy.log','/tmp/mecha-attribution-v2-tests.log':'verification/tests.log','/tmp/mecha-attribution-v2-red.log':'verification/red-regression.log','/tmp/mecha-attribution-v2-retirement-drill.log':'verification/trace-retirement-drill.log'}
for p,name in logs.items():
 d=out/name;d.parent.mkdir(parents=True,exist_ok=True);d.write_text(pathlib.Path(p).read_text().rstrip()+'\n')
conditions=json.loads((out/'conditions.json').read_text())
hashes={p:hashlib.sha256((repo/p).read_bytes()).hexdigest() for p in conditions['sha256']}
assert hashes==conditions['sha256'],'frozen input changed'
props=json.load(urllib.request.urlopen('http://localhost:8080/props',timeout=15))
assert props['model_alias']=='qwen3.6-35b-a3b'
save('finish-audit.json',{'finished_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'frozen_sha256_unchanged':True,'sha256':hashes,'server_props':props,'trials':len(trials),'configuration_audits_passed':len(configurations),'transfer_criterion_feedback_records':sum(s['position']>6 and bool(s.get('criterion')) for s in feedback)})
print(json.dumps(summary,indent=2));print(json.dumps({k:{a:b for a,b in v.items() if a!='store_file_sha256'} for k,v in inventory.items()},indent=2))
