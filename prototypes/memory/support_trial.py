#!/usr/bin/env python3
"""Frozen synthetic factual-support gate, with no Python model transport.

Freeze once before inference, run each native case once, then externally audit
admitted generated statements and score. A failed/missing case stays in all eight
case denominators. This development gate is not a reliability estimate.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import time

PROTOCOL = 'memory-factual-support-v1'
SEED = 70510
STAGES = ('source', 'capture', 'generate', 'evaluate_generated', 'evaluate_probe', 'retain', 'recall', 'changed_goal')
MODEL_STAGES = ('generate', 'evaluate_generated', 'evaluate_probe')
BUDGETS = {'output_tokens': 4096, 'call_seconds': 180, 'case_seconds': 600, 'retries': 0, 'max_model_calls': 3}
CATEGORIES = {'correct', 'empty', 'missing', 'request_output_mismatch', 'changed_goal',
              'unsupported_cause', 'injected_instructions', 'unknown_evidence'}
FIXTURE = Path(__file__).resolve().parent / 'fixtures' / 'support_cases.json'
MAX_REPORT_BYTES = 1024 * 1024


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False) + '\n'


def decode(text):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON object key: ' + key)
            result[key] = value
        return result
    return json.loads(text, object_pairs_hook=unique)


def digest(value):
    return hashlib.sha256(encode(value).encode()).hexdigest()


def file_hash(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def fields(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys), 'invalid ' + label + ' fields')


def load_cases(path=None):
    path = Path(path or FIXTURE)
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 65536, 'fixture must be a bounded regular file')
    fixture = decode(path.read_text())
    fields(fixture, ('version', 'cases'), 'fixture')
    cases = fixture['cases']
    require(fixture['version'] == 1 and isinstance(cases, list) and len(cases) == 8, 'expected exactly eight fresh cases')
    ids = set()
    for case in cases:
        fields(case, ('id', 'category', 'native', 'probe', 'annotations'), 'case')
        require(isinstance(case['id'], str) and case['id'] and all(c.islower() or c.isdigit() or c == '-' for c in case['id']), 'invalid case id')
        require(case['id'] not in ids, 'duplicate case id')
        ids.add(case['id'])
        fields(case['native'], ('input_text', 'requested_output', 'observed_output', 'historical_criterion', 'current_criterion', 'goal_text'), 'native')
        for key, value in case['native'].items():
            require((key == 'observed_output' and value is None) or isinstance(value, str), 'native values must be text or absent observed output')
            if isinstance(value, str):
                require(len(value.encode()) <= 2048, 'native text over capture limit')
        fields(case['probe'], ('units', 'judgments'), 'probe')
        units = case['probe']['units']
        require(isinstance(units, list) and len(units) == 3, 'expected true, false and uncertain probe units')
        unit_ids = set()
        for unit in units:
            fields(unit, ('id', 'kind', 'text', 'citation'), 'probe unit')
            require(isinstance(unit['id'], str) and unit['id'] not in unit_ids, 'duplicate/invalid probe unit id')
            unit_ids.add(unit['id'])
            require(unit['kind'] in ('RecordedEvent', 'GoalAssessment', 'Hypothesis', 'Prediction'), 'unknown unit kind')
            require(isinstance(unit['text'], str) and 0 < len(unit['text'].encode()) <= 512, 'invalid unit text')
            fields(unit['citation'], ('passage', 'quote'), 'citation selector')
            require(unit['citation']['passage'] in ('input', 'artifact', 'action'), 'unknown passage selector')
            quote = unit['citation']['quote']
            require(quote is None or isinstance(quote, str) and 12 <= len(quote) <= 300, 'invalid exact quote selector')
        judgments = case['probe']['judgments']
        require(isinstance(judgments, list) and len(judgments) == 2, 'expected two bearing probes')
        for judgment in judgments:
            fields(judgment, ('bearing', 'because'), 'probe judgment')
            require(judgment['bearing'] in ('Good', 'Bad', 'Unknown') and isinstance(judgment['because'], list)
                    and set(judgment['because']) <= unit_ids, 'invalid probe judgment')
        annotations = case['annotations']
        fields(annotations, ('true_unit', 'false_unit', 'uncertain_unit', 'expected_bearing', 'basis'), 'annotations')
        require({annotations[k] for k in ('true_unit', 'false_unit', 'uncertain_unit')} == unit_ids, 'annotation target mismatch')
        require(annotations['expected_bearing'] in ('Good', 'Bad', 'Unknown') and isinstance(annotations['basis'], str), 'invalid host annotations')
    require({case['category'] for case in cases} == CATEGORIES, 'missing qualification category')
    return cases


def native_input(case):
    """Only the native task. Probes and annotations cannot enter source/generation."""
    return copy.deepcopy(case['native'])


def trial_input(case):
    """Host-only Rust case envelope; Rust never serializes this into a prompt."""
    return {'version': 1, 'case_id': case['id'], 'case_sha256': digest(case),
            'native': native_input(case), 'probe': copy.deepcopy(case['probe'])}


def bounded_read(path, limit):
    require(not path.is_symlink(), 'symlink artifact refused')
    fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0))
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_size <= limit, 'oversized or nonregular artifact')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read(limit + 1)
        require(len(data) <= limit, 'oversized artifact')
        return data
    finally:
        os.close(fd)


def artifact_bytes(root, descriptor):
    fields(descriptor, ('path', 'sha256', 'bytes'), 'artifact descriptor')
    relative = Path(descriptor['path'])
    require(not relative.is_absolute() and relative.parts and all(p not in ('..', '.') for p in relative.parts), 'artifact path escapes root')
    path = root
    for component in relative.parts:
        path = path / component
        require(not path.is_symlink(), 'symlink artifact refused')
    require(path.resolve().is_relative_to(root.resolve()), 'artifact path escapes root')
    data = bounded_read(path, MAX_REPORT_BYTES)
    require(type(descriptor['bytes']) is int and descriptor['bytes'] == len(data)
            and descriptor['sha256'] == hashlib.sha256(data).hexdigest(), 'artifact digest/size mismatch')
    return data


def validate_call(root, stage, model):
    request = decode(artifact_bytes(root, stage['request']).decode())
    reply = decode(artifact_bytes(root, stage['reply']).decode())
    audit = [decode(line) for line in artifact_bytes(root, stage['provider_audit']).decode().splitlines() if line.strip()]
    require(isinstance(request, dict) and isinstance(reply, dict)
            and all(isinstance(event, dict) for event in audit), 'malformed raw provider event')
    starts = [r for r in audit if r.get('event') == 'request_started']
    replies = [r for r in audit if r.get('event') == 'response']
    require(starts == [request] and replies == [reply] and request.get('call') == reply.get('call') == 1,
            'model call count/raw artifact mismatch')
    require(not any(r.get('event') in ('error', 'stream_refused') for r in audit), 'provider audit records failure')
    effective, wire = request['request'], request['wire_body']
    require(isinstance(effective, dict) and isinstance(wire, dict), 'malformed raw request body')
    require(effective.get('model') == wire.get('model') == reply.get('model') == model, 'raw model identity mismatch')
    require(effective.get('local_only') is True and effective.get('think') is False
            and effective.get('max_tokens') == wire.get('max_tokens') == 4096
            and wire.get('autoload') is False and wire.get('seed') == SEED and wire.get('temperature') == 0.6,
            'raw runtime controls mismatch')
    require(reply.get('stop_reason') == 'end_turn' and not reply.get('refusal'), 'raw refusal/incomplete response')
    require(all(reply.get(k) == stage[k] for k in ('input_tokens', 'output_tokens')), 'raw usage mismatch')
    return reply


def response_payload(reply):
    message = reply.get('message')
    require(isinstance(message, dict) and isinstance(message.get('content'), list), 'missing raw reply message')
    text = ''.join(block['text'] for block in message['content']
                   if isinstance(block, dict) and block.get('type') == 'text' and isinstance(block.get('text'), str))
    require(len(text.encode()) <= 16384, 'oversized raw structured reply')
    return decode(text)


def validate_sources(root, case, report):
    artifacts = report.get('artifacts')
    require(isinstance(artifacts, dict) and set(artifacts) == {'initial_input', 'native_transcript', 'observed_output', 'admitted_generated'}, 'missing/unknown evidence artifacts')
    content = {}
    for key in ('initial_input', 'native_transcript', 'observed_output'):
        value = artifacts[key]
        if key == 'observed_output' and case['native']['observed_output'] is None:
            require(value is None, 'missing fixture fabricated an observed artifact')
        else:
            content[key] = artifact_bytes(root, value)
    require(content['initial_input'] == case['native']['input_text'].encode(), 'wrong initial input bytes')
    if case['native']['observed_output'] is not None:
        require(content['observed_output'] == case['native']['observed_output'].encode(), 'wrong observed output bytes')
    expected = {key: artifacts[key]['sha256'] if artifacts[key] else None
                for key in ('initial_input', 'native_transcript', 'observed_output')}
    seal = decode(artifact_bytes(root, report['source_seal']).decode())
    require(seal == expected == report.get('initial_hashes'), 'source hashes differ from pre-inference seal')


def validate_support(root, report, name, model, offered):
    descriptors = report.get('support_reports')
    fields(descriptors, ('generated', 'probe'), 'durable support reports')
    support = decode(artifact_bytes(root, descriptors[name]).decode())
    fields(support, ('protocol', 'generator_model', 'evaluator_model', 'proposal_hash', 'request_hash', 'units', 'judgments'), 'durable support report')
    require(support['protocol'] == 'situated-factual-support-v1' and support['evaluator_model'] == model
            and isinstance(support['generator_model'], str) and support['generator_model'], 'durable support identity mismatch')
    if name == 'generated':
        require(support['generator_model'] == model, 'durable generator identity mismatch')
    for key in ('proposal_hash', 'request_hash'):
        require(isinstance(support[key], str) and len(support[key]) == 64
                and all(c in '0123456789abcdef' for c in support[key]), 'invalid durable support hash')
    require(isinstance(report.get('stages'), dict), 'missing native stages')
    stage = report['stages']['evaluate_' + name]
    require(isinstance(stage, dict), 'invalid evaluator stage')
    require(stage.get('status') == 'completed' and stage.get('error') is None, 'probe/evaluator outcome not completed')
    payload = response_payload(validate_call(root, stage, model))
    fields(payload, ('checks',), 'raw support reply')
    require(isinstance(payload['checks'], list), 'missing raw checklist')
    checks = {}
    for check in payload['checks']:
        fields(check, ('target', 'status', 'unsupported_factual_premise', 'citations', 'reason', 'questions'), 'raw support check')
        target = check['target']
        require(isinstance(target, str) and target not in checks and check['status'] in ('Supported', 'Contradicted', 'Unknown')
                and type(check['unsupported_factual_premise']) is bool, 'invalid/duplicate raw support target')
        checks[target] = check
    require(isinstance(offered, dict) and isinstance(offered.get('units'), list) and isinstance(offered.get('judgments'), list), 'missing offered proposal')
    for unit in offered['units']:
        if name == 'generated':
            fields(unit, ('id', 'kind', 'text', 'citations'), 'generated proposal unit')
        require(isinstance(unit, dict) and isinstance(unit.get('id'), str), 'invalid offered unit')
    expected_units = {u['id']: u for u in offered['units']}
    require(len(expected_units) == len(offered['units']) and isinstance(support['units'], list)
            and len(support['units']) == len(expected_units), 'durable unit coverage mismatch')
    expected_targets = {'unit:' + identifier for identifier in expected_units} | {'judgment:' + str(i) for i in range(len(offered['judgments']))}
    require(set(checks) == expected_targets, 'raw support target coverage mismatch')
    accepted, units = [], {}
    for entry in support['units']:
        require(isinstance(entry, dict), 'invalid durable unit entry')
        admitted = entry.get('disposition') == 'Accepted'
        if admitted:
            fields(entry, ('disposition', 'unit', 'status'), 'accepted durable unit')
            unit = entry['unit']
            fields(unit, ('id', 'kind', 'text', 'citations'), 'accepted durable unit payload')
            identifier = unit['id']
        else:
            fields(entry, ('disposition', 'id', 'status'), 'rejected durable unit')
            require(entry['disposition'] == 'Rejected', 'unknown durable unit disposition')
            identifier, unit = entry['id'], None
        require(identifier in expected_units and identifier not in units, 'unknown/duplicate durable unit')
        original, check = expected_units[identifier], checks['unit:' + identifier]
        require(entry['status'] == check['status'], 'durable/raw unit status mismatch')
        should_admit = (not check['unsupported_factual_premise'] and
                        (check['status'] == 'Supported' or check['status'] == 'Unknown' and original['kind'] in ('Hypothesis', 'Prediction')))
        require(admitted == should_admit, 'durable/raw unit admission mismatch')
        if admitted:
            require(all(unit[k] == original[k] for k in ('id', 'kind', 'text')), 'durable accepted unit differs from offered proposal')
            if name == 'generated':
                require(unit == original, 'durable citations differ from generated proposal')
            accepted.append(dict(unit, status=entry['status']))
        units[identifier] = dict(id=identifier, status=entry['status'], admitted=admitted, withheld=not admitted,
                                 unsupported_factual_premise=check['unsupported_factual_premise'])
    require(isinstance(support['judgments'], list) and len(support['judgments']) == len(offered['judgments']), 'durable judgment coverage mismatch')
    judgments = []
    for index, (entry, original) in enumerate(zip(support['judgments'], offered['judgments'])):
        fields(entry, ('index', 'goal', 'offered_bearing', 'bearing', 'because', 'status', 'reason'), 'durable judgment')
        check = checks['judgment:' + str(index)]
        require(type(entry['index']) is int and entry['index'] == index and entry['offered_bearing'] == original['bearing']
                and entry['because'] == original['because'] and entry['status'] == check['status'], 'durable/raw offered judgment mismatch')
        reason = entry['reason']
        require(reason in ('Retained', 'ExplicitUnknown', 'UnsupportedJudgment', 'UnsupportedPremises', 'DuplicateGoal'), 'unknown durable judgment reason')
        disposition = 'dropped' if reason == 'DuplicateGoal' else 'retained' if reason in ('Retained', 'ExplicitUnknown') else 'downgraded'
        require(entry['bearing'] in ('Good', 'Bad', 'Unknown') and (disposition == 'retained' or entry['bearing'] == 'Unknown'), 'durable final bearing mismatch')
        judgments.append(dict(index=index, status=entry['status'], offered_bearing=entry['offered_bearing'], bearing=entry['bearing'],
                              disposition=disposition, reason=reason, unsupported_factual_premise=check['unsupported_factual_premise']))
    return accepted, units, judgments


def validate_probe(root, case, report, model):
    validate_sources(root, case, report)
    _, units, judgments = validate_support(root, report, 'probe', model, case['probe'])
    probe = report.get('probe')
    fields(probe, ('units', 'judgments'), 'probe result')
    require(isinstance(probe['units'], list) and len(probe['units']) == len(units)
            and {u['id'] for u in probe['units']} == set(units), 'probe unit coverage mismatch')
    for unit in probe['units']:
        require(unit == units[unit['id']], 'probe summary differs from durable/raw unit')
    require(probe['judgments'] == judgments, 'probe summary differs from durable/raw judgments')
    return probe


def validate_report(root, case, model):
    """Return a failed row on malformed/incomplete native output, never omit it."""
    root = Path(root)
    result = {'valid': False, 'errors': [], 'report': None, 'report_sha256': None, 'probe_outcome': None, 'probe_errors': []}
    try:
        report_bytes = bounded_read(root / 'report.json', MAX_REPORT_BYTES)
        result['report_sha256'] = hashlib.sha256(report_bytes).hexdigest()
        report = decode(report_bytes.decode())
        result['report'] = report
        require(isinstance(report, dict), 'report is not an object')
        # Probe evidence survives a later recall/changed-goal failure. Validate it
        # independently before checking the overall successful-attempt envelope.
        try:
            require(report.get('protocol') == PROTOCOL and report.get('seed') == SEED and report.get('model') == model
                    and report.get('case_id') == case['id'] and report.get('case_sha256') == digest(case), 'probe case identity mismatch')
            result['probe_outcome'] = validate_probe(root, case, report, model)
        except (ValueError, OSError, KeyError, TypeError, UnicodeError) as error:
            result['probe_errors'].append(str(error))
        require(report.get('protocol') == PROTOCOL and report.get('case_id') == case['id']
                and report.get('case_sha256') == digest(case), 'report protocol/case identity mismatch')
        require(report.get('seed') == SEED and report.get('model') == model, 'model/seed mismatch')
        require(report.get('status') == 'completed' and report.get('run_error') is None, 'native attempt did not complete')
        stages = report.get('stages')
        require(isinstance(stages, dict) and set(stages) == set(STAGES), 'missing/unknown intended stage')
        for name, stage in stages.items():
            require(isinstance(stage, dict), 'invalid stage')
            allowed = ('completed', 'withheld') if name in ('retain', 'recall', 'changed_goal') else ('completed',)
            require(stage.get('status') in allowed, 'incomplete/failed stage: ' + name)
            if stage['status'] == 'withheld':
                require(isinstance(stage.get('error'), str) and stage['error'], 'withheld stage needs diagnostic')
            else:
                require(stage.get('error') is None, 'completed stage with error')
            if name == 'capture' or name in MODEL_STAGES:
                limit = 20000 if name == 'capture' else 32768 if name == 'generate' else 49152
                require(type(stage.get('packet_bytes')) is int and 0 < stage['packet_bytes'] <= limit,
                        'missing/oversized atomic packet: ' + name)
                if name == 'capture':
                    require(stage.get('withheld') is False, 'completed capture missing withholding accounting')
            if name in MODEL_STAGES:
                require(stage.get('model') == model, 'served model mismatch: ' + name)
                require(stage.get('stop_reason') == 'end_turn', 'incomplete/refused model reply: ' + name)
                validate_call(root, stage, model)
                for key in ('input_tokens', 'output_tokens'):
                    require(type(stage.get(key)) is int and stage[key] >= 0, 'missing model usage')
                require(stage['output_tokens'] <= BUDGETS['output_tokens'], 'model output token bound exceeded')
        for key in ('input_tokens', 'output_tokens'):
            require(type(report.get(key)) is int and report[key] == sum(stages[name][key] for name in MODEL_STAGES), 'usage total mismatch')
        validate_sources(root, case, report)
        generated_reply = validate_call(root, stages['generate'], model)
        generated_proposal = response_payload(generated_reply)
        accepted, _, _ = validate_support(root, report, 'generated', model, generated_proposal)
        admitted = decode(artifact_bytes(root, report['artifacts']['admitted_generated']).decode())
        require(admitted == accepted, 'admitted export differs from durable accepted units')
        result['admitted_unit_ids'] = sorted(unit['id'] for unit in accepted)
        require(result['probe_outcome'] is not None, '; '.join(result['probe_errors']) or 'missing validated probe outcome')
        result['valid'] = True
    except FileNotFoundError as error:
        result['errors'].append('missing native report/artifact: ' + str(error))
    except (ValueError, OSError, KeyError, TypeError, UnicodeError) as error:
        result['errors'].append(str(error))
    return result


def manual_audit(validation, audit):
    """External review, bound to exact report and admitted unit bytes; never self-score."""
    try:
        report = validation['report']
        require(isinstance(audit, dict) and audit.get('reviewer_kind') == 'external_manual'
                and isinstance(audit.get('reviewer'), str) and audit['reviewer'].strip(), 'external reviewer required')
        require(audit.get('report_sha256') == validation['report_sha256']
                and audit.get('admitted_generated_sha256') == report['artifacts']['admitted_generated']['sha256'], 'audit digest mismatch')
        require(isinstance(audit.get('inspected_unit_ids'), list) and isinstance(audit.get('unsupported_unit_ids'), list), 'audit coverage required')
        require(set(audit['inspected_unit_ids']) == set(validation['admitted_unit_ids'])
                and len(audit['inspected_unit_ids']) == len(set(audit['inspected_unit_ids'])), 'audit coverage mismatch')
        require(set(audit['unsupported_unit_ids']) <= set(audit['inspected_unit_ids'])
                and len(audit['unsupported_unit_ids']) == len(set(audit['unsupported_unit_ids']))
                and isinstance(audit.get('notes'), str) and audit['notes'].strip(), 'audit findings/notes missing')
        return True, len(audit['unsupported_unit_ids'])
    except (ValueError, KeyError, TypeError):
        return False, None


def score_case(case, validation, audit):
    row = {'case_id': case['id'], 'valid': validation['valid'], 'errors': validation['errors'],
           'true_supported': None, 'uncertain_appropriate': None, 'false_facts_accepted': None,
           'wrong_bearings_accepted': None, 'wrong_bearings_supported': None, 'correct_bearing_supported': None,
           'probe_outcome_observed': validation.get('probe_outcome') is not None, 'manual_audit_complete': False, 'unsupported_generated': None,
           'input_tokens': None, 'output_tokens': None,
           'observed_input_tokens': None, 'observed_output_tokens': None}
    report = validation.get('report')
    # Preserve reported cost even for rejected output. Missing cost is unknown, never zero.
    if isinstance(report, dict):
        for key in ('input_tokens', 'output_tokens'):
            if type(report.get(key)) is int and report[key] >= 0:
                row[key] = report[key]
            observed = report.get('observed_' + key)
            if type(observed) is int and observed >= 0:
                row['observed_' + key] = observed
            if row[key] is not None:
                row['observed_' + key] = max(row[key], row['observed_' + key] or 0)
    probe = validation.get('probe_outcome')
    if probe is None:
        return row
    units = {u['id']: u for u in probe['units']}
    annotations = case['annotations']
    true, false, uncertain = [units[annotations[key]] for key in ('true_unit', 'false_unit', 'uncertain_unit')]
    row.update(true_supported=int(true['admitted'] and true['status'] == 'Supported'),
               false_facts_accepted=int(false['admitted']),
               uncertain_appropriate=int(uncertain['status'] == 'Unknown' and uncertain['withheld']),
               wrong_bearings_accepted=sum(j['bearing'] in ('Good', 'Bad') and j['bearing'] != annotations['expected_bearing']
                                          and j['disposition'] == 'retained' for j in probe['judgments']),
               wrong_bearings_supported=sum(j['offered_bearing'] in ('Good', 'Bad') and j['offered_bearing'] != annotations['expected_bearing']
                                            and j['status'] == 'Supported' for j in probe['judgments']),
               correct_bearing_supported=int(any(j['offered_bearing'] == annotations['expected_bearing'] and j['status'] == 'Supported'
                                                and not j['unsupported_factual_premise'] for j in probe['judgments'])))
    if validation['valid']:
        row['manual_audit_complete'], row['unsupported_generated'] = manual_audit(validation, audit)
    return row


def summarize(rows):
    total = len(rows)
    def count(key):
        return sum(row.get(key, 0) or 0 for row in rows)
    def observed_cost(key):
        return sum(row.get('observed_' + key) if type(row.get('observed_' + key)) is int
                   else row.get(key) or 0 for row in rows)
    valid = sum(row.get('valid', False) for row in rows)
    unique = len({row['case_id'] for row in rows}) == total
    audit_complete = sum(row.get('manual_audit_complete', False) for row in rows)
    observed = sum(row.get('probe_outcome_observed', False) for row in rows)
    metrics_complete = total == 8 and observed == 8
    passed = (total == 8 and unique and valid == 8 and audit_complete == 8
              and metrics_complete and count('true_supported') >= 6 and count('uncertain_appropriate') >= 6 and count('correct_bearing_supported') >= 6
              and count('false_facts_accepted') == count('wrong_bearings_accepted') == count('wrong_bearings_supported') == count('unsupported_generated') == 0)
    return {'protocol': PROTOCOL, 'attempts': total, 'planned_attempts': 8, 'valid_attempts': valid,
            'failed_attempts': total - valid, 'unique_cases': unique,
            'true_supported': {'count': count('true_supported'), 'denominator': 8},
            'uncertain_appropriate': {'count': count('uncertain_appropriate'), 'denominator': 8},
            'false_facts_accepted': count('false_facts_accepted') if metrics_complete else None,
            'wrong_bearings_accepted': count('wrong_bearings_accepted') if metrics_complete else None,
            'wrong_bearings_supported': count('wrong_bearings_supported') if metrics_complete else None,
            'observed_false_facts_accepted': count('false_facts_accepted'),
            'observed_wrong_bearings_accepted': count('wrong_bearings_accepted'),
            'observed_wrong_bearings_supported': count('wrong_bearings_supported'),
            'probe_metrics_complete': metrics_complete,
            'correct_bearing_supported': {'count': count('correct_bearing_supported'), 'denominator': 8},
            'observed_probe_outcomes': observed,
            'unknown_probe_outcomes': sum(not row.get('probe_outcome_observed', False) for row in rows),
            'manual_audits_complete': audit_complete, 'unsupported_generated': count('unsupported_generated') if audit_complete == 8 else None,
            'reported_input_tokens': observed_cost('input_tokens'), 'reported_output_tokens': observed_cost('output_tokens'),
            'observed_input_tokens': observed_cost('input_tokens'), 'observed_output_tokens': observed_cost('output_tokens'),
            'cost_complete': total == 8 and all(all(type(r.get(k)) is int for k in ('input_tokens', 'output_tokens')) for r in rows),
            'gate_passed': passed, 'downstream_comparison_authorized': passed,
            'evidence_boundary': 'Eight-case synthetic development gate only; model evaluation is fallible and correlated, not independent truth.'}


def write_json(path, value):
    """Atomic progress snapshots. Initial freeze uses an exclusively created root."""
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    with temporary.open('w') as stream:
        stream.write(encode(value))
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def source_hashes():
    repo = Path(__file__).resolve().parents[2]
    files = list((repo / 'mecha-core' / 'src').rglob('*.rs'))
    files += list((repo / 'mecha-core' / 'examples').rglob('*.rs'))
    files += [repo / p for p in ('Cargo.toml', 'Cargo.lock', 'mecha-core/Cargo.toml', 'rust-toolchain.toml')]
    return {str(path.relative_to(repo)): file_hash(path) for path in sorted(files)}


def freeze(root, binary, endpoint, model, holds_dir):
    cases = load_cases()
    root, binary = Path(root).absolute(), Path(binary).resolve(strict=True)
    require(binary.is_file() and os.access(binary, os.X_OK), 'executable native trial binary required')
    require(isinstance(model, str) and model.strip(), 'explicit model identity required')
    root.mkdir(parents=True, exist_ok=False)
    for directory in ('cases', 'runs', 'logs', 'audits'):
        (root / directory).mkdir()
    write_json(root / 'cases.json', {'version': 1, 'cases': cases})
    case_files = {}
    for case in cases:
        path = root / 'cases' / (case['id'] + '.json')
        write_json(path, trial_input(case))
        require(path.stat().st_size <= 65536, 'native case file exceeds 64KiB')
        case_files[case['id']] = {'path': str(path.relative_to(root)), 'sha256': file_hash(path)}
    attempts = [{'case_id': case['id'], 'status': 'planned', 'returncode': None,
                 'elapsed_ms': None, 'error': None,
                 'stages': {name: {'status': 'pending'} for name in STAGES}} for case in cases]
    repo = Path(__file__).resolve().parents[2]
    manifest = {'protocol': PROTOCOL, 'synthetic_only': True, 'seed': SEED, 'budgets': BUDGETS,
                'endpoint': endpoint, 'model': model, 'holds_dir': str(Path(holds_dir).absolute()),
                'binary': str(binary), 'binary_sha256': file_hash(binary),
                'helper_sha256': file_hash(__file__), 'fixture_sha256': file_hash(FIXTURE),
                'cases_sha256': file_hash(root / 'cases.json'), 'case_files': case_files,
                'source_files_sha256': source_hashes(),
                'build_correspondence': 'Unverified by this coordinator: independent binary/source hashes do not prove correspondence. Preserve the exact synced build command, successful build log, source snapshot and resulting binary digest in the controller audit.',
                'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip(),
                'attempts': attempts,
                'gate': {'max_false_facts': 0, 'max_wrong_bearings': 0, 'min_true_supported': 6,
                         'min_uncertain_appropriate': 6, 'max_unsupported_generated': 0,
                         'max_wrong_bearings_supported': 0, 'min_correct_bearing_supported': 6,
                         'require_all_eight_external_manual_audits': True},
                'created_unix': time.time()}
    write_json(root / 'manifest.json', manifest)
    (root / 'manifest.sha256').write_text(file_hash(root / 'manifest.json') + '\n')
    write_json(root / 'attempts.json', attempts)
    return manifest


def validate_frozen(root, manifest, runtime=True):
    root = Path(root)
    require(file_hash(root / 'manifest.json') == (root / 'manifest.sha256').read_text().strip()
            and decode((root / 'manifest.json').read_text()) == manifest, 'modified frozen manifest')
    require(manifest['protocol'] == PROTOCOL and manifest['seed'] == SEED and manifest['budgets'] == BUDGETS,
            'unknown/modified protocol')
    if runtime:
        require(file_hash(manifest['binary']) == manifest['binary_sha256']
                and file_hash(__file__) == manifest['helper_sha256']
                and file_hash(FIXTURE) == manifest['fixture_sha256']
                and source_hashes() == manifest['source_files_sha256'], 'modified native binary/helper/source/fixture')
    require(file_hash(root / 'cases.json') == manifest['cases_sha256'], 'modified frozen cases')
    cases = load_cases(root / 'cases.json')
    require(set(manifest['case_files']) == {c['id'] for c in cases}, 'missing case envelope')
    for case in cases:
        record = manifest['case_files'][case['id']]
        expected = 'cases/' + case['id'] + '.json'
        require(record['path'] == expected and file_hash(root / expected) == record['sha256']
                and decode((root / expected).read_text()) == trial_input(case), 'modified native case envelope')
    return cases


def score_root(root):
    root = Path(root)
    manifest = decode((root / 'manifest.json').read_text())
    cases = validate_frozen(root, manifest, runtime=False)
    runtime_error = None
    try:
        validate_frozen(root, manifest)
    except (ValueError, OSError) as error:
        runtime_error = str(error)
    ledger = decode((root / 'attempts.json').read_text())
    require(len(ledger) == 8 and [a['case_id'] for a in ledger] == [c['id'] for c in cases], 'missing/duplicate attempt ledger slot')
    rows = []
    for case, attempt in zip(cases, ledger):
        validation = validate_report(root / 'runs' / case['id'], case, manifest['model'])
        if runtime_error:
            validation['valid'] = False
            validation['errors'].append(runtime_error)
        if attempt['status'] != 'recorded' or attempt['returncode'] != 0:
            validation['valid'] = False
            validation['errors'].append(attempt.get('error') or 'native process not recorded successfully')
        audit_path = root / 'audits' / (case['id'] + '.json')
        audit = None
        if audit_path.exists():
            try:
                audit = decode(bounded_read(audit_path, 65536).decode())
            except (OSError, ValueError, UnicodeError):
                pass
        row = score_case(case, validation, audit)
        row['attempt'] = attempt
        rows.append(row)
    summary = summarize(rows)
    summary['cases'] = rows
    summary['manifest_sha256'] = file_hash(root / 'manifest.json')
    write_json(root / 'summary.json', summary)
    return summary


def run(root):
    """Exactly one native invocation per frozen case. Interrupted trials cannot retry."""
    root = Path(root).resolve(strict=True)
    manifest = decode((root / 'manifest.json').read_text())
    cases = validate_frozen(root, manifest)
    require(not (root / 'RUN_STARTED').exists(), 'trial already started; retries/tuning reruns forbidden')
    ledger = decode((root / 'attempts.json').read_text())
    require(ledger == manifest['attempts'], 'modified/unfresh attempt ledger')
    with (root / 'RUN_STARTED').open('x') as stream:
        stream.write(encode({'manifest_sha256': file_hash(root / 'manifest.json'), 'started_unix': time.time()}))
    environment = os.environ.copy()
    for key in ('OPENAI_API_KEY', 'ANTHROPIC_API_KEY', 'MECHA_MODEL', 'MECHA_PROVIDER', 'MECHA_EFFORT'):
        environment.pop(key, None)
    environment.update(MECHA_SESSION_KIND='test', MECHA_LOG='warn')
    for index, case in enumerate(cases):
        attempt = ledger[index]
        attempt.update(status='started', started_unix=time.time())
        write_json(root / 'attempts.json', ledger)
        started = time.monotonic()
        argv = [manifest['binary'], '--synthetic-only', str(root / 'runs' / case['id']),
                manifest['endpoint'], manifest['model'], manifest['holds_dir'], str(SEED),
                str(root / manifest['case_files'][case['id']]['path'])]
        stdout, stderr = b'', b''
        try:
            # Recheck the freeze before *every* call, including the binary itself.
            validate_frozen(root, manifest)
            call = subprocess.run(argv, env=environment, cwd=root, stdin=subprocess.DEVNULL,
                                  capture_output=True, timeout=BUDGETS['case_seconds'] + 15)
            stdout, stderr = call.stdout, call.stderr
            attempt.update(status='recorded' if call.returncode == 0 else 'failed', returncode=call.returncode)
            if call.returncode != 0:
                attempt['error'] = 'native process exited ' + str(call.returncode)
        except subprocess.TimeoutExpired as error:
            stdout, stderr = error.stdout or b'', error.stderr or b''
            attempt.update(status='failed', error='native 600s case bound plus 15s shutdown allowance expired')
        except (OSError, ValueError) as error:
            attempt.update(status='failed', error=str(error))
        attempt['elapsed_ms'] = round((time.monotonic() - started) * 1000)
        (root / 'logs' / (case['id'] + '.stdout')).write_bytes(stdout)
        (root / 'logs' / (case['id'] + '.stderr')).write_bytes(stderr)
        validation = validate_report(root / 'runs' / case['id'], case, manifest['model'])
        if isinstance(validation.get('report'), dict) and isinstance(validation['report'].get('stages'), dict):
            # Keep all intended stages visible even if the native report omitted one.
            for stage in STAGES:
                if stage in validation['report']['stages']:
                    attempt['stages'][stage] = validation['report']['stages'][stage]
        if not validation['valid']:
            attempt.update(status='failed', error='; '.join(validation['errors']))
        write_json(root / 'attempts.json', ledger)
    return score_root(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    freeze_parser = commands.add_parser('freeze')
    for arg in ('root', 'binary', 'endpoint', 'model', 'holds_dir'):
        freeze_parser.add_argument(arg)
    for command in ('run', 'score'):
        commands.add_parser(command).add_argument('root')
    args = parser.parse_args()
    if args.command == 'freeze':
        result = freeze(args.root, args.binary, args.endpoint, args.model, args.holds_dir)
        print(encode({'protocol': result['protocol'], 'frozen_attempts': len(result['attempts'])}), end='')
    else:
        result = run(args.root) if args.command == 'run' else score_root(args.root)
        print(encode({key: value for key, value in result.items() if key != 'cases'}), end='')
        raise SystemExit(0 if result['gate_passed'] else 1)


if __name__ == '__main__':
    main()
