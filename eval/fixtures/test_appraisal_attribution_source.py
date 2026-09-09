import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from appraisal_mismatch_source import grade

ROOT=Path(__file__).resolve().parents[2]
CASES=json.loads((ROOT/'eval/fixtures/appraisal-attribution/cases.json').read_text())
class AttributionCases(unittest.TestCase):
    def test_registration_and_independent_decisions(self):
        m=tomllib.loads((ROOT/'eval/appraisal-attribution.toml').read_text())
        self.assertEqual(m['tasks']['ids'],[c['id'] for c in CASES])
        self.assertEqual(m['seeds'],[1,2,3])
        self.assertEqual(set(m['tasks']['mismatch_cases']),set(c['id'] for c in CASES))
        for i,c in enumerate(CASES):
            self.assertEqual(m['tasks']['mismatch_cases'][c['id']],{k:v for k,v in c.items() if k!='id'})
            if i>=6:self.assertNotIn('criteria',c)
            records=json.loads(c['files']['records.json']);chosen={}
            for ident in {r['id'] for r in records}:
                row=max((r for r in records if r['id']==ident),key=lambda r:r['revision'])
                if row['approved']:chosen[ident]=row['amount']
            context=json.loads(c['files']['context.json']);waiting=context['outbox_waiting'];limit=context['review_threshold']
            expected=dict(selected=sorted(chosen),total=sum(chosen.values()),review_first="unknown" if waiting is None else waiting>limit)
            self.assertEqual(c['artifacts']['answer.json'],expected)
        self.assertEqual({type(c['artifacts']['answer.json']['review_first']) for c in CASES[6:]},{bool,str})
    def test_correct_and_wrong_context_choices(self):
        for c in CASES:
            with tempfile.TemporaryDirectory() as d:
                ws=Path(d)
                for n,text in c['files'].items():(ws/n).write_text(text)
                expected=c['artifacts']['answer.json'];(ws/'answer.json').write_text(json.dumps(expected));self.assertTrue(grade(c,ws)['passed'])
                wrong={**expected,'review_first':not expected['review_first']};(ws/'answer.json').write_text(json.dumps(wrong));self.assertFalse(grade(c,ws)['passed'])
                (ws/'answer.json').write_text(json.dumps(expected));(ws/'context.json').write_text('{}');self.assertFalse(grade(c,ws)['passed'])
if __name__=='__main__':unittest.main()
