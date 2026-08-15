# The image `[sandbox]` in ~/.mecha/config.toml points at (`mecha-sandbox`).
#
#   docker build -t mecha-sandbox -f scripts/sandbox.Dockerfile .
#
# Why this exists: the sandbox's default image, debian:stable-slim, is
# deliberately bare — no git, no compiler — so the first confined session died
# on "command not found" three times in a row (2026-08-15). This one carries
# what mecha's actual errands need: git for repo work, the Rust toolchain
# matching the host's (rust:1-slim tracks stable, same 1.97.1), and
# rg/jq/python3 because the model reaches for them constantly.
#
# The profile.d line is load-bearing and non-obvious. mecha's sandboxed shell
# runs `bash -lc` (tool/builtin.rs — the sandbox needs one argv, and -l is
# part of that contract), and a LOGIN shell sources /etc/profile, which on
# Debian RESETS PATH for non-root users — silently dropping
# /usr/local/cargo/bin, so `cargo` was "not found" while sitting right there.
# CARGO_HOME=/tmp/cargo-home because the container runs as the calling
# uid:gid, whose home does not exist inside; cargo needs somewhere writable
# for its locks, and the container's /tmp is private to the run.
#
# No network is the operating assumption (`network = false`): a cargo build
# in here only works when it needs nothing from crates.io. That is the trade
# that lets the trifecta interlock permit confined shell at all — do
# dependency-fetching builds outside, yourself.
FROM rust:1-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ripgrep jq python3 && rm -rf /var/lib/apt/lists/* \
 && echo 'export PATH=/usr/local/cargo/bin:$PATH CARGO_HOME=/tmp/cargo-home' > /etc/profile.d/cargo.sh
