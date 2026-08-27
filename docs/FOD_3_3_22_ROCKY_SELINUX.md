# FOD 3.3.22 Rocky Linux 10.2 SELinux Support

## Support Definition

FOD 3.3.22 supports SELinux operational enforcement on Rocky Linux 10.2
through the host FUSE `fusefs_t` label and normal SELinux domain policy.
Per-inode `security.selinux` labeling is not supported by the tested Rocky
FUSE/SELinux mount model and is a separate host/mount-stack capability.

## Verified Host Model

The tested host was Rocky Linux 10.2 with SELinux in `Enforcing` mode. The
targeted policy classifies ordinary FUSE through `genfscon fuse / ... fusefs_t`
instead of `fs_use_xattr`.

Practical consequences:

- files visible through an ordinary FOD FUSE mount are labeled
  `system_u:object_r:fusefs_t:s0`;
- `getxattr security.selinux` returns the synthetic genfs label;
- `setxattr security.selinux` returns `ENOTSUP` before FOD receives a
  `FUSE_SETXATTR` callback;
- ordinary user xattrs, such as `user.proof`, still reach FOD normally;
- native xattr-capable filesystems such as `/tmp` can accept a comparable
  `security.selinux` relabel on the same host.

This makes per-file SELinux relabeling a host/mount-stack capability, not a
change FOD can enable inside its userspace xattr callback.

## Operational SELinux Enforcement

The positive support path is policy-based access to `fusefs_t` content. The
Rocky operational test mounts FOD with `FOD_ALLOW_OTHER=1`, `FOD_ACL=on`, and
`FOD_SELINUX=on`, then serves a file from Apache running in `httpd_t`.

Observed behavior:

| Policy state | Result |
| --- | --- |
| `httpd_use_fusefs=off` | Apache returns `403` |
| `httpd_use_fusefs=on` | Apache returns `200` and serves `fod-httpd-ok` |

The test restores `httpd_use_fusefs=off`, stops Apache, and unmounts FOD at
the end.

## Test Targets

Use these targets on the Rocky host:

```bash
make rocky-selinux-prepare
make rocky-selinux-test-operational
make rocky-selinux-test-strict
```

Use these wrappers from the development machine:

```bash
make remote-rocky-selinux-prepare ROCKY_SELINUX_HOST=192.168.1.188
make remote-rocky-selinux-test-operational ROCKY_SELINUX_HOST=192.168.1.188
make remote-rocky-selinux-test-strict ROCKY_SELINUX_HOST=192.168.1.188
```

`rocky-selinux-test-operational` is the positive support gate for Rocky Linux
10.2. `rocky-selinux-test-strict` is a diagnostic for real per-inode
`security.selinux` support and is expected to fail on the tested ordinary FUSE
model.

## Mount Options

`FOD_SELINUX_CONTEXT`, `FOD_SELINUX_FSCONTEXT`, `FOD_SELINUX_DEFCONTEXT`, and
`FOD_SELINUX_ROOTCONTEXT` are forwarded as mount options when set. They do not
guarantee that the host accepts SELinux labeling for ordinary FUSE. On the
tested Rocky Linux 10.2 stack, ordinary `fusermount3` rejected these labeling
options for FOD.

`FOD_SELINUX=on` or `--selinux on` enables FOD's SELinux xattr path where the
kernel forwards requests. It cannot override a host-side SELinux/VFS rejection.

## Performance Snapshot

The accepted SELinux profiling run used commit `987550a`, SELinux `Enforcing`,
`FOD_SELINUX=on`, `FOD_ACL=on`, and `FOD_ALLOW_OTHER=1`.

Sequential fio, 32 MiB file, 4 KiB block size:

| Metric | Result |
| --- | --- |
| sequential write | 14.9 MiB/s, 3819 IOPS |
| sequential read | 16.5 MiB/s, 4216 IOPS |
| FOD callbacks | 8192 reads, 8192 writes, 0 copy_file_range |
| FOD profile | `fuse_read_total_us=1523143`, `fuse_write_total_us=1457988`, `repo_persist_blocks_us=354718`, `flush_execute_persist_plan_us=358838` |

Throughput smoke, 64 MiB `dd` zero write, 1 MiB blocks, fsync:

| Metric | Result |
| --- | --- |
| write throughput | 119.52 MiB/s |
| elapsed | 0.535 s |
| FOD callbacks | 0 reads, 64 writes, 0 copy_file_range |
| FOD profile | `fuse_read_total_us=42382`, `fuse_write_total_us=441636`, `repo_persist_blocks_us=314590`, `flush_execute_persist_plan_us=323069` |

No AVC or USER_AVC records were observed after the accepted profiling runs.

## Future Work

Do not change FOD's userspace `security.selinux` handler in response to
Rocky 10.2 ordinary FUSE `ENOTSUP` results. The request does not reach FOD.
Future per-inode SELinux support would need a verified host path that makes the
FUSE superblock use SELinux xattrs, such as a kernel/libfuse/policy model that
accepts this for a specific filesystem without changing the semantics of all
ordinary FUSE mounts globally.
