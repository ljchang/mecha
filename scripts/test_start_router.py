#!/usr/bin/env python3
"""scripts/start-router.sh's model lookup, against throwaway Hugging Face caches.

`python3 scripts/test_start_router.py`. Standard library only, like
test_model_idle.py beside it.

**Why this exists.** The router's presets are written from whatever the cache
holds, and the lookup only ever fails on a machine with the right cache shape:
the first version took "the first snapshot, then the file", and lost the
Qwen3.8 preset the day a second snapshot appeared; its replacement ended the
script silently under `set -euo pipefail` whenever a file was absent. Each
case below builds a cache of that shape and reads the generated models.ini.
The weights are empty files: nothing is loaded, because `LLAMA_SERVER` is a
stub that prints its arguments.
"""
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("start-router.sh")

PROD = "unsloth--Qwen3.6-35B-A3B-MTP-GGUF"
PROD_FILE = "Qwen3.6-35B-A3B-UD-Q4_K_M.gguf"
Q38 = "unsloth--Qwen3.8-27B-GGUF"


class Cache:
    """A throwaway hub: `put(repo, rev, name)` makes snapshots/<rev>/<name> a
    symlink into blobs/, the way huggingface_hub lays files out."""

    def __init__(self, root):
        self.root = Path(root)

    def put(self, repo, rev, name, dangling=False, age=0):
        blobs = self.root / f"models--{repo}" / "blobs"
        snap = self.root / f"models--{repo}" / "snapshots" / rev
        blobs.mkdir(parents=True, exist_ok=True)
        snap.mkdir(parents=True, exist_ok=True)
        blob = blobs / f"{rev}-{name}"
        if not dangling:
            blob.write_bytes(b"")
        link = snap / name
        link.symlink_to(os.path.relpath(blob, snap))
        stamp = time.time() - age
        os.utime(link, (stamp, stamp), follow_symlinks=False)
        return link

    def production(self):
        self.put(PROD, "r1", PROD_FILE)
        self.put(PROD, "r1", "mmproj-BF16.gguf")


class StartRouter(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.cache = Cache(base / "hub with a space")
        self.cache.root.mkdir()
        self.runtime = base / "run"
        self.runtime.mkdir()
        self.stub = base / "llama-server"
        self.stub.write_text("#!/bin/sh\necho started \"$@\"\n")
        self.stub.chmod(0o755)

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self):
        env = dict(
            os.environ,
            HF_HUB=str(self.cache.root),
            XDG_RUNTIME_DIR=str(self.runtime),
            LLAMA_SERVER=str(self.stub),
        )
        out = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=30)
        ini = self.runtime / "mecha-router" / "models.ini"
        return out.returncode, out.stdout, out.stderr, ini.read_text() if ini.exists() else ""

    def section(self, ini, name):
        """The key = value lines of one preset section."""
        lines, inside = {}, False
        for line in ini.splitlines():
            if line.startswith("["):
                inside = line == f"[{name}]"
            elif inside and " = " in line:
                k, v = line.split(" = ", 1)
                lines[k] = v
        return lines

    def test_an_empty_cache_stops_with_the_download_line(self):
        code, out, err, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("production model missing: hf download", err)
        self.assertNotIn("started", out)

    def test_a_missing_optional_model_is_skipped_with_a_warning_not_a_silent_exit(self):
        # Under set -euo pipefail, a lookup that fails on an absent file used
        # to end the script here with no message at all.
        self.cache.production()
        code, out, err, ini = self.run_script()
        self.assertEqual(code, 0, err)
        self.assertIn("started", out)
        self.assertIn("skipping qwen3.8-27b", err)
        self.assertIn("[qwen3.6-35b-a3b]", ini)

    def test_a_file_in_an_older_snapshot_is_found_when_a_newer_one_exists(self):
        # The day this broke: the Q4_K_M and its projector in one snapshot, a
        # newer file fetched into a second. First-snapshot lookup lost both.
        self.cache.production()
        self.cache.put(Q38, "old", "Qwen3.8-27B-Q4_K_M.gguf", age=100)
        self.cache.put(Q38, "old", "mmproj-BF16.gguf", age=100)
        self.cache.put(Q38, "new", "some-other-file.gguf")
        code, _, err, ini = self.run_script()
        self.assertEqual(code, 0, err)
        preset = self.section(ini, "qwen3.8-27b")
        self.assertTrue(preset.get("model", "").endswith("/old/Qwen3.8-27B-Q4_K_M.gguf"), preset)
        self.assertTrue(preset.get("mmproj", "").endswith("/old/mmproj-BF16.gguf"), preset)
        self.assertIn("serving the withdrawn Q4_K_M", err)

    def test_the_newest_copy_of_a_file_wins(self):
        self.cache.production()
        self.cache.put(Q38, "a", "Qwen3.8-27B-UD-Q4_K_XL.gguf", age=500)
        self.cache.put(Q38, "b", "Qwen3.8-27B-UD-Q4_K_XL.gguf", age=0)
        self.cache.put(Q38, "a", "mmproj-BF16.gguf")
        _, _, err, ini = self.run_script()
        self.assertTrue(self.section(ini, "qwen3.8-27b")["model"].endswith("/b/Qwen3.8-27B-UD-Q4_K_XL.gguf"), err)

    def test_a_dangling_snapshot_entry_is_not_a_model(self):
        # A pruned blob leaves the symlink behind; `ls` still lists it.
        self.cache.put(PROD, "r1", PROD_FILE, dangling=True)
        self.cache.put(PROD, "r1", "mmproj-BF16.gguf")
        code, _, err, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("production model missing", err)

    def test_an_f16_projector_is_used_when_there_is_no_bf16(self):
        self.cache.put(PROD, "r1", PROD_FILE)
        self.cache.put(PROD, "r1", "mmproj-F16.gguf")
        code, _, err, ini = self.run_script()
        self.assertEqual(code, 0, err)
        self.assertTrue(self.section(ini, "qwen3.6-35b-a3b")["mmproj"].endswith("mmproj-F16.gguf"))

    def test_no_projector_prints_a_fetch_line_into_a_real_snapshot(self):
        self.cache.put(PROD, "r1", PROD_FILE)
        code, _, err, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("vision tower is not on disk", err)
        self.assertIn("/snapshots/r1/", err, "the fetch line must name the snapshot that holds the weights")

    def test_gemmas_draft_comes_from_its_weights_snapshot(self):
        g = "unsloth--gemma-4-26B-A4B-it-GGUF"
        self.cache.production()
        self.cache.put(g, "w", "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf", age=100)
        self.cache.put(g, "w", "mmproj-BF16.gguf", age=100)
        self.cache.put(g, "other", "mtp-gemma-4-26B-A4B-it.gguf")
        _, _, err, ini = self.run_script()
        self.assertNotIn("[gemma-4-26b-a4b]", ini, "a draft from another revision must not be paired")
        self.assertIn("skipping gemma-4-26b-a4b", err)
        self.cache.put(g, "w", "mtp-gemma-4-26B-A4B-it.gguf")
        _, _, err, ini = self.run_script()
        self.assertTrue(self.section(ini, "gemma-4-26b-a4b")["model-draft"].endswith("/w/mtp-gemma-4-26B-A4B-it.gguf"), err)

    def test_a_matched_gemma_pair_in_an_older_snapshot_survives_newer_weights(self):
        # Weights re-fetched into a newer snapshot must not orphan the matched
        # weights + draft pair still in the older one.
        g = "unsloth--gemma-4-26B-A4B-it-GGUF"
        self.cache.production()
        self.cache.put(g, "old", "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf", age=100)
        self.cache.put(g, "old", "mtp-gemma-4-26B-A4B-it.gguf", age=100)
        self.cache.put(g, "old", "mmproj-BF16.gguf", age=100)
        self.cache.put(g, "new", "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf", age=0)
        _, _, err, ini = self.run_script()
        preset = self.section(ini, "gemma-4-26b-a4b")
        self.assertTrue(preset.get("model", "").endswith("/old/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf"), err)
        self.assertTrue(preset.get("model-draft", "").endswith("/old/mtp-gemma-4-26B-A4B-it.gguf"), err)

    def test_the_two_uncensored_qwen38_builds_get_their_own_presets(self):
        hh = "HauhauCS--Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF"
        hu = "huihui-ai--Huihui-Qwen3.8-27B-abliterated-GGUF"
        self.cache.production()
        self.cache.put(hh, "r", "Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-Q4_K_P.gguf")
        self.cache.put(hh, "r", "mmproj-Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-BF16.gguf")
        self.cache.put(hu, "r", "Huihui-Qwen3.8-27B-abliterated-UD-Q4_K_XL.gguf")
        self.cache.put(hu, "r", "mmproj-model-bf16.gguf")
        code, _, err, ini = self.run_script()
        self.assertEqual(code, 0, err)
        for name, model, proj in [
            ("qwen3.8-27b-uncensored", "Aggressive-Q4_K_P.gguf", "Aggressive-BF16.gguf"),
            ("qwen3.8-27b-abliterated", "abliterated-UD-Q4_K_XL.gguf", "mmproj-model-bf16.gguf"),
        ]:
            preset = self.section(ini, name)
            self.assertTrue(preset.get("model", "").endswith(model), (name, preset))
            self.assertTrue(preset.get("mmproj", "").endswith(proj), (name, preset))
            self.assertEqual(preset.get("spec-type"), "draft-mtp", name)
            self.assertEqual(preset.get("temp"), "1.0", name)

if __name__ == "__main__":
    unittest.main()
