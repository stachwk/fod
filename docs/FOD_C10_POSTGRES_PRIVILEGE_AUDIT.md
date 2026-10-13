# C10 — PostgreSQL role boundary and direct-write bypass audit

Status: **C10-A read-only privilege audit completed for the explicitly selected test database; security finding confirmed** (2026-10-13). After correcting database routing, `C10_TARGET database=foddbname_hotpath_test` matched `C10_ROLE database=foddbname_hotpath_test`, with `role=foduser`, `superuser=YES`, `createdb=YES`, `createrole=YES`, `bypassrls=YES`, `owns_database=YES` and full `INSERT,UPDATE,DELETE,TRUNCATE` on all 6/6 inspected tables. `C10_DIRECT_DML_BYPASS=YES`, `C10_AUDIT=UNRESTRICTED`, `audit=0`, `diff=0`. The caller also reported `policy=2` while an `OK: public Make target namespace is normalized and alias-free` line appeared; the policy exit-code discrepancy remains unverified and must be diagnosed separately. Earlier auditing `foddbname` instead of the requested test database was read-only and has been fixed.

## Why this audit is necessary

C1–C9 validate FOD ownership, leases and transaction-scoped fencing in the
Rust/FUSE/PostgreSQL execution paths. They **do not** make the PostgreSQL
storage tables write-protected from other SQL clients using the same role.

A PostgreSQL user who can run `UPDATE fod.files`, `INSERT/UPDATE fod.data_blocks`
or `DELETE FROM fod.file_write_leases` directly can avoid Rust's
`WritePersistenceFence` check altogether. Even if no active lease is present,
C6-B/C6-C retain some legacy mutation API for indexer/FFI compatibility.
These are different risks:

- **Accidental bypass:** a maintenance script or older FOD client writes
  through an unfenced code path.
- **Credential/role bypass:** a user or process with direct table DML bypasses
  the application and all its transaction checks.
- **Privilege escalation:** a database superuser or schema/table owner can
  bypass `REVOKE`-based restrictions and alter stored routines.

The current `docker-compose.yml` uses `POSTGRES_USER` (default: `foduser`)
for PostgreSQL initialization. FOD's fresh-install SQL creates `fod` schema
and storage tables without an explicit separate runtime role or grants
boundary. The actual effective permissions **must be inspected on the
running deployment**; they cannot be inferred solely from the compose file.

The application-level `fod.schema_admin` password is not a substitute for
database authorization: it protects schema administration within FOD tools,
not arbitrary SQL run using the database account.

## C10-A — read-only privilege inventory

Script:

`scripts/security/pg_runtime_role_audit.py`

Read-only target:

`make audit-postgres-runtime-role`

For an isolated test database:

```bash
cd ~/git/fod
POSTGRES_DB=foddbname_hotpath_test make audit-postgres-runtime-role
```

Or for the strict policy check (exit 2 if unrestricted or incomplete):

```bash
.venv/bin/python scripts/security/pg_runtime_role_audit.py \
  --database foddbname_hotpath_test --strict
```

The public Make target also accepts `FOD_AUDIT_DBNAME=foddbname_hotpath_test`
as an explicit override. It **requires** an explicitly chosen database;
omitting the database returns `C10_AUDIT=ERROR stage=configuration` instead
of silently checking `foddbname`. The script prints `C10_TARGET database=...`
and validates `current_database()` before interpreting any privilege results.

The script connects as the configured **runtime role**, enters a
`READ ONLY` transaction, reads `pg_roles`, schema ownership and effective
table DML capabilities through `has_table_privilege`, and then rolls back.
It does **not** issue DDL/DML, create users, modify grants or log passwords.

Reported authoritative tables:

- `fod.files`
- `fod.data_objects`
- `fod.data_blocks`
- `fod.file_write_leases`
- `fod.destination_write_leases`
- `fod.client_sessions`

`C10_DIRECT_DML_BYPASS=YES` means the role is a superuser or can execute
direct DML on at least one of those tables. **This is a design-risk finding,
not proof of a successful unauthorized write.** It is expected with the
current direct-SQL runtime model and should not be hidden by changing
PostgreSQL privileges prematurely.

`C10_AUDIT=INCOMPLETE` means not all expected tables were found; it must
not be considered safe. The default audit exits 0 for an observed
`UNRESTRICTED` or `RESTRICTED` result and reserves 3 for audit errors.
`--strict` exits 2 for `UNRESTRICTED` or `INCOMPLETE`.

## C10-B — design changes before revoking privileges

Do **not** execute blanket `REVOKE INSERT, UPDATE, DELETE` on FOD tables today.
The FUSE runtime, indexer and existing FFI rely on direct DML, and such a
revoke would break normal operation or failover.

Required follow-up design:

1. Separate schema/bootstrap/migration credentials from runtime credentials.
   The runtime principal should not be superuser, role administrator or owner
   of `fod` storage tables.
2. Inventory every runtime/indexer/FUSE/FFI mutation and label it as:
   fenced user write, namespace operation, metadata operation, or explicit
   maintenance operation.
3. Define privileged PostgreSQL transaction entry points for protected
   mutations (such as carefully designed `SECURITY DEFINER` routines)
   with fixed `search_path`, minimal grants, explicit lock ordering and
   same-transaction fencing validation. Avoid accepting client-controlled
   session settings as independent authorization evidence.
4. Migrate callers to those entry points before revoking direct DML from
   runtime accounts. Protect lease rows as carefully as file data.
5. Re-run C1–C9, C7/C8 real FUSE mount tests, indexer/FFI tests, and
   cross-host crash/failover checks, including an explicit negative SQL
   authorization test using the final restricted account.
6. Document migration/rollback and secret/role rotation without running
   mixed old/new runtimes that can silently bypass or deadlock.

PostgreSQL remains the **single logical authority** for conflicts across hosts.
This audit makes no changes to that authority or to the currently passing
write-persistence implementation.

## C10-A confirmed result and C10-B gate

The correctly targeted read-only audit proves **direct SQL writes are authorized**,
not that malicious writes have taken place. In this configuration the client
credential is a PostgreSQL superuser and can bypass any application-level
`WritePersistenceFence` check. PostgreSQL fencing still enforces the
ordering of requests that voluntarily go through FOD's repository API;
it is not an SQL privilege boundary.

C10-B must **not** start with `REVOKE`, `ALTER ROLE ... NOSUPERUSER`,
changing ownership of production tables, or modifying the current
`foduser`. Such operations can break ongoing mount/indexer activity and
cannot enforce fencing until the direct-write API has been migrated.

A safe next stage is an inventory of SQL mutations by component
(FUSE, hotpath, indexer, mkfs, libfod/FFI, monitoring and operations),
followed by a new `NOLOGIN` storage owner and an independent
`LOGIN NOSUPERUSER` runtime principal in a *separate test environment*.
The restricted runtime must not own schema/tables and must not have
direct DML on protected storage or lease records. Replace required
mutations with narrowly granted PostgreSQL entry points whose protected
transactions validate the lease/fence under the same advisory lock.
Only after successful regression and rollback drills should the
runtime use restricted credentials in production.

A role with unrestricted access to the database is not the same
as a filesystem writer holding a current FOD ownership lease. Never
equate these two authorization mechanisms.

### Make policy test failure: fixed assertion, local rerun pending

The direct policy test returned `DIRECT_POLICY_RC=1` with
`C10 audit lost explicit POSTGRES_DB environment selection`.
The public Make wrapper **did** preserve the chosen database and the
correctly targeted C10 audit succeeded. The test was incorrect: GNU Make
`-n` printed the *forwarded make* command without recursively printing
the final Python `--database` invocation. It therefore searched for
`--database ...` in the wrong layer.

The policy test now verifies two stages separately: the public
`FOD_AUDIT_DBNAME="..."` forwarding for both environment and command-line
selection, and the internal `--database "..."` argument. These checks
passed against a minimal reproducer; the full repository test remains to be
rerun locally. No PostgreSQL permissions or FOD runtime behavior changed.
