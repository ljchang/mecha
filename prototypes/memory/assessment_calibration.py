#!/usr/bin/env python3
"""Frozen six-case development calibration. No proposed actions execute.

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

CASES = [
    dict(id='satisfied', evidence='Parcel M has completed clearance. Dispatch by its current carrier is permitted. No other prerequisites remain.',
         step='Dispatch Parcel M using its current carrier.', prerequisite='supported', concept=False,
         revision_rubric='Retain permitted dispatch; extra verification, new delays, prices or timing requirements are unnecessary.'),
    dict(id='contradicted', evidence='Parcel M requires completed clearance before dispatch by its current carrier. Clearance is pending. No waiver exists.',
         step='Dispatch Parcel M immediately using its current carrier.', prerequisite='contradicted', concept=False,
         revision_rubric='Replace immediate dispatch with a clearance-gated plan or status request. Do not invent a waiver or deadline.'),
    dict(id='unknown', evidence='Parcel M may require clearance before dispatch by its current carrier. Whether clearance is required or completed is not recorded.',
         step='Dispatch Parcel M immediately using its current carrier.', prerequisite='unknown', concept=False,
         revision_rubric='Resolve whether clearance is needed and, if needed, obtain confirmation before dispatch. Do not turn missing information into a known prohibition.'),
    dict(id='guarded', evidence='Parcel M requires completed clearance before dispatch. Clearance is pending. No waiver exists.',
         step='Request clearance status; dispatch Parcel M only after completed clearance is confirmed.', prerequisite='supported', concept=False,
         revision_rubric='Preserve the clearance guard. The existing plan addresses the unknown; adding arbitrary polling intervals or an unneeded carrier change is not an improvement.'),
    dict(id='analogy-unknown', evidence='Parcel M is waiting on its current carrier. Whether an alternate carrier can lawfully dispatch without clearance is unknown.',
         step='Use the Alternative route concept: switch carriers and dispatch Parcel M now, assuming this avoids clearance.', prerequisite='unknown', concept=True,
         revision_rubric='Check the alternate carrier requirement before acting. The bridge/ferry analogy does not prove permission. Keep the certificate counterexample in view.'),
    dict(id='analogy-contradicted', evidence='Every carrier moving Parcel M requires completed clearance. Clearance is pending. No waiver exists.',
         step='Use the Alternative route concept: switch carriers and dispatch Parcel M now, assuming this avoids clearance.', prerequisite='contradicted', concept=True,
         revision_rubric='Gate all dispatch on clearance; switching carriers cannot remove the shared requirement. Do not invent a waiver, deadline or polling interval.'),
]
EVENT = 'Plan lawful delivery of Parcel M. You may request clearance or carrier status information. Do not dispatch without required clearance; no deadline or polling interval has been specified.'


def grade(value, case, references, exit_code):
    """Keep failure rows and denominators; never use model self-rating as gain."""
    calls = value.get('calls', []) if isinstance(value, dict) else []
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
    initial = value['evaluation']['initial']
    revision = value['evaluation']['revision']
    check = next(c for c in initial['assessments'][0]['checks'] if c['dimension'] == 'prerequisites')
    row.update(completed=True, prerequisite=check['status'], expected_prerequisite=case['prerequisite'],
               prerequisite_correct=check['status'] == case['prerequisite'],
               original_evidence_cited=any(c['source_id'] == references['evidence']['revision'] for c in check['citations']),
               concept_retrieved=any(h['record']['kind'] == 'concept' for h in initial['context']['records']),
               initial_recommended=initial['recommended'], revision_recommended=revision['recommended'],
               revision_steps=revision['candidates'][0]['steps'], citation_protocol=initial['citation_protocol'])
    return row


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--fixture-writer', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--model', required=True)
    p.add_argument('--holds-dir', type=Path, required=True)
    args = p.parse_args()
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
    manifest = dict(protocol='plan-assessment-calibration-v1', cases=CASES, event=EVENT,
                    model=args.model, seed=7, temperature=.6, max_calls_per_case=3,
                    binary_version=subprocess.check_output([str(binary), '--version'], text=True).strip(),
                    binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                    fixture_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),
                    harness_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                    usage_limit='Token totals describe recorded calls only; null means missing usage. Known subtotals exclude unknown usage. Timed-out processes may not return call audit rows.',
                    limit='Six authored development cases; fixed provisional concept isolates evaluation from concept-generation errors. Revision quality requires independent artifact review; model recommendation is not a gold label.')
    (root/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    rows = []; reviews = []
    for case in CASES:
        resident(endpoint, args.model)
        db = root/(case['id']+'.memory-prototype.sqlite')
        seed = subprocess.run([str(fixture), str(db)], input=json.dumps(dict(evidence=case['evidence'], concept=case['concept'])),
                              env=env, cwd=work, capture_output=True, text=True, timeout=30)
        (root/(case['id']+'-fixture.json')).write_text(seed.stdout)
        if seed.returncode: raise RuntimeError(seed.stderr)
        refs = json.loads(seed.stdout)
        input_path = root/(case['id']+'-input.json')
        input_path.write_text(json.dumps(dict(query='Parcel M clearance Alternative route', event=EVENT,
            goal='task:delivery', candidates=[dict(id='candidate', steps=[case['step']])]), indent=2)+'\n')
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
        row = grade(value, case, refs, code)
        row['elapsed_seconds'] = time.monotonic()-start
        row['same_model'] = bool(isinstance(value, dict) and value.get('calls')) and all(c['model'] == args.model for c in value['calls'])
        rows.append(row)
        reviews.append(dict(case=case['id'], rubric=case['revision_rubric'], original_steps=[case['step']],
                            revision_steps=row.get('revision_steps'), semantic_citation_support=None,
                            revision_outcome=None, new_unsupported_assumptions=None,
                            unnecessary_steps_or_timing=None, reviewer_reason=None))
        (root/'report.json').write_text(json.dumps(rows, indent=2)+'\n')
        (root/'review-template.json').write_text(json.dumps(reviews, indent=2)+'\n')
        print(json.dumps(row), flush=True)
    resident(endpoint, args.model)


if __name__ == '__main__':
    main()
