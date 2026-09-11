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

# **What source this measures, said out loud.** This script builds whatever
# working tree it sits in — `cd` above, no branch check — and `bench/run.sh`
# calls it unconditionally. So a benchmark run from a checkout left on a
# feature branch silently measures that branch and labels the scorecard with
# today's model, which is the code-side twin of the MECHA_BENCH_MODEL warning
# in `run.sh`: "the run measures whatever is on that port under the wrong
# name". The model half of that confound has been guarded since the day it
# voided a run; the source half was not, and on 2026-09-11 a shared checkout
# sat on a feature branch with the benchmark pointed at it.
#
# Branch is reported, never refused — measuring a branch is a legitimate
# thing to do. What is refused is an *unattributable* build: a dirty tree
# produces a binary matching no commit, so its scorecard cannot be tied to
# source later. `MECHA_BENCH_ALLOW_DIRTY=1` opts out for a throwaway
# measurement, and says so in the line below.
SOURCE_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
SOURCE_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)"
SOURCE_DIRTY=""
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  SOURCE_DIRTY=" +dirty"
  if [ "${MECHA_BENCH_ALLOW_DIRTY:-0}" != "1" ]; then
    echo "refusing: $PWD is dirty, so $OUT would match no commit and its" >&2
    echo "  scorecard could not be tied back to source. Commit, stash, or set" >&2
    echo "  MECHA_BENCH_ALLOW_DIRTY=1 to measure it anyway." >&2
    exit 1
  fi
fi
echo "benchmark source: $SOURCE_BRANCH @ $SOURCE_COMMIT$SOURCE_DIRTY ($PWD)" >&2
export MECHA_BENCH_SOURCE="$SOURCE_BRANCH@$SOURCE_COMMIT$SOURCE_DIRTY"

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
echo "  built from: $SOURCE_BRANCH @ $SOURCE_COMMIT$SOURCE_DIRTY" >&2
