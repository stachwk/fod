#!/bin/sh
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -eu

version="${FOD_LIBFUSE_VERSION:-3.18.3}"
case "${version}" in
    3.18.3)
        sha256="bcd19582c5e30f7fe45dd86a5540e998590aa01903afc7ebcbeea6c8ac5421ee"
        ;;
    *)
        echo "Unsupported unverified libfuse version: ${version}" >&2
        exit 2
        ;;
esac

build_dir="$(mktemp -d)"
trap 'rm -rf "${build_dir}"' EXIT HUP INT TERM
archive="${build_dir}/fuse-${version}.tar.gz"
curl -fsSL --retry 3 \
    "https://github.com/libfuse/libfuse/releases/download/fuse-${version}/fuse-${version}.tar.gz" \
    -o "${archive}"
printf '%s  %s\n' "${sha256}" "${archive}" | sha256sum --check --status
mkdir "${build_dir}/source" "${build_dir}/build"
tar -xzf "${archive}" --strip-components=1 -C "${build_dir}/source"
meson setup "${build_dir}/build" "${build_dir}/source" \
    --prefix=/usr/local --libdir=lib \
    -Dutils=true -Dexamples=false -Dtests=false -Duseroot=true
ninja -C "${build_dir}/build"
ninja -C "${build_dir}/build" install
ldconfig

actual="$(PKG_CONFIG_PATH=/usr/local/lib/pkgconfig pkg-config --modversion fuse3)"
if [ "${actual}" != "${version}" ]; then
    echo "libfuse version mismatch: expected=${version} actual=${actual}" >&2
    exit 1
fi
printf 'verified_libfuse_version=%s\n' "${actual}"

package_root="${build_dir}/package"
install -d "${package_root}/DEBIAN"
install -d \
    "${package_root}/usr/local/bin" \
    "${package_root}/usr/local/include" \
    "${package_root}/usr/local/lib/pkgconfig" \
    "${package_root}/usr/local/sbin" \
    "${package_root}/usr/local/share/man/man1" \
    "${package_root}/usr/local/share/man/man8"
cp -a /usr/local/bin/fusermount3 "${package_root}/usr/local/bin/"
cp -a /usr/local/include/fuse3 "${package_root}/usr/local/include/"
cp -a /usr/local/lib/libfuse3.so* "${package_root}/usr/local/lib/"
cp -a /usr/local/lib/pkgconfig/fuse3.pc "${package_root}/usr/local/lib/pkgconfig/"
cp -a /usr/local/sbin/mount.fuse3 "${package_root}/usr/local/sbin/"
cp -a /usr/local/share/man/man1/fusermount3.1 "${package_root}/usr/local/share/man/man1/"
cp -a /usr/local/share/man/man8/mount.fuse3.8 "${package_root}/usr/local/share/man/man8/"
architecture="$(dpkg --print-architecture)"
cat > "${package_root}/DEBIAN/control" <<EOF
Package: fuse3
Version: ${version}-1fod1
Section: utils
Priority: optional
Architecture: ${architecture}
Depends: libc6
Maintainer: FOD project
Description: libfuse3 ${version} built for FOD
 Custom libfuse3 runtime, utilities, headers and pkg-config metadata.
EOF
printf 'libfuse3 4 fuse3 (>= %s)\n' "${version}" > "${package_root}/DEBIAN/shlibs"
dpkg-deb --build --root-owner-group "${package_root}" \
    "/tmp/fuse3_${version}-1fod1_${architecture}.deb" >/dev/null
printf 'created_package=%s\n' "/tmp/fuse3_${version}-1fod1_${architecture}.deb"
