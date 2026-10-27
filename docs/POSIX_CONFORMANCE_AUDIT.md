# POSIX conformance audit

Status: **scoping started; no general POSIX conformance claim**.

## Scope and reference edition

The audit evaluates the behavior visible to applications using the Linux FUSE
mount, for the filesystem operations FOD implements or exposes through its
declared mount contract. The normative baseline is **POSIX.1-2024 (IEEE Std
1003.1-2024), The Open Group Base Specifications Issue 8**. When a requirement
is materially changed from POSIX.1-2017, record that difference rather than
silently mixing editions. POSIX.1-2024 is the currently published edition; the
technical corrigendum project is tracked separately and is not treated as
published normative text.

Primary references:

- [POSIX.1-2024 / Issue 8](https://pubs.opengroup.org/onlinepubs/9799919799/)
- [IEEE active standard record](https://standards.ieee.org/ieee/9945/12489/)
- [POSIX.1-2024 `open()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html)
- [POSIX.1-2024 `read()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/read.html)
- [POSIX.1-2024 `write()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html)
- [POSIX.1-2024 `lseek()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/lseek.html)
- [POSIX.1-2024 `stat()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/stat.html)
- [POSIX.1-2024 `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)

This is a filesystem compatibility audit, not a claim that FOD implements a
complete POSIX operating system. Shell and utility requirements, process and
terminal interfaces, device-driver behavior, and APIs absent from FOD's declared
filesystem contract are out of scope unless a concrete FOD mount behavior
depends on them. FUSE protocol behavior and Linux VFS behavior are recorded as
the delivery layer, not substituted for POSIX requirements.

POSIX ACL xattrs, Linux generic xattrs, Linux ioctl commands, SELinux, FUSE
capabilities, and FOD-specific extensions are tracked as separate Linux or FOD
compatibility contracts. They must not be presented as requirements from the
POSIX base specification. Multi-host ordering and PostgreSQL authority are
additional FOD correctness requirements and remain governed by
[architecture invariants](FOD_ARCHITECTURE_INVARIANTS.md).

## Status meanings

- **Pending** — requirement scope or evidence has not been reviewed against the
  normative text.
- **Pass** — every in-scope requirement in the row has implementation evidence
  and a passing targeted regression.
- **Fail** — a reproducible deviation from an in-scope requirement exists.
- **Not implemented** — the interface or required behavior is absent from FOD's
  declared support boundary; document whether that is an intentional exclusion.
- **Not applicable** — the normative requirement does not apply to the declared
  FOD interface; include the reason.

An existing test name is a starting point for evidence, not proof of compliance.
Every row must identify the normative section and the specific assertion before
its status can move from Pending.

## Initial requirement matrix

| Area | Normative interfaces / clauses to review | Existing FOD evidence to inspect | Status |
| --- | --- | --- | --- |
| Path resolution and name syntax | XSH `open`, `stat`, `mkdir`, `link`, `symlink`, `rename`, `unlink`; XBD pathname resolution | `rust_fuse/src/fs.rs`; `tests/integration/test_mount_suite.py`, `test_symlink.sh`, `test_rename_directory_transactional.py` | Pending |
| Directory operations and iteration | XSH `opendir`, `readdir`, `closedir`, `mkdir`, `rmdir`; XBD directory stream rules | FUSE callbacks in `rust_fuse/src/fs.rs`; `test_mount_suite.py`, `test_directories.sh`, `test_tree_scale.py` | Pending |
| Open and descriptor semantics | XSH `open`, `close`, `fcntl`, `dup`; access modes, append and create flags | `open`, `create`, `release`, `flush` callbacks; `test_mount_suite.py`, `test_create_write_ownership.py` | Pending |
| Read, write and offsets | XSH `read`, `write`, `pread`, `pwrite`, `lseek`; short I/O, EOF and offset rules | `test_lseek.py`, `test_lseek_sparse_contract.py`, `test_mount_suite.py` | Pending |
| Truncation and file length | XSH `truncate`, `ftruncate`, `open(O_TRUNC)` | `test_truncate_shrink_block_boundary.py`, `test_mount_suite.py`, Rust `mount_smoke` | Pending |
| Metadata and timestamps | XSH `stat`, `fstat`, `chmod`, `fchmod`, `chown`, `utimensat`; XBD file status fields | `test_mount_suite.py`, `test_permissions.py`, `test_timestamp_touch_once.py`, `test_utimens_noop` | Pending |
| Permissions and identity | XSH `access`, `umask`, `chmod`, `chown`; XBD file access permission rules | `test_permissions.py`, `test_access_groups.py`, `test_ownership_inheritance.py`, `test_root_owned_permissions.py` | Pending |
| Links and namespace mutation | XSH `link`, `unlink`, `rename`, `symlink`, `readlink`, `mkdir`, `rmdir`; atomicity and error precedence | `test_hardlink.sh`, `test_symlink.sh`, `test_rename_directory_transactional.py`, `test_chmod_rmdir.py`, primary-authority regressions | Pending |
| Special files and FIFOs | XSH FIFO creation/open/read/write behavior; classify Linux `mknod` and device-node provisioning separately | `test_mknod.py`, `test_mount_suite.py` | Pending |
| File locks | XSH `fcntl` record locking and applicable advisory-lock requirements | `getlk`/`setlk`; `test_mount_suite.py`, Rust lock smoke tests | Pending |
| Filesystem capacity and status | XSH `statvfs`; XBD `statvfs` fields and units | `statfs`; `test_df.sh`, two-mount quota tests, `docs/space-accounting.md` | Pending |
| Error behavior | XSH per-interface `ERRORS` and `RATIONALE`; errno precedence for supported operations | Targeted integration tests and `pjdfstest` subsets; map each error assertion to clause | Pending |
| Linux/FUSE extensions | Not POSIX base requirements; use Linux and FUSE references | `test_xattr.py`, `test_acl_mount_option.py`, `test_ioctl.py`, compatibility contracts | Separate extension audit |

## Audit procedure

1. For each row, list exact Issue 8 section and option-group applicability.
2. Compare its normative conditions, results, errors, and atomicity rules with
   the callback and storage transaction that implements them.
3. Run or add the narrowest deterministic mounted regression; use `pjdfstest`
   where a relevant case exists and record skipped cases with their reason.
4. Mark each requirement individually Pass, Fail, Not implemented, or Not
   applicable, then summarize the row. Keep unreviewed requirements Pending.
5. Fix reproducible in-scope failures in focused changes. Document intentional
   exclusions and the verified support boundary in
   [compatibility contracts](compatibility-contracts.md).

The audit is complete only when every in-scope requirement has evidence and
every excluded interface has a stated reason. Do not infer conformance from
successful common-command smoke tests or from FUSE capability negotiation.
