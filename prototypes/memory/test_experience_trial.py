import unittest
from experience_trial import summarize


class ArtifactTests(unittest.TestCase):
    def test_checked_counts_shared_draft_once_and_host_withholding_separately(self):
        call=dict(reply=dict(model='model',input_tokens=4,output_tokens=2),millis=5)
        result=dict(optimal=False,evidence_complete=False,citations_complete=False,
                    appropriate_abstention=False,trace={'outcome':{'feasible':False}})
        base=dict(case='cinder-1',arm='retrieved',view='measurements',policy='baseline',
                  error=None,call=call,request={'prompt':'same'},result=result)
        checked=dict(base,policy='checked',review=dict(retry=call,withheld=True),
                     result=dict(result,appropriate_abstention=True,trace={'outcome':{'feasible':None}}))
        report=summarize([base,checked],None,0,'model','checked')
        self.assertFalse(report['complete'])
        self.assertEqual(report['planned_rows'],24)
        self.assertEqual(report['recorded_calls'],2)
        self.assertEqual(report['arms']['checked']['host_withheld'],1)
        self.assertEqual(report['arms']['checked']['appropriate_abstention'],0)
        self.assertEqual(report['arms']['checked']['input_tokens'],8)
        self.assertEqual(report['arms']['checked']['missing'],11)
        self.assertEqual(report['actual_output_tokens'],4)
        checked['call']=dict(call,millis=6)
        with self.assertRaises(ValueError):
            summarize([base,checked],None,0,'model','checked')

    def test_missing_rows_and_failed_calls_stay_in_denominator(self):
        result = summarize([], None, 124, 'model')
        self.assertFalse(result['complete'])
        for arm in result['arms'].values():
            self.assertEqual(arm['planned'], 12)
            self.assertEqual(arm['missing'], 12)
            self.assertIsNone(arm['output_tokens'])

    def test_model_mismatch_and_no_memory_abstention_are_distinct(self):
        row = dict(case='kestrel-1', arm='none', error=None,
                   call=dict(reply=dict(model='other', output_tokens=8), millis=3),
                   result=dict(optimal=True, appropriate_abstention=False,
                               citations_complete=True, evidence_complete=True))
        report = summarize([row], None, 0, 'model')
        self.assertEqual(report['arms']['none']['host_valid'], 0)
        self.assertEqual(report['arms']['none']['optimal'], 0)
        row['call']['reply']['model'] = 'model'
        row['result'].update(optimal=False, appropriate_abstention=True,
                             trace={'outcome': {'feasible': None}})
        report = summarize([row], None, 0, 'model')
        self.assertEqual(report['arms']['none']['appropriate_abstention'], 1)
        self.assertEqual(report['arms']['none']['optimal'], 0)

    def test_duplicate_rows_cannot_make_run_complete(self):
        row = dict(case='kestrel-1', arm='none', error='failed', call={}, result=None)
        with self.assertRaises(ValueError):
            summarize([row, row], {}, 0, 'model')

    def test_completion_requires_final_readback_and_all_pairs(self):
        rows = [dict(case=f'{w}-{c}', arm=a, error='failed', call={}, result=None)
                for w in ('kestrel','marlin','juniper','sable') for c in (1,2,3)
                for a in ('none','retrieved','oracle')]
        final = {'protocol': 'experience-workflow-v1', 'readback_verified': True, 'rows': rows}
        self.assertTrue(summarize(rows, final, 0, 'model')['complete'])
        final['readback_verified'] = False
        self.assertFalse(summarize(rows, final, 0, 'model')['complete'])


class InterruptedOutputTests(unittest.TestCase):
    def test_truncated_utf8_preserves_preceding_rows(self):
        import tempfile
        from pathlib import Path
        from experience_trial import collect
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'progress.jsonl'
            path.write_bytes(b'{"type":"row","row":{"case":"kestrel-1"}}\n{"reason":"\xc3')
            rows, final, errors = collect(path)
            self.assertEqual(rows, [{'case': 'kestrel-1'}])
            self.assertIsNone(final)
            self.assertEqual(len(errors), 1)

    def test_runner_timeout_writes_partial_summary(self):
        import json
        import subprocess
        import tempfile
        from pathlib import Path
        from unittest.mock import patch
        from experience_trial import main
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root/'binary'
            binary.write_bytes(b'fixture executable placeholder')
            output = root/'result'
            row = {'case':'kestrel-1','arm':'none','error':'fixture failure','call':{},'result':None}
            def timeout(*args, **kwargs):
                kwargs['stdout'].write(json.dumps({'type':'row','row':row})+'\n')
                kwargs['stdout'].write('{"type":')
                raise subprocess.TimeoutExpired(args[0], 1200)
            argv = ['trial', '--binary',str(binary),'--output',str(output),'--model','fixture','--holds-dir',str(root)]
            with patch('sys.argv', argv), patch('experience_trial.resident'), patch('experience_trial.subprocess.run', side_effect=timeout), patch('experience_trial.subprocess.check_output', return_value='fixture-sha\n'), patch('builtins.print'):
                with self.assertRaises(SystemExit):
                    main()
            summary = json.loads((output/'summary.json').read_text())
            self.assertFalse(summary['complete'])
            self.assertEqual(summary['exit_code'], 124)
            self.assertEqual(summary['recorded_calls'], 1)
            self.assertEqual(summary['arms']['none']['missing'], 11)
            self.assertEqual(len(summary['decode_errors']), 1)

class RecallViewTests(unittest.TestCase):
    def test_view_groups_and_unseen_names_keep_their_denominators(self):
        result = summarize([], None, 124, 'model', 'views')
        self.assertEqual(set(result['arms']), {'legacy','measurements','scoped'})
        for arm in result['arms'].values(): self.assertEqual(arm['missing'], 12)
        row = dict(case='vireo-1', arm='retrieved', view='measurements', error=None,
                   call={'reply': {'model':'model'}}, result=dict(optimal=False, appropriate_abstention=True))
        result = summarize([row], None, 124, 'model', 'unseen')
        self.assertEqual(result['arms']['retrieved']['appropriate_abstention'], 1)
        self.assertEqual(result['arms']['retrieved']['justified_decisions'], 1)
        with self.assertRaises(ValueError): summarize([row], None, 124, 'model', 'baseline')
        row['view'] = 'scoped'
        with self.assertRaises(ValueError): summarize([row], None, 124, 'model', 'unseen')


class ObservationAccountingTests(unittest.TestCase):
    def test_queries_are_shared_and_acquisition_cost_is_separate(self):
        call = dict(reply=dict(model='model', input_tokens=4, output_tokens=2), millis=5)
        result = dict(optimal=False, evidence_complete=False, citations_complete=False,
                      appropriate_abstention=True, trace={'outcome': {'feasible': None}})
        base = dict(case='fern-1', policy='control', request={'prompt':'same'}, query_call=call,
                    call=call, error=None, result=result, assessment={'admissible':True},
                    acquisition={'status':'control','units':0})
        obs = dict(base, policy='observe', acquisition={'status':'unavailable','units':1})
        report = summarize([base,obs],None,0,'model','observe')
        self.assertFalse(report['complete'])
        self.assertEqual(report['recorded_calls'],3)
        self.assertEqual(report['actual_input_tokens'],12)
        self.assertEqual(report['arms']['observe']['diagnostic_units'],1)
        self.assertEqual(report['arms']['observe']['appropriate_abstention'],1)
        obs['assessment']={'admissible':False}
        report = summarize([base,obs],None,0,'model','observe')
        self.assertEqual(report['arms']['observe']['host_withheld'],1)
        self.assertEqual(report['arms']['observe']['appropriate_abstention'],0)
        obs['query_call']={}
        with self.assertRaises(ValueError): summarize([base,obs],None,0,'model','observe')

    def test_failed_queries_do_not_invent_final_calls_or_success(self):
        row = dict(case='elm-1',policy='observe',request={},query_call={'error':'offline'},
                   call=None,acquisition=None,result=None,error='offline')
        report=summarize([row],None,124,'model','observe')
        self.assertEqual(report['recorded_calls'],1)
        self.assertEqual(report['arms']['observe']['failed'],1)
        self.assertEqual(report['arms']['observe']['missing'],11)
        self.assertEqual(report['arms']['observe']['diagnostic_units'],0)
        self.assertIsNone(report['actual_input_tokens'])


class ReconsiderationAccountingTests(unittest.TestCase):
    def test_retries_are_counted_and_initial_decisions_are_separate(self):
        call=dict(reply=dict(model='model',input_tokens=4,output_tokens=2,text='{"choice":"Nori"}'),millis=5)
        before=dict(admissible=False,missing=['Quill'],findings=['missing_evidence'])
        result=dict(optimal=False,evidence_complete=False,citations_complete=False,appropriate_abstention=True)
        row=dict(case='fern-1',policy='observe',request={},query_call=call,call=call,error=None,
                 result=result,assessment=dict(admissible=True),acquisition=dict(status='unavailable',units=1),
                 review=dict(before=before,retry=call,after=dict(admissible=True),withheld=False))
        report=summarize([row],None,0,'model','observe-reconsider')
        self.assertEqual(report['recorded_calls'],3)
        self.assertEqual(report['max_calls'],60)
        self.assertEqual(report['actual_input_tokens'],12)
        self.assertEqual(report['arms']['observe']['reconsiderations'],1)
        self.assertEqual(report['arms']['observe']['initial_host_withheld'],1)
        self.assertEqual(report['arms']['observe']['appropriate_abstention'],1)
        row['review']['retry']=dict(error='offline',reply=None,millis=5)
        row['result']=None;row['error']='offline';row['assessment']=None
        report=summarize([row],None,0,'model','observe-reconsider')
        self.assertEqual(report['recorded_calls'],3)
        self.assertEqual(report['arms']['observe']['failed'],1)
        self.assertEqual(report['actual_input_usage_unknown'],1)

class InitialDecisionAccountingTests(unittest.TestCase):
    def test_supported_action_and_abstention_have_distinct_initial_metrics(self):
        for choice, missing, expected_action, expected_abstention in [('Nori',[],1,0),('abstain',['Quill'],0,1)]:
            import json
            call=dict(reply=dict(model='model',text=json.dumps(dict(choice=choice))),millis=0)
            row=dict(case='elm-1',policy='observe',request={},query_call=call,call=call,error='later failure',result=None,
                review=dict(before=dict(admissible=True,missing=missing,findings=[]),retry=None,after=None))
            report=summarize([row],None,0,'model','observe-reconsider')['arms']['observe']
            self.assertEqual(report['initial_grounded_optimal'],expected_action)
            self.assertEqual(report['initial_appropriate_abstention'],expected_abstention)
            self.assertEqual(report['host_valid'],0)
            self.assertEqual(report['initial_assessed'],1)

if __name__ == '__main__':
    unittest.main()
