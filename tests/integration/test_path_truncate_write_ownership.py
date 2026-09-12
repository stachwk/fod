#!/usr/bin/env python3
from __future__ import annotations

import errno
import os
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount

FAIL_FAST_SECONDS = 1.0
FILE_NAME = "path-truncate-write-ownership.bin"
INITIAL = b"0123456789abcdef"


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-path-truncate-ownership-") as temp_dir:
        temp = Path(temp_dir)
        mount_a_dir = temp / "mount-a"
        mount_b_dir = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        fd_a = None

        try:
            launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-truncate-own-a")
            launcher_b.start(str(mount_b_dir), log_prefix="/tmp/fod-truncate-own-b")

            path_a = mount_a_dir / FILE_NAME
            path_b = mount_b_dir / FILE_NAME
            path_a.write_bytes(INITIAL)

            fd_a = os.open(path_a, os.O_WRONLY)

            started = time.monotonic()
            loser_errno = None
            try:
                os.truncate(path_b, 0)
            except OSError as exc:
                loser_errno = exc.errno
            elapsed = time.monotonic() - started

            if elapsed >= FAIL_FAST_SECONDS:
                raise AssertionError(
                    f"path truncate loser did not fail fast: {elapsed:.6f}s"
                )
            if loser_errno != errno.EBUSY:
                raise AssertionError(
                    "path-based truncate bypassed write ownership: "
                    f"loser_errno={loser_errno}, expected EBUSY"
                )

            size_after = os.stat(path_a).st_size
            if size_after != len(INITIAL):
                raise AssertionError(
                    f"loser mutated file size: {size_after}, expected {len(INITIAL)}"
                )

            print(
                "OK path-truncate-write-ownership "
                f"loser_errno={loser_errno} "
                f"loser_elapsed_ms={elapsed * 1000.0:.3f} "
                f"size={size_after}"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd_a is not None:
                try:
                    os.close(fd_a)
                except OSError:
                    pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
