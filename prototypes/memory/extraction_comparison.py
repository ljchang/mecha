#!/usr/bin/env python3
"""Four-arm synthetic development diagnostic, not a memory benchmark.

Same frozen evidence/plan inputs; representation and explicit reasoning setting
vary. Labels/rubrics enter only post-generation grading. No actions or live DB.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request
import urllib.parse
from extraction_trial import digest, grade, model_input
from planning_trial import resident

THINKING_BUDGET = 512
ARMS = ('extraction-fast', 'extraction-thinking', 'direct-fast', 'direct-thinking')


def arms_for_case(index):
    offset = index % len(ARMS)
    return ARMS[offset:] + ARMS[:offset]


def grade_arm(value, code, case, model, arm):
    direct = arm.startswith('direct-')
    thinking = arm.endswith('-thinking')
    v = value if isinstance(value, dict) else {}
    calls = v.get('calls')
    call = calls[0] if isinstance(calls, list) and len(calls) == 1 and isinstance(calls[0], dict) else {}
    transport = v.get('transport')
    transport_ok = isinstance(transport, dict) and transport.get('thinking') is thinking and transport.get('thinking_budget') == (THINKING_BUDGET if thinking else None)
    if direct:
        report = v.get('direct_judgment')
        valid = (code == 0 and bool(call) and call.get('model') == model and isinstance(report, dict)
                 and report.get('protocol') == 'direct-coverage-v1'
                 and report.get('status') in ('covered', 'uncovered', 'unresolved', 'no_reachable_actions')
                 and isinstance(report.get('reason'), str) and bool(report['reason'].strip())
                 and all(isinstance(report.get(k), list) and report[k] for k in ('evidence', 'plan')))
        row = dict(case=case['id'], exit_code=code, host_valid=bool(valid),
                   same_model=bool(call) and call.get('model') == model,
                   coverage_label_matches=False, source_supported_explanation=None,
                   recorded_calls=len(calls) if isinstance(calls, list) else None)
        if valid:
            row.update(coverage=report['status'], coverage_label_matches=report['status'] == case['expected_coverage'])
        else:
            row['error'] = v.get('error', 'invalid direct report/model/call')
        for key in ('input_tokens', 'output_tokens'):
            count = call.get(key)
            row[key] = count if type(count) is int and count >= 0 else None
    else:
        row = grade(value, code, case, model)
    if not transport_ok:
        row.update(host_valid=False, coverage_label_matches=False)
        row.setdefault('error', 'transport configuration missing or mismatched')
    count = call.get('reasoning_chars')
    row.update(arm=arm, thinking_requested=thinking,
               reasoning_chars=count if type(count) is int and count >= 0 else None,
               reasoning_observed=(count > 0) if type(count) is int and count >= 0 else None)
    return row


def trial_environment(home):
    env = os.environ.copy()
    for key in ['OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT']:
        env.pop(key, None)
    # OpenAI provider debug logging can contain reasoning-only response content.
    env.update(MECHA_HOME=str(home), MECHA_SESSION_KIND='test', MECHA_LOG='warn')
    return env


def configured_budget(models, model):
    entries = [m for m in models.get('data', []) if m.get('id') == model]
    if len(entries) != 1:
        raise ValueError('cannot identify resident model configuration')
    args = entries[0].get('status', {}).get('args', [])
    indices = [i for i, arg in enumerate(args) if arg == '--reasoning-budget']
    if len(indices) != 1 or indices[0]+1 >= len(args):
        raise ValueError('cannot verify configured reasoning budget')
    budget = int(args[indices[0]+1])
    if not 0 < budget < 8192:
        raise ValueError('reasoning budget must leave room within total output cap')
    return budget


def server_settings(endpoint, model):
    resident(endpoint, model)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(endpoint+'/models', timeout=10) as response:
        models = json.load(response)
    query = urllib.parse.urlencode({'model':model, 'autoload':'false'})
    with opener.open(endpoint+'/props?'+query, timeout=10) as response:
        props = json.load(response)
    if props.get('model_alias') != model:
        raise ValueError('resident props model differs')
    return {'models':models, 'props':props, 'configured_reasoning_budget':configured_budget(models, model)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--model', required=True)
    p.add_argument('--holds-dir', type=Path, required=True)
    p.add_argument('--cases', type=Path, default=Path(__file__).parent/'fixtures/extraction_development.json')
    args = p.parse_args()
    binary = args.binary.resolve(strict=True)
    holds = args.holds_dir.resolve(strict=True)
    cases = json.loads(args.cases.read_text())
    ids = [c['id'] for c in cases]
    if not cases or len(set(ids)) != len(ids) or any(not s or any(c not in 'abcdefghijklmnopqrstuvwxyz0123456789-' for c in s) for s in ids):
        raise ValueError('invalid case IDs')
    endpoint = 'http://127.0.0.1:8080'
    args.output.mkdir(parents=True, exist_ok=False)
    root = args.output.resolve()
    home = root/'home'; home.mkdir()
    (home/'config.toml').write_text('not = [valid toml')
    env = trial_environment(home)
    settings = None
    manifest = dict(protocol='extraction-comparison-v2', model=args.model,
                    arms=ARMS, seed=7, requested_thinking_budget=THINKING_BUDGET, temperature=.6, max_tokens=8192, retries=0,
                    endpoint=endpoint, request_timeout_seconds=180, log_level=env['MECHA_LOG'],
                    binary_sha256=digest(binary), harness_sha256=digest(Path(__file__)),
                    grading_harness_sha256=digest(Path(__file__).with_name('extraction_trial.py')),
                    cases_sha256=digest(args.cases), cases=cases,
                    limit='Reused synthetic development cases, one call per arm/case, no retries or holdout. Equal token caps are not equal compute. Reasoning presence/count is measured, never its content. Labels do not certify semantic correctness; source/structure review remains separate.')
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    rows = []
    progress = dict(state='running', planned=len(cases)*len(ARMS), recorded=0)
    def save_progress():
        (root/'progress.json').write_text(json.dumps(progress, indent=2)+'\n')
        (root/'report.json').write_text(json.dumps(rows, indent=2)+'\n')
    save_progress()
    try:
        settings = server_settings(endpoint, args.model)
        manifest['server_settings'] = settings
        (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    except Exception as exc:
        progress.update(state='aborted', error='initial preflight: '+str(exc))
        save_progress()
        raise
    for index, case in enumerate(cases):
        path = root/(case['id']+'-input.json')
        path.write_text(json.dumps(model_input(case), ensure_ascii=False, indent=2)+'\n')
        for arm in arms_for_case(index):
            try:
                current = server_settings(endpoint, args.model)
                if current['configured_reasoning_budget'] != settings['configured_reasoning_budget'] or current['props'] != settings['props']:
                    raise RuntimeError('server configuration changed during comparison')
            except Exception as exc:
                row = grade_arm({'error':'preflight: '+str(exc)}, 125, case, args.model, arm)
                row.update(sequence=len(rows)+1, input_sha256=digest(path), elapsed_seconds=0)
                rows.append(row)
                progress.update(state='aborted', recorded=len(rows), error=str(exc))
                save_progress()
                raise
            argv = [str(binary), 'memory', '--experimental', '--db', str(root/'unused.memory-prototype.sqlite'),
                    'extract-plan', str(path), '--synthetic-input', '--endpoint', endpoint,
                    '--model', args.model, '--holds-dir', str(holds), '--seed', '7']
            if arm.startswith('direct-'): argv.append('--direct')
            if arm.endswith('-thinking'): argv.extend(['--thinking', '--thinking-budget', str(THINKING_BUDGET)])
            start = time.monotonic()
            try:
                run = subprocess.run(argv, env=env, cwd=root, capture_output=True, text=True, timeout=210)
                stdout, stderr, code = run.stdout, run.stderr, run.returncode
            except subprocess.TimeoutExpired as exc:
                stdout, stderr, code = exc.stdout or b'', exc.stderr or b'', 124
                if isinstance(stdout, bytes): stdout = stdout.decode(errors='replace')
                if isinstance(stderr, bytes): stderr = stderr.decode(errors='replace')
            except OSError as exc:
                stdout, stderr, code = '', str(exc), 127
            prefix = root/(case['id']+'-'+arm)
            output = prefix.with_name(prefix.name+'-output.json')
            output.write_text(stdout)
            prefix.with_name(prefix.name+'-stderr.log').write_text(stderr)
            try: value = json.loads(stdout)
            except ValueError: value = None
            row = grade_arm(value, code, case, args.model, arm)
            row.update(elapsed_seconds=time.monotonic()-start, input_sha256=digest(path),
                       output_sha256=digest(output), sequence=len(rows)+1)
            rows.append(row)
            progress['recorded'] = len(rows)
            save_progress()
            print(json.dumps(row), flush=True)
    if (root/'unused.memory-prototype.sqlite').exists():
        raise RuntimeError('unexpected database creation')
    progress.update(state='complete', recorded=len(rows))
    save_progress()

if __name__ == '__main__': main()
