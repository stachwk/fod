#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/packaging/fod-package-common.sh"

for tool in dpkg-deb dpkg dpkg-architecture dpkg-shlibdeps; do
  command -v "$tool" >/dev/null || { echo "Missing Debian packaging tool: $tool (install dpkg-dev)" >&2; exit 1; }
done
fod_package_check_links

work="$FOD_PACKAGE_ROOT/work/deb"
stage="$work/root"
outdir="$FOD_PACKAGE_ROOT/deb"
arch="$(dpkg --print-architecture)"
multiarch="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
libdir="/usr/lib/$multiarch"
mkdir -p "$work/debian" "$outdir"
fod_package_stage_payload "$stage" "$libdir" /usr/share/doc

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

shlib_line="$(cd "$work" && dpkg-shlibdeps -O \
  -e"$stage/usr/bin/fod-bootstrap" \
  -e"$stage/usr/sbin/mkfs.fod" \
  -e"$stage/usr/bin/fod-change" \
  -e"$stage/usr/bin/fod-indexer" \
  -e"$stage/usr/bin/fod-monitor" \
  -e"$stage/usr/bin/fod-rust-fuse" \
  -e"$stage$libdir/libfod.so")"
deps="${shlib_line#shlibs:Depends=}"
[[ "$deps" != "$shlib_line" && -n "$deps" ]] || { echo 'dpkg-shlibdeps did not produce shlibs:Depends' >&2; exit 1; }

mkdir -p "$stage/DEBIAN"
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
