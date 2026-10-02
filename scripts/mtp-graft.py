#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["gguf==0.19.0", "numpy==2.5.3"]
# ///
"""Give a GGUF that lost its MTP head the head from another conversion of the same model.

    uv run scripts/mtp-graft.py BASE DONOR OUT

BASE is the file to serve (an abliterated or fine-tuned conversion that dropped
the head); DONOR is a conversion of the same base model that kept it. Every
tensor and key of BASE is copied byte for byte; DONOR contributes only its
`blk.<n>.*` layers past BASE's block_count — the MTP head — and the
`nextn_predict_layers` key that declares them.

**Why a graft is enough.** HauhauCS's Qwen3.6-35B-A3B differs from unsloth's
MTP conversion by exactly `blk.40.*` (20 tensors, 504 MiB), `block_count`
40 → 41 and `nextn_predict_layers` — no per-layer array to extend (diffed
2026-10-02). The head reads the main model's `token_embd` and `output`, so
those must match in shape, which is checked. Speculation never changes what
the target says: every drafted token is verified by BASE, so a head trained on
the censored model costs acceptance, never behaviour.

The output is written beside OUT and renamed into place only once complete,
so an interrupted run can never leave a half file for start-router.sh to load.

**gguf is pinned** because this leans on its reader/writer internals and the
day it is next needed is a re-upload of an input, possibly months on. The
graft's name also carries a hash of this file (start-router.sh `graft_path`),
so an edit here retires every graft built by the old version.
scripts/test_mtp_graft.py round-trips it on synthetic files under the same pins.
"""
import os
import sys

from gguf import GGUFReader, GGUFValueType, GGUFWriter


def layer(name):
    return int(name.split(".")[1]) if name.startswith("blk.") else None


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__.split("\n\n")[1])
    base_p, donor_p, out_p = sys.argv[1:]
    base, donor = GGUFReader(base_p), GGUFReader(donor_p)
    arch = base.fields["general.architecture"].contents()
    if donor.fields["general.architecture"].contents() != arch:
        sys.exit(f"architectures differ: {arch} vs {donor.fields['general.architecture'].contents()}")
    if f"{arch}.nextn_predict_layers" in base.fields:
        sys.exit(f"{base_p} already declares an MTP head; nothing to graft")
    nextn_field = donor.fields.get(f"{arch}.nextn_predict_layers")
    if nextn_field is None:
        sys.exit(f"{donor_p} has no MTP head to give")
    n_base = base.fields[f"{arch}.block_count"].contents()
    n_donor = donor.fields[f"{arch}.block_count"].contents()
    nextn = nextn_field.contents()
    if n_donor != n_base + nextn:
        sys.exit(f"block counts do not line up: base {n_base} + nextn {nextn} != donor {n_donor}")

    ours = {t.name: t for t in base.tensors}
    head = []
    for t in donor.tensors:
        n = layer(t.name)
        if n is not None and n >= n_base:
            head.append(t)
        elif t.name in ours and list(ours[t.name].shape) != list(t.shape):
            sys.exit(f"{t.name}: shape {list(ours[t.name].shape)} vs donor {list(t.shape)} — not the same model")
    for shared in ("token_embd.weight", "output.weight"):
        if shared in ours and shared not in {t.name for t in donor.tensors}:
            sys.exit(f"donor lacks {shared}; cannot confirm the head shares BASE's vocabulary")
    if not head:
        sys.exit("donor declares a head but carries no tensors past the base's layers")

    tmp = out_p + ".partial"
    w = GGUFWriter(tmp, arch=arch, endianess=base.endianess)
    w.data_alignment = base.alignment
    for f in base.fields.values():
        if f.name == "general.architecture" or f.name.startswith("GGUF."):
            continue
        vt = f.types[0]
        v = f.contents()
        if f.name == f"{arch}.block_count":
            v = n_donor
        w.add_key_value(f.name, v, vt, sub_type=f.types[-1] if vt == GGUFValueType.ARRAY else None)
    w.add_key_value(f"{arch}.nextn_predict_layers", nextn, nextn_field.types[0])

    tensors = list(base.tensors) + head
    for t in tensors:
        w.add_tensor_info(t.name, t.data.shape, t.data.dtype, t.data.nbytes, t.tensor_type)
    w.write_header_to_file()
    w.write_kv_data_to_file()
    w.write_ti_data_to_file()
    for t in tensors:
        # The endianness of the *data* (the reader's memmap), not the writer's:
        # omitted, gguf assumes native and would byteswap a big-endian file.
        w.write_tensor_data(t.data, tensor_endianess=base.endianess)
    w.close()
    os.replace(tmp, out_p)
    print(f"grafted {len(head)} tensors ({sum(t.n_bytes for t in head) / 2**20:.0f} MiB): {out_p}")


if __name__ == "__main__":
    main()
