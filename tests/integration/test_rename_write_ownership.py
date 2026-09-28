#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import errno
import os
import subprocess
import tempfile
import threading
import time
import uuid
from pathlib import Path

from fod_mount import FODMount

BARRIER_ENV = "FOD_TEST_RENAME_BEFORE_OWNERSHIP_BARRIER_DIR"
BARRIER_TIMEOUT_SECONDS = 10.0
FAIL_FAST_SECONDS = 1.0
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


def wait_for_absent(path: Path) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        if not path.exists():
            return
        time.sleep(0.05)
    raise AssertionError(f"{path}: path still exists after timeout")


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


def expect_replace_ebusy(source: Path, destination: Path, label: str) -> float:
    started = time.monotonic()
    observed_errno = None
    try:
        os.replace(source, destination)
    except OSError as exc:
        observed_errno = exc.errno
    elapsed = time.monotonic() - started

    if observed_errno != errno.EBUSY:
        raise AssertionError(
            f"{label}: errno={observed_errno}, expected EBUSY={errno.EBUSY}"
        )
    if elapsed >= FAIL_FAST_SECONDS:
        raise AssertionError(
            f"{label}: did not fail fast; elapsed={elapsed:.6f}s"
        )
    return elapsed


def arm_rename_barrier(barrier_dir: Path, path: str) -> None:
    for ready in barrier_dir.glob("ready.*"):
        ready.unlink(missing_ok=True)
    for name in ("target", "release"):
        (barrier_dir / name).unlink(missing_ok=True)
    (barrier_dir / "target").write_text(path, encoding="utf-8")


def wait_rename_barrier_ready(barrier_dir: Path, expected: int = 2) -> None:
    deadline = time.monotonic() + BARRIER_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if len(list(barrier_dir.glob("ready.*"))) >= expected:
            return
        time.sleep(0.01)
    raise AssertionError(
        f"rename barrier did not reach expected participants={expected}"
    )


def release_rename_barrier(barrier_dir: Path) -> None:
    (barrier_dir / "release").write_text("release\n", encoding="utf-8")


def disarm_rename_barrier(barrier_dir: Path) -> None:
    for ready in barrier_dir.glob("ready.*"):
        ready.unlink(missing_ok=True)
    for name in ("target", "release"):
        (barrier_dir / name).unlink(missing_ok=True)


def destination_writer_is_fenced(
    launcher: FODMount,
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
        wait_for_counts(launcher, destination_name, (1, 1))
        temporary_b.write_bytes(replacement)
        wait_for_counts(launcher, temporary_name, (0, 0))
        elapsed = expect_replace_ebusy(
            temporary_b,
            destination_b,
            "active destination writer",
        )
        wait_for_bytes(destination_a, original)
        wait_for_bytes(destination_b, original)
        wait_for_bytes(temporary_b, replacement)
        if ownership_counts(launcher, destination_name) != (1, 1):
            raise AssertionError(
                "destination writer ownership changed after rejected replace"
            )
        print(
            "OK rename-active-destination-writer "
            f"errno={errno.EBUSY} elapsed_ms={elapsed * 1000.0:.3f} "
            "destination_unchanged=1 source_preserved=1"
        )
    finally:
        os.close(fd)

    wait_for_counts(launcher, destination_name, (0, 0))
    os.replace(temporary_b, destination_b)
    wait_for_bytes(destination_a, replacement)
    wait_for_bytes(destination_b, replacement)


def source_writer_is_fenced(
    launcher: FODMount,
    mount_a: Path,
    mount_b: Path,
) -> None:
    suffix = uuid.uuid4().hex
    source_name = f"rename-owned-source-{suffix}.bin"
    destination_name = f"rename-source-target-{suffix}.bin"
    source_a = mount_a / source_name
    source_b = mount_b / source_name
    destination_a = mount_a / destination_name
    destination_b = mount_b / destination_name
    payload = b"source-owned-by-writer-a"

    source_a.write_bytes(payload)
    wait_for_bytes(source_b, payload)

    fd = os.open(source_a, os.O_WRONLY)
    try:
        wait_for_counts(launcher, source_name, (1, 1))
        elapsed = expect_replace_ebusy(
            source_b,
            destination_b,
            "active source writer",
        )
        wait_for_bytes(source_a, payload)
        wait_for_bytes(source_b, payload)
        if destination_a.exists() or destination_b.exists():
            raise AssertionError("destination appeared after rejected source rename")
        if ownership_counts(launcher, source_name) != (1, 1):
            raise AssertionError("source ownership changed after rejected rename")
        print(
            "OK rename-active-source-writer "
            f"errno={errno.EBUSY} elapsed_ms={elapsed * 1000.0:.3f} "
            "source_preserved=1 destination_absent=1"
        )
    finally:
        os.close(fd)

    wait_for_counts(launcher, source_name, (0, 0))
    os.replace(source_b, destination_b)
    wait_for_bytes(destination_a, payload)
    wait_for_bytes(destination_b, payload)
    wait_for_absent(source_a)
    wait_for_absent(source_b)


def hardlink_target_writer_is_fenced(
    launcher: FODMount,
    mount_a: Path,
    mount_b: Path,
) -> None:
    suffix = uuid.uuid4().hex
    target_name = f"rename-hardlink-target-{suffix}.bin"
    alias_name = f"rename-hardlink-alias-{suffix}.bin"
    temporary_name = f"rename-hardlink-temp-{suffix}.bin"

    target_a = mount_a / target_name
    target_b = mount_b / target_name
    alias_a = mount_a / alias_name
    alias_b = mount_b / alias_name
    temporary_b = mount_b / temporary_name

    original = b"hardlink-owned-target"
    replacement = b"hardlink-replacement"

    target_a.write_bytes(original)
    wait_for_bytes(target_b, original)
    os.link(target_a, alias_a)
    wait_for_bytes(alias_b, original)

    fd = os.open(alias_a, os.O_WRONLY)
    try:
        wait_for_counts(launcher, target_name, (0, 1))
        temporary_b.write_bytes(replacement)
        wait_for_counts(launcher, temporary_name, (0, 0))

        elapsed = expect_replace_ebusy(
            temporary_b,
            target_b,
            "hardlink alias writer",
        )

        wait_for_bytes(target_a, original)
        wait_for_bytes(target_b, original)
        wait_for_bytes(alias_a, original)
        wait_for_bytes(alias_b, original)
        wait_for_bytes(temporary_b, replacement)

        print(
            "OK rename-hardlink-target-writer "
            f"errno={errno.EBUSY} elapsed_ms={elapsed * 1000.0:.3f} "
            "target_unchanged=1 source_preserved=1"
        )
    finally:
        os.close(fd)

    wait_for_counts(launcher, target_name, (0, 0))
    os.replace(temporary_b, target_b)
    wait_for_bytes(target_a, replacement)
    wait_for_bytes(target_b, replacement)
    wait_for_bytes(alias_a, original)
    wait_for_bytes(alias_b, original)


def same_destination_race(
    launcher: FODMount,
    mount_a: Path,
    mount_b: Path,
    barrier_dir: Path,
    existing_destination: bool,
) -> None:
    suffix = uuid.uuid4().hex
    source_a_name = f"rename-race-a-{suffix}.bin"
    source_b_name = f"rename-race-b-{suffix}.bin"
    destination_name = f"rename-race-destination-{suffix}.bin"

    source_a = mount_a / source_a_name
    source_a_other = mount_b / source_a_name
    source_b = mount_b / source_b_name
    source_b_other = mount_a / source_b_name
    destination_a = mount_a / destination_name
    destination_b = mount_b / destination_name

    payload_a = b"rename-race-payload-a"
    payload_b = b"rename-race-payload-b"
    initial = b"rename-race-initial-target"

    source_a.write_bytes(payload_a)
    source_b.write_bytes(payload_b)
    wait_for_bytes(source_a_other, payload_a)
    wait_for_bytes(source_b_other, payload_b)
    wait_for_counts(launcher, source_a_name, (0, 0))
    wait_for_counts(launcher, source_b_name, (0, 0))

    if existing_destination:
        destination_a.write_bytes(initial)
        wait_for_bytes(destination_b, initial)
        wait_for_counts(launcher, destination_name, (0, 0))

    arm_rename_barrier(barrier_dir, f"/{destination_name}")

    results: dict[str, int | None] = {}
    elapsed: dict[str, float] = {}

    def worker(label: str, source: Path, destination: Path) -> None:
        started = time.monotonic()
        try:
            os.replace(source, destination)
            results[label] = None
        except OSError as exc:
            results[label] = exc.errno
        elapsed[label] = time.monotonic() - started

    thread_a = threading.Thread(
        target=worker,
        args=("A", source_a, destination_a),
        daemon=True,
    )
    thread_b = threading.Thread(
        target=worker,
        args=("B", source_b, destination_b),
        daemon=True,
    )

    thread_a.start()
    thread_b.start()
    wait_rename_barrier_ready(barrier_dir, expected=2)
    release_rename_barrier(barrier_dir)

    thread_a.join(BARRIER_TIMEOUT_SECONDS)
    thread_b.join(BARRIER_TIMEOUT_SECONDS)
    disarm_rename_barrier(barrier_dir)

    if thread_a.is_alive() or thread_b.is_alive():
        raise AssertionError("rename race worker did not finish")

    winners = [label for label, value in results.items() if value is None]
    losers = [label for label, value in results.items() if value == errno.EBUSY]
    if len(winners) != 1 or len(losers) != 1:
        raise AssertionError(
            f"rename race expected one winner and one EBUSY loser, results={results}"
        )

    winner = winners[0]
    loser = losers[0]
    winner_payload = payload_a if winner == "A" else payload_b
    loser_payload = payload_b if loser == "B" else payload_a

    wait_for_bytes(destination_a, winner_payload)
    wait_for_bytes(destination_b, winner_payload)

    if winner == "A":
        wait_for_absent(source_a)
        wait_for_absent(source_a_other)
        wait_for_bytes(source_b, loser_payload)
        wait_for_bytes(source_b_other, loser_payload)
    else:
        wait_for_absent(source_b)
        wait_for_absent(source_b_other)
        wait_for_bytes(source_a, loser_payload)
        wait_for_bytes(source_a_other, loser_payload)

    wait_for_counts(launcher, destination_name, (0, 0))
    wait_for_counts(launcher, source_a_name, (0, 0))
    wait_for_counts(launcher, source_b_name, (0, 0))

    scenario = "existing" if existing_destination else "absent"
    print(
        "OK rename-same-destination-race "
        f"scenario={scenario} winner={winner} loser={loser} "
        f"loser_errno={errno.EBUSY} "
        f"elapsed_a_ms={elapsed['A'] * 1000.0:.3f} "
        f"elapsed_b_ms={elapsed['B'] * 1000.0:.3f} "
        "winner_payload_only=1 loser_source_preserved=1 ownership_leaks=0"
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    launcher_a = FODMount(str(root))
    launcher_b = FODMount(str(root))
    launcher_a.init_schema()

    with tempfile.TemporaryDirectory(prefix="fod-rename-own-") as temp_dir:
        temp = Path(temp_dir)
        mount_a = temp / "mount-a"
        mount_b = temp / "mount-b"
        barrier_dir = temp / "rename-barrier"
        barrier_dir.mkdir()

        previous_barrier = os.environ.get(BARRIER_ENV)
        os.environ[BARRIER_ENV] = str(barrier_dir)

        try:
            launcher_a.start(str(mount_a), log_prefix="/tmp/fod-rename-own-a")
            launcher_b.start(str(mount_b), log_prefix="/tmp/fod-rename-own-b")

            destination_writer_is_fenced(launcher_a, mount_a, mount_b)
            source_writer_is_fenced(launcher_a, mount_a, mount_b)
            hardlink_target_writer_is_fenced(launcher_a, mount_a, mount_b)
            same_destination_race(
                launcher_a,
                mount_a,
                mount_b,
                barrier_dir,
                existing_destination=False,
            )
            same_destination_race(
                launcher_a,
                mount_a,
                mount_b,
                barrier_dir,
                existing_destination=True,
            )

            print("OK rename-write-ownership protected_cases=5")
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            disarm_rename_barrier(barrier_dir)
            launcher_b.stop()
            launcher_a.stop()

            if previous_barrier is None:
                os.environ.pop(BARRIER_ENV, None)
            else:
                os.environ[BARRIER_ENV] = previous_barrier


if __name__ == "__main__":
    main()
