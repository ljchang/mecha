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
# Prove git can answer before trusting its silence. `git status --porcelain`
# exits non-zero with *empty stdout* when it refuses the tree — dubious
# ownership (plausible here: this script runs docker as root over `$PWD` and
# chowns back afterwards), git absent, or an rsync'd copy that is not a clone.
# An emptiness test then reads "cannot tell" as "clean", and the guard written
# to refuse an unattributable build would wave through the most unattributable
# source there is. Unknown is never clean.
if ! git rev-parse --git-dir >/dev/null 2>&1; then
  if [ "${MECHA_BENCH_ALLOW_DIRTY:-0}" != "1" ]; then
    echo "refusing: $PWD is not a readable git checkout, so $OUT could not be" >&2
    echo "  tied back to source. Set MECHA_BENCH_ALLOW_DIRTY=1 to build anyway." >&2
    exit 1
  fi
  SOURCE_BRANCH="unknown"; SOURCE_COMMIT="unknown"; SOURCE_DIRTY=" +unverified"
else
  SOURCE_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
  SOURCE_BRANCH="$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)"
  SOURCE_DIRTY=""
  # Capture the status, exit code and all. `[ -n "$(git status …)" ]` would
  # discard it, and a command substitution inside a test suppresses `errexit`
  # too — so a failing `git status` would be indistinguishable from a clean
  # tree, which is the failure the paragraph above describes happening one
  # line further down. `rev-parse --git-dir` does not settle this: it reads
  # neither the worktree nor the index, so it passes for a repo whose status
  # then fails on a bad `core.fsmonitor`, a corrupt index, or an unstat-able
  # path.
  if ! STATUS="$(git status --porcelain)"; then
    if [ "${MECHA_BENCH_ALLOW_DIRTY:-0}" != "1" ]; then
      echo "refusing: git could not read $PWD, so $OUT could not be tied back" >&2
      echo "  to source. Set MECHA_BENCH_ALLOW_DIRTY=1 to build anyway." >&2
      exit 1
    fi
    STATUS=""
    SOURCE_DIRTY=" +unverified"
  fi
  if [ -n "$STATUS" ]; then
    SOURCE_DIRTY=" +dirty"
    if [ "${MECHA_BENCH_ALLOW_DIRTY:-0}" != "1" ]; then
      echo "refusing: $PWD is dirty, so $OUT would match no commit and its" >&2
      echo "  scorecard could not be tied back to source. Commit, stash, or set" >&2
      echo "  MECHA_BENCH_ALLOW_DIRTY=1 to measure it anyway." >&2
      exit 1
    fi
  fi
fi
SOURCE="$SOURCE_BRANCH@$SOURCE_COMMIT$SOURCE_DIRTY"
echo "benchmark source: $SOURCE_BRANCH @ $SOURCE_COMMIT$SOURCE_DIRTY ($PWD)" >&2

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

# Beside the artifact, not in the environment. `bench/run.sh` calls this
# script as a child, so an exported variable dies with it and the parent that
# goes on to run the benchmark never sees it — the provenance would live only
# in a stderr line, and "tie the scorecard back to source later" is exactly
# when stderr is gone. A file next to the binary can be asked, which is the
# rule this repo applies to every other artifact.
#
# **The digest is what binds the two.** This script is not the only writer of
# `$OUT`: the procedure this repo has followed twice — build in a clean
# worktree, then `cp` the static binary into the shared checkout's
# `target-musl/release/mecha`, the path `bench/run.sh` executes — replaces the
# binary and not this file. Adjacency alone would then assert the provenance of
# a binary that is gone, which is worse than no file at all. With the digest
# recorded, a `.source` that does not describe the binary beside it says so
# when asked, and `mecha.source` becomes a claim that can be false out loud
# rather than a label that is quietly wrong.
printf '%s\n' "$SOURCE" > "$OUT.source"
printf 'sha256 %s\n' "$(sha256sum "$OUT" | cut -d' ' -f1)" >> "$OUT.source"

echo "portable binary: $OUT ($(file -b "$OUT" | cut -d, -f1-2))" >&2
echo "  built from: $SOURCE (recorded in $OUT.source)" >&2
