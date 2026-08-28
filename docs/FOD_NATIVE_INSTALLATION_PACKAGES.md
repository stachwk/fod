# Native installation packages for Ubuntu and RHEL/RockyLinux

Status: packaging infrastructure for the existing FOD runtime. The FOD runtime version is not changed by this packaging-only work.

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

For Debian packages `/etc/fod/fod_config.ini` is declared in `DEBIAN/conffiles`. For RPM it is `%config(noreplace)`, so a locally edited configuration is preserved across upgrades. Debian package dependencies for ELF libraries are generated with `dpkg-shlibdeps`; this also avoids hard-coding a specific Ubuntu FUSE library package name when the distribution changes SONAME packages. RPM keeps automatic ELF dependency generation and adds explicit `bash` and `fuse3` runtime requirements.

## Output

Artifacts are written below:

```text
target/packages/deb/
target/packages/rpm/
```

A package build never installs files into the running system and does not require `sudo`. Installation is a separate operator action with `apt/dpkg` or `dnf/rpm`.
