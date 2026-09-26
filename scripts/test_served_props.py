#!/usr/bin/env python3
"""scripts/served-props.sh against a stub llama-server, single-model and router.

`python3 scripts/test_served_props.py`. Standard library only, like
test_model_idle.py beside it.

**Why this exists.** The helper answers "what model is this server serving,
and what are its props" for the benchmark and replay scripts. Behind a router
the honest answer takes three requests and one of them must say
`autoload=false`, or asking would load a model — so each case below checks
the requests actually sent, not only the answer.
"""
import http.server
import json
import subprocess
import threading
import unittest
import urllib.parse
from pathlib import Path

HELPER = Path(__file__).with_name("served-props.sh")
SEEN = []

SINGLE = {"model_alias": "qwen3.6-35b-a3b", "total_slots": 4}
PLACEHOLDER = {"role": "router", "model_alias": "llama-server", "default_generation_settings": {"n_ctx": 0}}
CHILD = {"model_alias": "gemma-4-26b-a4b", "total_slots": 1}


def models(*pairs):
    return {"data": [{"id": i, "status": {"value": v}} for i, v in pairs]}


# prefix -> (/models body, the model whose props the router serves)
ROUTERS = {
    "/r": (models(("qwen3.6-35b-a3b", "unloaded"), ("gemma-4-26b-a4b", "loaded")), "gemma-4-26b-a4b"),
    "/r-none": (models(("gemma-4-26b-a4b", "unloaded")), None),
    "/r-renamed": (models(("gemma-4-26b-a4b", "resident")), "gemma-4-26b-a4b"),
    # Mid-eviction: the outgoing model sleeping, the incoming one loading.
    "/r-evicting": (models(("qwen3.6-35b-a3b", "sleeping"), ("gemma-4-26b-a4b", "loading")), None),
}


class Stub(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        SEEN.append(self.path)
        url = urllib.parse.urlsplit(self.path)
        code, body = 404, {}
        if url.path == "/single/props":
            code, body = 200, SINGLE
        elif url.path == "/nameless/props":
            code, body = 200, {"total_slots": 1}
        for prefix, (listing, serving) in ROUTERS.items():
            if url.path == f"{prefix}/props":
                q = urllib.parse.parse_qs(url.query)
                if not q:
                    code, body = 200, PLACEHOLDER
                elif q.get("autoload") != ["false"]:
                    code, body = 500, {"error": "this read would have loaded a model"}
                elif q.get("model") == [serving]:
                    code, body = 200, CHILD
                else:
                    code, body = 400, {"error": {"message": "model is not loaded"}}
            elif url.path == f"{prefix}/models":
                code, body = 200, listing
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


class ServedProps(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Stub)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def call(self, *args):
        SEEN.clear()
        quoted = " ".join(f"'{a}'" for a in args)
        out = subprocess.run(
            ["bash", "-c", f"source '{HELPER}'; {quoted}"],
            capture_output=True, text=True, timeout=30,
        )
        return out.returncode, out.stdout, out.stderr

    def test_a_single_model_server_answers_with_its_own_props(self):
        code, out, _ = self.call("served_props", f"{self.base}/single")
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["total_slots"], 4)
        self.assertEqual(SEEN, ["/single/props"])

    def test_a_v1_spelling_is_the_server_root(self):
        code, out, _ = self.call("served_model", f"{self.base}/single/v1")
        self.assertEqual((code, out.strip()), (0, "qwen3.6-35b-a3b"))
        self.assertEqual(SEEN, ["/single/props"])

    def test_a_router_answers_with_the_resident_models_props_without_loading_it(self):
        code, out, _ = self.call("served_props", f"{self.base}/r")
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["model_alias"], "gemma-4-26b-a4b")
        asked = [p for p in SEEN if "model=" in p]
        self.assertEqual(len(asked), 1, SEEN)
        self.assertIn("autoload=false", asked[0])

    def test_a_named_model_that_is_not_loaded_is_a_refusal_not_a_load(self):
        code, _, err = self.call("served_props", f"{self.base}/r", "qwen3.6-35b-a3b")
        self.assertNotEqual(code, 0)
        self.assertIn("mecha model use qwen3.6-35b-a3b", err)

    def test_the_resident_model_is_named_only_from_a_list_it_fully_reads(self):
        self.assertEqual(self.call("served_model", f"{self.base}/r")[1].strip(), "gemma-4-26b-a4b")
        for prefix in ["/r-none", "/r-renamed", "/r-evicting"]:
            with self.subTest(prefix=prefix):
                code, out, _ = self.call("served_model", f"{self.base}{prefix}")
                self.assertNotEqual(code, 0)
                self.assertEqual(out.strip(), "")

    def test_a_single_server_that_names_no_model_is_not_an_answer(self):
        code, out, err = self.call("served_model", f"{self.base}/nameless")
        self.assertNotEqual(code, 0)
        self.assertEqual(out.strip(), "")
        self.assertIn("names no model_alias", err)


if __name__ == "__main__":
    unittest.main()
