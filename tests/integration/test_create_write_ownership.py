#!/usr/bin/env python3
from __future__ import annotations

import errno
import os
import tempfile
import time
from pathlib import Path

from fod_mount import FODMount

FAIL_FAST_SECONDS = 1.0
FILE_NAME = "create-write-ownership.bin"


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    with tempfile.TemporaryDirectory(prefix="fod-create-ownership-") as temp_dir:
        temp = Path(temp_dir)
        mount_a_dir = temp / "mount-a"
        mount_b_dir = temp / "mount-b"

        launcher_a = FODMount(str(root))
        launcher_b = FODMount(str(root))
        launcher_a.init_schema()

        fd_a = None
        fd_b = None

        try:
            launcher_a.start(str(mount_a_dir), log_prefix="/tmp/fod-create-own-a")
            launcher_b.start(str(mount_b_dir), log_prefix="/tmp/fod-create-own-b")

            path_a = mount_a_dir / FILE_NAME
            path_b = mount_b_dir / FILE_NAME

            fd_a = os.open(
                path_a,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o644,
            )
            os.write(fd_a, b"A")

            started = time.monotonic()
            loser_errno = None
            try:
                fd_b = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            except OSError as exc:
                loser_errno = exc.errno
            elapsed = time.monotonic() - started

            if elapsed >= FAIL_FAST_SECONDS:
                raise AssertionError(
                    f"create ownership loser did not fail fast: {elapsed:.6f}s"
                )
            if loser_errno != errno.EBUSY:
                raise AssertionError(
                    "create handle did not own destination: "
                    f"loser_errno={loser_errno}, expected EBUSY"
                )

            print(
                "OK create-write-ownership "
                f"loser_errno={loser_errno} "
                f"loser_elapsed_ms={elapsed * 1000.0:.3f}"
            )
        except Exception:
            launcher_a._dump_log()
            launcher_b._dump_log()
            raise
        finally:
            if fd_b is not None:
                try:
                    os.close(fd_b)
                except OSError:
                    pass
            if fd_a is not None:
                try:
                    os.close(fd_a)
                except OSError:
                    pass
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
