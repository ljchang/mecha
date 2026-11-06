import json
import unittest
from durable_model_trial import frozen_cases, expected, score, summarize


class FrozenPilotTests(unittest.TestCase):
    def test_sixteen_initial_rows_and_only_source_derived_answers(self):
        cases = frozen_cases()
        self.assertEqual(len(cases), 8)
        self.assertEqual(len({c['id'] for c in cases}), 8)
        self.assertEqual([expected(c, 'observed') for c in cases], [
            {'Nori'}, {'Arden'}, {'Quill'}, {'Arden'}, set(), set(), {'Nori', 'Arden'}, {'Nori'}])
        self.assertTrue(all(expected(c, 'none') == set() for c in cases))
        self.assertEqual(cases[0]['source']['observations'], cases[1]['source']['observations'])

    def test_source_and_contract_have_no_gold_or_model_answer(self):
        for c in frozen_cases():
            self.assertEqual(set(c['source']), {'kind', 'workload', 'observations', 'complete'})
            self.assertEqual(c['source']['workload'], c['contract']['workload'])
            for obs in c['source']['observations']:
                self.assertEqual(set(obs), {'choice', 'observed_minutes', 'energy', 'quality'})

    def test_no_memory_abstention_is_appropriate_and_not_a_solved_task(self):
        reply = dict(text=json.dumps(dict(choice='abstain', citations=[], reason='No evidence.')))
        result = score(frozen_cases()[0], 'none', reply, [])
        self.assertTrue(result['appropriate_abstention'])
        self.assertFalse(result['supported_optimal'])

    def test_tied_optimum_accepts_either_but_requires_all_source_citations(self):
        case = frozen_cases()[6]
        for choice in ('Nori', 'Arden'):
            reply = dict(text=json.dumps(dict(choice=choice, citations=['one', 'two', 'three'], reason='Synthetic.')))
            self.assertTrue(score(case, 'observed', reply, ['one', 'two', 'three'])['supported_optimal'])
        reply['text'] = json.dumps(dict(choice='Nori', citations=['one'], reason='Synthetic.'))
        self.assertFalse(score(case, 'observed', reply, ['one', 'two', 'three'])['supported_optimal'])

    def test_duplicate_json_and_refused_payload_cannot_score_as_supported(self):
        duplicate = dict(text='{"choice":"Arden","choice":"Nori","citations":["a"],"reason":"Synthetic."}')
        self.assertFalse(score(frozen_cases()[0], 'observed', duplicate, ['a'])['valid'])
        refused = dict(text=json.dumps(dict(choice='Nori', citations=['a'], reason='Synthetic.')), error='refusal')
        self.assertFalse(score(frozen_cases()[0], 'observed', refused, ['a'])['supported_optimal'])

    def rows(self):
        result = []
        for case in frozen_cases():
            for view in ('none', 'observed'):
                gold = expected(case, view)
                ids = ['e'+str(i+1) for i in range(len(case['source']['observations']))] if view == 'observed' else []
                reply = dict(text=json.dumps(dict(choice=sorted(gold)[0] if gold else 'abstain', citations=ids, reason='Synthetic.')),
                             model='synthetic-model', input_tokens=11, output_tokens=7, error=None)
                request = dict(model='synthetic-model', seed=7, response_schema=dict(type='object', description='Allowed citation IDs: '+json.dumps(ids)))
                report = dict(spec=dict(contract=case['contract'], model='synthetic-model', seed=7, view=view),
                              status=dict(initial=dict(phase='assessed'), retry=dict(phase='unused')),
                              initial=dict(request=request, result=dict(reply=reply, error=None, millis=10)), retry=None,
                              decision=dict(disposition='recommend' if gold else 'abstain', executed=False))
                result.append(dict(case=case['id'], view=view, report=report, error=None))
        return result

    def test_summary_audits_budget_schema_contract_and_unknown_usage(self):
        rows = self.rows()
        summary = summarize(rows, 'synthetic-model')
        self.assertTrue(summary['complete'])
        self.assertEqual(summary['initial_reserved'], 16)
        self.assertEqual(summary['revision_reserved'], 0)
        self.assertEqual(summary['arms']['none']['initial_appropriate_abstention'], 8)
        self.assertEqual(summary['arms']['observed']['initial_supported_optimal'], 6)
        self.assertEqual(summary['arms']['observed']['initial_appropriate_abstention'], 2)
        rows[0]['report']['initial']['result']['reply']['input_tokens'] = None
        self.assertEqual(summarize(rows, 'synthetic-model')['arms']['none']['initial_input_tokens_unknown'], 1)
        rows[0]['report']['spec']['seed'] = 99
        self.assertFalse(summarize(rows, 'synthetic-model')['complete'])

    def test_duplicate_missing_rows_and_schema_drift_are_not_complete(self):
        rows = self.rows()
        with self.assertRaises(ValueError):
            summarize(rows+[rows[0]], 'synthetic-model')
        self.assertFalse(summarize(rows[:-1], 'synthetic-model')['complete'])
        rows[0]['report']['initial']['request']['response_schema']['type'] = 'array'
        self.assertFalse(summarize(rows, 'synthetic-model')['complete'])


if __name__ == '__main__':
    unittest.main()
