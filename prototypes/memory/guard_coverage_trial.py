#!/usr/bin/env python3
"""Frozen synthetic guard contracts; labels only grade returned checker reports.

No LLM, execution, private data, or equivalence claim against free-text V5.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    cases_file = Path(__file__).with_name('guard_cases.json')
    cases_bytes = cases_file.read_bytes()
    cases = json.loads(cases_bytes)
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = dict(protocol='guard-coverage-fixtures-v1', cases_sha256=hashlib.sha256(cases_bytes).hexdigest(),
                    binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), binary=str(binary),
                    limit='Fixture-authored contracts are additional information. Deterministic development checks, not language extraction, held-out reasoning accuracy, real execution or security qualification.')
    (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    rows = []
    for case in cases:
        payload = json.dumps(case['input'])
        (args.output / (case['id'] + '-input.json')).write_text(payload)
        proc = subprocess.run([str(binary)], input=payload, text=True, capture_output=True, timeout=10)
        (args.output / (case['id'] + '-stderr.txt')).write_text(proc.stderr)
        report = json.loads(proc.stdout) if proc.returncode == 0 else None
        actual = dict(status=report['status'], occurrences=[x['status'] for x in report['occurrences']]) if report else dict(status='invalid', occurrences=[])
        # An invalid fixture must fail for the declared bad reference, not a crash.
        expected_error = case['expected']['status'] == 'invalid'
        matched = actual == case['expected'] and (not expected_error or (proc.returncode == 1 and 'unknown action reference' in proc.stderr))
        row = dict(case=case['id'], input_sha256=hashlib.sha256(payload.encode()).hexdigest(), matched=matched, exit_code=proc.returncode, expected=case['expected'], actual=actual, report=report)
        rows.append(row)
    result = dict(cases=len(rows), matched=sum(x['matched'] for x in rows), rows=rows)
    (args.output / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: result[k] for k in ('cases', 'matched')}))
    return 0 if all(x['matched'] for x in rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
