# Native installation packages for Ubuntu and RHEL/RockyLinux

Status: packaging infrastructure for the existing FOD runtime. The FOD runtime remains version 3.3.30; packaging-only corrections use the native package release suffix. Current package revision is `3.3.30-2`.

## Targets

```bash
make package-plan
make package-deps-ubuntu
make package-ubuntu
make package-deps-redhat
make package-rocky
make package-native
make package-clean
```

Aliases:

```text
package-deb     -> package-ubuntu
package-rpm     -> package-rocky
package-redhat  -> package-rocky
```

`package-native` detects the host family from `/etc/os-release`.

## Native-build rule

Binary packages are intentionally built only on the target distribution family:

- `.deb` on Ubuntu/Debian;
- `.rpm` on RHEL/Rocky/Fedora-family systems.

The builder rejects cross-distro repackaging. FOD is dynamically linked to host libraries such as glibc, libpq and FUSE; wrapping an Ubuntu-built binary in an RPM would not prove that it is compatible with RockyLinux.

Official package targets keep the project production build contract:

```text
Rust 1.98.0
FOD_CARGO_PROFILE=release-lto
```

The generated ELF dependency requirements describe the host used for the native build. For example, a package built against a newer FUSE SONAME is not automatically suitable for an older Ubuntu/RHEL release. Distributable packages should therefore be built and tested on each supported target release, or on the oldest intentionally supported release when ABI compatibility permits it.

## Package contents

The native packages install:

```text
/usr/bin/fod-bootstrap
/usr/bin/fod-change
/usr/bin/fod.change -> fod-change
/usr/bin/fod-indexer
/usr/bin/fod-monitor
/usr/bin/fod-rust-fuse
/usr/sbin/mkfs.fod
/usr/sbin/mount.fod
/usr/include/fod/libfod.h
<distribution libdir>/libfod.so
/etc/fod/fod_config.ini
```

The packaged configuration comes from `fod_config.example.ini`, never from the local `fod_config.ini`. This prevents local database passwords from being copied into an installation package.

For Debian packages `/etc/fod/fod_config.ini` is declared in `DEBIAN/conffiles`. For RPM it is `%config(noreplace)`, so a locally edited configuration is preserved across upgrades. Debian package dependencies for ELF libraries are generated with `dpkg-shlibdeps`; this also avoids hard-coding a specific Ubuntu FUSE library package name when the distribution changes SONAME packages. Because the package installs `libfod.so` into the dynamic-linker library path, the Debian package also declares the `activate-noawait ldconfig` trigger. On RHEL/Rocky 8 and newer, glibc transaction file triggers update the `ldconfig` cache, so no package-specific RPM scriptlet is added. RPM keeps automatic ELF dependency generation and adds explicit `bash` and `fuse3` runtime requirements.

## Debian staging and permissions

The Debian builder stages payload files under the conventional path:

```text
target/packages/work/deb/debian/fod/
```

`dpkg-shlibdeps` is invoked from the Debian build root with `debian/fod/...` relative ELF paths. This lets the tool associate each binary with package `fod` and avoids the `binaries to analyze should already be installed in their package's directory` warning caused by staging payloads in an unrelated `work/deb/root/` directory.

All payload directories are normalized to mode `0755` after staging. This is deliberate: package contents must not inherit a developer shell's `umask 0002` and become group-writable `0775` directories. Regular executable files remain `0755`; configuration, documentation, headers and `libfod.so` remain `0644`.

On merged-/usr Ubuntu systems `dpkg-shlibdeps` can still report a libc loader diversion such as `/lib64/ld-linux-x86-64.so.2` -> `.usr-is-merged`. That warning describes the host's dpkg/libc diversion state and is separate from the package-directory staging warning. It is not suppressed; generated dependencies must still be reviewed in the resulting package metadata.

The default package maintainer is the repository's GitHub noreply identity and can be overridden with `FOD_PACKAGE_MAINTAINER`.

## Package revision

`FOD_PACKAGE_RELEASE` is incremented for packaging-only corrections that do not change FOD runtime code or ABI. Consequently this staging/metadata correction produces:

```text
FOD runtime:       3.3.30
DEB/RPM revision:  3.3.30-2
```

A later runtime version resets/chooses its package release independently according to the packaging change being published.

## Output

Artifacts are written below:

```text
target/packages/deb/
target/packages/rpm/
```

A package build never installs files into the running system and does not require `sudo`. Installation is a separate operator action with `apt/dpkg` or `dnf/rpm`.
