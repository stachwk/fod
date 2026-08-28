#!/usr/bin/env bash
set -euo pipefail

repo_root="${FOD_TEST_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
driver="$repo_root/scripts/fod-native-package.sh"
deb="$repo_root/packaging/fod-package-deb.sh"
rpm="$repo_root/packaging/fod-package-rpm.sh"
common="$repo_root/packaging/fod-package-common.sh"
makefile="$repo_root/GNUmakefile"
pmk="$repo_root/packaging/fod-packaging.mk"

for script in "$driver" "$deb" "$rpm" "$common"; do
  bash -n "$script"
done

deb_plan="$(bash "$driver" plan deb)"
rpm_plan="$(bash "$driver" plan rpm)"
grep -Fq 'resolved_format=deb' <<<"$deb_plan"
grep -Fq 'resolved_format=rpm' <<<"$rpm_plan"
grep -Fq 'package_release=2' <<<"$deb_plan"
grep -Fq 'production_profile=release-lto' <<<"$deb_plan"
grep -Fq 'fod_config.example.ini' <<<"$deb_plan"
grep -Fq 'Refusing cross-distro package build' "$driver"
grep -Fq 'dpkg-shlibdeps -O' "$deb"
grep -Fq 'stage_rel="debian/$FOD_PACKAGE_NAME"' "$deb"
grep -Fq -- '-e"$stage_rel/usr/bin/fod-bootstrap"' "$deb"
grep -Fq "printf '/etc/fod/fod_config.ini" "$deb"
grep -Fq 'activate-noawait ldconfig' "$deb"
grep -Fq '%config(noreplace) %{_sysconfdir}/fod/fod_config.ini' "$rpm"
grep -Fq '$stage/usr/sbin/mkfs.fod' "$common"
grep -Fq '$stage/usr/sbin/mount.fod' "$common"
grep -Fq '$stage/usr/bin/fod-rust-fuse' "$common"
grep -Fq '$stage/usr/include/fod/libfod.h' "$common"
grep -Fq 'find "$stage" -type d -exec chmod 0755 {} +' "$common"
grep -Fq 'FOD Project <33524981+stachwk@users.noreply.github.com>' "$driver"
grep -Fq 'FOD Project <33524981+stachwk@users.noreply.github.com>' "$pmk"
grep -Fq 'include packaging/fod-packaging.mk' "$makefile"
grep -Eq '^package-ubuntu: package-artifacts' "$pmk"
grep -Eq '^package-rocky: package-artifacts' "$pmk"
grep -Eq '^package-native: package-artifacts' "$pmk"
grep -Fq 'Official FOD packages require FOD_CARGO_PROFILE=release-lto' "$pmk"

if FOD_PACKAGE_ROOT=/tmp/fod-package-policy-outside bash "$driver" plan deb >/dev/null 2>&1; then
  echo 'package policy accepted package root outside repository target/' >&2
  exit 1
fi

# Reproduce a common developer umask and verify that staging normalizes all
# package payload directories to 0755 rather than leaking 0775 into artifacts.
tmp="$(mktemp -d "${TMPDIR:-/tmp}/fod-package-policy.XXXXXX")"
trap 'rm -rf -- "$tmp"' EXIT
export FOD_PACKAGE_NAME=fod
export FOD_PACKAGE_BOOTSTRAP_BIN=/bin/true
export FOD_PACKAGE_MKFS_BIN=/bin/true
export FOD_PACKAGE_CHANGE_BIN=/bin/true
export FOD_PACKAGE_INDEXER_BIN=/bin/true
export FOD_PACKAGE_MONITOR_BIN=/bin/true
export FOD_PACKAGE_FUSE_BIN=/bin/true
export FOD_PACKAGE_MOUNT_HELPER=/bin/true
export FOD_PACKAGE_LIBFOD_SO="$repo_root/LICENSE"
export FOD_PACKAGE_LIBFOD_HEADER="$repo_root/LICENSE"
export FOD_PACKAGE_CONFIG_SOURCE="$repo_root/fod_config.example.ini"
export FOD_PACKAGE_LICENSE_FILE="$repo_root/LICENSE"
export FOD_PACKAGE_README_FILE="$repo_root/README.md"
# shellcheck source=../packaging/fod-package-common.sh
. "$common"
(
  umask 0002
  fod_package_stage_payload "$tmp/root" /usr/lib/test /usr/share/doc
)
bad_dir="$(find "$tmp/root" -type d ! -perm 0755 -print -quit)"
if [[ -n "$bad_dir" ]]; then
  printf 'package staging left non-0755 directory: %s\n' "$bad_dir" >&2
  exit 1
fi

echo 'OK native-package-policy'
