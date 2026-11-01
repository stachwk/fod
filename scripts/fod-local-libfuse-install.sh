#!/usr/bin/env bash
# Instalacja lokalnej kopii libfuse dla buildu FOD.
# Kopia trafia do ignorowanego przez Git katalogu .fod/toolchains.
# Nie modyfikujemy systemowego libfuse ani nie usuwamy instalacji zrodlowej.
set -euo pipefail

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source_root="/var/tmp/fod-libfuse-3.18.3-install"
toolchains_root="${repo_root}/.fod/toolchains"
target_root="${toolchains_root}/libfuse-3.18.3"
lib_relative="lib/x86_64-linux-gnu"
pc_relative="${lib_relative}/pkgconfig/fuse3.pc"

[[ -d "${source_root}" ]] || fail "missing libfuse 3.18.3 source install: ${source_root}"
[[ -f "${source_root}/${pc_relative}" ]] || fail "missing fuse3.pc in source install"
[[ -d "${source_root}/include/fuse3" ]] || fail "missing libfuse headers in source install"
[[ -f "${source_root}/${lib_relative}/libfuse3.so" ]] || fail "missing libfuse shared library in source install"

# Symlinki w katalogu docelowym moglyby przeniesc zapis poza repozytorium.
[[ ! -L "${repo_root}/.fod" ]] || fail "refusing .fod symlink"
[[ ! -L "${toolchains_root}" ]] || fail "refusing toolchains symlink"
[[ ! -e "${target_root}" && ! -L "${target_root}" ]] || fail "target exists; will not overwrite: ${target_root}"

# Sciezki te sa podstawiane do sed; odrzucamy znaki zmieniajace replacement.
case "${repo_root}" in
  *'|'*|*'&'*|*'\'*|*$'\n'*) fail "unsupported repository path for pkg-config relocation" ;;
esac

mkdir -p -- "${toolchains_root}"
stage="$(mktemp -d "${toolchains_root}/.libfuse-3.18.3.stage.XXXXXX")"
cleanup() {
  if [[ -n "${stage:-}" && -d "${stage}" ]]; then
    rm -rf -- "${stage}"
  fi
}
trap cleanup EXIT

cp -a -- "${source_root}/." "${stage}/"
stage_pc="${stage}/${pc_relative}"

# Meson zapisuje w pliku .pc absolutny prefix starego katalogu.
# Zmieniamy go na docelowy katalog przed atomowym opublikowaniem kopii.
grep -Fq -- "${source_root}" "${stage_pc}" ||
  fail "fuse3.pc has no expected source prefix; inspect it manually"
sed -i "s|${source_root}|${target_root}|g" "${stage_pc}"
if grep -Fq -- "${source_root}" "${stage_pc}"; then
  fail "old libfuse prefix remains in fuse3.pc"
fi

mv -- "${stage}" "${target_root}"
stage=""

pc_dir="${target_root}/${lib_relative}/pkgconfig"
found_version="$(PKG_CONFIG_PATH="${pc_dir}" pkg-config --modversion fuse3)"
found_libdir="$(PKG_CONFIG_PATH="${pc_dir}" pkg-config --variable=libdir fuse3)"
found_includedir="$(PKG_CONFIG_PATH="${pc_dir}" pkg-config --variable=includedir fuse3)"
[[ "${found_version}" == "3.18.3" ]] || fail "unexpected libfuse version: ${found_version}"
[[ "${found_libdir}" == "${target_root}/${lib_relative}" ]] ||
  fail "libdir still points outside project: ${found_libdir}"
[[ "${found_includedir}" == "${target_root}/include" ]] ||
  fail "includedir still points outside project: ${found_includedir}"

printf 'LOCAL_LIBFUSE=PASS version=%s path=%s\n' "${found_version}" "${target_root}"
printf 'Activate it in this shell: source scripts/fod-local-libfuse-env.sh\n'
printf 'Source copy at %s was preserved (no automatic deletion).\n' "${source_root}"
