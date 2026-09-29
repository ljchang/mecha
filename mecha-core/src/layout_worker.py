# The layout model's worker: PP-DocLayoutV3 under ONNX Runtime, run by
# mecha-core/src/layout.rs inside the same confinement as the PDF renderer
# (docs/DOCUMENT-EXTRACTION-DESIGN.md §5). It is compiled into the mecha
# binary and passed with `python -I -B -c`, so the interpreter runs exactly
# this text and nothing on disk can stand in for it.
#
# It never sees an image file. The parent decodes the page with the
# memory-safe `image` crate, resizes it, and writes a fixed-size tensor:
# 3 x 800 x 800 little-endian float32 in [0, 1], RGB, channel-first. The
# only thing a hostile page controls here is the values of those floats.
#
# Protocol, all little-endian:
#   worker -> parent  b"MLY1" once the model is loaded (the ready marker)
#   parent -> worker  one tensor (7,680,000 bytes) per page
#   worker -> parent  u32 n, then n rows of 7 float32:
#                     class, score, x0, y0, x1, y1, reading order
#                     in the 800 x 800 input's pixels
#   parent closes stdin -> worker exits 0
import struct
import sys

import numpy as np
import onnxruntime as ort

SIDE = 800
TENSOR_BYTES = 3 * SIDE * SIDE * 4


def main():
    model, threads = sys.argv[1], int(sys.argv[2])
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = threads
    opts.inter_op_num_threads = 1
    # Warnings go to stderr, which the parent keeps only a bounded tail of.
    opts.log_severity_level = 3
    session = ort.InferenceSession(model, opts, providers=["CPUExecutionProvider"])
    boxes_out = session.get_outputs()[0].name
    im_shape = np.array([[SIDE, SIDE]], dtype=np.float32)
    scale = np.array([[1.0, 1.0]], dtype=np.float32)
    src, dst = sys.stdin.buffer, sys.stdout.buffer
    dst.write(b"MLY1")
    dst.flush()
    while True:
        buf = src.read(TENSOR_BYTES)
        if not buf:
            return 0
        if len(buf) != TENSOR_BYTES:
            sys.stderr.write("short tensor: %d bytes\n" % len(buf))
            return 2
        image = np.frombuffer(buf, dtype="<f4").reshape(1, 3, SIDE, SIDE)
        rows = session.run(
            [boxes_out], {"im_shape": im_shape, "image": image, "scale_factor": scale}
        )[0]
        rows = np.ascontiguousarray(rows[:, :7], dtype="<f4")
        dst.write(struct.pack("<I", rows.shape[0]))
        dst.write(rows.tobytes())
        dst.flush()


sys.exit(main())
