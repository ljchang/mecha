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
import re
from pathlib import Path
import stat
import subprocess
import time

PROTOCOL = 'memory-factual-support-v1'
GROUNDED_PROTOCOL = 'memory-factual-support-v2'
PROTOCOLS = (PROTOCOL, GROUNDED_PROTOCOL)
TRANSPORT_AUDIT_BOUNDARY = ('The actual JSON body is audited. The autoload=false URL policy is checked by frozen provider source and authored HTTP tests; the request URL is not recorded in these body artifacts.')
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


def compact_bytes(value):
    # Rust serde_json emits compact UTF-8; object ordering does not alter length.
    return json.dumps(value, separators=(',', ':'), ensure_ascii=False).encode()


def validate_catalog(packet):
    """Reconstruct every catalog row from source bytes without consulting gold.

    The opaque prepared-context hash remains host-owned. Audit its equality across
    packet/catalog/reply/handle, while Rust tests establish the hidden-context binding.
    """
    require(isinstance(packet, dict) and packet.get('protocol') == 'situated-support-wire-v2', 'invalid wire packet')
    key = packet.get('request_key')
    require(isinstance(key, str) and re.fullmatch('[0-9a-f]{64}', key), 'invalid wire request key')
    fields(packet.get('catalog'), ('request_key', 'citations'), 'citation catalog')
    require(packet['catalog']['request_key'] == key and isinstance(packet.get('sources'), list), 'catalog binding mismatch')
    expected, ids = [], set()
    for source in packet['sources']:
        fields(source, ('id', 'source', 'field', 'kind', 'text'), 'wire source')
        require(isinstance(source['id'], str) and source['id'] not in ids and isinstance(source['text'], str), 'invalid/duplicate wire source')
        fields(source['source'], ('object', 'revision'), 'source revision locator')
        require(all(isinstance(value, str) and value for value in source['source'].values())
                and isinstance(source['field'], str) and source['field']
                and source['kind'] in ('Observation', 'AssistantStatement', 'ActionInvocation', 'ArtifactObservation', 'PriorInterpretation', 'InterpretationInfluenced'), 'invalid source revision/role')
        ids.add(source['id'])
        text = source['text']
        if len(text) < 12:
            continue
        count = (len(text) + 299) // 300
        width, extra = divmod(len(text), count)
        start = 0
        for index in range(count):
            end = start + width + (index < extra)
            expected.append(dict(handle=source['kind'] + ':' + key + ':c' + str(len(expected)), source_id=source['id'],
                                 source=source['source'], field=source['field'], kind=source['kind'], quote=text[start:end]))
            start = end
    require(packet['catalog']['citations'] == expected, 'catalog source identity/quote/coverage mismatch')
    return {row['handle']: {'source_id': row['source_id'], 'quote': row['quote']} for row in expected}


def resolve_evidence(catalog, handles):
    require(isinstance(handles, list) and len(handles) <= 2 and all(isinstance(h, str) and h in catalog for h in handles), 'foreign or overbound evidence handles')
    citations = [catalog[h] for h in handles]
    require(len({(c['source_id'], c['quote']) for c in citations}) == len(citations), 'duplicate canonical citation')
    return copy.deepcopy(citations)


def validate_canonical_proposal(packet, proposal):
    fields(proposal, ('units', 'judgments'), 'canonical proposal')
    require(isinstance(proposal['units'], list) and len(proposal['units']) <= 8
            and isinstance(proposal['judgments'], list) and len(proposal['judgments']) <= 8
            and len(compact_bytes(proposal)) <= 16384, 'canonical proposal over bounds')
    sources = {s['id']: s for s in packet['sources']}
    ids = set()
    for unit in proposal['units']:
        fields(unit, ('id', 'kind', 'text', 'citations'), 'canonical unit')
        require(isinstance(unit['id'], str) and re.fullmatch('[A-Za-z0-9_-]{1,32}', unit['id'])
                and unit['id'] not in ids, 'invalid/duplicate unit id')
        ids.add(unit['id'])
        require(unit['kind'] in ('RecordedEvent', 'GoalAssessment', 'Hypothesis', 'Prediction')
                and isinstance(unit['text'], str) and unit['text'].strip()
                and len(unit['text'].encode()) <= 512, 'invalid unit kind/text')
        citations = unit['citations']
        require(isinstance(citations, list) and len(citations) <= 2, 'invalid canonical citations')
        selected = set()
        for citation in citations:
            fields(citation, ('source_id', 'quote'), 'canonical citation')
            require(isinstance(citation['source_id'], str) and citation['source_id'] in sources
                    and isinstance(citation['quote'], str) and 12 <= len(citation['quote']) <= 300
                    and citation['quote'] in sources[citation['source_id']]['text'], 'invalid canonical source quotation')
            selected.add((citation['source_id'], citation['quote']))
        require(len(selected) == len(citations) and (bool(citations) or unit['kind'] in ('Hypothesis', 'Prediction')), 'missing/duplicate canonical citation')
    for judgment in proposal['judgments']:
        fields(judgment, ('goal', 'bearing', 'because'), 'canonical judgment')
        require(judgment['goal'] == packet['criterion']['goal'] and judgment['bearing'] in ('Good', 'Bad', 'Unknown')
                and isinstance(judgment['because'], list) and len(judgment['because']) <= 8
                and all(isinstance(i, str) and i in ids for i in judgment['because'])
                and len(set(judgment['because'])) == len(judgment['because']), 'invalid canonical judgment')
    return proposal


def expand_generation(packet, payload):
    catalog = validate_catalog(packet)
    fields(payload, ('request_key', 'units', 'judgments'), 'wire proposal')
    require(payload['request_key'] == packet['request_key'] and isinstance(payload['units'], list)
            and isinstance(payload['judgments'], list), 'wire proposal binding/shape mismatch')
    units, judgments = [], []
    for unit in payload['units']:
        fields(unit, ('id', 'kind', 'text', 'evidence'), 'wire unit')
        units.append(dict(id=unit['id'], kind=unit['kind'], text=unit['text'], citations=resolve_evidence(catalog, unit['evidence'])))
    for judgment in payload['judgments']:
        fields(judgment, ('bearing', 'because'), 'wire judgment')
        judgments.append(dict(goal=packet['criterion']['goal'], bearing=judgment['bearing'], because=judgment['because']))
    return validate_canonical_proposal(packet, dict(units=units, judgments=judgments))


def expand_evaluation(packet, payload):
    catalog = validate_catalog(packet)
    offered = validate_canonical_proposal(packet, packet['proposal'])
    fields(payload, ('request_key', 'criterion', 'units', 'judgments'), 'wire evaluation')
    require(payload['request_key'] == packet['request_key'], 'wire evaluator binding mismatch')
    fields(payload['criterion'], ('evidence', 'reason', 'questions', 'state'), 'wire criterion')
    state = payload['criterion']['state']
    require(state in ('Met', 'NotMet', 'Unknown'), 'invalid criterion state')
    def diagnostic(item, status, target, unsupported=False):
        require(isinstance(item['reason'], str) and item['reason'].strip() and len(item['reason'].encode()) <= 512
                and isinstance(item['questions'], list) and len(item['questions']) <= 4
                and all(isinstance(q, str) and q.strip() and len(q.encode()) <= 256 for q in item['questions']), 'invalid evaluation diagnostic')
        citations = resolve_evidence(catalog, item['evidence'])
        require(status in ('Supported', 'Contradicted', 'Unknown'), 'unknown support status')
        require(status != 'Supported' or bool(citations) and not item['questions'], 'invalid Supported diagnostic')
        require(status != 'Unknown' or bool(item['questions']), 'Unknown diagnostic lacks questions')
        return dict(target=target, status=status, unsupported_factual_premise=unsupported,
                    citations=citations, reason=item['reason'], questions=item['questions'])
    diagnostic(payload['criterion'], 'Unknown' if state == 'Unknown' else 'Supported', 'criterion')
    expected = ['unit:' + u['id'] for u in offered['units']] + ['judgment:' + str(i) for i in range(len(offered['judgments']))]
    require(packet['targets'] == expected, 'packet target coverage mismatch')
    checks = []
    for name, keys, targets in (
            ('units', ('target', 'evidence', 'reason', 'questions', 'factual_premises', 'status'), expected[:len(offered['units'])]),
            ('judgments', ('target', 'evidence', 'reason', 'questions', 'premises_sufficient'), expected[len(offered['units']):])):
        require(isinstance(payload[name], list) and len(payload[name]) == len(targets), 'evaluation target count mismatch')
        rows = {}
        for item in payload[name]:
            fields(item, keys, 'wire ' + name)
            require(isinstance(item['target'], str) and item['target'] in targets and item['target'] not in rows, 'foreign/duplicate evaluation target')
            rows[item['target']] = item
        for target, original in zip(targets, offered[name]):
            item = rows[target]
            if name == 'units':
                premise = item['factual_premises']
                require(premise in ('None', 'Supported', 'Contradicted', 'Unknown')
                        and (premise != 'None' or original['kind'] in ('Hypothesis', 'Prediction')), 'invalid factual premise state')
                unsupported = premise in ('Contradicted', 'Unknown')
                require(item['status'] != 'Supported' or not unsupported, 'Supported unit has unsupported factual premises')
                checks.append(diagnostic(item, item['status'], target, unsupported))
            else:
                enough, bearing = item['premises_sufficient'], original['bearing']
                require(type(enough) is bool and (bearing == 'Unknown' or original['because'] or not enough), 'invalid judgment premise sufficiency')
                if bearing == 'Unknown':
                    status = 'Supported' if state == 'Unknown' else 'Contradicted'
                elif state == 'Unknown':
                    status = 'Unknown'
                elif (state, bearing) in (('Met', 'Good'), ('NotMet', 'Bad')):
                    status = 'Supported' if enough else 'Unknown'
                else:
                    status = 'Contradicted'
                checks.append(diagnostic(item, status, target))
    result = {'checks': checks}
    require(len(compact_bytes(result)) <= 16384, 'expanded evaluation reply over bounds')
    return result


def wire_packet(request, stage):
    """Audit the actual constrained request, not just a provider capability flag."""
    effective, wire = request['request'], request['wire_body']
    schema = effective.get('response_schema')
    require(isinstance(schema, dict) and wire.get('response_format') == {'type': 'json_object', 'schema': schema}, 'missing/mismatched actual wire schema')
    messages = effective.get('messages')
    require(isinstance(messages, list) and len(messages) == 1 and isinstance(messages[0], dict) and isinstance(messages[0].get('content'), list), 'invalid support request messages')
    blocks = messages[0]['content']
    require(len(blocks) == 1 and isinstance(blocks[0], dict) and blocks[0].get('type') == 'text' and isinstance(blocks[0].get('text'), str), 'invalid support input block')
    text = blocks[0]['text']
    require(text.startswith('<support-input>\n') and text.endswith('\n</support-input>'), 'invalid support input wrapper')
    packet = decode(text[len('<support-input>\n'):-len('\n</support-input>')])
    catalog = validate_catalog(packet)
    require(isinstance(effective.get('system'), str), 'missing support frame')
    require(messages[0].get('role') == 'user' and effective.get('tools') == [] and not wire.get('tools')
            and wire.get('messages') == [{'role': 'system', 'content': effective['system']}, {'role': 'user', 'content': text}]
            and isinstance(wire.get('chat_template_kwargs'), dict) and wire['chat_template_kwargs'].get('enable_thinking') is False, 'actual support messages/tools/thinking mismatch')
    size = len(effective['system'].encode()) + len(text.encode()) + len(compact_bytes(schema))
    require(size == stage['packet_bytes'] and size <= (49152 if 'proposal' in packet else 32768), 'complete request schema/frame/input budget mismatch')
    require(schema.get('type') == 'object' and schema.get('additionalProperties') is False
            and isinstance(schema.get('properties'), dict) and schema['properties'].get('request_key') == {'type': 'string', 'enum': [packet['request_key']]}, 'unbound response schema')
    expected_keys = {'request_key', 'criterion', 'units', 'judgments'} if 'proposal' in packet else {'request_key', 'units', 'judgments'}
    require(set(schema.get('required', [])) == expected_keys == set(schema['properties']), 'incomplete wire response schema')
    def object_properties(value, keys):
        require(isinstance(value, dict) and value.get('type') == 'object' and value.get('additionalProperties') is False
                and isinstance(value.get('required'), list) and len(value['required']) == len(keys)
                and isinstance(value.get('properties'), dict) and set(value['required']) == set(keys) == set(value['properties']), 'incomplete strict object schema')
        return value['properties']
    require(all(isinstance(schema['properties'][name], dict) for name in ('units', 'judgments')), 'invalid target array schema')
    unit_keys = ('target', 'evidence', 'reason', 'questions', 'factual_premises', 'status') if 'proposal' in packet else ('id', 'kind', 'text', 'evidence')
    judgment_keys = ('target', 'evidence', 'reason', 'questions', 'premises_sufficient') if 'proposal' in packet else ('bearing', 'because')
    unit_variants = []
    if 'proposal' in packet:
        offered_units = packet['proposal']['units']
        groups = [(['unit:' + u['id'] for u in offered_units if u['kind'] in kinds], premises)
                  for kinds, premises in ((('RecordedEvent', 'GoalAssessment'), ['Supported', 'Contradicted', 'Unknown']),
                                          (('Hypothesis', 'Prediction'), ['None', 'Supported', 'Contradicted', 'Unknown']))]
        groups = [(targets, premises) for targets, premises in groups if targets]
        require(sum(len(targets) for targets, _ in groups) == len(offered_units), 'unknown proposal unit kind')
        if offered_units:
            item = schema['properties']['units']['items']
            fields(item, ('oneOf',), 'unit target alternatives')
            expected_branches = [(targets, premises, status) for targets, premises in groups
                                 for status in ('Supported', 'Contradicted', 'Unknown')]
            require(isinstance(item['oneOf'], list) and len(item['oneOf']) == len(expected_branches), 'unit target/status group count mismatch')
            for branch, (targets, premises, status) in zip(item['oneOf'], expected_branches):
                properties = object_properties(branch, unit_keys)
                if status == 'Supported':
                    premises = ['None', 'Supported'] if 'None' in premises else ['Supported']
                require(properties['target'] == {'type': 'string', 'enum': targets}
                        and properties['factual_premises'] == {'type': 'string', 'enum': premises}
                        and properties['status'] == {'type': 'string', 'enum': [status]}
                        and properties['evidence'] == {'type': 'array', 'minItems': int(status == 'Supported'), 'maxItems': 2,
                            'uniqueItems': True, 'items': {'type': 'string', 'enum': list(catalog)}}
                        and properties['questions'] == {'type': 'array', 'minItems': int(status == 'Unknown'),
                            'maxItems': 0 if status == 'Supported' else 4,
                            'items': {'type': 'string', 'minLength': 1, 'maxLength': 64}}, 'unconstrained unit target/status diagnostic')
                unit_variants.append(properties)
    else:
        unit = object_properties(schema['properties']['units']['items'], unit_keys)
        unit_variants.append(unit)
    judgment = object_properties(schema['properties']['judgments']['items'], judgment_keys) if 'proposal' not in packet or packet['proposal']['judgments'] else None
    if 'proposal' in packet:
        criterion_schema = schema['properties']['criterion']
        fields(criterion_schema, ('oneOf',), 'criterion alternatives')
        require(isinstance(criterion_schema['oneOf'], list) and len(criterion_schema['oneOf']) == 2, 'criterion needs known and unknown alternatives')
        criteria = [object_properties(branch, ('evidence', 'reason', 'questions', 'state')) for branch in criterion_schema['oneOf']]
        question_items = {'type': 'string', 'minLength': 1, 'maxLength': 64}
        citation_items = {'type': 'string', 'enum': list(catalog)}
        for diagnostic, states, evidence_min, questions_min, questions_max in (
                (criteria[0], ['Met', 'NotMet'], 1, 0, 0), (criteria[1], ['Unknown'], 0, 1, 4)):
            require(diagnostic['state'] == {'type': 'string', 'enum': states}
                    and diagnostic['evidence'] == {'type': 'array', 'minItems': evidence_min, 'maxItems': 2, 'uniqueItems': True, 'items': citation_items}
                    and diagnostic['questions'] == {'type': 'array', 'minItems': questions_min, 'maxItems': questions_max, 'items': question_items}
                    and diagnostic['reason'] == {'type': 'string', 'minLength': 1, 'maxLength': 128}, 'unconstrained criterion evidence/questions/state')
        require(judgment is None or judgment['premises_sufficient'] == {'type': 'boolean'}, 'unconstrained evaluator conclusions')
        for diagnostic in unit_variants + ([judgment] if judgment is not None else []):
            require(diagnostic['reason'] == {'type': 'string', 'minLength': 1, 'maxLength': 128}, 'unbounded diagnostic reason schema')
        if judgment is not None:
            require(judgment['questions'] == {'type': 'array', 'maxItems': 4, 'items': question_items}, 'unbounded judgment questions schema')
    else:
        identifier = {'type': 'string', 'minLength': 1, 'maxLength': 32, 'pattern': '^[A-Za-z0-9_-]{1,32}$'}
        require(unit['id'] == identifier and unit['kind'] == {'type': 'string', 'enum': ['RecordedEvent', 'GoalAssessment', 'Hypothesis', 'Prediction']}
                and unit['text'] == {'type': 'string', 'minLength': 1, 'maxLength': 128}
                and judgment['bearing'] == {'type': 'string', 'enum': ['Good', 'Bad', 'Unknown']}
                and judgment['because'] == {'type': 'array', 'maxItems': 8, 'uniqueItems': True, 'items': identifier}
                and all(schema['properties'][name].get('maxItems') == 8 for name in ('units', 'judgments')), 'unbounded generation schema')
    require(all(schema['properties'][name].get('type') == 'array' for name in ('units', 'judgments')), 'invalid target array schema')
    # Every evidence position must enumerate the complete host catalog, never a gold-selected subset.
    positions = [unit['evidence']] if 'proposal' not in packet else []
    if 'proposal' in packet:
        if judgment is not None:
            positions.append(judgment['evidence'])
    for evidence in positions:
        expected_evidence = {'type': 'array', 'maxItems': 2, 'uniqueItems': True, 'items': {'type': 'string', 'enum': list(catalog)}}
        if 'proposal' in packet:
            expected_evidence['minItems'] = 1
        require(evidence == expected_evidence, 'incomplete/modified citation-choice schema')
    if 'proposal' in packet:
        for name, targets in (('units', ['unit:' + u['id'] for u in packet['proposal']['units']]),
                              ('judgments', ['judgment:' + str(i) for i in range(len(packet['proposal']['judgments']))])):
            array = schema['properties'][name]
            if targets:
                require(array.get('minItems') == array.get('maxItems') == len(targets), 'unconstrained target coverage')
                if name == 'judgments':
                    require(array['items']['properties']['target'] == {'type': 'string', 'enum': targets}, 'unconstrained judgment coverage')
            else:
                require(array == {'type': 'array', 'minItems': 0, 'maxItems': 0, 'items': {'type': 'null'}}, 'empty target list must constrain an empty-only array')
    return packet


def request_packet(root, stage):
    return wire_packet(decode(artifact_bytes(root, stage['request']).decode()), stage)


def validate_case_packet(root, case, report, name):
    """Bind model-visible task/evidence to this frozen native case.

    This bounded audit is not a replacement for Rust's database/source authority
    checks. It verifies current criteria, source-list consistency, the input and
    independent artifact bytes; canonical revision authorization stays in core.
    """
    packet = request_packet(root, report['stages'][name])
    native = case['native']
    criterion = native['current_criterion']
    expected = dict(goal='task:synthetic-support', goal_text=native['goal_text'], criterion_text=criterion,
                    criterion_revision=hashlib.sha256(criterion.encode()).hexdigest(),
                    confirmation_digest=hashlib.sha256(('synthetic-owner-confirmation:' + criterion).encode()).hexdigest())
    require(packet.get('criterion') == expected, 'wire criterion differs from frozen current criterion')
    generation = report['stages']['generate']
    if name != 'generate' and generation.get('request') is not None:
        require(packet['sources'] == request_packet(root, generation)['sources'], 'source list differs between model stages')
    require(any(s['kind'] == 'Observation' and s['text'] == native['input_text'] for s in packet['sources']), 'frozen input observation missing or relabeled')
    artifacts = [s for s in packet['sources'] if s['kind'] == 'ArtifactObservation']
    require(len(artifacts) == 1 and artifacts[0]['field'] == '/artifact_observation', 'independent artifact observation missing or relabeled')
    observed = decode(artifacts[0]['text'])
    fields(observed, ('content_status', 'artifact_text', 'artifact_digest', 'verdict'), 'artifact observation')
    content = native['observed_output']
    require(observed['content_status'] == ('Missing' if content is None else 'Complete')
            and observed['artifact_text'] == content
            and observed['artifact_digest'] == (None if content is None else hashlib.sha256(content.encode()).hexdigest()), 'model artifact observation differs from frozen bytes')
    return packet


def validate_call(root, stage, model, protocol=PROTOCOL):
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
            and wire.get('seed') == SEED and wire.get('temperature') == 0.6,
            'raw runtime controls mismatch')
    if protocol == GROUNDED_PROTOCOL:
        # OpenAiCompatible::request puts autoload=false on the URL. The recorder
        # exposes only its JSON body; never mislabel URL policy as observed here.
        require('autoload' not in wire, 'unexpected body autoload field')
    else:
        # Preserve the original frozen-v1 audit contract and its historical scores.
        require(wire.get('autoload') is False, 'raw runtime controls mismatch')
    require(reply.get('stop_reason') == 'end_turn' and not reply.get('refusal'), 'raw refusal/incomplete response')
    require(all(reply.get(k) == stage[k] for k in ('input_tokens', 'output_tokens')), 'raw usage mismatch')
    if protocol == GROUNDED_PROTOCOL:
        wire_packet(request, stage)
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


def validate_support(root, report, name, model, offered, protocol=PROTOCOL):
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
    payload = response_payload(validate_call(root, stage, model, protocol))
    if protocol == GROUNDED_PROTOCOL:
        packet = request_packet(root, stage)
        canonical = decode(bounded_read(root / (name + '-proposal.json'), 16384).decode())
        require(packet['proposal'] == canonical, 'evaluator request differs from canonical proposal export')
        if name == 'generated':
            require(canonical == offered, 'evaluator proposal differs from expanded generation')
        else:
            require([{k: u[k] for k in ('id', 'kind', 'text')} for u in canonical['units']]
                    == [{k: u[k] for k in ('id', 'kind', 'text')} for u in offered['units']]
                    and [{k: j[k] for k in ('bearing', 'because')} for j in canonical['judgments']]
                    == offered['judgments'], 'evaluator changed host-authored probe')
            sources = {source['id']: source for source in packet['sources']}
            for actual, original in zip(canonical['units'], offered['units']):
                selector = original['citation']
                require(len(actual['citations']) == 1, 'probe citation selector count mismatch')
                citation = actual['citations'][0]
                source = sources.get(citation['source_id'])
                kind = {'artifact': 'ArtifactObservation', 'input': 'Observation', 'action': 'ActionInvocation'}[selector['passage']]
                require(source is not None and source['kind'] == kind
                        and citation['quote'] == (selector['quote'] if selector['quote'] is not None else source['text'])
                        and (kind != 'ActionInvocation' or 'fs_write' in source['text']), 'probe citation selector mismatch')
        offered = canonical
        payload = expand_evaluation(packet, payload)
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
            if name == 'generated' or protocol == GROUNDED_PROTOCOL:
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
        if protocol == GROUNDED_PROTOCOL:
            factual_support = bool(original['because']) and all(
                units[i]['admitted'] and units[i]['status'] == 'Supported'
                and expected_units[i]['kind'] in ('RecordedEvent', 'GoalAssessment') for i in original['because'])
            expected_reason = ('DuplicateGoal' if index > 0 else 'ExplicitUnknown' if original['bearing'] == 'Unknown'
                               else 'UnsupportedJudgment' if check['status'] != 'Supported' or check['unsupported_factual_premise']
                               else 'UnsupportedPremises' if not factual_support else 'Retained')
            require(entry['goal'] == original['goal'] and reason == expected_reason
                    and entry['bearing'] == (original['bearing'] if reason == 'Retained' else 'Unknown'), 'durable judgment derivation mismatch')
        require(reason in ('Retained', 'ExplicitUnknown', 'UnsupportedJudgment', 'UnsupportedPremises', 'DuplicateGoal'), 'unknown durable judgment reason')
        disposition = 'dropped' if reason == 'DuplicateGoal' else 'retained' if reason in ('Retained', 'ExplicitUnknown') else 'downgraded'
        require(entry['bearing'] in ('Good', 'Bad', 'Unknown') and (disposition == 'retained' or entry['bearing'] == 'Unknown'), 'durable final bearing mismatch')
        judgments.append(dict(index=index, status=entry['status'], offered_bearing=entry['offered_bearing'], bearing=entry['bearing'],
                              disposition=disposition, reason=reason, unsupported_factual_premise=check['unsupported_factual_premise']))
    return accepted, units, judgments


def validate_probe(root, case, report, model, protocol=PROTOCOL):
    validate_sources(root, case, report)
    if protocol == GROUNDED_PROTOCOL:
        validate_case_packet(root, case, report, 'evaluate_probe')
    _, units, judgments = validate_support(root, report, 'probe', model, case['probe'], protocol)
    probe = report.get('probe')
    fields(probe, ('units', 'judgments'), 'probe result')
    require(isinstance(probe['units'], list) and len(probe['units']) == len(units)
            and {u['id'] for u in probe['units']} == set(units), 'probe unit coverage mismatch')
    for unit in probe['units']:
        require(unit == units[unit['id']], 'probe summary differs from durable/raw unit')
    require(probe['judgments'] == judgments, 'probe summary differs from durable/raw judgments')
    if protocol == GROUNDED_PROTOCOL:
        raw = response_payload(validate_call(root, report['stages']['evaluate_probe'], model, protocol))
        return dict(probe, criterion_state=raw['criterion']['state'])
    return probe


def validate_generated(root, case, report, model):
    """Independently preserve generated outcomes, even if later stages fail."""
    validate_sources(root, case, report)
    stage = report['stages']['generate']
    require(stage.get('status') == 'completed' and stage.get('error') is None, 'generator outcome not completed')
    packet = validate_case_packet(root, case, report, 'generate')
    validate_case_packet(root, case, report, 'evaluate_generated')
    proposal = expand_generation(packet, response_payload(validate_call(root, stage, model, GROUNDED_PROTOCOL)))
    require(decode(bounded_read(root / 'generated-proposal.json', 16384).decode()) == proposal,
            'canonical generator export differs from expanded wire')
    accepted, units, judgments = validate_support(root, report, 'generated', model, proposal, GROUNDED_PROTOCOL)
    admitted = decode(artifact_bytes(root, report['artifacts']['admitted_generated']).decode())
    require(admitted == accepted, 'admitted export differs from durable accepted units')
    return {'units': list(units.values()), 'judgments': judgments,
            'admitted_unit_ids': sorted(unit['id'] for unit in accepted)}


def validate_report(root, case, model, protocol=PROTOCOL):
    """Return a failed row on malformed/incomplete native output, never omit it."""
    root = Path(root)
    result = {'valid': False, 'errors': [], 'report': None, 'report_sha256': None, 'probe_outcome': None, 'probe_errors': []}
    if protocol == GROUNDED_PROTOCOL:
        result['transport_audit_boundary'] = TRANSPORT_AUDIT_BOUNDARY
        result.update(generated_outcome=None, generated_errors=[])
    try:
        report_bytes = bounded_read(root / 'report.json', MAX_REPORT_BYTES)
        result['report_sha256'] = hashlib.sha256(report_bytes).hexdigest()
        report = decode(report_bytes.decode())
        result['report'] = report
        require(isinstance(report, dict), 'report is not an object')
        if protocol == GROUNDED_PROTOCOL:
            try:
                require(report.get('protocol') == protocol and report.get('seed') == SEED and report.get('model') == model
                        and report.get('case_id') == case['id'] and report.get('case_sha256') == digest(case), 'generated case identity mismatch')
                result['generated_outcome'] = validate_generated(root, case, report, model)
                result['admitted_unit_ids'] = result['generated_outcome']['admitted_unit_ids']
            except (ValueError, OSError, KeyError, TypeError, UnicodeError) as error:
                result['generated_errors'].append(str(error))
        # Probe evidence survives a later recall/changed-goal failure. Validate it
        # independently before checking the overall successful-attempt envelope.
        try:
            require(report.get('protocol') == protocol and report.get('seed') == SEED and report.get('model') == model
                    and report.get('case_id') == case['id'] and report.get('case_sha256') == digest(case), 'probe case identity mismatch')
            result['probe_outcome'] = validate_probe(root, case, report, model, protocol)
        except (ValueError, OSError, KeyError, TypeError, UnicodeError) as error:
            result['probe_errors'].append(str(error))
        require(report.get('protocol') == protocol and report.get('case_id') == case['id']
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
                validate_call(root, stage, model, protocol)
                if protocol == GROUNDED_PROTOCOL:
                    validate_case_packet(root, case, report, name)
                for key in ('input_tokens', 'output_tokens'):
                    require(type(stage.get(key)) is int and stage[key] >= 0, 'missing model usage')
                require(stage['output_tokens'] <= BUDGETS['output_tokens'], 'model output token bound exceeded')
        for key in ('input_tokens', 'output_tokens'):
            require(type(report.get(key)) is int and report[key] == sum(stages[name][key] for name in MODEL_STAGES), 'usage total mismatch')
        validate_sources(root, case, report)
        if protocol == GROUNDED_PROTOCOL:
            require(result['generated_outcome'] is not None, '; '.join(result['generated_errors']) or 'missing validated generated outcome')
        else:
            generated_reply = validate_call(root, stages['generate'], model, protocol)
            generated_proposal = response_payload(generated_reply)
            accepted, _, _ = validate_support(root, report, 'generated', model, generated_proposal, protocol)
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


def bearing_errors(judgments, expected):
    supported = sum(j['offered_bearing'] in ('Good', 'Bad') and j['offered_bearing'] != expected
                    and j['status'] == 'Supported' for j in judgments)
    retained = sum(j['bearing'] in ('Good', 'Bad') and j['bearing'] != expected
                   and j['disposition'] == 'retained' for j in judgments)
    return supported, retained


def score_case(case, validation, audit, protocol=PROTOCOL):
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
    if protocol == GROUNDED_PROTOCOL:
        generated = validation.get('generated_outcome')
        row.update(false_facts_supported=None, criterion_correct=None,
                   generated_outcome_observed=generated is not None, generated_errors=validation.get('generated_errors', []),
                   wrong_generated_bearings_supported=None, wrong_generated_bearings_retained=None)
        if generated is not None:
            row['wrong_generated_bearings_supported'], row['wrong_generated_bearings_retained'] = bearing_errors(
                generated['judgments'], case['annotations']['expected_bearing'])
    if probe is None:
        return row
    units = {u['id']: u for u in probe['units']}
    annotations = case['annotations']
    true, false, uncertain = [units[annotations[key]] for key in ('true_unit', 'false_unit', 'uncertain_unit')]
    wrong_supported, wrong_retained = bearing_errors(probe['judgments'], annotations['expected_bearing'])
    row.update(true_supported=int(true['admitted'] and true['status'] == 'Supported'),
               false_facts_accepted=int(false['admitted']),
               uncertain_appropriate=int(uncertain['status'] == 'Unknown' and uncertain['withheld']),
               wrong_bearings_accepted=wrong_retained, wrong_bearings_supported=wrong_supported,
               correct_bearing_supported=int(any(j['offered_bearing'] == annotations['expected_bearing'] and j['status'] == 'Supported'
                                                and not j['unsupported_factual_premise'] for j in probe['judgments'])))
    if protocol == GROUNDED_PROTOCOL:
        row['false_facts_supported'] = int(false['status'] == 'Supported')
        if probe.get('criterion_state') in ('Met', 'NotMet', 'Unknown'):
            row['criterion_correct'] = int(probe['criterion_state'] == {'Good': 'Met', 'Bad': 'NotMet', 'Unknown': 'Unknown'}[annotations['expected_bearing']])
    if validation['valid']:
        row['manual_audit_complete'], row['unsupported_generated'] = manual_audit(validation, audit)
    return row


def summarize(rows, protocol=PROTOCOL):
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
    if protocol == GROUNDED_PROTOCOL:
        generated_observed = sum(row.get('generated_outcome_observed', False) for row in rows)
        generated_complete = total == 8 and generated_observed == 8 and all(
            type(row.get(key)) is int for row in rows for key in ('wrong_generated_bearings_supported', 'wrong_generated_bearings_retained'))
        passed = (passed and all(type(row.get('false_facts_supported')) is int for row in rows)
                  and count('false_facts_supported') == 0 and generated_complete
                  and count('wrong_generated_bearings_supported') == count('wrong_generated_bearings_retained') == 0)
    result = {'protocol': protocol, 'attempts': total, 'planned_attempts': 8, 'valid_attempts': valid,
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
    if protocol == GROUNDED_PROTOCOL:
        result['transport_audit_boundary'] = TRANSPORT_AUDIT_BOUNDARY
        result.update(generated_metrics_complete=generated_complete,
                      observed_generated_outcomes=generated_observed,
                      unknown_generated_outcomes=max(8, total) - generated_observed)
        for metric in ('wrong_generated_bearings_supported', 'wrong_generated_bearings_retained'):
            result[metric] = count(metric) if generated_complete else None
            result['observed_' + metric] = count(metric)
        support_complete = metrics_complete and all(type(row.get('false_facts_supported')) is int for row in rows)
        result.update(false_facts_supported=count('false_facts_supported') if support_complete else None,
                      observed_false_facts_supported=count('false_facts_supported'),
                      criterion_correct={'count': count('criterion_correct'), 'denominator': 8},
                      criterion_metrics_complete=metrics_complete and all(type(row.get('criterion_correct')) is int for row in rows))
    return result


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


def freeze(root, binary, endpoint, model, holds_dir, protocol=PROTOCOL, fixture=None):
    require(protocol in PROTOCOLS, 'unknown support protocol')
    require(protocol == GROUNDED_PROTOCOL or fixture is None, 'custom fixtures require explicit grounded-v2 protocol')
    selected_fixture = Path(fixture or FIXTURE).absolute()
    cases = load_cases(selected_fixture)
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
    manifest = {'protocol': protocol, 'synthetic_only': True, 'seed': SEED, 'budgets': BUDGETS,
                'endpoint': endpoint, 'model': model, 'holds_dir': str(Path(holds_dir).absolute()),
                'binary': str(binary), 'binary_sha256': file_hash(binary),
                'helper_sha256': file_hash(__file__), 'fixture_sha256': file_hash(selected_fixture),
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
    if protocol == GROUNDED_PROTOCOL:
        manifest['fixture_path'] = str(selected_fixture.resolve(strict=True))
        manifest['gate']['max_false_facts_supported'] = 0
        manifest['gate'].update(max_wrong_generated_bearings_supported=0, max_wrong_generated_bearings_retained=0,
                                require_all_eight_generated_outcomes=True)
    write_json(root / 'manifest.json', manifest)
    (root / 'manifest.sha256').write_text(file_hash(root / 'manifest.json') + '\n')
    write_json(root / 'attempts.json', attempts)
    return manifest


def validate_frozen(root, manifest, runtime=True):
    root = Path(root)
    require(file_hash(root / 'manifest.json') == (root / 'manifest.sha256').read_text().strip()
            and decode((root / 'manifest.json').read_text()) == manifest, 'modified frozen manifest')
    require(manifest['protocol'] in PROTOCOLS and manifest['seed'] == SEED and manifest['budgets'] == BUDGETS,
            'unknown/modified protocol')
    if runtime:
        require(file_hash(manifest['binary']) == manifest['binary_sha256']
                and file_hash(__file__) == manifest['helper_sha256']
                and file_hash(manifest['fixture_path'] if manifest['protocol'] == GROUNDED_PROTOCOL else FIXTURE) == manifest['fixture_sha256']
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
        validation = validate_report(root / 'runs' / case['id'], case, manifest['model'], manifest['protocol'])
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
        row = score_case(case, validation, audit, manifest['protocol'])
        row['attempt'] = attempt
        rows.append(row)
    summary = summarize(rows, manifest['protocol'])
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
        if manifest['protocol'] == GROUNDED_PROTOCOL:
            argv.insert(1, '--grounded-v2')
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
        validation = validate_report(root / 'runs' / case['id'], case, manifest['model'], manifest['protocol'])
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
    freeze_parser.add_argument('--grounded-v2', action='store_true')
    freeze_parser.add_argument('--fixture', type=Path)
    for command in ('run', 'score'):
        commands.add_parser(command).add_argument('root')
    args = parser.parse_args()
    if args.command == 'freeze':
        result = freeze(args.root, args.binary, args.endpoint, args.model, args.holds_dir,
                        protocol=GROUNDED_PROTOCOL if args.grounded_v2 else PROTOCOL, fixture=args.fixture)
        print(encode({'protocol': result['protocol'], 'frozen_attempts': len(result['attempts'])}), end='')
    else:
        result = run(args.root) if args.command == 'run' else score_root(args.root)
        print(encode({key: value for key, value in result.items() if key != 'cases'}), end='')
        raise SystemExit(0 if result['gate_passed'] else 1)


if __name__ == '__main__':
    main()
