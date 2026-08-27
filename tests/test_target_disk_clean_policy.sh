#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
repo_root="$(realpath -m "${repo_root}")"
helper="${repo_root}/scripts/fod-target-clean.sh"
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
  echo "target disk cleanup policy test failed: $*" >&2
  exit 1
}

mkdir -p "${repo_root}/target"
"${helper}" status > "${tmp}/status.txt"
grep -q "^target=${repo_root}/target$" "${tmp}/status.txt" || fail "status did not resolve repository target"
grep -q '^eligible=' "${tmp}/status.txt" || fail "status did not report eligibility"

"${helper}" plan > "${tmp}/plan.txt" 2>&1
grep -q '^cargo_clean_dry_run:' "${tmp}/plan.txt" || fail "plan did not execute Cargo dry-run"

if FOD_TARGET_CLEAN_DIR="${tmp}/not-repo-target" "${helper}" status > "${tmp}/outside.txt" 2>&1; then
  fail "helper accepted cleanup path outside repository ./target"
fi
grep -q 'refusing cleanup outside repository ./target' "${tmp}/outside.txt" || fail "outside-target rejection had unexpected reason"

if FOD_TARGET_CLEAN_MIN_SIZE_BYTES=invalid "${helper}" status > "${tmp}/invalid-size.txt" 2>&1; then
  fail "helper accepted invalid size threshold"
fi
grep -q 'FOD_TARGET_CLEAN_MIN_SIZE_BYTES must be' "${tmp}/invalid-size.txt" || fail "invalid-size rejection had unexpected reason"

if FOD_TARGET_CLEAN_FORCE=1 "${helper}" clean > "${tmp}/no-confirm.txt" 2>&1; then
  fail "forced cleanup succeeded without explicit confirmation"
fi
grep -q 'explicit confirmation required' "${tmp}/no-confirm.txt" || fail "missing-confirmation rejection had unexpected reason"

if FOD_TARGET_CLEAN_FORCE=1 FOD_TARGET_CLEAN_CONFIRM=wrong-token "${helper}" clean > "${tmp}/wrong-confirm.txt" 2>&1; then
  fail "forced cleanup accepted wrong confirmation token"
fi
grep -q 'explicit confirmation required' "${tmp}/wrong-confirm.txt" || fail "wrong-confirmation rejection had unexpected reason"

# Positive end-to-end path in an isolated nested repository. This proves that
# the confirmation token plus eligible policy reaches Cargo clean without
# touching the real FOD target.
fixture="$(mktemp -d "${repo_root}/target/.fod-clean-policy-fixture-XXXXXX")"
mkdir -p "${fixture}/src" "${fixture}/target/debug"
git init -q "${fixture}"
cat > "${fixture}/Cargo.toml" <<'CARGOEOF'
[package]
name = "fod-clean-policy-fixture"
version = "0.0.0"
edition = "2021"

[workspace]
CARGOEOF
printf '%s\n' 'pub fn fixture() {}' > "${fixture}/src/lib.rs"
printf '%s\n' 'sentinel' > "${fixture}/target/debug/fod-clean-policy-sentinel"

(
  cd "${fixture}"
  FOD_TARGET_CLEAN_MIN_SIZE_BYTES=0 \
  FOD_TARGET_CLEAN_MIN_AGE_DAYS=0 \
  FOD_TARGET_CLEAN_CONFIRM=clean-disk-target \
    "${helper}" clean > "${tmp}/positive-clean.txt" 2>&1
)

grep -q '^reclaimed_bytes=' "${tmp}/positive-clean.txt" || \
  fail "successful cleanup did not report reclaimed bytes"
[[ ! -e "${fixture}/target/debug/fod-clean-policy-sentinel" ]] || \
  fail "successful cleanup left the target sentinel in place"

rm -rf -- "${fixture}"
fixture=""

echo "Controlled disk target cleanup policy: OK"
