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

The release version changes for every repository commit. It is not a database
compatibility marker. The schema version records which ordered migrations have
been applied. A storage-format boundary is narrower: it exists only when
persisted payload cannot be interpreted safely by both sides of an upgrade
without an explicit conversion or compatibility decision.

Schema version 18 is the current database shape. Payload ownership is rooted in
`data_objects`; `data_blocks`, `data_extents`, and `copy_block_crc` refer to the
object; extents remain opt-in; and payload-capacity reservations are control
records rather than file data. There is no separate storage-format column or
marker today.

## Decision

Keep the schema version as the only persisted compatibility marker until a
measured change requires a distinct storage-format marker.

Every schema change must:

- receive the next numbered migration and update the fresh-install base schema;
- be applied transactionally or fail before publishing the new schema version;
- define whether existing rows need conversion, validation, or no data change;
- update strict latest-shape recovery checks;
- include fresh-init, supported-upgrade, status, and remount/readback coverage;
- reject an unsupported newer schema rather than attempting a legacy fallback.

Introduce a separate storage-format marker only when a schema version cannot
fully describe how payload bytes or ownership relationships must be
interpreted. Examples include incompatible block encoding, compression,
encryption, immutable chunk manifests, or a payload layout that requires
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

## Consequences

Release bumps do not imply database migrations. Schema migrations do not
automatically imply a new physical storage format. The runtime remains simple:
it accepts the schema and storage contract it was built for and fails clearly
on unsupported states.

Future storage redesigns cannot add an ad hoc marker or schema column before
updating this ADR or replacing it with a more specific accepted decision.

The current schema version 18 requires no new storage-format marker.
