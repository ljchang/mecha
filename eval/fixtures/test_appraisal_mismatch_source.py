import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from appraisal_mismatch_source import CASES, grade

class MismatchSourceTests(unittest.TestCase):
    def test_manifest_and_independently_walked_gold_agree(self):
        manifest=tomllib.loads((Path(__file__).resolve().parents[1]/'appraisal-mismatch.toml').read_text())
        self.assertEqual(manifest['tasks']['ids'],[c['id'] for c in CASES])
        for c in CASES:
            self.assertEqual(manifest['tasks']['mismatch_cases'][c['id']],{k:v for k,v in c.items() if k!='id'})
            latest={};seen=set();name='head.json'
            while name:
                self.assertNotIn(name,seen);seen.add(name)
                node=json.loads(c['files'][name]);name=node['next']
                r=node.get('record')
                if not r or ('purpose' in r and r['purpose']!='support'):continue
                if r['revision']>latest.get(r['id'],{}).get('revision',0):latest[r['id']]=r
            self.assertEqual(len(seen),13)
            if c['id'].startswith('revision'):
                chosen=sorted(k for k,r in latest.items() if r['approved']);context=json.loads(c['files']['context.json'])
                expected=dict(selected=chosen,total=sum(latest[k]['amount'] for k in chosen),review_first=context['outbox_waiting']>context['review_threshold'])
            else:
                chosen=sorted(k for k,r in latest.items() if r['consent']);expected=dict(recipients=chosen,excluded_count=4-len(chosen),send_now=False)
            self.assertEqual(c['artifacts']['answer.json'],expected)

    def test_correct_wrong_and_mutated_artifacts(self):
        for c in CASES:
            with tempfile.TemporaryDirectory() as d:
                ws=Path(d)
                for name,text in c['files'].items():(ws/name).write_text(text)
                self.assertFalse(grade(c,ws)['passed'])
                (ws/'answer.json').write_text(json.dumps(c['artifacts']['answer.json']))
                self.assertTrue(grade(c,ws)['passed'])
                (ws/'answer.json').write_text('{}')
                self.assertFalse(grade(c,ws)['passed'])
                (ws/'answer.json').write_text(json.dumps(c['artifacts']['answer.json']))
                (ws/'head.json').write_text('{}')
                self.assertFalse(grade(c,ws)['passed'])

if __name__=='__main__':unittest.main()
