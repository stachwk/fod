# FOD 3.3.31 - per-INI runtime logging

FOD 3.3.31 adds optional file logging for the Rust FUSE runtime. The destination is selected from the same INI that defines the mounted FOD instance, so multiple instances can use separate logs without separate wrapper scripts.

## Configuration

```ini
[logging]
enabled = true
directory = /var/log/fod
# filename = db-primary.log

[fod]
log_level = info
```

`log_level` continues to control severity. The new `[logging]` section controls the destination.

When `filename` is omitted, FOD derives the log name from the selected INI basename:

```text
/etc/fod/fod_config.ini -> /var/log/fod/fod_config.log
/etc/fod/db-primary.ini -> /var/log/fod/db-primary.log
/etc/fod/db-archive.ini -> /var/log/fod/db-archive.log
```

If two configuration files in different directories have the same basename and must not share a log, give one or both an explicit `filename`.

## Compatibility

Existing INI files without `[logging]` keep the previous stderr logging behavior. This avoids changing test and development behavior merely because the binary was upgraded.

The packaged `fod_config.example.ini` enables file logging so a normal installed instance writes under `/var/log/fod`.

## Overrides

Environment overrides have higher priority than INI settings:

```text
FOD_LOG_FILE=/absolute/path/file.log
FOD_LOG_DIR=/absolute/path/directory
FOD_LOG_LEVEL=debug
```

`FOD_LOG_FILE` has the highest priority and must be absolute. `FOD_LOG_DIR` overrides the INI directory while retaining the configured or INI-derived file name.

`logging.directory` must be absolute. `logging.filename` is a single file name, not a path; path separators and parent-directory traversal are rejected.

## Failure behavior

FOD opens the selected log file in append mode. If the configured file or directory cannot be created/opened, mount startup is not rejected solely because of logging. A warning is written to stderr and logging falls back to stderr.

Log records include timestamp, PID, level and message so concurrent processes or repeated mounts can be distinguished in one file.

## Packages

Native DEB/RPM packages create:

```text
/var/log/fod
```

with directory mode `0755`. New log files are requested with mode `0640` (subject to process umask).

Runtime 3.3.31 resets the native package revision to `3.3.31-1`.

## Scope

This change does not alter the FOD storage format, PostgreSQL schema, 4 KiB storage block size, FUSE request-size defaults, ACL/SELinux semantics, or Rust 1.98.0 + release-lto production build policy.
