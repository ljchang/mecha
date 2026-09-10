#!/usr/bin/env python3
"""Run a frozen, file-only pilot through `mecha exp`, then report paired task success.

The output directory contains a PRIVATE frozen rule/config snapshot. No stage
learns, changes a live proposal, or writes into the operator's stores.
"""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
NAME = 'executable-validation-v1'
MANIFEST = ROOT / 'eval/executable-validation.toml'
TOOLS = ['fs_edit', 'fs_list', 'fs_read', 'fs_write', 'todo']
INPUTS = [MANIFEST, ROOT/'eval/fixtures/executable-validation/cases.json',
          ROOT/'eval/fixtures/executable-validation/workspace/README.txt',
          ROOT/'eval/fixtures/executable_validation_source.py',
          ROOT/'eval/fixtures/appraisal_mismatch_source.py',
          ROOT/'eval/fixtures/appraisal_source.py', Path(__file__).resolve()]


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def props():
    with urllib.request.urlopen('http://127.0.0.1:8080/props', timeout=10) as response:
        d = json.load(response)
    return dict(model=d.get('model_alias'), slots=d.get('total_slots'),
                context=d.get('default_generation_settings', {}).get('n_ctx'))


def snapshot_inputs(binary, home):
    files = INPUTS + [binary, home/'config.toml'] + sorted((home/'learning/rules').glob('*.toml'))
    return {str(p): sha(p) for p in files}


def rule_exposure(config):
    """RunConfig hashes the empty block too; absent metadata is unknown."""
    value = config.get('rules_hash')
    if not isinstance(value, str) or not value:
        return None
    return value != 'cbf29ce484222325'  # learning::rules_hash("") / RulesCarried::none


def paired(rows, control, treatment):
    def index(arm):
        return {(r['task'], r.get('seed'), r['repetition']): r for r in rows if r['arm'] == arm}
    a, b = index(control), index(treatment)
    pairs = []
    for key in sorted(a.keys() & b.keys()):
        x, y = a[key], b[key]
        if any(r.get('status') != 'done' or not isinstance(r.get('passed'), bool) for r in (x, y)):
            continue
        # Task failures are never wins because their runs are cheaper.
        outcome = ('improved' if y['passed'] else 'regressed') if x['passed'] != y['passed'] else ('both_pass' if x['passed'] else 'both_fail')
        pairs.append(dict(task=key[0], seed=key[1], repetition=key[2], outcome=outcome,
                          control=x['id'], treatment=y['id']))
    both = [p for p in pairs if p['outcome'] == 'both_pass']
    deltas = {}
    for field in ('turns', 'tool_calls', 'duration_secs'):
        values = []
        for p in both:
            key = (p['task'], p['seed'], p['repetition'])
            x, y = a[key].get('stats') or {}, b[key].get('stats') or {}
            if isinstance(x.get(field), (int, float)) and isinstance(y.get(field), (int, float)):
                values.append(y[field]-x[field])
        deltas[field] = dict(n=len(values), treatment_minus_control=sum(values) if values else None)
    return dict(control=control, treatment=treatment, paired=len(pairs),
                missing_or_ungraded=len(a.keys() | b.keys())-len(pairs),
                outcomes=dict(Counter(p['outcome'] for p in pairs)),
                cost_on_both_pass= deltas, pairs=pairs)


def report(out):
    data = json.loads((out/'export.json').read_text())
    rows = data['trials']
    exp = out/'bootstrap/experiments'/NAME
    audit = []
    for row in rows:
        paths = list((exp/'homes').glob(f"*/sessions/{row.get('session_id')}.jsonl"))
        configs = []
        if len(paths) == 1:
            configs = [r for r in map(json.loads, paths[0].read_text().splitlines()) if r.get('record') == 'config']
        rules_on = '-on-' in row['arm']
        cap = int(row['arm'].rsplit('-', 1)[1])
        checks = dict(one_config=len(configs) == 1)
        if configs:
            c = configs[0]
            checks.update(tools=sorted(c.get('tools', [])) == TOOLS,
                          cap=c.get('max_turns') == cap,
                          seed=c.get('seed') == row.get('seed'),
                          rule_exposure=rule_exposure(c) is rules_on,
                          rule_lever=('learned_rules' not in c.get('levers_off', [])) == rules_on,
                          model=c.get('model') == data['manifest']['arms'][row['arm']]['model'])
        audit.append(dict(trial=row['id'], checks=checks, rules_hash=configs[0].get('rules_hash') if configs else None))
    conditions = json.loads((out/'conditions.json').read_text())
    finish = json.loads((out/'finish.json').read_text())
    valid = conditions['inputs'] == finish['inputs'] and conditions['server'] == finish['server'] and all(all(r['checks'].values()) for r in audit)
    expected = len(data['manifest']['arms']) * len(data['manifest']['tasks']['ids']) * len(data['manifest']['seeds']) * data['manifest']['repetitions']
    complete = len(rows) == expected and all(r.get('status') == 'done' for r in rows) and not data['unreadable_trials'] and not data['unreadable_stage_lines']
    arms = {}
    for arm in data['manifest']['arms']:
        own = [r for r in rows if r['arm'] == arm]
        stats = [r['stats'] for r in own if r.get('stats')]
        usage = [s.get('usage') or {} for s in stats]
        artifact_checks = [[c for c in r.get('checks', []) if c['name'].startswith(('artifact ', 'preserved '))] for r in own]
        arms[arm] = dict(n=len(own), passed=sum(r.get('passed') is True for r in own),
                         artifact_passed=sum(any(c['name'].startswith('artifact ') for c in cs) and all(c['passed'] for c in cs) for cs in artifact_checks),
                         ungraded=sum(not isinstance(r.get('passed'), bool) for r in own),
                         stops=dict(Counter(s.get('stop_cause', 'unknown') for s in stats)),
                         turns=sum(s.get('turns',0) for s in stats), calls=sum(s.get('tool_calls',0) for s in stats),
                         output_tokens=sum(u.get('output_tokens',0) for u in usage),
                         input_tokens=sum(u.get('input_tokens',0) for u in usage),
                         cached_input_tokens=sum(u.get('cache_read_input_tokens',0) for u in usage),
                         duration_secs=sum(s.get('duration_secs',0) for s in stats))
    comparisons = [paired(rows,a,b) for a,b in [('rules-on-12','rules-on-10'), ('rules-off-12','rules-off-10'), ('rules-off-12','rules-on-12'), ('rules-off-10','rules-on-10')]]
    per_task = {}
    for task in data['manifest']['tasks']['ids']:
        per_task[task] = {arm: dict(n=len(own), passed=sum(r.get('passed') is True for r in own),
            artifact_passed=sum(any(c['name'].startswith('artifact ') for c in r.get('checks', []))
                and all(c['passed'] for c in r.get('checks', []) if c['name'].startswith(('artifact ', 'preserved '))) for r in own),
            cap_hits=sum((r.get('stats') or {}).get('stop_cause') == 'max_turns' for r in own))
            for arm in arms for own in [[r for r in rows if r['task'] == task and r['arm'] == arm]]}
    result = dict(report_sha256=sha(Path(__file__).resolve()), per_task=per_task, valid=valid, complete=complete, expected_trials=expected, arms=arms, comparisons=comparisons, exposure_audit=audit,
                  proposal=conditions.get('proposal'),
                  promotion='disabled: synthetic corrective-task pilot; native judgements are descriptive',
                  historical_followups='not reconstructed: tasks start from registered incorrect artifacts',
                  grouping='three seeds per task are repeats, not three independent task families; no held-out task-family claim')
    write_json(out/'scorecard.json', result)
    lines = ['# Executable validation pilot', '', f'Conditions valid: **{valid}**. Complete: **{complete}** ({len(rows)}/{expected} trials).', '',
             '| Arm | Task passes | Calls | Turns | Output tokens | Run seconds |', '|---|---:|---:|---:|---:|---:|']
    for a,s in arms.items():
        lines.append(f"| {a} | {s['passed']}/{s['n']} | {s['calls']} | {s['turns']} | {s['output_tokens']} | {s['duration_secs']:.1f} |")
    lines += ['', 'Overall success includes the native run checks plus exact output artifacts and preserved inputs; artifact-only counts are also recorded in scorecard.json. Aggregate costs include failures; paired cost deltas in scorecard.json use only tasks both arms passed.', '', '| Comparison | Improved | Regressed | Both pass | Both fail |', '|---|---:|---:|---:|---:|']
    for c in comparisons:
        v=c['outcomes']; lines.append(f"| {c['control']} → {c['treatment']} | {v.get('improved',0)} | {v.get('regressed',0)} | {v.get('both_pass',0)} | {v.get('both_fail',0)} |")
    lines += ['', 'This pilot executes real file tools on reset workspaces. It does not reconstruct historical conversations, validate live service effects, train rules, or authorize a production override. Rules are frozen before execution; this measures their exposure, not the learning process.', '', 'Eight synthetic tasks covering seven patterns, three seeds each, fixed native arm order. Repeated seeds are not independent unseen tasks. Native selection/holdout judgements do not establish transfer to new task families. Latency includes shared-server conditions and is descriptive.', '', 'Full evidence: scorecard.json, export.json, conditions.json, finish.json, run.log and bootstrap/experiments/executable-validation-v1/. The bootstrap contains private operator rules/config; do not publish it.']
    (out/'README.md').write_text('\n'.join(lines)+'\n')
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/debug/mecha')
    parser.add_argument('--config', type=Path, default=Path.home()/'.mecha/config.toml')
    parser.add_argument('--rules', type=Path, default=Path.home()/'.mecha/learning/rules')
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--report-only', action='store_true', help='Regenerate analysis from an existing frozen export; execute no trials')
    args=parser.parse_args()
    out=args.out.resolve(); binary=args.binary.resolve()
    if args.report_only:
        result=report(out)
        print(out/'README.md')
        if not result['valid'] or not result['complete']: raise SystemExit('pilot incomplete or conditions invalid')
        return
    if out.exists(): raise ValueError('output already exists; a frozen experiment is never overwritten')
    import tomllib  # Pilot setup requires Python 3.11; report/oracle tests do not.
    cfg=tomllib.loads(args.config.read_text())
    if cfg.get('agent',{}).get('max_turns') != 12: raise ValueError('registered control requires configured max_turns=12')
    local=cfg.get('providers',{}).get('local',{})
    if local.get('base_url','').rstrip('/') != 'http://127.0.0.1:8080':
        raise ValueError('registered pilot requires local endpoint http://127.0.0.1:8080')
    server=props()
    if local.get('context_window') != server['context']:
        raise ValueError('provider context differs from the served per-slot context')
    if server['model'] != 'qwen3.6-35b-a3b': raise ValueError('server model differs from registered manifest')
    proposal=None
    if args.candidate:
        c=json.loads(args.candidate.read_text())
        if c['change'] != 'max_turns=10' or c['class'] != 'config': raise ValueError('candidate differs from registered treatment')
        proposal={k:c[k] for k in ('id','change','class','status')}
    out.mkdir(mode=0o700,parents=True)
    home=out/'bootstrap'; (home/'learning/rules').mkdir(parents=True)
    spec=importlib.util.spec_from_file_location('builder',ROOT/'eval/fixtures/build_executable_validation.py')
    builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)
    for provider in cfg.get('providers',{}).values(): provider.pop('api_key',None)
    cfg['default_provider']='local'
    # Freeze file-backed prompt content rather than leaving a mutable external input.
    agent=cfg.setdefault('agent',{})
    if agent.get('system_prompt_file'):
        agent['system_prompt']=Path(agent.pop('system_prompt_file')).read_text()
    (home/'config.toml').write_text('\n'.join(f'{k} = {builder.toml(v)}' for k,v in cfg.items())+'\n')
    for path in sorted(args.rules.glob('*.toml')): shutil.copyfile(path,home/'learning/rules'/path.name)
    if not list((home/'learning/rules').glob('*.learned.toml')): raise ValueError('no frozen learned rules')
    conditions=dict(at=datetime.now(timezone.utc).isoformat(),inputs=snapshot_inputs(binary,home),server=server,proposal=proposal,
                    runtime_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip())
    write_json(out/'conditions.json',conditions)
    shutil.copyfile(MANIFEST,out/'manifest.toml')
    env={k:v for k,v in os.environ.items() if not k.startswith('MECHA_')}
    env['MECHA_HOME']=str(home)
    def invoke(argv, log):
        with (out/log).open('w') as f: subprocess.run([str(binary),'exp',*argv],cwd=ROOT,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
    invoke(['new',str(MANIFEST)],'new.log')
    invoke(['run',NAME,'--dry-run'],'plan.log')
    invoke(['run',NAME],'run.log')
    invoke(['export',NAME],'export.json')
    write_json(out/'finish.json',dict(at=datetime.now(timezone.utc).isoformat(),inputs=snapshot_inputs(binary,home),server=props()))
    result=report(out)
    print(out/'README.md',flush=True)
    if not result['valid'] or not result['complete']: raise SystemExit('pilot incomplete or conditions invalid; inspect scorecard')


if __name__=='__main__': main()
