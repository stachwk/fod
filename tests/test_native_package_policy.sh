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
grep -Fq 'production_profile=release-lto' <<<"$deb_plan"
grep -Fq 'fod_config.example.ini' <<<"$deb_plan"
grep -Fq 'Refusing cross-distro package build' "$driver"
grep -Fq 'dpkg-shlibdeps -O' "$deb"
grep -Fq "printf '/etc/fod/fod_config.ini" "$deb"
grep -Fq '%config(noreplace) %{_sysconfdir}/fod/fod_config.ini' "$rpm"
grep -Fq '$stage/usr/sbin/mkfs.fod' "$common"
grep -Fq '$stage/usr/sbin/mount.fod' "$common"
grep -Fq '$stage/usr/bin/fod-rust-fuse' "$common"
grep -Fq '$stage/usr/include/fod/libfod.h' "$common"
grep -Fq 'include packaging/fod-packaging.mk' "$makefile"
grep -Eq '^package-ubuntu: package-artifacts' "$pmk"
grep -Eq '^package-rocky: package-artifacts' "$pmk"
grep -Eq '^package-native: package-artifacts' "$pmk"
grep -Fq 'Official FOD packages require FOD_CARGO_PROFILE=release-lto' "$pmk"

if FOD_PACKAGE_ROOT=/tmp/fod-package-policy-outside bash "$driver" plan deb >/dev/null 2>&1; then
  echo 'package policy accepted package root outside repository target/' >&2
  exit 1
fi

echo 'OK native-package-policy'
