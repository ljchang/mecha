"""Trial isolation checks; no model server or private store required."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import planning_trial as trial


class PlanningTrialTests(unittest.TestCase):
    def test_inherited_provider_overrides_do_not_reach_trial_commands(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / 'mecha'; binary.touch()
            captured = {}
            class Captured(Exception):
                pass
            def command(binary, args, env, cwd, timeout=240):
                captured.update(env)
                raise Captured()
            argv = ['planning_trial.py', '--binary', str(binary), '--output', str(root/'run'),
                    '--model', 'resident-fixture', '--holds-dir', str(root), '--context-window', '8192']
            with patch('sys.argv', argv), patch.object(trial, 'resident'), patch.object(trial, 'command', command), patch.dict('os.environ', {'MECHA_PROVIDER':'local','MECHA_MODEL':'other','MECHA_EFFORT':'max'}):
                with self.assertRaises(Captured):
                    trial.main()
            for key in ['MECHA_PROVIDER','MECHA_MODEL','MECHA_EFFORT']:
                self.assertFalse(key in captured, key)

    def test_model_mismatch_is_rejected_before_scoring(self):
        with self.assertRaisesRegex(RuntimeError, 'model'):
            trial.require_model({'model':'different-model'}, 'resident-fixture')
        with self.assertRaisesRegex(RuntimeError, 'model'):
            trial.require_model({}, 'resident-fixture')
        trial.require_model({'model':'resident-fixture'}, 'resident-fixture')


if __name__ == '__main__':
    unittest.main()
