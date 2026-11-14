import unittest
from extraction_trial import model_input, grade

class ExtractionTrialTests(unittest.TestCase):
    def test_gold_and_case_identity_never_enter_model_input(self):
        case={'id':'unsafe-tail','input':{'evidence':[],'plan':['Publish.']},'expected_coverage':'uncovered','rubric':'secret gold'}
        self.assertEqual(model_input(case),case['input'])
        self.assertNotIn('secret gold',str(model_input(case)))

    def test_matching_status_is_not_semantic_success(self):
        value={'extraction':{'protocol':'source-linked-extraction-v2','coverage':{'status':'covered'},'unresolved':[]},'calls':[{'model':'fixture','input_tokens':10,'output_tokens':5}]}
        row=grade(value,0,{'id':'x','expected_coverage':'covered'},'fixture')
        self.assertTrue(row['host_valid'])
        self.assertTrue(row['coverage_label_matches'])
        self.assertIsNone(row['faithful_extraction'])
        self.assertEqual(row['input_tokens'],10)

    def test_failure_and_model_mismatch_remain_failures(self):
        case={'id':'x','expected_coverage':'covered'}
        row=grade({'extraction':None,'error':'invalid source','calls':[]},1,case,'fixture')
        self.assertFalse(row['host_valid'])
        self.assertIsNone(row['input_tokens'])
        row=grade({'extraction':{},'calls':[{'model':'wrong'}]},0,case,'fixture')
        self.assertFalse(row['host_valid'])

    def test_malformed_report_stays_a_failed_row(self):
        value={'extraction':{'protocol':'source-linked-extraction-v2'},'calls':[{'model':'fixture'}]}
        row=grade(value,0,{'id':'x','expected_coverage':'covered'},'fixture')
        self.assertFalse(row['host_valid'])
        self.assertFalse(row['coverage_label_matches'])

if __name__=='__main__': unittest.main()
