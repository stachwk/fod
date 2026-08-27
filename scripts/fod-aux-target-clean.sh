#!/usr/bin/env bash
set -euo pipefail

command_name="${1:-status}"
case "${command_name}" in
  status|plan|clean) ;;
  *)
    echo "usage: $0 [status|plan|clean]" >&2
    exit 2
    ;;
esac

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || \
  fail "must be run inside a git repository"
repo_root="$(realpath -m "${repo_root}")"
manifest="${repo_root}/Cargo.toml"
root_target="${repo_root}/target"
aux_name="${FOD_TARGET_AUX_NAME:-test-locking}"
min_size="${FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES:-1073741824}"
min_age_days="${FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS:-7}"
confirm="${FOD_TARGET_AUX_CLEAN_CONFIRM:-}"
force_raw="${FOD_TARGET_AUX_CLEAN_FORCE:-0}"
cargo_bin="${RUST_CARGO:-cargo}"

[[ -f "${manifest}" ]] || fail "Cargo.toml not found at ${manifest}"

case "${aux_name}" in
  test-locking) ;;
  *) fail "unsupported auxiliary target: ${aux_name}; allowed: test-locking" ;;
esac

case "${min_size}" in
  ''|*[!0-9]*) fail "FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES must be a non-negative integer" ;;
esac
case "${min_age_days}" in
  ''|*[!0-9]*) fail "FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS must be a non-negative integer" ;;
esac
case "${force_raw}" in
  1|true|yes|on) force=1 ;;
  0|false|no|off|'') force=0 ;;
  *) fail "FOD_TARGET_AUX_CLEAN_FORCE must be 0/1, false/true, no/yes, or off/on" ;;
esac

[[ ! -L "${root_target}" ]] || \
  fail "refusing auxiliary cleanup because repository target is a symlink: ${root_target}"

target="${root_target}/${aux_name}"
[[ ! -L "${target}" ]] || \
  fail "refusing auxiliary cleanup because target is a symlink: ${target}"

target_abs="$(realpath -m "${target}")"
expected_abs="$(realpath -m "${repo_root}/target/test-locking")"
[[ "${target_abs}" == "${expected_abs}" ]] || \
  fail "refusing auxiliary cleanup outside allowlisted target: ${target_abs}"

target_exists=0
target_size=0
newest_epoch=0
newest_iso="none"
age_seconds=0
age_days=0
fs_type="none"

if [[ -e "${target_abs}" && ! -d "${target_abs}" ]]; then
  fail "auxiliary target exists but is not a directory: ${target_abs}"
fi

if [[ -d "${target_abs}" ]]; then
  target_exists=1
  fs_type="$(stat -f -c %T "${target_abs}")"
  [[ "${fs_type}" != "tmpfs" ]] || \
    fail "refusing disk auxiliary cleanup on tmpfs: ${target_abs}"
  target_size="$(du -sb -- "${target_abs}" | awk '{print $1}')"
  newest_epoch="$(
    find "${target_abs}" -mindepth 1 -printf '%T@\n' 2>/dev/null \
      | awk 'BEGIN { max = 0 } { if ($1 > max) max = $1 } END { printf "%.0f\n", max }'
  )"
  now_epoch="$(date +%s)"
  if [[ "${newest_epoch}" -gt 0 ]]; then
    if [[ "${newest_epoch}" -gt "${now_epoch}" ]]; then
      age_seconds=0
    else
      age_seconds=$((now_epoch - newest_epoch))
    fi
    age_days=$((age_seconds / 86400))
    newest_iso="$(date -d "@${newest_epoch}" -Is)"
  fi
fi

min_age_seconds=$((min_age_days * 86400))
eligible=0
size_ok=0
age_ok=0
[[ "${target_size}" -ge "${min_size}" ]] && size_ok=1
[[ "${target_exists}" -eq 1 && "${age_seconds}" -ge "${min_age_seconds}" ]] && age_ok=1
[[ "${target_exists}" -eq 1 && "${size_ok}" -eq 1 && "${age_ok}" -eq 1 ]] && eligible=1

human_bytes() {
  local bytes="$1"
  numfmt --to=iec-i --suffix=B "${bytes}" 2>/dev/null || printf '%s B' "${bytes}"
}

print_status() {
  printf 'aux_name=%s\n' "${aux_name}"
  printf 'target=%s\n' "${target_abs}"
  printf 'target_exists=%s\n' "$([[ "${target_exists}" -eq 1 ]] && echo yes || echo no)"
  printf 'filesystem_type=%s\n' "${fs_type}"
  printf 'target_size_bytes=%s\n' "${target_size}"
  printf 'target_size_human=%s\n' "$(human_bytes "${target_size}")"
  printf 'newest_activity=%s\n' "${newest_iso}"
  printf 'age_seconds=%s\n' "${age_seconds}"
  printf 'age_days=%s\n' "${age_days}"
  printf 'min_size_bytes=%s\n' "${min_size}"
  printf 'min_size_human=%s\n' "$(human_bytes "${min_size}")"
  printf 'min_age_days=%s\n' "${min_age_days}"
  printf 'size_threshold_met=%s\n' "$([[ "${size_ok}" -eq 1 ]] && echo yes || echo no)"
  printf 'age_threshold_met=%s\n' "$([[ "${age_ok}" -eq 1 ]] && echo yes || echo no)"
  printf 'eligible=%s\n' "$([[ "${eligible}" -eq 1 ]] && echo yes || echo no)"
  printf 'force=%s\n' "$([[ "${force}" -eq 1 ]] && echo yes || echo no)"
}

print_largest_entries() {
  [[ "${target_exists}" -eq 1 ]] || return 0
  printf '%s\n' 'largest_top_level_entries:'
  find "${target_abs}" -mindepth 1 -maxdepth 1 -print0 2>/dev/null \
    | xargs -0 -r du -sb -- 2>/dev/null \
    | sort -nr \
    | sed -n '1,20p' \
    | awk '{ bytes=$1; $1=""; sub(/^ /, "", $0); printf "  %s\t%s\n", bytes, $0 }'
}

cargo_supports_dry_run() {
  "${cargo_bin}" clean --help 2>/dev/null | grep -q -- '--dry-run'
}

run_cargo_dry_run() {
  cargo_supports_dry_run || \
    fail "${cargo_bin} clean does not expose --dry-run; refusing selective cleanup"
  "${cargo_bin}" clean \
    --manifest-path "${manifest}" \
    --target-dir "${target_abs}" \
    --dry-run
}

print_status
print_largest_entries

if [[ "${command_name}" == "status" ]]; then
  exit 0
fi

printf '%s\n' 'cargo_clean_dry_run:'
run_cargo_dry_run

if [[ "${command_name}" == "plan" ]]; then
  if [[ "${eligible}" -eq 1 ]]; then
    printf '%s\n' 'decision=eligible-but-not-deleted'
    printf '%s\n' \
      'next=make target-aux-clean FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target'
  else
    printf '%s\n' 'decision=not-eligible'
    printf '%s\n' \
      'override=FOD_TARGET_AUX_CLEAN_FORCE=1 still requires FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target'
  fi
  exit 0
fi

[[ "${target_exists}" -eq 1 ]] || {
  printf '%s\n' 'Nothing to clean: allowlisted auxiliary target does not exist.'
  exit 0
}

if [[ "${eligible}" -ne 1 && "${force}" -ne 1 ]]; then
  fail "auxiliary target is not eligible: require both size >= ${min_size} bytes and inactivity >= ${min_age_days} days"
fi

[[ "${confirm}" == "clean-test-locking-target" ]] || \
  fail "explicit confirmation required: FOD_TARGET_AUX_CLEAN_CONFIRM=clean-test-locking-target"

before_size="${target_size}"
printf 'Cleaning allowlisted Cargo auxiliary target through Cargo: %s\n' "${target_abs}"
"${cargo_bin}" clean \
  --manifest-path "${manifest}" \
  --target-dir "${target_abs}"

after_size=0
if [[ -d "${target_abs}" ]]; then
  after_size="$(du -sb -- "${target_abs}" | awk '{print $1}')"
fi
reclaimed=$((before_size - after_size))
[[ "${reclaimed}" -ge 0 ]] || reclaimed=0
printf 'before_bytes=%s\n' "${before_size}"
printf 'after_bytes=%s\n' "${after_size}"
printf 'reclaimed_bytes=%s\n' "${reclaimed}"
printf 'reclaimed_human=%s\n' "$(human_bytes "${reclaimed}")"
