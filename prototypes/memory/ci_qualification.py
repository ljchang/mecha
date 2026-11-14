#!/usr/bin/env python3
"""Run all memory Python tests with a fresh authored fixture and zero skips.

Only authored loopback transports supply replies; no real model
or owner store is used. The fixture and reports live in a disposable directory.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

QUALIFICATION = ('MECHA_OUTCOME_QUALIFICATION_BINARY',
                 'MECHA_OUTCOME_QUALIFICATION_DB',
                 'MECHA_OUTCOME_QUALIFICATION_ACTION')
HERE = Path(__file__).resolve().parent


def run_suite(directory=HERE, *, stream=None):
    """CI must execute the opt-in qualifications, never silently skip them."""
    missing = [name for name in QUALIFICATION if not os.environ.get(name)]
    if missing:
        raise ValueError('missing qualification configuration: ' + ', '.join(missing))
    suite = unittest.TestLoader().discover(str(directory), pattern='test_*.py')
    if suite.countTestCases() == 0:
        raise ValueError('memory Python discovery found no tests')
    result = unittest.TextTestRunner(stream=stream, verbosity=2).run(suite)
    return result.wasSuccessful() and not result.skipped


def prepare_fixture(binary, output):
    fixture = subprocess.run([sys.executable, str(HERE/'durable_action_trial.py'),
                    '--binary', str(binary), '--output', str(output)],
                   text=True, capture_output=True)
    if fixture.returncode:
        raise RuntimeError('authored fixture failed:\n' + fixture.stdout + fixture.stderr)
    summary = json.loads((output/'summary.json').read_text())
    if (summary.get('complete') is not True or summary.get('cases_failed') != 0
            or summary.get('new_real_model_inference_calls') != 0):
        raise ValueError('authored fixture qualification failed')
    report = json.loads((output/'mismatch'/'report.json').read_text())
    action = report['action']
    if (action['status']['phase'] != 'finalized'
            or action['feedback']['step_feedback']['verification'] != 'failed'):
        raise ValueError('fixture is not a finalized mismatch action')
    action_id = action['status']['id']
    if not isinstance(action_id, str) or not action_id:
        raise ValueError('fixture action reference is missing')
    return output/'mismatch'/'synthetic.memory-prototype.sqlite', action_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='mecha-memory-ci-') as temporary:
        db, action = prepare_fixture(binary, Path(temporary)/'fixture')
        from ordinary_lifetime_smoke import qualify
        qualify(binary, Path(temporary)/'ordinary-lifetime')
        os.environ.update(zip(QUALIFICATION, (str(binary), str(db), action)))
        if not run_suite():
            raise SystemExit('memory Python CI requires passing tests and zero skips')


if __name__ == '__main__':
    main()
