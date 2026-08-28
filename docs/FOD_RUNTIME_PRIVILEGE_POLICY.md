# FOD runtime privilege policy

Status: official operational policy for the current FOD deployment model.

## Decision

FOD mounts and the FOD FUSE runtime are operated as `root`.

The standard runtime ownership model is:

```text
fod-rust-fuse process   -> root
mount.fod / bootstrap   -> root
mount operation         -> root
/etc/fod                -> root controlled
/var/log/fod            -> root:root 0755
FOD log files           -> root owned, normally 0640
mounted FOD data        -> accessible according to FOD UID/GID/mode/ACL policy
```

A dedicated unprivileged `fod` service account is not part of the current deployment model.

## Separation of daemon privileges and data access

Running the FOD daemon as `root` does not mean that only `root` may use the mounted filesystem.

The identity that mounts and runs `fod-rust-fuse` is separate from access control applied to files and directories exposed through the FOD mount. Ordinary system users may read and write mounted FOD data when the effective FOD permissions allow it.

User access is controlled through the existing filesystem mechanisms, including:

- UID and GID ownership;
- file and directory mode bits;
- `default_permissions` where enabled;
- ACL support where enabled;
- `allow_other` where the mount is intentionally shared with users other than the mounting identity.

The runtime therefore remains privileged while data access is delegated according to the permissions of the mounted resource.

## Configuration ownership

Operational FOD configuration under `/etc/fod` is root-controlled.

The installation package may install a non-secret example/default INI with normal package-readable permissions. Once an INI contains database passwords, private key paths, certificates, tokens or other secrets, operators should restrict that operational file to `root` as appropriate for the deployment, normally `root:root` with mode `0600` or `0640`.

Multiple FOD instances may use different INI files. Runtime logging follows the selected INI, so separate configurations can naturally produce separate logs, for example:

```text
/etc/fod/db-primary.ini -> /var/log/fod/db-primary.log
/etc/fod/db-archive.ini -> /var/log/fod/db-archive.log
```

## Logging ownership

Native packages create:

```text
/var/log/fod
```

as `root:root` with mode `0755`.

Because the supported runtime model starts FOD as `root`, `fod-rust-fuse` can create and append its instance log files there without making the log directory group- or world-writable. New log files are requested with mode `0640`, subject to the process umask.

The log directory must not be made generally writable solely to support non-root FOD daemons. If a future deployment model introduces a dedicated service account, its ownership and access model must be designed explicitly rather than weakening `/var/log/fod` permissions globally.

## Why root is the current default

Keeping the daemon and mount operation under `root` avoids additional privilege plumbing around:

- `/dev/fuse` access;
- mountpoint ownership and mount permissions;
- `/etc/fod` configuration and database credentials;
- TLS certificates and private keys;
- `/var/log/fod` ownership;
- UID/GID and ACL behavior across mounted instances;
- service-manager and package-specific account provisioning.

This keeps the operational model simple while FOD's own permission model continues to control access to mounted data.

## Future dedicated service account

A dedicated `fod` user may be considered later as a security-hardening option. Such a change is not a simple ownership substitution and must be treated as a separate design task.

Before adopting it, FOD would need an explicit policy for at least:

- `/dev/fuse` and mount permissions;
- ownership/group of `/etc/fod` and secrets;
- ownership/group of `/var/log/fod`;
- mountpoint ownership;
- `allow_other` behavior;
- UID/GID mapping and ACL semantics;
- systemd or other service-manager integration;
- package creation/removal of the service account and group;
- upgrade and rollback behavior.

Until that work is deliberately implemented and tested, production FOD mounts are expected to run as `root`.

## Scope

This policy concerns runtime privileges and ownership only. It does not change the FOD storage format, PostgreSQL schema, 4 KiB storage block size, FUSE request-size defaults, or the Rust 1.98.0 + `release-lto` production build policy.
