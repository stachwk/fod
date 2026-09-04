# FOD security and permissions

This document is the task-oriented entry point for security, privilege,
permissions and host policy. It summarizes the current operational model and
links to the detailed contracts that remain authoritative for each area.

## Runtime privilege model

The current supported FOD mount/runtime model is privileged:

```text
mount.fod / fod-bootstrap / fod-rust-fuse -> root
/etc/fod                                 -> root controlled
/var/log/fod                             -> root:root 0755
new FOD log files                        -> normally 0640
```

Running the daemon as `root` is separate from access to files exposed by the
mount. Mounted data is still governed by FOD/Linux ownership, mode bits, ACL
and mount access policy.

See [`FOD_RUNTIME_PRIVILEGE_POLICY.md`](FOD_RUNTIME_PRIVILEGE_POLICY.md) for
the authoritative operational policy.

## Configuration and secrets

Operational configuration under `/etc/fod` is root-controlled. A packaged
example/default INI may be readable as normal package data, but an operational
INI containing database passwords, certificate/private-key paths, tokens or
other secrets should be restricted appropriately, normally to `0600` or
`0640` according to deployment policy.

The Docker/systemd deployment keeps PostgreSQL and schema passwords in the
protected generated deployment state. `/etc/fod/docker-deploy.env` contains
topology, paths and the exact FOD image tag rather than duplicating those
passwords.

Runtime configuration precedence and environment-only controls are documented
in [`runtime-configuration.md`](runtime-configuration.md). PostgreSQL session
requirements are documented in
[`POSTGRESQL_REQUIREMENTS.md`](POSTGRESQL_REQUIREMENTS.md) and
[`postgresql-runtime-requirements.md`](postgresql-runtime-requirements.md).

## Docker and AppArmor

The FOD client container requires:

- `/dev/fuse`,
- `CAP_SYS_ADMIN`,
- `rshared` mount propagation,
- host AppArmor policy that allows the required FUSE mount operations.

AppArmor is host-managed. On an AppArmor-enabled Docker host the deployment
startup guard can use `apparmor=unconfined` when the host's default profile is
too restrictive. Operators may select `auto`, `unconfined` or `default`
through the documented deployment setting.

See [`DOCKER_FOD_INSTALL.md`](DOCKER_FOD_INSTALL.md) and
[`../docker/fod-client/README.md`](../docker/fod-client/README.md).

## SELinux on Rocky Linux

The verified Rocky Linux 10.2 model uses SELinux enforcement through the
ordinary FUSE `fusefs_t` type and normal SELinux domain policy.

This is not equivalent to the per-inode SELinux labeling model of ext4/XFS.
On the tested ordinary FUSE stack, per-file `security.selinux` relabeling is
not supported and the request does not reach the FOD userspace xattr callback.

The positive operational support path is host policy controlling access to
`fusefs_t` content. The detailed support definition, test targets and host
observations are in [`history/FOD_3_3_22_ROCKY_SELINUX.md`](history/FOD_3_3_22_ROCKY_SELINUX.md).

## UID/GID, mode bits, ACL and `allow_other`

The privileged daemon identity does not grant arbitrary users access to the
mounted filesystem. User-visible access remains controlled through the normal
filesystem mechanisms implemented/exposed by FOD:

- UID/GID ownership,
- file and directory mode bits,
- `default_permissions` where selected,
- ACL support where enabled,
- `allow_other` when a mount is intentionally shared beyond the mounting
  identity.

Do not weaken `/etc/fod` or `/var/log/fod` permissions merely to make mounted
data available to ordinary users. Those are separate concerns.

## Native packages

Native `.deb`/`.rpm` packages preserve the same root-operated runtime model and
install the non-secret example configuration rather than the developer's local
`fod_config.ini`.

See [`FOD_NATIVE_INSTALLATION_PACKAGES.md`](FOD_NATIVE_INSTALLATION_PACKAGES.md).

## Local test data privacy

Before using real local paths, databases or source trees in tests, follow
[`LOCAL_TEST_DATA_PRIVACY.md`](LOCAL_TEST_DATA_PRIVACY.md). Test fixtures and
published documentation must not accidentally capture local credentials or
private source data.

## Security change checklist

For a change that affects privileges, access control or host policy:

1. identify whether the decision belongs to FOD, Linux VFS/FUSE, Docker,
   AppArmor/SELinux or PostgreSQL;
2. keep runtime privilege and mounted-data permissions separate;
3. do not copy secrets into Compose/systemd files or documentation;
4. validate on the target host policy rather than assuming native-filesystem
   SELinux/AppArmor behavior;
5. run the relevant local policy/integration tests and document any
   host-dependent limitation explicitly.
