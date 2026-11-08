"""Offline authored smoke contracts; optional actual binary test never uses a model."""
import copy
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import support_native_smoke as smoke
from test_support_trial import GroundedWireTests
from test_support_evidence_trial import FinalEvidenceSchemaTests


class NativeSmokeTests(unittest.TestCase):
    def test_authored_server_reply_covers_zero_one_two_original_slots(self):
        packet = GroundedWireTests().packet()
        packet['protocol'] = 'situated-support-wire-v3'
        source = dict(id='source:b', source=dict(object='b',revision='1'), field='/action', kind='ActionInvocation', text='A native write action was captured.')
        packet['sources'].append(source)
        packet['catalog']['citations'].append(dict(handle='ActionInvocation:'+packet['request_key']+':c1', source_id=source['id'], source=source['source'], field=source['field'], kind=source['kind'], quote=source['text']))
        packet['evidence_metadata'] = [dict(source_id=s['id'], role=s['kind'], interpretation_influenced=False, captures=[]) for s in packet['sources']]
        generation = smoke.authored_reply(packet)
        proposal = smoke.trial.expand_generation(packet, generation)
        self.assertEqual([len(u['citations']) for u in proposal['units']], [1,2,0])
        packet.update(proposal=proposal, targets=['unit:'+u['id'] for u in proposal['units']]+['judgment:0','judgment:1'])
        request, stage = FinalEvidenceSchemaTests().request(packet)
        self.assertEqual(smoke.trial.wire_packet(request, stage), packet)
        checks = smoke.trial.expand_evaluation(packet, smoke.authored_reply(packet))['checks']
        self.assertEqual([c['status'] for c in checks[:3]], ['Supported','Supported','Unknown'])
        self.assertEqual(checks[2]['citations'], [])

    def test_authored_probe_ids_do_not_expose_native_forbidden_labels(self):
        case = smoke.contract_case()
        forbidden = {'true', 'false', 'uncertain', 'supported', 'contradicted', 'unknown'}
        ids = {unit['id'] for unit in case['probe']['units']}
        self.assertFalse(ids & forbidden)
        self.assertEqual({case['annotations'][key] for key in ('true_unit','false_unit','uncertain_unit')}, ids)

    def test_explicit_smoke_rejects_missing_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                smoke.run(Path(directory)/'absent-native-binary', Path(directory)/'output')

    @unittest.skipUnless(os.environ.get('MECHA_SUPPORT_NATIVE_BINARY'), 'set MECHA_SUPPORT_NATIVE_BINARY for authored actual-native HTTP contract; explicit CLI never skips')
    def test_actual_native_binary_full_saved_payload_audit(self):
        with tempfile.TemporaryDirectory() as directory:
            result = smoke.run(os.environ['MECHA_SUPPORT_NATIVE_BINARY'], Path(directory)/'smoke')
            self.assertTrue(result['valid'])
            self.assertEqual(result['calls'], 3)
            self.assertFalse(result['model_inference'])
