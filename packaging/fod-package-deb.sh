#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/packaging/fod-package-common.sh"

for tool in dpkg-deb dpkg dpkg-architecture dpkg-shlibdeps; do
  command -v "$tool" >/dev/null || { echo "Missing Debian packaging tool: $tool (install dpkg-dev)" >&2; exit 1; }
done
fod_package_check_links

work="$FOD_PACKAGE_ROOT/work/deb"
stage_rel="debian/$FOD_PACKAGE_NAME"
stage="$work/$stage_rel"
outdir="$FOD_PACKAGE_ROOT/deb"
arch="$(dpkg --print-architecture)"
multiarch="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
libdir="/usr/lib/$multiarch"
mkdir -p "$work/debian" "$outdir"
fod_package_stage_payload "$stage" "$libdir" /usr/share/doc
# Dpkg identifies a binary package build root by its DEBIAN/ directory.
# Create it before dpkg-shlibdeps scans the staged ELF objects.
install -d -m0755 "$stage/DEBIAN"

cat > "$work/debian/control" <<EOF_CONTROL
Source: $FOD_PACKAGE_NAME
Section: utils
Priority: optional
Maintainer: $FOD_PACKAGE_MAINTAINER
Standards-Version: 4.7.0

Package: $FOD_PACKAGE_NAME
Architecture: any
Description: PostgreSQL-backed FUSE filesystem
 FOD provides a Rust FUSE filesystem backed by PostgreSQL storage.
EOF_CONTROL

# Keep the payload in the conventional debian/<package>/ tree and pass paths
# relative to the Debian build root. With DEBIAN/ already present,
# dpkg-shlibdeps can associate every ELF object with package fod.
shlib_line="$(cd "$work" && dpkg-shlibdeps -O \
  -e"$stage_rel/usr/bin/fod-bootstrap" \
  -e"$stage_rel/usr/sbin/mkfs.fod" \
  -e"$stage_rel/usr/bin/fod-change" \
  -e"$stage_rel/usr/bin/fod-indexer" \
  -e"$stage_rel/usr/bin/fod-monitor" \
  -e"$stage_rel/usr/bin/fod-rust-fuse" \
  -e"$stage_rel$libdir/libfod.so")"
deps="${shlib_line#shlibs:Depends=}"
[[ "$deps" != "$shlib_line" && -n "$deps" ]] || { echo 'dpkg-shlibdeps did not produce shlibs:Depends' >&2; exit 1; }

cat > "$stage/DEBIAN/control" <<EOF_CONTROL
Package: $FOD_PACKAGE_NAME
Version: $FOD_PACKAGE_VERSION-$FOD_PACKAGE_RELEASE
Section: utils
Priority: optional
Architecture: $arch
Maintainer: $FOD_PACKAGE_MAINTAINER
Depends: $deps, bash, fuse3
Homepage: $FOD_PACKAGE_URL
Description: PostgreSQL-backed FUSE filesystem
 FOD provides a Rust FUSE filesystem backed by PostgreSQL storage.
EOF_CONTROL
printf '/etc/fod/fod_config.ini\n' > "$stage/DEBIAN/conffiles"
printf 'activate-noawait ldconfig\n' > "$stage/DEBIAN/triggers"

out="$outdir/${FOD_PACKAGE_NAME}_${FOD_PACKAGE_VERSION}-${FOD_PACKAGE_RELEASE}_${arch}.deb"
dpkg-deb --build --root-owner-group "$stage" "$out"
dpkg-deb --info "$out" >/dev/null
dpkg-deb --contents "$out" >/dev/null
printf '%s\n' "$out"
