#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import shutil
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def expect_errno(expected: int, operation, label: str) -> None:
    try:
        operation()
    except OSError as error:
        assert error.errno == expected, (label, error.errno, expected)
    else:
        raise AssertionError(f"{label} unexpectedly succeeded")


def main() -> None:
    suffix = uuid.uuid4().hex[:10]
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    with tempfile.TemporaryDirectory(prefix=f"fod-symlink-hardlink-{suffix}-") as temp:
        mountpoint = Path(temp)
        launcher.start(str(mountpoint))
        directory = mountpoint / f"nested-{suffix}"
        payload = directory / "payload"
        source = directory / "source-link"
        alias = directory / "alias-link"
        renamed_alias = directory / "renamed-alias-link"
        replacement = directory / "replacement-link"
        replacement_alias = directory / "replacement-alias"
        root_alias = mountpoint / f"root-alias-{suffix}"
        try:
            directory.mkdir()
            payload.write_bytes(b"symlink hardlink target\n")
            os.symlink(payload.name, source)

            expect_errno(
                errno.ENOENT,
                lambda: os.link(directory / "missing", directory / "missing-alias", follow_symlinks=False),
                "link missing symlink",
            )
            expect_errno(
                errno.EPERM,
                lambda: os.link(directory, directory / "directory-alias", follow_symlinks=False),
                "link directory",
            )

            os.link(source, alias, follow_symlinks=False)
            expect_errno(
                errno.EEXIST,
                lambda: os.link(source, alias, follow_symlinks=False),
                "existing symlink alias",
            )
            original = os.lstat(source)
            linked = os.lstat(alias)
            assert os.path.islink(source) and os.path.islink(alias)
            assert original.st_ino == linked.st_ino, (original.st_ino, linked.st_ino)
            assert original.st_nlink == linked.st_nlink == 2, (original.st_nlink, linked.st_nlink)
            assert os.readlink(source) == os.readlink(alias) == payload.name

            names = {entry.name: entry.inode() for entry in os.scandir(directory)}
            assert names["source-link"] == names["alias-link"] == original.st_ino, names

            # Symlink timestamps are inode metadata too; use microsecond-aligned
            # values because PostgreSQL TIMESTAMP has microsecond resolution.
            timestamp_ns = 1_700_000_000_123_456_000
            os.utime(source, ns=(timestamp_ns, timestamp_ns), follow_symlinks=False)
            assert os.lstat(alias).st_mtime_ns == timestamp_ns

            os.rename(alias, renamed_alias)
            assert os.lstat(renamed_alias).st_ino == original.st_ino
            assert os.lstat(renamed_alias).st_nlink == 2
            source.unlink()
            promoted = os.lstat(renamed_alias)
            assert promoted.st_ino == original.st_ino
            assert promoted.st_nlink == 1
            assert os.readlink(renamed_alias) == payload.name
            assert not os.path.lexists(source)

            os.symlink("replacement-target", replacement)
            os.link(replacement, replacement_alias, follow_symlinks=False)
            replacement_ino = os.lstat(replacement).st_ino
            source_ino = os.lstat(renamed_alias).st_ino
            os.rename(renamed_alias, replacement)
            assert os.lstat(replacement).st_ino == source_ino
            assert os.readlink(replacement) == payload.name
            assert os.lstat(replacement_alias).st_ino == replacement_ino
            assert os.readlink(replacement_alias) == "replacement-target"
            assert os.lstat(replacement_alias).st_nlink == 1

            os.link(replacement_alias, root_alias, follow_symlinks=False)
            assert os.lstat(root_alias).st_ino == replacement_ino
            assert os.lstat(root_alias).st_nlink == 2
            assert os.readlink(root_alias) == os.readlink(replacement_alias)
            assert any(entry.name == root_alias.name for entry in os.scandir(mountpoint))
            replacement_alias.unlink()
            assert os.lstat(root_alias).st_nlink == 1
            assert os.readlink(root_alias) == "replacement-target"
            root_alias.unlink()
            assert not os.path.lexists(root_alias)

            replacement.unlink()
            assert not os.path.lexists(replacement)
            assert not os.path.lexists(replacement_alias)
            print(
                "OK symlink-hardlink mounted=1 stable_inode=1 shared_target=1 "
                "nlink=1 rename=1 canonical_promotion=1 replacement_promotion=1 root=1"
            )
        finally:
            for path in (
                root_alias,
                replacement_alias,
                replacement,
                renamed_alias,
                alias,
                source,
                payload,
            ):
                try:
                    path.unlink()
                except FileNotFoundError:
                    pass
            try:
                directory.rmdir()
            except OSError:
                pass
            launcher.stop()
            # The mount is stopped before removing its temporary mountpoint.
            shutil.rmtree(mountpoint, ignore_errors=True)


if __name__ == "__main__":
    main()
