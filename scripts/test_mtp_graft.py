#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["gguf==0.19.0", "numpy==2.5.3"]
# ///
"""scripts/mtp-graft.py, round-tripped on synthetic GGUFs.

`uv run scripts/test_mtp_graft.py`, under the same pins as the script, because
the graft leans on gguf's reader/writer internals: this is what notices a
version that moved them, the day it moves rather than the day a re-upload
needs a rebuild.

A two-layer base and a three-layer donor with `nextn_predict_layers = 1` stand
in for HauhauCS and unsloth. The donor's shared tensors hold different bytes
from the base's, so "BASE byte for byte" is a measured claim, not a name check.
"""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
from gguf import GGUFReader, GGUFValueType, GGUFWriter

GRAFT = Path(__file__).with_name("mtp-graft.py")
ARCH = "qwen35moe"


def write(path, blocks, nextn=None, fill=0.0, tensors=None, arch=ARCH, embd_rows=8, split=False):
    w = GGUFWriter(str(path), arch=arch)
    w.add_key_value(f"{arch}.block_count", blocks, GGUFValueType.UINT32)
    if split:
        w.add_key_value("split.no", 0, GGUFValueType.UINT16)
        w.add_key_value("split.count", 2, GGUFValueType.UINT16)
    w.add_key_value("general.name", f"fill-{fill}", GGUFValueType.STRING)
    if nextn is not None:
        w.add_key_value(f"{arch}.nextn_predict_layers", nextn, GGUFValueType.UINT32)
    names = tensors if tensors is not None else (
        [f"blk.{i}.attn_q.weight" for i in range(blocks)] + ["token_embd.weight", "output.weight"])
    for i, name in enumerate(names):
        rows = embd_rows if name in ("token_embd.weight", "output.weight") else 4
        w.add_tensor(name, np.full((rows, 4), fill + i, dtype=np.float32))
    w.write_header_to_file()
    w.write_kv_data_to_file()
    w.write_tensors_to_file()
    w.close()


class Graft(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.base, self.donor, self.out = self.dir / "base.gguf", self.dir / "donor.gguf", self.dir / "out.gguf"

    def tearDown(self):
        self.tmp.cleanup()

    def graft(self):
        return subprocess.run([sys.executable, str(GRAFT), str(self.base), str(self.donor), str(self.out)],
                              capture_output=True, text=True, timeout=60)

    def refused(self, why):
        r = self.graft()
        self.assertNotEqual(r.returncode, 0, why)
        self.assertFalse(self.out.exists(), "a refusal must leave nothing under the name")
        self.assertFalse(Path(str(self.out) + ".partial").exists())
        return r.stderr

    def test_the_head_is_added_and_the_base_is_kept_byte_for_byte(self):
        write(self.base, 2, fill=0.0)
        write(self.donor, 3, nextn=1, fill=100.0,
              tensors=["blk.0.attn_q.weight", "blk.1.attn_q.weight", "blk.2.attn_q.weight",
                       "blk.2.nextn.eh_proj.weight", "token_embd.weight", "output.weight"])
        r = self.graft()
        self.assertEqual(r.returncode, 0, r.stderr)
        out, base, donor = GGUFReader(self.out), GGUFReader(self.base), GGUFReader(self.donor)
        self.assertEqual(out.fields[f"{ARCH}.block_count"].contents(), 3)
        self.assertEqual(out.fields[f"{ARCH}.nextn_predict_layers"].contents(), 1)
        self.assertEqual(out.fields["general.name"].contents(), "fill-0.0", "the base's keys, not the donor's")
        got = {t.name: t for t in out.tensors}
        self.assertEqual(set(got), {t.name for t in base.tensors} | {"blk.2.attn_q.weight", "blk.2.nextn.eh_proj.weight"})
        for t in base.tensors:
            self.assertTrue(np.array_equal(got[t.name].data, t.data), t.name)
        for t in donor.tensors:
            if t.name.startswith("blk.2."):
                self.assertTrue(np.array_equal(got[t.name].data, t.data), t.name)
        self.assertFalse(Path(str(self.out) + ".partial").exists())

    def test_a_base_that_already_has_a_head_is_refused(self):
        write(self.base, 3, nextn=1)
        write(self.donor, 3, nextn=1)
        self.assertIn("already declares", self.refused("nothing to graft"))

    def test_a_donor_without_a_head_is_refused(self):
        write(self.base, 2)
        write(self.donor, 3)
        self.assertIn("no MTP head", self.refused("nothing to give"))

    def test_block_counts_that_do_not_line_up_are_refused(self):
        write(self.base, 2)
        write(self.donor, 4, nextn=1)
        self.assertIn("do not line up", self.refused("the head would land on the wrong layer"))

    def test_a_shard_of_a_split_file_is_refused_on_either_side(self):
        # One shard passes every other check and would yield a tensor subset
        # still naming siblings it no longer has.
        for base_split in (True, False):
            with self.subTest(base_split=base_split):
                write(self.base, 2, split=base_split)
                write(self.donor, 3, nextn=1, split=not base_split)
                self.assertIn("split GGUF", self.refused("a shard is not a model"))

    def test_the_pins_here_are_the_scripts_pins(self):
        # The graft runs under *this* file's environment, so a drifted header
        # would test one gguf and build with another.
        deps = lambda p: [l for l in p.read_text().splitlines() if l.startswith("# dependencies")]
        self.assertEqual(deps(Path(__file__)), deps(GRAFT))
        self.assertTrue(deps(GRAFT), "the script must pin its dependencies")

    def test_a_donor_of_another_model_is_refused(self):
        write(self.base, 2)
        write(self.donor, 3, nextn=1, embd_rows=16)
        self.assertIn("not the same model", self.refused("a head over another vocabulary mis-drafts"))


if __name__ == "__main__":
    unittest.main()
