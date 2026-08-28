#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

FOD_FINDMNT="${FOD_FINDMNT:-findmnt}"
FOD_MOUNTPOINT="${FOD_MOUNTPOINT:-mountpoint}"
FOD_FUSERMOUNT3="${FOD_FUSERMOUNT3:-fusermount3}"
FOD_FUSERMOUNT="${FOD_FUSERMOUNT:-fusermount}"
FOD_UMOUNT="${FOD_UMOUNT:-umount}"
FOD_TEST_FUSE_CLEANUP_RETRIES="${FOD_TEST_FUSE_CLEANUP_RETRIES:-5}"
FOD_TEST_FUSE_CLEANUP_SLEEP="${FOD_TEST_FUSE_CLEANUP_SLEEP:-0.10}"

case "${1:-clean}" in
  clean) ;;
  *)
    printf 'usage: %s [clean]\n' "$0" >&2
    exit 2
    ;;
esac

is_test_mount_path() {
  case "$1" in
    /tmp/fod-rust-fuse-*/mount) return 0 ;;
    *) return 1 ;;
  esac
}

is_mounted() {
  "${FOD_MOUNTPOINT}" -q -- "$1" >/dev/null 2>&1
}

list_test_mounts() {
  "${FOD_FINDMNT}" -rn -o TARGET,FSTYPE 2>/dev/null |
    awk '$1 ~ "^/tmp/fod-rust-fuse-[^/]+/mount$" && $2 ~ "^fuse([.]|$)" { print $1 }'
}

run_normal_unmounts() {
  local mountpoint="$1"
  local attempted=0

  if command -v "${FOD_FUSERMOUNT3}" >/dev/null 2>&1; then
    attempted=1
    "${FOD_FUSERMOUNT3}" -u "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi
  if command -v "${FOD_FUSERMOUNT}" >/dev/null 2>&1; then
    attempted=1
    "${FOD_FUSERMOUNT}" -u "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi
  if command -v "${FOD_UMOUNT}" >/dev/null 2>&1; then
    attempted=1
    "${FOD_UMOUNT}" "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi

  if (( attempted == 0 )); then
    printf 'FOD test cleanup: no unmount helper is available for %s\n' "$mountpoint" >&2
    return 1
  fi
  return 1
}

run_lazy_unmounts() {
  local mountpoint="$1"

  if command -v "${FOD_FUSERMOUNT3}" >/dev/null 2>&1; then
    "${FOD_FUSERMOUNT3}" -uz "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi
  if command -v "${FOD_FUSERMOUNT}" >/dev/null 2>&1; then
    "${FOD_FUSERMOUNT}" -uz "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi
  if command -v "${FOD_UMOUNT}" >/dev/null 2>&1; then
    "${FOD_UMOUNT}" -l "$mountpoint" >/dev/null 2>&1 || true
    is_mounted "$mountpoint" || return 0
  fi
  return 1
}

cleanup_mount() {
  local mountpoint="$1"
  local attempt

  is_test_mount_path "$mountpoint" || {
    printf 'FOD test cleanup: refusing non-test mount path: %s\n' "$mountpoint" >&2
    return 2
  }

  is_mounted "$mountpoint" || return 0
  printf 'FOD test cleanup: removing stale mount %s\n' "$mountpoint"

  for ((attempt = 1; attempt <= FOD_TEST_FUSE_CLEANUP_RETRIES; attempt++)); do
    run_normal_unmounts "$mountpoint" && break
    sleep "${FOD_TEST_FUSE_CLEANUP_SLEEP}"
  done

  if is_mounted "$mountpoint"; then
    run_lazy_unmounts "$mountpoint" || true
  fi

  for ((attempt = 1; attempt <= 20; attempt++)); do
    is_mounted "$mountpoint" || break
    sleep "${FOD_TEST_FUSE_CLEANUP_SLEEP}"
  done

  if is_mounted "$mountpoint"; then
    printf 'FOD test cleanup: mount is still active after normal and lazy unmount: %s\n' "$mountpoint" >&2
    "${FOD_FINDMNT}" -rn -T "$mountpoint" 2>/dev/null >&2 || true
    return 1
  fi

  rm -rf -- "$(dirname -- "$mountpoint")"
}

if ! command -v "${FOD_FINDMNT}" >/dev/null 2>&1; then
  printf 'FOD test cleanup: findmnt is required\n' >&2
  exit 1
fi
if ! command -v "${FOD_MOUNTPOINT}" >/dev/null 2>&1; then
  printf 'FOD test cleanup: mountpoint is required\n' >&2
  exit 1
fi

mapfile -t test_mounts < <(list_test_mounts)
for mountpoint in "${test_mounts[@]}"; do
  [[ -n "$mountpoint" ]] || continue
  cleanup_mount "$mountpoint"
done

mapfile -t remaining_mounts < <(list_test_mounts)
if ((${#remaining_mounts[@]} != 0)); then
  printf 'FOD test cleanup: stale FUSE test mounts remain:\n' >&2
  printf '  %s\n' "${remaining_mounts[@]}" >&2
  exit 1
fi
