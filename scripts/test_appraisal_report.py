#!/usr/bin/env python3
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("report", Path(__file__).with_name("appraisal-report.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def fixture():
    return dict(manifest=dict(kind="single", control="control", arms={"control":{},"guided":{}},
                              tasks={"ids":["budget"]}, seeds=[1,2], repetitions=1),
                trials=[dict(arm=arm,task="budget",seed=seed,repetition=1,status="done",
                             condition_hash=arm,passed=(arm=="guided"),stats={"turns":4,"step_reopens":0})
                        for arm in ["control","guided"] for seed in [1,2]],unreadable_trials=0)


class ReportControls(unittest.TestCase):
    def test_paired_outcomes_and_unknown_are_distinct_from_zero(self):
        result = module.report(fixture())
        self.assertTrue(result["complete"])
        self.assertEqual(result["paired_failure_delta"], -1)
        self.assertEqual(result["improved_pairs"], 2)
        self.assertEqual(result["metrics"]["step_reopens"]["guided_mean"], 0)
        self.assertIsNone(result["metrics"]["owner_actions"]["guided_mean"])
        self.assertEqual(result["metrics"]["owner_actions"]["missing_pairs"], 2)

    def test_incomplete_pairs_cannot_count_as_success_or_as_zero(self):
        data=fixture()
        data["trials"].pop()
        result=module.report(data)
        self.assertFalse(result["complete"])
        self.assertEqual(result["graded_pairs"], 1)
        self.assertEqual(result["ungraded_or_missing_pairs"], 1)
        data["trials"][0]["status"]="failed"
        result=module.report(data)
        self.assertEqual(result["graded_pairs"], 0)
        self.assertIsNone(result["paired_failure_delta"])
        self.assertEqual(result["arms"]["control"]["graded"], 1)
        self.assertEqual(result["arms"]["control"]["status_counts"]["failed"], 1)

    def test_missing_metrics_have_their_own_paired_denominator(self):
        data=fixture()
        data["trials"][0]["stats"]={}
        data["trials"][1]["stats"]["cost_usd"]=float("nan")
        result=module.report(data)
        self.assertEqual(result["metrics"]["turns"]["paired_n"], 1)
        self.assertEqual(result["metrics"]["cost_usd"]["paired_n"], 0)

    def test_duplicates_unplanned_trials_and_identical_conditions_refuse(self):
        for change in (lambda d:d["trials"].append(copy.deepcopy(d["trials"][0])),
                       lambda d:d["trials"][0].update(task="unknown"),
                       lambda d:d["trials"][0].update(condition_hash="guided")):
            data=fixture()
            change(data)
            with self.assertRaises(ValueError):
                module.report(data)

    def test_explicit_case_set_preserves_registered_pairing(self):
        data = fixture()
        data["manifest"]["tasks"]["ids"] = ["new-task"]
        for row in data["trials"]:
            row["task"] = "new-task"
        with self.assertRaises(ValueError):
            module.report(data)
        self.assertTrue(module.report(data, ["new-task"])["complete"])
        with self.assertRaises(ValueError):
            module.report(data, ["different-task"])

    def test_unreadable_rows_prevent_complete_claim(self):
        data=fixture()
        data["unreadable_trials"]=1
        self.assertFalse(module.report(data)["complete"])


if __name__ == "__main__":
    unittest.main()
