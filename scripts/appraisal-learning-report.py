#!/usr/bin/env python3
"""Describe a registered sequential learning pilot, including actual rule exposure.

The second half of each sequence is the transfer slice, fixed before execution.
Native exp judge remains separate. A stage exiting successfully need not have
produced or measured a rule; the transcript's config is the exposure evidence.
"""
import collections
import json
from pathlib import Path
import sys


def report(data, traces):
    manifest = data['manifest']
    if manifest['kind'] != 'lifetime' or manifest['control'] != 'control' or set(manifest['arms']) != {'control', 'learning'}:
        raise ValueError('expected the registered control/learning lifetime design')
    ids = manifest['tasks']['ids']
    if not ids or len(ids) % 2 or len(ids) != len(set(ids)):
        raise ValueError('need two equal, explicitly ordered task halves')
    expected = {(arm, task, seed, rep): pos for arm in manifest['arms']
                for seed in (manifest.get('seeds') or [None])
                for rep in range(1, manifest.get('repetitions', 1) + 1)
                for pos, task in enumerate(ids, 1)}
    rows = {}
    exposure = {}
    for trace in traces['trials']:
        if trace['trial'] in exposure:
            raise ValueError('duplicate trace')
        exposure[trace['trial']] = trace
    for row in data['trials']:
        key = (row['arm'], row['task'], row.get('seed'), row['repetition'])
        if key not in expected or key in rows or row.get('position') != expected[key]:
            raise ValueError('duplicate, unplanned or out-of-order trial')
        rows[key] = row
    graded = lambda r: r is not None and r.get('status') == 'done' and type(r.get('passed')) is bool
    slices = {}
    for label, positions in [('initial', range(1, len(ids)//2+1)), ('transfer', range(len(ids)//2+1, len(ids)+1))]:
        arms = {}
        for arm in manifest['arms']:
            selected = [rows.get(k) for k, pos in expected.items() if k[0] == arm and pos in positions]
            valid = [r for r in selected if graded(r)]
            carried = [exposure.get(r['id']) for r in valid]
            known = [t for t in carried if t and t.get('rules_hash') is not None]
            arms[arm] = dict(expected=len(selected), graded=len(valid), passed=sum(r['passed'] for r in valid),
                             rules_exposure_known=len(known), runs_with_rules=sum(bool(t.get('rule_ids')) for t in known),
                             rule_ids=sorted({rid for t in known for rid in t.get('rule_ids', [])}))
        pairs = []
        for k, pos in expected.items():
            if k[0] == 'control' and pos in positions:
                a, b = rows.get(k), rows.get(('learning', *k[1:]))
                if graded(a) and graded(b):
                    pairs.append((a['passed'], b['passed']))
        slices[label] = dict(arms=arms, graded_pairs=len(pairs), improved=sum(not a and b for a,b in pairs),
                            regressed=sum(a and not b for a,b in pairs))
    return dict(slices=slices, stage_status_counts=dict(collections.Counter((s.get('status', 'unknown')) for s in data.get('stages', []))),
                unreadable_trials=data.get('unreadable_trials', 0), unreadable_stage_lines=data.get('unreadable_stage_lines', 0),
                native_judgements=data.get('judgements'),
                note='Descriptive mechanism pilot. Rule creation, rule exposure, validation coverage and task success are distinct. A one-seed sequence does not establish general efficacy.')


if __name__ == '__main__':
    data, traces = [json.loads(Path(p).read_text()) for p in sys.argv[1:]]
    print(json.dumps(report(data, traces), indent=2))
