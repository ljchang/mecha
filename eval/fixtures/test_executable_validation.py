"""The oracle must reject promises, stale artifacts and damaged evidence."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import executable_validation_source as source

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('pilot', ROOT/'scripts/executable-validation.py')
pilot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pilot)


class ExecutableValidationTests(unittest.TestCase):
    def test_every_registered_start_fails_and_correct_artifact_passes(self):
        for case in source.source.CASES:
            with self.subTest(task=case['id']), tempfile.TemporaryDirectory() as tmp:
                workspace=Path(tmp)
                for name,text in case['files'].items(): (workspace/name).write_text(text)
                self.assertFalse(source.source.grade(case,workspace)['passed'])
                # No trace requirement: correct artifacts pass however they were produced.
                for name,value in case['artifacts'].items(): (workspace/name).write_text(json.dumps(value))
                self.assertTrue(source.source.grade(case,workspace)['passed'])
                (workspace/case['preserve'][0]).write_text('changed evidence')
                self.assertFalse(source.source.grade(case,workspace)['passed'])

    def test_duplicate_keys_and_links_are_not_success(self):
        case=next(c for c in source.source.CASES if c['id']=='missing-context')
        with tempfile.TemporaryDirectory() as tmp:
            workspace=Path(tmp)
            for name,text in case['files'].items(): (workspace/name).write_text(text)
            answer=workspace/'answer.json'
            answer.write_text('{"review_first": true, "review_first": "unknown"}')
            self.assertFalse(source.source.grade(case,workspace)['passed'])
            answer.unlink(); answer.symlink_to(workspace/'context.json')
            self.assertFalse(source.source.grade(case,workspace)['passed'])

    def test_empty_rules_hash_is_no_exposure_and_missing_is_unknown(self):
        self.assertIs(pilot.rule_exposure({'rules_hash':'cbf29ce484222325'}), False)
        self.assertIs(pilot.rule_exposure({'rules_hash':'39a27bbb5ef07c9e'}), True)
        self.assertIsNone(pilot.rule_exposure({}))

    def test_cheaper_unfinished_task_is_a_regression(self):
        rows=[dict(id='a',arm='control',task='t',seed=1,repetition=0,status='done',passed=True,stats={'turns':12}),
              dict(id='b',arm='candidate',task='t',seed=1,repetition=0,status='done',passed=False,stats={'turns':10})]
        result=pilot.paired(rows,'control','candidate')
        self.assertEqual(result['outcomes'],{'regressed':1})
        self.assertIsNone(result['cost_on_both_pass']['turns']['treatment_minus_control'])
        rows[1]['passed']=None
        self.assertEqual(pilot.paired(rows,'control','candidate')['missing_or_ungraded'],1)


if __name__=='__main__': unittest.main()
