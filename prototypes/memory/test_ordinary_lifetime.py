"""Authored, offline regression tests for the ordinary lifetime benchmark."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def implementation(test):
    path = HERE / 'ordinary_lifetime.py'
    test.assertTrue(path.exists(), 'ordinary lifetime implementation is missing')
    spec = importlib.util.spec_from_file_location('ordinary_lifetime', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class LifetimeTests(unittest.TestCase):
    def setUp(self):
        self.m = implementation(self)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.binary = self.root / 'mecha'
        self.binary.write_text('#!/bin/sh\nexit 0\n')
        self.binary.chmod(0o700)
        self.resident = self.root / 'resident.json'
        self.resident.write_text(json.dumps({'models': {'data': [{'id': 'fixture-model', 'status': {'value': 'loaded'}}]}, 'props': {'model_alias': 'fixture-model'}}))

    def freeze(self, seeds=(17,)):
        return self.m.freeze(self.root / 'trial', self.binary, 'http://127.0.0.1:19999', 'fixture-model', list(seeds), self.resident)

    def test_freeze_has_fixed_chronology_and_separate_persistent_arms(self):
        manifest = self.freeze((17, 23))
        self.assertEqual(len(manifest['slots']), 46)
        self.assertEqual(manifest['planned_decisions'], 24)
        for arm in self.m.ARMS:
            slots = [s for s in manifest['slots'] if s['arm'] == arm and s['seed'] == 17]
            expected = ['O1', 'O2', 'induce', 'D1', 'D2', 'O3', 'ruminate', 'D3', 'D4'] if arm == 'adaptive' else ['O1', 'O2', 'D1', 'D2', 'O3', 'D3', 'D4']
            self.assertEqual([s['episode'] for s in slots], expected)
        self.assertEqual(len({s['home'] for s in manifest['slots']}), 6)
        self.assertEqual(manifest['cohort_role'], 'development')
        self.assertEqual(manifest['independent_families'], 1)
        with self.assertRaises((FileExistsError, self.m.LifetimeError)):
            self.freeze()

    def test_current_materialization_has_no_future_or_gold_and_matches_arms(self):
        manifest = self.freeze()
        contents = []
        for arm in self.m.ARMS:
            slot = next(s for s in manifest['slots'] if s['arm'] == arm and s['episode'] == 'O1')
            workspace = self.m.materialize(self.root / 'trial', manifest, slot)
            data = {p.name: p.read_bytes() for p in workspace.iterdir()}
            self.assertEqual(set(data), {'observation.json'})
            self.assertNotIn(b'identifiers', b''.join(data.values()))
            self.assertNotIn(b'PRIVATE_GOLD', b''.join(data.values()))
            contents.append(data)
        self.assertTrue(all(data == contents[0] for data in contents))

    def test_artifact_missing_unsafe_duplicate_malformed_and_contract(self):
        path = self.root / 'decision.json'
        self.assertEqual(self.m.artifact(path)['status'], 'missing')
        path.symlink_to(self.binary)
        self.assertEqual(self.m.artifact(path)['status'], 'unsafe_path')
        path.unlink()
        for data, expected in [(b'{', 'malformed'), (b'{"status":"selected","status":"abstained"}', 'malformed'), (b'\xff', 'nonutf8'), (b'{}', 'invalid_contract')]:
            path.write_bytes(data)
            self.assertEqual(self.m.artifact(path)['status'], expected)
        path.write_text(json.dumps({'status': 'selected', 'route': 'compact', 'reason': 'observed', 'citations': []}))
        self.assertEqual(self.m.artifact(path)['status'], 'valid')
        path.write_bytes(b'x' * 16385)
        self.assertEqual(self.m.artifact(path)['status'], 'oversized')

    def test_missing_usage_is_unknown_and_all_calls_count(self):
        result = self.m.usage_totals([{'forwarded': True, 'usage': {'input_tokens': 20, 'output_tokens': 5}}, {'forwarded': True, 'usage': None}, {'forwarded': False, 'usage': None}])
        self.assertEqual(result['forwarded'], 2)
        self.assertEqual(result['attempted'], 3)
        self.assertEqual(result['known_output_tokens'], 5)
        self.assertEqual(result['missing_usage_calls'], 1)
        self.assertIsNone(result['output_tokens'])
        self.assertFalse(result['usage_complete'])

    def test_quote_must_be_exact_and_in_delivered_reference_field(self):
        citation = {'reference': {'object': 'o', 'revision': 'r'}, 'field': '/text', 'quote': 'complete took 6 h'}
        evidence = {'reference': citation['reference'], 'field': '/text', 'text': 'complete took 6 h', 'episode': 'O1', 'role': 'original', 'delivery': 'wire'}
        self.assertFalse(self.m.check_citation(citation, [] )['valid'])
        self.assertTrue(self.m.check_citation(citation, [evidence])['valid'])
        self.assertFalse(self.m.check_citation(dict(citation, field='/other'), [evidence])['valid'])
        self.assertFalse(self.m.check_citation(dict(citation, quote='complete took 2 h'), [evidence])['valid'])

    def test_fixed_denominator_phase_pairs_and_capture_independence(self):
        manifest = self.freeze()
        slot = next(s for s in manifest['slots'] if s['arm'] == 'adaptive' and s['episode'] == 'D1')
        slotdir = self.root / 'trial' / 'slots' / slot['id']
        slotdir.mkdir(parents=True)
        (slotdir / 'decision.json').write_text(json.dumps({'status': 'selected', 'route': 'complete', 'reason': 'evidence', 'citations': []}))
        self.m.append_event(self.root / 'trial', {'slot': slot['id'], 'state': 'started'})
        self.m.append_event(self.root / 'trial', {'slot': slot['id'], 'state': 'terminal', 'execution': 'failed', 'capture_status': 'unavailable', 'artifact': 'decision.json', 'artifact_observation': self.m.artifact(slotdir/'decision.json'), 'calls': [{'forwarded': True, 'usage': None}], 'delivered': []})
        card = self.m.score(self.root / 'trial')
        self.assertEqual(card['arms']['adaptive']['decision_denominator'], 4)
        self.assertEqual(card['arms']['adaptive']['decision_correct'], 1)
        self.assertEqual(card['arms']['adaptive']['phases']['initial']['accuracy'], .5)
        self.assertEqual(card['arms']['adaptive']['phases']['revised']['accuracy'], 0)
        self.assertEqual(card['pairs']['adaptive_vs_frozen']['raw_diagnostic']['wins'], 1)
        self.assertEqual(card['pairs']['adaptive_vs_frozen']['raw_diagnostic']['planned_pairs'], 4)
        self.assertEqual(card['pairs']['adaptive_vs_frozen']['raw_diagnostic']['completed_pairs'], 0)
        self.assertEqual(card['semantic_entailment'], 'not_evaluated')
        self.assertEqual(card['costs']['missing_usage_calls'], 1)

    def test_ledger_started_is_spent_and_tail_corruption_fails_closed(self):
        manifest = self.freeze()
        slot = manifest['slots'][0]
        self.m.append_event(self.root / 'trial', {'slot': slot['id'], 'state': 'started'})
        states = self.m.slot_states(self.root / 'trial', manifest)
        self.assertEqual(states[slot['id']], 'indeterminate')
        later = next(s for s in manifest['slots'] if s['arm'] == slot['arm'] and s['seed'] == slot['seed'] and s['episode'] == 'D1')
        self.assertEqual(states[later['id']], 'not_run_due_to_interruption')
        other = next(s for s in manifest['slots'] if s['arm'] != slot['arm'])
        self.assertEqual(states[other['id']], 'pending')
        with (self.root / 'trial' / 'events.jsonl').open('ab') as f:
            f.write(b'{"partial"')
        with self.assertRaises(self.m.LifetimeError):
            self.m.slot_states(self.root / 'trial', manifest)

    def test_artifact_surrogate_and_deep_json_do_not_abort_score(self):
        path=self.root/'decision.json'
        path.write_bytes(b'{"status":"selected","route":"compact","reason":"\\ud800","citations":[]}')
        self.assertIn(self.m.artifact(path)['status'],('malformed','invalid_contract'))
        path.write_text('['*1200+'0'+']'*1200)
        self.assertEqual(self.m.artifact(path)['status'],'malformed')

    def test_interrupted_wire_cost_is_not_zero_or_complete(self):
        manifest=self.freeze()
        slot=manifest['slots'][0]
        wire=self.root/'trial'/'slots'/slot['id']/'wire'
        wire.mkdir(parents=True)
        (wire/'001.json').write_text(json.dumps({'forwarded':True,'usage':{'input_tokens':20,'output_tokens':4}}))
        self.m.append_event(self.root/'trial',{'slot':slot['id'],'state':'started'})
        card=self.m.score(self.root/'trial')
        self.assertEqual(card['costs']['known_output_tokens'],4)
        self.assertEqual(card['costs']['interrupted_slots'],1)
        self.assertFalse(card['costs']['usage_complete'])
        self.assertIsNone(card['costs']['elapsed_ms'])
        self.assertEqual(card['arms']['off']['costs']['known_output_tokens'],4)
        self.assertEqual(card['costs_by_kind']['observation']['interrupted_slots'],1)

    def test_episodes_are_interleaved_and_protocol_violations_do_not_qualify(self):
        manifest=self.freeze()
        self.assertEqual([s['episode'] for s in manifest['slots'][:3]],['O1']*3)
        slot=next(s for s in manifest['slots'] if s['arm']=='adaptive' and s['episode']=='D1')
        out=self.root/'trial'/'slots'/slot['id'];out.mkdir(parents=True)
        (out/'decision.json').write_text(json.dumps({'status':'selected','route':'complete','reason':'x','citations':[]}))
        self.m.append_event(self.root/'trial',{'slot':slot['id'],'state':'started'})
        self.m.append_event(self.root/'trial',{'slot':slot['id'],'state':'terminal','execution':'completed','protocol_status':'violated','protocol_violations':['response alias mismatch'],'artifact_observation':self.m.artifact(out/'decision.json'),'calls':[],'elapsed_ms':10})
        card=self.m.score(self.root/'trial')
        row=next(r for r in card['rows'] if r['id']==slot['id'])
        self.assertEqual(row['decision_correct'],1)
        self.assertEqual(row['qualified_correct'],0)
        self.assertEqual(card['arms']['adaptive']['qualified_correct'],0)


    def test_delivery_requires_forwarded_exact_tool_result_and_original_field(self):
        self.assertTrue(callable(getattr(self.m,'wire_evidence',None)), 'wire evidence extraction missing')
        wire=self.root/'wire';wire.mkdir()
        reference={'object':'o','revision':'r'}
        descriptor={'reference':reference,'field':'/content/0/content','text':'route observation','episode':'O1'}
        packet={'memory':{'records':[{'reference':reference,'record':{'kind':'evidence','data':{'text':'route observation'}}}]}}
        content=self.m.MEMORY_PREFIX+json.dumps(packet)+'\n</untrusted-content>'
        messages=[{'role':'assistant','tool_calls':[{'id':'recall','function':{'name':'goal_context','arguments':'{"memory":{"query":"report-release-v1"}}'}}]}, {'role':'tool','tool_call_id':'recall','content':content}]
        (wire/'001.request').write_text(json.dumps({'messages':messages}))
        self.assertEqual(self.m.wire_evidence(wire,[],[descriptor])['delivered'],[])
        (wire/'001.forwarded').write_text(json.dumps({'messages':messages}))
        native=[{'kind':'call','id':'recall','name':'goal_context','input':{'memory':{'query':'report-release-v1'}}},{'kind':'result','id':'recall','content':content,'is_error':False}]
        result=self.m.wire_evidence(wire,[{'attempt':1,'forwarded':True,'acknowledged':True,'upstream_complete':True}],[descriptor],native)
        self.assertEqual(len(result['delivered']),1)
        self.assertEqual(result['delivered'][0]['field'],descriptor['field'])
        self.assertEqual(result['delivered'][0]['episode'],'O1')
        messages[-1]['role']='assistant'
        (wire/'001.forwarded').write_text(json.dumps({'messages':messages}))
        self.assertEqual(self.m.wire_evidence(wire,[{'attempt':1,'forwarded':True,'acknowledged':True,'upstream_complete':True}],[descriptor],native)['delivered'],[])

    def test_observation_selection_requires_unique_authenticated_read(self):
        self.assertTrue(callable(getattr(self.m,'select_observation',None)), 'observation selector missing')
        descriptor={'source':{'run_id':'r','original_index':2},'reference':{'object':'o','revision':'v'},'kind':'ToolResult','field':'/content/0/content','text':'exact readout','tool':{'call_id':'x','tool_name':'fs_read','input_status':'Complete','input':{'path':'observation.json'}},'receipt_status':'Succeeded'}
        self.assertEqual(self.m.select_observation([descriptor],'exact readout'),descriptor)
        for sources in ([],[descriptor,descriptor],[dict(descriptor,receipt_status='Failed')],[dict(descriptor,text='other')]):
            with self.assertRaises(self.m.LifetimeError):
                self.m.select_observation(sources,'exact readout')

    def test_raw_host_originals_are_citeable_without_independent_episode_credit(self):
        wire=self.root/'raw-wire';wire.mkdir()
        selected={'object':'selected','revision':'v'};copy={'object':'receipt-copy','revision':'v'}
        prompt={'object':'owner-prompt','revision':'v'}
        def hit(ref,source_id,text='route observation',connector='host-session-field-v1'):
            return {'reference':ref,'record':{'kind':'evidence','data':{'source':{'connector':connector,'account':'synthetic','id':source_id},'text':text}}}
        records=[hit(selected,'20261110T142838-abd908f0:6:/content/0/content'),hit(copy,'20261110T142838-abd908f0:10:/content/0/content'),hit(prompt,'20261110T142838-abd908f0:1:/content/0/text')]
        records.extend(hit({'object':'bad'+str(i),'revision':'v'},sid) for i,sid in enumerate(['run:0:/content/0/content','run:2:/data/text','run:2:/content/-1/content','run:2:/content/0/input','run:2:/content/0/content:extra']))
        records.append(hit({'object':'foreign','revision':'v'},'run:2:/content/0/content',connector='arbitrary'))
        content=self.m.MEMORY_PREFIX+json.dumps({'memory':{'records':records}})+'\n</untrusted-content>'
        messages=[{'role':'assistant','tool_calls':[{'id':'recall','function':{'name':'goal_context','arguments':'{"memory":{"query":"report-release-v1"}}'}}]}, {'role':'tool','tool_call_id':'recall','content':content}]
        (wire/'001.forwarded').write_text(json.dumps({'messages':messages}))
        native=[{'kind':'call','id':'recall','name':'goal_context','input':{'memory':{'query':'report-release-v1'}}},{'kind':'result','id':'recall','content':content,'is_error':False}]
        sources=[{'reference':selected,'field':'/content/0/content','text':'route observation','episode':'O2'}]
        delivered=self.m.wire_evidence(wire,[{'attempt':1,'forwarded':True,'acknowledged':True,'upstream_complete':True}],sources,native)['delivered']
        self.assertEqual(len(delivered),3)
        for ref,field,episodes in [(selected,'/content/0/content',['O2']),(copy,'/content/0/content',[]),(prompt,'/content/0/text',[])]:
            citation={'reference':ref,'field':field,'quote':'route observation'}
            checked=self.m.check_citation(citation,delivered)
            self.assertTrue(checked['valid'])
            self.assertEqual(checked['episodes'],episodes)
            self.assertFalse(self.m.check_citation(dict(citation,field='/data/text'),delivered)['valid'])


    def test_partial_dispatch_cost_remains_unknown(self):
        result=self.m.usage_totals([{'forwarded':False,'dispatch_uncertain':True,'usage':None}])
        self.assertFalse(result['usage_complete'])
        self.assertEqual(result['uncertain_dispatch_calls'],1)
        self.assertIsNone(result['output_tokens'])


    def test_alias_output_root_materializes_canonical_workspace(self):
        alias=self.root/'alias';alias.symlink_to(self.root,target_is_directory=True)
        manifest=self.m.freeze(alias/'trial',self.binary,'http://127.0.0.1:19999','fixture-model',[17],self.resident)
        workspace=self.m.materialize(alias/'trial',manifest,manifest['slots'][0])
        self.assertEqual(workspace,workspace.resolve())


    def test_pending_and_tampered_artifacts_cannot_score(self):
        manifest=self.freeze();trial=self.root/'trial'
        slot=next(s for s in manifest['slots'] if s['arm']=='off' and s['episode']=='D1')
        path=trial/'slots'/slot['id']/'decision.json';path.parent.mkdir(parents=True)
        correct={'status':'selected','route':'complete','reason':'fixture','citations':[]}
        path.write_text(json.dumps(correct))
        def row(): return next(r for r in self.m.score(trial)['rows'] if r['id']==slot['id'])
        self.assertEqual(row()['decision_correct'],0)
        self.assertEqual(row()['artifact_status'],'unbound')
        self.m.append_event(trial,{'slot':slot['id'],'state':'started'})
        binding=self.m.artifact(path)
        self.m.append_event(trial,{'slot':slot['id'],'state':'terminal','execution':'completed','protocol_status':'qualified','artifact_observation':binding})
        self.assertEqual(row()['qualified_correct'],1)
        path.write_text(json.dumps(dict(correct,reason='changed')))
        self.assertEqual(row()['artifact_status'],'tampered')
        self.assertEqual(row()['decision_correct'],0)
        path.unlink()
        self.assertEqual(row()['artifact_status'],'missing_snapshot')
        self.assertEqual(row()['decision_correct'],0)

    def test_freeze_uses_one_buffer_and_execution_pins_verified_fixture(self):
        from unittest.mock import patch
        fixture=self.root/'source.json';original=self.m.FIXTURE.read_bytes();fixture.write_bytes(original)
        changed=json.loads(original);changed['episodes'][0]['input']['batch']='MUTATED'
        original_bounded=self.m.bounded
        reads=[]
        def read_then_replace(path,*args):
            value=original_bounded(path,*args)
            if Path(path)==fixture:
                reads.append(path);fixture.write_text(json.dumps(changed))
            return value
        with patch.object(self.m,'bounded',read_then_replace):
            manifest=self.m.freeze(self.root/'trial',self.binary,'http://127.0.0.1:19999','fixture-model',[17],self.resident,fixture=fixture)
        self.assertEqual(len(reads),1)
        self.assertEqual((self.root/'trial'/'fixture.json').read_bytes(),original)
        pinned=self.m.load_manifest(self.root/'trial',execution=True)
        (self.root/'trial'/'fixture.json').write_text(json.dumps(changed))
        workspace=self.m.materialize(self.root/'trial',pinned,pinned['slots'][0])
        self.assertEqual(json.loads((workspace/'observation.json').read_text())['batch'],'A1')

    def test_task_inputs_cannot_be_truncated_by_file_tool_style_open(self):
        manifest=self.freeze();workspace=self.m.materialize(self.root/'trial',manifest,manifest['slots'][0])
        with self.assertRaises(PermissionError):
            with (workspace/'observation.json').open('w') as stream: stream.write('replacement')


    def test_recall_requires_successful_native_result_acknowledged_before_write(self):
        wire=self.root/'wire';wire.mkdir()
        content=json.dumps({'memory':{'records':[]}})
        invocation={'kind':'call','id':'recall','name':'goal_context','input':{'memory':{'query':'report-release-v1'}}}
        result={'kind':'result','id':'recall','content':content,'is_error':False}
        write={'kind':'call','id':'write','name':'fs_write','input':{'path':'decision.json'}}
        messages=[{'role':'assistant','tool_calls':[{'id':'recall','function':{'name':'goal_context','arguments':json.dumps(invocation['input'])}}]}, {'role':'tool','tool_call_id':'recall','content':content}]
        (wire/'001.forwarded').write_text(json.dumps({'messages':messages}))
        call={'attempt':1,'forwarded':True,'acknowledged':True,'upstream_complete':True}
        def check(native,metadata=call): return self.m.wire_evidence(wire,[metadata],[],native)['recall_status']
        self.assertEqual(check([invocation,result,write]),'successful_empty')
        self.assertEqual(check([invocation,write]),'missing_result')
        self.assertEqual(check([invocation,dict(result,is_error=True),write]),'error')
        self.assertEqual(check([write,invocation,result]),'after_write')
        self.assertEqual(check([invocation,result,invocation,result,write]),'duplicate_attempt')
        self.assertEqual(check([invocation,result,write],dict(call,acknowledged=False)),'unacknowledged')
        messages.append({'role':'assistant','tool_calls':[{'id':'write','function':{'name':'fs_write','arguments':'{"path":"decision.json"}'}}]})
        (wire/'001.forwarded').write_text(json.dumps({'messages':messages}))
        self.assertEqual(check([invocation,result,write]),'after_write')

    def test_changed_input_pair_is_raw_diagnostic_not_qualified_win(self):
        manifest=self.freeze();trial=self.root/'trial'
        slot=next(s for s in manifest['slots'] if s['arm']=='adaptive' and s['episode']=='D1')
        path=trial/'slots'/slot['id']/'decision.json';path.parent.mkdir(parents=True)
        path.write_text(json.dumps({'status':'selected','route':'complete','reason':'fixture','citations':[]}))
        self.m.append_event(trial,{'slot':slot['id'],'state':'started'})
        self.m.append_event(trial,{'slot':slot['id'],'state':'terminal','execution':'failed','protocol_status':'violated','input_unchanged':False,'halt_arm':True,'artifact_observation':self.m.artifact(path)})
        pair=self.m.score(trial)['pairs']['adaptive_vs_frozen']
        self.assertEqual(pair['raw_diagnostic']['wins'],1)
        self.assertEqual(pair['qualified']['wins'],0)
        self.assertEqual(pair['qualified']['planned_pairs'],4)
        self.assertEqual(pair['qualified']['qualified_pairs'],0)


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.m = implementation(self)
        proxy_path = HERE / 'ordinary_lifetime_transport.py'
        self.assertTrue(proxy_path.exists(), 'bounded lifetime transport is missing')
        spec = importlib.util.spec_from_file_location('lifetime_transport_test', proxy_path)
        self.p = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.p)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def server(self, responder):
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        import threading
        calls=[]
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self):
                calls.append((self.path,None))
                self.respond(self.path,b'')
            def do_POST(self):
                body=self.rfile.read(int(self.headers['Content-Length']))
                calls.append((self.path,json.loads(body)))
                self.respond(self.path,body)
            def respond(self,path,body):
                status,data,content=responder(path,body)
                self.send_response(status)
                self.send_header('Content-Type',content)
                self.send_header('Content-Length',str(len(data)))
                self.end_headers()
                try: self.wfile.write(data)
                except OSError: pass
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server.server_address,calls

    def request(self,proxy,max_tokens=8192):
        from http.client import HTTPConnection
        conn=HTTPConnection('127.0.0.1',proxy.port,timeout=5)
        body=json.dumps({'model':'fixture-model','messages':[],'max_tokens':max_tokens,'stream':False}).encode()
        conn.request('POST','/v1/chat/completions',body,{'Authorization':'Bearer token','Content-Type':'application/json'})
        response=conn.getresponse()
        result=response.status,response.read()
        conn.close()
        return result

    def test_discovery_preserves_router_and_validates_targeted_query(self):
        from http.client import HTTPConnection
        from urllib.parse import parse_qs,urlsplit
        def responder(path,body):
            parsed=urlsplit(path);query=parse_qs(parsed.query)
            value={'role':'router'} if parsed.path=='/props' and 'model' not in query else {'model_alias':'fixture-model'}
            return 200,json.dumps(value).encode(),'application/json'
        address,calls=self.server(responder)
        def get(proxy,path):
            connection=HTTPConnection('127.0.0.1',proxy.port,timeout=5)
            connection.request('GET',path)
            response=connection.getresponse();result=response.status,json.loads(response.read())
            connection.close();return result
        with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
            accepted=[('/props',{'role':'router'}),('/props?autoload=false',{'role':'router'}),('/props?model=fixture-model',{'model_alias':'fixture-model'}),('/props?autoload=false&model=fixture-model',{'model_alias':'fixture-model'})]
            (self.root/'accepted').mkdir()
            with proxy.phase(self.root/'accepted',5,1,8192):
                for path,value in accepted:
                    self.assertEqual(get(proxy,path),(200,value))
            self.assertEqual([p for p,_ in calls],['/props?autoload=false','/props?autoload=false','/props?autoload=false&model=fixture-model','/props?autoload=false&model=fixture-model'])
            rejected=['/props?model=foreign','/props?model=fixture-model&model=fixture-model','/props?autoload=false&autoload=false','/props?autoload=true','/props?other=value','/props?model=','/props?bad','/models?model=fixture-model','http://other/props']
            for index,path in enumerate(rejected):
                (self.root/f'rejected-{index}').mkdir()
                with proxy.phase(self.root/f'rejected-{index}',5,1,8192) as phase:
                    self.assertEqual(get(proxy,path)[0],502,path)
                self.assertFalse(phase.discovery[0]['forwarded'],path)
            self.assertEqual(len(calls),4)

    def test_direct_noautoload_alias_and_cumulative_cap(self):
        def responder(path,body):
            return 200,json.dumps({'model':'fixture-model','choices':[{'message':{'content':'ok'}}],'usage':{'prompt_tokens':20,'completion_tokens':5000}}).encode(),'application/json'
        address,calls=self.server(responder)
        with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
            with proxy.phase(self.root,5,8,8192) as phase:
                self.assertEqual(self.request(proxy)[0],200)
                # Upstream is constrained to 3192; its false 5000 usage is a violation.
                self.assertEqual(self.request(proxy)[0],502)
            self.assertEqual(calls[0][0],'/v1/chat/completions?autoload=false')
            self.assertEqual(calls[1][1]['max_tokens'],3192)
            self.assertEqual(phase.active,0)
            self.assertEqual(phase.calls[1]['usage']['output_tokens'],5000)
            self.assertTrue(phase.calls[1]['error'])

    def test_response_alias_mismatch_recorded_and_refused(self):
        address,calls=self.server(lambda p,b:(200,b'{"model":"other","choices":[],"usage":null}','application/json'))
        with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
            with proxy.phase(self.root,5,1,8192) as phase:
                self.assertEqual(self.request(proxy)[0],502)
            self.assertEqual(len(phase.calls),1)
            self.assertIn('alias',phase.calls[0]['error'])
            self.assertTrue((self.root/'001.response').exists())

    def test_unknown_usage_preserved_and_not_rejected(self):
        address,calls=self.server(lambda p,b:(200,b'{"model":"fixture-model","choices":[],"usage":null}','application/json'))
        with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
            with proxy.phase(self.root,5,1,8192) as phase:
                self.assertEqual(self.request(proxy)[0],200)
            self.assertIsNone(phase.calls[0]['usage'])
            self.assertNotIn('error',phase.calls[0])

    def test_endpoint_refuses_hostname_remote_credentials_and_bad_path(self):
        for value in ('http://localhost:8080','http://192.168.1.1','https://127.0.0.1','http://a:b@127.0.0.1','http://127.0.0.1/redirect','http://127.0.0.1?x=y'):
            with self.assertRaises(self.p.TransportError):
                self.p.RecordingProxy(value,'fixture-model','token')

    def test_unacknowledged_post_blocks_all_later_phases(self):
        import threading
        release=threading.Event();active=threading.Event()
        def responder(path,body):
            active.set();release.wait(3)
            return 200,b'{"model":"fixture-model","choices":[]}','application/json'
        address,calls=self.server(responder)
        self.addCleanup(release.set)
        try:
            with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
                with self.assertRaisesRegex(RuntimeError,'upstream'):
                    with proxy.phase(self.root,.1,1,8192) as phase:
                        self.assertEqual(self.request(proxy)[0],502)
                self.assertTrue(active.is_set())
                self.assertEqual(phase.active,0)
                second=self.root/'second';second.mkdir()
                with self.assertRaises(RuntimeError):
                    with proxy.phase(second,1,1,8192): self.request(proxy)
                self.assertEqual(len(calls),1)
        finally:
            release.set()

    def test_unknown_output_reserves_cap_before_second_request(self):
        for usage in (None,{'prompt_tokens':20}):
            with self.subTest(usage=usage):
                directory=self.root/str(len(list(self.root.iterdir())));directory.mkdir()
                response=json.dumps({'model':'fixture-model','choices':[],'usage':usage}).encode()
                address,calls=self.server(lambda p,b:(200,response,'application/json'))
                with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy:
                    with proxy.phase(directory,5,8,8192) as phase:
                        self.assertEqual(self.request(proxy)[0],200)
                        self.assertEqual(self.request(proxy)[0],502)
                    self.assertEqual(len(calls),1)
                    self.assertFalse(self.m.usage_totals(phase.calls)['usage_complete'])

    def test_failed_connect_is_attempted_not_forwarded(self):
        import socket
        with socket.socket() as probe:
            probe.bind(('127.0.0.1',0))
            port=probe.getsockname()[1]
        with self.p.RecordingProxy(f'http://127.0.0.1:{port}','fixture-model','token') as proxy:
            with proxy.phase(self.root,1,1,8192) as phase:
                self.assertEqual(self.request(proxy)[0],502)
            self.assertFalse(phase.calls[0]['forwarded'])
            self.assertEqual(len(phase.calls),1)

    def test_accepted_handler_stays_bound_until_teardown(self):
        import threading,time
        from unittest.mock import patch
        parked=threading.Event();release=threading.Event();closing=[]
        address,calls=self.server(lambda p,b:(200,b'{"model":"fixture-model","choices":[]}','application/json'))
        original=self.p.Handler.dispatch
        def delayed(handler,inference):
            parked.set();release.wait(3);return original(handler,inference)
        with self.p.RecordingProxy(f'http://{address[0]}:{address[1]}','fixture-model','token') as proxy, patch.object(self.p.Handler,'dispatch',delayed):
            context=proxy.phase(self.root,5,1,8192);phase=context.__enter__()
            def client():
                try: self.request(proxy)
                except OSError: pass
            client_thread=threading.Thread(target=client);client_thread.start()
            self.assertTrue(parked.wait(2))
            def close():
                try: context.__exit__(None,None,None)
                except Exception as error: closing.append(error)
            closer=threading.Thread(target=close);closer.start()
            try:
                deadline=time.monotonic()+1
                while not phase.closed and time.monotonic()<deadline: time.sleep(.01)
                closer.join(.1)
                self.assertTrue(closer.is_alive(),'phase teardown forgot an accepted handler')
                second=self.root/'second';second.mkdir()
                with self.assertRaises(RuntimeError):
                    with proxy.phase(second,1,1,8192): pass
            finally:
                release.set();client_thread.join(2);closer.join(2)
            self.assertEqual(closing,[])
            self.assertEqual(calls,[])
            self.assertEqual(phase.accepted,0)
            with proxy.phase(second,1,1,8192) as next_phase:
                self.assertEqual(next_phase.calls,[])
            self.assertEqual(calls,[])

class AdapterTests(unittest.TestCase):
    def setUp(self):
        TransportTests.setUp(self)
        self.assertTrue(callable(getattr(self.m,'run',None)), 'ordinary CLI adapter is missing')

    def server(self,responder=None):
        from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
        import threading
        calls=[]
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self):
                from urllib.parse import parse_qs,urlsplit
                query=parse_qs(urlsplit(self.path).query)
                value={'data':[{'id':'fixture-model','status':{'value':'loaded'}}]} if self.path.startswith('/models') else ({'model_alias':'fixture-model','default_generation_settings':{'n_ctx':32768}} if 'model' in query else {'role':'router'})
                data=json.dumps(value).encode()
                self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
            def do_POST(self):
                body=self.rfile.read(int(self.headers['Content-Length']))
                calls.append((self.path,json.loads(body)))
                data=json.dumps({'model':'fixture-model','choices':[{'message':{'content':'fixture'}}],'usage':None}).encode()
                if responder is not None: data=responder(self.path,body)
                self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers()
                try: self.wfile.write(data)
                except OSError: pass
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        self.addCleanup(server.server_close);self.addCleanup(server.shutdown)
        return server.server_address,calls

    def test_restart_with_indeterminate_slot_dispatches_nothing(self):
        import shutil
        address,calls=self.server()
        binary=self.root/'fake-mecha';shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial'
        manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        self.m.append_event(trial,{'slot':manifest['slots'][0]['id'],'state':'started'})
        with self.assertRaises(RuntimeError):
            self.m.run(trial)
        self.assertEqual(calls,[])

    def test_timeout_kills_process_group_and_preserves_outputs(self):
        import sys,time
        import ordinary_lifetime_adapter as adapter
        marker=self.root/'orphan-output'
        code='import subprocess,sys,time; subprocess.Popen([sys.executable,"-c",'+repr('import time;from pathlib import Path;time.sleep(.5);Path('+repr(str(marker))+').write_text("orphan")')+']); print("started",flush=True); time.sleep(5)'
        result=adapter.invoke([sys.executable,'-c',code],self.root,{'PATH':'/usr/bin:/bin'},self.root/'process',.1)
        self.assertTrue(result['timed_out'])
        self.assertIn(b'started',(self.root/'process.stdout').read_bytes())
        time.sleep(.6)
        self.assertFalse(marker.exists())

    def test_authored_provider_smoke_entrypoint_and_required_binary(self):
        import shutil
        from ordinary_lifetime_smoke import qualify
        binary=self.root/'fake-mecha';shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary);binary.chmod(0o700)
        summary=qualify(binary,self.root/'smoke',replace_inputs=False)
        self.assertEqual(summary['status'],'passed')
        self.assertEqual(summary['executed_slots'],9)
        self.assertEqual(summary['learning_posts_checked'],1)
        self.assertEqual(summary['model_capability'],'not_evaluated')
        with self.assertRaises(FileNotFoundError):
            qualify(self.root/'absent-binary',self.root/'missing-smoke')

    def test_memory_database_name_satisfies_native_opt_in_contract(self):
        import ordinary_lifetime_adapter as adapter
        paths=adapter.arm_paths(self.root,{'home':'arms/17-frozen'})
        self.assertTrue(str(paths['db']).endswith('.memory-prototype.sqlite'))

    def test_malformed_cli_report_preserves_independently_correct_artifact(self):
        address,calls=self.server()
        binary=self.root/'fake-mecha'
        source=(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py').read_text()
        source=source.replace('output(result)',"if slot.endswith('frozen-D1'):\n    (workspace/'decision.json').write_text(json.dumps({'status':'selected','route':'complete','reason':'authored artifact fixture','citations':[]}))\n    print('malformed CLI report')\nelse:\n    output(result)")
        binary.write_text(source);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial'
        self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        self.m.run(trial,max_slots=9)
        card=self.m.score(trial)
        row=next(r for r in card['rows'] if r['id']=='17-frozen-D1')
        self.assertEqual(row['artifact_status'],'valid')
        self.assertEqual(row['decision_correct'],1)
        self.assertEqual(row['qualified_correct'],0)
        self.assertEqual(row['execution'],'failed')

    def test_changed_input_halts_arm_before_any_later_memory_use(self):
        address,calls=self.server();binary=self.root/'fake-mecha'
        source=(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py').read_text()
        source=source.replace("text=(workspace/filename).read_text()","if slot=='17-frozen-O1':\n    (workspace/filename).chmod(0o600)\n    (workspace/filename).write_text('substituted observation')\ntext=(workspace/filename).read_text()")
        binary.write_text(source);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial';manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        result=self.m.run(trial)
        events=self.m.events(trial,manifest)
        event=next(e for e in events if e['slot']=='17-frozen-O1' and e['state']=='terminal')
        self.assertTrue(event['halt_arm'])
        self.assertNotIn('observation_source',event)
        self.assertFalse(any(e['slot']=='17-frozen-O2' for e in events))
        self.assertEqual(result['slot_states']['17-frozen-D1'],'not_run_due_to_input_change')
        self.assertEqual(self.m.run(trial)['executed_slots'],0)

    def test_binary_drift_after_first_slot_prevents_another_dispatch(self):
        import shutil
        from unittest.mock import patch
        import ordinary_lifetime_adapter as adapter
        address,calls=self.server();binary=self.root/'fake-mecha'
        shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial';manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        execute=adapter.execute_slot
        def change_after(*args):
            result=execute(*args);binary.write_text('#!/bin/sh\nexit 0\n');return result
        with patch.object(adapter,'execute_slot',change_after):
            with self.assertRaisesRegex(RuntimeError,'binary changed'): self.m.run(trial)
        self.assertEqual(len(calls),1)
        self.assertEqual(len(self.m.events(trial,manifest)),2)

    def test_artifact_snapshot_reads_exactly_one_bound_buffer(self):
        from unittest.mock import patch
        import ordinary_lifetime as core
        import ordinary_lifetime_adapter as adapter
        workspace=self.root/'workspace';workspace.mkdir();slotdir=self.root/'slot';slotdir.mkdir()
        raw=b'{"status":"abstained","route":null,"reason":"x","citations":[]}'
        (workspace/'decision.json').write_bytes(raw)
        with patch.object(core,'bounded',wraps=core.bounded) as reader:
            binding=adapter.snapshot_artifact(workspace,slotdir,'decision.json')
        self.assertEqual(reader.call_count,1)
        self.assertEqual((slotdir/'decision.json').read_bytes(),raw)
        self.assertEqual(binding['sha256'],core.digest(slotdir/'decision.json'))

    def test_unresolved_upstream_stops_run_and_restart_while_server_is_busy(self):
        import shutil,threading
        release=threading.Event();active=threading.Event()
        def held(path,body):
            active.set();release.wait(5)
            return b'{"model":"fixture-model","choices":[]}'
        address,calls=self.server(held);self.addCleanup(release.set)
        binary=self.root/'fake-mecha';shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial';manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        manifest['bounds']['seconds']=.3
        (trial/'manifest.json').write_bytes(self.m.encode(manifest))
        (trial/'manifest.sha256').write_text(self.m.digest(trial/'manifest.json')+'\n')
        try:
            with self.assertRaises(RuntimeError): self.m.run(trial)
            self.assertTrue(active.is_set())
            self.assertEqual(len(calls),1)
            self.assertEqual(self.m.slot_states(trial,manifest)[manifest['slots'][0]['id']],'indeterminate')
            with self.assertRaisesRegex(RuntimeError,'indeterminate'): self.m.run(trial)
            self.assertEqual(len(calls),1)
        finally:
            release.set()

    def test_fixture_drift_after_first_slot_cannot_change_later_arm_input(self):
        import shutil
        from unittest.mock import patch
        import ordinary_lifetime_adapter as adapter
        address,calls=self.server();binary=self.root/'fake-mecha'
        shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary);binary.chmod(0o700)
        resident=self.root/'resident.json';resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial';manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        execute=adapter.execute_slot
        def change_after(*args):
            result=execute(*args)
            public=json.loads((trial/'fixture.json').read_text());public['episodes'][0]['input']['batch']='MUTATED'
            (trial/'fixture.json').write_text(json.dumps(public))
            (trial/'system.txt').write_text('MUTATED SYSTEM')
            return result
        with patch.object(adapter,'execute_slot',change_after): self.m.run(trial,max_slots=3)
        for arm in self.m.ARMS:
            path=trial/'slots'/('17-'+arm+'-O1')/'workspace'/'observation.json'
            self.assertEqual(json.loads(path.read_text())['batch'],'A1')
            terminal=next(e for e in self.m.events(trial,manifest) if e['slot']=='17-'+arm+'-O1' and e['state']=='terminal')
            self.assertEqual(terminal['argv'][terminal['argv'].index('--system')+1],adapter.SYSTEM)

    def test_actual_subprocess_argv_isolation_failures_wire_and_no_replay(self):
        import shutil
        address,calls=self.server()
        binary=self.root/'fake-mecha'
        shutil.copyfile(HERE/'fixtures'/'ordinary_lifetime_fake_cli.py',binary)
        binary.chmod(0o700)
        resident=self.root/'resident.json'
        resident.write_text(json.dumps({'models':{'data':[{'id':'fixture-model','status':{'value':'loaded'}}]},'props':{'model_alias':'fixture-model'}}))
        trial=self.root/'trial'
        manifest=self.m.freeze(trial,binary,f'http://{address[0]}:{address[1]}','fixture-model',[17],resident)
        self.m.run(trial)
        card=self.m.score(trial)
        self.assertEqual(card['costs']['attempted_slots'],23)
        self.assertEqual(card['costs']['forwarded'],22) # failed induction spends its call; no reference for revision
        self.assertEqual(card['learning'][0]['outcome'],'failed')
        self.assertEqual(card['learning'][1]['outcome'],'unavailable_sources')
        self.assertEqual(card['costs_by_kind']['learning']['forwarded'],1)
        self.assertFalse(card['costs']['usage_complete'])
        for slot in manifest['slots']:
            if slot['kind']=='learning': continue
            event=next(e for e in self.m.events(trial,manifest) if e['slot']==slot['id'] and e['state']=='terminal')
            self.assertEqual(event['execution'],'completed',event)
            command=event['argv']
            self.assertEqual('--episodic-memory' in command,slot['arm']!='off')
            self.assertEqual('goal_context' in command,slot['arm']!='off')
            wire=b''.join(p.read_bytes() for p in (trial/'slots'/slot['id']/'wire').glob('*.request'))
            self.assertNotIn(b'PRIVATE_GOLD',wire)
            if slot['episode'] in ('O1','O2','D1','D2'):
                self.assertNotIn(b'identifiers',wire)
        before=len(calls)
        self.m.run(trial)
        self.assertEqual(len(calls),before)
