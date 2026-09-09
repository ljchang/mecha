import collections, json
from pathlib import Path

import argparse
parser = argparse.ArgumentParser(description="Describe persisted appraisal exposure; never grade model prose.")
parser.add_argument("store", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
root = args.store
name = root.name
out = args.output
trials=[]
for path in sorted((root/'trials').glob('*.json')):
    trial=json.loads(path.read_text())
    if trial.get('status')!='done' or not trial.get('session_id'):
        continue
    session=root/'homes'/(trial.get('lifetime') or trial['arm'])/'sessions'/f"{trial['session_id']}.jsonl"
    records=[json.loads(line) for line in session.read_text().splitlines() if line.strip()]
    if any(r.get('record')=='rewrite' for r in records):
        trials.append({'trial':trial['id'],'coverage':'unknown: rewritten transcript'})
        continue
    configs=[r.get('config', r) for r in records if r.get('record')=='config']
    messages=[r.get('message',r) for r in records if r.get('record')=='message']
    accepted={b.get('tool_use_id') for m in messages for b in m.get('content',[])
              if b.get('type')=='tool_result' and b.get('is_error') is False}
    decisions=[d for m in messages for d in (m.get('planning') or {}).get('decisions',[])]
    steps=[s for m in messages for s in (m.get('planning') or {}).get('steps',[])]
    omissions=[]; prior={}; todo_calls=0; checks_named=0
    for m in messages:
        for b in m.get('content',[]):
            if b.get('type')!='tool_use' or b.get('name')!='todo' or b.get('id') not in accepted:
                continue
            todo_calls+=1
            items=b.get('input',{}).get('items',[])
            for item in items:
                key=item.get('content','').strip(); old=prior.get(key,{})
                if item.get('check'): checks_named+=1
                if item.get('status')=='completed' and old.get('status')!='completed' and old.get('check') and not item.get('check'):
                    observation=[s for s in steps if s.get('call_id')==b['id'] and s.get('step')==key]
                    omissions.append({'call_id':b['id'],'step':key,'recorded_verification':[s.get('verification') for s in observation]})
            prior={item.get('content','').strip():item for item in items}
    actions=collections.Counter(d.get('action','unknown') for d in decisions)
    trials.append({'trial':trial['id'],'arm':trial['arm'],'task':trial['task'],'seed':trial.get('seed'),
                   'rule_ids':configs[-1].get('rule_ids', []) if configs else None,
                   'rules_hash':configs[-1].get('rules_hash') if configs else None,
                   'position':trial.get('position'),
                   'accepted_todo_calls':todo_calls,'check_declarations_in_inputs':checks_named,
                   'decisions':len(decisions),'applied_decisions':sum(d.get('applied') is True for d in decisions),
                   'anchored_decisions':sum(d.get('anchor') is not None for d in decisions),
                   'actions':dict(actions),'verification_observations':dict(collections.Counter(s.get('verification','unknown') for s in steps)),
                   'check_omissions_on_completion':omissions,
                   'recorded_checks_executed':(trial.get('stats') or {}).get('checks_declared'),
                   'recorded_checks_passed':(trial.get('stats') or {}).get('checks_passed'),
                   'sensor_snapshots':[d.get('charter') for d in decisions[:1]]})
summary={}
for arm in sorted({r.get('arm') for r in trials if r.get('arm')}):
    rows=[r for r in trials if r.get('arm')==arm]
    summary[arm]={'trials_observed':len(rows),'trials_with_applied_guidance':sum(r['applied_decisions']>0 for r in rows),
                  'trials_with_rules':sum(bool(r.get('rule_ids')) for r in rows),
                  'trials_with_anchor':sum(r['anchored_decisions']>0 for r in rows),
                  'trials_with_executed_checks':sum((r['recorded_checks_executed'] or 0)>0 for r in rows),
                  'trials_with_check_omissions_on_completion':sum(bool(r['check_omissions_on_completion']) for r in rows),
                  'check_omissions_on_completion':sum(len(r['check_omissions_on_completion']) for r in rows),
                  'trials_with_unrestored_check_omissions':sum(any('not_declared' in o['recorded_verification'] for o in r['check_omissions_on_completion']) for r in rows),
                  'unrestored_check_omissions':sum('not_declared' in o['recorded_verification'] for r in rows for o in r['check_omissions_on_completion']),
                  'omissions_without_matching_feedback':sum(not o['recorded_verification'] for r in rows for o in r['check_omissions_on_completion']),
                  'actions':dict(sum((collections.Counter(r['actions']) for r in rows),collections.Counter()))}
result={'experiment':name,'summary':summary,'trials':trials,
        'method':'Count accepted todo inputs and persisted planning metadata in unrewritten transcripts. Omission compares adjacent accepted plans by exact trimmed step content; it is an observational diagnostic, not an artifact grade. Rewrites are marked unknown rather than counted twice.'}
out.write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(summary,indent=2))
