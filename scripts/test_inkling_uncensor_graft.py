#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["gguf==0.19.0", "numpy==2.5.3"]
# ///
"""scripts/inkling-uncensor-graft.py, run on synthetic files.

`uv run scripts/test_inkling_uncensor_graft.py`, under the same pins as the
script, because the graft's one claim is that gguf's Q8_0 re-encodes a tensor
to the bytes the base already holds. A gguf version that changed its rounding
would make every real run refuse, and this is where that shows first.

The base is a three-layer `inkling2_loop` GGUF whose `attn_output` tensors are
Q8_0, plus one F32 tensor that must come through untouched. The edited file is
a safetensors holding each layer's `attn.wo_ud` in BF16. Every value is chosen
to be exactly representable in BF16, so an unedited layer reproduces the
base's bytes for real rather than by construction.
"""
import json
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
from gguf import GGMLQuantizationType, GGUFReader, GGUFValueType, GGUFWriter, quants

GRAFT = Path(__file__).with_name("inkling-uncensor-graft.py")
ROWS, COLS = 4, 64  # out, in: Q8_0 blocks are 32 wide


def bf16_exact(seed):
    """F32 values whose low 16 bits are zero, so BF16 holds them exactly."""
    x = np.random.default_rng(seed).standard_normal((ROWS, COLS)).astype(np.float32)
    return (x.view(np.uint32) & 0xFFFF0000).view(np.float32)


def write_base(path, layers, arch="inkling2_loop", qtype=GGMLQuantizationType.Q8_0, cols=COLS):
    w = GGUFWriter(str(path), arch=arch)
    w.add_key_value(f"{arch}.block_count", len(layers), GGUFValueType.UINT32)
    for i, x in enumerate(layers):
        if cols != COLS:
            x = np.zeros((ROWS, cols), dtype=np.float32)
        if qtype == GGMLQuantizationType.F32:
            w.add_tensor(f"blk.{i}.attn_output.weight", x)
        else:
            w.add_tensor(f"blk.{i}.attn_output.weight", quants.quantize(x, qtype), raw_dtype=qtype)
    w.add_tensor("output_norm.weight", np.arange(COLS, dtype=np.float32))
    w.write_header_to_file()
    w.write_kv_data_to_file()
    w.write_tensors_to_file()
    w.close()


def write_edited(path, layers, dtype="BF16"):
    """A safetensors of `model.llm.layers.<n>.attn.wo_ud.weight`, written by hand."""
    meta, blobs, off = {}, [], 0
    for i, x in layers.items():
        raw = (x.view(np.uint32) >> 16).astype("<u2").tobytes()
        meta[f"model.llm.layers.{i}.attn.wo_ud.weight"] = {
            "dtype": dtype, "shape": [ROWS, COLS], "data_offsets": [off, off + len(raw)]}
        blobs.append(raw)
        off += len(raw)
    meta["model.llm.layers.0.attn.wq.weight"] = {"dtype": "BF16", "shape": [1], "data_offsets": [off, off + 2]}
    blobs.append(b"\0\0")
    j = json.dumps(meta).encode()
    j += b" " * (-len(j) % 8)
    with open(path, "wb") as f:
        f.write(struct.pack("<Q", len(j)) + j + b"".join(blobs))


def run(*args):
    return subprocess.run([sys.executable, str(GRAFT), *map(str, args)], capture_output=True, text=True)


class Graft(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = Path(self.tmp.name)
        self.orig = [bf16_exact(i) for i in range(3)]
        self.base = self.d / "base.gguf"
        self.out = self.d / "out.gguf"
        write_base(self.base, self.orig)

    def tearDown(self):
        self.tmp.cleanup()

    def edited(self, layers, **kw):
        p = self.d / "edited.safetensors"
        write_edited(p, layers, **kw)
        return p

    def refused(self, r, words):
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn(words, r.stderr)
        self.assertFalse(self.out.exists())
        self.assertFalse(Path(str(self.out) + ".partial").exists())

    def test_grafts_only_the_edited_layers(self):
        new = bf16_exact(99)
        r = run(self.base, self.edited({0: self.orig[0], 1: new, 2: self.orig[2]}), self.out)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("reproduced 2 unedited layers exactly (0, 2); grafted 1 (1)", r.stdout)
        base, out = self.base.read_bytes(), self.out.read_bytes()
        self.assertEqual(len(base), len(out))
        got = {t.name: bytes(t.data.tobytes()) for t in GGUFReader(self.out).tensors}
        want = {t.name: bytes(t.data.tobytes()) for t in GGUFReader(self.base).tensors}
        self.assertEqual(got["blk.1.attn_output.weight"],
                         quants.quantize(new, GGMLQuantizationType.Q8_0).tobytes())
        for name in want:
            if name != "blk.1.attn_output.weight":
                self.assertEqual(got[name], want[name], name)
        # Everything outside the patched tensor — header, metadata, other
        # tensors, padding — is the base's, byte for byte.
        t = next(t for t in GGUFReader(self.base).tensors if t.name == "blk.1.attn_output.weight")
        lo, hi = t.data_offset, t.data_offset + t.n_bytes
        self.assertEqual(out[:lo], base[:lo])
        self.assertEqual(out[hi:], base[hi:])

    def test_refuses_when_no_layer_reproduces_the_base(self):
        # A different base or quantizer: nothing proves the re-encoding matches.
        r = run(self.base, self.edited({i: bf16_exact(50 + i) for i in range(3)}), self.out)
        self.refused(r, "no edited tensor reproduces")

    def test_refuses_when_nothing_differs(self):
        r = run(self.base, self.edited(dict(enumerate(self.orig))), self.out)
        self.refused(r, "nothing to graft")

    def test_refuses_another_architecture(self):
        write_base(self.base, self.orig, arch="inkling2")
        r = run(self.base, self.edited({1: bf16_exact(99)}), self.out)
        self.refused(r, "not inkling2_loop")

    def test_refuses_a_base_whose_attn_output_is_not_q8_0(self):
        write_base(self.base, self.orig, qtype=GGMLQuantizationType.F32)
        r = run(self.base, self.edited({1: bf16_exact(99)}), self.out)
        self.refused(r, "only Q8_0 re-encodes exactly")

    def test_refuses_a_shape_mismatch(self):
        write_base(self.base, self.orig, cols=2 * COLS)
        r = run(self.base, self.edited({1: bf16_exact(99)}), self.out)
        self.refused(r, "not the same model")

    def test_refuses_an_edited_file_that_is_not_bf16(self):
        r = run(self.base, self.edited({1: bf16_exact(99)}, dtype="F16"), self.out)
        self.refused(r, "expected BF16")

    def test_refuses_an_edited_file_without_wo_ud(self):
        r = run(self.base, self.edited({}), self.out)
        self.refused(r, "no model.llm.layers.<n>.attn.wo_ud.weight tensors")


if __name__ == "__main__":
    unittest.main()
