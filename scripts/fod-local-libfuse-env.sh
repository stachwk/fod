#!/usr/bin/env bash
# Aktywacja projektowej biblioteki libfuse 3.18.3 dla kompilacji i testow.
# Ten plik trzeba wykonac przez source, aby ustawienia trafily do terminala.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  printf 'Use: source scripts/fod-local-libfuse-env.sh\n' >&2
  exit 2
fi

_fod_fuse_repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)" || return 1
_fod_fuse_root="${_fod_fuse_repo_root}/.fod/toolchains/libfuse-3.18.3"
_fod_fuse_lib="${_fod_fuse_root}/lib/x86_64-linux-gnu"
_fod_fuse_pc="${_fod_fuse_lib}/pkgconfig"

if [[ ! -f "${_fod_fuse_pc}/fuse3.pc" || ! -f "${_fod_fuse_lib}/libfuse3.so" ]]; then
  printf 'Missing local libfuse 3.18.3: run bash scripts/fod-local-libfuse-install.sh\n' >&2
  return 1
fi

# Sprawdzamy docelowy plik .pc, zanim zmienimy srodowisko terminala.
_fod_fuse_version="$(PKG_CONFIG_PATH="${_fod_fuse_pc}${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}" pkg-config --modversion fuse3)" || return 1
_fod_fuse_pc_dir="$(PKG_CONFIG_PATH="${_fod_fuse_pc}${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}" pkg-config --variable=pcfiledir fuse3)" || return 1
_fod_fuse_libdir="$(PKG_CONFIG_PATH="${_fod_fuse_pc}${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}" pkg-config --variable=libdir fuse3)" || return 1

if [[ "${_fod_fuse_version}" != "3.18.3" || "${_fod_fuse_pc_dir}" != "${_fod_fuse_pc}" || "${_fod_fuse_libdir}" != "${_fod_fuse_lib}" ]]; then
  printf 'Local libfuse metadata is inconsistent; rerun or inspect installer\n' >&2
  return 1
fi

export PKG_CONFIG_PATH="${_fod_fuse_pc}${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}"
export LD_LIBRARY_PATH="${_fod_fuse_lib}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"

printf 'FOD_LOCAL_LIBFUSE=3.18.3 (%s)\n' "${_fod_fuse_lib}"
