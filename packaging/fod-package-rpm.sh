#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$repo_root/packaging/fod-package-common.sh"

command -v rpmbuild >/dev/null || { echo 'Missing RPM packaging tool: rpmbuild (install rpm-build)' >&2; exit 1; }
command -v rpm >/dev/null || { echo 'Missing RPM packaging tool: rpm' >&2; exit 1; }
fod_package_check_links

work="$FOD_PACKAGE_ROOT/work/rpm"
stage="$work/root"
outdir="$FOD_PACKAGE_ROOT/rpm"
libdir="$(rpm --eval '%{_libdir}')"
licensedir="$(rpm --eval '%{_licensedir}')"
spec="$work/$FOD_PACKAGE_NAME.spec"
mkdir -p "$work/rpmbuild/BUILD" "$work/rpmbuild/BUILDROOT" "$work/rpmbuild/RPMS" "$work/rpmbuild/SOURCES" "$work/rpmbuild/SPECS" "$work/rpmbuild/SRPMS" "$outdir"
fod_package_stage_payload "$stage" "$libdir" "$licensedir"

cat > "$spec" <<EOF_SPEC
Name: $FOD_PACKAGE_NAME
Version: $FOD_PACKAGE_VERSION
Release: $FOD_PACKAGE_RELEASE%{?dist}
Summary: PostgreSQL-backed FUSE filesystem
License: BSL-1.1
URL: $FOD_PACKAGE_URL
%global debug_package %{nil}
Requires: bash
Requires: fuse3

%description
FOD provides a Rust FUSE filesystem backed by PostgreSQL storage.

%prep
%build
%install
rm -rf %{buildroot}
mkdir -p %{buildroot}
cp -a "%{fod_payload}/." %{buildroot}/

%files
%config(noreplace) %{_sysconfdir}/fod/fod_config.ini
%{_bindir}/fod-bootstrap
%{_bindir}/fod-change
%{_bindir}/fod.change
%{_bindir}/fod-indexer
%{_bindir}/fod-monitor
%{_bindir}/fod-rust-fuse
%{_sbindir}/mkfs.fod
%{_sbindir}/mount.fod
%{_libdir}/libfod.so
%{_includedir}/fod/libfod.h
%dir %{_localstatedir}/log/fod
%license %{_licensedir}/$FOD_PACKAGE_NAME/LICENSE
%doc %{_docdir}/$FOD_PACKAGE_NAME/README.md
EOF_SPEC

rpmbuild -bb \
  --define "_topdir $work/rpmbuild" \
  --define "_rpmdir $outdir" \
  --define "fod_payload $stage" \
  "$spec"

out="$(find "$outdir" -type f -name "$FOD_PACKAGE_NAME-$FOD_PACKAGE_VERSION-$FOD_PACKAGE_RELEASE*.rpm" ! -name '*.src.rpm' | sort | tail -n1)"
[[ -n "$out" ]] || { echo "No binary RPM found under $outdir" >&2; exit 1; }
rpm -qp "$out" >/dev/null
rpm -qlp "$out" >/dev/null
printf '%s\n' "$out"
