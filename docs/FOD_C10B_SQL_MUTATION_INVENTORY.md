# C10-B1 — inventory of SQL mutations and PostgreSQL authorization boundaries

Status: **C10-B2 static candidate collection locally validated; manual authorization/transaction review pending** (2026-10-13). The first local run passed 4/4 offline tests and returned scan_rc=0. Subsequent fixes for Markdown SyntaxWarning and new grouped-summary CLI have not yet been locally revalidated. This remains a read-only audit. No DDL, GRANT, REVOKE, role changes or production write-path changes have been applied.

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

### First local source scan, 2026-10-13

Baseline reported by the local checkout at `63ee5df`:

- 149 source files scanned, 1,045 candidate findings;
- 700 textual SQL mutation candidates and 345 SQL execution sinks;
- category counts: filesystem_state 135, index_catalog 26,
  lease_or_session 109, schema_admin 277, manual_review 498;
- offline tests 4/4 PASS, `test=0 scan=0`;
- Python emitted `SyntaxWarning` for escaped Markdown backticks.

The warnings were corrected by rendering literal backticks, and a
Markdown-regression assertion was added. C10-B2 also gained `by_component`,
`top_paths` (top 20), and a JSON `--summary` option to focus manual review
without copying all 1,045 findings. The new summary deliberately retains
the total count of scanned files, even when a source has zero matches.

Run after updating to the new commits:

```bash
cd ~/git/fod
python3 -W error::SyntaxWarning -m py_compile \
  scripts/security/inventory_sql_mutations.py \
  tests/test_c10b_sql_mutation_inventory.py
python3 -W error::SyntaxWarning tests/test_c10b_sql_mutation_inventory.py -v
python3 scripts/security/inventory_sql_mutations.py --summary --fail-on-empty
```

Do not interpret counts or guard hint presence as a validated privilege
boundary. Confirm the individual SQL operation and its caller, authenticated
role, transaction/advisory lock, fence, and a negative test before restricting
runtime permissions.

## C10-B2 — repeatable candidate collection (requires manual classification)

New components:

- `scripts/security/inventory_sql_mutations.py` scans Rust source,
  `migrations/*.sql`, and Python, shell and SQL files under `scripts/`
  and `docker/`.
- `tests/test_c10b_sql_mutation_inventory.py` exercises source locations,
  comment skipping, migration DDL, execution sinks, deployment scripts,
  Markdown output and compact JSON summaries.
- The collector emits deterministic JSON or Markdown including file,
  line number, approximate enclosing function, operation/object hint and
  optional guard markers. It does not connect to PostgreSQL.

Run from the checkout:

```bash
cd ~/git/fod
python3 tests/test_c10b_sql_mutation_inventory.py -v
python3 scripts/security/inventory_sql_mutations.py \
  --format json --fail-on-empty > /tmp/fod-c10b2-mutations.json
python3 scripts/security/inventory_sql_mutations.py \
  --format markdown > /tmp/fod-c10b2-mutations.md
python3 - <<'PY'
import json
with open('/tmp/fod-c10b2-mutations.json', encoding='utf-8') as stream:
    report = json.load(stream)
print('C10_B2 files=', report['files_scanned'])
print('C10_B2 findings=', report['findings_count'])
print('C10_B2 categories=', report['by_category'])
print('C10_B2 kinds=', report['by_kind'])
PY
```

**Coverage limitations:** This is a conservative text scanner, not
a Rust AST or PostgreSQL parser. Dynamic SQL, multiline constructs,
indirect SQL helpers and generated statements can be missed.
`relation_hint=UNKNOWN` does not imply a safe statement.
Source-function and `guard_hints_not_proof` values are navigation aids;
they do not establish a valid transaction, authenticated session or
successful fencing. There may also be false positive textual matches.

Every high-risk row must be manually classified against callers, tables,
transaction/advisory locks, lease/fence checks, PostgreSQL role grants
and fail-closed negative tests. C10-B3 must not start simply because
the scanner produced a report. Priority: indexer cleanup, tokenless
legacy paths, runtime DDL and direct lease-table updates.

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

1. **C10-B2 manual completion:** run the static inventory, review all
   candidates and dynamic SQL execution sinks, and map each reachable
   mutation to component -> table -> operation -> authorization ->
   transaction -> fence/lease/lock. The static collector alone does not
   satisfy the complete-inventory gate.
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
