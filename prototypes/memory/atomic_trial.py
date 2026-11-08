#!/usr/bin/env python3
"""Independent synthetic v4 audit; never infer semantic truth from admission.

No model transport, case authoring, or legacy scoring lives here. Raw-source
reconstruction and bounded artifact I/O reuse immutable Python v3 utilities;
all v4 protocol, retention and comparison checks are independent of Rust verdicts.
Blinded review is an external annotation requirement, not something this script
can verify about a reviewer's conduct. Export hashes establish correspondence,
not live source authorization or binary/source build correspondence.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re
import sys

import support_trial as legacy

PROTOCOL = 'memory-atomic-comparison-v4'
AUDIT_PROTOCOL = 'atomic-semantic-review-v2'
QUALITY_GATE = dict(planned_cases=8, complete_workflows=8, complete_independent_reviews=8,
    correct_current_bearings=8, accepted_factual_defects=0, accepted_citation_defects=0,
    retained_factual_defects=0, retained_citation_defects=0,
    known_bearings_require_sufficient_current_premises=True, rejected_generation_errors_disqualify=False,
    supplied_frozen_labels_must_match=True)
WIRE = 'situated-support-wire-v4'
ARMS = ('pipeline', 'direct')
STAGES = {'pipeline': ('generate', 'check', 'appraise'), 'direct': ('direct',)}
LIMITS = {'generate': 1536, 'check': 3072, 'appraise': 1536, 'direct': 6144}
# Frozen policy-frame identities: dynamic evidence belongs only in the packet.
FRAME_SHA256 = {'generate': '34b38043590c4aadd0a867576a9aca7c6071b94058143b630f54078eebc12ac4', 'check': '7af7ff9d98f0515e866f46afd6f8d152035c4de09d0524ed59c706396e1e9061', 'appraise': 'f6d149cbf883c59c44d4c43f7851163dc7107415e99cd5d5515fdbeffd4fb96c', 'direct': '8f85d0c7dedd4c9fa4165db211e8ec99378d775b5604961b704c310e55d7bc11'}
SEED = 70510
ERRORS = (ValueError, KeyError, TypeError, OSError, UnicodeError, IndexError, AttributeError)
require = legacy.require
fields = legacy.fields
artifact_bytes = legacy.artifact_bytes


def decode(text):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'duplicate JSON key: ' + key)
            result[key] = value
        return result
    def nonfinite(value):
        raise ValueError('nonfinite JSON: ' + value)
    return json.loads(text, object_pairs_hook=pairs, parse_constant=nonfinite)


def digest(value):
    return hashlib.sha256((json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False) + '\n').encode()).hexdigest()


def read(root, descriptor):
    return decode(artifact_bytes(Path(root), descriptor).decode())


def bounded_text(value, size, label):
    require(isinstance(value, str) and value.strip() and len(value.encode()) <= size and len(value) <= size // 4, 'invalid ' + label)


def diagnostic(value, unknown=False):
    bounded_text(value['reason'], 512, 'reason')
    questions = value['questions']
    require(isinstance(questions, list) and len(questions) <= 4, 'invalid questions')
    for question in questions:
        bounded_text(question, 256, 'question')
    require(not unknown or bool(questions), 'unknown requires explicit question')


def empty_row(case_id, arm):
    return dict(case_id=case_id, arm=arm, structural_valid=False, completed=False,
                final_state=None, final_state_source='unobserved', observed_final_states=[], retained_bearing=None, frozen_expected_state=None, frozen_label_correct=None,
                observation_targets=[], observation_inventory_complete=False, observation_audit_complete=False,
                retention_verified=False, recall_verified=False, criterion_review=None, selected_premises_sufficient=None,
                temporally_applicable=None, criterion_supported=None, usage_stages={},
                defect_counts={group: dict(factual=0, citation=0) for group in ('generated', 'rejected', 'accepted', 'retained', 'diagnostic')},
                errors=[], review_targets=[], semantic_complete=False,
                observed_factual_defects=0, observed_citation_defects=0,
                reviewed_targets=0, criterion_correct=None,
                usage_complete=False, observed_input_tokens=0,
                observed_output_tokens=0, input_tokens=None, output_tokens=None,
                elapsed_ms=None, observed_input_packet_bytes=0, observed_policy_calls=0, requested_thinking={}, effective_thinking={})


def summarize(case_ids, rows):
    """Every planned case remains in both denominators, including absent reports."""
    require(len(case_ids) == len(set(case_ids)), 'duplicate planned case')
    indexed = {}
    for row in rows:
        key = (row['case_id'], row['arm'])
        require(key[0] in case_ids and key[1] in ARMS and key not in indexed, 'duplicate/unplanned result')
        indexed[key] = row
    result = {}
    for arm in ARMS:
        values = [indexed.get((case, arm), empty_row(case, arm)) for case in case_ids]
        n = len(values)
        sem = sum(v['semantic_complete'] for v in values)
        usage = sum(v['usage_complete'] for v in values)
        correct = sum(v['criterion_correct'] is True for v in values)
        known = sum(v['criterion_correct'] is not None for v in values)
        item = dict(planned_cases=n, completed_cases=sum(v['completed'] for v in values),
                    structurally_valid_cases=sum(v['structural_valid'] for v in values),
                    semantically_audited_cases=sem, unknown_semantic_cases=n-sem,
                    criterion_correct_cases=correct, criterion_incorrect_cases=known-correct,
                    criterion_unknown_cases=n-known, criterion_accuracy=correct/n if n else None,
                    semantic_coverage=sem/n if n else None,
                    reviewed_targets=sum(v['reviewed_targets'] for v in values),
                    observed_review_targets=sum(len(v['review_targets']) for v in values),
                    unknown_usage_cases=n-usage)
        for key in ('factual_defects', 'citation_defects', 'input_tokens', 'output_tokens'):
            observed = sum(v['observed_' + key] for v in values)
            item['observed_' + key] = observed
            item[key] = observed if n and (sem if key.endswith('defects') else usage) == n else None
        item['observed_input_packet_bytes'] = sum(v.get('observed_input_packet_bytes', 0) for v in values)
        item['observed_policy_calls'] = sum(v.get('observed_policy_calls', 0) for v in values)
        item['elapsed_ms'] = sum(v['elapsed_ms'] for v in values) if n and all(v['elapsed_ms'] is not None for v in values) else None
        item['frozen_label_correct_cases'] = sum(v['frozen_label_correct'] is True for v in values)
        item['frozen_label_incorrect_cases'] = sum(v['frozen_label_correct'] is False for v in values)
        item['frozen_label_unknown_cases'] = sum(v['frozen_label_correct'] is None for v in values)
        item['frozen_label_accuracy'] = item['frozen_label_correct_cases'] / n if n and any(v['frozen_expected_state'] is not None for v in values) else None
        item['selected_premises_sufficient_cases'] = sum(v['selected_premises_sufficient'] is True for v in values)
        item['temporally_applicable_cases'] = sum(v['temporally_applicable'] is True for v in values)
        item['supported_final_bearing_cases'] = sum(v['criterion_supported'] is True for v in values)
        for group in ('generated', 'rejected', 'accepted', 'retained', 'diagnostic'):
            coverage = all(v['semantic_complete'] if group == 'diagnostic' else
                v['observation_audit_complete'] and
                (group == 'generated' or all(t['accepted'] is not None for t in v['observation_targets'])) and
                (group != 'retained' or arm == 'direct' or v['retention_verified']) for v in values)
            for kind in ('factual', 'citation'):
                key = group + '_' + kind + '_defects'
                observed = sum(v['defect_counts'][group][kind] for v in values)
                item['observed_' + key] = observed
                item[key] = observed if n and coverage else None
        # Rejected extraction mistakes remain measured, but do not count as
        # accepted or retained errors in the predeclared output-quality gate.
        item['retained_error_gate_passed'] = bool(n and sem == n and correct == n and
            all(v['structural_valid'] and v['completed'] and v['criterion_supported'] is True and
                (v['frozen_expected_state'] is None or v['frozen_label_correct'] is True) for v in values) and
            all(item[group + '_' + kind + '_defects'] == 0 for group in ('accepted', 'retained') for kind in ('factual', 'citation')))
        item['pilot_gate_passed'] = n == 8 and item['retained_error_gate_passed']
        item['all_output_error_free'] = bool(item['retained_error_gate_passed'] and
            item['factual_defects'] == item['citation_defects'] == 0)
        result[arm] = item
    return result


def review_targets(stage, reply):
    """Review every generated text block, even malformed/truncated/failed replies.

    A reviewer inventories all assertions within each exact text block, including
    reasons, questions and private reasoning emitted by the provider. Counts are
    assertions/slots, not an assumption that each JSON object is one assertion.
    """
    result = []
    require(isinstance(reply, dict) and isinstance(reply.get('message'), dict) and isinstance(reply['message'].get('content'), list), 'invalid raw response message')
    for index, block in enumerate(reply['message']['content']):
        if isinstance(block, dict):
            for key in ('text', 'thinking'):
                if isinstance(block.get(key), str) and block[key]:
                    result.append(dict(id=f'{stage}:{index}:{key}', sha256=hashlib.sha256(block[key].encode()).hexdigest(), text=block[key]))
    return result


def semantic_review(row, annotation, report_sha256):
    """Separate raw diagnostic mistakes from attributed accepted/retained defects."""
    if annotation is None:
        return
    try:
        require(annotation['protocol'] == AUDIT_PROTOCOL, 'semantic annotation protocol mismatch')
        require(annotation['report_sha256'] == report_sha256 and annotation['arm'] == row['arm'], 'semantic annotation binding mismatch')
        require(annotation['reviewer_kind'] in ('human', 'agent') and annotation['blinded'] is True
                and annotation['independent'] is True, 'separate label-blinded reviewer required')
        bounded_text(annotation['reviewer'], 256, 'reviewer identity')
        targets = {t['id']: t for t in row['review_targets']}
        seen = set()
        observation_faults = dict(factual=0, citation=0)
        generation_stage = 'generate' if row['arm'] == 'pipeline' else 'direct'
        for entry in annotation['reviews']:
            identifier = entry['target']
            require(identifier in targets and identifier not in seen and entry['sha256'] == targets[identifier]['sha256'], 'semantic target binding mismatch')
            for key in ('assertions_reviewed', 'citation_slots_reviewed', 'factual_defects', 'citation_defects', 'diagnostic_factual_defects', 'diagnostic_citation_defects'):
                require(type(entry[key]) is int and entry[key] >= 0, 'invalid semantic counts')
            require(entry['factual_defects'] <= entry['assertions_reviewed'] and entry['citation_defects'] <= entry['citation_slots_reviewed'], 'defects exceed inspected assertions/slots')
            require(entry['all_assertions_and_citations_reviewed'] is True, 'review target not fully inspected')
            for kind in ('factual', 'citation'):
                all_faults, diagnostics = entry[kind + '_defects'], entry['diagnostic_' + kind + '_defects']
                require(diagnostics <= all_faults, 'diagnostic defects exceed all raw defects')
                require(identifier.startswith(generation_stage + ':') or diagnostics == all_faults, 'unattributed non-observation defects')
                observation_faults[kind] += all_faults - diagnostics
                row['observed_' + kind + '_defects'] += all_faults
                row['defect_counts']['diagnostic'][kind] += diagnostics
            seen.add(identifier)
            row['reviewed_targets'] += 1
        originals = {t['id']: t for t in row['observation_targets']}
        inspected = set()
        for entry in annotation.get('observation_reviews', []):
            identifier = entry['id']
            require(identifier in originals and identifier not in inspected and entry['sha256'] == originals[identifier]['sha256'], 'original observation audit binding mismatch')
            require(type(entry['factual_correct']) is bool and type(entry['original_citation_correct']) is bool, 'missing observation/citation assessment')
            original = originals[identifier]
            for kind, correctness in (('factual', 'factual_correct'), ('citation', 'original_citation_correct')):
                count = entry.get(kind + '_defects', int(not entry[correctness]))
                require(type(count) is int and count >= 0 and (count == 0) == entry[correctness], 'inconsistent attributed defect count')
                require(kind != 'citation' or count <= len(original['citations']), 'too many original citation defects')
                row['defect_counts']['generated'][kind] += count
                if original['accepted'] is False:
                    row['defect_counts']['rejected'][kind] += count
                if original['accepted'] is True:
                    row['defect_counts']['accepted'][kind] += count
                if original['retained'] is True:
                    row['defect_counts']['retained'][kind] += count
            inspected.add(identifier)
        row['observation_audit_complete'] = bool(row['observation_inventory_complete'] and inspected == set(originals))
        if row['observation_audit_complete']:
            require(all(observation_faults[kind] == row['defect_counts']['generated'][kind] for kind in ('factual', 'citation')), 'raw observation defects not fully attributed to originals')
        judgment = annotation.get('criterion')
        if judgment is not None:
            require(judgment['target'] in seen and type(judgment['correct']) is bool, 'unreviewed criterion annotation')
            final_stage = 'appraise' if row['arm'] == 'pipeline' else 'direct'
            require(judgment['target'].startswith(final_stage + ':'), 'criterion annotation does not review final judgment')
            context = row['criterion_review']
            require(context is not None and judgment['binding_sha256'] == context['sha256'], 'criterion selected-premise/context binding mismatch')
            row['criterion_correct'] = judgment['correct']
            require(type(judgment['selected_premises_sufficient']) is bool and type(judgment['temporally_applicable']) is bool, 'explicit sufficiency and current temporal applicability review required')
            row['selected_premises_sufficient'] = judgment['selected_premises_sufficient']
            row['temporally_applicable'] = judgment['temporally_applicable']
            row['criterion_supported'] = context['state'] == 'unknown' or (row['selected_premises_sufficient'] and row['temporally_applicable'])
        row['semantic_complete'] = bool(targets and seen == set(targets) and row['observation_audit_complete'] and
            row['criterion_correct'] is not None and row['criterion_supported'] is not None and
            set(STAGES[row['arm']]) <= {identifier.split(':', 1)[0] for identifier in seen})
    except ERRORS as error:
        row['semantic_complete'] = False
        row['observation_audit_complete'] = False
        row['errors'].append('semantic review: ' + str(error))


def catalog(packet):
    # The catalog and metadata format deliberately retain v3 identity. This is
    # raw Python reconstruction, never a Rust expected-admission result.
    adapted = dict(packet, protocol='situated-support-wire-v3')
    return legacy.validate_catalog(adapted)


def observations(packet, payload):
    fields(payload, ('request_key', 'observations'), 'generation reply')
    require(payload['request_key'] == packet['request_key'], 'generation request binding mismatch')
    handles = catalog(packet)
    offered = payload['observations']
    require(isinstance(offered, list) and len(offered) <= 8, 'observation bound exceeded')
    units = []
    for index, observation in enumerate(offered):
        fields(observation, ('text', 'evidence'), 'observation')
        bounded_text(observation['text'], 512, 'observation text')
        require(isinstance(observation['evidence'], str) and observation['evidence'] in handles, 'foreign original evidence handle')
        units.append(dict(id=f'o{index+1}', kind='RecordedEvent', text=observation['text'], citations=[copy.deepcopy(handles[observation['evidence']])]))
    return dict(units=units, judgments=[])


def factual_checks(packet, payload, proposal):
    fields(payload, ('request_key', 'checks'), 'factual reply')
    require(payload['request_key'] == packet['request_key'], 'check request binding mismatch')
    require(packet['proposal'] == proposal, 'checker changed original observation/citation slots')
    offered = {f'unit:{u["id"]}': u for u in proposal['units']}
    require(isinstance(payload['checks'], list) and len(payload['checks']) == len(offered), 'exact check coverage mismatch')
    checked = {}
    for check in payload['checks']:
        fields(check, ('target', 'status', 'original_citation', 'reason', 'questions'), 'fact check')
        target = check['target']
        require(isinstance(target, str) and target in offered and target not in checked, 'foreign/duplicate fact check')
        require(check['status'] in ('Supported', 'Contradicted', 'Unknown') and
                check['original_citation'] in ('DirectSupport', 'ContextOnly', 'Contradiction', 'Uncertain'), 'invalid fact check verdict')
        diagnostic(check, check['status'] == 'Unknown')
        require(check['status'] != 'Supported' or (check['original_citation'] == 'DirectSupport' and not check['questions']), 'supported fact lacks original direct support')
        checked[target] = copy.deepcopy(check)
    return checked


def appraisal(packet, payload, proposal, checks):
    fields(payload, ('request_key', 'state', 'because', 'reason', 'questions'), 'appraisal reply')
    require(payload['request_key'] == packet['request_key'], 'appraisal request binding mismatch')
    require(payload['state'] in ('met', 'not_met', 'unknown'), 'invalid criterion state')
    diagnostic(payload, payload['state'] == 'unknown')
    supported = {u['id'] for u in proposal['units'] if checks['unit:' + u['id']]['status'] == 'Supported'}
    because = payload['because']
    require(isinstance(because, list) and all(isinstance(i, str) for i in because)
            and len(because) == len(set(because)) and set(because) <= supported, 'appraisal selected foreign or unsupported premise')
    require(payload['state'] == 'unknown' or bool(because) and not payload['questions'], 'known appraisal needs supported premises and no questions')
    return copy.deepcopy(payload)


def canonical_checks(proposal, checks):
    result = []
    units = {'unit:' + u['id']: u for u in proposal['units']}
    for check in checks.values():
        unit = units[check['target']]
        result.append(dict(target=check['target'], status=check['status'],
            unsupported_factual_premise=check['original_citation'] == 'Contradiction',
            citations=unit['citations'] if check['original_citation'] == 'DirectSupport' else [],
            reason=check['reason'], questions=check['questions']))
    return {'checks': result}


def response_payload(reply):
    content = reply['message']['content']
    require(isinstance(content, list) and all(isinstance(b, dict) for b in content), 'invalid reply content')
    text = ''.join(b['text'] for b in content if b.get('type') == 'text' and isinstance(b.get('text'), str))
    require(len(text.encode()) <= 16384, 'oversized reply')
    return decode(text)


def request_packet(request, stage, name, model):
    effective, wire = request['request'], request['wire_body']
    schema = effective['response_schema']
    require(hashlib.sha256(effective['system'].encode()).hexdigest() == FRAME_SHA256[name], 'policy frame changed or contains dynamic context')
    require(wire.get('response_format') == {'type': 'json_object', 'schema': schema}, 'actual response schema differs')
    messages = effective['messages']
    require(len(messages) == 1 and messages[0]['role'] == 'user' and len(messages[0]['content']) == 1, 'extra request history')
    block = messages[0]['content'][0]
    require(block['type'] == 'text', 'nontext request')
    text = block['text']
    require(text.startswith('<support-input>\n') and text.endswith('\n</support-input>'), 'invalid request wrapper')
    packet = decode(text[len('<support-input>\n'):-len('\n</support-input>')])
    require(packet['protocol'] == WIRE, 'wrong wire protocol')
    require(effective.get('tools') == [] and not wire.get('tools') and
            wire['messages'] == [{'role': 'system', 'content': effective['system']}, {'role': 'user', 'content': text}], 'actual wire input/tools differ')
    require(effective['model'] == wire['model'] == model and effective['local_only'] is True and
            effective['max_tokens'] == wire['max_tokens'] == LIMITS[name] and wire['seed'] == SEED,
            'model/seed/output allowance mismatch')
    require(effective.get('think') is False and wire.get('chat_template_kwargs', {}).get('enable_thinking') is False, 'unrecorded thinking control')
    variants = schema.get('oneOf', [schema])
    require(isinstance(variants, list) and bool(variants), 'missing schema variants')
    if 'oneOf' in schema:
        fields(schema, ('oneOf',), 'complete schema alternatives')
    for variant in variants:
        require(variant.get('type') == 'object' and variant.get('additionalProperties') is False and
                variant['properties']['request_key'] == {'type': 'string', 'enum': [packet['request_key']]}, 'unbound complete schema branch')
    expected_fields = {'generate': {'request_key', 'observations'}, 'check': {'request_key', 'checks'},
        'appraise': {'request_key', 'state', 'because', 'reason', 'questions'},
        'direct': {'request_key', 'observations', 'criterion'}}[name]
    require(all(set(v.get('required', [])) == set(v['properties']) == expected_fields for v in variants), 'wrong response schema branch fields')
    size = len(effective['system'].encode()) + len(text.encode()) + len(legacy.compact_bytes(schema))
    require(size == stage['packet_bytes'] and size <= 49152, 'packet bytes/bound mismatch')
    if name in ('generate', 'direct'):
        fields(packet, ('protocol', 'criterion', 'sources', 'evidence_metadata', 'request_key', 'catalog'), 'evidence-only request')
    elif name == 'check':
        fields(packet, ('protocol', 'criterion', 'sources', 'evidence_metadata', 'request_key', 'catalog', 'proposal', 'targets'), 'check request')
    else:
        fields(packet, ('protocol', 'criterion', 'sources', 'evidence_metadata', 'request_key', 'catalog', 'proposal', 'checks', 'supported_ids'), 'appraisal request')
    catalog(packet)
    require(schema == expected_schema(packet, name), 'response schema differs from independent v4 contract')
    return packet


def inspect_call(root, stage, name, model, row):
    """Collect usage/review evidence before enforcing successful call completion."""
    reply = None
    events = []
    audit_error = None
    try:
        events = [decode(line) for line in artifact_bytes(Path(root), stage['provider_audit']).decode().splitlines() if line.strip()]
    except ERRORS as error:
        audit_error = error
    reply_error = None
    if stage.get('reply'):
        try:
            reply = read(root, stage['reply'])
        except ERRORS as error:
            reply_error = error
    account_call_usage(row, name, stage, events, reply, audit_error is None)
    observed_responses = [reply] if reply is not None else [e for e in events if e.get('event') == 'response']
    for observed in observed_responses:
        row['review_targets'].extend(review_targets(name, observed))
        observe_final_state(row, name, observed)
    for index, event in enumerate(events):
        rejected = event.get('rejected_response')
        if event.get('event') == 'error' and isinstance(rejected, dict):
            row['review_targets'].extend(review_targets(name + ':rejected:' + str(index), rejected))
            observe_final_state(row, name, rejected)
    if audit_error is not None:
        raise audit_error
    if reply_error is not None:
        raise reply_error
    request = read(root, stage['request'])
    packet = request_packet(request, stage, name, model)
    row['observed_input_packet_bytes'] += stage['packet_bytes']
    row['requested_thinking'][name] = request['request'].get('think')
    row['effective_thinking'][name] = request['wire_body'].get('chat_template_kwargs', {}).get('enable_thinking')
    require([e for e in events if e.get('event') == 'request_started'] == [request], 'request audit mismatch or duplicate call')
    require(reply is not None and [e for e in events if e.get('event') == 'response'] == [reply], 'reply audit mismatch or missing response')
    require(request.get('call') == reply.get('call') == 1 and not any(e.get('event') in ('error', 'stream_refused') for e in events), 'provider call failure/retry')
    require(reply.get('model') == model and reply.get('stop_reason') == 'end_turn' and not reply.get('refusal'), 'incomplete/refused/wrong-model response')
    require(stage.get('model') == model and stage.get('stop_reason') == reply['stop_reason'], 'stage model/stop metadata differ')
    require(stage.get('status') == 'completed' and stage.get('error') is None, 'stage did not complete')
    for key in ('input_tokens', 'output_tokens'):
        require(type(reply.get(key)) is int and reply[key] >= 0 and reply[key] == stage.get(key), 'unknown or inconsistent usage')
    require(reply['output_tokens'] <= LIMITS[name], 'stage output bound exceeded')
    return packet, response_payload(reply)


def criterion_from_case(case):
    native = case['native']
    criterion = native['current_criterion']
    return dict(goal='task:synthetic-support', goal_text=native['goal_text'], criterion_text=criterion,
        criterion_revision=hashlib.sha256(criterion.encode()).hexdigest(),
        confirmation_digest=hashlib.sha256(('synthetic-owner-confirmation:' + criterion).encode()).hexdigest())


def validate_initial(root, case, report):
    for key in ('initial_input', 'native_transcript', 'observed_output'):
        descriptor = report['artifacts'].get(key)
        if descriptor is not None:
            artifact_bytes(Path(root), descriptor)
    seal = read(root, report['source_seal'])
    require(seal == report['initial_hashes'] == {k: (report['artifacts'][k]['sha256'] if report['artifacts'].get(k) else None) for k in ('initial_input', 'native_transcript', 'observed_output')}, 'initial source seal differs')
    initial = read(root, report['initial_evidence'])
    fields(initial, ('criterion', 'sources', 'evidence_metadata'), 'initial evidence')
    require(report['initial_evidence_sha256'] == report['initial_evidence']['sha256'], 'initial evidence seal mismatch')
    require(initial['criterion'] == criterion_from_case(case), 'current criterion differs from frozen case')
    legacy.audit_capture_metadata(Path(root), report, initial)
    require(any(s['text'] == case['native']['input_text'] and s['kind'] == 'Observation' for s in initial['sources']), 'frozen input missing')
    observed = [s for s in initial['sources'] if s['kind'] == 'ArtifactObservation']
    require(len(observed) == 1 and observed[0]['field'] == '/artifact_observation', 'artifact source missing/ambiguous')
    artifact = decode(observed[0]['text'])
    text = case['native']['observed_output']
    require(artifact['artifact_text'] == text and artifact['artifact_digest'] == (hashlib.sha256(text.encode()).hexdigest() if text is not None else None)
        and artifact['content_status'] == ('Complete' if text is not None else 'Missing'), 'frozen observed bytes differ')
    return initial


def audit_case(root, case, model, annotations=None):
    root = Path(root)
    rows = [empty_row(case['id'], arm) for arm in ARMS]
    for row in rows:
        try:
            apply_frozen_label(row, case)
        except ERRORS as error:
            row['errors'].append('frozen label: ' + str(error))
    try:
        raw = legacy.bounded_read(root / 'report.json', legacy.MAX_REPORT_BYTES)
        report = decode(raw.decode())
        require(isinstance(report, dict), 'report must be an object')
        report_sha256 = hashlib.sha256(raw).hexdigest()
    except ERRORS as error:
        for row in rows:
            row['errors'].append('report unavailable: ' + str(error))
        return rows
    shared_errors = []
    initial = None
    try:
        require(report['protocol'] == PROTOCOL and report['case_id'] == case['id'] and
                report['case_sha256'] == digest(case) and report['model'] == model and report['seed'] == SEED, 'report identity mismatch')
        require(type(report['case_index']) is int and report['case_index'] >= 0 and
                report['arm_order'] == (['pipeline', 'direct'] if report['case_index'] % 2 == 0 else ['direct', 'pipeline']), 'arm order mismatch')
        initial = validate_initial(root, case, report)
    except ERRORS as error:
        shared_errors.append(str(error))
    all_calls = {}
    for row in rows:
        row['report_sha256'] = report_sha256
        row['errors'].extend(shared_errors)
        arm = row['arm']
        arms = report.get('arms', {})
        details = arms.get(arm, {}) if isinstance(arms, dict) else {}
        if not isinstance(details, dict):
            details = {}
        calls = {}
        for name in STAGES[arm]:
            try:
                calls[name] = inspect_call(root, details.get('stages', {}).get(name, {}), name, model, row)
                all_calls[name] = calls[name]
                if initial is not None:
                    packet = calls[name][0]
                    require(all(packet[k] == initial[k] for k in initial), 'call changed initial evidence/criterion/metadata')
            except ERRORS as error:
                row['errors'].append(name + ': ' + str(error))
        refresh_usage(row)
        prepare_semantic_context(row, calls)
        try:
            expected_stages = set(STAGES[arm]) | (set(('retain', 'recall', 'changed_goal', 'historical')) if arm == 'pipeline' else set())
            actual_stages = set(details.get('stages', {}))
            require(actual_stages == expected_stages or (arm == 'pipeline' and actual_stages == expected_stages | {'stored_export'}), 'unknown/missing stages')
            elapsed = details['elapsed_ms']
            require(type(elapsed) is int and 0 <= elapsed <= 540000, 'arm deadline exceeded or unknown')
            row['elapsed_ms'] = elapsed
            require(len(calls) == len(STAGES[arm]), 'incomplete raw exchanges')
            require(details['initial_evidence_sha256'] == report['initial_evidence']['sha256'] and details['model'] == model and details['seed'] == SEED, 'arm initial evidence/model/seed mismatch')
            require(details['output_token_allowance'] == 6144 and details['deadline_seconds'] == 540, 'arm budget mismatch')
            require(details['policy_calls_started'] == row['observed_policy_calls'] and details['input_packet_bytes'] == row['observed_input_packet_bytes'], 'arm call/input overhead totals differ')
            require(all(details[k] == row['observed_' + k] for k in ('input_tokens', 'output_tokens')), 'arm usage totals differ')
            validate_bindings(root, report, details, calls, initial)
            if arm == 'pipeline':
                validate_pipeline(root, details, calls, initial, model, row)
            else:
                validate_direct(root, details, calls['direct'])
            require(details['status'] == 'completed' and details.get('error') is None, 'arm incomplete')
            row['completed'] = True
        except ERRORS as error:
            row['errors'].append('arm: ' + str(error))
        try:
            apply_frozen_label(row, case)
        except ERRORS as error:
            row['errors'].append('frozen label: ' + str(error))
        row['structural_valid'] = not row['errors']
        semantic_review(row, (annotations or {}).get(arm), report_sha256)
    if 'generate' in all_calls and 'direct' in all_calls and all_calls['generate'][0] != all_calls['direct'][0]:
        for row in rows:
            row['errors'].append('pipeline/direct initial request packets differ')
            row['structural_valid'] = False
    return rows


def validate_direct(root, arm, exchange):
    packet, payload = exchange
    fields(arm['artifacts'], ('result',), 'direct artifacts')
    require(arm.get('independently_checked_retention') is False, 'direct arm claimed independent retention')
    fields(payload, ('request_key', 'observations', 'criterion'), 'direct reply')
    proposal = observations(packet, {k: payload[k] for k in ('request_key', 'observations')})
    criterion = payload['criterion']
    fields(criterion, ('state', 'because', 'reason', 'questions'), 'direct criterion')
    require(criterion['state'] in ('met', 'not_met', 'unknown'), 'invalid direct criterion')
    because = criterion['because']
    require(isinstance(because, list) and len(because) <= 8 and all(type(i) is int and 0 <= i < len(proposal['units']) for i in because) and len(because) == len(set(because)), 'invalid direct premise indices')
    require(criterion['state'] == 'unknown' or bool(because), 'known direct needs premises')
    diagnostic(criterion, criterion['state'] == 'unknown')
    require(criterion['state'] == 'unknown' or not criterion['questions'], 'known direct criterion has questions')
    exported = read(root, arm['artifacts']['result'])
    require(exported == dict(proposal=proposal, criterion=criterion, premise_ids=[proposal['units'][i]['id'] for i in because]), 'direct result differs from raw response')


def validate_pipeline(root, arm, calls, initial, model, row=None):
    generation_packet, generation = calls['generate']
    check_packet, check = calls['check']
    appraisal_packet, final = calls['appraise']
    require(len({p['request_key'] for p in (generation_packet, check_packet, appraisal_packet)}) == 3, 'phases reused a request binding')
    proposal = observations(generation_packet, generation)
    require(check_packet['targets'] == ['unit:' + u['id'] for u in proposal['units']], 'checker targets differ from numbered originals')
    checked = factual_checks(check_packet, check, proposal)
    canonical = canonical_checks(proposal, checked)
    require(appraisal_packet['proposal'] == proposal and appraisal_packet['checks'] == canonical['checks'], 'appraiser rewrote original slots/checks')
    supported = [u['id'] for u in proposal['units'] if checked['unit:' + u['id']]['status'] == 'Supported']
    require(appraisal_packet['supported_ids'] == supported, 'appraiser supported IDs differ from exact checks')
    final = appraisal(appraisal_packet, final, proposal, checked)
    artifacts = arm['artifacts']
    allowed = {'numbered_observations', 'fact_checks', 'appraisal', 'retained', 'receipt', 'database', 'recall', 'historical', 'changed_goal', 'checking_binding_input', 'appraisal_binding_input'}
    require(isinstance(artifacts, dict) and set(artifacts) <= allowed, 'unknown pipeline artifacts')
    require(arm.get('independently_checked_retention') is True, 'pipeline retention unavailable')
    require(read(root, artifacts['numbered_observations']) == proposal, 'host numbered observation export differs')
    require(read(root, artifacts['fact_checks']) == canonical, 'canonical factual export differs')
    require(read(root, artifacts['appraisal']) == final, 'appraisal export differs from raw response')
    retained = read(root, artifacts['retained'])
    validate_retained(retained, proposal, checked, final, initial, model,
                      [p['request_key'] for p in (generation_packet, check_packet, appraisal_packet)])
    validate_persistence(root, artifacts, retained, initial, row)
    for name in ('retain', 'recall', 'historical', 'changed_goal') + (('stored_export',) if 'stored_export' in arm['stages'] else ()):
        require(arm['stages'][name]['status'] == 'completed' and arm['stages'][name].get('error') is None, 'incomplete persistence/control stage: ' + name)


def validate_retained(retained, proposal, checks, final, initial, model, bindings):
    """Reconstruct accepted content and selected premise indices from raw replies."""
    fields(retained, ('context_key', 'criterion', 'host_context_id', 'draft', 'claim_sources'), 'durable interpretation')
    require(retained['criterion'] == initial['criterion'], 'retained criterion changed')
    draft = retained['draft']
    fields(draft, ('interpretation', 'claims', 'judgments', 'prediction', 'grounding', 'model', 'producer_version', 'support'), 'durable draft')
    accepted = [u for u in proposal['units'] if checks['unit:' + u['id']]['status'] == 'Supported']
    claims = [dict(source_id=c['source_id'], statement=u['text'], quote=c['quote']) for u in accepted for c in u['citations']]
    indices = {u['id']: index for index, u in enumerate(accepted)}
    bearing = {'met': 'Good', 'not_met': 'Bad', 'unknown': 'Unknown'}[final['state']]
    judgments = [dict(goal=initial['criterion']['goal'], bearing=bearing,
        because=[indices[i] for i in final['because']] if bearing != 'Unknown' else [])]
    require(draft['claims'] == claims and draft['judgments'] == judgments and draft['prediction'] is None, 'retained facts/judgment differ from checked originals')
    require(draft['interpretation'] == ('\n'.join('Recorded event: ' + u['text'] for u in accepted) if accepted else 'No supported statements retained.'), 'retained prose differs')
    require(draft['model'] == model and draft['producer_version'] == 'situated-interpretation-v4', 'retained producer mismatch')
    selected = {c['source_id'] for c in claims}
    locators = [{k: s[k] for k in ('id', 'source', 'field', 'kind')} for s in initial['sources'] if s['id'] in selected]
    require(retained['claim_sources'] == locators, 'retained original source locator differs')
    support = draft['support']
    fields(support, ('protocol', 'generator_model', 'evaluator_model', 'proposal_hash', 'request_hash', 'units', 'judgments', 'atomic'), 'durable support')
    require(all(isinstance(support[k], str) and re.fullmatch('[0-9a-f]{64}', support[k]) for k in ('proposal_hash', 'request_hash')), 'invalid support identity digest')
    require(support['protocol'] == 'situated-atomic-appraisal-v4' and support['generator_model'] == support['evaluator_model'] == model, 'durable support identity mismatch')
    require(support['atomic'] == dict(extraction_binding=bindings[0], checking_binding=bindings[1], appraisal_binding=bindings[2], appraiser_model=model), 'durable phase bindings differ')
    expected_units = []
    for unit in proposal['units']:
        status = checks['unit:' + unit['id']]['status']
        expected_units.append(dict(disposition='Accepted', unit=unit, status=status) if status == 'Supported' else dict(disposition='Rejected', id=unit['id'], status=status))
    require(support['units'] == expected_units, 'durable dispositions differ from raw checks')
    expected_judgment = dict(index=0, goal=initial['criterion']['goal'], offered_bearing=bearing, bearing=bearing, because=final['because'], status='Unknown' if bearing == 'Unknown' else 'Supported', reason='ExplicitUnknown' if bearing == 'Unknown' else 'Retained')
    require(support['judgments'] == [expected_judgment], 'durable derived premise selection differs')
    dropped_by = {}
    for unit in proposal['units']:
        status = checks['unit:' + unit['id']]['status']
        if status != 'Supported':
            dropped_by[status] = dropped_by.get(status, 0) + 1
    require(draft['grounding'] == dict(offered=len(proposal['units']), dropped=sum(dropped_by.values()), dropped_by=dropped_by, unresolved_goals=0), 'durable grounding differs')


def freeze_implementation(output, binary, source_paths):
    """Seal implementation/auditor before an independent author creates cases."""
    require(bool(source_paths), 'freeze requires explicit implementation source files')
    paths = [Path(__file__).resolve(), Path(legacy.__file__).resolve(), Path(binary).resolve()]
    paths.extend(Path(path).resolve() for path in source_paths)
    manifest = dict(protocol=PROTOCOL, semantic_audit_protocol=AUDIT_PROTOCOL, quality_gate=QUALITY_GATE, binary=str(Path(binary).resolve()),
        files={str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths})
    with Path(output).open('x') as stream:
        json.dump(manifest, stream, indent=2)
        stream.write('\n')
    return manifest


def freeze(output, fixture, binary, model, source_paths, implementation_manifest):
    """Freeze an eight-case trial after independent authoring; never overwrite."""
    implementation = decode(Path(implementation_manifest).read_text())
    require(implementation['protocol'] == PROTOCOL and implementation['binary'] == str(Path(binary).resolve()), 'implementation freeze identity mismatch')
    require(implementation['semantic_audit_protocol'] == AUDIT_PROTOCOL and implementation['quality_gate'] == QUALITY_GATE, 'implementation semantic gate mismatch')
    for path, expected in implementation['files'].items():
        require(hashlib.sha256(Path(path).read_bytes()).hexdigest() == expected, 'implementation changed after case-authoring boundary: ' + path)
    fixture = Path(fixture).resolve()
    cases = decode(legacy.bounded_read(fixture, 1024 * 1024).decode())['cases']
    require(len(cases) == 8 and len({c['id'] for c in cases}) == 8, 'comparison requires eight distinct planned cases')
    paths = [Path(__file__).resolve(), Path(legacy.__file__).resolve(), Path(binary).resolve(), fixture]
    paths.extend(Path(path).resolve() for path in source_paths)
    manifest = dict(protocol=PROTOCOL, semantic_audit_protocol=AUDIT_PROTOCOL, quality_gate=QUALITY_GATE, model=model, seed=SEED, case_ids=[c['id'] for c in cases],
        case_hashes={c['id']: digest(c) for c in cases}, fixture=str(fixture), binary=str(Path(binary).resolve()),
        output_tokens=6144, arm_deadline_seconds=540, pipeline_allocations=LIMITS,
        files={**implementation['files'], **{str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}},
        implementation_manifest_sha256=hashlib.sha256(Path(implementation_manifest).read_bytes()).hexdigest())
    with Path(output).open('x') as stream:
        json.dump(manifest, stream, indent=2)
        stream.write('\n')
    return manifest


def score(manifest_path, root, annotation_path=None):
    manifest = decode(Path(manifest_path).read_text())
    require(manifest['protocol'] == PROTOCOL and manifest['semantic_audit_protocol'] == AUDIT_PROTOCOL and manifest['quality_gate'] == QUALITY_GATE, 'wrong manifest protocol or semantic gate')
    for path, expected in manifest['files'].items():
        require(hashlib.sha256(Path(path).read_bytes()).hexdigest() == expected, 'frozen file changed: ' + path)
    cases = decode(Path(manifest['fixture']).read_text())['cases']
    require([c['id'] for c in cases] == manifest['case_ids'] and {c['id']: digest(c) for c in cases} == manifest['case_hashes'], 'frozen cohort changed')
    annotations = decode(Path(annotation_path).read_text()) if annotation_path else {}
    rows = []
    for case in cases:
        require(re.fullmatch('[a-z0-9][a-z0-9-]*', case['id']) is not None, 'unsafe case directory')
        rows.extend(audit_case(Path(root) / case['id'], case, manifest['model'], annotations.get(case['id'])))
    return dict(protocol=PROTOCOL, semantic_audit_protocol=AUDIT_PROTOCOL, quality_gate=QUALITY_GATE, rows=rows, summary=summarize(manifest['case_ids'], rows),
        boundary='Structural checks are not semantic truth. Missing independent blinded reviews and usage remain unknown; observed failures from incomplete arms remain counted.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    implementation = commands.add_parser('freeze-implementation')
    implementation.add_argument('--output', required=True)
    implementation.add_argument('--binary', required=True)
    implementation.add_argument('--source', action='append', required=True)
    frozen = commands.add_parser('freeze')
    frozen.add_argument('--output', required=True)
    frozen.add_argument('--implementation', required=True)
    frozen.add_argument('--fixture', required=True)
    frozen.add_argument('--binary', required=True)
    frozen.add_argument('--model', required=True)
    frozen.add_argument('--source', action='append', default=[])
    scoring = commands.add_parser('score')
    scoring.add_argument('--manifest', required=True)
    scoring.add_argument('--root', required=True)
    scoring.add_argument('--annotations')
    args = parser.parse_args()
    if args.command == 'freeze-implementation':
        result = freeze_implementation(args.output, args.binary, args.source)
    elif args.command == 'freeze':
        result = freeze(args.output, args.fixture, args.binary, args.model, args.source, args.implementation)
    else:
        result = score(args.manifest, args.root, args.annotations)
    print(json.dumps(result, indent=2))


def validate_recall_record(record, retained):
    """Independently reconstruct the versioned presentation from canonical storage."""
    fields(record, ('kind', 'data'), 'recalled record')
    if record['kind'] == 'interpretation':
        expected = retained
    else:
        require(record['kind'] == 'interpretation_reader_v1', 'unknown interpretation presentation')
        draft = retained['draft']
        require(draft['producer_version'] == 'situated-interpretation-v4' and
                draft['support']['protocol'] == 'situated-atomic-appraisal-v4' and
                isinstance(draft['support']['atomic'], dict), 'compact presentation requires canonical atomic v4')
        expected = {key: retained[key] for key in ('context_key', 'criterion', 'host_context_id', 'claim_sources')}
        expected.update(producer_version=draft['producer_version'], support=draft['support'])
    require(record['data'] == expected, 'ordinary recall differs from durable record')


def validate_persistence(root, artifacts, retained, initial, row=None):
    import sqlite3
    receipt = read(root, artifacts['receipt'])
    reference = receipt['reference']
    connection = sqlite3.connect(':memory:')
    try:
        connection.deserialize(artifact_bytes(Path(root), artifacts['database']))
        records = connection.execute("SELECT payload FROM revision WHERE id=? AND object_id=? AND lifecycle='active'", (reference['revision'], reference['object'])).fetchall()
        require(len(records) == 1 and decode(records[0][0]) == dict(kind='interpretation', data=retained), 'durable SQLite record differs from retained export')
        receipts = connection.execute("SELECT receipt_id,context_key FROM interpretation_receipt WHERE revision_id=? AND object_id=? AND state='active'", (reference['revision'], reference['object'])).fetchall()
        require(receipts == [(receipt['id'], retained['context_key'])], 'durable receipt identity differs')
        dependencies = {row[0] for row in connection.execute('WITH RECURSIVE inputs(id) AS (SELECT input FROM dependency WHERE output=? UNION SELECT d.input FROM dependency d JOIN inputs i ON d.output=i.id) SELECT id FROM inputs', (reference['revision'],))}
        require({s['source']['revision'] for s in initial['sources']} <= dependencies, 'durable record dropped consulted source dependencies')
    except sqlite3.Error as error:
        raise ValueError('invalid durable database: ' + str(error)) from error
    finally:
        connection.close()
    if row is not None:
        row['retention_verified'] = True
        row['retained_bearing'] = retained['draft']['judgments'][0]['bearing']
        for target in row['observation_targets']:
            target['retained'] = target['accepted']
    recall = read(root, artifacts['recall'])['memory']
    require(any(r['reference'] == reference and r['role'] == 'SelectedCurrent' for r in recall['interpretation_roles']), 'ordinary recall did not select durable record')
    matching = [r for r in recall['records'] if r['reference'] == reference]
    require(len(matching) == 1, 'ordinary recall lacks unique durable record')
    validate_recall_record(matching[0]['record'], retained)
    if row is not None:
        row['recall_verified'] = True
    for name in ('historical', 'changed_goal'):
        control = read(root, artifacts[name])['memory']
        require(not any(r['role'] == 'SelectedCurrent' for r in control.get('interpretation_roles', [])), name + ' promoted old interpretation into current context')





def expected_schema(packet, stage):
    """Independent complete-branch contract for the pinned grammar converter."""
    def obj(properties):
        return dict(type='object', additionalProperties=False,
                    required=list(properties), properties=properties)
    text = dict(type='string', minLength=1, maxLength=128)
    questions = dict(type='array', maxItems=4, items=dict(type='string', minLength=1, maxLength=64))
    handles = list(catalog(packet))
    observation = obj(dict(text=text, evidence=dict(type='string', enum=handles) if handles else dict(type='string')))
    obs = dict(type='array', maxItems=8 if handles else 0, items=observation)
    binding = dict(type='string', enum=[packet['request_key']])
    def criterion(because, include_binding=False, known_allowed=True):
        branches = []
        for known in (True, False):
            if known and not known_allowed:
                continue
            premises = dict(because, minItems=1 if known else 0)
            branch = obj(dict(state=dict(type='string', enum=['met', 'not_met'] if known else ['unknown']),
                because=premises, reason=text, questions=dict(questions,minItems=0 if known else 1,maxItems=0 if known else 4)))
            if include_binding:
                branch['required'].append('request_key')
                branch['properties']['request_key'] = binding
            branches.append(branch)
        return dict(oneOf=branches)
    if stage == 'generate':
        return obj(dict(request_key=binding, observations=obs))
    if stage == 'direct':
        premises = dict(type='array', maxItems=8, uniqueItems=True, items=dict(type='integer', minimum=0, maximum=7))
        return obj(dict(request_key=binding, observations=obs, criterion=criterion(premises)))
    if stage == 'appraise':
        ids = packet['supported_ids']
        because = dict(type='array', maxItems=8, uniqueItems=True, items=dict(type='string', enum=ids)) if ids else dict(type='array', maxItems=0, items=dict(type='null'))
        return criterion(because, include_binding=True, known_allowed=bool(ids))
    targets = packet['targets']
    if not targets:
        array = dict(type='array', minItems=0, maxItems=0, items=dict(type='null'))
    else:
        branches = []
        for status, lower, upper in (('Supported', 0, 0), ('Unknown', 1, 4), ('Contradicted', 0, 4)):
            relations = ['DirectSupport'] if status == 'Supported' else ['DirectSupport', 'ContextOnly', 'Contradiction', 'Uncertain']
            branches.append(obj(dict(target=dict(type='string',enum=targets), status=dict(type='string',enum=[status]),
                original_citation=dict(type='string',enum=relations), reason=text, questions=dict(questions,minItems=lower,maxItems=upper))))
        array = dict(type='array',minItems=len(targets),maxItems=len(targets),items=dict(oneOf=branches))
    return obj(dict(request_key=binding,checks=array))


def binding_hash(prepared_request_id, metadata, phase, input_string):
    """Hash the declared Rust struct order, independently of JSON export ordering."""
    require(isinstance(prepared_request_id, str) and re.fullmatch('[0-9a-f]{64}', prepared_request_id), 'invalid opaque prepared identity')
    ordered = []
    for row in metadata:
        captures = []
        for capture in row['captures']:
            captures.append(dict(capture=dict(object=capture['capture']['object'], revision=capture['capture']['revision']),
                relation=capture['relation'], capture_time=capture['capture_time'],
                passage_time=capture['passage_time'], transcript_line=capture['transcript_line']))
        ordered.append(dict(source_id=row['source_id'], role=row['role'], interpretation_influenced=row['interpretation_influenced'], captures=captures))
    payload = ['mecha-atomic-support-v4', prepared_request_id, ordered, phase, input_string]
    return hashlib.sha256(json.dumps(payload, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()


def validate_bindings(root, report, arm, calls, initial):
    context = read(root, report['binding_context'])
    fields(context, ('prepared_request_id', 'generation_input', 'evidence_metadata_json'), 'host binding context')
    require(isinstance(context['generation_input'], str) and decode(context['generation_input']) == {k: initial[k] for k in ('criterion', 'sources')}, 'extraction binding input differs from initial evidence')
    metadata = initial['evidence_metadata']
    require(isinstance(context['evidence_metadata_json'], str) and decode(context['evidence_metadata_json']) == metadata, 'raw typed metadata differs from authenticated source metadata')
    identifier = context['prepared_request_id']
    extraction = binding_hash(identifier, metadata, 'atomic-v4-extraction', context['generation_input'])
    if 'direct' in calls:
        require(calls['direct'][0]['request_key'] == extraction, 'direct request key does not bind prepared identity and initial evidence')
        return
    generation, checking, final = (calls[name][0] for name in ('generate', 'check', 'appraise'))
    require(generation['request_key'] == extraction, 'generation request key does not bind prepared identity and initial evidence')
    check_input = read(root, arm['artifacts']['checking_binding_input'])
    expected = {k: checking[k] for k in ('criterion', 'sources', 'proposal', 'targets')}
    expected['protocol'] = 'situated-factual-support-v1'
    require(isinstance(check_input, str) and decode(check_input) == expected, 'checking binding input changed original proposal or source packet')
    require(checking['request_key'] == binding_hash(identifier, metadata, 'atomic-v4-checking', check_input), 'checking request key hash mismatch')
    appraisal_input = read(root, arm['artifacts']['appraisal_binding_input'])
    require(isinstance(appraisal_input, str) and decode(appraisal_input) == [final['proposal'], dict(checks=final['checks'])], 'appraisal binding input changed original slots or exact checks')
    require(final['request_key'] == binding_hash(identifier, metadata, 'atomic-v4-appraisal', appraisal_input), 'appraisal request key hash mismatch')



def prepare_semantic_context(row, calls):
    """Attribute original atoms using raw replies; Rust admission is not an oracle."""
    generation_stage = 'generate' if row['arm'] == 'pipeline' else 'direct'
    if generation_stage not in calls:
        return
    try:
        packet, raw = calls[generation_stage]
        proposal = observations(packet, {k: raw[k] for k in ('request_key', 'observations')})
        row['observation_targets'] = []
        for index, unit in enumerate(proposal['units']):
            identity = dict(index=index, **unit)
            row['observation_targets'].append(dict(identity, sha256=digest(identity),
                accepted=True if row['arm'] == 'direct' else None,
                retained=False if row['arm'] == 'direct' else None))
        row['observation_inventory_complete'] = True
        if row['arm'] == 'pipeline':
            if 'check' not in calls:
                return
            checked = factual_checks(*calls['check'], proposal)
            for target in row['observation_targets']:
                target['accepted'] = checked['unit:' + target['id']]['status'] == 'Supported'
                if row['retention_verified']:
                    target['retained'] = target['accepted']
            if 'appraise' not in calls:
                return
            final_packet, payload = calls['appraise']
            appraisal(final_packet, payload, proposal, checked)
            selected_ids = payload['because']
            state = payload['state']
        else:
            final_packet, payload = calls['direct']
            criterion = payload['criterion']
            fields(criterion, ('state', 'because', 'reason', 'questions'), 'direct criterion review')
            require(criterion['state'] in ('met', 'not_met', 'unknown'), 'invalid direct criterion state')
            indices = criterion['because']
            require(isinstance(indices, list) and all(type(i) is int and 0 <= i < len(proposal['units']) for i in indices)
                    and len(indices) == len(set(indices)), 'invalid direct selected observations')
            selected_ids = [proposal['units'][i]['id'] for i in indices]
            state = criterion['state']
        by_id = {unit['id']: unit for unit in proposal['units']}
        context = dict(payload=payload, selected_observations=[by_id[i] for i in selected_ids],
            criterion=final_packet['criterion'], sources=final_packet['sources'],
            evidence_metadata=final_packet['evidence_metadata'])
        row['final_state'] = state
        row['final_state_source'] = 'validated_payload'
        row['criterion_review'] = dict(sha256=digest(context), state=state,
            selected_ids=selected_ids, **context)
    except ERRORS as error:
        row['errors'].append('semantic attribution: ' + str(error))


def refresh_usage(row):
    stages = row['usage_stages']
    row['usage_complete'] = all(stages.get(name, {}).get('complete', False) for name in STAGES[row['arm']])
    for key in ('input_tokens', 'output_tokens'):
        row['observed_' + key] = sum(stage['observed_' + key] for stage in stages.values())
        row[key] = row['observed_' + key] if row['usage_complete'] else None
    row['observed_policy_calls'] = sum(stage['started_calls'] for stage in stages.values())


def account_call_usage(row, name, stage, events, reply, audit_available):
    """Exact cost depends on recorded calls, never JSON/schema/stop validity."""
    starts = [e for e in events if isinstance(e, dict) and e.get('event') == 'request_started']
    responses = [e for e in events if isinstance(e, dict) and
                 (e.get('event') == 'response' or isinstance(e.get('rejected_response'), dict))]
    if reply is not None and not any(e == reply for e in responses):
        responses.append(reply)
    no_call = not starts and not responses and not stage.get('request') and not stage.get('reply') and (
        audit_available and not events and stage.get('status') in ('failed', 'completed', 'withheld') or
        stage.get('status') == 'withheld' and not stage.get('provider_audit'))
    ids = [e.get('call') for e in starts]
    ids_valid = all(type(i) is int and i > 0 for i in ids) and len(set(ids)) == len(ids)
    complete = bool(no_call or audit_available and starts and ids_valid)
    observed = dict(input_tokens=0, output_tokens=0)
    seen = set()
    for response in responses:
        identifier = response.get('call')
        # Ambiguous duplicate responses never become an exact cost, and are not
        # silently double-counted as two actual calls.
        if type(identifier) is not int or identifier <= 0 or identifier in seen:
            complete = False
            continue
        seen.add(identifier)
        if response.get('event') == 'error':
            usage = response['rejected_response'].get('native_usage', {})
            parts = [usage.get(k) for k in ('input_tokens', 'cache_creation_input_tokens', 'cache_read_input_tokens')]
            input_tokens = sum(parts) if all(type(v) is int and v >= 0 for v in parts) else None
            output_tokens = usage.get('output_tokens')
            # Recorder-native zeros cannot distinguish absent usage from zero.
            input_tokens = input_tokens if input_tokens and input_tokens > 0 else None
            output_tokens = output_tokens if type(output_tokens) is int and output_tokens > 0 else None
        else:
            input_tokens, output_tokens = response.get('input_tokens'), response.get('output_tokens')
        for key, value in (('input_tokens', input_tokens), ('output_tokens', output_tokens)):
            if type(value) is int and value >= 0:
                observed[key] += value
            else:
                complete = False
    complete = bool(complete and (no_call or seen == set(ids)))
    row['usage_stages'][name] = dict(complete=complete, started_calls=len(set(ids) | seen) if ids_valid else max(len(starts), len(seen)),
        observed_input_tokens=observed['input_tokens'], observed_output_tokens=observed['output_tokens'],
        proven_no_call=no_call)
    refresh_usage(row)



def apply_frozen_label(row, case):
    """Gold stays outside native input and never rewrites external semantic review."""
    expected = case.get('expected_state')
    require(expected is None or expected in ('met', 'not_met', 'unknown'), 'invalid frozen expected state')
    row['frozen_expected_state'] = expected
    row['frozen_label_correct'] = row['final_state'] == expected if expected is not None and row['final_state'] is not None else None



def observe_final_state(row, name, reply):
    """An observed label survives refusal/truncation if its JSON can be parsed."""
    if name not in ('appraise', 'direct'):
        return
    try:
        payload = response_payload(reply)
        state = payload['state'] if name == 'appraise' else payload['criterion']['state']
        if state in ('met', 'not_met', 'unknown'):
            row['observed_final_states'].append(state)
            unique = set(row['observed_final_states'])
            row['final_state'] = state if len(unique) == 1 else None
            row['final_state_source'] = 'raw_reply' if len(unique) == 1 else 'ambiguous_raw_replies'
    except ERRORS:
        pass


if __name__ == '__main__':
    main()
