"""Evidence-v3 regressions use authored packets only; no fresh qualification gold."""
import copy
import unittest
from test_support_trial import GroundedWireTests, trial


class EvidenceWireTests(unittest.TestCase):
    generation = GroundedWireTests.generation
    def packet(self):
        packet = GroundedWireTests.packet(self)
        packet['protocol'] = 'situated-support-wire-v3'
        packet['evidence_metadata'] = [dict(source_id='source:a', role='ArtifactObservation', interpretation_influenced=False, captures=[])]
        return packet

    def evaluation(self, packet):
        payload = GroundedWireTests.evaluation(self, packet)
        payload['units'][0]['original_citations'] = [dict(citation_index=0, relation='DirectSupport')]
        return payload

    def test_optional_diagnostic_does_not_replace_original(self):
        packet = self.packet()
        payload = self.evaluation(packet)
        payload['units'][0]['evidence'] = []
        self.assertEqual(trial.expand_evaluation(packet, payload)['checks'][0]['citations'], packet['proposal']['units'][0]['citations'])
        payload['units'][0]['original_citations'][0]['relation'] = 'ContextOnly'
        with self.assertRaises(ValueError):
            trial.expand_evaluation(packet, payload)

    def test_exact_original_slot_coverage(self):
        for slots in ([], [dict(citation_index=1, relation='DirectSupport')], [dict(citation_index=True, relation='DirectSupport')], [dict(citation_index=0, relation='bogus')]):
            packet = self.packet()
            payload = self.evaluation(packet)
            payload['units'][0]['original_citations'] = slots
            with self.assertRaises(ValueError):
                trial.expand_evaluation(packet, payload)

    def test_unknown_contradictory_hypothesis_is_unsupported(self):
        packet = self.packet()
        payload = self.evaluation(packet)
        packet['proposal']['units'][0]['kind'] = 'Hypothesis'
        payload['units'][0].update(status='Unknown', factual_premises='None', questions=['What happened?'])
        payload['units'][0]['original_citations'][0]['relation'] = 'Contradiction'
        check = trial.expand_evaluation(packet, payload)['checks'][0]
        self.assertTrue(check['unsupported_factual_premise'])
        self.assertEqual(check['status'], 'Unknown')

    def test_metadata_exact_coverage_and_influence(self):
        for change in (lambda p: p['evidence_metadata'].clear(), lambda p: p['evidence_metadata'][0].update(source_id='foreign'), lambda p: p['evidence_metadata'][0].update(interpretation_influenced=True)):
            packet = self.packet()
            change(packet)
            with self.assertRaises(ValueError):
                trial.validate_catalog(packet)


class RevisedSelectionTests(unittest.TestCase):
    def test_later_qualified_known_and_conflict(self):
        offered = dict(units=[dict(id='u', kind='RecordedEvent')], judgments=[dict(bearing='Unknown', because=[]), dict(bearing='Good', because=['u']), dict(bearing='Good', because=['u'])])
        units = {'u': dict(admitted=True, status='Supported')}
        checks = {'judgment:' + str(i): dict(status='Supported', unsupported_factual_premise=False) for i in range(3)}
        self.assertEqual(trial.revised_decisions(offered, units, checks), ['ExplicitUnknown', 'Retained', 'Superseded'])
        offered['judgments'][2]['bearing'] = 'Bad'
        self.assertEqual(trial.revised_decisions(offered, units, checks), ['ExplicitUnknown', 'Conflicting', 'Conflicting'])
        checks['judgment:2']['unsupported_factual_premise'] = True
        self.assertEqual(trial.revised_decisions(offered, units, checks), ['ExplicitUnknown', 'Retained', 'UnsupportedJudgment'])

    def test_zero_factual_defects_does_not_prove_clean_citations(self):
        rows = [dict(case_id=str(i), valid=True, manual_audit_complete=True, probe_outcome_observed=True, generated_outcome_observed=True, true_supported=1, uncertain_appropriate=1, correct_bearing_supported=1, false_facts_supported=0, wrong_generated_bearings_supported=0, wrong_generated_bearings_retained=0) for i in range(8)]
        self.assertFalse(trial.summarize(rows, trial.EVIDENCE_PROTOCOL)['gate_passed'])

class EvidenceAuthorityTests(unittest.TestCase):
    def test_metadata_reconstruction_rejects_forged_capture_association(self):
        import hashlib
        import sqlite3
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            connection = sqlite3.connect(':memory:')
            connection.executescript('CREATE TABLE revision(object_id TEXT,id TEXT,lifecycle TEXT,payload TEXT,seq INTEGER DEFAULT 1); CREATE TABLE host_capture_receipt(projection_object TEXT,projection_revision TEXT,state TEXT,originals TEXT,receipt_id TEXT DEFAULT "receipt"); CREATE TABLE dependency(output TEXT,input TEXT);')
            reference = dict(object='capture', revision='c1')
            original = dict(object='original', revision='o1')
            projection = dict(criterion={'text':'Historical requirement'}, receipts=[], artifact={}, occurred_at=None, originals=[dict(reference=original, kind='UserRequest', line=7, pointer='/text', occurred_at=None)])
            record = dict(kind='evidence', data=dict(text=trial.encode(projection), source=dict(connector='host-capture-projection-v2')))
            connection.execute('INSERT INTO revision(object_id,id,lifecycle,payload) VALUES (?,?,?,?)', ('capture', 'c1', 'active', trial.encode(record)))
            connection.execute('INSERT INTO revision(object_id,id,lifecycle,payload) VALUES (?,?,?,?)', ('original', 'o1', 'active', trial.encode(dict(kind='evidence', data=dict(text='The input is recorded.', source=dict(connector='original',account='synthetic',id='o'))))))
            connection.execute('INSERT INTO host_capture_receipt(projection_object,projection_revision,state,originals) VALUES (?,?,?,?)', ('capture', 'c1', 'active', trial.encode([original])))
            def artifact(name, value):
                (root / name).write_bytes(value)
                return dict(path=name, sha256=hashlib.sha256(value).hexdigest(), bytes=len(value))
            report = dict(source_only_database=artifact('source.sqlite', connection.serialize()), capture_receipt=artifact('receipt.json', trial.encode(dict(id='receipt', projection=reference, originals=[original])).encode()))
            connection.close()
            def source_id(reference, field):
                return 'source:' + hashlib.sha256(trial.compact_bytes([reference,field])).hexdigest()
            cid, oid = source_id(reference,'/criterion'), source_id(original,'/text')
            packet = dict(sources=[dict(id=cid, source=reference, field='/criterion', kind='Observation', text=trial.compact_bytes(projection['criterion']).decode()), dict(id=oid, source=original, field='/text', kind='Observation', text='The input is recorded.')])
            association = dict(capture=reference, relation='SelectedCapture', capture_time=None, passage_time=None, transcript_line=None)
            packet['evidence_metadata'] = [dict(source_id=cid, role='HistoricalCriterion', interpretation_influenced=False, captures=[association]), dict(source_id=oid, role='UserStatement', interpretation_influenced=False, captures=[dict(association, transcript_line=7)])]
            self.assertTrue(trial.audit_capture_metadata(root, report, packet))
            forged = copy.deepcopy(packet)
            forged['sources'][1]['text'] = 'Invented source bytes.'
            with self.assertRaises(ValueError):
                trial.audit_capture_metadata(root, report, forged)
            forged = copy.deepcopy(packet)
            forged['sources'][1]['kind'] = 'PriorInterpretation'
            forged['evidence_metadata'][1].update(role='PriorInterpretation', captures=[])
            with self.assertRaises(ValueError):
                trial.audit_capture_metadata(root, report, forged)
            for mutate in (lambda p: p['sources'][1].update(field='/forged'), lambda p: p['sources'][1].update(kind='InterpretationInfluenced')):
                forged = copy.deepcopy(packet)
                mutate(forged)
                with self.assertRaises(ValueError):
                    trial.audit_capture_metadata(root, report, forged)
            bad_report = dict(report, capture_receipt=artifact('forged-receipt.json', trial.encode(dict(id='forged', projection=reference, originals=[original])).encode()))
            with self.assertRaises(ValueError):
                trial.audit_capture_metadata(root, bad_report, packet)
            for key, value in [('role', 'AssistantStatement'), ('captures', [dict(association, transcript_line=99)])]:
                forged = copy.deepcopy(packet)
                forged['evidence_metadata'][1][key] = value
                with self.assertRaises(ValueError):
                    trial.audit_capture_metadata(root, report, forged)
            with self.assertRaises(ValueError):
                trial.audit_capture_metadata(root, {}, packet)

    def test_renderer_uses_selected_bearing_and_canonical_indices(self):
        proposal = dict(judgments=[dict(goal='g', bearing='Unknown', because=[]), dict(goal='g', bearing='Good', because=['b'])])
        judgments = [dict(index=0, reason='ExplicitUnknown', bearing='Unknown'), dict(index=1, reason='Retained', bearing='Good')]
        accepted = [dict(id='a', kind='RecordedEvent', citations=[{}, {}]), dict(id='b', kind='GoalAssessment', citations=[{}])]
        self.assertEqual(trial.retained_judgments(proposal, judgments, accepted), [dict(goal='g', bearing='Good', because=[2])])
        judgments[1].update(reason='Conflicting', bearing='Unknown')
        self.assertEqual(trial.retained_judgments(proposal, judgments, accepted), [dict(goal='g', bearing='Unknown', because=[])])

    def test_citation_audit_covers_rejected_originals_and_preserves_missing_denominators(self):
        validation = dict(original_citation_slots=['generated:rejected:0', 'probe:u:0'])
        with unittest.mock.patch.object(trial, 'manual_audit', return_value=(True, 0)):
            self.assertEqual(trial.citation_audit(validation, dict(inspected_original_citations=['probe:u:0'], original_citation_defects=[])), (False, None))
            self.assertEqual(trial.citation_audit(validation, dict(inspected_original_citations=validation['original_citation_slots'], original_citation_defects=['generated:rejected:0'])), (True, 1))
        summary = trial.summarize([], trial.EVIDENCE_PROTOCOL)
        self.assertFalse(summary['gate_passed'])
        self.assertEqual(summary['unknown_probe_outcomes'], 8)
        self.assertEqual(summary['unknown_generated_outcomes'], 8)

class EvidenceReportTests(unittest.TestCase):
    def test_complete_v3_raw_durable_and_rendered_selection_tamper(self):
        helper = GroundedWireTests()
        helper.addCleanup = self.addCleanup
        old, report = helper.grounded_report(('Unknown', 'Good'))
        report['protocol'] = trial.EVIDENCE_PROTOCOL
        for name in ('generate', 'evaluate_generated', 'evaluate_probe'):
            stage = report['stages'][name]
            request = trial.decode((old.root / stage['request']['path']).read_text())
            text = request['request']['messages'][0]['content'][0]['text']
            packet = trial.decode(text[len('<support-input>\n'):-len('\n</support-input>')])
            packet['protocol'] = 'situated-support-wire-v3'
            packet['evidence_metadata'] = [dict(source_id=s['id'], role='Unclassified', interpretation_influenced=False, captures=[]) for s in packet['sources']]
            raw = trial.decode((old.root / stage['reply']['path']).read_text())
            payload = trial.decode(raw['message']['content'][0]['text'])
            if name != 'generate':
                for item, unit in zip(payload['units'], packet['proposal']['units']):
                    item['original_citations'] = [dict(citation_index=i, relation='DirectSupport' if item['status'] == 'Supported' else 'Uncertain') for i in range(len(unit['citations']))]
            wire, bounds = FinalEvidenceSchemaTests().request(packet) if name != 'generate' else helper.request(packet)
            request['request'].update(wire['request'])
            request['wire_body'].update(wire['wire_body'])
            raw['message']['content'][0]['text'] = trial.encode(payload)
            rb, pb = trial.encode(request).encode(), trial.encode(raw).encode()
            stage.update(bounds)
            stage['request'] = old.artifact(stage['request']['path'], rb)
            stage['reply'] = old.artifact(stage['reply']['path'], pb)
            stage['provider_audit'] = old.artifact(stage['provider_audit']['path'], rb + pb)
            if name == 'generate':
                continue
            kind = name.removeprefix('evaluate_')
            support = trial.decode((old.root / report['support_reports'][kind]['path']).read_text())
            support['protocol'] = 'situated-factual-support-v2'
            checks = {c['target']: c for c in trial.expand_evaluation(packet, payload)['checks']}
            units = {u['id']: dict(admitted=checks['unit:'+u['id']]['status']=='Supported', status=checks['unit:'+u['id']]['status']) for u in packet['proposal']['units']}
            reasons = trial.revised_decisions(packet['proposal'], units, checks)
            for index, entry in enumerate(support['judgments']):
                entry.update(status=checks['judgment:'+str(index)]['status'], reason=reasons[index], unsupported_factual_premise=False,
                             bearing=entry['offered_bearing'] if reasons[index]=='Retained' else 'Unknown')
            report['support_reports'][kind] = old.artifact(report['support_reports'][kind]['path'], trial.encode(support).encode())
            accepted = [dict(u['unit'], status=u['status']) for u in support['units'] if u['disposition']=='Accepted']
            judgments = [dict(entry) for entry in support['judgments']]
            draft = dict(producer_version='situated-interpretation-v3', model=support['generator_model'], support=support,
                         claims=[dict(source_id=c['source_id'], statement=u['text'], quote=c['quote']) for u in accepted if u['kind'] in ('RecordedEvent','GoalAssessment') for c in u['citations']],
                         judgments=trial.retained_judgments(packet['proposal'], judgments, accepted))
            labels = {'RecordedEvent':'Recorded event', 'GoalAssessment':'Goal assessment', 'Hypothesis':'Speculative hypothesis', 'Prediction':'Speculative prediction'}
            dropped_by = {}
            for entry in support['units']:
                if entry['disposition'] == 'Rejected':
                    dropped_by[entry['status']] = dropped_by.get(entry['status'], 0) + 1
            draft.update(interpretation='\n'.join(labels[u['kind']]+': '+u['text'] for u in accepted) if accepted else 'No supported statements retained.', prediction=None,
                         grounding=dict(offered=len(draft['claims'])+sum(dropped_by.values()), dropped=sum(dropped_by.values()), dropped_by=dropped_by, unresolved_goals=0))
            claimed = {c['source_id'] for c in draft['claims']}
            locators = [{key: source[key] for key in ('id','source','field','kind')} for source in packet['sources'] if source['id'] in claimed]
            record = dict(context_key='opaque', criterion=packet['criterion'], host_context_id='opaque', draft=draft, claim_sources=locators)
            report['artifacts']['retained_'+kind] = old.artifact('retained-'+kind+'.json', trial.encode(record).encode())
            if kind == 'probe':
                for summary, entry in zip(report['probe']['judgments'], support['judgments']):
                    summary.update(status=entry['status'], reason=entry['reason'], bearing=entry['bearing'],
                                   disposition='retained' if entry['reason'] in ('Retained','ExplicitUnknown') else 'downgraded')
        old.write_report(report)
        with unittest.mock.patch.object(trial, 'audit_capture_metadata', return_value=True):
            validation = trial.validate_report(old.root, old.case, 'resident-test', trial.EVIDENCE_PROTOCOL)
            self.assertTrue(validation['valid'], validation['errors'] + validation['generated_errors'] + validation['probe_errors'])
            self.assertEqual(validation['generated_outcome']['retained_judgments'][0]['bearing'], 'Good')
            descriptor = report['artifacts']['retained_generated']
            retained = trial.decode((old.root / descriptor['path']).read_text())
            for mutate in (lambda r: r['draft'].update(interpretation='Injected rejected prose.'),
                           lambda r: r['draft'].pop('grounding'),
                           lambda r: r['draft'].update(prediction='Invented outcome.'),
                           lambda r: r['claim_sources'][0].update(field='/forged'),
                           lambda r: r['draft']['grounding'].update(dropped=999)):
                forged = copy.deepcopy(retained)
                mutate(forged)
                report['artifacts']['retained_generated'] = old.artifact(descriptor['path'], trial.encode(forged).encode())
                old.write_report(report)
                self.assertFalse(trial.validate_report(old.root, old.case, 'resident-test', trial.EVIDENCE_PROTOCOL)['valid'])

class FinalEvidenceSchemaTests(unittest.TestCase):
    def request(self, packet):
        request, stage = GroundedWireTests.request(self, packet)
        schema = request['request']['response_schema']
        branches = []
        for branch in schema['properties']['units']['items']['oneOf']:
            properties = branch['properties']
            properties['evidence']['minItems'] = 0
            properties['original_citations'] = copy.deepcopy(trial.ORIGINAL_CITATION_SCHEMA)
            branch['required'].append('original_citations')
            if properties['status']['enum'] != ['Supported']:
                branches.append(branch)
                continue
            for count in (1, 2):
                targets = ['unit:'+u['id'] for u in packet['proposal']['units'] if len(u['citations']) == count and 'unit:'+u['id'] in properties['target']['enum']]
                if targets:
                    split = copy.deepcopy(branch)
                    split['properties']['target']['enum'] = targets
                    originals = split['properties']['original_citations']
                    originals.update(uniqueItems=True, minItems=count, maxItems=count)
                    originals['items']['properties']['citation_index']['maximum'] = count-1
                    originals['items']['properties']['relation']['enum'] = ['DirectSupport']
                    branches.append(split)
        schema['properties']['units']['items']['oneOf'] = branches
        request['wire_body']['response_format']['schema'] = copy.deepcopy(schema)
        text = request['request']['messages'][0]['content'][0]['text']
        stage['packet_bytes'] = len(request['request']['system'].encode()) + len(text.encode()) + len(trial.compact_bytes(schema))
        return request, stage

    def test_supported_branches_exact_one_two_and_zero_slots(self):
        helper = EvidenceWireTests()
        packet = helper.packet()
        payload = helper.evaluation(packet)
        unit = packet['proposal']['units'][0]
        packet['proposal']['units'].extend([dict(unit, id='two', citations=unit['citations'] + [dict(unit['citations'][0], quote='observed artifact contains item p1.')]), dict(id='zero', kind='Hypothesis', text='A future check might succeed.', citations=[])])
        packet['targets'] = ['unit:'+u['id'] for u in packet['proposal']['units']] + ['judgment:0']
        request, stage = self.request(packet)
        self.assertEqual(trial.wire_packet(request, stage), packet)
        branches = request['request']['response_schema']['properties']['units']['items']['oneOf']
        supported = [b for b in branches if b['properties']['status']['enum'] == ['Supported']]
        self.assertEqual([b['properties']['target']['enum'] for b in supported], [['unit:u1'], ['unit:two']])
        for mutate in (lambda b: b['properties']['original_citations'].update(uniqueItems=False), lambda b: b['properties']['original_citations']['items']['properties']['relation'].update(enum=['DirectSupport','ContextOnly']), lambda b: b['properties']['original_citations'].update(minItems=0)):
            changed = copy.deepcopy(request)
            branch = changed['request']['response_schema']['properties']['units']['items']['oneOf'][0]
            mutate(branch)
            changed['wire_body']['response_format']['schema'] = copy.deepcopy(changed['request']['response_schema'])
            size = len(changed['request']['system'].encode()) + len(changed['request']['messages'][0]['content'][0]['text'].encode()) + len(trial.compact_bytes(changed['request']['response_schema']))
            with self.assertRaises(ValueError):
                trial.wire_packet(changed, dict(packet_bytes=size))
