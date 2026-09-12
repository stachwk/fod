#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

BLOCK_SIZE = 4096
FAIL_FAST_LIMIT_SECONDS = 1.0


def wait_for_size(path: Path, expected_size: int, timeout_seconds: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_size = None

    while time.monotonic() < deadline:
        try:
            last_size = path.stat().st_size
        except FileNotFoundError:
            last_size = None

        if last_size == expected_size:
            return

        time.sleep(0.05)

    raise AssertionError(
        f"{path} did not reach size {expected_size}; last_size={last_size}"
    )


def main() -> None:
    suffix = uuid.uuid4().hex[:12]
    name = f"first-writer-wins-{suffix}.bin"

    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    launcher_a.init_schema()

    fd_a = None
    fd_b = None

    with (
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-fww-a-{suffix}.") as mount_a_dir,
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-fww-b-{suffix}.") as mount_b_dir,
    ):
        mount_a = Path(mount_a_dir)
        mount_b = Path(mount_b_dir)

        launcher_a.start(str(mount_a), log_prefix="fod-fww-a")
        launcher_b.start(str(mount_b), log_prefix="fod-fww-b")

        path_a = mount_a / name
        path_b = mount_b / name

        try:
            baseline = b"S" * (2 * BLOCK_SIZE)
            writer_a_block = b"A" * BLOCK_SIZE

            descriptor = os.open(
                path_a,
                os.O_WRONLY | os.O_CREAT | os.O_TRUNC,
                0o644,
            )
            try:
                written = os.write(descriptor, baseline)
                if written != len(baseline):
                    raise AssertionError(
                        f"short baseline write: {written} != {len(baseline)}"
                    )
                os.fsync(descriptor)
            finally:
                os.close(descriptor)

            wait_for_size(path_b, len(baseline))

            # Writer A zdobywa cel pierwszy i utrzymuje aktywny uchwyt.
            fd_a = os.open(path_a, os.O_WRONLY | os.O_TRUNC)
            written = os.pwrite(fd_a, writer_a_block, 0)
            if written != BLOCK_SIZE:
                raise AssertionError(
                    f"short writer A first block: {written} != {BLOCK_SIZE}"
                )
            os.fsync(fd_a)

            # Writer B nie moze czekac ani wykonac O_TRUNC.
            started = time.monotonic()
            second_errno = None
            try:
                fd_b = os.open(path_b, os.O_WRONLY | os.O_TRUNC)
            except OSError as error:
                second_errno = error.errno
            elapsed = time.monotonic() - started

            if fd_b is not None:
                os.close(fd_b)
                fd_b = None
                raise AssertionError(
                    "second writer opened active destination; "
                    f"expected EBUSY, elapsed={elapsed:.6f}s"
                )

            if second_errno != errno.EBUSY:
                raise AssertionError(
                    f"second writer errno={second_errno}; expected EBUSY={errno.EBUSY}"
                )

            if elapsed >= FAIL_FAST_LIMIT_SECONDS:
                raise AssertionError(
                    "second writer did not fail fast: "
                    f"elapsed={elapsed:.6f}s limit={FAIL_FAST_LIMIT_SECONDS:.6f}s"
                )

            # Pierwszy writer nadal moze kontynuowac po odrzuceniu konkurenta.
            written = os.pwrite(fd_a, writer_a_block, BLOCK_SIZE)
            if written != BLOCK_SIZE:
                raise AssertionError(
                    f"short writer A second block: {written} != {BLOCK_SIZE}"
                )
            os.fsync(fd_a)
            os.close(fd_a)
            fd_a = None

            expected = writer_a_block + writer_a_block
            deadline = time.monotonic() + 10.0
            observed = b""

            while time.monotonic() < deadline:
                observed = path_a.read_bytes()
                if observed == expected:
                    break
                time.sleep(0.05)

            if observed != expected:
                raise AssertionError(
                    "winner payload mismatch after loser rejection: "
                    f"size={len(observed)}"
                )

            print(
                "OK first-writer-wins "
                f"loser_errno={second_errno} "
                f"loser_elapsed_ms={elapsed * 1000.0:.3f} "
                f"winner_size={len(observed)}"
            )

        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
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

            try:
                path_a.unlink()
            except OSError:
                pass

            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
