import copy
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gossip_fixture", HERE / "server.py")
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.case = json.loads((HERE / "cases.json").read_text())[0]
        self.args = dict(query="Mara Vale: responsibilities", sources=["slack"], since="2026-01-01",
                         until="2026-09-10", scope="evidence_only", probe=True, include_private=True, k=10)

    def test_query_changes_real_retrieval_without_disclosing_gold(self):
        first = server.search(self.case, self.args)
        self.args["query"] = "Mara Vale: Cedar inventory"
        next = server.search(self.case, self.args)
        self.assertNotEqual(first["items"][0]["id"], next["items"][0]["id"])
        self.assertIn("Cedar inventory", next["items"][0]["text"])
        self.assertEqual(len(next["items"]), 2)
        self.assertNotIn("gold", json.dumps(next))
        self.assertTrue(all(e["source"] == "slack" for e in next["items"]))
        self.args["k"] = 1
        self.assertEqual(len(server.search(self.case, self.args)["items"]), 1)

    def test_boundary_violations_fail_instead_of_widening(self):
        for key, value in [("sources", ["slack", "bee"]), ("probe", False),
                           ("query", "another person"), ("scope", "all"), ("since", "1900-01-01")]:
            args = copy.deepcopy(self.args)
            args[key] = value
            with self.assertRaises(ValueError): server.search(self.case, args)

    def test_search_does_not_depend_on_gold_or_query_history(self):
        before = server.search(self.case, self.args)
        self.case["gold"] = [{"statement": "arbitrary replacement"}]
        server.search(self.case, dict(self.args, query="Mara Vale: Juniper checklist"))
        self.assertEqual(before, server.search(self.case, self.args))


if __name__ == "__main__": unittest.main()
