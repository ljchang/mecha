"""Exercise scoring with known failures and repetition, independent of a model."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("gossip_report", ROOT / "scripts/gossip-comparison.py")
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name)
        report.write(self.out / "finish.json", dict(server_matches=True, input_hashes_match=True))
        self.verdicts = [dict(id="good", label="supported", facts=["f1"], reason="source entails"),
                         dict(id="bad", label="contradicted", facts=[], reason="wrong person")]
        report.write(self.out / "audit-verdicts.json", self.verdicts)
        self.claims = {name: dict(reference_facts=[dict(id="f1")]) for name in ["good", "bad"]}
        self.trials = []
        for case in range(4):
            for seed in [1, 2]:
                for mode in ["own_evidence", "peer"]:
                    rows = [dict(round=n, claims=["good"] + (["bad"] if mode == "peer" and n==3 else []),
                                 abstentions=0, ungrounded_answers=0, stalls=0) for n in [1,2,3]]
                    self.trials.append(dict(case=str(case), seed=seed, mode=mode, problems=[], rounds=rows,
                                            requests=16, searches=6, input_tokens=100, output_tokens=50))

    def score(self):
        with patch.object(report, "audit", return_value=(self.trials, self.claims)):
            report.report(self.out)
        return json.loads((self.out / "scorecard.json").read_text())

    def test_repetition_adds_no_new_fact_and_contradictions_remain_visible(self):
        score = self.score()
        self.assertTrue(score["complete"])
        for mode in ["own_evidence", "peer"]:
            rounds = score["summary"][mode]["rounds"]
            self.assertEqual([r["new_supported_facts"] for r in rounds], [8,0,0])
            self.assertEqual([r["cumulative_supported_facts"] for r in rounds], [8,8,8])
        self.assertEqual(score["summary"]["peer"]["rounds"][2]["contradicted"], 8)
        self.assertTrue(all(p["control_facts"] == p["peer_facts"] == 1 for p in score["paired"]))

    def test_missing_judgment_and_wrong_reference_do_not_score(self):
        report.write(self.out / "audit-verdicts.json", self.verdicts + [self.verdicts[0]])
        with self.assertRaises(ValueError): self.score()
        report.write(self.out / "audit-verdicts.json", self.verdicts[:1])
        with self.assertRaises(ValueError): self.score()
        self.verdicts[0]["facts"] = ["invented"]
        report.write(self.out / "audit-verdicts.json", self.verdicts)
        with self.assertRaises(ValueError): self.score()

    def test_failed_trial_and_changed_execution_condition_are_incomplete(self):
        self.trials[0]["problems"] = ["missing response"]
        self.assertFalse(self.score()["complete"])
        self.trials[0]["problems"] = []
        report.write(self.out / "finish.json", dict(server_matches=False, input_hashes_match=True))
        self.assertFalse(self.score()["complete"])


if __name__ == "__main__": unittest.main()
