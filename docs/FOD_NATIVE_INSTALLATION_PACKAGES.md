# Native installation packages for Ubuntu and RHEL/RockyLinux

Status: current native packaging infrastructure. Package builds derive the FOD runtime version from the repository release metadata; the `3.4.1-1` values below are retained only as the historical example that introduced this packaging layout.

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
/var/log/fod/
```

The packaged configuration comes from `fod_config.example.ini`, never from the local `fod_config.ini`. This prevents local database passwords from being copied into an installation package.

FOD 3.3.31 adds INI-controlled per-instance file logging. The package creates `/var/log/fod` with mode `0755`; individual log files are opened by `fod-rust-fuse` in append mode and are created with mode `0640` subject to the system umask. The default packaged INI enables file logging and derives the file name from the selected INI basename. For example, `/etc/fod/db-primary.ini` writes to `/var/log/fod/db-primary.log`.

## Runtime privilege model

The supported deployment model runs `mount.fod`, `fod-bootstrap` and `fod-rust-fuse` as `root`. The package therefore deliberately keeps `/var/log/fod` as `root:root 0755`; it is not made generally writable for non-root daemons.

Operational configuration under `/etc/fod` is root-controlled. The packaged INI is a non-secret template/default configuration; once an operational INI contains database passwords, private key locations, tokens or other secrets, it should be restricted to `root` according to deployment policy, normally with mode `0600` or `0640`.

The privileged runtime identity is independent of access to mounted FOD data. Ordinary users may use the mounted filesystem according to effective UID/GID ownership, mode bits, `default_permissions`, ACL and `allow_other` settings.

A dedicated `fod` service account is intentionally deferred. Introducing one requires a separate design for `/dev/fuse`, configuration secrets, log ownership, mountpoints, UID/GID/ACL behavior and service-manager/package lifecycle.

See [FOD_RUNTIME_PRIVILEGE_POLICY.md](FOD_RUNTIME_PRIVILEGE_POLICY.md) for the authoritative policy.

For Debian packages `/etc/fod/fod_config.ini` is declared in `DEBIAN/conffiles`. For RPM it is `%config(noreplace)`, so a locally edited configuration is preserved across upgrades. Debian package dependencies for ELF libraries are generated with `dpkg-shlibdeps`; this also avoids hard-coding a specific Ubuntu FUSE library package name when the distribution changes SONAME packages. Because the package installs `libfod.so` into the dynamic-linker library path, the Debian package also declares the `activate-noawait ldconfig` trigger. On RHEL/Rocky 8 and newer, glibc transaction file triggers update the `ldconfig` cache, so no package-specific RPM scriptlet is added. RPM keeps automatic ELF dependency generation and adds explicit `bash` and `fuse3` runtime requirements.

## Debian staging and permissions

The Debian builder stages payload files under the conventional path:

```text
target/packages/work/deb/debian/fod/
```

The staged package root creates `debian/fod/DEBIAN/` before `dpkg-shlibdeps` scans ELF objects. Dpkg uses that directory to recognize the binary package build root. The ELF paths passed to `dpkg-shlibdeps` are relative to the Debian build root (`debian/fod/...`). Together these rules prevent the `binaries to analyze should already be installed in their package's directory` warning.

All payload directories are normalized to mode `0755` after staging. This is deliberate: package contents must not inherit a developer shell's `umask 0002` and become group-writable `0775` directories. Regular executable files remain `0755`; configuration, documentation, headers and `libfod.so` remain `0644`.

On merged-/usr Ubuntu systems `dpkg-shlibdeps` can still report a libc loader diversion such as `/lib64/ld-linux-x86-64.so.2` -> `.usr-is-merged`. That warning describes the host's dpkg/libc diversion state and is separate from package-directory staging. It is not suppressed; generated dependencies must still be reviewed in the resulting package metadata.

The default package maintainer is the repository's GitHub noreply identity and can be overridden with `FOD_PACKAGE_MAINTAINER`.

## Package revision

The native packaging layout was introduced with the historical example:

```text
FOD runtime:       3.4.1
DEB/RPM revision:  3.4.1-1
```

Current package builds use the current FOD repository release rather than treating `3.4.1` as a fixed package runtime. Repository changes continue to follow [`versioning.md`](versioning.md): every FOD commit increments the FOD patch version and keeps package/release metadata aligned.

## Output

Artifacts are written below:

```text
target/packages/deb/
target/packages/rpm/
```

A package build never installs files into the running system and does not require `sudo`. Installation is a separate operator action with `apt/dpkg` or `dnf/rpm`.
