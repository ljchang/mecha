#!/usr/bin/env python3
"""Frozen read-only outcome-reappraisal development pilot (parent execution only).

Consumes one existing finalized synthetic action. Never acquires source files,
creates actions, retries inference, loads models or resumes an output directory.
The offline simulator rubric is deliberately separate from model-visible inputs.
"""
import argparse
from contextlib import closing
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import urllib.parse
import urllib.request

PROTOCOL = 'outcome-reappraisal-pilot-v1'
CORE_PROTOCOL = 'outcome-reappraisal-v1'
POLICY = 'same-workload-actual-over-expectation-v1'
WORKLOAD = 'durable-action-fixture-v1'
METHODS = ('Nori', 'Arden', 'Quill')
ARMS = ('source_only', 'source_and_outcome')
SEEDS = (7, 19)
MODEL_CONTROLS = dict(temperature=0.6, max_tokens=8192, thinking=False, max_retries=0,
                      local_only=True, structured_output='llama_json', timeout_seconds=180)
# Rubric only: these values are NEVER serialized into a CLI model request.
SOURCE = dict(Nori=(4, 2, 8), Arden=(8, 3, 9), Quill=(6, 5, 7))
ACTUAL = (4, 9, 8)
SIMULATOR = dict(Nori=ACTUAL, Arden=(8, 3, 9), Quill=(6, 5, 6))


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)+'\n'


def hash_text(text):
    return hashlib.sha256(text.encode()).hexdigest()


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON member')
        result[key] = value
    return result


def decode(text):
    def nonfinite(value):
        raise ValueError('non-finite JSON number: '+value)
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=nonfinite)


def frozen_cases():
    return [dict(id=name, diagnostic=diagnostic,
        contract=dict(workload=WORKLOAD, objective='minimize_minutes', energy_budget=budget, minimum_quality=quality))
        for name, budget, quality, diagnostic in (
            ('tight-energy', 4, 7, False), ('relaxed-energy', 10, 7, False),
            ('none-feasible', 2, 7, False), ('quality-bound', 4, 9, False),
            ('hidden-quality', 5, 7, True))]


def planned_pairs():
    return [dict(case=c['id'], seed=seed, outcome_first=bool((i+j) % 2))
            for j, seed in enumerate(SEEDS) for i, c in enumerate(frozen_cases())]


def expected(case, arm, simulator=False):
    if arm not in ARMS:
        raise ValueError('unplanned arm')
    rows = dict(SIMULATOR if simulator else SOURCE)
    if not simulator and arm == 'source_and_outcome':
        rows['Nori'] = ACTUAL
    contract = case['contract']
    feasible = {name: values for name, values in rows.items()
                if values[1] <= contract['energy_budget'] and values[2] >= contract['minimum_quality']}
    if not feasible:
        return set()
    best = min(v[0] for v in feasible.values())
    return {name for name, values in feasible.items() if values[0] == best}


def arm_request(prepared, arm):
    return next(a['request'] for a in prepared['arms'] if a['arm'] == arm)


def validate_preparation(prepared, case, action):
    """Fail before inference if this is not the frozen retained-mismatch pilot."""

    def require(condition):
        if not condition:
            raise ValueError('invalid frozen preparation')

    try:
        require(prepared['protocol'] == CORE_PROTOCOL and prepared['policy'] == POLICY)
        require(prepared['action_id'] == action and prepared['contract'] == case['contract'])
        require(prepared['private_data'] is True and prepared['untrusted_input'] is True
                and prepared['executed'] is False)
        require(set(prepared['snapshot']) == {'instance', 'sequence'})
        query = prepared['recall_query']
        require(query['known_at'] == prepared['snapshot'])
        require((query['text'], query['limit'], query['neighbors'], query['max_bytes']) == ('Nori', 100, 0, 100000))
        require(prepared['raw_recall'])
        for name in ('read', 'observation'):
            require(hash_text(prepared[name]['text']) == prepared[name]['sha256'])
        source = decode(prepared['read']['text'])
        require(source['kind'] == 'host_authored_synthetic_measurements_v1')
        require(source['workload'] == WORKLOAD and source['complete'] is True)
        measurements = source['observations']
        require(len(measurements) == 3)
        require({r['choice']: (r['observed_minutes'], r['energy'], r['quality']) for r in measurements} == SOURCE)
        observation = decode(prepared['observation']['text'])
        require(observation['kind'] == 'host_action_observation' and observation['action_id'] == action)
        require(observation['cancelled_action'] is False)
        actual = observation['observation']
        require((actual['kind'], actual['fixture'], actual['state'], actual['choice']) == (
            'host_observed_synthetic_action_v1', 'measurements_v1', 'baseline', 'Nori'))
        require(actual['outcome'] == dict(status='observed', observed_minutes=4, energy=9, quality=8))
        require(any(h['anchor']['reference'] == prepared['observation']['source'] for h in prepared['raw_recall']))
        require(len(prepared['arms']) == 2 and {a['arm'] for a in prepared['arms']} == set(ARMS))
        first = arm_request(prepared, ARMS[0])
        second = arm_request(prepared, ARMS[1])
        require(len(first['passages']) == 3 and len(second['passages']) == 4)
        require(first['passages'] == second['passages'][:3])
        require(second['passages'][3]['source'] == prepared['observation']['source'])
        for request in (first, second):
            require(isinstance(request['prompt'], str) and request['prompt'])
            ids = [p['id'] for p in request['passages']]
            require(len(set(ids)) == len(ids))
            require(set(request['response_schema']['properties']['citations']['items']['enum']) == set(ids))
    except (AttributeError, KeyError, TypeError, ValueError, StopIteration) as error:
        raise ValueError('invalid frozen preparation') from error


def score_row(case, row, prepared, model):
    """Grade raw first response independently; host disposition is a separate axis.

    Complete citations support the structured comparison, not explanation entailment.
    A free-form reason still needs independent review in the published analysis.
    """
    arm = row['arm']
    result = dict(valid=False, supported_optimal=False, appropriate_model_abstention=False,
                  citation_coverage=False, model_abstained=False, simulator_optimal=False, information_gap=False,
                  host_withheld=(row.get('decision') or {}).get('disposition') == 'withheld',
                  model_choice=None)
    try:
        validate_preparation(prepared, case, prepared['action_id'])
        call = row['call']; reply = call['reply']; request = arm_request(prepared, arm)
        answer = decode(reply['text'])
        ids = {p['id'] for p in request['passages']}
        valid = (row.get('error') is None and call.get('error') is None and reply.get('error') is None
                 and reply['model'] == model and call['prompt'] == request['prompt']
                 and call['response_schema'] == request['response_schema']
                 and isinstance(answer, dict) and set(answer) == {'choice', 'citations', 'reason'}
                 and answer['choice'] in (*METHODS, 'abstain')
                 and isinstance(answer['reason'], str) and 0 < len(answer['reason'].strip()) <= 1000
                 and isinstance(answer['citations'], list)
                 and all(isinstance(c, str) for c in answer['citations'])
                 and len(answer['citations']) == len(set(answer['citations']))
                 and set(answer['citations']).issubset(ids))
        if not valid:
            return result
        covered = set(answer['citations']) == ids
        visible = expected(case, arm); hidden = expected(case, arm, simulator=True)
        abstained = answer['choice'] == 'abstain'
        result.update(valid=True, model_choice=answer['choice'], citation_coverage=covered,
            supported_optimal=answer['choice'] in visible and covered,
            model_abstained=abstained, appropriate_model_abstention=abstained and not visible and covered,
            simulator_optimal=(answer['choice'] in hidden and covered) or (abstained and not hidden and covered),
            information_gap=case['diagnostic'] and answer['choice'] in visible and answer['choice'] not in hidden)
    except (KeyError, TypeError, ValueError, StopIteration):
        pass
    return result


def summarize(pairs, model):
    plans = {(p['case'], p['seed']): p for p in planned_pairs()}
    cases = {c['id']: c for c in frozen_cases()}
    seen = set(); matrix = []; analyzed = []
    for pair in pairs:
        key = (pair['case'], pair['seed'])
        if key in seen or key not in plans or pair['outcome_first'] != plans[key]['outcome_first']:
            raise ValueError('duplicate or unplanned pair')
        seen.add(key)
        report = pair.get('report')
        if report:
            prepared = pair['preparation']
            validate_preparation(prepared, cases[key[0]], prepared['action_id'])
            order = list(reversed(ARMS)) if pair['outcome_first'] else list(ARMS)
            if (report['protocol'] != CORE_PROTOCOL or report['preparation'] != prepared
                    or report['model'] != model or report['seed'] != pair['seed']
                    or report['model_controls'] != MODEL_CONTROLS or report['arm_order'] != order
                    or len(report['rows']) != 2 or [r['arm'] for r in report['rows']] != order
                    or report['executed'] is not False or report['private_data'] is not True
                    or report['untrusted_input'] is not True):
                raise ValueError('runtime pair audit mismatch')
            for row in report['rows']:
                score = score_row(cases[key[0]], row, prepared, model)
                call = row.get('call') or {}; reply = call.get('reply') or {}
                analyzed.append(dict(case=key[0], seed=key[1], arm=row['arm'], diagnostic=cases[key[0]]['diagnostic'],
                    score=score, disposition=(row.get('decision') or {}).get('disposition'),
                    input_tokens=reply.get('input_tokens'), output_tokens=reply.get('output_tokens'), millis=call.get('millis'),
                    error=row.get('error') or call.get('error') or reply.get('error')))
    for key, plan in plans.items():
        pair = next((p for p in pairs if (p['case'], p['seed']) == key), {})
        for arm in ARMS:
            value = next((r for r in analyzed if (r['case'], r['seed'], r['arm']) == (*key, arm)), None)
            matrix.append(dict(case=key[0], seed=key[1], arm=arm,
                status='recorded' if value else ('dispatch_unknown' if pair.get('started') else 'not_started'),
                result=value, pair_error=pair.get('error')))
    summary = dict(protocol=PROTOCOL, planned_calls=20, recorded_pairs=sum(bool(p.get('report')) for p in pairs),
        calls_recorded=len(analyzed), unknown_dispatch_slots=sum(r['status'] == 'dispatch_unknown' for r in matrix),
        matrix_complete=len(analyzed) == 20, complete=len(analyzed) == 20 and not any(p.get('error') for p in pairs),
        arms={}, primary={}, incomplete_knowledge_diagnostic={}, matrix=matrix,
        limitations='Five authored development cases and two seeds; no holdout, significance or generalization claim. '
            'Source-supported choice and hidden simulator optimum are separate. Citation coverage does not establish '
            'free-form explanation entailment. Equal call allowance does not imply equal token cost. '
            'Failed CLI pairs have unknown dispatch/usage, never fabricated zero calls. Exports are point-in-time private '
            'untrusted advisory artifacts outside local erasure; no action is executed.')
    for arm in ARMS:
        selected = [r for r in analyzed if r['arm'] == arm]
        totals = dict(recorded=len(selected), invalid=sum(not r['score']['valid'] for r in selected),
                      host_withheld=sum(r['score']['host_withheld'] for r in selected))
        for metric in ('supported_optimal', 'appropriate_model_abstention', 'model_abstained', 'simulator_optimal', 'information_gap'):
            totals[metric] = sum(r['score'][metric] for r in selected)
        for metric in ('input_tokens', 'output_tokens', 'millis'):
            values = [r[metric] for r in selected]
            known = [v for v in values if type(v) is int and v >= 0]
            totals[metric] = sum(known) if known else None
            totals[metric+'_unknown'] = len(values)-len(known)
        summary['arms'][arm] = totals
        for field, diagnostic in (('primary', False), ('incomplete_knowledge_diagnostic', True)):
            subset = [r for r in selected if r['diagnostic'] is diagnostic]
            summary[field][arm] = dict(planned=2 if diagnostic else 8, recorded=len(subset),
                evidence_appropriate=sum(r['score']['supported_optimal'] or r['score']['appropriate_model_abstention'] for r in subset),
                simulator_optimal=sum(r['score']['simulator_optimal'] for r in subset))
    return summary


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise ValueError('resident settings endpoint redirected')


def model_state(endpoint, model):
    url = urllib.parse.urlsplit(endpoint)
    if (url.scheme != 'http' or url.username or url.password or url.query or url.fragment
            or url.path not in ('', '/') or not ipaddress.ip_address(url.hostname).is_loopback):
        raise ValueError('plain numeric loopback endpoint required')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    def get(path):
        with opener.open(endpoint.rstrip('/')+path, timeout=15) as response:
            return decode(response.read().decode())
    models = get('/models')
    loaded = [r['id'] for r in models.get('data', []) if r.get('status', {}).get('value') in ('loaded', 'sleeping')]
    if loaded != [model]:
        raise ValueError('resident model changed or unavailable')
    props = get('/props?'+urllib.parse.urlencode(dict(model=model, autoload='false')))
    return dict(models=models, props=props)


def settings_identity(state):
    props = state['props']
    keys = ('default_generation_settings', 'model_path', 'model_alias', 'chat_template',
            'total_slots', 'modalities', 'build_info', 'n_ctx', 'n_batch', 'n_ubatch', 'n_gpu_layers')
    return {key: props[key] for key in keys if key in props}


def database_state(path):
    # mode=ro forbids data mutation; dump hashes include every table, including lifecycle rows.
    with closing(sqlite3.connect(path.as_uri()+'?mode=ro', uri=True)) as connection:
        digest = hashlib.sha256(); tables = {}
        for line in connection.iterdump():
            digest.update((line+'\n').encode())
        for (table,) in connection.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
            quoted = '"'+table.replace('"', '""')+'"'
            tables[table] = connection.execute('SELECT count(*) FROM '+quoted).fetchone()[0]
        version = connection.execute('PRAGMA user_version').fetchone()[0]
    return dict(sha256=sha(path), logical_sha256=digest.hexdigest(), schema_version=version, table_counts=tables)


def invoke_cli(argv, *, env, cwd, binary):
    result = subprocess.run([str(binary), *argv], env=env, cwd=cwd, text=True, capture_output=True, timeout=420)
    if result.returncode:
        # Invalidation must expose no new content; do not store a failed process's stdout/stderr as source audit.
        raise RuntimeError(f'CLI exited {result.returncode}; dispatch count unknown')
    return decode(result.stdout)


def write_json(path, value):
    # Atomic replacement preserves the last complete artifact on interruption.
    temporary = path.with_name('.'+path.name+'.tmp')
    data = json.dumps(value, indent=2, sort_keys=True, allow_nan=False)+'\n'
    with temporary.open('w') as stream:
        temporary.chmod(0o600)
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def run_trial(*, binary, db, action, endpoint, model, holds_dir, output, invoke=invoke_cli,
              inspect_model=model_state, inspect_database=database_state):
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    root = output.resolve(); binary = binary.resolve(strict=True); db = db.resolve(strict=True)
    holds_dir = holds_dir.resolve(strict=True)
    home = root/'isolated-home'; home.mkdir(mode=0o700)
    (home/'config.toml').write_text('not = [valid toml\n')
    env = os.environ.copy()
    for key in ('OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT'):
        env.pop(key, None)
    env.update(MECHA_HOME=str(home), MECHA_SESSION_KIND='test', MECHA_LOG='warn', RUST_LOG='off')
    manifest = dict(protocol=PROTOCOL, private_data=True, untrusted_input=True, executed=False,
        binary=str(binary), binary_sha256=sha(binary), runner_sha256=sha(Path(__file__)), db=str(db), action=action,
        model=model, endpoint=endpoint, seeds=list(SEEDS), planned_calls=20,
        model_controls=MODEL_CONTROLS, calls_per_arm=1,
        cases=frozen_cases(), offline_rubric=dict(source_expectations=SOURCE, actual_nori=ACTUAL, simulator=SIMULATOR),
        pairs=[], error=None)
    pairs = manifest['pairs']; error = None; before = None
    interruption = None; interruption_traceback = None
    write_json(root/'manifest.json', manifest)
    try:
        before = inspect_database(db); manifest['database_before'] = before
        state = inspect_model(endpoint, model); manifest['resident_before'] = state
        first = None
        for plan in planned_pairs():
            case = next(c for c in frozen_cases() if c['id'] == plan['case'])
            argv = ['memory', '--experimental', '--db', str(db), 'evaluate-outcome-recall',
                '--synthetic-input', '--domain', 'synthetic-demo', '--scope', 'fixture-mail',
                '--action', action, '--contract', encode(case['contract']), '--query', 'Nori',
                '--model', model, '--seed', str(plan['seed'])]
            if first is not None:
                argv += ['--snapshot', encode(first['snapshot'])]
            prepared = invoke(argv+['--prepare-only'], env=env, cwd=root, binary=binary)
            validate_preparation(prepared, case, action)
            if first is None:
                first = prepared
            elif any(prepared[key] != first[key] for key in ('snapshot', 'raw_recall', 'read', 'observation')):
                raise ValueError('preparation drift before inference')
            if '--snapshot' not in argv:
                argv += ['--snapshot', encode(first['snapshot'])]
            pairs.append(dict(**plan, argv=argv, preparation=prepared, report=None, error=None, started=False))
            write_json(root/'manifest.json', manifest)
        # Never rewrite this before-call freeze when progress/results arrive.
        write_json(root/'freeze.json', manifest)
        manifest['freeze_sha256'] = sha(root/'freeze.json')
        write_json(root/'manifest.json', manifest)
        # All ten full preparations and controls are durably exported before any model call.
        for pair in pairs:
            if inspect_database(db) != before:
                raise ValueError('database changed before model pair')
            current = inspect_model(endpoint, model)
            if settings_identity(current) != settings_identity(state):
                raise ValueError('resident model settings changed')
            pair['started'] = True
            progress = root/f"{pair['case']}-seed{pair['seed']}"
            progress.mkdir(mode=0o700)
            write_json(progress/'progress.json', dict(status='started', planned_arm_order=
                list(reversed(ARMS)) if pair['outcome_first'] else list(ARMS), dispatch_count=None))
            try:
                argv = pair['argv']+['--endpoint', endpoint, '--holds-dir', str(holds_dir)]
                if pair['outcome_first']:
                    argv.append('--outcome-first')
                report = invoke(argv, env=env, cwd=root, binary=binary)
                if report.get('preparation') != pair['preparation']:
                    raise ValueError('runtime preparation drift; pair excluded from scoring')
                candidate = dict(pair, report=report)
                summarize([candidate], model)  # Reject malformed/duplicate rows before storing scored calls.
                pair['report'] = report
                write_json(progress/'report.json', report)
                for row in report['rows']:
                    write_json(progress/(row['arm']+'.json'), row)
                write_json(progress/'progress.json', dict(status='recorded', reported_calls=len(report['rows'])))
            except BaseException as failure:
                pair['error'] = f'{type(failure).__name__}: {failure}'
                status = 'failed' if isinstance(failure, Exception) else 'interrupted'
                write_json(progress/'progress.json', dict(status=status, error=pair['error'], dispatch_count=None))
                raise
            write_json(root/'manifest.json', manifest)
        manifest['resident_after'] = inspect_model(endpoint, model)
        if settings_identity(manifest['resident_after']) != settings_identity(state):
            raise ValueError('resident model settings changed after pilot')
    except BaseException as failure:
        error = f'{type(failure).__name__}: {failure}'
        if not isinstance(failure, Exception):
            interruption = failure
            interruption_traceback = failure.__traceback__
    finally:
        if before is not None:
            try:
                manifest['database_after'] = inspect_database(db)
                manifest['database_unchanged'] = manifest['database_after'] == before
                if not manifest['database_unchanged']:
                    error = (error+'; ' if error else '')+'database changed during read-only evaluation'
            except BaseException as failure:
                error = (error+'; ' if error else '')+f'database audit failed: {type(failure).__name__}: {failure}'
                if interruption is None and not isinstance(failure, Exception):
                    interruption = failure
                    interruption_traceback = failure.__traceback__
        manifest['interrupted'] = interruption is not None
        manifest['error'] = error
        write_json(root/'manifest.json', manifest)
    summary = summarize(pairs, model)
    summary.update(error=error, interrupted=interruption is not None,
                   database_unchanged=manifest.get('database_unchanged'),
                   private_data=True, untrusted_input=True, executed=False)
    summary['complete'] &= error is None and manifest.get('database_unchanged') is True
    write_json(root/'summary.json', summary)
    if interruption is not None:
        # Persist the incomplete matrix first, then preserve operator/exit semantics.
        raise interruption.with_traceback(interruption_traceback)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'db', 'holds-dir', 'output'):
        parser.add_argument('--'+name, type=Path, required=True)
    for name in ('action', 'endpoint', 'model'):
        parser.add_argument('--'+name, required=True)
    summary = run_trial(**vars(parser.parse_args()))
    print(json.dumps(summary, indent=2))
    if not summary['complete']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
