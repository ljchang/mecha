#!/usr/bin/env python3
"""Conservative-revision replay of the frozen six development cases. No proposed actions execute.

Expected labels never enter evaluator input. Status/reference checks are limited
host diagnostics; an independent human must grade semantic support and revisions
using review.json, rather than treating the evaluator's self-score as improvement.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from planning_trial import resident

from assessment_calibration import CASES, EVENT


def validate_report(report, protocol, revision=False):
    if report['evaluation_protocol'] != protocol or report['citation_protocol'] != 'host-passages-v1':
        raise ValueError('assessment protocol mismatch')
    candidates = report['candidates']
    ids = [c['id'] for c in candidates]
    if not ids or len(ids) != len(set(ids)) or (revision and ids != ['revision']):
        raise ValueError('invalid candidate identities')
    for candidate in candidates:
        if not candidate['steps'] or not all(isinstance(s,str) and s for s in candidate['steps']):
            raise ValueError('invalid candidate steps')
    assessments = report['assessments']
    if sorted(a['id'] for a in assessments) != sorted(ids):
        raise ValueError('assessment identities differ from candidates')
    dimensions = {'prerequisites','factual_support','concept_applicability'}
    if protocol in ('requirements-assessment-v3','requirements-assessment-v4','bound-repair-v5'):
        dimensions.add('outcome')
    for assessment in assessments:
        checks = assessment['checks']
        if len(checks) != len(dimensions) or {c['dimension'] for c in checks} != dimensions:
            raise ValueError('incomplete assessment dimensions')
        if assessment['disposition'] not in ('consider','revise','needs_evidence'):
            raise ValueError('invalid disposition')
        if any(c['status'] not in ('supported','contradicted','unknown','not_applicable') for c in checks):
            raise ValueError('invalid check status')
        if protocol in ('requirements-assessment-v3','requirements-assessment-v4','bound-repair-v5'):
            if not isinstance(assessment['requirements'], list) or assessment['requirement_status'] not in ('supported','contradicted','unknown'):
                raise ValueError('invalid V3 requirements')
    if report['recommended'] is not None and not any(a['id'] == report['recommended'] and a['disposition'] == 'consider' for a in assessments):
        raise ValueError('recommendation absent or not eligible')


def validate_repair(evaluation):
    repair=evaluation['repair']
    revision=evaluation['revision']
    if revision is None:
        if repair is not None: raise ValueError('repair without reassessment')
        return
    initial=evaluation['initial']
    original=next(c for c in initial['candidates'] if c['id']==repair['candidate'])
    assessment=next(a for a in initial['assessments'] if a['id']==repair['candidate'])
    steps=repair['steps']
    if len(steps)!=len(original['steps']) or len(steps)!=len(revision['candidates'][0]['steps']):
        raise ValueError('repair step count mismatch')
    for i,step in enumerate(steps):
        if step['step']!=i+1 or step['original']!=original['steps'][i] or step['rendered']!=revision['candidates'][0]['steps'][i]:
            raise ValueError('repair source/rendered step mismatch')
        if step['operation'] not in ('keep','gate','defer'): raise ValueError('invalid repair operation')
        required=[r['id'] for r in assessment['requirements'] if r['step']==i+1]
        if step['requirement_ids']!=required: raise ValueError('repair dropped requirement bindings')
        if step['operation']=='keep' and step['rendered']!=step['original']: raise ValueError('keep changed step')
    if all(s['operation']=='keep' for s in steps): raise ValueError('unchanged repair')
    deferred=all(s['operation']=='defer' for s in steps)
    if type(repair['all_deferred']) is not bool or repair['all_deferred']!=deferred: raise ValueError('invalid abstention flag')
    if deferred and revision['recommended'] is not None: raise ValueError('abstention counted as recommended revision')


def grade(value, case, references, exit_code, expected_protocol="constraint-assessment-v2"):
    """Keep failure rows and denominators; never use model self-rating as gain."""
    calls = value.get('calls', []) if isinstance(value, dict) else []
    valid_calls = isinstance(calls,list) and all(isinstance(c,dict) and all(c.get(d+'_tokens') is None or (type(c.get(d+'_tokens')) is int and c[d+'_tokens'] >= 0) for d in ('input','output')) for c in calls)
    if not valid_calls:
        return dict(case=case['id'],exit_code=exit_code,completed=False,prerequisite_correct=False,error='invalid call audit',recorded_calls=None)
    row = dict(case=case['id'], exit_code=exit_code, completed=False,
               prerequisite_correct=False, original_evidence_cited=False,
               recorded_calls=len(calls))
    for direction in ['input', 'output']:
        counts = [c.get(direction+'_tokens') for c in calls]
        known = [n for n in counts if n is not None]
        row[direction+'_tokens'] = sum(known) if counts and len(known) == len(counts) else None
        row['known_'+direction+'_tokens'] = sum(known)
        row[direction+'_usage_calls'] = len(known)
    if exit_code or not isinstance(value, dict) or not value.get('evaluation'):
        row['error'] = value.get('error', 'no completed evaluation') if isinstance(value, dict) else 'invalid JSON'
        return row
    try:
        evaluation = value['evaluation']
        initial = evaluation['initial']
        revision = evaluation['revision']
        revision_status = evaluation['revision_status']
        validate_report(initial, expected_protocol)
        if revision_status not in ('not_requested', 'kept_existing', 'reassessed'):
            raise ValueError('unknown revision status')
        if (revision_status == 'reassessed') != (revision is not None):
            raise ValueError('revision/status mismatch')
        if revision_status == 'kept_existing' and initial['recommended'] is None:
            raise ValueError('kept_existing without recommendation')
        if revision_status == 'reassessed' and initial['recommended'] is not None:
            raise ValueError('revised an already recommended plan')
        if revision is not None:
            validate_report(revision, expected_protocol, revision=True)
        if initial['recommended'] is not None and initial['recommended'] not in [c['id'] for c in initial['candidates']]:
            raise ValueError('recommendation absent from candidates')
        if expected_protocol == 'bound-repair-v5': validate_repair(evaluation)
        check = next(c for c in initial['assessments'][0]['checks'] if c['dimension'] == 'prerequisites')
        row.update(completed=True, prerequisite=check['status'], expected_prerequisite=case['prerequisite'],
                   prerequisite_correct=check['status'] == case['prerequisite'],
                   original_evidence_cited=any(c['source_id'] == references['evidence']['revision'] for c in check['citations']),
                   concept_retrieved=any(h['record']['kind'] == 'concept' for h in initial['context']['records']),
                   initial_recommended=initial['recommended'], revision_status=revision_status,
                   revision_recommended=revision['recommended'] if revision else None,
                   revision_steps=revision['candidates'][0]['steps'] if revision else None,
                   retained_steps=next((c['steps'] for c in initial['candidates'] if c['id'] == initial['recommended']), None) if revision_status == 'kept_existing' else None,
                   citation_protocol=initial['citation_protocol'], evaluation_protocol=initial['evaluation_protocol'])
        repair=evaluation.get('repair')
        row['all_deferred']=repair['all_deferred'] if repair else None
        row['repair_operations']=[s['operation'] for s in repair['steps']] if repair else None
        row['requirement_status'] = initial['assessments'][0].get('requirement_status')
        row['requirements'] = initial['assessments'][0].get('requirements')
        row['initial_checks'] = {c['dimension']:c['status'] for c in initial['assessments'][0]['checks']}
        row['model_prerequisite'] = check['status']
        states = [check['status'],row['requirement_status']]
        effective = 'contradicted' if 'contradicted' in states else 'unknown' if 'unknown' in states else check['status']
        row.update(prerequisite=effective,prerequisite_correct=effective == case['prerequisite'])
    except (KeyError, TypeError, IndexError, StopIteration, ValueError, AttributeError) as exc:
        row.update(completed=False, prerequisite_correct=False, error='invalid assessment report: '+str(exc))
    return row


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--fixture-writer', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--model', required=True)
    p.add_argument('--holds-dir', type=Path, required=True)
    p.add_argument('--cases', type=Path, help='Explicit synthetic development cases; expected labels are grading-only')
    p.add_argument('--expected-protocol', default='constraint-assessment-v2', choices=['constraint-assessment-v2','requirements-assessment-v3','requirements-assessment-v4','bound-repair-v5'])
    args = p.parse_args()
    cases = json.loads(args.cases.read_text()) if args.cases else CASES
    binary = args.binary.resolve(strict=True)
    fixture = args.fixture_writer.resolve(strict=True)
    holds = args.holds_dir.resolve(strict=True)
    endpoint = 'http://127.0.0.1:8080'
    resident(endpoint, args.model)
    args.output.mkdir(parents=True, exist_ok=False)
    root = args.output.resolve()
    home = root/'home'; home.mkdir()
    work = root/'work'; work.mkdir()
    (home/'config.toml').write_text('not = [valid toml')
    env = os.environ.copy()
    for key in ['OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT']:
        env.pop(key, None)
    env.update(MECHA_HOME=str(home), MECHA_SESSION_KIND='test')
    manifest = dict(protocol='plan-assessment-calibration-v2', cases=cases, event=EVENT, expected_protocol=args.expected_protocol,
                    model=args.model, seed=7, temperature=.6, max_calls_per_case=3,
                    binary_version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                    binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    fixture_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),
                    harness_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                    cases_sha256=hashlib.sha256(json.dumps(cases,sort_keys=True).encode()).hexdigest(),
                    usage_limit='Token totals describe recorded calls only; null means missing usage. Known subtotals exclude unknown usage. Timed-out processes may not return call audit rows.',
                    limit='Synthetic development cases, not an independent holdout or causal old/new comparison; fixed provisional concept isolates evaluation from concept-generation errors. Revision quality requires independent artifact review; model recommendation is not a gold label.')
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    rows = []; reviews = []
    for case in cases:
        resident(endpoint, args.model)
        db = root/(case['id']+'.memory-prototype.sqlite')
        seed = subprocess.run([str(fixture), str(db)], input=json.dumps(dict(evidence=case['evidence'], concept=case['concept'])),
                              env=env, cwd=work, capture_output=True, text=True, timeout=30)
        (root/(case['id']+'-fixture.json')).write_text(seed.stdout)
        if seed.returncode: raise RuntimeError(seed.stderr)
        refs = json.loads(seed.stdout)
        input_path = root/(case['id']+'-input.json')
        input_path.write_text(json.dumps(dict(query=case.get('query','Parcel M clearance Alternative route'), event=case.get('event',EVENT),
            goal=case.get('goal','task:delivery'), candidates=[dict(id='candidate', steps=[case['step']])]), indent=2)+'\n')
        argv = [str(binary), 'memory', '--experimental', '--db', str(db), 'evaluate-plans', str(input_path),
                '--synthetic-input', '--account', 'synthetic-calibration', '--endpoint', endpoint,
                '--model', args.model, '--holds-dir', str(holds), '--seed', '7', '--revise']
        start = time.monotonic()
        try:
            run = subprocess.run(argv, env=env, cwd=work, capture_output=True, text=True, timeout=600)
            stdout, stderr, code = run.stdout, run.stderr, run.returncode
        except subprocess.TimeoutExpired as exc:
            stdout = exc.stdout or b''; stderr = exc.stderr or b''; code = 124
            if isinstance(stdout, bytes): stdout = stdout.decode(errors='replace')
            if isinstance(stderr, bytes): stderr = stderr.decode(errors='replace')
        (root/(case['id']+'-evaluation.json')).write_text(stdout)
        (root/(case['id']+'-stderr.log')).write_text(stderr)
        try: value = json.loads(stdout)
        except ValueError: value = None
        row = grade(value, case, refs, code, args.expected_protocol)
        row['elapsed_seconds'] = time.monotonic()-start
        row['same_model'] = bool(isinstance(value, dict) and isinstance(value.get('calls'),list) and value['calls']) and all(isinstance(c,dict) and c.get('model') == args.model for c in value['calls'])
        rows.append(row)
        reviews.append(dict(case=case['id'], rubric=case['revision_rubric'], original_steps=[case['step']],
                            revision_steps=row.get('revision_steps'), revision_status=row.get('revision_status'),
                            retained_steps=row.get('retained_steps'), semantic_citation_support=None,
                            revision_outcome=None, new_unsupported_assumptions=None,
                            unnecessary_steps_or_timing=None, reviewer_reason=None))
        (root/'report.json').write_text(json.dumps(rows, indent=2)+'\n')
        (root/'review-template.json').write_text(json.dumps(reviews, indent=2)+'\n')
        print(json.dumps(row), flush=True)
    resident(endpoint, args.model)


if __name__ == '__main__':
    main()
