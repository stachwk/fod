#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import sys
import tempfile
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


def main() -> None:
    launcher_a = FODMount(str(ROOT), role="primary")
    launcher_b = FODMount(str(ROOT), role="primary")
    launcher_a.acl = "on"
    launcher_b.acl = "on"
    launcher_a.init_schema()

    suffix = uuid.uuid4().hex
    name = "user.fod_stale_inode"
    original_value = b"original-inode"
    updated_value = b"updated-open-inode"
    replacement_value = b"replacement-inode"

    with tempfile.TemporaryDirectory(prefix="fod-xattr-stale-a.") as mount_a_tmp, tempfile.TemporaryDirectory(
        prefix="fod-xattr-stale-b."
    ) as mount_b_tmp:
        mount_a = Path(mount_a_tmp)
        mount_b = Path(mount_b_tmp)
        timeout_names = (
            "FOD_ENTRY_TIMEOUT_SECONDS",
            "FOD_ATTR_TIMEOUT_SECONDS",
            "FOD_NEGATIVE_TIMEOUT_SECONDS",
        )
        previous_timeouts = {name: os.environ.get(name) for name in timeout_names}
        os.environ["FOD_ENTRY_TIMEOUT_SECONDS"] = "60"
        os.environ["FOD_ATTR_TIMEOUT_SECONDS"] = "30"
        os.environ["FOD_NEGATIVE_TIMEOUT_SECONDS"] = "60"
        stale_path_a: Path | None = None
        missing_path_a: Path | None = None
        try:
            launcher_a.start(str(mount_a))
            launcher_b.start(str(mount_b))
            path_a: Path | None = None
            try:
                path_a = mount_a / f"stale-{suffix}.txt"
                path_b = mount_b / path_a.name
                path_a.write_bytes(b"original")
                os.setxattr(path_a, name, original_value)

                fd_b = os.open(path_b, os.O_RDONLY)
                try:
                    if os.getxattr(fd_b, name) != original_value:
                        raise AssertionError("second mount did not read original inode xattr")
                    if name not in os.listxattr(fd_b):
                        raise AssertionError("second mount did not list original inode xattr")

                    path_a.unlink()
                    path_a.write_bytes(b"replacement")
                    os.setxattr(path_a, name, replacement_value)

                    if os.getxattr(fd_b, name) != original_value:
                        raise AssertionError("open unlinked inode followed replacement xattr")
                    if name not in os.listxattr(fd_b):
                        raise AssertionError("open unlinked inode lost its xattr name")

                    os.setxattr(fd_b, name, updated_value)
                    if os.getxattr(fd_b, name) != updated_value:
                        raise AssertionError("fsetxattr did not update the open unlinked inode")
                    if os.getxattr(path_a, name) != replacement_value:
                        raise AssertionError("fsetxattr modified the replacement inode")

                    os.removexattr(fd_b, name)
                    if name in os.listxattr(fd_b):
                        raise AssertionError("fremovexattr left the old inode xattr listed")
                    try:
                        os.getxattr(fd_b, name)
                    except OSError as exc:
                        if exc.errno != errno.ENODATA:
                            raise
                    else:
                        raise AssertionError("removed old inode xattr remained readable")

                    if os.getxattr(path_a, name) != replacement_value:
                        raise AssertionError("fremovexattr modified the replacement inode")
                finally:
                    os.close(fd_b)

                stale_path_a = mount_a / f"stale-path-{suffix}.txt"
                stale_path_b = mount_b / stale_path_a.name
                stale_path_a.write_bytes(b"old path inode")
                os.setxattr(stale_path_a, name, original_value)
                stale_path_b.stat()
                if os.getxattr(stale_path_b, name) != original_value:
                    raise AssertionError("second mount did not prime the old path inode")

                stale_path_a.unlink()
                stale_path_a.write_bytes(b"replacement path inode")
                os.setxattr(stale_path_a, name, replacement_value)

                try:
                    os.setxattr(stale_path_b, name, updated_value)
                except OSError as exc:
                    if exc.errno != errno.ENOENT:
                        raise AssertionError(
                            "stale path xattr mutation returned unexpected errno "
                            f"{exc.errno}, expected ENOENT"
                        ) from exc
                else:
                    raise AssertionError(
                        "setxattr through the cached old path inode did not fail closed"
                    )

                if os.getxattr(stale_path_a, name) != replacement_value:
                    raise AssertionError(
                        "stale path setxattr modified the replacement inode"
                    )

                missing_path_a = mount_a / f"negative-cache-{suffix}.txt"
                missing_path_b = mount_b / missing_path_a.name
                try:
                    missing_path_b.stat()
                except FileNotFoundError:
                    pass
                else:
                    raise AssertionError("negative-cache target unexpectedly exists")
                missing_path_a.write_bytes(b"created after negative lookup")
                try:
                    missing_path_b.stat()
                except FileNotFoundError:
                    pass
                else:
                    raise AssertionError("negative lookup was not cached on mount B")
            finally:
                if path_a is not None:
                    try:
                        path_a.unlink()
                    except FileNotFoundError:
                        pass
                if stale_path_a is not None:
                    try:
                        stale_path_a.unlink()
                    except FileNotFoundError:
                        pass
                if missing_path_a is not None:
                    try:
                        missing_path_a.unlink()
                    except FileNotFoundError:
                        pass
                launcher_b.stop()
        finally:
            launcher_a.stop()
            for name, previous_value in previous_timeouts.items():
                if previous_value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = previous_value

    print(
        "OK: stale inode xattr operations stay bound to the open file and "
        "cached path replacements fail closed with ACL enabled"
    )


if __name__ == "__main__":
    main()
