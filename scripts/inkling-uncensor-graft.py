#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["gguf==0.19.0", "numpy==2.5.3"]
# ///
"""Give a mainline Inkling 2 Mini GGUF the attention-output edit of an abliterated checkpoint.

    uv run inkling-uncensor-graft.py BASE EDITED OUT

BASE is a mainline-layout (`inkling2_loop`) GGUF whose `blk.<n>.attn_output`
tensors are Q8_0 (unsloth's Mainline_Support UD quants are). EDITED is a
safetensors file carrying `model.llm.layers.<n>.attn.wo_ud.weight` in BF16 —
the abliterated checkpoint itself, or any file holding just those tensors.

**Why a byte patch is enough.** OpenUncensored's Inkling-2-Mini-Uncensored
differs from the original by exactly the 20 `attn.wo_ud` tensors of layers
4–23 (diffed 2026-11-21: same safetensors header, every other tensor's
leading and trailing 64 KiB equal). The converter maps `attn.wo_ud` to
`attn_output` with no transform, and Q8_0 takes no importance matrix, so
re-quantizing an edited tensor to Q8_0 is what the quantizer that made BASE
would have written. That is checked, not assumed: an EDITED tensor equal to
the original must reproduce BASE's bytes exactly, and a run where none does
is refused — it means a different base or a different quantizer.

The output is a copy of BASE with those tensors' bytes overwritten, written
beside OUT and renamed into place only once complete.
"""
import json
import os
import shutil
import struct
import sys

import numpy as np
from gguf import GGMLQuantizationType, GGUFReader
from gguf.quants import quantize


def bf16(raw, shape):
    return (np.frombuffer(raw, dtype="<u2").astype(np.uint32) << 16).view(np.float32).reshape(shape)


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__.split("\n\n")[1])
    base_p, edit_p, out_p = sys.argv[1:]
    try:
        base = GGUFReader(base_p)
    except Exception as e:
        sys.exit(f"{base_p}: not a readable GGUF ({e})")
    arch = base.fields["general.architecture"].contents()
    if arch != "inkling2_loop":
        sys.exit(f"{base_p}: architecture {arch}, not inkling2_loop (the mainline layout)")
    tensors = {t.name: t for t in base.tensors}

    with open(edit_p, "rb") as f:
        n = struct.unpack("<Q", f.read(8))[0]
        hdr = json.loads(f.read(n))
        start = 8 + n
        wanted = sorted((k for k in hdr if k.endswith(".attn.wo_ud.weight")), key=lambda k: int(k.split(".")[3]))
        if not wanted:
            sys.exit(f"{edit_p}: no model.llm.layers.<n>.attn.wo_ud.weight tensors")
        patches, same = [], []
        for k in wanted:
            m = hdr[k]
            layer = int(k.split(".")[3])
            t = tensors.get(f"blk.{layer}.attn_output.weight")
            if t is None:
                sys.exit(f"{base_p} has no blk.{layer}.attn_output.weight")
            if m["dtype"] != "BF16":
                sys.exit(f"{k}: {m['dtype']}, expected BF16")
            if t.tensor_type != GGMLQuantizationType.Q8_0:
                sys.exit(f"blk.{layer}.attn_output.weight is {t.tensor_type.name}; only Q8_0 re-encodes exactly")
            if list(m["shape"][::-1]) != [int(x) for x in t.shape]:
                sys.exit(f"{k}: shape {m['shape']} vs GGUF {list(t.shape)} — not the same model")
            s, e = m["data_offsets"]
            f.seek(start + s)
            q = quantize(bf16(f.read(e - s), m["shape"]), GGMLQuantizationType.Q8_0).tobytes()
            if len(q) != t.n_bytes:
                sys.exit(f"{k}: re-encoded to {len(q)} bytes, GGUF holds {t.n_bytes}")
            (same if q == bytes(t.data.tobytes()) else patches).append((layer, t.data_offset, q))
    if not same:
        sys.exit("no edited tensor reproduces the base's bytes: a different base or quantizer; refusing to graft")
    if not patches:
        sys.exit("every tensor already matches the base; nothing to graft")

    tmp = out_p + ".partial"
    shutil.copyfile(base_p, tmp)
    with open(tmp, "r+b") as f:
        for _, off, q in patches:
            f.seek(off)
            f.write(q)
    # Read the patched copy back through the reader: every tensor where it was,
    # the patched ones holding the new bytes.
    check = {t.name: t for t in GGUFReader(tmp).tensors}
    for layer, _, q in patches:
        if bytes(check[f"blk.{layer}.attn_output.weight"].data.tobytes()) != q:
            sys.exit(f"read-back of blk.{layer}.attn_output.weight failed; {tmp} left for inspection")
    os.replace(tmp, out_p)
    print(f"reproduced {len(same)} unedited layers exactly ({', '.join(str(l) for l, _, _ in same)}); "
          f"grafted {len(patches)} ({', '.join(str(l) for l, _, _ in patches)}): {out_p}")


if __name__ == "__main__":
    main()
