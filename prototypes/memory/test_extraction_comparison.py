import unittest
from extraction_comparison import grade_arm, arms_for_case

class ComparisonTests(unittest.TestCase):
    def value(self):
        return {'calls':[{'model':'fixture','reasoning_chars':0,'input_tokens':10,'output_tokens':5}],
                'transport':{'thinking':False}, 'direct_judgment':{'protocol':'direct-coverage-v1','status':'covered',
                'reason':'Guard present.','evidence':[{'passage':'e1'}],'plan':[{'passage':'p1'}]}}
    def test_balanced_order(self):
        orders=[arms_for_case(i) for i in range(4)]
        self.assertEqual(len({tuple(x) for x in orders}),4)
        for col in zip(*orders): self.assertEqual(len(set(col)),4)
    def test_direct_matching_label_is_not_semantic_success(self):
        row=grade_arm(self.value(),0,{'id':'x','expected_coverage':'covered'},'fixture','direct-fast')
        self.assertTrue(row['host_valid']); self.assertTrue(row['coverage_label_matches'])
        self.assertIsNone(row['source_supported_explanation'])
    def test_mismatched_transport_or_model_fails(self):
        v=self.value()
        for arm in ['direct-thinking','extraction-fast']:
            self.assertFalse(grade_arm(v,0,{'id':'x','expected_coverage':'covered'},'fixture',arm)['host_valid'])
        v['calls'][0]['model']='other'
        self.assertFalse(grade_arm(v,0,{'id':'x','expected_coverage':'covered'},'fixture','direct-fast')['host_valid'])
    def test_malformed_and_failed_calls_stay_in_denominator(self):
        for v in [None, {}, {'calls':None}, self.value()]:
            row=grade_arm(v,1,{'id':'x','expected_coverage':'covered'},'fixture','direct-fast')
            self.assertFalse(row['host_valid']); self.assertFalse(row['coverage_label_matches'])
        v=self.value();v['direct_judgment']['status']='approved'
        self.assertFalse(grade_arm(v,0,{'id':'x','expected_coverage':'covered'},'fixture','direct-fast')['host_valid'])
    def test_requested_budget_must_match_the_arm(self):
        v=self.value();v['transport'].update(thinking=True,thinking_budget=1024)
        self.assertFalse(grade_arm(v,0,{'id':'x','expected_coverage':'covered'},'fixture','direct-thinking')['host_valid'])
    def test_reasoning_presence_is_measured_not_assumed(self):
        v=self.value();v['transport'].update(thinking=True,thinking_budget=512)
        row=grade_arm(v,0,{'id':'x','expected_coverage':'covered'},'fixture','direct-thinking')
        self.assertTrue(row['host_valid']);self.assertFalse(row['reasoning_observed'])


class RunnerBoundaryTests(unittest.TestCase):
    def test_budget_must_be_explicit_and_bounded(self):
        from extraction_comparison import configured_budget
        def m(args): return {'data':[{'id':'fixture','status':{'args':args}}]}
        self.assertEqual(configured_budget(m(['--reasoning-budget','4096']), 'fixture'),4096)
        for args in [[],['--reasoning-budget'],['--reasoning-budget','0'],['--reasoning-budget','8192'],['--reasoning-budget','4096','--reasoning-budget','12']]:
            with self.assertRaises((ValueError,TypeError)):configured_budget(m(args),'fixture')

    def run_fake(self, failure):
        import tempfile, json, subprocess
        from pathlib import Path
        from unittest.mock import patch
        import extraction_comparison as runner
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);binary=root/'binary';binary.write_text('fake');holds=root/'holds';holds.mkdir()
            cases=root/'cases.json';cases.write_text(json.dumps([{'id':'case-01','input':{'evidence':[{'source_id':'source','text':'evidence'}],'plan':['plan']},'expected_coverage':'covered','rubric':'GOLD SECRET'}]))
            output=root/'out'
            settings={'configured_reasoning_budget':4096,'props':{},'models':{}}
            responses=[settings]*5 if failure=='timeout' else [settings, RuntimeError('resident changed')]
            def invoke(argv, **kw):
                self.assertNotIn('GOLD SECRET', Path(argv[6]).read_text())
                self.assertEqual(kw['env']['MECHA_LOG'],'warn')
                self.assertNotIn('OPENAI_API_KEY',kw['env'])
                if '--thinking' in argv: self.assertEqual(argv[-2:],['--thinking-budget','512'])
                else: self.assertNotIn('--thinking-budget',argv)
                # Simulate the provider's reasoning-only debug log conditional.
                stderr='SECRET REASONING' if kw['env']['MECHA_LOG']=='debug' else 'empty response'
                raise subprocess.TimeoutExpired(argv,210,output='partial',stderr=stderr)
            argv=['trial','--binary',str(binary),'--output',str(output),'--model','fixture','--holds-dir',str(holds),'--cases',str(cases)]
            with patch('sys.argv',argv), patch.dict('os.environ',{'MECHA_LOG':'debug','OPENAI_API_KEY':'secret'}), patch.object(runner,'server_settings',side_effect=responses), patch.object(runner.subprocess,'run',side_effect=invoke) as calls:
                if failure=='timeout':runner.main()
                else:
                    with self.assertRaisesRegex(RuntimeError,'resident changed'):runner.main()
            progress=json.loads((output/'progress.json').read_text());rows=json.loads((output/'report.json').read_text())
            if failure=='timeout':
                self.assertEqual(len(rows),4);self.assertEqual(calls.call_count,4)
                self.assertTrue(all(r['exit_code']==124 and not r['host_valid'] for r in rows))
                self.assertEqual(progress['state'],'complete')
                self.assertNotIn('SECRET REASONING',''.join(p.read_text() for p in output.glob('*stderr.log')))
                self.assertEqual((output/'case-01-direct-fast-output.json').read_text(),'partial')
            else:
                self.assertEqual(progress['state'],'aborted');self.assertEqual(progress['planned'],4)
                self.assertEqual(len(rows),1);self.assertEqual(rows[0]['recorded_calls'],0)
                self.assertEqual(calls.call_count,0)
    def test_timeout_records_each_failure_without_gold_or_debug_leak(self):self.run_fake('timeout')
    def test_preflight_failure_marks_matrix_aborted_without_inference(self):self.run_fake('preflight')

if __name__=='__main__': unittest.main()
