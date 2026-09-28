#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount

WAIT_SECONDS = 10.0


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def psql_scalar(launcher: FODMount, sql: str) -> str:
    env = os.environ.copy()
    env["PGPASSWORD"] = launcher.postgres_password
    host = env.get("FOD_PG_HOST") or env.get("POSTGRES_HOST") or "127.0.0.1"
    port = env.get("FOD_PG_PORT") or env.get("POSTGRES_PORT") or "5432"

    result = subprocess.run(
        [
            "psql",
            "-h",
            host,
            "-p",
            port,
            "-U",
            launcher.postgres_user,
            "-d",
            launcher.postgres_db,
            "-At",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ],
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def ownership_counts(launcher: FODMount, name: str) -> tuple[int, int]:
    raw = psql_scalar(
        launcher,
        f"""
        SELECT
          (SELECT COUNT(*)
             FROM fod.destination_write_leases
            WHERE parent_key = 0
              AND name = {sql_quote(name)})::text || '|' ||
          (SELECT COUNT(*)
             FROM fod.file_write_leases fwl
             JOIN fod.files f ON f.id_file = fwl.file_id
            WHERE f.id_directory IS NULL
              AND f.name = {sql_quote(name)})::text
        """,
    )
    destination_count, file_count = raw.split("|", 1)
    return int(destination_count), int(file_count)


def wait_for_counts(
    launcher: FODMount,
    name: str,
    expected: tuple[int, int],
) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    last = (-1, -1)

    while time.monotonic() < deadline:
        last = ownership_counts(launcher, name)
        if last == expected:
            return
        time.sleep(0.05)

    raise AssertionError(
        f"ownership counts for {name}: expected={expected} observed={last}"
    )


def wait_for_bytes(path: Path, expected: bytes) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    observed: bytes | None = None

    while time.monotonic() < deadline:
        try:
            observed = path.read_bytes()
        except FileNotFoundError:
            observed = None

        if observed == expected:
            return
        time.sleep(0.05)

    raise AssertionError(
        f"{path}: expected payload {expected!r}, observed={observed!r}"
    )


def destination_writer_bypass(
    launcher_a: FODMount,
    launcher_b: FODMount,
    mount_a: Path,
    mount_b: Path,
) -> None:
    suffix = uuid.uuid4().hex
    destination_name = f"rename-owned-destination-{suffix}.bin"
    temporary_name = f"rename-temp-destination-{suffix}.bin"

    destination_a = mount_a / destination_name
    destination_b = mount_b / destination_name
    temporary_b = mount_b / temporary_name

    original = b"destination-owned-by-writer-a"
    replacement = b"replacement-from-mount-b"

    destination_a.write_bytes(original)
    wait_for_bytes(destination_b, original)

    fd = os.open(destination_a, os.O_WRONLY)
    try:
        wait_for_counts(launcher_a, destination_name, (1, 1))

        temporary_b.write_bytes(replacement)
        wait_for_counts(launcher_b, temporary_name, (0, 0))

        started = time.monotonic()
        os.replace(temporary_b, destination_b)
        elapsed = time.monotonic() - started

        wait_for_bytes(destination_b, replacement)

        if temporary_b.exists():
            raise AssertionError("temporary source still exists after baseline replace")

        after = ownership_counts(launcher_a, destination_name)

        print(
            "BASELINE_UNSAFE rename-active-destination-writer "
            f"replace_result=SUCCESS elapsed_ms={elapsed * 1000.0:.3f} "
            f"lease_before=1|1 lease_after={after[0]}|{after[1]} "
            f"final_payload={replacement.decode('ascii')}"
        )
    finally:
        os.close(fd)

    wait_for_counts(launcher_a, destination_name, (0, 0))


def source_writer_bypass(
    launcher_a: FODMount,
    launcher_b: FODMount,
    mount_a: Path,
    mount_b: Path,
) -> None:
    suffix = uuid.uuid4().hex
    source_name = f"rename-owned-source-{suffix}.bin"
    destination_name = f"rename-source-target-{suffix}.bin"

    source_a = mount_a / source_name
    source_b = mount_b / source_name
    destination_b = mount_b / destination_name
    destination_a = mount_a / destination_name

    payload = b"source-owned-by-writer-a"

    source_a.write_bytes(payload)
    wait_for_bytes(source_b, payload)

    fd = os.open(source_a, os.O_WRONLY)
    try:
        wait_for_counts(launcher_a, source_name, (1, 1))

        started = time.monotonic()
        os.replace(source_b, destination_b)
        elapsed = time.monotonic() - started

        wait_for_bytes(destination_a, payload)

        if source_b.exists():
            raise AssertionError("source still exists after baseline rename")

        old_name_counts = ownership_counts(launcher_a, source_name)
        new_name_counts = ownership_counts(launcher_a, destination_name)

        print(
            "BASELINE_UNSAFE rename-active-source-writer "
            f"rename_result=SUCCESS elapsed_ms={elapsed * 1000.0:.3f} "
            f"old_name_lease={old_name_counts[0]}|{old_name_counts[1]} "
            f"new_name_lease={new_name_counts[0]}|{new_name_counts[1]}"
        )
    finally:
        os.close(fd)

    wait_for_counts(launcher_a, source_name, (0, 0))
    wait_for_counts(launcher_a, destination_name, (0, 0))


def main() -> None:
    root = Path(__file__).resolve().parents[2]

    launcher_a = FODMount(str(root))
    launcher_b = FODMount(str(root))
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-rename-own-baseline-") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"

        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-rename-own-base-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-rename-own-base-b")

            destination_writer_bypass(
                launcher_a,
                launcher_b,
                mount_a,
                mount_b,
            )
            source_writer_bypass(
                launcher_a,
                launcher_b,
                mount_a,
                mount_b,
            )

            print(
                "OK rename-write-ownership-baseline "
                "known_gap_reproduced=2"
            )
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
