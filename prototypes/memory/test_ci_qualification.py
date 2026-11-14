"""Regression checks for CI's nonempty, fully executed qualification gate."""
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import ci_qualification as ci


class CiQualificationTests(unittest.TestCase):
    def test_missing_qualification_is_a_failure(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(ValueError, 'missing qualification'):
                ci.run_suite()

    def test_empty_discovery_is_a_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, dict.fromkeys(ci.QUALIFICATION, 'explicit')):
                with self.assertRaisesRegex(ValueError, 'no tests'):
                    ci.run_suite(Path(directory))

    def test_skip_fails_even_when_unittest_reports_success(self):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, 'test_ci_skip_fixture.py').write_text(
                'import unittest\n'
                'class GateFixture(unittest.TestCase):\n'
                '    @unittest.skip("qualification unavailable")\n'
                '    def test_unavailable(self): pass\n')
            with patch.dict(os.environ, dict.fromkeys(ci.QUALIFICATION, 'explicit')):
                self.assertFalse(ci.run_suite(Path(directory), stream=io.StringIO()))


if __name__ == '__main__':
    unittest.main()
