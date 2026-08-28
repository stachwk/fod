#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_root="${FOD_PACKAGE_ROOT:-${repo_root}/target/packages}"
export FOD_PACKAGE_NAME="${FOD_PACKAGE_NAME:-fod}"
export FOD_PACKAGE_VERSION="${FOD_PACKAGE_VERSION:-$(cat "${repo_root}/fod_version.txt")}"
export FOD_PACKAGE_RELEASE="${FOD_PACKAGE_RELEASE:-1}"
export FOD_PACKAGE_MAINTAINER="${FOD_PACKAGE_MAINTAINER:-FOD Project <maintainer@fod.invalid>}"
export FOD_PACKAGE_URL="${FOD_PACKAGE_URL:-https://github.com/stachwk/fod}"
export FOD_PACKAGE_CONFIG_SOURCE="${FOD_PACKAGE_CONFIG_SOURCE:-${repo_root}/fod_config.example.ini}"
export FOD_PACKAGE_MOUNT_HELPER="${FOD_PACKAGE_MOUNT_HELPER:-${repo_root}/mount.fod}"
export FOD_PACKAGE_LICENSE_FILE="${FOD_PACKAGE_LICENSE_FILE:-${repo_root}/LICENSE}"
export FOD_PACKAGE_README_FILE="${FOD_PACKAGE_README_FILE:-${repo_root}/README.md}"

canonical_root() {
  local root="$package_root"
  [[ "$root" == /* ]] || root="${repo_root}/${root}"
  realpath -m -- "$root"
}
check_root() {
  local resolved expected
  resolved="$(canonical_root)"
  expected="$(realpath -m -- "${repo_root}/target")/"
  case "${resolved}/" in "${expected}"*) ;; *)
    printf 'FOD package root must stay below %s: %s\n' "${repo_root}/target" "$resolved" >&2
    exit 2;;
  esac
}
host_family() {
  [[ -r /etc/os-release ]] || { echo unknown; return; }
  . /etc/os-release
  local tokens=" ${ID:-unknown} ${ID_LIKE:-} "
  if [[ "$tokens" == *" ubuntu "* || "$tokens" == *" debian "* ]]; then echo deb; return; fi
  if [[ "$tokens" == *" rhel "* || "$tokens" == *" rocky "* || "$tokens" == *" fedora "* || "$tokens" == *" centos "* || "$tokens" == *" almalinux "* || "$tokens" == *" ol "* ]]; then echo rpm; return; fi
  echo unknown
}
resolve_format() {
  local value="${1:-native}"
  [[ "$value" == native ]] && value="$(host_family)"
  case "$value" in deb|rpm) echo "$value";; *) echo "Unsupported package format/host: $value" >&2; exit 2;; esac
}
require_native() {
  local requested="$1" actual
  actual="$(host_family)"
  [[ "$actual" == "$requested" ]] || {
    printf 'Refusing cross-distro package build: requested=%s host_family=%s\n' "$requested" "$actual" >&2
    exit 2
  }
}
check_inputs() {
  local key value
  for key in FOD_PACKAGE_BOOTSTRAP_BIN FOD_PACKAGE_MKFS_BIN FOD_PACKAGE_CHANGE_BIN FOD_PACKAGE_INDEXER_BIN FOD_PACKAGE_MONITOR_BIN FOD_PACKAGE_FUSE_BIN FOD_PACKAGE_MOUNT_HELPER; do
    value="${!key:-}"
    [[ -n "$value" && -x "$value" ]] || { printf 'Missing package executable %s=%s\n' "$key" "$value" >&2; exit 1; }
  done
  for key in FOD_PACKAGE_LIBFOD_SO FOD_PACKAGE_LIBFOD_HEADER FOD_PACKAGE_CONFIG_SOURCE FOD_PACKAGE_LICENSE_FILE FOD_PACKAGE_README_FILE; do
    value="${!key:-}"
    [[ -n "$value" && -f "$value" ]] || { printf 'Missing package input %s=%s\n' "$key" "$value" >&2; exit 1; }
  done
}

check_root
case "${1:-plan}" in
  plan)
    fmt="$(resolve_format "${2:-native}")"
    printf 'package_name=%s\npackage_version=%s\npackage_release=%s\nhost_family=%s\nresolved_format=%s\npackage_root=%s\nconfig_source=%s\nproduction_profile=release-lto\n' \
      "$FOD_PACKAGE_NAME" "$FOD_PACKAGE_VERSION" "$FOD_PACKAGE_RELEASE" "$(host_family)" "$fmt" "$(canonical_root)" "$FOD_PACKAGE_CONFIG_SOURCE"
    ;;
  build)
    fmt="$(resolve_format "${2:-native}")"
    require_native "$fmt"
    check_inputs
    export FOD_PACKAGE_ROOT="$(canonical_root)"
    bash "${repo_root}/packaging/fod-package-${fmt}.sh"
    ;;
  clean)
    resolved="$(canonical_root)"
    [[ ! -d "$resolved" ]] || rm -rf -- "$resolved"
    ;;
  *)
    echo 'usage: fod-native-package.sh <plan|build|clean> [native|deb|rpm]' >&2
    exit 2
    ;;
esac
