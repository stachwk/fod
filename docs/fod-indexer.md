# FOD Indexer Architecture Note

## Goal

`fod-indexer` is a separate FOD service for scanning external file sources, building a deduplication-aware index, and preparing safe import plans into the FOD database.

The indexer should not be part of the FUSE hot path. It is an offline / background service that can scan slow or removable sources without affecting mounted FOD runtime behavior.

## Problem

A user may have the same files spread across many sources:

- local disks,
- QNAP / NAS shares,
- Google Drive exports or mounts,
- USB disks,
- phones mounted over USB/MTP,
- backup directories,
- old project snapshots.

A naive importer would copy every discovered file into FOD, including duplicates. That wastes storage, hides useful duplicate information, and makes later cleanup harder.

`fod-indexer` should first learn what exists, detect duplicates, and then import only unique content while preserving all source paths as metadata or FOD-level links.

## Preferred Model

Use a content-addressed pipeline:

```text
scan sources -> index metadata -> hash candidates -> build duplicate sets -> create import plan -> import unique payloads -> materialize logical paths/links
```

Do not copy files during the first scan. The first scan should only collect metadata and enough fingerprints to decide what needs full hashing.

## Deduplication Semantics

The indexer should distinguish between:

1. physical source files,
2. unique content payloads,
3. logical FOD paths that should point to the imported content.

If the same file content appears in many source locations, FOD should store the payload once and create multiple logical references to the same imported content object.

This is better than creating external symlinks to the old locations, because external links break when the original disk, NAS, phone, or cloud mount is unavailable.

External links may exist as an optional cleanup/materialization mode, but they must not be the default archival model.

## Recommended Pipeline

### 1. Source registration

Register each scanned source with a stable source id and metadata:

- source name,
- source type,
- root path or connector id,
- host name,
- device id when available,
- scan timestamp,
- availability state.

Examples:

```text
source_id=local_home      type=local_fs root=/home/wojtek
source_id=qnap_backup     type=nas      root=/mnt/qnap/backup
source_id=phone_usb       type=mtp      root=/run/user/.../phone
```

### 2. Metadata scan

For every file, store cheap metadata first:

- source id,
- source path,
- file size,
- mtime / ctime when available,
- device / inode when available,
- permissions and ownership when relevant,
- file type,
- scan generation id,
- error state if the file could not be read.

Metadata alone is not enough to prove duplicates, but it is useful to group candidates before expensive hashing.

### 3. Candidate grouping

Group potential duplicates by cheap keys:

```text
(size)
(size, mtime)
(size, partial_hash)
```

Only groups with more than one candidate need stronger hashing.

### 4. Hashing strategy

Use staged hashing:

1. size-only grouping,
2. optional partial hash for large files,
3. full content hash for duplicate candidates,
4. optional second strong hash if collision risk must be minimized.

For correctness, final deduplication must be based on full content hashing, not only file name, size, or timestamp.

### 5. Import plan

Build an import plan before writing to FOD:

- one canonical payload per unique content hash,
- all duplicate source paths attached as references,
- selected logical destination paths in FOD,
- conflict policy for same destination path,
- dry-run summary,
- expected byte count,
- skipped duplicate byte count,
- files with read errors,
- files changed during scan.

The import plan must be reviewable before execution.

### 6. Safe import

Before importing each file, revalidate that the source file did not change since it was indexed:

- size still matches,
- mtime still matches when reliable,
- final hash still matches,
- source still readable.

If the file changed, mark it stale and require a rescan or rehash.

### 7. Logical path materialization

After unique payloads are imported, create logical FOD paths for all planned entries.

Duplicate source paths should point to the same underlying content object instead of storing the payload again.

Prefer FOD-level hardlink/data-object sharing semantics over external symlinks.

## Optional External Link Mode

An optional mode may replace duplicate source files outside FOD with links to a selected canonical source location, but this must be separate from the default import mode.

This mode is risky and must require explicit confirmation because it can modify user data outside FOD.

Rules for this mode:

- never run by default,
- require dry-run first,
- require backup/rollback metadata,
- only replace a file after full hash verification,
- never replace files that changed after indexing,
- prefer hardlinks only on the same filesystem,
- use symlinks only when the user explicitly accepts path fragility,
- never delete the last verified copy before import success is confirmed.

## Suggested Database Concepts

The first implementation can use FOD-owned tables or a dedicated schema for the indexer.

Possible tables:

```text
index_sources
index_scan_runs
index_files
index_file_hashes
index_duplicate_sets
index_import_plans
index_import_plan_entries
index_import_jobs
```

Core relationships:

```text
index_files(source_id, source_path, size, mtime, scan_run_id, status)
index_file_hashes(file_id, hash_kind, hash_value)
index_duplicate_sets(content_hash, representative_file_id)
index_import_plan_entries(plan_id, source_file_id, content_hash, destination_path, action)
```

The runtime FOD storage layer should remain separate from the indexer schema. The indexer produces a validated import plan; the FOD storage/import layer executes it.

## CLI / Service Shape

Suggested commands:

```text
fod-indexer source add NAME PATH
fod-indexer scan SOURCE
fod-indexer hash --candidates-only
fod-indexer plan-import --target /archive
fod-indexer import --plan PLAN_ID
fod-indexer report duplicates
fod-indexer report savings
fod-indexer verify PLAN_ID
```

The first service implementation can be a CLI plus PostgreSQL tables. A long-running daemon can be added later only if scheduling, monitoring, or background indexing is needed.

## Safety Rules

- Do not modify source files during indexing.
- Do not copy files into FOD until an import plan is produced.
- Do not trust metadata-only deduplication.
- Do not treat equal file names as duplicate proof.
- Do not treat equal file size as duplicate proof.
- Do not treat unavailable files as deleted unless the user explicitly asks for cleanup reconciliation.
- Always preserve original source path metadata.
- Always support dry-run and report mode.
- Always revalidate files before import.

## Better Alternative to External Links

Instead of copying one file and creating links to old external locations, prefer:

```text
one imported payload in FOD
many FOD logical paths / references
all original external source paths stored as metadata
```

This gives the deduplication benefit while keeping the FOD archive self-contained.

External source links are useful as audit metadata, but they should not be the primary storage model.

## Integration With Existing FOD Direction

`fod-indexer` should fit the existing FOD architecture:

- Rust implementation preferred,
- PostgreSQL as shared backend,
- no Python fallback for Rust-owned paths,
- no impact on FUSE hot path,
- import execution separated from scan/index planning,
- content dedup aligned with the existing storage and extent direction,
- all performance claims verified with real scan/import benchmarks.

## Initial Scope

Start narrow:

- local filesystem source only,
- metadata scan,
- full hash for duplicate candidates,
- duplicate report,
- dry-run import plan,
- no source modification,
- no daemon yet,
- no cloud connector yet,
- no automatic external symlink replacement.

After that, add:

- QNAP/NAS source support,
- Google Drive export/mount source support,
- phone/USB source support,
- scheduled scans,
- import execution,
- FOD-level deduplicated path materialization.
