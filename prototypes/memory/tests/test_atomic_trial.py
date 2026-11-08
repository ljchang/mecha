"""Offline authored v4 auditor regressions; no fresh evaluation labels."""
import copy
import json
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
import atomic_trial as audit


class AccountingTests(unittest.TestCase):
    def test_missing_rows_keep_every_arm_in_denominator(self):
        result = audit.summarize(['one', 'two'], [])
        for arm in ('pipeline', 'direct'):
            self.assertEqual(result[arm]['planned_cases'], 2)
            self.assertEqual(result[arm]['completed_cases'], 0)
            self.assertEqual(result[arm]['unknown_semantic_cases'], 2)
            self.assertIsNone(result[arm]['factual_defects'])
            self.assertIsNone(result[arm]['input_tokens'])
            self.assertEqual(result[arm]['criterion_accuracy'], 0)

    def test_partial_audit_keeps_observed_defects(self):
        row = audit.empty_row('one', 'pipeline')
        row.update(semantic_complete=False, observed_factual_defects=2,
                   observed_citation_defects=1, criterion_correct=False,
                   observed_input_tokens=17)
        result = audit.summarize(['one', 'two'], [row])['pipeline']
        self.assertEqual(result['observed_factual_defects'], 2)
        self.assertEqual(result['observed_citation_defects'], 1)
        self.assertEqual(result['observed_input_tokens'], 17)
        self.assertIsNone(result['factual_defects'])
        self.assertIsNone(result['input_tokens'])
        self.assertEqual(result['criterion_incorrect_cases'], 1)
        self.assertEqual(result['criterion_unknown_cases'], 1)

    def test_duplicate_or_unplanned_rows_refused(self):
        row = audit.empty_row('one', 'pipeline')
        for rows in ([row, row], [audit.empty_row('foreign', 'direct')]):
            with self.assertRaises(ValueError):
                audit.summarize(['one'], rows)


class StrictJsonTests(unittest.TestCase):
    def test_duplicate_nonfinite_or_truncated_json_refused(self):
        for text in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":'):
            with self.assertRaises(ValueError):
                audit.decode(text)




def packet(key='a'):
    request_key = key * 64
    source = dict(id='source:test', source=dict(object='source', revision='revision'),
                  field='/text', kind='Observation', text='The artifact contains zero records.')
    row = dict(handle='Observation:' + request_key + ':c0', source_id=source['id'],
               source=source['source'], field=source['field'], kind=source['kind'], quote=source['text'])
    return dict(protocol='situated-support-wire-v4', request_key=request_key,
        criterion=dict(goal='g', goal_text='Count', criterion_text='One record', criterion_revision='c', confirmation_digest='d'),
        sources=[source], evidence_metadata=[dict(source_id=source['id'], role='Unclassified', interpretation_influenced=False, captures=[])],
        catalog=dict(request_key=request_key, citations=[row]))


def generation(p):
    return dict(request_key=p['request_key'], observations=[dict(text='The artifact contains zero records.', evidence=p['catalog']['citations'][0]['handle'])])


def checked(p):
    proposal = audit.observations(p, generation(p))
    cp = dict(packet('b'), proposal=proposal, targets=['unit:o1'])
    response = dict(request_key=cp['request_key'], checks=[dict(target='unit:o1', status='Supported', original_citation='DirectSupport', reason='The original records this count.', questions=[])])
    return proposal, cp, response


class ProtocolTests(unittest.TestCase):
    def test_host_assigns_ids_and_preserves_original_citation(self):
        p = packet()
        value = audit.observations(p, generation(p))
        self.assertEqual(value['units'][0]['id'], 'o1')
        self.assertEqual(value['units'][0]['citations'], [{'source_id':'source:test', 'quote':p['sources'][0]['text']}])
        for mutation in (lambda v: v['observations'][0].update(id='model-id'),
                         lambda v: v.update(judgments=[]),
                         lambda v: v['observations'][0].update(evidence=packet('b')['catalog']['citations'][0]['handle']),
                         lambda v: v.update(request_key='f'*64),
                         lambda v: v['observations'][0].update(text='x'*129)):
            response = generation(p)
            mutation(response)
            with self.assertRaises(ValueError):
                audit.observations(p, response)

    def test_exact_check_targets_no_omission_foreign_or_duplicate(self):
        proposal, cp, response = checked(packet())
        for change in (lambda v: v['checks'].clear(), lambda v: v['checks'].append(v['checks'][0]),
                       lambda v: v['checks'][0].update(target='unit:foreign'),
                       lambda v: v['checks'][0].update(original_citation='ContextOnly'),
                       lambda v: v['checks'][0].update(questions=['Which?']),
                       lambda v: v['checks'][0].update(status='Unknown')):
            forged = copy.deepcopy(response)
            change(forged)
            with self.assertRaises(ValueError):
                audit.factual_checks(cp, forged, proposal)

    def test_checker_cannot_rewrite_original_slot(self):
        proposal, cp, response = checked(packet())
        cp['proposal'] = copy.deepcopy(proposal)
        cp['proposal']['units'][0]['citations'][0]['quote'] = 'A replacement quote that sounds right.'
        with self.assertRaises(ValueError):
            audit.factual_checks(cp, response, proposal)

    def test_canonical_check_keeps_original_contradiction_flag(self):
        proposal, cp, response = checked(packet())
        response['checks'][0].update(status='Unknown', original_citation='Uncertain', questions=['What was recorded?'])
        checks = audit.factual_checks(cp, response, proposal)
        value = audit.canonical_checks(proposal, checks)['checks'][0]
        self.assertFalse(value['unsupported_factual_premise'])
        self.assertEqual(value['citations'], [])
        response['checks'][0]['original_citation'] = 'Contradiction'
        checks = audit.factual_checks(cp, response, proposal)
        self.assertTrue(audit.canonical_checks(proposal, checks)['checks'][0]['unsupported_factual_premise'])

    def test_appraiser_uses_only_supported_original_ids(self):
        proposal, cp, response = checked(packet())
        checks = audit.factual_checks(cp, response, proposal)
        ap = packet('c')
        final = dict(request_key=ap['request_key'], state='not_met', because=['o1'], reason='No record meets the requirement.', questions=[])
        audit.appraisal(ap, final, proposal, checks)
        for because in ([], ['o1','o1'], ['o2']):
            with self.assertRaises(ValueError):
                audit.appraisal(ap, dict(final,because=because), proposal, checks)
        checks['unit:o1']['status'] = 'Unknown'
        with self.assertRaises(ValueError):
            audit.appraisal(ap, final, proposal, checks)
        with self.assertRaises(ValueError):
            audit.appraisal(ap, dict(final,state='unknown'), proposal, checks)

    def test_catalog_tampering_detected(self):
        for mutation in (lambda p: p['sources'][0].update(text='Changed source contents.'),
                         lambda p: p['catalog']['citations'][0].update(source_id='other'),
                         lambda p: p['evidence_metadata'][0].update(source_id='other')):
            p = packet()
            mutation(p)
            with self.assertRaises(ValueError):
                audit.catalog(p)


class SemanticTests(unittest.TestCase):
    def annotation(self, targets):
        return dict(protocol=audit.AUDIT_PROTOCOL,report_sha256='hash', arm='pipeline', reviewer='reviewer-two',
            reviewer_kind='agent', blinded=True, independent=True,
            reviews=[dict(target=t['id'], sha256=t['sha256'], assertions_reviewed=1,
                citation_slots_reviewed=1, factual_defects=1, citation_defects=1,
                diagnostic_factual_defects=1, diagnostic_citation_defects=1,
                all_assertions_and_citations_reviewed=True) for t in targets])

    def test_structural_success_never_proves_semantics(self):
        row = audit.empty_row('one','pipeline')
        row.update(structural_valid=True, completed=True)
        audit.semantic_review(row, None, 'hash')
        self.assertFalse(row['semantic_complete'])
        self.assertIsNone(row['criterion_correct'])

    def test_truncated_and_reasoning_blocks_still_require_review(self):
        reply = dict(message=dict(content=[dict(type='text', text='{"observations":['), dict(type='thinking', thinking='An unsupported factual inference.')]))
        targets = audit.review_targets('generate', reply)
        self.assertEqual(len(targets), 2)
        row = audit.empty_row('one','pipeline')
        row['review_targets'] = targets
        annotation = self.annotation(targets[:1])
        audit.semantic_review(row, annotation, 'hash')
        self.assertEqual(row['observed_factual_defects'], 1)
        self.assertEqual(row['observed_citation_defects'], 1)
        self.assertFalse(row['semantic_complete'])

    def test_wrong_report_hash_and_unblinded_review_are_unknown(self):
        targets = audit.review_targets('appraise', dict(message=dict(content=[dict(type='text',text='Judgment.')])) )
        for mutation in (lambda a: a.update(report_sha256='wrong'),lambda a:a.update(blinded=False),lambda a:a.update(independent=False)):
            row = audit.empty_row('one','pipeline')
            row['review_targets'] = targets
            annotation = self.annotation(targets)
            annotation['criterion'] = dict(target=targets[0]['id'], correct=True)
            mutation(annotation)
            audit.semantic_review(row,annotation,'hash')
            self.assertFalse(row['semantic_complete'])
            self.assertEqual(row['observed_factual_defects'],0)

    def test_complete_external_review_can_record_incorrect_final(self):
        targets = [target for stage in ('generate','check','appraise') for target in audit.review_targets(stage,dict(message=dict(content=[dict(type='text',text='Judgment.')])))]
        row = audit.empty_row('one','pipeline')
        row['review_targets'] = targets
        annotation = self.annotation(targets)
        row['observation_inventory_complete'] = True
        row['criterion_review'] = dict(sha256='binding',state='unknown')
        annotation['criterion'] = dict(target=targets[-1]['id'],correct=False,binding_sha256='binding',selected_premises_sufficient=False,temporally_applicable=False)
        audit.semantic_review(row,annotation,'hash')
        self.assertTrue(row['semantic_complete'])
        self.assertFalse(row['criterion_correct'])


class NativeContractTests(unittest.TestCase):
    @unittest.skipUnless(__import__('os').environ.get('MECHA_ATOMIC_NATIVE_BINARY'),
                         'set MECHA_ATOMIC_NATIVE_BINARY to require actual authored HTTP contract')
    def test_actual_native_binary_and_raw_auditor(self):
        import hashlib
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        import os
        import subprocess
        import tempfile
        import threading
        import urllib.parse
        binary = Path(os.environ['MECHA_ATOMIC_NATIVE_BINARY']).resolve(strict=True)
        model = 'authored-atomic-contract'
        case = dict(id='atomic-contract', native=dict(input_text='item,keep\ns7,yes\ns8,no\n',
            requested_output='[{"item":"s7"}]', observed_output='[{"item":"s7"}]',
            historical_criterion='Include only keep=yes items.', current_criterion='Include only keep=yes items.', goal_text='Select recorded items.'),
            probe=dict(units=[],judgments=[]))
        calls, errors = [], []
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args):
                pass
            def respond(self,status,value):
                data = json.dumps(value).encode()
                self.send_response(status)
                self.send_header('Content-Type','application/json')
                self.send_header('Content-Length',str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            def do_GET(self):
                path = urllib.parse.urlsplit(self.path).path
                if path.endswith('/models'):
                    self.respond(200,dict(data=[dict(id=model,status=dict(value='loaded'))]))
                elif path == '/props':
                    self.respond(200,dict(role='router'))
                else:
                    self.respond(404,dict(error='unknown route'))
            def do_POST(self):
                try:
                    url = urllib.parse.urlsplit(self.path)
                    audit.require(url.path.endswith('/chat/completions') and urllib.parse.parse_qs(url.query).get('autoload') == ['false'], 'resident-only request required')
                    body = audit.decode(self.rfile.read(int(self.headers['Content-Length'])).decode())
                    text = body['messages'][-1]['content']
                    p = audit.decode(text[len('<support-input>\n'):-len('\n</support-input>')])
                    schema = body['response_format']['schema']
                    props = (schema['oneOf'][0] if 'oneOf' in schema else schema)['properties']
                    audit.require(p['protocol'] == audit.WIRE, 'wire mismatch')
                    response = dict(request_key=p['request_key'])
                    if 'checks' in props:
                        stage = 'check'
                        response['checks'] = [dict(target=target,status='Supported',original_citation='DirectSupport',reason='The original records the observed item.',questions=[]) for target in p['targets']]
                    elif 'state' in props:
                        stage = 'appraise'
                        response.update(state='met',because=['o1'],reason='The item meets the criterion.',questions=[])
                    else:
                        stage = 'direct' if 'criterion' in props else 'generate'
                        handle = next(c['handle'] for c in p['catalog']['citations'] if c['kind']=='ArtifactObservation')
                        response['observations'] = [dict(text='The artifact records item s7.',evidence=handle)]
                        if stage == 'direct':
                            response['criterion'] = dict(state='met',because=[0],reason='The item meets the criterion.',questions=[])
                    audit.require(body['max_tokens'] == audit.LIMITS[stage], 'wrong per-stage budget')
                    calls.append(stage)
                    self.respond(200,dict(model=model,choices=[dict(message=dict(role='assistant',content=json.dumps(response)),finish_reason='stop')],usage=dict(prompt_tokens=100,completion_tokens=100)))
                except Exception as error:
                    errors.append(str(error))
                    self.respond(500,dict(error=str(error)))
        from contextlib import nullcontext
        persistent = os.environ.get('MECHA_ATOMIC_SMOKE_ROOT')
        if persistent:
            Path(persistent).mkdir(parents=True, exist_ok=False)
        with (nullcontext(persistent) if persistent else tempfile.TemporaryDirectory()) as directory:
            root = Path(directory).resolve()
            casefile = root/'case.json'
            casefile.write_text(json.dumps(dict(version=1,case_id=case['id'],case_sha256=audit.digest(case),native=case['native'],probe=case['probe'])))
            server = ThreadingHTTPServer(('127.0.0.1',0),Handler)
            worker = threading.Thread(target=server.serve_forever,daemon=True)
            worker.start()
            environment = os.environ.copy()
            for key in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','MECHA_MODEL','MECHA_PROVIDER','MECHA_EFFORT'):
                environment.pop(key,None)
            environment.update(MECHA_SESSION_KIND='test',MECHA_LOG='warn')
            try:
                completed = subprocess.run([str(binary),'--synthetic-only',str(root/'native'),
                    f'http://127.0.0.1:{server.server_port}',model,str(root/'holds'),str(audit.SEED),str(casefile),'0'],
                    capture_output=True,timeout=120,env=environment,cwd=root)
            finally:
                server.shutdown()
                server.server_close()
                worker.join(timeout=5)
            (root/'native.stdout').write_bytes(completed.stdout)
            (root/'native.stderr').write_bytes(completed.stderr)
            (root/'authored-calls.json').write_text(json.dumps(calls, indent=2))
            self.assertFalse(errors, errors)
            self.assertEqual(completed.returncode,0,completed.stderr.decode())
            self.assertEqual(calls,['generate','check','appraise','direct'])
            rows = audit.audit_case(root/'native',case,model)
            (root/'independent-audit.json').write_text(json.dumps(rows,indent=2))
            self.assertTrue(all(r['structural_valid'] for r in rows),[r['errors'] for r in rows])
            self.assertTrue(all(r['completed'] for r in rows))
            self.assertTrue(all(not r['semantic_complete'] for r in rows))
            self.assertEqual([r['input_tokens'] for r in rows],[300,100])
            # Rehash modified JSON exports: a matching descriptor must not make
            # changed criterion, originals, premise selection or persistence pass.
            report_path = root/'native'/'report.json'
            original_bytes = report_path.read_bytes()
            original = audit.decode(original_bytes.decode())
            mutations = [('numbered_observations',lambda v:v['units'][0].update(id='o9')),
                ('fact_checks',lambda v:v['checks'][0].update(status='Unknown')),
                ('appraisal',lambda v:v.update(because=['o9'])),
                ('retained',lambda v:v['draft']['claims'][0].update(statement='Invented retained fact.'))]
            for key,mutate in mutations:
                report = copy.deepcopy(original)
                descriptor = report['arms']['pipeline']['artifacts'][key]
                path = root/'native'/descriptor['path']
                saved = path.read_bytes()
                value = audit.decode(saved.decode())
                mutate(value)
                data = json.dumps(value).encode()
                path.write_bytes(data)
                descriptor.update(sha256=hashlib.sha256(data).hexdigest(),bytes=len(data))
                report_path.write_text(json.dumps(report))
                changed = audit.audit_case(root/'native',case,model)
                self.assertFalse(changed[0]['structural_valid'],key)
                path.write_bytes(saved)
            report_path.write_bytes(original_bytes)


class DurableTests(unittest.TestCase):
    def fixture(self):
        p = packet()
        proposal, cp, response = checked(p)
        checks = audit.factual_checks(cp,response,proposal)
        final = dict(state='not_met',because=['o1'],reason='The record count is insufficient.',questions=[])
        unit = proposal['units'][0]
        support = dict(protocol='situated-atomic-appraisal-v4',generator_model='m',evaluator_model='m',proposal_hash='d'*64,request_hash='e'*64,
            units=[dict(disposition='Accepted',unit=unit,status='Supported')],
            judgments=[dict(index=0,goal='g',offered_bearing='Bad',bearing='Bad',because=['o1'],status='Supported',reason='Retained')],
            atomic=dict(extraction_binding='a'*64,checking_binding='b'*64,appraisal_binding='c'*64,appraiser_model='m'))
        record = dict(context_key='context',host_context_id='host',criterion=p['criterion'],
            claim_sources=[{k:p['sources'][0][k] for k in ('id','source','field','kind')}],
            draft=dict(interpretation='Recorded event: '+unit['text'],claims=[dict(source_id='source:test',statement=unit['text'],quote=p['sources'][0]['text'])],
                judgments=[dict(goal='g',bearing='Bad',because=[0])],prediction=None,model='m',producer_version='situated-interpretation-v4',support=support,
                grounding=dict(offered=1,dropped=0,dropped_by={},unresolved_goals=0)))
        return record, proposal, checks, final, p

    def test_durable_tampering_cannot_rewrite_checked_content(self):
        record, proposal, checks, final, p = self.fixture()
        bindings = [c*64 for c in 'abc']
        audit.validate_retained(record,proposal,checks,final,p,'m',bindings)
        mutations = [lambda r:r['draft']['claims'][0].update(statement='A false stored fact.'),
            lambda r:r['draft']['support']['units'][0]['unit']['citations'][0].update(quote='A replacement quote.'),
            lambda r:r['draft']['judgments'][0].update(because=[]),
            lambda r:r['criterion'].update(criterion_text='An easier criterion.'),
            lambda r:r['draft']['support']['atomic'].update(checking_binding='0'*64),
            lambda r:r['claim_sources'][0].update(field='/other'),
            lambda r:r['draft']['grounding'].update(dropped=1)]
        for mutate in mutations:
            bad = copy.deepcopy(record)
            mutate(bad)
            with self.assertRaises(ValueError):
                audit.validate_retained(bad,proposal,checks,final,p,'m',bindings)

    def test_sqlite_row_receipt_and_all_source_dependencies(self):
        import hashlib
        import sqlite3
        import tempfile
        record, _, _, _, p = self.fixture()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference = dict(object='retained',revision='r2')
            def artifact(name,value):
                data = value if isinstance(value,bytes) else json.dumps(value).encode()
                (root/name).write_bytes(data)
                return dict(path=name,sha256=hashlib.sha256(data).hexdigest(),bytes=len(data))
            db = sqlite3.connect(':memory:')
            db.executescript('CREATE TABLE revision(id TEXT,object_id TEXT,lifecycle TEXT,payload TEXT); CREATE TABLE interpretation_receipt(receipt_id TEXT,context_key TEXT,revision_id TEXT,object_id TEXT,state TEXT); CREATE TABLE dependency(output TEXT,input TEXT);')
            db.execute('INSERT INTO revision VALUES(?,?,?,?)',('r2','retained','active',json.dumps(dict(kind='interpretation',data=record))))
            db.execute('INSERT INTO interpretation_receipt VALUES(?,?,?,?,?)',('receipt','context','r2','retained','active'))
            db.execute('INSERT INTO dependency VALUES(?,?)',('r2','revision'))
            artifacts = dict(receipt=artifact('receipt.json',dict(id='receipt',reference=reference)),
                database=artifact('db.sqlite',db.serialize()),
                recall=artifact('recall.json',dict(memory=dict(interpretation_roles=[dict(reference=reference,role='SelectedCurrent')],records=[dict(reference=reference,record=dict(kind='interpretation',data=record))]))),
                historical=artifact('historical.json',dict(memory=dict(interpretation_roles=[]))),
                changed_goal=artifact('changed.json',dict(memory=dict(interpretation_roles=[]))))
            audit.validate_persistence(root,artifacts,record,p)
            db.execute('DELETE FROM dependency')
            artifacts['database'] = artifact('db.sqlite',db.serialize())
            with self.assertRaisesRegex(ValueError,'dependencies'):
                audit.validate_persistence(root,artifacts,record,p)
            db.execute('INSERT INTO dependency VALUES(?,?)',('r2','revision'))
            db.execute('UPDATE interpretation_receipt SET receipt_id=?',('forged',))
            artifacts['database'] = artifact('db.sqlite',db.serialize())
            with self.assertRaisesRegex(ValueError,'receipt'):
                audit.validate_persistence(root,artifacts,record,p)
            db.close()



class FailedResponseTests(unittest.TestCase):
    def test_identity_rejected_response_still_counts_usage_and_assertions(self):
        import tempfile
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            event = dict(event='error', call=1, error='wrong model', rejected_response=dict(
                message=dict(content=[dict(type='text',text='A false observed statement.')]),
                native_usage=dict(input_tokens=12,cache_creation_input_tokens=3,cache_read_input_tokens=2,output_tokens=9)))
            data = (json.dumps(event)+'\n').encode()
            (root/'audit.jsonl').write_bytes(data)
            stage = dict(provider_audit=dict(path='audit.jsonl',sha256=hashlib.sha256(data).hexdigest(),bytes=len(data)))
            row = audit.empty_row('one','pipeline')
            with self.assertRaises((ValueError,KeyError)):
                audit.inspect_call(root,stage,'generate','m',row)
            self.assertEqual(len(row['review_targets']),1)
            self.assertEqual(row['observed_input_tokens'],17)
            self.assertEqual(row['observed_output_tokens'],9)
            self.assertFalse(row['usage_complete'])


class BrokenArtifactTests(unittest.TestCase):
    def test_malformed_reports_still_return_two_failed_rows(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for content in ('[]','null','{"arms":[]}','{"arms":{"pipeline":[]}}','{"arms":'):
                (root/'report.json').write_text(content)
                rows = audit.audit_case(root,dict(id='case'),'model')
                self.assertEqual(len(rows),2)
                self.assertTrue(all(not row['structural_valid'] for row in rows))
                self.assertTrue(all(row['errors'] for row in rows))


class BindingTests(unittest.TestCase):
    def test_phase_hash_uses_typed_metadata_order_and_opaque_identity(self):
        metadata = packet()['evidence_metadata']
        reordered = [dict(reversed(list(row.items()))) for row in metadata]
        args = ('a'*64,metadata,'atomic-v4-extraction','{"sources":[]}')
        expected = audit.binding_hash(*args)
        self.assertEqual(expected,audit.binding_hash(args[0],reordered,args[2],args[3]))
        self.assertNotEqual(expected,audit.binding_hash('b'*64,*args[1:]))
        self.assertNotEqual(expected,audit.binding_hash(args[0],args[1],'atomic-v4-checking',args[3]))
        self.assertNotEqual(expected,audit.binding_hash(*args[:3],'{"sources":[1]}'))

    def test_recomputed_key_rejects_forged_host_identity_and_input(self):
        import hashlib
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            p = packet()
            initial = {k:p[k] for k in ('criterion','sources','evidence_metadata')}
            raw = json.dumps({k:initial[k] for k in ('criterion','sources')},sort_keys=True,separators=(',',':'))
            context = dict(prepared_request_id='a'*64,generation_input=raw,evidence_metadata_json=json.dumps(initial['evidence_metadata']))
            def descriptor():
                data = json.dumps(context).encode()
                (root/'context.json').write_bytes(data)
                return dict(path='context.json',sha256=hashlib.sha256(data).hexdigest(),bytes=len(data))
            p['request_key'] = audit.binding_hash(context['prepared_request_id'],initial['evidence_metadata'],'atomic-v4-extraction',raw)
            audit.validate_bindings(root,dict(binding_context=descriptor()),{},dict(direct=(p,{})),initial)
            context['prepared_request_id'] = 'b'*64
            with self.assertRaisesRegex(ValueError,'request key'):
                audit.validate_bindings(root,dict(binding_context=descriptor()),{},dict(direct=(p,{})),initial)
            context['generation_input'] = '{"criterion":{},"sources":[]}'
            with self.assertRaisesRegex(ValueError,'initial evidence'):
                audit.validate_bindings(root,dict(binding_context=descriptor()),{},dict(direct=(p,{})),initial)


class GrammarContractTests(unittest.TestCase):
    def test_every_oneof_branch_is_a_complete_object(self):
        p = packet()
        p.update(targets=['unit:o1'],supported_ids=['o1'])
        def verify(node):
            if isinstance(node,dict):
                if 'oneOf' in node:
                    for branch in node['oneOf']:
                        self.assertEqual(branch.get('type'),'object')
                        self.assertIs(branch.get('additionalProperties'),False)
                        self.assertEqual(set(branch['required']),set(branch['properties']))
                for value in node.values():
                    verify(value)
            elif isinstance(node,list):
                for value in node:
                    verify(value)
        for stage in audit.LIMITS:
            verify(audit.expected_schema(p,stage))


class FreezeTests(unittest.TestCase):
    def test_exclusive_implementation_boundary_and_missing_result_denominators(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary, source = root/'binary', root/'source'
            binary.write_bytes(b'authored binary hash fixture')
            source.write_text('original source')
            implementation = root/'implementation.json'
            audit.freeze_implementation(implementation,binary,[source])
            with self.assertRaises(FileExistsError):
                audit.freeze_implementation(implementation,binary,[source])
            # IDs only test manifest accounting; these are not qualification cases.
            fixture = root/'cases.json'
            fixture.write_text(json.dumps(dict(cases=[dict(id='case-'+str(i)) for i in range(8)])))
            source.write_text('changed source')
            with self.assertRaisesRegex(ValueError,'implementation changed'):
                audit.freeze(root/'trial.json',fixture,binary,'model',[],implementation)
            source.write_text('original source')
            trial = root/'trial.json'
            audit.freeze(trial,fixture,binary,'model',[],implementation)
            report = audit.score(trial,root/'missing-runs')
            for arm in audit.ARMS:
                self.assertEqual(report['summary'][arm]['planned_cases'],8)
                self.assertEqual(report['summary'][arm]['unknown_semantic_cases'],8)
                self.assertIsNone(report['summary'][arm]['input_tokens'])
            saved_manifest = trial.read_text()
            tampered = audit.decode(saved_manifest)
            tampered['quality_gate']['known_bearings_require_sufficient_current_premises'] = False
            trial.write_text(json.dumps(tampered))
            with self.assertRaisesRegex(ValueError,'semantic gate'):
                audit.score(trial,root/'missing-runs')
            trial.write_text(saved_manifest)
            source.write_text('changed after freeze')
            with self.assertRaisesRegex(ValueError,'frozen file changed'):
                audit.score(trial,root/'missing-runs')


class MeasurementReviewTests(unittest.TestCase):
    def reviewed_row(self, accepted_defect=False, sufficient=True, applicable=True):
        p = packet()
        generated = generation(p)
        generated['observations'] = [dict(generated['observations'][0],text='The artifact contains nine records.'), generated['observations'][0]]
        proposal = audit.observations(p,generated)
        cp = dict(packet('b'),proposal=proposal,targets=['unit:o1','unit:o2'])
        checked_payload = dict(request_key=cp['request_key'],checks=[
            dict(target='unit:o1',status='Contradicted',original_citation='Contradiction',reason='The original contradicts this count.',questions=[]),
            dict(target='unit:o2',status='Supported',original_citation='DirectSupport',reason='The original records this count.',questions=[])])
        checks = audit.factual_checks(cp,checked_payload,proposal)
        ap = dict(packet('c'),proposal=proposal,checks=audit.canonical_checks(proposal,checks)['checks'],supported_ids=['o2'])
        final = dict(request_key=ap['request_key'],state='not_met',because=['o2'],reason='Zero records cannot meet the requirement.',questions=[])
        calls = dict(generate=(p,generated),check=(cp,checked_payload),appraise=(ap,final))
        row = audit.empty_row('one','pipeline')
        row.update(completed=True,structural_valid=True,retention_verified=True)
        for stage, (_,payload) in calls.items():
            row['review_targets'].extend(audit.review_targets(stage,dict(message=dict(content=[dict(type='text',text=json.dumps(payload))]))))
        audit.prepare_semantic_context(row,calls)
        annotations = dict(protocol=audit.AUDIT_PROTOCOL,report_sha256='hash',arm='pipeline',reviewer='independent-reviewer',reviewer_kind='human',blinded=True,independent=True,
            reviews=[dict(target=t['id'],sha256=t['sha256'],assertions_reviewed=2,citation_slots_reviewed=2,
                          factual_defects=1 if t['id'].startswith('generate:') else 0,
                          citation_defects=1 if t['id'].startswith('generate:') else 0,
                          diagnostic_factual_defects=0,diagnostic_citation_defects=0,
                          all_assertions_and_citations_reviewed=True) for t in row['review_targets']],
            observation_reviews=[dict(id=t['id'],sha256=t['sha256'],factual_correct=(t['id'] != ('o2' if accepted_defect else 'o1')),
                                      original_citation_correct=(t['id'] != ('o2' if accepted_defect else 'o1'))) for t in row['observation_targets']],
            criterion=dict(target='appraise:0:text',binding_sha256=row['criterion_review']['sha256'],correct=True,
                           selected_premises_sufficient=sufficient,temporally_applicable=applicable))
        audit.semantic_review(row,annotations,'hash')
        return row,annotations

    def test_rejected_generation_defect_remains_observed_but_passes_retained_gate(self):
        row,_ = self.reviewed_row()
        self.assertTrue(row['semantic_complete'],row['errors'])
        summary = audit.summarize(['one'],[row])['pipeline']
        self.assertEqual(summary['generated_factual_defects'],1)
        self.assertEqual(summary['rejected_factual_defects'],1)
        self.assertEqual(summary['accepted_factual_defects'],0)
        self.assertEqual(summary['retained_factual_defects'],0)
        self.assertEqual(summary['retained_citation_defects'],0)
        self.assertTrue(summary['retained_error_gate_passed'])
        self.assertFalse(summary['all_output_error_free'])
        self.assertFalse(summary['pilot_gate_passed'])  # eight planned cases required

    def test_accepted_atom_defect_fails_retained_gate(self):
        row,_ = self.reviewed_row(accepted_defect=True)
        summary = audit.summarize(['one'],[row])['pipeline']
        self.assertEqual(summary['accepted_factual_defects'],1)
        self.assertEqual(summary['retained_factual_defects'],1)
        self.assertEqual(summary['retained_citation_defects'],1)
        self.assertFalse(summary['retained_error_gate_passed'])

    def test_correct_label_with_insufficient_or_historical_premises_does_not_qualify(self):
        for sufficient,applicable in ((False,True),(True,False)):
            row,_ = self.reviewed_row(sufficient=sufficient,applicable=applicable)
            summary = audit.summarize(['one'],[row])['pipeline']
            self.assertEqual(summary['criterion_correct_cases'],1)
            self.assertTrue(row['semantic_complete'])
            self.assertFalse(summary['retained_error_gate_passed'])

    def test_eight_complete_supported_outcomes_meet_pilot_despite_rejected_errors(self):
        row,_ = self.reviewed_row()
        rows = [dict(copy.deepcopy(row),case_id='row-'+str(i)) for i in range(8)]
        summary = audit.summarize([r['case_id'] for r in rows],rows)['pipeline']
        self.assertTrue(summary['pilot_gate_passed'])
        self.assertEqual(summary['rejected_factual_defects'],8)
        self.assertEqual(summary['retained_factual_defects'],0)


class UsageCompletenessTests(unittest.TestCase):
    def test_known_malformed_length_and_rejected_response_usage_is_exact(self):
        for event in (
            dict(event='response',call=1,input_tokens=17,output_tokens=9,stop_reason='end_turn',message=dict(content=[dict(type='text',text='{malformed')])),
            dict(event='response',call=1,input_tokens=17,output_tokens=9,stop_reason='max_tokens',message=dict(content=[dict(type='text',text='truncated')])),
            dict(event='error',call=1,rejected_response=dict(native_usage=dict(input_tokens=12,cache_creation_input_tokens=3,cache_read_input_tokens=2,output_tokens=9)))):
            row = audit.empty_row('one','pipeline')
            audit.account_call_usage(row,'generate',dict(status='failed'),[dict(event='request_started',call=1),event],None,True)
            for stage in ('check','appraise'):
                audit.account_call_usage(row,stage,dict(status='withheld'),[],None,False)
            self.assertTrue(row['usage_complete'])
            self.assertEqual(row['input_tokens'],17)
            self.assertEqual(row['output_tokens'],9)
            self.assertFalse(row['completed'])
            summary = audit.summarize(['one'],[row])['pipeline']
            self.assertEqual(summary['unknown_usage_cases'],0)
            self.assertEqual(summary['input_tokens'],17)

    def test_started_call_without_usage_remains_unknown(self):
        row = audit.empty_row('one','direct')
        audit.account_call_usage(row,'direct',dict(status='failed'),[dict(event='request_started',call=1)],None,True)
        self.assertFalse(row['usage_complete'])
        self.assertIsNone(row['input_tokens'])
        missing = audit.empty_row('one','direct')
        audit.account_call_usage(missing,'direct',{},[],None,False)
        self.assertFalse(missing['usage_complete'])


class UsageIntegrationTests(unittest.TestCase):
    def test_failed_malformed_arm_costs_survive_full_audit_without_parsed_calls(self):
        import hashlib
        import tempfile
        from unittest import mock
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def artifact(name,value,raw=False):
                data = value.encode() if raw else json.dumps(value).encode()
                (root/name).write_bytes(data)
                return dict(path=name,sha256=hashlib.sha256(data).hexdigest(),bytes=len(data))
            request = dict(event='request_started',call=1,request=dict(think=False),wire_body=dict(chat_template_kwargs=dict(enable_thinking=False)))
            reply = dict(event='response',call=1,model='model',stop_reason='max_tokens',refusal=False,input_tokens=17,output_tokens=9,message=dict(content=[dict(type='text',text='{truncated')]))
            stage = dict(status='failed',error='response truncated',model='model',stop_reason='max_tokens',packet_bytes=100,input_tokens=17,output_tokens=9,
                         request=artifact('request.json',request),reply=artifact('reply.json',reply),
                         provider_audit=artifact('provider.jsonl',json.dumps(request)+'\n'+json.dumps(reply)+'\n',raw=True))
            withheld = dict(status='withheld',error='upstream stage unavailable')
            pipeline = dict(status='failed',error='response truncated',elapsed_ms=1,stages=dict(generate=stage,**{name:withheld for name in ('check','appraise','retain','recall','historical','changed_goal')}))
            case = dict(id='one')
            report = dict(protocol=audit.PROTOCOL,case_id='one',case_sha256=audit.digest(case),model='model',seed=audit.SEED,case_index=0,arm_order=['pipeline','direct'],
                          arms=dict(pipeline=pipeline,direct=dict(status='failed',error='response truncated',elapsed_ms=1,stages=dict(direct=stage))))
            (root/'report.json').write_text(json.dumps(report))
            with mock.patch.object(audit,'validate_initial',return_value=packet()), mock.patch.object(audit,'request_packet',return_value=packet()):
                rows = audit.audit_case(root,case,'model')
            self.assertTrue(all(r['usage_complete'] for r in rows),rows)
            self.assertTrue(all(not r['completed'] and not r['structural_valid'] for r in rows))
            self.assertEqual([r['input_tokens'] for r in rows],[17,17])
            self.assertEqual([len(r['review_targets']) for r in rows],[1,1])

    def test_missing_sufficiency_review_never_qualifies(self):
        row,annotation = MeasurementReviewTests().reviewed_row()
        # Re-run from an unreviewed copy while preserving structural attribution.
        fresh = audit.empty_row('one','pipeline')
        for key in ('review_targets','observation_targets','criterion_review','observation_inventory_complete','retention_verified','completed','structural_valid'):
            fresh[key] = copy.deepcopy(row[key])
        del annotation['criterion']['selected_premises_sufficient']
        audit.semantic_review(fresh,annotation,'hash')
        self.assertTrue(fresh['criterion_correct'])
        self.assertFalse(fresh['semantic_complete'])
        self.assertFalse(audit.summarize(['one'],[fresh])['pipeline']['retained_error_gate_passed'])


class AttributionCoverageTests(unittest.TestCase):
    def test_missing_or_wrong_original_binding_never_means_zero_accepted_errors(self):
        row,annotation = MeasurementReviewTests().reviewed_row()
        for mutation in (lambda a:a['observation_reviews'].pop(),lambda a:a['observation_reviews'][0].update(sha256='wrong')):
            fresh = audit.empty_row('one','pipeline')
            for key in ('review_targets','observation_targets','criterion_review','observation_inventory_complete','retention_verified','completed','structural_valid'):
                fresh[key] = copy.deepcopy(row[key])
            changed = copy.deepcopy(annotation)
            mutation(changed)
            audit.semantic_review(fresh,changed,'hash')
            summary = audit.summarize(['one'],[fresh])['pipeline']
            self.assertFalse(fresh['semantic_complete'])
            self.assertIsNone(summary['accepted_factual_defects'])
            self.assertFalse(summary['retained_error_gate_passed'])

    def test_direct_atoms_are_final_outputs_without_independent_retention(self):
        p = packet()
        payload = dict(generation(p),criterion=dict(state='not_met',because=[0],reason='The count is insufficient.',questions=[]))
        row = audit.empty_row('one','direct')
        row.update(completed=True,structural_valid=True)
        row['review_targets'] = audit.review_targets('direct',dict(message=dict(content=[dict(type='text',text=json.dumps(payload))])))
        audit.prepare_semantic_context(row,dict(direct=(p,payload)))
        target = row['review_targets'][0]
        original = row['observation_targets'][0]
        annotation = dict(protocol=audit.AUDIT_PROTOCOL,report_sha256='hash',arm='direct',reviewer='separate',reviewer_kind='human',blinded=True,independent=True,
            reviews=[dict(target=target['id'],sha256=target['sha256'],assertions_reviewed=1,citation_slots_reviewed=1,factual_defects=1,citation_defects=1,
                          diagnostic_factual_defects=0,diagnostic_citation_defects=0,all_assertions_and_citations_reviewed=True)],
            observation_reviews=[dict(id=original['id'],sha256=original['sha256'],factual_correct=False,original_citation_correct=False)],
            criterion=dict(target=target['id'],binding_sha256=row['criterion_review']['sha256'],correct=True,selected_premises_sufficient=True,temporally_applicable=True))
        audit.semantic_review(row,annotation,'hash')
        self.assertTrue(row['semantic_complete'],row['errors'])
        self.assertFalse(row['retention_verified'])
        summary = audit.summarize(['one'],[row])['direct']
        self.assertEqual(summary['generated_factual_defects'],1)
        self.assertEqual(summary['accepted_factual_defects'],1)
        self.assertEqual(summary['retained_factual_defects'],0)
        self.assertFalse(summary['retained_error_gate_passed'])


class FrozenLabelTests(unittest.TestCase):
    def test_frozen_gold_is_separate_and_cannot_be_overridden_by_reviewer(self):
        row,_ = MeasurementReviewTests().reviewed_row()
        self.assertEqual(row['final_state'],'not_met')
        audit.apply_frozen_label(row,dict(expected_state='not_met'))
        self.assertTrue(row['frozen_label_correct'])
        self.assertTrue(row['criterion_correct'])
        self.assertTrue(audit.summarize(['one'],[row])['pipeline']['retained_error_gate_passed'])
        audit.apply_frozen_label(row,dict(expected_state='met'))
        self.assertFalse(row['frozen_label_correct'])
        self.assertTrue(row['criterion_correct'])
        summary = audit.summarize(['one'],[row])['pipeline']
        self.assertEqual(summary['frozen_label_incorrect_cases'],1)
        self.assertEqual(summary['criterion_correct_cases'],1)
        self.assertFalse(summary['retained_error_gate_passed'])

    def test_missing_actual_final_state_has_unknown_gold_outcome(self):
        row = audit.empty_row('one','pipeline')
        audit.apply_frozen_label(row,dict(expected_state='unknown'))
        self.assertIsNone(row['frozen_label_correct'])
        self.assertIsNone(row['retained_bearing'])
        summary = audit.summarize(['one'],[row])['pipeline']
        self.assertEqual(summary['frozen_label_unknown_cases'],1)
        self.assertEqual(summary['frozen_label_accuracy'],0)


class ObservedLabelTests(unittest.TestCase):
    def test_missing_report_preserves_frozen_expected_label(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            rows = audit.audit_case(Path(directory),dict(id='one',expected_state='not_met'),'model')
            self.assertTrue(all(r['frozen_expected_state']=='not_met' for r in rows))
            self.assertTrue(all(r['frozen_label_correct'] is None for r in rows))
            self.assertEqual(audit.summarize(['one'],rows)['pipeline']['frozen_label_accuracy'],0)

    def test_raw_label_is_reported_even_when_other_payload_fields_are_invalid(self):
        row = audit.empty_row('one','pipeline')
        raw = dict(message=dict(content=[dict(type='text',text=json.dumps(dict(state='met',because=['foreign'])))]))
        audit.observe_final_state(row,'appraise',raw)
        audit.apply_frozen_label(row,dict(expected_state='not_met'))
        self.assertEqual(row['final_state'],'met')
        self.assertEqual(row['final_state_source'],'raw_reply')
        self.assertFalse(row['frozen_label_correct'])
        self.assertFalse(row['completed'])
        self.assertIsNone(row['retained_bearing'])
