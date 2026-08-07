#!/bin/bash
# Build a statically-linked mecha for benchmark task containers.
#
# The benchmark uploads one binary into every task container and runs it there.
# Those containers are other people's images — Debian 11, Ubuntu 22.04, Alpine,
# whatever the task author chose — while the host is Ubuntu 24.04 with glibc
# 2.39. A `cargo build --release` here therefore produces a binary that refuses
# to start almost everywhere:
#
#   /installed-agent/mecha: /lib/aarch64-linux-gnu/libc.so.6:
#       version `GLIBC_2.39' not found (required by /installed-agent/mecha)
#
# And it fails *as an agent error*, not as a harness error: the trial records
# NonZeroAgentExitCodeError and reward 0.0, which is indistinguishable in a
# scorecard from a model that tried and failed. This voided the first real run
# (2026-08-07) after 4 trials — 3 of them dead in ~20 seconds.
#
# Static musl fixes it for every base at once, Alpine included, where even an
# older-glibc build would still be "not found". Built in a container because
# musl-tools needs root to install and `ring` needs a C toolchain targeting
# musl. rustls is already the TLS backend (no OpenSSL), so nothing here needs
# a system library.
#
# Caveat worth knowing: a static musl binary cannot use glibc's NSS, so
# hostname lookups go through musl's resolver. The benchmark config points at
# the container's gateway *by IP*, so no DNS is involved on the hot path.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="target-musl/release/mecha"

docker run --rm \
  -v "$PWD":/w -w /w \
  -e CARGO_HOME=/w/.cargo-musl \
  -e CARGO_TARGET_DIR=/w/target-musl \
  rust:alpine sh -c 'apk add --no-cache musl-dev >/dev/null && cargo build --release --bin mecha'

# The build ran as root; hand the artifacts back so the host can read, replace
# and clean them without sudo.
docker run --rm -v "$PWD":/w alpine chown -R "$(id -u):$(id -g)" /w/target-musl /w/.cargo-musl

# Assert what this script exists to guarantee, rather than trusting the build:
# a dynamic binary here is the exact failure that is invisible until a trial
# has already been scored 0.0.
file "$OUT" | grep -q "statically linked" || {
  echo "refusing: $OUT is not statically linked" >&2; exit 1; }
"$OUT" --version >/dev/null || { echo "refusing: $OUT does not run" >&2; exit 1; }

echo "portable binary: $OUT ($(file -b "$OUT" | cut -d, -f1-2))" >&2
