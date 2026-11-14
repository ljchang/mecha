#!/usr/bin/env python3
"""Public development completion comparison; authored acquisition, eight fresh runs.

seed -> freeze -> independently review -> run -> audit. This is an adapter around
Mecha's existing native CLI, not another agent executor. First results are immutable.
Portable artifact scores never require Mecha-specific receipt or tool vocabulary.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import sqlite3
import stat

import native_vector_trial as native
from native_vector_trial import (AUTHORED_MODEL, authored_seed_server, canonical,
    closure, database_read, decode, digest, encode, evidence_qualified, invoke,
    native_argv, read, require, save, seed_support, sha, snapshot, transcript_recall)

FIXTURES = Path(__file__).resolve().parent / 'fixtures' / 'completion-feedback'
ARMS = ('off', 'on')
PROTOCOL = 'native-completion-development-v1'
# Frozen Task1 vocabulary; exact strings only, never model-authored prefixes.
NOTICES = (
    '(From the harness: required operations are incomplete. Perform the required read before finishing. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. The required artifact is missing. Save it in the requested format. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. The required artifact is malformed. Save it in the requested format. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. Perform the required read before finishing. The required artifact is missing. Save it in the requested format. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. Perform the required read before finishing. The required artifact is malformed. Save it in the requested format. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. The required artifact is missing. Save it in the requested format. Do not repeat the required read. Continue within the existing permissions and budget.)',
    '(From the harness: required operations are incomplete. The required artifact is malformed. Save it in the requested format. Do not repeat the required read. Continue within the existing permissions and budget.)',
)


def load_fixture(directory=FIXTURES):
    fixture, protocol = read(directory/'cases.json'), read(directory/'protocol.json')
    require(fixture['synthetic_only'] and protocol['synthetic_only'], 'synthetic-only required')
    require(protocol['protocol'] == PROTOCOL, 'unexpected protocol')
    cases = fixture['cases']
    require(len(cases) == 4 and len({c['id'] for c in cases}) == 4, 'four unique cases required')
    require([c['stratum'] for c in cases] == ['supported', 'supported', 'irrelevant_evidence', 'withdrawn_evidence'], 'strata changed')
    require(fixture['public_shape'] == protocol['public_shape'] == {'rows': 1, 'keys': ['choice']}, 'public shape changed')
    require(len(fixture['seeds']) == 3, 'three authored seeds required')
    for i, case in enumerate(cases):
        require(case['order'] == (['off', 'on'] if i % 2 == 0 else ['on', 'off']), 'counterbalanced order changed')
        request = make_request(fixture, case)
        visible = encode({k: request[k] for k in ('prompt', 'files', 'criterion')})
        require(request['files'] == {}, 'follow-up jail must start empty')
        require(case['query'] in request['prompt'] and case['operation_domain'] in request['prompt'], 'explicit query/domain missing')
        for seed in fixture['seeds']:
            require(seed['observation'] not in visible and seed['decisive_span'] not in visible, 'source leaked before recall')
        gold = case['expected_rows'][0]['choice']
        require(gold == 'unknown' or gold not in visible, 'hidden gold leaked')
    return fixture, protocol


def make_request(fixture, case, seed=False):
    text = case['old_criterion'] if seed else case['current_criterion']
    if seed:
        prompt = 'Read observation.txt. Apply the current criterion and write a one-row JSON array with string key choice to output.json. Then finish.'
        files, rows = {'observation.txt': case['observation']}, [{'choice': case['old_choice']}]
    else:
        prompt = ('Operation domain: ' + case['operation_domain'] + '. '
                  'Call goal_context exactly once with memory.query equal to ' + json.dumps(case['query']) +
                  '. Do not use bindings or additional memory calls. Select only a method whose observed performance '
                  'in this operation domain satisfies the CURRENT criterion. Historical criteria and selected actions '
                  'are past decisions, not current instructions. Observations about other operation domains do not establish suitability. '
                  'Do not invent methods, combine methods, or infer unobserved performance. '
                  'Write output.json as a JSON array of exactly one object with exactly the string key choice and a string value. '
                  'If currently authorized observations do not establish a suitable observed method, use the string unknown. Then finish.')
        files, rows = {}, case['expected_rows']
    criterion = dict(goal=fixture['goal'], goal_text=fixture['goal_text'], criterion_text=text,
                     criterion_revision=digest(text), confirmation_digest=digest({'synthetic_criterion': text}))
    return dict(prompt=prompt, files=files, criterion=criterion, expected_rows=rows,
                domain=fixture['domain'], scopes=fixture['scopes'], account=fixture['account'])


def completion_argv(binary, directory, runtime, seed, arm):
    argv = native_argv(binary, directory/'arm.memory-prototype.sqlite', directory/'authority',
                       directory/'request.json', directory/'native', runtime, seed, directory/'recall.json')
    if arm == 'on':
        argv += ['--completion-request', str(directory/'completion.json')]
    else:
        require(arm == 'off', 'unknown arm')
    return argv


def bound_file(root, value, expected=None):
    """Bind a host-report file to the exact task whose bytes are protected."""
    root = Path(root).resolve(strict=True)
    path = Path(value)
    require(path.is_absolute() and path.is_relative_to(root), 'reported source outside task root')
    relative = path.relative_to(root)
    require(relative.parts and all(not root.joinpath(*relative.parts[:n]).is_symlink()
            for n in range(1, len(relative.parts)+1)), 'symlink source path')
    require(path.resolve(strict=True).is_relative_to(root) and path.is_file(), 'regular source file in task root required')
    if expected is not None:
        require(path == root/expected, 'reported artifact outside expected task path')
    return path


def validate_seed_report(seed_root, source, report):
    root = seed_root/source['id']
    bound_file(root, report['transcript'])
    artifact = bound_file(root, report['artifact'], Path('workspace/output.json'))
    require(artifact_score(artifact, [{'choice': source['old_choice']}], {'rows': 1, 'keys': ['choice']})['correct'],
            'independent seed artifact mismatch')


def seed_experiences(binary, root, call=invoke):
    fixture, _ = load_fixture()
    binary, root = Path(binary).resolve(strict=True), Path(root).absolute()
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = root.resolve()
    database, authority = root / 'seed.memory-prototype.sqlite', root / 'authority'
    result = call([str(binary), 'memory', '--experimental', '--db', str(database), 'init-native',
                   '--synthetic-input', '--domain', fixture['domain'], '--authority', str(authority)], root / 'init')
    require(result['returncode'] == 0, 'native initialization failed; root retained')
    reports = {}
    for source in fixture['seeds']:
        request = root / (source['id'] + '.request.json')
        save(request, make_request(fixture, source, seed=True))
        with authored_seed_server(source['old_choice']) as (endpoint, calls, errors):
            runtime = dict(endpoint=endpoint, model=AUTHORED_MODEL, holds_dir=str(root / 'holds'))
            result = call(native_argv(binary, database, authority, request, root / source['id'], runtime, 7), root / source['id'])
        save(root / (source['id'] + '.authored-http.json'), {'calls': calls, 'errors': errors})
        report = read(root / source['id'] / 'report.json')
        require(result['returncode'] == 0 and len(calls) == 3 and not errors and report.get('receipt')
                and not report.get('run_error') and not report.get('capture_error'), 'authored native seed failed')
        validate_seed_report(root, source, report)
        reports[source['id']] = report
    save(root / 'seed-reports.json', reports)
    return reports


def seed_costs(root, fixture):
    """Shared authored acquisition is recorded once, separately from follow-ups."""
    rows = []
    for source in fixture['seeds']:
        sid = source['id']
        row = dict(source=sid, authored=True, wall_elapsed_ms=None, input_tokens=None, output_tokens=None)
        try:
            dispatch = read(root/(sid+'.result.json'))
            row.update(wall_elapsed_ms=dispatch.get('elapsed_ms'), returncode=dispatch.get('returncode'))
            audit = [decode(line) for line in (root/sid/'provider-audit.jsonl').read_text().splitlines() if line.strip()]
            started = [r['call'] for r in audit if r.get('event') == 'request_started']
            responses = [r for r in audit if r.get('event') == 'response']
            complete = bool(started) and len(set(started)) == len(started) and sorted(started) == sorted(r['call'] for r in responses)
            row['requests_started'] = len(started)
            for name in ('input_tokens','output_tokens'):
                values = [r[name] for r in responses if r.get(name) is not None]
                row['known_partial_'+name] = sum(values) if values else None
                row[name] = sum(values) if complete and len(values) == len(started) else None
        except (OSError, ValueError, KeyError, TypeError) as error:
            row['error'] = str(error)
        rows.append(row)
    return dict(shared=True, scored_model_behavior=False, initialization=dispatch_cost(root/'init.result.json'), sources=rows)


def dispatch_cost(path):
    try:
        result = read(path)
        return dict(returncode=result.get('returncode'), wall_elapsed_ms=result.get('elapsed_ms'), error=result.get('error'))
    except (OSError, ValueError, TypeError) as error:
        return dict(returncode=None, wall_elapsed_ms=None, error=str(error))


def validate_runtime(runtime):
    require(runtime.get('model') and runtime.get('endpoint') and runtime.get('holds_dir'), 'runtime controls required')
    if runtime.get('qualification_only') is True:
        require(runtime.get('observed_identity', {}).get('authored_only') is True, 'explicit authored-only identity required')
        return 'authored-qualification-only'
    source, observed = runtime.get('source_identity', {}), runtime.get('observed_identity', {})
    require(source.get('verified') is True and re.fullmatch(r'[0-9a-f]{40}', source.get('commit', '')) is not None
            and re.fullmatch(r'[0-9a-f]{64}', source.get('dirty_tree_sha256', '')) is not None,
            'verified source commit and dirty-tree hash required')
    require(observed.get('verified') is True and observed.get('model') == runtime['model']
            and observed.get('artifact_revision') and isinstance(observed.get('evidence'), dict) and observed['evidence'],
            'verified observed model/artifact identity and evidence required')
    # These are explicit operator verification receipts, sealed here. This
    # driver neither probes a live endpoint nor upgrades declarations to proof.
    return 'sealed-operator-verification-receipts'


def freeze(binary, withdraw_binary, seed_root, root, runtime, call=invoke):
    fixture, protocol = load_fixture()
    binary, withdraw_binary = Path(binary).resolve(strict=True), Path(withdraw_binary).resolve(strict=True)
    seed_root, root = Path(seed_root).resolve(strict=True), Path(root).absolute()
    identity_basis = validate_runtime(runtime)
    database = seed_root / 'seed.memory-prototype.sqlite'
    require(not Path(str(database)+'-wal').exists() or Path(str(database)+'-wal').stat().st_size == 0, 'close/checkpoint seed DB first')
    reports = read(seed_root / 'seed-reports.json')
    for source in fixture['seeds']:
        validate_seed_report(seed_root, source, reports[source['id']])
    support = {s['id']: seed_support(database, reports[s['id']], s) for s in fixture['seeds']}
    original_seal = snapshot(seed_root)
    authority_seal = snapshot(seed_root/'authority')
    withdrawn_marker = support['S3']['binding']['locator']+'.withdrawn'
    require(withdrawn_marker not in authority_seal, 'S3 already withdrawn in seed baseline')
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    root = root.resolve()
    save(root / 'cases.json', fixture)
    save(root / 'protocol.json', protocol)
    # Exact tracked bytes are sealed independently from parsed copies.
    manifest = dict(protocol_sha256=sha(FIXTURES/'protocol.json'), fixture_sha256=sha(FIXTURES/'cases.json'),
                    driver_sha256=sha(__file__), helper_sha256=sha(native.__file__), binary=str(binary), binary_sha256=sha(binary),
                    withdraw_binary=str(withdraw_binary), withdraw_binary_sha256=sha(withdraw_binary),
                    seed_root=str(seed_root), source_seal=original_seal, authority_seal=authority_seal, runtime=runtime, identity_basis=identity_basis, support=support,
                    seed_acquisition=seed_costs(seed_root, fixture), slots=[])
    for case in fixture['cases']:
        for mode in case['order']:
            opaque = hashlib.sha256((case['id'] + ':' + mode).encode()).hexdigest()[:16]
            slot = root / opaque
            slot.mkdir(mode=0o700)
            db = slot / 'arm.memory-prototype.sqlite'
            shutil.copyfile(database, db)
            require(sha(db) == sha(database) and db.stat().st_ino != database.stat().st_ino, 'independent exact DB copy required')
            shutil.copytree(seed_root/'authority', slot/'authority')
            require(snapshot(slot/'authority') == snapshot(seed_root/'authority'), 'full authority copy required')
            save(slot/'request.json', make_request(fixture, case))
            save(slot/'recall.json', dict(mode='lexical', query=case['query']))
            save(slot/'completion.json', fixture['public_shape'])
            if case['withdraw']:
                result = call([str(withdraw_binary), '--synthetic-only', str(slot/'authority'), reports[case['source']]['confirmation']], slot/'withdraw')
                require(result['returncode'] == 0, 'withdrawal helper failed')
                expected_authority = dict(authority_seal)
                expected_authority[withdrawn_marker] = hashlib.sha256(b'withdrawn').hexdigest()
                require(snapshot(slot/'authority') == expected_authority, 'withdrawal must add only exact S3 marker')
            else:
                require(withdrawn_marker not in snapshot(slot/'authority'), 'authorized baseline unexpectedly withdrawn')
            manifest['slots'].append(dict(case=case['id'], mode=mode, directory=opaque, seed=case['seed'], state='planned',
                                         preparation=dispatch_cost(slot/'withdraw.result.json') if case['withdraw'] else None,
                                         initial_seal=snapshot(slot)))
    require(len(manifest['slots']) == 8 and snapshot(seed_root) == original_seal, 'freeze changed source or slot count')
    save(root/'manifest.json', manifest)
    with (root/'manifest.sha256').open('x') as out:
        out.write(sha(root/'manifest.json')+'\n')
    return manifest


def terminal_snapshot(root):
    root = Path(root)
    result = {}
    for path in sorted(root.rglob('*')):
        name = str(path.relative_to(root))
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode):
            result[name] = dict(kind='symlink', target=os.readlink(path))
        elif stat.S_ISREG(mode):
            result[name] = dict(kind='file', sha256=sha(path))
        elif not stat.S_ISDIR(mode):
            result[name] = dict(kind='nonregular', mode=mode)
    return result


def seal_terminal(root, slot):
    seals = root/'terminal-seals'
    seals.mkdir(exist_ok=True)
    save(seals/(slot['directory']+'.json'), dict(slot=slot['directory'], files=terminal_snapshot(root/slot['directory'])))


def verify_frozen(root):
    root = Path(root)
    manifest = read(root/'manifest.json')
    require(sha(root/'manifest.json') == (root/'manifest.sha256').read_text().strip(), 'manifest changed')
    require(sha(FIXTURES/'protocol.json') == manifest['protocol_sha256'] and sha(FIXTURES/'cases.json') == manifest['fixture_sha256'], 'tracked protocol/fixture changed')
    require(sha(__file__) == manifest['driver_sha256'] and sha(native.__file__) == manifest['helper_sha256'], 'driver/helper changed')
    require(sha(manifest['binary']) == manifest['binary_sha256'] and sha(manifest['withdraw_binary']) == manifest['withdraw_binary_sha256'], 'binary changed')
    require(snapshot(manifest['seed_root']) == manifest['source_seal'], 'shared originals changed')
    fixture, protocol = load_fixture()
    require(read(root/'cases.json') == fixture and read(root/'protocol.json') == protocol, 'frozen fixture copies changed')
    for case in fixture['cases']:
        slots = [s for s in manifest['slots'] if s['case'] == case['id']]
        require(len(slots) == 2, 'paired slots required')
        requests = [read(root/s['directory']/'request.json') for s in slots]
        require(requests[0] == requests[1] == make_request(fixture, case), 'paired public input mismatch')
        for slot in slots:
            directory = root/slot['directory']
            require(read(directory/'recall.json') == {'mode': 'lexical', 'query': case['query']}, 'recall treatment changed')
            require(read(directory/'completion.json') == fixture['public_shape'], 'public shape changed')
    for slot in manifest['slots']:
        seal = root/'terminal-seals'/(slot['directory']+'.json')
        if seal.exists():
            require(terminal_snapshot(root/slot['directory']) == read(seal)['files'], 'terminal slot artifacts changed')
    if (root/'execution-finished.json').exists():
        sealed = read(root/'execution-finished.json')['slot_seals']
        require(all(terminal_snapshot(root/name) == seal for name, seal in sealed.items()), 'first-result artifacts changed')
    return manifest, fixture


def run_trial(root, call=invoke):
    root = Path(root).resolve(strict=True)
    manifest, _ = verify_frozen(root)
    require(all(snapshot(root/s['directory']) == s['initial_seal'] for s in manifest['slots']), 'slot changed before execution')
    save(root/'execution-started.json', {'planned_slots': 8, 'manifest_sha256': sha(root/'manifest.json')})
    for slot in manifest['slots']:
        verify_frozen(root)
        directory = root/slot['directory']
        require(snapshot(directory) == slot['initial_seal'], 'unstarted slot changed')
        try:
            call(completion_argv(manifest['binary'], directory, manifest['runtime'], slot['seed'], slot['mode']), directory/'dispatch')
        finally:
            # Preserve even a partial/failed terminal attempt before any scorer.
            seal_terminal(root, slot)
        verify_frozen(root)
    result = audit(root)
    save(root/'audit-first.json', result)
    save(root/'execution-finished.json', {'slot_seals': {s['directory']: terminal_snapshot(root/s['directory']) for s in manifest['slots']}})
    return result


def artifact_score(path, expected_rows, shape):
    """Portable bounded artifact oracle. No report, SQL or tool names required."""
    result = dict(status='missing', format_complete=False, correct=False, abstained=False, rows=None, error=None)
    try:
        # The host supplies the run root; reject altered native/workspace paths
        # and the leaf, while permitting host aliases such as macOS /tmp.
        require(not any(p.is_symlink() for p in [path, path.parent, path.parent.parent]), 'symlink artifact path')
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as stream:
            info = os.fstat(stream.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_size <= 1024 * 1024, 'bounded regular artifact required')
            data = stream.read(1024 * 1024 + 1)
            require(len(data) <= 1024 * 1024, 'oversize artifact')
        value = decode(data.decode('utf-8'))
        result['format_complete'] = (isinstance(value, list) and len(value) == shape['rows'] and
            all(isinstance(row, dict) and set(row) == set(shape['keys']) and
                all(isinstance(v, str) for v in row.values()) for row in value))
        require(result['format_complete'], 'artifact does not match public shape')
        encode(value).encode('utf-8', errors='strict')
        result['rows'] = value
        result['status'] = 'valid'
        result['correct'] = result['format_complete'] and value == expected_rows
        result['abstained'] = result['format_complete'] and value == [{'choice': 'unknown'}]
    except FileNotFoundError:
        pass
    except (ValueError, OSError, UnicodeError, RecursionError) as error:
        result.update(rows=None, format_complete=False, correct=False, abstained=False, status='unavailable' if isinstance(error, OSError) else 'malformed', error=str(error))
    return result


def withdrawal_state(root, directory, case, manifest):
    marker = manifest['support']['S3']['binding']['locator'] + '.withdrawn'
    expected_hash = hashlib.sha256(b'withdrawn').hexdigest()
    require(marker not in manifest['authority_seal'] and not (Path(manifest['seed_root'])/'authority'/marker).exists(), 'seed S3 baseline withdrawn')
    # Runtime capture appends authority files, so compare only pre-existing files
    # and marker presence here; freeze already checked the entire exact delta.
    actual = snapshot(directory/'authority')
    require(all(actual.get(name) == value for name, value in manifest['authority_seal'].items()), 'baseline authority changed')
    require((actual.get(marker) == expected_hash) if case['withdraw'] else marker not in actual,
            'exact S3 withdrawal state missing or unexpected')
    for slot in manifest['slots']:
        if not next(c for c in read(root/'cases.json')['cases'] if c['id'] == slot['case'])['withdraw']:
            require(not (root/slot['directory']/'authority'/marker).exists(), 'paired authorized S3 control withdrawn')
    return True


def checked_delivery_closure(connection, packets):
    """Validate every presented record and dependency before withdrawal credit."""
    admitted = set()
    for packet in packets:
        require(isinstance(packet.get('records'), list), 'unverifiable memory packet')
        for hit in packet['records']:
            reference = hit['reference']
            require(canonical(connection, reference) == hit['record'], 'delivered record is not canonical')
            dependencies = closure(connection, reference)
            for revision in dependencies:
                require(connection.execute('SELECT 1 FROM revision WHERE id=?', (revision,)).fetchone() is not None,
                        'delivered dependency missing from canonical store')
            admitted.update(dependencies)
    return admitted


def provider_events(directory):
    return [decode(line) for line in (directory/'native'/'provider-audit.jsonl').read_text().splitlines() if line.strip()]


def request_received(events, index):
    """A local prepared body proves no POST; only a later response/stream does."""
    request = events[index]
    call = request['call']
    if sum(e.get('event') == 'request_started' and e.get('call') == call for e in events) != 1:
        return False
    return any(e.get('call') == call and (e.get('event') == 'response' or
        e.get('event') == 'stream' and e.get('kind') in ('text_delta', 'thinking_delta', 'tool_use_start', 'usage'))
        for e in events[index+1:])


def exact_message(body, role, text, tool_id=None):
    for message in body.get('messages', []):
        if message.get('role') != role or tool_id is not None and message.get('tool_call_id') != tool_id:
            continue
        content = message.get('content')
        if content == text or isinstance(content, list) and any(isinstance(block, dict) and block.get('text') == text for block in content):
            return True
    return False


def recall_exposure(directory, report):
    """Transcript delivery and subsequent actor exposure are separate evidence."""
    try:
        path = bound_file(directory/'native', report['transcript'])
        records = [decode(line) for line in path.read_text().splitlines() if line.strip()]
        blocks = [b for r in records if r.get('record') == 'message' for b in r.get('content', [])]
        ids = {b['id'] for b in blocks if b.get('type') == 'tool_use' and b.get('name') == 'goal_context'}
        results = [b for b in blocks if b.get('type') == 'tool_result' and b.get('tool_use_id') in ids and not b.get('is_error')]
        events = provider_events(directory)
        if results and all(any(e.get('event') == 'request_started' and request_received(events, i)
                and exact_message(e['wire_body'], 'tool', result['content'], result['tool_use_id'])
                for i, e in enumerate(events)) for result in results):
            return True
    except (KeyError, ValueError, TypeError, OSError, UnicodeError, RecursionError):
        pass
    return None


def audit_slot(root, slot, case, manifest):
    directory = root/slot['directory']
    result = dict(case=case['id'], source_family=case['source_family'], stratum=case['stratum'], mode=slot['mode'], state='not_started', raw_correct=False,
                  protocol_ok=False, lineage_ok=False, decisive_evidence=False, qualified_positive=False,
                  qualified_no_evidence=False, observed_rows=None, abstained=False, artifact_status='missing',
                  returncode=None, wall_elapsed_ms=None, withdrawn_not_delivered=None, errors=[])
    result['portable'] = artifact_score(directory/'native'/'workspace'/'output.json', case['expected_rows'], read(root/'cases.json')['public_shape'])
    if not (directory/'dispatch.started.json').exists():
        if result['portable']['status'] != 'missing':
            result['unstarted_artifact_anomaly'] = result['portable']
        result['portable'] = dict(status='not_started', format_complete=False, correct=False, abstained=False, rows=None, error=None)
        return result
    result['state'] = 'failed'
    catches = (KeyError, ValueError, TypeError, OSError, sqlite3.Error, UnicodeError, RecursionError)
    dispatch, report, packets, attempts_ok = {}, {}, [], False
    report_paths_valid = False
    try:
        dispatch = read(directory/'dispatch.result.json')
        result.update(returncode=dispatch['returncode'], wall_elapsed_ms=dispatch['elapsed_ms'], dispatch_error=dispatch.get('error'))
    except catches as error:
        result['errors'].append('dispatch: '+str(error))
    # The expected jailed artifact remains independently measurable even when
    # report.json is minimal, malformed or absent, or provider discovery failed.
    artifact = directory/'native'/'workspace'/'output.json'
    portable = artifact_score(artifact, case['expected_rows'], read(root/'cases.json')['public_shape'])
    result.update(portable=portable, observed_rows=portable['rows'], artifact_status=portable['status'],
                  raw_correct=portable['correct'], abstained=portable['abstained'])
    try:
        report = read(directory/'native'/'report.json')
        result.update(native_run_error=report.get('run_error'), native_capture_error=report.get('capture_error'),
                      recall=report.get('recall'), native_check=report.get('check'),
                      native_status=report.get('status'), native_error=report.get('error'), completion=report.get('completion'))
        if report.get('artifact'):
            require(Path(report['artifact']) == artifact, 'reported artifact outside expected jail')
        if report.get('transcript'):
            bound_file(directory/'native', report['transcript'])
        report_paths_valid = True
        result['state'] = 'failed' if report.get('status') == 'failed' or report.get('run_error') or report.get('capture_error') or dispatch.get('returncode') != 0 or dispatch.get('error') else 'recorded'
    except catches as error:
        result['errors'].append('native report: '+str(error))
    try:
        transcript = bound_file(directory/'native', report['transcript'])
        attempts_ok, packets, result['transcript_attempts'] = transcript_recall(transcript, case['query'], read(root/'cases.json')['goal'])
    except catches as error:
        result['errors'].append('transcript: '+str(error))
    try:
        recall = report['recall']
        result['protocol_ok'] = (attempts_ok and recall['entered_calls'] == 1 and recall['admitted_calls'] == 1
                                 and recall['rejected_calls'] == 0 and recall['status'] == 'completed'
                                 and recall['identity']['protocol'] == 'native-recall-v1'
                                 and recall['identity']['query_digest'] == hashlib.sha256(case['query'].encode()).hexdigest()
                                 and recall['identity']['mode'] == 'lexical' and recall['effective_mode'] == 'lexical'
                                 and recall.get('fallback') is None)
    except catches as error:
        result['errors'].append('recall: '+str(error))
    try:
        result['withdrawal_state_ok'] = withdrawal_state(root, directory, case, manifest)
    except catches as error:
        result['withdrawal_state_ok'] = False
        result['errors'].append('withdrawal: '+str(error))
    try:
        receipt = report.get('receipt')
        with database_read(directory/'arm.memory-prototype.sqlite') as conn:
            # Delivery coverage is independent of task correctness and recapture.
            if case['stratum'] in ('supported', 'irrelevant_evidence'):
                support = manifest['support'][case['source']]
                canonical_record = canonical(conn, support['reference'])
                result['source_original_delivered'] = any(h.get('reference') == support['reference']
                    and h.get('record') == canonical_record and canonical_record['data']['text'] == support['original_text']
                    and support['span'] in canonical_record['data']['text'] for p in packets for h in p.get('records', []))
                result['decisive_evidence'] = case['stratum'] == 'supported' and result['source_original_delivered']
            delivered = [h['reference'] for p in packets for h in p.get('records', [])]
            if case['withdraw']:
                forbidden = set(manifest['support'][case['source']]['capture_closure'])
                if packets:
                    delivered_closure = checked_delivery_closure(conn, packets)
                    result['withdrawn_not_delivered'] = not forbidden.intersection(delivered_closure)
            if receipt and not report.get('capture_error') and not report.get('run_error'):
                row = conn.execute('SELECT projection_object, projection_revision, originals FROM host_capture_receipt WHERE receipt_id=? AND state=?', (receipt['id'], 'active')).fetchone()
                require(row is not None and {'object': row[0], 'revision': row[1]} == receipt['projection']
                        and decode(row[2]) == receipt['originals'], 'native receipt does not match store')
                admitted = closure(conn, receipt['projection'])
                require(not any(canonical(conn, r)['data']['text'] in NOTICES for r in receipt['originals']), 'harness feedback admitted as independent original')
                require(all(closure(conn, r).issubset(admitted) for r in delivered), 'complete delivered closure absent from capture')
                if case['stratum'] in ('supported', 'irrelevant_evidence'):
                    require(not result.get('source_original_delivered', False) or evidence_qualified(conn, packets, receipt, manifest['support'][case['source']]), 'decisive closure absent')
                elif case['withdraw']:
                    result['withdrawn_not_admitted'] = not forbidden.intersection(admitted)
                    require(result['withdrawn_not_delivered'] and result['withdrawn_not_admitted'], 'withdrawn source delivered or admitted')
                result['lineage_ok'] = True
    except catches as error:
        result['errors'].append('lineage: '+str(error))
    result['evidence_delivery_basis'] = 'canonical_tool_result_in_transcript'
    result['recall_result_seen_by_provider'] = recall_exposure(directory, report)
    common = report_paths_valid and dispatch.get('returncode') == 0 and not dispatch.get('error') and not report.get('run_error') and not report.get('capture_error') and result['raw_correct'] and result['protocol_ok'] and result['lineage_ok'] and result.get('withdrawal_state_ok', False)
    result['qualified_positive'] = common and result['decisive_evidence']
    result['qualified_no_evidence'] = common and (case['stratum'] == 'withdrawn_evidence' or
        case['stratum'] == 'irrelevant_evidence' and result.get('source_original_delivered', False))
    return result


def provider_contract(directory, case, runtime):
    result = dict(first=None, costs=None, valid=False, errors=[])
    try:
        rows = [decode(line) for line in (directory/'native'/'provider-audit.jsonl').read_text().splitlines() if line.strip()]
        requests = [r for r in rows if r.get('event') == 'request_started']
        responses = [r for r in rows if r.get('event') == 'response']
        started = [r['call'] for r in requests]
        finished = [r['call'] for r in responses]
        costs = dict(requests_started=len(started), responses_completed=len(finished),
                     unaccounted_call_ids=[call for call in started if finished.count(call) != 1])
        complete = bool(started) and len(set(started)) == len(started) and sorted(started) == sorted(finished)
        for name in ('input_tokens','output_tokens'):
            known = [r[name] for r in responses if r.get(name) is not None and r['call'] in started]
            costs['known_partial_'+name] = sum(known) if known else None
            costs[name] = sum(known) if complete and len(known) == len(started) else None
        result['costs'] = costs
        require(requests, 'no recorded provider request')
        result['first'] = {'request': requests[0]['request'], 'wire_body': requests[0]['wire_body']}
        for row in requests:
            request, wire = row['request'], row['wire_body']
            require(request['think'] is False and request['think_budget'] is None and request['local_only'], 'thinking/local controls changed')
            require(0 < request['max_tokens'] <= 4096 and wire['model'] == runtime['model'], 'provider controls changed')
            require(wire.get('chat_template_kwargs', {}).get('enable_thinking') is False, 'effective thinking enabled')
            require(wire.get('seed') == case['seed'] and wire.get('temperature') == 0.6, 'matched sampling controls changed')
        visible = encode(result['first'])
        gold = case['expected_rows'][0]['choice']
        require(gold == 'unknown' or gold not in visible, 'gold leaked in initial provider request')
        fixture, _ = load_fixture()
        for seed in fixture['seeds']:
            require(seed['observation'] not in visible and seed['decisive_span'] not in visible, 'source leaked before recall')
        result['valid'] = True
    except (KeyError, ValueError, TypeError, OSError) as error:
        result['errors'].append(str(error))
    return result


def completion_diagnostics(directory, arm, report):
    """Separate appended, locally prepared, and response-attested feedback."""
    result = dict(report=report, report_valid=False, feedback_appended=False,
                  feedback_prepared_requests=[], feedback_seen_by_provider=None,
                  first_feedback_prepared_request=None, first_feedback_request=None,
                  previous_response_call=None, errors=[])
    try:
        if arm == 'off':
            require(report is None, 'control unexpectedly has completion report')
            result['report_valid'] = True
            return result
        require(report['protocol'] == 'native-completion-feedback-v1', 'completion protocol changed')
        expected_digest = hashlib.sha256(json.dumps(['native-completion-feedback-v1', 1, ['choice']], separators=(',', ':')).encode()).hexdigest()
        require(report['shape_digest'] == expected_digest, 'shape identity mismatch')
        loop = report['report']
        require(set(loop) == {'first', 'latest', 'feedback_delivered', 'timed_out', 'interrupted'}
                and isinstance(loop['feedback_delivered'], bool), 'completion report shape changed')
        result['feedback_appended'] = loop['feedback_delivered']
        transcript = bound_file(directory/'native', read(directory/'native'/'report.json')['transcript'])
        records = [decode(line) for line in transcript.read_text().splitlines() if line.strip()]
        notices = [block['text'] for record in records if record.get('record') == 'message'
                   and record.get('role') == 'user' and record.get('harness') is True for block in record.get('content', [])
                   if block.get('type') == 'text' and block.get('text') in NOTICES]
        require(len(notices) == int(loop['feedback_delivered']), 'marked notice and appended flag disagree')
        events = provider_events(directory)
        requests = [(i, e) for i, e in enumerate(events) if e.get('event') == 'request_started']
        bearing = [(i, e) for i, e in requests if any(exact_message(e['wire_body'], 'user', n) for n in NOTICES)]
        if bearing:
            require(notices and all(exact_message(e['wire_body'], 'user', notices[0]) for _, e in bearing),
                    'provider notice does not match marked transcript notice')
            first_index, first = bearing[0]
            previous = [(i, e) for i, e in requests if i < first_index]
            require(previous, 'notice appeared in initial request')
            previous_index, previous_request = previous[-1]
            require(any(e.get('event') == 'response' and e.get('call') == previous_request['call']
                        for e in events[previous_index+1:first_index]), 'notice prepared before preceding response')
            result['previous_response_call'] = previous_request['call']
            result['feedback_prepared_requests'] = [e['call'] for _, e in bearing]
            result['first_feedback_prepared_request'] = first['call']
            received = [e['call'] for i, e in bearing if request_received(events, i)]
            if received:
                result.update(feedback_seen_by_provider=True, first_feedback_request=received[0])
        elif not notices:
            result['feedback_seen_by_provider'] = False
        result['report_valid'] = True
    except (KeyError, ValueError, TypeError, OSError, UnicodeError, RecursionError) as error:
        result['errors'].append(str(error))
    return result


def audit(root):
    root = Path(root).resolve(strict=True)
    manifest, fixture = verify_frozen(root)
    cases = {c['id']: c for c in fixture['cases']}
    slots = [audit_slot(root, s, cases[s['case']], manifest) for s in manifest['slots']]
    pairs = []
    for case in fixture['cases']:
        pair = [s for s in manifest['slots'] if s['case'] == case['id']]
        rows = [r for r in slots if r['case'] == case['id']]
        contracts = [provider_contract(root/s['directory'], case, manifest['runtime']) for s in pair]
        matched = all(c['valid'] for c in contracts) and contracts[0]['first'] == contracts[1]['first']
        by_arm = {r['mode']: r for r in rows}
        pairs.append(dict(case=case['id'], source_family=case['source_family'], matched_provider_contract=matched,
                          portable_correct_difference=(int(by_arm['on']['portable']['correct']) - int(by_arm['off']['portable']['correct'])) if matched else None,
                          format_complete_difference=(int(by_arm['on']['portable']['format_complete']) - int(by_arm['off']['portable']['format_complete'])) if matched else None))
        for row, slot, contract in zip(rows, pair, contracts):
            row['model_tokens'] = contract['costs']
            row['provider_contract_ok'] = contract['valid']
            row['matched_provider_contract'] = matched
            row['errors'].extend(contract['errors'])
            row['completion_diagnostics'] = completion_diagnostics(root/slot['directory'], slot['mode'], row.get('completion'))
            row['qualified_positive'] = contract['valid'] and row['qualified_positive']
            row['qualified_no_evidence'] = contract['valid'] and row['qualified_no_evidence']
    totals = {}
    for arm in ARMS:
        rows = [r for r in slots if r['mode'] == arm]
        totals[arm] = dict(planned=4, attempted=sum(r['state'] != 'not_started' for r in rows),
            portable=dict(format_complete=sum(r['portable']['format_complete'] for r in rows),
                          correct=sum(r['portable']['correct'] for r in rows),
                          supported_correct=sum(r['portable']['correct'] for r in rows if r['stratum'] == 'supported'),
                          unknown_controls={r['stratum']: r['portable']['correct'] for r in rows if r['stratum'] != 'supported'}),
            mecha_qualification=dict(supported=sum(r['qualified_positive'] for r in rows),
                                     decisive_delivery=sum(r['decisive_evidence'] for r in rows),
                                     unknown_controls={r['stratum']: r['qualified_no_evidence'] for r in rows if r['stratum'] != 'supported'}))
    return dict(protocol=PROTOCOL, suite_candidate='mecha-memory-development-v0.1/completion',
                protocol_sha256=manifest['protocol_sha256'], fixture_sha256=manifest['fixture_sha256'], feature_profile=read(FIXTURES/'protocol.json')['feature_profile'],
                seed_acquisition=manifest['seed_acquisition'], preparation=[dict(case=s['case'], mode=s['mode'], dispatch=s['preparation']) for s in manifest['slots'] if s['preparation'] is not None],
                matched_comparison=dict(planned_pairs=4, qualified_pairs=sum(p['matched_provider_contract'] for p in pairs),
                    families={family: [p for p in pairs if p['source_family'] == family] for family in sorted({p['source_family'] for p in pairs})}),
                totals=totals, slots=slots, pairs=pairs,
                boundary='Public development cohort; authored prior experiences. Artifact correctness, completion, delivered support and Mecha qualification are distinct. Three source families do not establish generalization or causal evidence use.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    seed = sub.add_parser('seed')
    seed.add_argument('--binary', required=True, type=Path)
    seed.add_argument('--root', required=True, type=Path)
    freeze_parser = sub.add_parser('freeze')
    for flag in ('binary', 'withdraw-binary', 'seed-root', 'root', 'runtime'):
        freeze_parser.add_argument('--'+flag, required=True, type=Path)
    for command in ('run', 'audit'):
        sub.add_parser(command).add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    if args.command == 'seed':
        result = seed_experiences(args.binary, args.root)
    elif args.command == 'freeze':
        result = freeze(args.binary, args.withdraw_binary, args.seed_root, args.root, read(args.runtime))
    elif args.command == 'run':
        result = run_trial(args.root)
    else:
        result = audit(args.root)
    print(encode(result), end='')


if __name__ == "__main__":
    main()
