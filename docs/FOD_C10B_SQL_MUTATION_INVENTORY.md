# C10-B1 — inventory of SQL mutations and PostgreSQL authorization boundaries

Status: **initial, source-verified inventory** (2026-10-13). This is a
read-only design artifact. No DDL, GRANT, REVOKE, role changes, or production
code changes have been applied.

## Safety objective

FOD has multiple FUSE mounts across independent hosts. PostgreSQL must remain
the **single logical authority** for the order of conflicting mutations.

C1–C9 tested that FOD-controlled transactions and advisory locks enforce
ownership and reject stale writers. The C10-A audit of
`foddbname_hotpath_test` confirmed that the current `foduser` database
principal is `SUPERUSER`, owns the schema and database, and has full DML
rights on all six inspected authoritative tables. A client using those
credentials can issue direct SQL outside FOD's fencing contract.

A database role that owns storage, can issue unrestricted DML, or has
`SUPERUSER` remains outside the protection of application-level fencing.
The target is to remove such privileges from the **runtime credentials**,
not to eliminate the privileges needed by a separate bootstrap authority.

## First-pass mutation inventory (verified from repository source)

| Area | Entry point / evidence | Mutation class | Fencing / authorization concern |
|---|---|---|---|
| PostgreSQL bootstrap | `docker-compose.yml`, `POSTGRES_USER` | Creates the initial DB owner/superuser in the local Compose installation | Bootstrap identity is reused as FOD runtime credentials |
| Schema initialization/upgrade | `rust_mkfs/src/main.rs`: `apply_base_schema`, `apply_migration`, `write_schema_version` | Schema DDL and metadata DML | Must be available only to a schema migration principal |
| Grants on init/upgrade | `rust_mkfs/src/main.rs`: `ensure_fod_privileges` | **Explicit** `GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA fod` and sequence grants to the selected DB user | Migration to a restricted role will be undone unless this grant policy is redesigned |
| Runtime schema establishment | `rust_hotpath/src/pg.rs`: `ensure_client_session_schema` | `CREATE TABLE`, `ALTER TABLE`, `CREATE INDEX` during runtime schema readiness | Restricted runtime cannot safely retain blanket DDL privileges; bootstrap/upgrade must precreate required objects |
| Namespace create | `rust_hotpath/src/pg.rs`: `create_file` | `INSERT data_objects`, `INSERT files`, parent directory metadata update | Namespace serialization is not a write-content fence; must preserve separate multi-host namespace invariants |
| Fenced payload persist | `rust_hotpath/src/pg.rs`: `persist_file_blocks_with_crc_flag_reservation_and_fence` | Data-object/block/metadata mutations | With fence: `validate_write_persistence_fence_on_conn`; without fence: legacy no-active-lease gate |
| Import/legacy payload persist | `rust_hotpath/src/pg.rs`: `persist_file_blocks_from_path` | Streaming payload write | `reject_unfenced_write_during_active_lease_on_conn` protects live leases, but not a general prohibition on tokenless writes |
| Data-object adoption | `rust_hotpath/src/pg.rs`: `adopt_source_data_object_with_fence`, `adopt_source_data_object` | Copy-on-write object reference and size changes | Fenced API validates the destination token; legacy API permits adoption only without an active file lease |
| File size | `rust_hotpath/src/pg.rs`: `set_file_size` | `UPDATE files.size` | Legacy no-active-lease guard only |
| Lease issuance | `rust_hotpath/src/pg.rs`: `acquire_write_ownership` | Destination/file lease INSERT, UPDATE, expiry pruning; token sequence | PostgreSQL advisory locks and session validation must remain in the same authoritative transaction; direct lease DML is currently possible |
| Open/unlink lifetime | `rust_hotpath/src/pg.rs`: `register_file_open_lease`, `release_file_open_lease`, `defer_primary_file_unlink` | Open lease and deferred namespace mutation | Separate cross-host coordination contract; must not break while restricting storage table DML |
| Index catalog | `rust_hotpath/src/pg.rs`: `upsert_index_*_staged` | Staged index catalog writes | Needs scoped indexer permissions separate from unrestricted filesystem payload writes |
| Indexer import | `rust_indexer/src/materialize.rs`: `materialize_source`, `materialize_reference_file` | `create_file`, tokenless `persist_file_blocks_from_path`, `adopt_source_data_object` | Must migrate to an authorized import operation rather than granting unrestricted payload DML |
| Indexer cleanup | `rust_indexer/src/cleanup.rs`: `cleanup_failed_materialization` | Direct `DELETE files`, `DELETE directories`, `DELETE data_objects`, `UPDATE data_objects.reference_count`, `UPDATE index_import_plans`; explicit transaction | High-priority bypass: a cleanup action can alter data/namespace without the fenced payload API; investigate synchronization with active mounts and surviving references |
| Generic SQL execution | `rust_hotpath/src/pg.rs`: `DbRepo::exec` | Arbitrary SQL issued by Rust callers | A restricted runtime role must be enforced by PostgreSQL; filtering strings in `DbRepo::exec` is not an authorization boundary |

**Scope limitation:** this is a *risk-first map*, not yet a complete list of
SQL statements. Further inventory must include every `DbRepo` method and
every caller in `rust_fuse`, `rust_indexer`, `rust_libfod`, `rust_monitor`,
`rust_mkfs`, scripts/FFI, plus dynamically assembled SQL. `migrations/*.sql`
are separate privileged DDL and must not be granted to the FUSE runtime.
This document does not assert that every path listed is currently exploitable
under a valid FUSE API; direct SQL rights are a separate, confirmed boundary.

## Risk priority and migration order

**P0 — control-plane authorization.**
Separate authentication of bootstrap/schema owner from FUSE/indexer runtime.
Rework `ensure_fod_privileges` so it never broadly re-grants protected DML to
the restricted identity on a later `init`/`upgrade`. Ensure runtime startup
can validate schema without creating or altering it. Failing closed is safer
than runtime DDL using a privileged credential.

**P0 — write ownership and protected data.**
Runtime access to `file_write_leases`,
`destination_write_leases`, `files`, `data_objects` and `data_blocks`
must be mediated by PostgreSQL-authorized operations with explicit privilege
grants. A routine exposed to the runtime should execute one protected
transaction: identify its trusted caller, take the appropriate shared
advisory serialization lock, validate current lease/session/fencing token,
perform the mutation, and commit. The runtime must not be able to change lease
rows or call unsafe maintenance entry points directly.

**P1 — indexer and cleanup.**
Introduce separate authorized import/cleanup capabilities with explicit
preconditions, source/plan ownership, and cross-host open/lease checks.
Do not merely copy the current `DELETE ...` SQL into a broadly executable
`SECURITY DEFINER` routine. Mixed FUSE/indexer operations need a documented
global lock order and transaction boundary.

**P1 — schema lifecycle and namespace operations.**
Precreate runtime tables and indexes during migration, and migrate
namespace/open-unlink mutations to dedicated operations with validated
permissions. Ensure old and new runtime versions cannot run concurrently in a
way that bypasses fencing.

## Proposed role model (design only — no SQL executed)

| Role | Login | Responsibility | Forbidden authority |
|---|---|---|---|
| `fod_schema_owner` | NOLOGIN | Owns `fod` schema, tables, sequences, protected functions | Not used as a mount credential |
| `fod_schema_admin` | LOGIN or controlled deployment identity | Authorized schema migration and grants, only during maintenance | Not distributed to hosts running FUSE |
| `fod_runtime` | LOGIN, NOSUPERUSER, NOCREATEDB, NOCREATEROLE | Execute a narrow list of secured PostgreSQL operations plus necessary read-only queries | Direct DML on protected data/lease tables, arbitrary DDL, schema ownership |
| `fod_indexer` | LOGIN, restricted | Catalog writes and explicitly authorized import/cleanup | Unrestricted direct DML on filesystem payload/lease state |
| `fod_monitor` (if needed) | LOGIN, restricted | Read telemetry and narrowly scoped append-only snapshots | File content and lease modifications |

These are proposed role responsibilities, **not** a deployed schema.

For any `SECURITY DEFINER` functions, review `search_path` hardening,
ownership, EXECUTE privileges, input validation and caller identity; revoke
the default `PUBLIC` execute privileges where appropriate. Client-supplied
owner/session IDs and fencing tokens alone should not substitute for an
authenticated PostgreSQL capability. Role-based DML removal is only effective
if protected functions cannot be used as an equivalent unrestricted bypass.

## Next implementation gates

1. **C10-B2 exhaustive inventory:** collect every SQL mutation and all
   `DbRepo::exec` callsites into a reviewable table, including dynamic SQL.
   Map component -> table -> operation -> fence/lease/transaction guard.
2. **C10-B3 isolated role bootstrap:** separate test database with distinct
   owner/admin/runtime/indexer credentials and no silent grants at upgrade.
   Produce a read-only report for each role before performing negative tests.
3. **C10-B4 restricted API prototype:** select one narrow write operation and
   prove legitimate fenced success, stale/unfenced denial and direct-SQL denial.
4. **C10-B5 integration:** test C1–C9, C7/C8 two-mount FUSE, indexer,
   cleanup, FFI, restart, failover and mixed-version upgrade/rollback gates.

Acceptance conditions include **no application direct DML** for protected
tables, denied unauthorized SQL even when the application forgets to call
Rust checks, a complete migration/rollback plan, and preserved multi-host
FOD consistency.

Do **not** modify the current `foduser`, run `REVOKE`, or change the
production database as part of this inventory.
