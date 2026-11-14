#!/usr/bin/env python3
"""Frozen synthetic durable decision pilot; never resumes an existing output directory.

Parent execution only. The observations are host-authored fixture inputs, not actual
method executions. The runner reports initial evidence benefit separately from the
optional revision and never treats host withholding as model abstention.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

METHODS = ('Nori', 'Arden', 'Quill')
VIEWS = ('none', 'observed')
PROTOCOL = 'durable-model-pilot-v1'


def frozen_cases():
    """Eight valid sources; expected answers are independently computed below."""
    common = ((4, 9, 8), (8, 3, 9), (6, 5, 7))
    specifications = (
        ('objective-minutes', 'minimize_minutes', 10, 6, common),
        ('objective-energy', 'minimize_energy', 10, 6, common),
        ('energy-bound', 'minimize_minutes', 5, 6, common),
        ('quality-bound', 'minimize_minutes', 10, 9, common),
        ('missing-alternative', 'minimize_minutes', 10, 6, common[:2]),
        ('none-feasible', 'minimize_minutes', 1, 6, common),
        ('tied-optimum', 'minimize_minutes', 10, 6, ((4, 3, 8), (4, 5, 9), (8, 2, 7))),
        ('fresh-combination', 'minimize_energy', 8, 8, ((11, 4, 8), (3, 7, 9), (6, 2, 7))),
    )
    return [dict(id=name, contract=dict(workload='Synthetic '+name, objective=objective,
                                      energy_budget=energy, minimum_quality=quality),
                 source=dict(kind='host_authored_synthetic_measurements_v1',
                             workload='Synthetic '+name, complete=True,
                             observations=[dict(choice=method, observed_minutes=m,
                                                energy=e, quality=q)
                                           for method, (m, e, q) in zip(METHODS, rows)]))
            for name, objective, energy, quality, rows in specifications]


def expected(case, view):
    """No gold goes into a model request, source text or response schema."""
    rows = case['source']['observations'] if view == 'observed' else []
    if {row['choice'] for row in rows} != set(METHODS):
        return set()
    contract = case['contract']
    feasible = [r for r in rows if r['energy'] <= contract['energy_budget']
                and r['quality'] >= contract['minimum_quality']]
    if not feasible:
        return set()
    key = 'energy' if contract['objective'] == 'minimize_energy' else 'observed_minutes'
    optimum = min(r[key] for r in feasible)
    return {r['choice'] for r in feasible if r[key] == optimum}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))+'\n'


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON member')
        result[key] = value
    return result


def score(case, view, reply, allowed_citations):
    """Independently grade the structured choice; explanations are not graded."""
    gold = expected(case, view)
    try:
        answer = json.loads(reply['text'], object_pairs_hook=unique_object)
        valid = (reply.get('error') is None and set(answer) == {'choice', 'citations', 'reason'}
                 and answer['choice'] in (*METHODS, 'abstain')
                 and isinstance(answer['citations'], list)
                 and len(answer['citations']) <= 4
                 and isinstance(answer['reason'], str) and 0 < len(answer['reason'].strip()) <= 1000
                 and len(answer['citations']) == len(set(answer['citations']))
                 and all(c in allowed_citations for c in answer['citations']))
    except (ValueError, KeyError, TypeError):
        return dict(valid=False, supported_optimal=False, appropriate_abstention=False,
                    citation_coverage=False)
    if not valid:
        return dict(valid=False, supported_optimal=False, appropriate_abstention=False,
                    citation_coverage=False)
    cited = set(answer['citations'])
    complete = bool(allowed_citations) and cited == set(allowed_citations)
    return dict(valid=valid,
                supported_optimal=valid and answer['choice'] in gold and complete,
                appropriate_abstention=valid and answer['choice'] == 'abstain' and not gold,
                citation_coverage=valid and complete)


def citations(request):
    description = request['response_schema'].get('description', '')
    return json.loads(description.removeprefix('Allowed citation IDs: '))


def analyze_row(row, case, model):
    report = row.get('report') or {}
    result = dict(case=case['id'], view=row['view'], initial=None, final=None,
                  initial_reserved=False, revision_reserved=False, calls=[], error=row.get('error'),
                  disposition=(report.get('decision') or {}).get('disposition'))
    for slot, label in (('initial', 'initial'), ('retry', 'revision')):
        status = (report.get('status') or {}).get(slot) or {}
        result[label+'_reserved'] = status.get('phase', 'unused') != 'unused'
        attempt = report.get(slot)
        if not attempt or not attempt.get('result'):
            continue
        call = attempt['result']
        reply = call.get('reply') or {}
        valid_model = reply.get('model') == model and not call.get('error')
        graded = score(case, row['view'], reply, citations(attempt['request']))
        if not valid_model:
            graded = dict.fromkeys(graded, False)
        result['calls'].append(dict(slot=slot, millis=call.get('millis'),
                                    input_tokens=reply.get('input_tokens'), output_tokens=reply.get('output_tokens'),
                                    transport_error=call.get('error'), generation_error=reply.get('error')))
        if slot == 'initial':
            result['initial'] = graded
        result['final'] = graded
    return result


def summarize(rows, model):
    cases = {c['id']: c for c in frozen_cases()}
    seen = set()
    analyzed = []
    for row in rows:
        key = (row['case'], row['view'])
        if key in seen or key[0] not in cases or key[1] not in VIEWS:
            raise ValueError('duplicate or unplanned row')
        seen.add(key)
        analyzed.append(analyze_row(row, cases[key[0]], model))
    summary = dict(protocol=PROTOCOL, planned_initial_calls=16, max_revision_calls=16,
                   recorded_rows=len(rows), initial_reserved=sum(r['initial_reserved'] for r in analyzed),
                   revision_reserved=sum(r['revision_reserved'] for r in analyzed), arms={})
    for view in VIEWS:
        selected = [r for r in analyzed if r['view'] == view]
        initial = [r['initial'] for r in selected if r['initial'] is not None]
        final = [r['final'] for r in selected if r['final'] is not None]
        counts = dict(planned=8, recorded=len(selected), failed=sum(r['error'] is not None or r['disposition'] == 'failed' for r in selected),
                      initial_results=len(initial), revision_calls=sum(r['revision_reserved'] for r in selected),
                      model_abstained=sum(r['disposition'] == 'abstain' for r in selected),
                      host_withheld=sum(r['disposition'] == 'withheld' for r in selected),
                      recommendations=sum(r['disposition'] == 'recommend' for r in selected))
        for metric in ('supported_optimal', 'appropriate_abstention', 'citation_coverage'):
            counts['initial_'+metric] = sum(r[metric] for r in initial)
            counts['final_'+metric] = sum(r[metric] for r in final)
        for slot in ('initial', 'retry'):
            calls = [c for r in selected for c in r['calls'] if c['slot'] == slot]
            counts[slot+'_settled_calls'] = len(calls)
            for metric in ('input_tokens', 'output_tokens', 'millis'):
                values = [c[metric] for c in calls]
                known = [v for v in values if type(v) is int and v >= 0]
                counts[slot+'_'+metric] = sum(known) if known else None
                counts[slot+'_'+metric+'_unknown'] = len(values)-len(known)
        summary['arms'][view] = counts
    pair_audit = True
    for case in cases.values():
        pair = [r for r in rows if r['case'] == case['id']]
        schemas = []
        for row in pair:
            report = row.get('report') or {}
            spec = report.get('spec') or {}
            request = (report.get('initial') or {}).get('request') or {}
            schema = dict(request.get('response_schema') or {})
            schema.pop('description', None)  # Visible citation IDs differ by intervention.
            schemas.append(schema)
            pair_audit &= (spec == dict(contract=case['contract'], model=model, seed=7, view=row['view'])
                           and request.get('model') == model and request.get('seed') == 7
                           and (report.get('decision') or {}).get('executed') is False)
        pair_audit &= len(pair) == 2 and schemas[0] == schemas[1] and bool(schemas[0])
    summary['paired_contract_model_schema_verified'] = bool(pair_audit)
    summary['complete'] = (len(seen) == 16 and summary['initial_reserved'] == 16 and pair_audit
                           and summary['revision_reserved'] <= 16
                           and all(r['error'] is None and r['disposition'] in ('recommend', 'abstain', 'withheld', 'failed') for r in analyzed))
    summary['limitations'] = ('Eight authored cases, one seed, no significance claim. Source observations are supplied fixture data; '
                              'this measures conditional use of retained observations, not search ranking, real outcome accuracy, or executed methods. '
                              'No-memory appropriate abstention is separate from a supported optimal recommendation. '
                              'Optional revision compute and failed/unknown call usage are reported separately. Exported reports are outside local database erasure.')
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--holds-dir', type=Path, required=True)
    args = parser.parse_args()
    from planning_trial import resident
    endpoint = 'http://127.0.0.1:8080'
    binary = args.binary.resolve(strict=True)
    holds = args.holds_dir.resolve(strict=True)
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
    cases = frozen_cases()
    # Every source and contract is frozen before either model arm can write an outcome.
    for case in cases:
        directory = root/case['id']
        directory.mkdir()
        (directory/'measurements.json').write_text(encode(case['source']))
        (directory/'contract.json').write_text(encode(case['contract']))
        provenance = [dict(record='meta', id='synthetic-'+case['id'], created_at='2026-01-01T00:00:00Z'),
                      dict(record='message', role='user', content=[dict(type='text', text='Synthetic fixture provenance.')])]
        (directory/'provenance.jsonl').write_text(''.join(encode(r) for r in provenance))
    manifest = dict(protocol=PROTOCOL, model=args.model, endpoint=endpoint, seed=7, temperature=.6,
                    thinking=False, max_tokens=8192, retries=0, planned_initial_calls=16, max_revision_calls=16,
                    source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=Path(__file__).resolve().parents[2], text=True).strip(),
                    binary_sha256=sha(binary), runner_sha256=sha(Path(__file__)),
                    files={str(p.relative_to(root)):sha(p) for p in sorted(root.glob('*/*.json*'))},
                    order=[dict(case=c['id'], view=v) for i,c in enumerate(cases) for v in (VIEWS if i%2 == 0 else VIEWS[::-1])])
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    rows = []
    def command(db, *argv):
        result = subprocess.run([str(binary), 'memory', '--experimental', '--db', str(db), *argv],
                                cwd=root, env=env, text=True, capture_output=True, timeout=600)
        with (root/'stderr.log').open('a') as log:
            log.write(result.stderr)
        if result.returncode:
            raise RuntimeError(f'CLI command {argv[0]} exited {result.returncode}')
        return json.loads(result.stdout, object_pairs_hook=unique_object)
    domain, scope = 'synthetic-demo', 'fixture-mail'
    for item in manifest['order']:
        directory = root/item['case']
        view = item['view']
        db = directory/(view+'.memory-prototype.sqlite')
        row = dict(item, report=None, error=None)
        task_id = None
        started = time.monotonic()
        try:
            command(db, 'init-model', '--domain', domain)
            imported = command(db, 'import-session', str(directory/'provenance.jsonl'), '--account', 'synthetic-pilot', '--synthetic-input')
            reference = imported['records'][-1]
            read = command(db, 'durable-read', '--synthetic-input', '--domain', domain, '--scope', scope,
                           'start', '--workspace', str(directory), '--file', 'measurements.json',
                           '--source', reference['object']+':'+reference['revision'], '--max-lines', '4', '--max-bytes', '4096')
            read_id = read['status']['id']
            command(db, 'durable-read', '--synthetic-input', '--domain', domain, '--scope', scope,
                    'resume', '--task', read_id, '--workspace', str(directory))
            bound = command(db, 'durable-decide', '--synthetic-input', '--domain', domain, '--scope', scope,
                            'bind', '--read', read_id, '--contract', (directory/'contract.json').read_text(),
                            '--model', args.model, '--seed', '7', '--view', view)
            task_id = bound['status']['id']
            row['report'] = command(db, 'durable-decide', '--synthetic-input', '--domain', domain, '--scope', scope,
                                    'run', '--task', task_id, '--endpoint', endpoint, '--holds-dir', str(holds))
        except (ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
            row['error'] = str(error)
            if task_id is not None:
                try:
                    row['report'] = command(db, 'durable-decide', '--synthetic-input', '--domain', domain, '--scope', scope, 'status', '--task', task_id)
                except Exception as status_error:
                    row['status_error'] = str(status_error)
        row['elapsed_seconds'] = time.monotonic()-started
        rows.append(row)
        with (root/'progress.jsonl').open('a') as output:
            output.write(encode(row))
    summary = summarize(rows, args.model)
    summary['frozen_sources_verified'] = all(sha(root/name) == digest for name,digest in manifest['files'].items())
    summary['complete'] &= summary['frozen_sources_verified']
    try:
        resident(endpoint, args.model)
        summary['resident_after'] = True
    except Exception as error:
        summary.update(resident_after=False, resident_error=str(error), complete=False)
    (root/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    print(json.dumps(summary, indent=2))
    if not summary['complete']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
