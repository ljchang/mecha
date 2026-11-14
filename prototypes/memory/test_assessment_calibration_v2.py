import unittest
import json
from pathlib import Path
from assessment_calibration_v2 import CASES, grade


def result(status='kept_existing'):
    return dict(calls=[dict(input_tokens=12, output_tokens=4)], evaluation=dict(
        revision_status=status, revision=None, initial=dict(
            assessments=[dict(id='candidate',disposition='consider',checks=[dict(dimension=d, status='supported' if d != 'concept_applicability' else 'not_applicable', citations=[dict(source_id='e')]) for d in ['prerequisites','factual_support','concept_applicability']])],
            candidates=[dict(id='candidate', steps=['Already adequate.'])],
            context=dict(records=[]), recommended='candidate', citation_protocol='host-passages-v1', evaluation_protocol='constraint-assessment-v2')))


class ConservativeCalibration(unittest.TestCase):
    def test_development_goal_does_not_expose_diagnostic_case_names(self):
        cases = json.loads((Path(__file__).parent / 'fixtures/requirements_development.json').read_text())
        for case in cases:
            self.assertNotIn(case['id'], case['goal'])
            for label in ('blocked', 'ready', 'unknown', 'universal', 'local', 'guarded'):
                self.assertNotIn(label, case['goal'])

    def test_kept_plan_is_complete_without_a_fabricated_revision(self):
        row = grade(result(), CASES[0], dict(evidence=dict(revision='e')), 0)
        self.assertTrue(row['completed'])
        self.assertTrue(row['prerequisite_correct'])
        self.assertEqual(row['revision_status'], 'kept_existing')
        self.assertIsNone(row['revision_steps'])
        self.assertIsNone(row['revision_recommended'])
        self.assertEqual(row['retained_steps'], ['Already adequate.'])
        self.assertEqual(row['recorded_calls'], 1)
        self.assertNotIn('revision_improved', row)

    def test_not_requested_is_not_claimed_as_preserving_an_adequate_plan(self):
        row = grade(result('not_requested'), CASES[0], dict(evidence=dict(revision='e')), 0)
        self.assertEqual(row['revision_status'], 'not_requested')
        self.assertIsNone(row['retained_steps'])

    def test_incomplete_reassessment_remains_a_failed_trial(self):
        row = grade(dict(evaluation=None, error='missing evidence', calls=[dict(input_tokens=None, output_tokens=None)]), CASES[0], {}, 1)
        self.assertFalse(row['completed'])
        self.assertFalse(row['prerequisite_correct'])
        self.assertIsNone(row['input_tokens'])
        self.assertEqual(row['recorded_calls'], 1)

    def test_old_protocol_and_inconsistent_revisions_are_failed_rows(self):
        variants=[]
        old=result();old['evaluation']['initial']['evaluation_protocol']='old';variants.append(old)
        missing=result();del missing['evaluation']['revision_status'];variants.append(missing)
        variants.append(result('reassessed'))
        invalid=result();invalid['evaluation']['revision']={};variants.append(invalid)
        invalid=result();invalid['evaluation']['initial']['recommended']=None;variants.append(invalid)
        for value in variants:
            row=grade(value, CASES[0], dict(evidence=dict(revision='e')),0)
            self.assertFalse(row['completed'])
            self.assertIn('error',row)

    def test_v3_requires_new_fields_and_outcome_check(self):
        value=result();value['evaluation']['initial']['evaluation_protocol']='requirements-assessment-v3'
        row=grade(value,CASES[0],dict(evidence=dict(revision='e')),0,'requirements-assessment-v3')
        self.assertFalse(row['completed'])

    def test_v3_host_requirement_blocker_overrides_optimistic_prerequisite(self):
        value=result();value['evaluation']['initial']['evaluation_protocol']='requirements-assessment-v3'
        a=value['evaluation']['initial']['assessments'][0]
        a.update(requirements=[],requirement_status='contradicted')
        a['checks'].append(dict(dimension='outcome',status='unknown',citations=[]))
        a['disposition']='revise'
        value['evaluation']['initial']['recommended']=None
        value['evaluation']['revision_status']='not_requested'
        row=grade(value,CASES[1],dict(evidence=dict(revision='e')),0,'requirements-assessment-v3')
        self.assertTrue(row['prerequisite_correct'])
        self.assertEqual(row['model_prerequisite'],'supported')
        self.assertEqual(row['prerequisite'],'contradicted')

    def test_malformed_calls_and_invented_revision_recommendations_fail_closed(self):
        value=result();value['calls']=[None]
        self.assertFalse(grade(value,CASES[0],{},0)['completed'])
        value=result('reassessed');value['evaluation']['initial']['recommended']=None
        revision=result()['evaluation']['initial'];revision['recommended']='nonexistent'
        value['evaluation']['revision']=revision
        self.assertFalse(grade(value,CASES[0],dict(evidence=dict(revision='e')),0)['completed'])

    def test_v5_requires_traceable_repair_and_does_not_count_deferral_as_progress(self):
        value=result('reassessed');initial=value['evaluation']['initial']
        initial.update(evaluation_protocol='bound-repair-v5',recommended=None)
        initial['assessments'][0].update(requirements=[],requirement_status='unknown',disposition='needs_evidence')
        initial['assessments'][0]['checks'].append(dict(dimension='outcome',status='unknown',citations=[]))
        revision=json.loads(json.dumps(initial))
        revision['candidates']=[dict(id='revision',steps=['Defer the original step.'])]
        revision['assessments'][0]['id']='revision'
        value['evaluation']['revision']=revision
        refs=dict(evidence=dict(revision='e'))
        self.assertFalse(grade(value,CASES[0],refs,0,'bound-repair-v5')['completed'])
        value['evaluation']['repair']=dict(candidate='candidate',all_deferred=True,steps=[dict(step=1,operation='defer',original='Already adequate.',rendered='Defer the original step.',requirement_ids=[])])
        row=grade(value,CASES[0],refs,0,'bound-repair-v5')
        self.assertTrue(row['completed']);self.assertTrue(row['all_deferred'])
        value['evaluation']['repair']['steps'][0]['original']='A different action'
        self.assertFalse(grade(value,CASES[0],refs,0,'bound-repair-v5')['completed'])
        value['evaluation']['repair']['steps'][0]['original']='Already adequate.'
        revision.update(recommended='revision');revision['assessments'][0]['disposition']='consider'
        self.assertFalse(grade(value,CASES[0],refs,0,'bound-repair-v5')['completed'])


if __name__ == '__main__':
    unittest.main()
