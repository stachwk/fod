#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
script="${repo_root}/scripts/fod-rust-toolchain-benchmark.sh"

bash -n "${script}"

plan="$({
  FOD_RUST_MSRV=1.85 \
  FOD_RUST_BASELINE_TOOLCHAIN=1.85.0 \
  FOD_RUST_CANDIDATE_TOOLCHAIN=1.98.0 \
  FOD_RUST_TOOLCHAIN_BENCH_ROOT="${repo_root}/target/toolchain-benchmark-policy-test" \
  FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS=3 \
  FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE=512K \
  FOD_RUST_TOOLCHAIN_BENCH_COUNT=64 \
  FOD_RUST_TOOLCHAIN_BENCH_SOURCE=pattern \
  bash "${script}" plan
})"

grep -Fqx 'workspace_rust_version=1.85' <<<"${plan}"
grep -Fqx 'baseline_toolchain=1.85.0' <<<"${plan}"
grep -Fqx 'candidate_toolchain=1.98.0' <<<"${plan}"
grep -Fqx 'automatic_toolchain_install=no' <<<"${plan}"
grep -Fqx 'automatic_target_cleanup=no' <<<"${plan}"
grep -Fq 'baseline-release: toolchain=1.85.0 profile=release' <<<"${plan}"
grep -Fq 'candidate-release: toolchain=1.98.0 profile=release' <<<"${plan}"
grep -Fq 'candidate-release-lto: toolchain=1.98.0 profile=release-lto' <<<"${plan}"

if FOD_RUST_TOOLCHAIN_BENCH_ROOT="${TMPDIR:-/tmp}/fod-toolchain-bad-root" bash "${script}" plan >/dev/null 2>&1; then
  echo "expected target-root safety check to reject a path outside repo target" >&2
  exit 1
fi

if FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS=0 bash "${script}" plan >/dev/null 2>&1; then
  echo "expected zero repetitions to be rejected" >&2
  exit 1
fi

if grep -Eq '^[[:space:]]*(rustup[[:space:]]+toolchain[[:space:]]+install|cargo[[:space:]]+clean|rm[[:space:]]+-rf)([[:space:]]|$)' "${script}"; then
  echo "benchmark helper must not execute toolchain installation or Cargo target cleanup" >&2
  exit 1
fi

grep -Eq '^rust-version[[:space:]]*=[[:space:]]*"1\.85"' "${repo_root}/Cargo.toml"
grep -Fq 'FOD_RUST_FUSE_BIN' "${repo_root}/rust_mkfs/src/bin/fod-bootstrap.rs"

echo "OK rust-toolchain-benchmark-policy"
