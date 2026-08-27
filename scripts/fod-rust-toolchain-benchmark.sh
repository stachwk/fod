#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

mode="${1:-plan}"
case "${mode}" in
  plan|run) ;;
  *)
    echo "usage: $0 [plan|run]" >&2
    exit 2
    ;;
esac

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "${script_dir}" rev-parse --show-toplevel)"
manifest="${repo_root}/Cargo.toml"

msrv_expected="${FOD_RUST_MSRV:-1.85}"
baseline_toolchain="${FOD_RUST_BASELINE_TOOLCHAIN:-1.85.0}"
candidate_toolchain="${FOD_RUST_CANDIDATE_TOOLCHAIN:-1.98.0}"
target_root="${FOD_RUST_TOOLCHAIN_BENCH_ROOT:-${repo_root}/target/toolchain-benchmark}"
repetitions="${FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS:-3}"
block_size="${FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE:-512K}"
block_count="${FOD_RUST_TOOLCHAIN_BENCH_COUNT:-64}"
source_mode="${FOD_RUST_TOOLCHAIN_BENCH_SOURCE:-pattern}"

die() {
  echo "ERROR: $*" >&2
  exit 1
}

require_positive_integer() {
  local name="$1"
  local value="$2"
  [[ "${value}" =~ ^[1-9][0-9]*$ ]] || die "${name} must be a positive integer, got '${value}'"
}

require_positive_integer FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS "${repetitions}"
require_positive_integer FOD_RUST_TOOLCHAIN_BENCH_COUNT "${block_count}"

command -v realpath >/dev/null 2>&1 || die "realpath is required"
target_abs="$(realpath -m "${target_root}")"
repo_target_abs="$(realpath -m "${repo_root}/target")"
case "${target_abs}" in
  "${repo_target_abs}"/*) ;;
  *) die "benchmark target root must stay below ${repo_target_abs}, got ${target_abs}" ;;
esac

manifest_msrv="$(sed -n 's/^[[:space:]]*rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${manifest}" | sed -n '1p')"
[[ -n "${manifest_msrv}" ]] || die "cannot read workspace rust-version from Cargo.toml"
[[ "${manifest_msrv}" == "${msrv_expected}" ]] || die "workspace rust-version=${manifest_msrv}, expected ${msrv_expected}"

variant_names=(baseline-release candidate-release candidate-release-lto)
variant_toolchains=("${baseline_toolchain}" "${candidate_toolchain}" "${candidate_toolchain}")
variant_profiles=(release release release-lto)

print_plan() {
  cat <<EOF
FOD Rust toolchain benchmark plan
repo_root=${repo_root}
workspace_rust_version=${manifest_msrv}
baseline_toolchain=${baseline_toolchain}
candidate_toolchain=${candidate_toolchain}
target_root=${target_abs}
repetitions=${repetitions}
throughput_block_size=${block_size}
throughput_count=${block_count}
throughput_source=${source_mode}
automatic_toolchain_install=no
automatic_target_cleanup=no
matrix:
  baseline-release: toolchain=${baseline_toolchain} profile=release
  candidate-release: toolchain=${candidate_toolchain} profile=release
  candidate-release-lto: toolchain=${candidate_toolchain} profile=release-lto
EOF
}

print_plan
[[ "${mode}" == "run" ]] || exit 0

command -v rustup >/dev/null 2>&1 || die "rustup is required; this benchmark never installs toolchains automatically"
command -v awk >/dev/null 2>&1 || die "awk is required"
command -v sed >/dev/null 2>&1 || die "sed is required"
command -v stat >/dev/null 2>&1 || die "stat is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

for toolchain in "${baseline_toolchain}" "${candidate_toolchain}"; do
  if ! rustup run "${toolchain}" rustc --version >/dev/null 2>&1; then
    die "Rust toolchain ${toolchain} is not installed; install it explicitly with: rustup toolchain install ${toolchain}"
  fi
done

mkdir -p "${target_abs}"
run_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
report_dir="${target_abs}/reports/${run_stamp}"
mkdir -p "${report_dir}"

metadata_file="${report_dir}/metadata.txt"
{
  echo "captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git_commit=$(git -C "${repo_root}" rev-parse HEAD)"
  echo "git_status=$(git -C "${repo_root}" status --porcelain | wc -l | tr -d ' ') changed_paths"
  echo "uname=$(uname -srmo 2>/dev/null || uname -a)"
  if command -v lscpu >/dev/null 2>&1; then
    echo "cpu_model=$(lscpu | sed -n 's/^Model name:[[:space:]]*//p' | sed -n '1p')"
  fi
  echo "RUSTFLAGS=${RUSTFLAGS:-}"
  echo "CARGO_ENCODED_RUSTFLAGS=${CARGO_ENCODED_RUSTFLAGS:-}"
} >"${metadata_file}"

build_tsv="${report_dir}/builds.tsv"
printf 'variant\ttoolchain\tprofile\trustc\tllvm\tbuild_seconds\tfuse_bytes\tbootstrap_bytes\tmkfs_bytes\tfuse_sha256\n' >"${build_tsv}"

declare -a variant_bootstrap variant_mkfs variant_fuse variant_build_seconds

build_variant() {
  local idx="$1"
  local name="${variant_names[$idx]}"
  local toolchain="${variant_toolchains[$idx]}"
  local profile="${variant_profiles[$idx]}"
  local target_dir="${target_abs}/${name}"
  local build_log="${report_dir}/${name}-build.log"
  local rustc_info="${report_dir}/${name}-rustc-vV.txt"
  local started_ns finished_ns elapsed_s
  local bootstrap mkfs fuse rustc_release llvm_version fuse_bytes bootstrap_bytes mkfs_bytes fuse_sha

  mkdir -p "${target_dir}"
  rustup run "${toolchain}" rustc -vV >"${rustc_info}"
  rustc_release="$(sed -n 's/^release: //p' "${rustc_info}")"
  llvm_version="$(sed -n 's/^LLVM version: //p' "${rustc_info}")"

  echo "=== build ${name}: rust=${toolchain} profile=${profile} ==="
  started_ns="$(date +%s%N)"
  if ! CARGO_TARGET_DIR="${target_dir}" rustup run "${toolchain}" cargo build \
      --locked \
      --manifest-path "${manifest}" \
      --profile "${profile}" \
      -p fod-rust-mkfs --bins \
      -p fod-rust-fuse --bin fod-rust-fuse \
      >"${build_log}" 2>&1; then
    cat "${build_log}" >&2
    die "build failed for ${name}"
  fi
  finished_ns="$(date +%s%N)"
  elapsed_s="$(awk -v start="${started_ns}" -v finish="${finished_ns}" 'BEGIN { printf "%.3f", (finish-start)/1000000000 }')"
  cat "${build_log}"

  bootstrap="${target_dir}/${profile}/fod-bootstrap"
  mkfs="${target_dir}/${profile}/fod-rust-mkfs"
  fuse="${target_dir}/${profile}/fod-rust-fuse"
  [[ -x "${bootstrap}" ]] || die "missing benchmark bootstrap binary: ${bootstrap}"
  [[ -x "${mkfs}" ]] || die "missing benchmark mkfs binary: ${mkfs}"
  [[ -x "${fuse}" ]] || die "missing benchmark FUSE binary: ${fuse}"

  "${bootstrap}" --version
  "${mkfs}" --version

  fuse_bytes="$(stat -c '%s' "${fuse}")"
  bootstrap_bytes="$(stat -c '%s' "${bootstrap}")"
  mkfs_bytes="$(stat -c '%s' "${mkfs}")"
  fuse_sha="$(sha256sum "${fuse}" | awk '{print $1}')"

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${name}" "${toolchain}" "${profile}" "${rustc_release}" "${llvm_version}" \
    "${elapsed_s}" "${fuse_bytes}" "${bootstrap_bytes}" "${mkfs_bytes}" "${fuse_sha}" \
    >>"${build_tsv}"

  variant_bootstrap[$idx]="${bootstrap}"
  variant_mkfs[$idx]="${mkfs}"
  variant_fuse[$idx]="${fuse}"
  variant_build_seconds[$idx]="${elapsed_s}"
}

for idx in 0 1 2; do
  build_variant "${idx}"
done

for idx in 0 1 2; do
  : >"${report_dir}/${variant_names[$idx]}-throughput.samples"
done

run_sample() {
  local idx="$1"
  local repetition="$2"
  local name="${variant_names[$idx]}"
  local log_file="${report_dir}/${name}-run-${repetition}.log"
  local throughput

  echo "=== benchmark ${name}: repetition=${repetition} ==="
  if ! FOD_BOOTSTRAP_BIN="${variant_bootstrap[$idx]}" \
      FOD_MKFS_BIN="${variant_mkfs[$idx]}" \
      FOD_RUST_FUSE_BIN="${variant_fuse[$idx]}" \
      THROUGHPUT_BLOCK_SIZE="${block_size}" \
      THROUGHPUT_COUNT="${block_count}" \
      THROUGHPUT_SOURCE="${source_mode}" \
      THROUGHPUT_SYNC=0 \
      bash "${repo_root}/tests/integration/test_throughput.sh" \
      >"${log_file}" 2>&1; then
    cat "${log_file}" >&2
    die "throughput benchmark failed for ${name}, repetition ${repetition}"
  fi
  cat "${log_file}"

  throughput="$(sed -n 's/.*(\([0-9][0-9.]*\) MiB\/s).*/\1/p' "${log_file}" | tail -n 1)"
  [[ -n "${throughput}" ]] || die "cannot parse throughput from ${log_file}"
  printf '%s\n' "${throughput}" >>"${report_dir}/${name}-throughput.samples"
}

for repetition in $(seq 1 "${repetitions}"); do
  if (( repetition % 2 == 1 )); then
    order=(0 1 2)
  else
    order=(2 1 0)
  fi
  for idx in "${order[@]}"; do
    run_sample "${idx}" "${repetition}"
  done
done

summary_tsv="${report_dir}/summary.tsv"
printf 'variant\ttoolchain\tprofile\trepetitions\tavg_mib_s\tmin_mib_s\tmax_mib_s\tbuild_seconds\tfuse_bytes\n' >"${summary_tsv}"

summary_md="${report_dir}/summary.md"
{
  echo "# FOD Rust toolchain benchmark"
  echo
  echo "- commit: \`$(git -C "${repo_root}" rev-parse HEAD)\`"
  echo "- workspace MSRV declaration: \`${manifest_msrv}\` (unchanged by this benchmark)"
  echo "- workload: write, block size \`${block_size}\`, count \`${block_count}\`, source \`${source_mode}\`"
  echo "- repetitions: \`${repetitions}\`; run order alternates to reduce fixed ordering bias"
  echo "- build directories are isolated and retained; no Cargo target is cleaned automatically"
  echo
  echo "| variant | toolchain | profile | avg MiB/s | min | max | build s | FUSE bytes |"
  echo "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |"
} >"${summary_md}"

for idx in 0 1 2; do
  name="${variant_names[$idx]}"
  toolchain="${variant_toolchains[$idx]}"
  profile="${variant_profiles[$idx]}"
  samples="${report_dir}/${name}-throughput.samples"
  read -r avg min max < <(awk '
    NR == 1 { min=$1; max=$1 }
    { sum += $1; if ($1 < min) min=$1; if ($1 > max) max=$1; n++ }
    END { if (n == 0) exit 1; printf "%.2f %.2f %.2f\n", sum/n, min, max }
  ' "${samples}")
  fuse_bytes="$(stat -c '%s' "${variant_fuse[$idx]}")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${name}" "${toolchain}" "${profile}" "${repetitions}" "${avg}" "${min}" "${max}" \
    "${variant_build_seconds[$idx]}" "${fuse_bytes}" >>"${summary_tsv}"
  printf '| %s | %s | %s | %s | %s | %s | %s | %s |\n' \
    "${name}" "${toolchain}" "${profile}" "${avg}" "${min}" "${max}" \
    "${variant_build_seconds[$idx]}" "${fuse_bytes}" >>"${summary_md}"
done

echo
echo "Benchmark report: ${summary_md}"
cat "${summary_md}"
