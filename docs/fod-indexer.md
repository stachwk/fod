# fod-indexer

`fod-indexer` is a FOD-side indexing tool for external file sources.

## Goal

The tool scans external sources, groups files by content, and prepares a safe import plan before any data is materialized into FOD. Importing into FOD is non-destructive for the source tree:

- do not delete source files,
- do not rename or move source files,
- do not replace source files with symlinks,
- do not write marker files into source trees.

## Identity model

The design keeps these concepts separate:

- physical source file
- content payload
- logical FOD path or reference

Duplicate detection must be content-based. A matching filename is never enough.

## MVP scope

The first implementation supports local filesystem sources only.

Supported actions:

- `fod-indexer source add [--name <name>] --path <path> --kind local`
- `fod-indexer scan --source <name>`
- `fod-indexer hash --source <name> --candidates-only`
- `fod-indexer report duplicates`
- `fod-indexer plan-import --dry-run`
- `fod-indexer clean --source <name> --dry-run`
- `fod-indexer materialize --source <name>`

If `--name` is omitted for a local source, `fod-indexer` uses the current hostname as the default source name. Explicit `--name` stays available when you want to override the suggestion or register a source that does not fit the default heuristic.

## Hashing strategy

The staged dedupe flow is:

1. group by file size,
2. optionally consider modification time as metadata,
3. compute a partial hash from the first, middle, and last chunks for candidate files,
4. compute a full SHA-256 hash only when the partial hash still leaves duplicates,
5. treat the pair `(full hash, size)` as the duplicate-set identity.

Zero-length files may be grouped as a special duplicate case.

## Scan status

Scanner status values are explicit:

- `ok`
- `unreadable`
- `unsupported_type`
- `stat_failed`

Unreadable files should be recorded and the scan should continue. A database write failure is the only case that should abort the scan.

## Import planning

`plan-import --dry-run` produces a human-readable summary and stores a plan snapshot. Canonical payload selection must be deterministic, for example:

- lowest source id,
- then shortest path,
- then lexical path order.

The dry run should report:

- scanned file count,
- candidate duplicate groups,
- confirmed duplicate groups,
- unique payload count,
- total source bytes,
- estimated bytes to import after dedupe,
- saved bytes,
- canonical source file for each duplicate set,
- logical references for the remaining paths.

## Materialization

`materialize --source <name>` revalidates the current source view before creating FOD entries. The import phase checks:

- size,
- modification time when available,
- full hash,
- source path,
- optional inode and device ids when available.

Materialization writes into a per-run root directory inside FOD named `index-source-<source id>-import-<plan id>`, keeps the source tree untouched, writes each canonical payload once, and reuses the canonical data object for duplicate references when the payload is non-empty. Zero-length duplicates remain harmless independent zero-sized entries.

## Cleanup

`clean --source <name>` compares the current local source tree with the indexed rows for that source and removes stale file entries that no longer exist. The command also removes dependent import-plan entries that reference those stale files and refreshes duplicate-set metadata after a real cleanup.

Use `--dry-run` first if you want to preview which rows would be removed without touching PostgreSQL. If the source root itself is gone, the cleanup treats the source as fully orphaned and removes the indexed rows for that source.

## Architecture note

`fod-indexer` should stay outside the FUSE hot path and behave as an offline/background indexing service. The intended pipeline is:

```text
scan sources -> index metadata -> hash candidates -> build duplicate sets -> create import plan -> import unique payloads -> materialize logical paths/links
```

Long-term, the indexer should support richer source types, but the default archival model should remain self-contained inside FOD. Optional external link replacement must stay opt-in and separate from the normal import path.

The indexer schema should remain separate from the runtime FOD storage layer, and all safety decisions should be based on revalidation rather than metadata alone.
