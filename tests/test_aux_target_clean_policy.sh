#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
repo_root="$(realpath -m "${repo_root}")"
helper="${repo_root}/scripts/fod-aux-target-clean.sh"
tmp="$(mktemp -d)"
fixture=""

cleanup() {
  rm -rf -- "${tmp}"
  if [[ -n "${fixture}" ]]; then
    rm -rf -- "${fixture}"
  fi
}
trap cleanup EXIT

fail() {
  echo "aux target cleanup policy test failed: $*" >&2
  exit 1
}

mkdir -p "${repo_root}/target/test-locking"
cat > "${repo_root}/target/test-locking/CACHEDIR.TAG" <<'CACHEEOF'
Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by cargo.
# For information about cache directory tags see https://bford.info/cachedir/
CACHEEOF
"${helper}" status > "${tmp}/status.txt"
grep -q '^aux_name=test-locking$' "${tmp}/status.txt" || \
  fail "status did not select test-locking"
grep -q "^target=${repo_root}/target/test-locking$" "${tmp}/status.txt" || \
  fail "status did not resolve allowlisted auxiliary target"

"${helper}" plan > "${tmp}/plan.txt" 2>&1
grep -q '^cargo_clean_dry_run:' "${tmp}/plan.txt" || \
  fail "plan did not execute Cargo dry-run"

if FOD_TARGET_AUX_NAME=debug "${helper}" status > "${tmp}/debug.txt" 2>&1; then
  fail "helper accepted main debug target"
fi
grep -q 'unsupported auxiliary target' "${tmp}/debug.txt" || \
  fail "debug-target rejection had unexpected reason"

if FOD_TARGET_AUX_NAME=release "${helper}" status > "${tmp}/release.txt" 2>&1; then
  fail "helper accepted main release target"
fi
grep -q 'unsupported auxiliary target' "${tmp}/release.txt" || \
  fail "release-target rejection had unexpected reason"

if FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES=invalid \
   "${helper}" status > "${tmp}/invalid-size.txt" 2>&1; then
  fail "helper accepted invalid size threshold"
fi
grep -q 'FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES must be' "${tmp}/invalid-size.txt" || \
  fail "invalid-size rejection had unexpected reason"

if FOD_TARGET_AUX_CLEAN_FORCE=1 \
   "${helper}" clean > "${tmp}/no-confirm.txt" 2>&1; then
  fail "forced auxiliary cleanup succeeded without explicit confirmation"
fi
grep -q 'explicit confirmation required' "${tmp}/no-confirm.txt" || \
  fail "missing-confirmation rejection had unexpected reason"

if FOD_TARGET_AUX_CLEAN_FORCE=1 \
   FOD_TARGET_AUX_CLEAN_CONFIRM=wrong-token \
   "${helper}" clean > "${tmp}/wrong-confirm.txt" 2>&1; then
  fail "forced auxiliary cleanup accepted wrong confirmation token"
fi
grep -q 'explicit confirmation required' "${tmp}/wrong-confirm.txt" || \
  fail "wrong-confirmation rejection had unexpected reason"

# Positive end-to-end path in an isolated nested repository. The auxiliary
# target must be removed while the main debug/release caches and CARGO_HOME
# sentinel remain untouched.
fixture="$(mktemp -d "${repo_root}/target/.fod-aux-clean-policy-XXXXXX")"
mkdir -p \
  "${fixture}/src" \
  "${fixture}/target/test-locking/debug" \
  "${fixture}/target/debug" \
  "${fixture}/target/release" \
  "${fixture}/cargo-home"
git init -q "${fixture}"
cat > "${fixture}/Cargo.toml" <<'CARGOEOF'
[package]
name = "fod-aux-clean-policy-fixture"
version = "0.0.0"
edition = "2021"

[workspace]
CARGOEOF
printf '%s\n' 'pub fn fixture() {}' > "${fixture}/src/lib.rs"
cat > "${fixture}/target/test-locking/CACHEDIR.TAG" <<'CACHEEOF'
Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by cargo.
# For information about cache directory tags see https://bford.info/cachedir/
CACHEEOF
printf '%s\n' 'aux' > "${fixture}/target/test-locking/debug/aux-sentinel"
printf '%s\n' 'debug' > "${fixture}/target/debug/main-debug-sentinel"
printf '%s\n' 'release' > "${fixture}/target/release/main-release-sentinel"
printf '%s\n' 'download-cache' > "${fixture}/cargo-home/download-cache-sentinel"

(
  cd "${fixture}"
  CARGO_HOME="${fixture}/cargo-home" \
  FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES=0 \
  FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS=0 \
  FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target \
    "${helper}" clean > "${tmp}/positive-clean.txt" 2>&1
)

grep -q '^reclaimed_bytes=' "${tmp}/positive-clean.txt" || \
  fail "successful auxiliary cleanup did not report reclaimed bytes"
[[ ! -e "${fixture}/target/test-locking/debug/aux-sentinel" ]] || \
  fail "successful cleanup left auxiliary sentinel in place"
[[ -e "${fixture}/target/debug/main-debug-sentinel" ]] || \
  fail "successful cleanup damaged main debug cache"
[[ -e "${fixture}/target/release/main-release-sentinel" ]] || \
  fail "successful cleanup damaged main release cache"
[[ -e "${fixture}/cargo-home/download-cache-sentinel" ]] || \
  fail "successful cleanup damaged CARGO_HOME cache"

rm -rf -- "${fixture}"
fixture=""

echo "Selective auxiliary target cleanup policy: OK"
