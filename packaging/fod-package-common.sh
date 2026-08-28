#!/usr/bin/env bash
set -euo pipefail

fod_package_stage_payload() {
  local stage="$1" libdir="$2" licensedir="$3"
  rm -rf -- "$stage"
  mkdir -p -- "$stage"
  install -Dm0755 "$FOD_PACKAGE_BOOTSTRAP_BIN" "$stage/usr/bin/fod-bootstrap"
  install -Dm0755 "$FOD_PACKAGE_CHANGE_BIN" "$stage/usr/bin/fod-change"
  ln -s fod-change "$stage/usr/bin/fod.change"
  install -Dm0755 "$FOD_PACKAGE_INDEXER_BIN" "$stage/usr/bin/fod-indexer"
  install -Dm0755 "$FOD_PACKAGE_MONITOR_BIN" "$stage/usr/bin/fod-monitor"
  install -Dm0755 "$FOD_PACKAGE_FUSE_BIN" "$stage/usr/bin/fod-rust-fuse"
  install -Dm0755 "$FOD_PACKAGE_MKFS_BIN" "$stage/usr/sbin/mkfs.fod"
  install -Dm0755 "$FOD_PACKAGE_MOUNT_HELPER" "$stage/usr/sbin/mount.fod"
  install -Dm0644 "$FOD_PACKAGE_LIBFOD_SO" "$stage$libdir/libfod.so"
  install -Dm0644 "$FOD_PACKAGE_LIBFOD_HEADER" "$stage/usr/include/fod/libfod.h"
  install -Dm0644 "$FOD_PACKAGE_CONFIG_SOURCE" "$stage/etc/fod/fod_config.ini"
  install -Dm0644 "$FOD_PACKAGE_LICENSE_FILE" "$stage$licensedir/${FOD_PACKAGE_NAME}/LICENSE"
  install -Dm0644 "$FOD_PACKAGE_README_FILE" "$stage/usr/share/doc/${FOD_PACKAGE_NAME}/README.md"

  # Package payload directory modes must not depend on the caller's umask.
  # In particular, developer shells commonly use umask 0002, which otherwise
  # produces group-writable 0775 directories in DEB/RPM artifacts.
  find "$stage" -type d -exec chmod 0755 {} +
}

fod_package_check_links() {
  command -v ldd >/dev/null 2>&1 || return 0
  local f out
  for f in "$FOD_PACKAGE_BOOTSTRAP_BIN" "$FOD_PACKAGE_MKFS_BIN" "$FOD_PACKAGE_CHANGE_BIN" "$FOD_PACKAGE_INDEXER_BIN" "$FOD_PACKAGE_MONITOR_BIN" "$FOD_PACKAGE_FUSE_BIN" "$FOD_PACKAGE_LIBFOD_SO"; do
    out="$(ldd "$f" 2>&1 || true)"
    if grep -Fq 'not found' <<<"$out"; then
      printf 'Unresolved shared library dependency in %s:\n%s\n' "$f" "$out" >&2
      exit 1
    fi
  done
}
