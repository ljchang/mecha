"""Host grading must retain failed trials and keep self-assessment out of gain."""
import unittest
from assessment_calibration import CASES, grade


class CalibrationGrading(unittest.TestCase):
    def test_missing_json_is_a_failure_even_with_zero_exit(self):
        row = grade(None, CASES[0], {}, 0)
        self.assertFalse(row['completed'])
        self.assertFalse(row['prerequisite_correct'])
        self.assertEqual(row['recorded_calls'], 0)

    def test_nonzero_exit_never_becomes_a_completed_trial(self):
        row = grade({'evaluation': {'initial': {}}, 'calls': [{'input_tokens': 12, 'output_tokens': 3}], 'error': 'invalid citation'}, CASES[0], {}, 1)
        self.assertFalse(row['completed'])
        self.assertEqual(row['input_tokens'], 12)
        self.assertEqual(row['error'], 'invalid citation')

    def test_missing_usage_is_not_zero_cost(self):
        value = {'calls': [{'input_tokens': 12, 'output_tokens': 3},
                           {'input_tokens': None, 'output_tokens': None}], 'error': 'timeout'}
        row = grade(value, CASES[0], {}, 1)
        self.assertIsNone(row['input_tokens'])
        self.assertIsNone(row['output_tokens'])
        self.assertEqual(row['known_input_tokens'], 12)
        self.assertEqual(row['input_usage_calls'], 1)
        self.assertEqual(row['recorded_calls'], 2)
        empty = grade(None, CASES[0], {}, 124)
        self.assertIsNone(empty['input_tokens'])

    def test_self_recommendation_is_not_scored_as_revision_improvement(self):
        check = dict(dimension='prerequisites', status='unknown', citations=[dict(source_id='other')])
        value = dict(calls=[], evaluation=dict(initial=dict(assessments=[dict(checks=[check])],
            context=dict(records=[]), recommended=None, citation_protocol='host-passages-v1'),
            revision=dict(recommended='revision', candidates=[dict(steps=['invented deadline'])])))
        row = grade(value, CASES[0], dict(evidence=dict(revision='original')), 0)
        self.assertTrue(row['completed'])
        self.assertFalse(row['prerequisite_correct'])
        self.assertFalse(row['original_evidence_cited'])
        self.assertNotIn('revision_improved', row)
        self.assertEqual(row['revision_recommended'], 'revision')


if __name__ == '__main__':
    unittest.main()
