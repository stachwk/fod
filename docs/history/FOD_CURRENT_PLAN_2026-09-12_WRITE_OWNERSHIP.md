# FOD 3.4.23 cross-mount write ownership and fencing

Status: completed 2026-09-12.

This record archives the P1 implementation sequence that introduced
PostgreSQL-authoritative write ownership across independent FOD mounts,
processes and machines.

## Completed boundary

The completed implementation provides:

- PostgreSQL-authoritative destination and file write leases in schema 24;
- first-writer-wins acquisition with non-blocking try-lock behavior;
- immediate `EBUSY` rejection for a competing writer;
- lease heartbeat tied to active client sessions;
- PostgreSQL `clock_timestamp()` as the authoritative lease clock;
- monotonic destination/file fencing tokens;
- persistence fencing that rejects stale writers after ownership loss or
  takeover;
- ownership coverage for writable `open`, `O_TRUNC`, `create`, handle-based
  truncate and path-based `truncate`/`setattr`;
- `FUSE_ATOMIC_O_TRUNC` as a required writable-mount capability;
- cleanup/release behavior that does not permit a stale writer to resume
  persistence after a failed final flush;
- a bootstrap FUSE hang guard that detects a stuck `statfs`, aborts the
  connection when fusectl is available, terminates the frontend and detaches
  the aborted mount.

All FOD hosts and PostgreSQL nodes should keep system time synchronized with
`chrony` or an equivalent NTP implementation. Lease decisions themselves use
PostgreSQL server time rather than client timestamps.

## Schema

Migration `0024_write_ownership_leases.sql` adds:

- `fod.destination_write_leases`,
- `fod.file_write_leases`,
- unique resource indexes for destination and file ownership,
- session and expiry indexes required for cleanup and heartbeat paths.

The canonical freshly initialized schema is version 24.

## Validation evidence

The P1 regression sequence was validated against isolated PostgreSQL test
databases and two independent FOD mounts.

Validated integration coverage:

- `test_first_writer_wins.py`;
- `test_create_write_ownership.py`;
- `test_path_truncate_write_ownership.py`;
- `test_write_ownership_heartbeat.py`;
- `test_stale_writer_fencing.py`;
- `test_fuse_hang_guard.py`.

The Rust hot-path ownership test validated first-writer-wins, heartbeat,
explicit release, lease expiry/takeover and fencing-token advancement.

The schema/mkfs suite validated migration 24 and destructive upgrade/status
paths on a dedicated isolated database.

Observed cleanup after the completed tests contained zero destination write
leases and zero file write leases, and no P1 test mount remained attached.

## Failure model

A writer owns both the destination identity and, when a file already exists,
the file identity. Heartbeats extend leases without changing fencing tokens.
After release or expiry, the next successful acquisition receives newer fencing
tokens. Persistence validates the active file token and client session inside
the PostgreSQL transaction before changing durable payload state.

A stale writer can retain a local file descriptor, but it cannot persist after
its lease has been revoked, expired or superseded.

## Operational guardrails

- Keep time synchronized on FOD and PostgreSQL hosts.
- Do not run independent writable PostgreSQL primaries as one FOD filesystem.
- Do not weaken `FUSE_ATOMIC_O_TRUNC` negotiation on writable mounts.
- Do not convert the ownership try-lock path into a blocking wait.
- Run destructive schema tests only against dedicated databases whose names
  clearly identify them as test databases.
