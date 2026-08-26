#!/usr/bin/env bash
set -euo pipefail

command_name="${1:-check}"
case "${command_name}" in
  check|status|clean) ;;
  *)
    echo "usage: $0 [check|status|clean]" >&2
    exit 2
    ;;
esac

uid="$(id -u)"
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)"
repo_root="$(realpath -m "${repo_root}")"

shm_root="${FOD_SHM_TARGET_ROOT:-/dev/shm}"
repo_key="$(printf '%s\n' "${repo_root}" | cksum | awk '{print $1}')"
default_target="${shm_root}/fod-target-${uid}-${repo_key}"
target="${CARGO_TARGET_DIR:-${FOD_SHM_TARGET_DIR:-${default_target}}}"
min_free="${FOD_SHM_MIN_FREE_BYTES:-2147483648}"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

[[ -d "${shm_root}" ]] || fail "tmpfs root does not exist: ${shm_root}"
[[ -w "${shm_root}" ]] || fail "tmpfs root is not writable: ${shm_root}"

root_abs="$(realpath -e "${shm_root}")"
target_abs="$(realpath -m "${target}")"

case "${target_abs}/" in
  "${root_abs}/"*) ;;
  *) fail "Cargo target must stay below ${root_abs}; got ${target_abs}" ;;
esac
[[ "${target_abs}" != "${root_abs}" ]] || fail "refusing to use tmpfs root itself as Cargo target"

fs_type="$(stat -f -c %T "${root_abs}")"
[[ "${fs_type}" == "tmpfs" ]] || \
  fail "${root_abs} is filesystem type ${fs_type}, expected tmpfs"

case "${min_free}" in
  ''|*[!0-9]*) fail "FOD_SHM_MIN_FREE_BYTES must be an integer, got ${min_free}" ;;
esac

marker="${target_abs}/.fod-shm-target"

validate_marker() {
  local marker_lines=()
  [[ -f "${marker}" ]] || fail "missing shm target marker: ${marker}"
  mapfile -t marker_lines < "${marker}"
  [[ "${marker_lines[0]:-}" == "FOD_SHM_TARGET_V1" ]] || \
    fail "invalid shm target marker: ${marker}"
  [[ "${marker_lines[1]:-}" == "${repo_root}" ]] || \
    fail "shm target belongs to another repository: ${marker_lines[1]:-unknown}"
  [[ "${marker_lines[2]:-}" == "${uid}" ]] || \
    fail "shm target belongs to another uid: ${marker_lines[2]:-unknown}"
}

target_size_bytes() {
  if [[ -d "${target_abs}" ]]; then
    du -sb "${target_abs}" 2>/dev/null | awk '{print $1}'
  else
    printf '0\n'
  fi
}

available_bytes() {
  df -PB1 "${root_abs}" | awk 'NR == 2 {print $4}'
}

print_status() {
  local available size managed
  available="$(available_bytes)"
  size="$(target_size_bytes)"
  managed="no"
  if [[ -f "${marker}" ]]; then
    managed="yes"
  fi
  printf 'shm_root=%s\n' "${root_abs}"
  printf 'shm_fs_type=%s\n' "${fs_type}"
  printf 'shm_available_bytes=%s\n' "${available}"
  printf 'shm_min_free_bytes=%s\n' "${min_free}"
  printf 'cargo_target_dir=%s\n' "${target_abs}"
  printf 'cargo_target_size_bytes=%s\n' "${size}"
  printf 'cargo_target_managed=%s\n' "${managed}"
}

if [[ "${command_name}" == "status" ]]; then
  if [[ -f "${marker}" ]]; then
    validate_marker
  fi
  print_status
  exit 0
fi

if [[ "${command_name}" == "clean" ]]; then
  [[ -d "${target_abs}" ]] || {
    printf 'Cargo shm target already absent: %s\n' "${target_abs}"
    exit 0
  }
  [[ -f "${marker}" ]] || \
    fail "refusing to clean unmarked directory: ${target_abs}"

  validate_marker

  rm -rf --one-file-system -- "${target_abs}"
  printf 'Removed managed Cargo shm target: %s\n' "${target_abs}"
  exit 0
fi

available="$(available_bytes)"
if (( available < min_free )); then
  fail "not enough free tmpfs space: available=${available}, required=${min_free}"
fi

if [[ -e "${target_abs}" && ! -d "${target_abs}" ]]; then
  fail "Cargo target path exists but is not a directory: ${target_abs}"
fi

if [[ -f "${marker}" ]]; then
  validate_marker
elif [[ -d "${target_abs}" ]]; then
  if find "${target_abs}" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
    fail "refusing to adopt non-empty unmarked directory: ${target_abs}"
  fi
fi

mkdir -p "${target_abs}"
owner_uid="$(stat -c %u "${target_abs}")"
[[ "${owner_uid}" == "${uid}" ]] || \
  fail "Cargo target is owned by uid ${owner_uid}, current uid is ${uid}"

if [[ ! -f "${marker}" ]]; then
  printf 'FOD_SHM_TARGET_V1\n%s\n%s\n' "${repo_root}" "${uid}" > "${marker}"
fi

probe="${target_abs}/.fod-exec-probe-$$"
cleanup_probe() {
  rm -f -- "${probe}"
}
trap cleanup_probe EXIT
cat > "${probe}" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 700 "${probe}"
if ! "${probe}"; then
  fail "${root_abs} does not allow executing build artifacts (likely mounted noexec)"
fi
cleanup_probe
trap - EXIT

print_status
printf 'Cargo shm target preflight: OK\n'
