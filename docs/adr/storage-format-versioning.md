# ADR: Storage Format and Schema Versioning

## Status

Accepted on 2026-07-18.

## Context

FOD has three version domains with different purposes:

- the FOD release version from `fod_version.txt`;
- the monotonically increasing PostgreSQL schema version maintained by
  `mkfs.fod` and numbered migration files;
- the semantic storage format used to interpret file payload and ownership
  rows.

The release version changes for every repository commit. It is not implicitly
a database compatibility marker. The schema version records which ordered
migrations have been applied. A `minimum_client_version` records which FOD
releases may mount after the latest `mkfs.fod init` or `mkfs.fod upgrade`; it is
raised monotonically by that command and replicated with the schema-version
row. The database is the authority for this decision: clients compare their
own FOD release with the value read from the database endpoint they selected.
They do not infer compatibility from the FOD release running on a PostgreSQL
primary, a standby, or another FOD mount. A storage-format boundary is
narrower: it exists only when persisted
payload cannot be interpreted safely by both sides of an upgrade without an
explicit conversion or compatibility decision.

At the time this decision was accepted, schema version 21 was the database
shape and migration gate. The current version is maintained by the migration
manifest in `rust_mkfs/src/main.rs`. At schema version 21, payload
ownership is rooted in `data_objects`; `data_blocks` and `copy_block_crc` refer
to the object; migration 20 converts remaining `data_extents` rows to canonical
`data_blocks`; migration 21 drops the retired `data_extents` table;
payload-capacity reservations are control records rather than file data; and immutable index catalogue
snapshots are part of the schema contract. No separate storage-format marker
was needed for these migrations.

## Decision

Keep the schema version as the authority for database shape and storage
interpretation. Store a separate minimum FOD client release in the same
`schema_version` row to prevent older client releases from mounting after an
explicit database initialization or upgrade. This release floor advances only
through `mkfs.fod init` or `mkfs.fod upgrade`; ordinary release bumps do not
implicitly change it.

Every schema change must:

- receive the next numbered migration and update the fresh-install base schema;
- be applied transactionally or fail before publishing the new schema version;
- define whether existing rows need conversion, validation, or no data change;
- update strict latest-shape recovery checks;
- include fresh-init, supported-upgrade, status, and remount/readback coverage;
- reject an unsupported newer schema rather than attempting a legacy fallback;
- write the current FOD release as `minimum_client_version` when publishing the
  schema-version row, and reject attempts by an older `mkfs` to lower that
  floor;
- establish the floor from the client that runs `mkfs.fod init`, then advance
  it to the client that successfully runs `mkfs.fod upgrade`; all primary and
  replica mounts read the same replicated database value;
- set `upgrade_in_progress` and raise the client floor before applying
  migrations; retain the gate after an interrupted upgrade and clear it in the
  same transaction that publishes the final schema number and client floor;
- serialize upgrade processes with a PostgreSQL advisory lock, serialize new
  client-session registration against that lock, and refuse upgrades while
  unexpired client sessions exist;
- declare the minimum schema version required by each runtime build, so a
  replica that has not replayed the required schema cannot admit that client;
- break startup of pre-gate clients by renaming the schema-number column so
  those clients cannot silently ignore the release floor.

Introduce a separate storage-format marker only when a schema version cannot
fully describe how payload bytes or ownership relationships must be
interpreted. The client-release floor does not replace schema migration checks
or claim binary payload incompatibility. Examples include incompatible block
encoding, compression, encryption, immutable chunk manifests, or a payload layout that requires
multiple readers during migration.

If a separate marker becomes necessary, its design must define:

- format identifier and version source;
- exact tables and payloads governed by the marker;
- atomic publication order between converted data and the marker;
- crash recovery and replay behavior;
- upgrade and rollback policy;
- mixed-version mount behavior;
- backup, restore, and replication requirements;
- removal criteria for any temporary migration reader.

FOD does not maintain permanent backward-compatibility branches in the runtime.
An older supported database must be upgraded by `mkfs.fod upgrade` before
mounting with code that requires a newer schema or format. Downgrade is not
supported unless a future ADR defines and tests an explicit reverse migration.
Before an upgrade, operators must unmount FOD on every host. The database gate
rejects new mounts and checks tracked active sessions; it cannot terminate
already-running clients that predate the gate or are connected to a standby
that has not replayed the gate yet.

## Consequences

Release bumps do not imply database migrations. Schema migrations do not
automatically imply a new physical storage format. The runtime remains simple:
it accepts the schema and storage contract it was built for and fails clearly
on unsupported states.

Future storage redesigns cannot add an ad hoc marker or schema column before
updating this ADR or replacing it with a more specific accepted decision.

At schema version 21 no new storage-format marker was required because the
ordered migrations fully defined the legacy extent conversion and table
removal. The current release floor is a client admission check, not a second
description of payload format.
