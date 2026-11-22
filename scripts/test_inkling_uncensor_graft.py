#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["gguf==0.19.0", "numpy==2.5.3"]
# ///
"""scripts/inkling-uncensor-graft.py, run on synthetic files.

`uv run scripts/test_inkling_uncensor_graft.py`, under the same pins as the
script (`test_the_pins_here_are_the_scripts_pins`), because the graft's one
claim is that gguf's Q8_0 re-encodes a tensor to the bytes the base already
holds. A gguf version that changed its rounding would make every real run
refuse; `test_gguf_s_q8_0_is_ggml_s` is where that shows first, since every
other case encodes both sides with gguf and would move with it.

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

    def test_refuses_a_gguf_that_names_no_architecture(self):
        w = GGUFWriter(str(self.base), arch="inkling2_loop")
        w.add_tensor("x", np.zeros(32, dtype=np.float32))
        w.write_header_to_file()
        w.write_kv_data_to_file()
        w.write_tensors_to_file()
        w.close()
        # GGUFWriter always writes general.architecture; drop it by rewriting
        # the key's name to one the script does not read.
        raw = self.base.read_bytes().replace(b"general.architecture", b"general.architecturX")
        self.base.write_bytes(raw)
        r = run(self.base, self.edited({1: bf16_exact(99)}), self.out)
        self.refused(r, "architecture unnamed, not inkling2_loop")

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

    def test_refuses_to_write_over_the_base(self):
        r = run(self.base, self.edited({1: bf16_exact(99)}), self.base)
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is BASE itself", r.stderr)
        e = self.edited({1: bf16_exact(99)})
        r = run(self.base, e, e)
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is EDITED itself", r.stderr)
        want = [bytes(t.data.tobytes()) for t in GGUFReader(self.base).tensors]
        write_base(self.d / "again.gguf", self.orig)
        self.assertEqual(want, [bytes(t.data.tobytes()) for t in GGUFReader(self.d / "again.gguf").tensors])

    def test_gguf_s_q8_0_is_ggml_s(self):
        # Every case above encodes both sides with gguf, so a change in gguf's
        # rounding would move them together. This holds gguf to ggml's own
        # quantize_row_q8_0_ref — the quantizer that wrote the real base: d =
        # amax / 127 stored as f16, each value roundf(x / d), ties away from
        # zero — including exact ties, which numpy's round would send to even.
        def ggml(x):
            b = x.reshape(-1, 32).astype(np.float32)
            d = np.abs(b).max(axis=1, keepdims=True) / 127
            with np.errstate(divide="ignore"):
                inv = np.where(d != 0, 1.0 / d, 0.0).astype(np.float32)
            v = b * inv
            q = (np.sign(v) * np.floor(np.abs(v) + 0.5)).astype(np.int8)
            return b"".join(di.tobytes() + qi.tobytes() for di, qi in zip(d.astype(np.float16), q))
        ties = np.zeros((1, 32), dtype=np.float32)
        ties[0, 0] = 127.0
        ties[0, 1:] = np.arange(31) + 0.5
        for x in [ties, np.zeros((1, 32), dtype=np.float32)] + [bf16_exact(s) for s in range(32)]:
            self.assertEqual(quants.quantize(x, GGMLQuantizationType.Q8_0).tobytes(), ggml(x))

    def test_the_pins_here_are_the_scripts_pins(self):
        # The script runs under *this* file's interpreter (sys.executable), so
        # a drifted header would test one gguf and graft with another — and
        # the byte-for-byte claim is exactly what a gguf version decides. The
        # whole PEP 723 block, as test_mtp_graft.py holds its own.
        def block(p):
            lines = p.read_text().splitlines()
            start = lines.index("# /// script")
            return lines[start:lines.index("# ///", start) + 1]
        self.assertEqual(block(Path(__file__)), block(GRAFT))
        self.assertTrue(any(l.startswith("# dependencies") and "==" in l for l in block(GRAFT)),
                        "the script must pin its dependencies")

    def test_refuses_an_edited_file_without_wo_ud(self):
        r = run(self.base, self.edited({}), self.out)
        self.refused(r, "no model.llm.layers.<n>.attn.wo_ud.weight tensors")


if __name__ == "__main__":
    unittest.main()
