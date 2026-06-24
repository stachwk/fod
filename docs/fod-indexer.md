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

The current implementation supports filesystem-backed source adapters. The adapter kind now carries an explicit policy and capability profile so the shared path-backed flow stays separate from future direct crawlers. The indexed root is still a local filesystem path today.

## Shared Core

`fod-indexer` is the shared indexing core for FOD. It owns source registration, scanning, hashing, duplicate detection, import planning, materialization, and cleanup for external sources.

`msfind` should reuse this same core instead of growing a separate indexing pipeline. If `msfind` needs new indexing behavior, that behavior should land in `fod-indexer` first and then be exposed through the existing source and capability model.

Keep source-specific logic at the adapter boundary. The core should stay responsible for the common scan/hash/plan/materialize/cleanup flow, while kinds such as `local`, `smb`, `qnap`, `adb`, and `github` only describe how a source is reached or mirrored.

## Source capabilities

The current supported kinds all still index a local path, but the CLI surfaces their intended storage model explicitly:

- `local`: policy `path-backed`
- `smb` / `qnap`: policy `mirrored`
- `adb` / `github`: policy `export-backed`

The underlying capability flags stay visible too:

- `local`: path-backed, read-only from the indexer's point of view, no mirror required, no export required, no direct crawler yet
- `smb` / `qnap`: path-backed, read-only, mirror-backed, direct crawler possible later
- `adb` / `github`: path-backed, read-only, mirror-backed, export-backed, direct crawler possible later

That keeps the registration contract simple while making the adapter boundary visible to users and future code.

Supported actions:

- `fod-indexer source add [--name <name>] --path <path> --kind local|smb|qnap|adb|github`
- `fod-indexer source list [--kind <kind>]`
- `fod-indexer source list --path <path> [--kind <kind>]`
- `fod-indexer source remove --name <name>`
- `fod-indexer scan --source <name>`
- `fod-indexer hash --source <name> --candidates-only`
- `fod-indexer report duplicates`
- `fod-indexer plan-import --dry-run`
- `fod-indexer clean --source <name> --dry-run`
- `fod-indexer materialize --source <name>`

If `--name` is omitted, `fod-indexer` uses a kind-aware naming heuristic with the current hostname as the final fallback. Examples:

- `local`: current hostname,
- `smb` / `qnap`: remote host or IP from the mounted share when it can be inferred,
- `adb`: device serial from `ANDROID_SERIAL`, `ADB_SERIAL`, `ADB_DEVICE_SERIAL`, or `adb devices`,
- `github`: git remote slug or repository name when the source path is a checkout.

Explicit `--name` stays available when you want to override the suggestion or register a source that does not fit the default heuristic.

Use `fod-indexer source list --kind adb` when you want to browse the detected ADB runtime root before scanning it. The command probes the device through `adb shell`, reads `EXTERNAL_STORAGE` and `SECONDARY_STORAGE`, and then maps the chosen storage root to a local `gvfs` mount when one is available so the printed `source add --path` values stay usable and shell-quoted. Use `fod-indexer source list --path /run/user/1000/adb/<serial> --kind adb` when you want to override the detected root and inspect one specific mounted device or copy a child directory path into `source add`.
Use `fod-indexer source remove --name <name>` to unregister an added source once you no longer want it indexed.

## Indexer Filters

`fod-indexer` reads an optional `[fod-indexer]` section from `fod_config.ini` and uses it as a skip list for scan, hash, planning, materialization previews, duplicate-report rebuilds, and cleanup walks.
Zero-length files are skipped during scan before they enter the index, so they do not reach hashing, planning, duplicate-report rebuilding, or materialization.

Supported keys:

- `skip_hidden = true|false`
- `skip_components = name1,name2,...`
- `skip_prefixes = path/prefix1,path/prefix2,...`
- `skip_paths = convenience alias that accepts either component names or nested prefixes`
- `allow_extensions = txt,pdf,jpg,png,doc,xls,...`

Examples:

```ini
[fod-indexer]
skip_hidden = true
skip_components = cache,caches,build,dist,node_modules,target,tmp,temp,out,__pycache__
skip_prefixes = work/cache,Android/data/com.example/cache
# Optional allowlist of file extensions to keep in the index.
# allow_extensions = txt,pdf,jpg,jpeg,png,gif,webp,heic,doc,docx,xls,xlsx,ppt,pptx,csv,odt,ods,odp,gdoc,gsheet,gslides
```

Plain names in `skip_paths` are treated as component matches, while values containing `/` or `\` are treated as relative path prefixes. Hidden dotfiles are skipped by default unless `skip_hidden = false` is set. Common Android game cache directories such as `DownloadCacheManager`, `PlatformRequestCache`, `ServerRequestCache`, and `UnityCache` are also skipped by default so phone scans stay focused on user files instead of large cache blobs. When `allow_extensions` is set, files without one of the listed extensions are skipped in scan, hash, plan, materialization previews, and cleanup tree rebuilding.

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
While a scan runs, `fod-indexer scan --source <name>` prints periodic progress lines to stderr with the scanned-file counts, current file path, and elapsed time.
While `fod-indexer hash --source <name>` runs, it prints periodic progress lines to stderr with candidate, partial, full, and retry-needed counts, plus the current file path and elapsed time.
`fod-indexer report duplicates` skips zero-size duplicate groups so cache and lock files do not dominate the report. In the current pipeline those groups should not appear because scan skips zero-length files before indexing.

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

Materialization writes into a per-run root directory inside FOD named `index-source-<source id>-import-<plan id>`, keeps the source tree untouched, and writes each canonical payload once while reusing the canonical data object for duplicate references.

## Cleanup

`clean --source <name>` compares the current source tree with the indexed rows for that source and removes stale file entries that no longer exist. The command also removes dependent import-plan entries that reference those stale files and refreshes duplicate-set metadata after a real cleanup.

Use `--dry-run` first if you want to preview which rows would be removed without touching PostgreSQL. If the source root itself is gone, the cleanup treats the source as fully orphaned and removes the indexed rows for that source.

Hidden dotfiles, zero-length files, and common cache/build directories are skipped during scan, hash, plan, materialize, and cleanup view rebuilding. That keeps paths such as `.bashrc`, `.venv`, `.git`, `node_modules`, `target`, `build`, and similar cache trees out of the index.

## Architecture note

`fod-indexer` should stay outside the FUSE hot path and behave as an offline/background indexing service. The intended pipeline is:

```text
scan sources -> index metadata -> hash candidates -> build duplicate sets -> create import plan -> import unique payloads -> materialize logical paths/links
```

Long-term, the indexer should support richer source types, but the default archival model should remain self-contained inside FOD. Optional external link replacement must stay opt-in and separate from the normal import path.

The indexer schema should remain separate from the runtime FOD storage layer, and all safety decisions should be based on revalidation rather than metadata alone. `msfind` should keep using this shared core rather than duplicating the pipeline inside its own codebase.
