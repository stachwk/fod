# P3 external unmount/session teardown

Status: completed 2026-09-13.

P3 rechecked the historical external-unmount warning on the current FOD FUSE
stack before allowing any teardown implementation change.

## Current stack

The live reproduction used:

```text
FOD 3.4.23
fuser 0.18.0
fusermount3/libfuse3 3.17.4
PostgreSQL 16.15
FOD schema 24
kernel FUSE protocol 7.44
negotiated FUSE protocol 7.40
```

The client host was `lt7300`.

## Historical reference

The retained 2026-07-12 `fuser 0.17` migration baseline documented a benign
post-unmount warning:

```text
Failed to umount filesystem: Invalid argument
```

At that time an external `fusermount3 -u` had already removed the mount and
the later session teardown attempted a second unmount.

P3 required current reproduction before changing runtime teardown behavior.

## 2026-09-13 reproduction

A writable FOD mount was started through the production bootstrap path. The
probe wrote and `fsync()`ed a file, then unmounted from outside the FOD process
with:

```text
fusermount3 -u <mountpoint>
```

Observed result:

```text
external_unmount_rc=0
external_unmount_elapsed_ms=7.531
mount_gone_after_external_unmount=1
bootstrap_exited_after_external_unmount=1
bootstrap_exit_rc=0
teardown_einval_warning_reproduced=0
p3_probe_rc=0
```

No mount remained after the probe and `git diff --check` stayed clean.

The current log reported `fuser=0.18.0`, kernel protocol `7.44` and negotiated
protocol `7.40`.

## Client-session row

The PostgreSQL `fod.client_sessions` row was still present immediately after
the process exited. This is consistent with the current lease-based session
contract rather than evidence of the historical FUSE teardown warning:

- client sessions carry `lease_expires_at`;
- the current writable session lease TTL is 30 seconds;
- session maintenance removes rows only after `lease_expires_at <= NOW()`;
- maintenance is performed by active FOD session maintenance loops.

External unmount therefore does not imply an immediate SQL row delete. The row
becomes stale by lease expiry and is eligible for normal maintenance cleanup.

P3 does not change this ownership/lease contract.

## Decision

Do not change the FUSE teardown implementation.

The historical `EINVAL` could not be reproduced on `fuser 0.18.0` with the
current libfuse3 stack, while the required correctness properties were all
satisfied:

- external unmount returned success;
- the mount disappeared;
- the FOD process exited normally;
- no teardown `EINVAL` was emitted;
- no leftover mount remained.

A focused regression test now records this boundary as
`test-external-unmount-teardown`.

No runtime code/default changed, so the FOD version remains `3.4.23`.

The next active implementation priority is P4: compatibility diagnostics
aggregation.
