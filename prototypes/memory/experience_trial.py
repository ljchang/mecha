#!/usr/bin/env python3
"""Frozen synthetic experience workflow. Preserve JSONL progress even on interruption.

No live data, model switching, extraction pipeline, or external actions. Evidence
comes from host-observed fixture outcomes; scoring is separate from explanations.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from planning_trial import resident

ARMS = ('none', 'retrieved', 'oracle')
CASES = tuple(f'{w}-{c}' for w in ('kestrel', 'marlin', 'juniper', 'sable') for c in (1, 2, 3))


def specification(experiment):
    if experiment not in ('baseline', 'views', 'unseen', 'checked', 'observe', 'observe-reconsider'):
        raise ValueError('unknown experiment')
    names = ('orchid','tamarack','vireo','willow') if experiment == 'unseen' else ('kestrel','marlin','juniper','sable')
    if experiment == 'checked':
        names = ('aster', 'bramble', 'cinder', 'dahlia')
    if experiment in ('observe','observe-reconsider'):
        names = ('elm','fern','grove','heather')
    cases = tuple(f'{w}-{c}' for w in names for c in (1,2,3))
    groups = ('legacy','measurements','scoped') if experiment == 'views' else ARMS
    if experiment == 'checked':
        groups = ('baseline', 'checked')
    if experiment in ('observe','observe-reconsider'):
        groups = ('control','observe')
    protocol = {'observe-reconsider':'experience-observe-v3','observe':'experience-observe-v2','checked':'experience-checked-v1', 'baseline':'experience-workflow-v1', 'views':'experience-recall-views-v1', 'unseen':'experience-unseen-v1'}[experiment]
    return names, cases, groups, protocol


def summarize(rows, final, exit_code, model, experiment='baseline'):
    if experiment in ('observe','observe-reconsider'):
        from observation_summary import summarize as observations
        return observations(rows, final, exit_code, model, experiment)
    names, cases, groups, protocol = specification(experiment)
    def group(row):
        if experiment == 'checked':
            return row.get('policy')
        return row.get('view') if experiment == 'views' else row['arm']
    seen, drafts = set(), {}
    planned_rows = len(cases) * len(groups)
    for row in rows:
        key = (row['case'], group(row))
        if key in seen or key[0] not in cases or key[1] not in groups:
            raise ValueError('duplicate or unknown workflow row')
        if experiment == 'views' and row.get('arm') != 'retrieved':
            raise ValueError('view comparison requires retrieved evidence')
        if experiment == 'unseen' and row.get('view') != 'measurements':
            raise ValueError('unseen protocol freezes measurements view')
        if experiment == 'checked':
            if row.get('arm') != 'retrieved' or row.get('view') != 'measurements':
                raise ValueError('checked protocol freezes retrieved measurements')
            draft = (row.get('call'), row.get('request'))
            if row['case'] in drafts and drafts[row['case']] != draft:
                raise ValueError('paired first draft differs')
            drafts[row['case']] = draft
        seen.add(key)
    arms = {}
    for arm in groups:
        selected = [r for r in rows if group(r) == arm]
        valid = [r for r in selected if not r.get('error') and r.get('result') is not None
                 and (r.get('call', {}).get('reply') or {}).get('model') == model]
        calls = [r.get('call', {}) for r in selected]
        retries = [(r.get('review') or {}).get('retry') for r in selected]
        retries = [r for r in retries if r is not None]
        calls += retries
        usage = [c.get('reply') or {} for c in calls]
        withheld = lambda r: (r.get('review') or {}).get('withheld') is True
        count = dict(planned=12, recorded=len(selected), missing=12-len(selected),
                     host_valid=len(valid), failed=len(selected)-len(valid))
        for metric in ('optimal', 'evidence_complete', 'citations_complete', 'appropriate_abstention'):
            count[metric] = sum(r['result'].get(metric) is True and
                                (metric != 'appropriate_abstention' or not withheld(r)) for r in valid)
        if experiment == 'checked':
            count['host_withheld'] = sum(withheld(r) for r in valid)
            count['reconsiderations'] = len(retries)
        count['feasible'] = sum(r['result'].get('trace', {}).get('outcome', {}).get('feasible') is True for r in valid)
        count['grounded_optimal'] = sum(all(r['result'].get(m) is True for m in
                                            ('optimal', 'evidence_complete', 'citations_complete')) for r in valid)
        count['justified_decisions'] = count['grounded_optimal'] + count['appropriate_abstention']
        count['all_contexts_optimal_workloads'] = sum(
            all(any(r['case'] == f'{w}-{c}' and r['result']['optimal'] for r in valid) for c in (1, 2, 3))
            for w in names)
        for direction in ('input', 'output'):
            values = [r.get(direction+'_tokens') for r in usage]
            known = [v for v in values if type(v) is int and v >= 0]
            count[direction+'_tokens'] = sum(known) if known else None
            count[direction+'_usage_unknown'] = 12+len(retries)-len(known)
        count['elapsed_ms'] = sum(c.get('millis', 0) for c in calls)
        arms[arm] = count
    complete = (exit_code == 0 and len(seen) == planned_rows and isinstance(final, dict)
                and final.get('protocol') == protocol
                and final.get('readback_verified') is True and final.get('rows') == rows)
    summary = dict(protocol='experience-trial-v1', experiment=experiment, complete=complete, exit_code=exit_code, arms=arms,
                planned_calls=36, recorded_calls=len(rows),
                interpretation='Abstaining with no observations is appropriate uncertainty. Optimal counts tasks actually solved by the finite simulator. Grounded-optimal checks source/citation coverage plus host outcome, not free-text explanation entailment. Development fixtures, exact-name search, one seed, no generalization claim.')

    if experiment == 'checked':
        actual = [call for call, _ in drafts.values()]
        actual += [r['review']['retry'] for r in rows
                   if (r.get('review') or {}).get('retry') is not None]
        summary.update(planned_rows=24, planned_calls=12, max_calls=24,
                       recorded_calls=len(actual), recorded_initial_calls=len(drafts),
                       interpretation='Shared initial draft per case. One optional reconsideration; no transport retries. Host withholding is not model abstention or task success. Arithmetic over supplied synthetic observations does not certify explanation entailment or production actions. One seed, no generalization claim.')
        for direction in ('input', 'output'):
            values = [(c.get('reply') or {}).get(direction+'_tokens') for c in actual]
            known = [v for v in values if type(v) is int and v >= 0]
            summary['actual_'+direction+'_tokens'] = sum(known) if known else None
            summary['actual_'+direction+'_usage_unknown'] = 12-len(drafts)+len(actual)-len(known)
    return summary


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect(path):
    rows, final, errors = [], None, []
    for number, line in enumerate(path.read_bytes().splitlines(), 1):
        try:
            value = json.loads(line)
            if value.get('type') == 'row':
                rows.append(value['row'])
            elif value.get('type') == 'report' and final is None:
                final = value['report']
            else:
                raise ValueError('unexpected or duplicate envelope')
        except (ValueError, KeyError, AttributeError) as error:
            errors.append(f'line {number}: {error}')
    return rows, final, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--holds-dir', type=Path, required=True)
    parser.add_argument('--experiment', choices=('baseline','views','unseen','checked','observe','observe-reconsider'), default='baseline')
    args = parser.parse_args()
    names, cases, groups, protocol = specification(args.experiment)
    binary = args.binary.resolve(strict=True)
    holds = args.holds_dir.resolve(strict=True)
    endpoint = 'http://127.0.0.1:8080'
    resident(endpoint, args.model)
    args.output.mkdir(parents=True, exist_ok=False)
    root = args.output.resolve()
    home = root/'home'
    home.mkdir()
    (home/'config.toml').write_text('not = [valid toml')
    env = os.environ.copy()
    for key in ('OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT'):
        env.pop(key, None)
    env.update(MECHA_HOME=str(home), MECHA_SESSION_KIND='test', MECHA_LOG='warn', RUST_LOG='off')
    source_root = Path(__file__).resolve().parents[2]
    source = source_root/'mecha-core/src/memory/experience.rs'
    manifest = dict(protocol='experience-trial-v1', model=args.model, endpoint=endpoint,
                    seed=7, temperature=.6, thinking=False, max_tokens=2048, retries=0,
                    timeout_seconds=1200, planned_calls=12 if args.experiment == 'checked' else 36,
                    max_calls=24 if args.experiment == 'checked' else 60 if args.experiment == 'observe-reconsider' else 36,
                    planned_rows=len(cases)*len(groups), cases=cases, arms=groups, experiment=args.experiment, expected_protocol=protocol,
                    binary_sha256=digest(binary), harness_sha256=digest(Path(__file__)),
                    observation_source_sha256=digest(source.with_name('experience_observations.rs')), observation_summary_sha256=digest(Path(__file__).with_name('observation_summary.py')),
                    core_source_sha256=digest(source), check_source_sha256=digest(source.with_name('experience_checks.rs')), view_source_sha256=digest(source.with_name('experience_views.rs')),
                    source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source_root, text=True).strip())
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    argv = [str(binary), 'memory', '--experimental', '--db', str(root/'experience.memory-prototype.sqlite'),
            'workflow-trial', '--synthetic-input', '--endpoint', endpoint, '--model', args.model,
            '--holds-dir', str(holds), '--seed', '7']
    if args.experiment != 'baseline':
        argv.append({'views':'--recall-views', 'unseen':'--unseen', 'checked':'--checked', 'observe':'--observe','observe-reconsider':'--observe'}[args.experiment])
    if args.experiment == 'observe-reconsider':
        argv.append('--reconsider')
    started = time.monotonic()
    with (root/'progress.jsonl').open('w') as output, (root/'stderr.log').open('w') as errors:
        try:
            code = subprocess.run(argv, cwd=root, env=env, stdout=output, stderr=errors, timeout=1200).returncode
        except subprocess.TimeoutExpired:
            code = 124
    rows, final, decode_errors = collect(root/'progress.jsonl')
    summary = summarize(rows, final, code, args.model, args.experiment)
    summary.update(elapsed_seconds=time.monotonic()-started, decode_errors=decode_errors)
    if decode_errors:
        summary['complete'] = False
    try:
        resident(endpoint, args.model)
        summary['resident_after'] = True
    except Exception as error:
        summary.update(resident_after=False, resident_error=str(error), complete=False)
    (root/'rows.json').write_text(json.dumps(rows, indent=2)+'\n')
    if final is not None:
        (root/'report.json').write_text(json.dumps(final, indent=2)+'\n')
    (root/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary, indent=2))
    if not summary['complete']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
