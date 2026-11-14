"""Authored orchestration/scoring tests; no Cargo, models, or embedding calls."""
import contextlib
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
import urllib.request
from unittest import mock

MODULE = Path(__file__).resolve().parents[1] / 'native_vector_trial.py'
spec = importlib.util.spec_from_file_location('native_vector_trial', MODULE)
t = importlib.util.module_from_spec(spec)
spec.loader.exec_module(t)


def ref(name):
    return {'object': 'o-'+name, 'revision': 'r-'+name}


def evidence(text):
    return {'kind': 'evidence', 'data': {'source': {'connector': 'synthetic', 'account': 'synthetic', 'id': text[:5]}, 'text': text, 'scopes': ['synthetic']}}


def insert(conn, name, payload):
    r = ref(name)
    conn.execute('INSERT INTO revision VALUES (?,?,?)', (r['revision'], r['object'], t.encode(payload)))
    return r


class TrialTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fixture, self.protocol = t.load_fixture()
        self.binary = self.root/'binary'
        self.binary.write_text('authored inert binary identity')
        self.seed_root = self.root/'seed'
        self.seed_root.mkdir()
        (self.seed_root/'authority').mkdir()
        (self.seed_root/'authority'/'authority').write_text('authored test authority')
        db = self.seed_root/'seed.memory-prototype.sqlite'
        reports = {}
        with contextlib.closing(sqlite3.connect(db)) as conn, conn:
            conn.executescript('PRAGMA user_version=8; CREATE TABLE revision(id TEXT PRIMARY KEY,object_id TEXT,payload TEXT); CREATE TABLE dependency(output TEXT,input TEXT); CREATE TABLE host_capture_receipt(receipt_id TEXT,projection_object TEXT,projection_revision TEXT,originals TEXT,state TEXT);')
            for source in self.fixture['seeds']:
                sid = source['id']
                observation = insert(conn, sid+'-observation', evidence(source['observation']))
                action = insert(conn, sid+'-action', evidence('Historical choice: '+source['old_choice']))
                descriptor = dict(reference=observation, kind='ToolResult', line=2, pointer='/content/0/content')
                projection = insert(conn, sid, evidence(t.encode(dict(originals=[descriptor], binding=dict(locator='locator-'+sid)))))
                conn.executemany('INSERT INTO dependency VALUES (?,?)', [(projection['revision'], observation['revision']), (projection['revision'], action['revision'])])
                conn.execute('INSERT INTO host_capture_receipt VALUES (?,?,?,?,?)', ('capture-'+sid, projection['object'], projection['revision'], t.encode([observation]), 'active'))
                transcript = self.seed_root/(sid+'.jsonl')
                transcript.write_text(t.encode(dict(record='meta'))+t.encode(dict(record='message', role='user', content=[dict(type='tool_result',tool_use_id='read',content=source['observation'])])))
                reports[sid] = dict(confirmation='a'*64, transcript=str(transcript), receipt=dict(id='capture-'+sid, projection=projection, originals=[observation]))
        t.save(self.seed_root/'seed-reports.json', reports)
        self.runtime = dict(endpoint='http://127.0.0.1:8080', model='resident-test', holds_dir=str(self.root/'holds'),
                            observed_identity={'explicit_fixture': True}, dense=dict(endpoint='http://127.0.0.1:8081', expected_alias='embed-test', artifact_revision='fixture-artifact', dimensions=4, pooling='last', normalization='l2'))

    def fake_withdraw(self, argv, log):
        # Injectable helper boundary, never modifies an actual authority format.
        (Path(argv[2])/'locator-S4.withdrawn').write_text('withdrawn')
        return {'returncode': 0}

    def freeze(self):
        output = self.root/'trial'
        manifest = t.freeze(self.binary, self.binary, self.seed_root, output, self.runtime, call=self.fake_withdraw)
        return output, manifest

    def finish_slot(self, output, manifest, evidence_name='S1-observation', choice='UP4', extra_attempt=False, slot_index=0):
        slot = manifest['slots'][slot_index]
        case = self.fixture['cases'][0]
        directory = output/slot['directory']
        native = directory/'native'
        (native/'workspace').mkdir(parents=True)
        artifact = native/'workspace'/'output.json'
        t.save(artifact, [{'choice': choice}])
        database = directory/'arm.memory-prototype.sqlite'
        with contextlib.closing(sqlite3.connect(database)) as conn, conn:
            observed = t.canonical(conn, ref(evidence_name))
            projection = insert(conn, 'followup', evidence('current capture'))
            conn.execute('INSERT INTO dependency VALUES (?,?)', (projection['revision'], ref(evidence_name)['revision']))
            conn.execute('INSERT INTO host_capture_receipt VALUES (?,?,?,?,?)', ('followup', projection['object'], projection['revision'], '[]', 'active'))
        packet = {'records': [{'reference': ref(evidence_name), 'record': observed}]}
        call = dict(type='tool_use',id='a',name='goal_context',input={'memory': {'query': case['query']}})
        blocks = [call]
        if extra_attempt:
            blocks.append(dict(call, id='rejected-before-tool-dispatch'))
        transcript = native/'transcript.jsonl'
        transcript.write_text(t.encode(dict(record='message',role='assistant',content=blocks))+
                              t.encode(dict(record='message',role='user',content=[dict(type='tool_result',tool_use_id='a',content=t.encode({'memory':packet}),is_error=False)])))
        report = dict(artifact=str(artifact), transcript=str(transcript), check={'verdict':'Passed'},
                      receipt=dict(id='followup',projection=projection,originals=[]),run_error=None,capture_error=None,
                      recall=dict(entered_calls=1,admitted_calls=1,rejected_calls=0,status='completed',effective_mode=slot['mode'],fallback=None,
                                  identity=dict(protocol='native-recall-v1',mode=slot['mode'],query_digest=t.hashlib.sha256(case['query'].encode()).hexdigest())))
        t.save(native/'report.json',report)
        t.save(directory/'dispatch.started.json',{})
        t.save(directory/'dispatch.result.json',dict(returncode=0,elapsed_ms=1))
        return t.audit_slot(output,slot,case,manifest), report

    def test_fixture_is_portable_frozen_eight_pairs(self):
        self.assertEqual(len(self.fixture['cases']),8)
        self.assertEqual(self.protocol['scored_sessions'],16)
        self.assertEqual(self.protocol['budgets']['anchors'],3)
        self.assertEqual([c['order'][0] for c in self.fixture['cases']].count('lexical'),4)
        self.assertNotIn('protocol_sha256',self.protocol)

    def test_public_contract_has_no_gold_or_source_files(self):
        for case in self.fixture['cases']:
            request = t.make_request(self.fixture,case)
            self.assertEqual(request['files'],{})
            public = t.encode({k:request[k] for k in ('prompt','criterion','files')})
            gold = case['expected_rows'][0]['choice']
            if gold != 'unknown':
                self.assertNotIn(gold,public)
            for source in self.fixture['seeds']:
                self.assertNotIn(source['decisive_span'],public)
        self.assertIn('MK7',t.make_request(self.fixture,self.fixture['cases'][4])['prompt'])

    def test_duplicate_json_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'duplicate'):
            t.decode('{"choice":"unknown","choice":"UP4"}')

    def test_freeze_creates_all_sixteen_independent_slots_and_pair_requests(self):
        output, manifest = self.freeze()
        self.assertEqual(len(manifest['slots']),16)
        self.assertTrue(all(s['state']=='planned' for s in manifest['slots']))
        self.assertEqual(manifest['protocol_sha256'],t.sha(t.FIXTURES/'protocol.json'))
        dbs = [output/s['directory']/'arm.memory-prototype.sqlite' for s in manifest['slots']]
        self.assertEqual(len({p.stat().st_ino for p in dbs}),16)
        self.assertEqual(len({t.sha(p) for p in dbs}),1)
        dbs[0].write_bytes(b'changed independently')
        self.assertNotEqual(t.sha(dbs[0]),t.sha(dbs[1]))
        for i in range(0,16,2):
            a,b = manifest['slots'][i:i+2]
            self.assertEqual(t.read(output/a['directory']/'request.json'),t.read(output/b['directory']/'request.json'))
            self.assertNotIn(a['mode'],a['directory'])

    def test_decisive_closure_does_not_require_unrelated_capture_siblings(self):
        _, manifest = self.freeze()
        support = manifest['support']['S1']
        self.assertEqual(support['closure'],['r-S1-observation'])
        self.assertIn('r-S1-action',support['capture_closure'])

    def test_seed_descriptor_must_be_authenticated_and_observed(self):
        db = self.seed_root/'seed.memory-prototype.sqlite'
        reports = t.read(self.seed_root/'seed-reports.json')
        reports['S1']['receipt']['originals']=[]
        with self.assertRaisesRegex(ValueError,'receipt mismatch'):
            t.seed_support(db,reports['S1'],self.fixture['seeds'][0])
        reports = t.read(self.seed_root/'seed-reports.json')
        Path(reports['S1']['transcript']).write_text('{}\n{}\n')
        with self.assertRaises((ValueError,KeyError)):
            t.seed_support(db,reports['S1'],self.fixture['seeds'][0])

    def test_frozen_original_tampering_refuses_before_dispatch(self):
        output,_ = self.freeze()
        (self.seed_root/'S1.jsonl').write_text('tampered')
        with self.assertRaisesRegex(ValueError,'originals changed'):
            t.run_trial(output,call=lambda *_:self.fail('must not dispatch'))
        self.assertFalse((output/'execution-started.json').exists())

    def test_pair_request_tamper_refuses(self):
        output,manifest = self.freeze()
        path=output/manifest['slots'][0]['directory']/'request.json'
        request=t.read(path);request['prompt']+=' UP4'
        path.write_text(t.encode(request))
        with self.assertRaisesRegex(ValueError,'paired public input'):
            t.verify_frozen(output)

    def test_no_replay_after_started_marker(self):
        output,_ = self.freeze()
        t.save(output/'execution-started.json',{})
        with self.assertRaises(FileExistsError):
            t.run_trial(output,call=lambda *_:self.fail('must not replay'))

    def test_invoke_retains_timeout_and_refuses_replay(self):
        log=self.root/'dispatch'
        with mock.patch.object(t.subprocess,'run',side_effect=subprocess.TimeoutExpired(['fake'],195,output=b'partial')):
            result=t.invoke(['fake'],log)
        self.assertIsNone(result['returncode'])
        self.assertEqual(result['stdout'],'partial')
        self.assertTrue(log.with_suffix('.result.json').exists())
        with self.assertRaises(FileExistsError):
            t.invoke(['fake'],log)

    def test_qualifies_independent_original_with_its_actual_closure(self):
        output,manifest=self.freeze()
        result,_=self.finish_slot(output,manifest)
        self.assertTrue(result['raw_correct'])
        self.assertTrue(result['qualified_positive'],result)

    def test_correct_guess_with_unrelated_recall_never_qualifies(self):
        output,manifest=self.freeze()
        result,_=self.finish_slot(output,manifest,evidence_name='S2-observation')
        self.assertTrue(result['raw_correct'])
        self.assertTrue(result['protocol_ok'])
        self.assertTrue(result['lineage_ok'])
        self.assertFalse(result['decisive_evidence'])
        self.assertFalse(result['qualified_positive'])

    def test_historical_action_reference_is_not_decisive_observation(self):
        output,manifest=self.freeze()
        result,_=self.finish_slot(output,manifest,evidence_name='S1-action')
        self.assertTrue(result['raw_correct'])
        self.assertFalse(result['qualified_positive'])

    def test_report_passed_label_does_not_override_actual_artifact(self):
        output,manifest=self.freeze()
        result,_=self.finish_slot(output,manifest,choice='MK7')
        self.assertFalse(result['raw_correct'])
        self.assertFalse(result['qualified_positive'])

    def test_predispatch_extra_attempt_disqualifies_even_if_handle_did_not_see_it(self):
        output,manifest=self.freeze()
        result,_=self.finish_slot(output,manifest,extra_attempt=True)
        self.assertTrue(result['raw_correct'])
        self.assertEqual(result['transcript_attempts'],2)
        self.assertFalse(result['protocol_ok'])
        self.assertFalse(result['qualified_positive'])

    def test_packet_must_contain_exact_observed_text_not_just_reference(self):
        output,manifest=self.freeze()
        _,report=self.finish_slot(output,manifest)
        conn=sqlite3.connect(output/manifest['slots'][0]['directory']/'arm.memory-prototype.sqlite')
        self.addCleanup(conn.close)
        support=manifest['support']['S1']
        packet={'records':[{'reference':support['reference'],'record':evidence('Synthesized summary says UP4.')} ]}
        self.assertFalse(t.evidence_qualified(conn,[packet],report['receipt'],support))

    def test_missing_and_withdrawn_controls_are_distinct(self):
        output,manifest=self.freeze()
        result=t.audit(output)
        for mode in ('lexical','dense'):
            self.assertEqual(result['totals'][mode]['planned'],8)
            self.assertEqual(result['totals'][mode]['positive_planned'],6)
            self.assertEqual(result['totals'][mode]['no_evidence'],{'missing_evidence':False,'withdrawn_evidence':False})
        self.assertEqual(len(result['slots']),16)

    def test_native_seed_fixture_emits_sse_tool_frames_usage_and_done(self):
        with t.authored_seed_server('MK7') as (endpoint,calls,errors):
            body=t.encode(dict(model=t.AUTHORED_MODEL,stream=True)).encode()
            request=urllib.request.Request(endpoint+'/v1/chat/completions?autoload=false',data=body,headers={'Content-Type':'application/json'})
            with urllib.request.urlopen(request) as response:
                self.assertEqual(response.headers['Content-Type'],'text/event-stream')
                data=response.read().decode()
            frames=[line[6:] for line in data.splitlines() if line.startswith('data: ')]
            self.assertEqual(frames[-1],'[DONE]')
            self.assertEqual(t.decode(frames[0])['choices'][0]['delta']['tool_calls'][0]['function']['name'],'fs_read')
            self.assertEqual(t.decode(frames[1])['choices'][0]['finish_reason'],'tool_calls')
            self.assertEqual(t.decode(frames[2])['usage']['completion_tokens'],100)
            self.assertEqual(len(calls),1)
            self.assertEqual(errors,[])

    def test_exact_optional_untrusted_envelope_and_bare_empty_packet(self):
        output,manifest=self.freeze()
        _,report=self.finish_slot(output,manifest)
        path=Path(report['transcript'])
        records=[t.decode(line) for line in path.read_text().splitlines()]
        original=records[1]['content'][0]['content']
        records[1]['content'][0]['content']=t.MEMORY_PREFIX+original+t.MEMORY_SUFFIX
        path.write_text(''.join(t.encode(r) for r in records))
        valid,packets,_=t.transcript_recall(path,self.fixture['cases'][0]['query'],self.fixture['goal'])
        self.assertTrue(valid);self.assertTrue(packets[0]['records'])
        records[1]['content'][0]['content']=t.encode({'memory':{'records':[]}})
        path.write_text(''.join(t.encode(r) for r in records))
        valid,packets,_=t.transcript_recall(path,self.fixture['cases'][0]['query'],self.fixture['goal'])
        self.assertTrue(valid);self.assertEqual(packets[0]['records'],[])
        for malformed in (t.MEMORY_PREFIX+original, '<untrusted-content>'+original+t.MEMORY_SUFFIX,original+t.MEMORY_SUFFIX):
            records[1]['content'][0]['content']=malformed
            path.write_text(''.join(t.encode(r) for r in records))
            with self.assertRaises(ValueError):
                t.transcript_recall(path,self.fixture['cases'][0]['query'],self.fixture['goal'])

    def test_missing_artifact_preserves_recall_lineage_and_returncode(self):
        output,manifest=self.freeze()
        _,report=self.finish_slot(output,manifest)
        Path(report['artifact']).unlink()
        result=t.audit_slot(output,manifest['slots'][0],self.fixture['cases'][0],manifest)
        self.assertFalse(result['raw_correct'])
        self.assertEqual(result['artifact_status'],'missing')
        self.assertEqual(result['returncode'],0)
        self.assertTrue(result['protocol_ok'])
        self.assertTrue(result['lineage_ok'])
        self.assertTrue(result['decisive_evidence'])

    def test_minimal_failure_report_does_not_hide_correct_artifact(self):
        output,manifest=self.freeze()
        self.finish_slot(output,manifest)
        path=output/manifest['slots'][0]['directory']/'native'/'report.json'
        path.write_text(t.encode({'status':'failed','error':'provider unavailable'}))
        result=t.audit_slot(output,manifest['slots'][0],self.fixture['cases'][0],manifest)
        self.assertTrue(result['raw_correct'])
        self.assertEqual(result['native_error'],'provider unavailable')
        self.assertEqual(result['native_status'],'failed')
        self.assertEqual(result['state'],'failed')
        self.assertFalse(result['qualified_positive'])

    def provider_rows(self):
        request=dict(think=False,think_budget=None,local_only=True,max_tokens=4096)
        wire=dict(model=self.runtime['model'],seed=self.fixture['cases'][0]['seed'],temperature=0.6,chat_template_kwargs={'enable_thinking':False})
        return [dict(event='request_started',call=1,request=request,wire_body=wire),
                dict(event='response',call=1,input_tokens=8,output_tokens=4),
                dict(event='request_started',call=2,request=request,wire_body=wire),
                dict(event='error',call=2,error='transport failed')]

    def test_partial_costs_are_not_misreported_as_session_totals(self):
        output,manifest=self.freeze()
        self.finish_slot(output,manifest)
        directory=output/manifest['slots'][0]['directory']
        (directory/'native'/'provider-audit.jsonl').write_text(''.join(t.encode(r) for r in self.provider_rows()))
        contract=t.provider_contract(directory,self.fixture['cases'][0],self.runtime)
        self.assertTrue(contract['valid'])
        self.assertEqual(contract['costs']['known_partial_input_tokens'],8)
        self.assertEqual(contract['costs']['known_partial_output_tokens'],4)
        self.assertIsNone(contract['costs']['input_tokens'])
        self.assertIsNone(contract['costs']['output_tokens'])
        self.assertEqual(contract['costs']['unaccounted_call_ids'],[2])
        result=t.audit(output)
        first=result['slots'][0]
        self.assertTrue(first['raw_correct'])
        self.assertTrue(first['protocol_ok'])
        self.assertTrue(first['qualified_positive'])
        self.assertFalse(first['matched_provider_contract'])
        self.assertFalse(first['matched_positive_success'])
        self.assertEqual(first['model_tokens']['known_partial_input_tokens'],8)

    def test_wrong_withdrawal_marker_refuses_freeze(self):
        def wrong(argv,log):
            (Path(argv[2])/'locator-S1.withdrawn').write_text('withdrawn')
            return {'returncode':0}
        with self.assertRaisesRegex(ValueError,'only exact S4 marker'):
            t.freeze(self.binary,self.binary,self.seed_root,self.root/'trial',self.runtime,call=wrong)

    def test_withdrawal_marker_is_rechecked_and_authorized_control_stays_active(self):
        output,manifest=self.freeze()
        case=self.fixture['cases'][-1]
        slot=manifest['slots'][-1]
        directory=output/slot['directory']
        self.assertTrue(t.withdrawal_state(output,directory,case,manifest))
        (directory/'authority'/'locator-S4.withdrawn').unlink()
        with self.assertRaisesRegex(ValueError,'withdrawal state'):
            t.withdrawal_state(output,directory,case,manifest)
        (directory/'authority'/'locator-S4.withdrawn').write_text('withdrawn')
        p04=next(s for s in manifest['slots'] if s['case']=='P04')
        (output/p04['directory']/'authority'/'locator-S4.withdrawn').write_text('withdrawn')
        with self.assertRaisesRegex(ValueError,'authorized S4 control'):
            t.withdrawal_state(output,directory,case,manifest)

    def test_invalid_own_provider_contract_disqualifies_without_hiding_observations(self):
        output,manifest=self.freeze()
        self.finish_slot(output,manifest)
        directory=output/manifest['slots'][0]['directory']
        rows=self.provider_rows()
        rows[0]['wire_body']['temperature']=0.9
        (directory/'native'/'provider-audit.jsonl').write_text(''.join(t.encode(r) for r in rows))
        result=t.audit(output)['slots'][0]
        self.assertFalse(result['provider_contract_ok'])
        self.assertFalse(result['qualified_positive'])
        self.assertTrue(result['raw_correct'])
        self.assertTrue(result['protocol_ok'])
        self.assertTrue(result['decisive_evidence'])
        self.assertEqual(result['model_tokens']['known_partial_input_tokens'],8)

    def test_invalid_mate_does_not_disqualify_valid_arm(self):
        output,manifest=self.freeze()
        for index in (0,1):
            self.finish_slot(output,manifest,slot_index=index)
            directory=output/manifest['slots'][index]['directory']
            rows=self.provider_rows()
            if index==1:
                rows[0]['wire_body']['temperature']=0.9
            (directory/'native'/'provider-audit.jsonl').write_text(''.join(t.encode(r) for r in rows))
        result=t.audit(output)
        good,bad=result['slots'][:2]
        self.assertTrue(good['qualified_positive'])
        self.assertFalse(bad['qualified_positive'])
        self.assertFalse(good['matched_positive_success'])
        self.assertFalse(bad['matched_positive_success'])
        self.assertEqual(result['totals']['lexical']['qualified_positive'],1)
        self.assertEqual(result['totals']['dense']['qualified_positive'],0)

    def test_missing_withdrawal_transcript_reports_unknown_delivery(self):
        output,manifest=self.freeze()
        slot=manifest['slots'][-1]
        directory=output/slot['directory']
        (directory/'native').mkdir()
        t.save(directory/'dispatch.started.json',{})
        t.save(directory/'dispatch.result.json',dict(returncode=1,elapsed_ms=1))
        t.save(directory/'native'/'report.json',dict(status='failed',error='provider unavailable'))
        result=t.audit_slot(output,slot,self.fixture['cases'][-1],manifest)
        self.assertIsNone(result['withdrawn_not_delivered'])
        self.assertFalse(result['qualified_no_evidence'])
        self.assertEqual(result['state'],'failed')

    def test_optional_confirmed_goal_has_equal_credit_and_other_shapes_fail(self):
        output,manifest=self.freeze()
        baseline,report=self.finish_slot(output,manifest)
        path=Path(report['transcript'])
        records=[t.decode(line) for line in path.read_text().splitlines()]
        exact={'memory':{'query':self.fixture['cases'][0]['query']}}
        variants=[(exact,True),(dict(exact,serves=self.fixture['goal']),True),
                  (dict(exact,serves='task:other'),False),(dict(exact,serves=None),False),
                  (dict(exact,extra=True),False),(dict(exact,serves=self.fixture['goal'],extra=True),False),
                  ({'memory':{'query':self.fixture['cases'][0]['query'],'bindings':[]}},False)]
        for tool_input,expected in variants:
            with self.subTest(tool_input=tool_input):
                records[0]['content'][0]['input']=tool_input
                path.write_text(''.join(t.encode(r) for r in records))
                result=t.audit_slot(output,manifest['slots'][0],self.fixture['cases'][0],manifest)
                self.assertEqual(result['protocol_ok'],expected)
                self.assertEqual(result['qualified_positive'],expected)
                self.assertEqual(result['raw_correct'],baseline['raw_correct'])


if __name__ == '__main__':
    unittest.main()
