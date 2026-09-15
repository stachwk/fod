#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import ctypes
import errno
import hashlib
import json
import os
import platform
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import psycopg2

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

FALLOC_FL_KEEP_SIZE = 0x01
FALLOC_FL_PUNCH_HOLE = 0x02
FALLOC_FL_ZERO_RANGE = 0x10

INITIAL_BYTES = 64 * 1024
RANGE_OFFSET = 32 * 1024
RANGE_LENGTH = 32 * 1024
EXTEND_OFFSET = 96 * 1024
EXTEND_LENGTH = 32 * 1024

libc = ctypes.CDLL(None, use_errno=True)
if not hasattr(libc, "fallocate"):
    raise RuntimeError("libc does not expose fallocate()")
libc.fallocate.argtypes = [
    ctypes.c_int,
    ctypes.c_int,
    ctypes.c_longlong,
    ctypes.c_longlong,
]
libc.fallocate.restype = ctypes.c_int


@dataclass(frozen=True)
class CallResult:
    rc: int
    errno: int
    errno_name: str


@dataclass(frozen=True)
class FileSnapshot:
    size: int
    blocks_512: int
    mtime_ns: int
    ctime_ns: int
    sha256: str


@dataclass(frozen=True)
class DbSnapshot:
    file_size: int
    object_file_size: int
    data_object_id: int
    block_rows: int
    payload_bytes: int


@dataclass(frozen=True)
class StatfsSnapshot:
    block_size: int
    blocks: int
    free_blocks: int
    available_blocks: int


@dataclass(frozen=True)
class Case:
    name: str
    mode: int
    offset: int
    length: int
    call_kind: str = "raw"


def deterministic_payload() -> bytes:
    return bytes(((index * 17) + 3) % 251 for index in range(INITIAL_BYTES))


def command_output(command: list[str]) -> str:
    try:
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            check=False,
            timeout=5,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return "unavailable"
    text = (result.stdout or result.stderr).strip()
    return text if text else f"rc={result.returncode}"


def database_connection(launcher: FODMount):
    connection = psycopg2.connect(
        host=os.environ.get("POSTGRES_HOST", "127.0.0.1"),
        port=os.environ.get("POSTGRES_PORT", "5432"),
        dbname=launcher.postgres_db,
        user=launcher.postgres_user,
        password=launcher.postgres_password,
    )
    connection.autocommit = True
    return connection


def db_snapshot(connection, name: str) -> DbSnapshot:
    with connection.cursor() as cursor:
        cursor.execute(
            """
            SELECT
                f.size,
                o.file_size,
                f.data_object_id,
                COUNT(b.id_block)::bigint,
                COALESCE(SUM(octet_length(b.data)), 0)::bigint
            FROM fod.files AS f
            JOIN fod.data_objects AS o
              ON o.id_data_object = f.data_object_id
            LEFT JOIN fod.data_blocks AS b
              ON b.data_object_id = f.data_object_id
            WHERE f.id_directory IS NULL
              AND f.name = %s
            GROUP BY f.size, o.file_size, f.data_object_id
            """,
            (name,),
        )
        row = cursor.fetchone()
    if row is None:
        raise AssertionError(f"missing database row for {name}")
    return DbSnapshot(*(int(value) for value in row))


def file_snapshot(path: Path) -> FileSnapshot:
    stat = path.stat()
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    return FileSnapshot(
        size=int(stat.st_size),
        blocks_512=int(stat.st_blocks),
        mtime_ns=int(stat.st_mtime_ns),
        ctime_ns=int(stat.st_ctime_ns),
        sha256=digest,
    )


def statfs_snapshot(path: Path) -> StatfsSnapshot:
    stat = os.statvfs(path)
    return StatfsSnapshot(
        block_size=int(stat.f_frsize),
        blocks=int(stat.f_blocks),
        free_blocks=int(stat.f_bfree),
        available_blocks=int(stat.f_bavail),
    )


def raw_fallocate(fd: int, mode: int, offset: int, length: int) -> CallResult:
    ctypes.set_errno(0)
    rc = int(
        libc.fallocate(
            fd,
            mode,
            ctypes.c_longlong(offset),
            ctypes.c_longlong(length),
        )
    )
    error_number = int(ctypes.get_errno()) if rc != 0 else 0
    return CallResult(
        rc=rc,
        errno=error_number,
        errno_name=errno.errorcode.get(
            error_number,
            "OK" if error_number == 0 else "UNKNOWN",
        ),
    )


def posix_fallocate(fd: int, offset: int, length: int) -> CallResult:
    try:
        os.posix_fallocate(fd, offset, length)
    except OSError as error:
        error_number = int(error.errno or 0)
        return CallResult(
            rc=-1,
            errno=error_number,
            errno_name=errno.errorcode.get(error_number, "UNKNOWN"),
        )
    return CallResult(rc=0, errno=0, errno_name="OK")


def compatibility_lines(log_text: str) -> list[str]:
    prefixes = (
        "FOD FUSE compatibility:",
        "FOD FUSE negotiated:",
    )
    return [
        line.strip()
        for line in log_text.splitlines()
        if any(prefix in line for prefix in prefixes)
    ]


def failed_call_must_not_mutate(
    case: Case,
    before_file: FileSnapshot,
    after_file: FileSnapshot,
    before_db: DbSnapshot,
    after_db: DbSnapshot,
) -> None:
    stable_file = (
        before_file.size,
        before_file.blocks_512,
        before_file.sha256,
    )
    current_file = (
        after_file.size,
        after_file.blocks_512,
        after_file.sha256,
    )
    if current_file != stable_file:
        raise AssertionError(
            f"{case.name}: failed fallocate mutated file state "
            f"before={stable_file} after={current_file}"
        )

    stable_db = (
        before_db.file_size,
        before_db.object_file_size,
        before_db.data_object_id,
        before_db.block_rows,
        before_db.payload_bytes,
    )
    current_db = (
        after_db.file_size,
        after_db.object_file_size,
        after_db.data_object_id,
        after_db.block_rows,
        after_db.payload_bytes,
    )
    if current_db != stable_db:
        raise AssertionError(
            f"{case.name}: failed fallocate mutated database state "
            f"before={stable_db} after={current_db}"
        )


def validate_successful_mode_zero(
    case: Case,
    before_content: bytes,
    after_content: bytes,
) -> None:
    expected_size = max(len(before_content), case.offset + case.length)
    if len(after_content) != expected_size:
        raise AssertionError(
            f"{case.name}: successful mode=0 size={len(after_content)} "
            f"expected={expected_size}"
        )
    if after_content[: len(before_content)] != before_content:
        raise AssertionError(f"{case.name}: successful mode=0 changed existing payload")
    if any(after_content[len(before_content) :]):
        raise AssertionError(f"{case.name}: successful mode=0 extension is not zero-filled")


def validate_successful_keep_size(
    case: Case,
    before_content: bytes,
    after_content: bytes,
) -> None:
    if after_content != before_content:
        raise AssertionError(f"{case.name}: KEEP_SIZE changed visible file contents")


def validate_successful_zeroing(
    case: Case,
    before_content: bytes,
    after_content: bytes,
) -> None:
    if len(after_content) != len(before_content):
        raise AssertionError(f"{case.name}: zeroing mode changed file size")
    start = case.offset
    end = min(len(before_content), case.offset + case.length)
    expected = before_content[:start] + (b"\x00" * (end - start)) + before_content[end:]
    if after_content != expected:
        raise AssertionError(f"{case.name}: zeroing mode produced unexpected visible contents")


def validate_success(
    case: Case,
    before_content: bytes,
    after_content: bytes,
) -> None:
    if case.call_kind == "posix":
        validate_successful_mode_zero(case, before_content, after_content)
        return
    if case.mode == 0:
        validate_successful_mode_zero(case, before_content, after_content)
    elif case.mode == FALLOC_FL_KEEP_SIZE:
        validate_successful_keep_size(case, before_content, after_content)
    elif case.mode in {
        FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
        FALLOC_FL_ZERO_RANGE,
    }:
        validate_successful_zeroing(case, before_content, after_content)
    elif case.mode == FALLOC_FL_PUNCH_HOLE:
        raise AssertionError(
            f"{case.name}: PUNCH_HOLE without KEEP_SIZE unexpectedly succeeded"
        )


def run_case(
    case: Case,
    suffix: str,
    launcher_template: FODMount,
) -> tuple[dict[str, Any], dict[str, Any]]:
    name = f"fallocate-f1-{case.name}-{suffix}.bin"
    launcher = FODMount(str(ROOT))
    launcher.postgres_db = launcher_template.postgres_db
    launcher.postgres_user = launcher_template.postgres_user
    launcher.postgres_password = launcher_template.postgres_password

    with tempfile.TemporaryDirectory(
        prefix=f"/tmp/fod-fallocate-f1-{case.name}."
    ) as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(
            tmpdir,
            log_prefix=f"/tmp/fod-fallocate-f1-{case.name}",
        )
        try:
            path = mountpoint / name
            baseline_content = deterministic_payload()
            descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_TRUNC, 0o644)
            try:
                written = os.write(descriptor, baseline_content)
                if written != len(baseline_content):
                    raise AssertionError(
                        f"{case.name}: short baseline write "
                        f"{written}/{len(baseline_content)}"
                    )
                os.fsync(descriptor)
            finally:
                os.close(descriptor)

            connection = database_connection(launcher)
            try:
                before_file = file_snapshot(path)
                before_db = db_snapshot(connection, name)
                before_statfs = statfs_snapshot(mountpoint)

                descriptor = os.open(path, os.O_RDWR)
                try:
                    if case.call_kind == "raw":
                        call = raw_fallocate(
                            descriptor,
                            case.mode,
                            case.offset,
                            case.length,
                        )
                    else:
                        call = posix_fallocate(
                            descriptor,
                            case.offset,
                            case.length,
                        )
                    if call.rc == 0:
                        os.fsync(descriptor)
                finally:
                    os.close(descriptor)

                time.sleep(0.05)
                after_content = path.read_bytes()
                after_file = file_snapshot(path)
                after_db = db_snapshot(connection, name)
                after_statfs = statfs_snapshot(mountpoint)
            finally:
                connection.close()

            if call.rc != 0:
                failed_call_must_not_mutate(
                    case,
                    before_file,
                    after_file,
                    before_db,
                    after_db,
                )
            else:
                validate_success(case, baseline_content, after_content)

            log_text = (
                launcher.config.log_file.read_text(
                    encoding="utf-8",
                    errors="replace",
                )
                if launcher.config is not None
                else ""
            )
            result = {
                "case": case.name,
                "call_kind": case.call_kind,
                "mode": case.mode,
                "offset": case.offset,
                "length": case.length,
                "call": asdict(call),
                "fuser_default_fallocate_logged": (
                    "[Not Implemented] fallocate(" in log_text
                ),
                "before_file": asdict(before_file),
                "after_file": asdict(after_file),
                "before_db": asdict(before_db),
                "after_db": asdict(after_db),
                "before_statfs": asdict(before_statfs),
                "after_statfs": asdict(after_statfs),
            }
            remount_reference = {
                "name": name,
                "file": asdict(after_file),
                "db": asdict(after_db),
            }
            result["compatibility_lines"] = compatibility_lines(log_text)
            return result, remount_reference
        finally:
            launcher.stop()


def verify_after_remount(
    launcher_template: FODMount,
    references: list[dict[str, Any]],
) -> None:
    launcher = FODMount(str(ROOT))
    launcher.postgres_db = launcher_template.postgres_db
    launcher.postgres_user = launcher_template.postgres_user
    launcher.postgres_password = launcher_template.postgres_password

    with tempfile.TemporaryDirectory(prefix="/tmp/fod-fallocate-f1-remount.") as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(tmpdir, log_prefix="/tmp/fod-fallocate-f1-remount")
        try:
            connection = database_connection(launcher)
            try:
                for reference in references:
                    path = mountpoint / reference["name"]
                    current_file = file_snapshot(path)
                    current_db = db_snapshot(connection, reference["name"])

                    expected_file = reference["file"]
                    if (
                        current_file.size != expected_file["size"]
                        or current_file.blocks_512 != expected_file["blocks_512"]
                        or current_file.sha256 != expected_file["sha256"]
                    ):
                        raise AssertionError(
                            f"{reference['name']}: remount file state changed "
                            f"expected={expected_file} current={asdict(current_file)}"
                        )

                    expected_db = reference["db"]
                    if asdict(current_db) != expected_db:
                        raise AssertionError(
                            f"{reference['name']}: remount DB state changed "
                            f"expected={expected_db} current={asdict(current_db)}"
                        )

                for reference in references:
                    path = mountpoint / reference["name"]
                    if path.exists():
                        path.unlink()
            finally:
                connection.close()
        finally:
            launcher.stop()


def main() -> int:
    if sys.platform != "linux":
        raise RuntimeError("F1 fallocate contract baseline is Linux-specific")

    template = FODMount(str(ROOT))
    template.init_schema()

    suffix = uuid.uuid4().hex[:12]
    cases = [
        Case(
            name="mode0_extend",
            mode=0,
            offset=EXTEND_OFFSET,
            length=EXTEND_LENGTH,
        ),
        Case(
            name="keep_size",
            mode=FALLOC_FL_KEEP_SIZE,
            offset=EXTEND_OFFSET,
            length=EXTEND_LENGTH,
        ),
        Case(
            name="punch_hole_keep_size",
            mode=FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
            offset=RANGE_OFFSET,
            length=RANGE_LENGTH,
        ),
        Case(
            name="zero_range",
            mode=FALLOC_FL_ZERO_RANGE,
            offset=RANGE_OFFSET,
            length=RANGE_LENGTH,
        ),
        Case(
            name="invalid_punch_without_keep_size",
            mode=FALLOC_FL_PUNCH_HOLE,
            offset=RANGE_OFFSET,
            length=RANGE_LENGTH,
        ),
        Case(
            name="legacy_posix_fallocate",
            mode=0,
            offset=EXTEND_OFFSET,
            length=EXTEND_LENGTH,
            call_kind="posix",
        ),
    ]

    results: list[dict[str, Any]] = []
    references: list[dict[str, Any]] = []

    for case in cases:
        result, reference = run_case(case, suffix, template)
        results.append(result)
        references.append(reference)

    by_case = {result["case"]: result for result in results}
    unsupported = {
        "mode0_extend",
        "keep_size",
        "zero_range",
        "invalid_punch_without_keep_size",
    }
    for name in unsupported:
        result = by_case[name]
        if result["call"]["rc"] == 0:
            raise AssertionError(f"{name}: unsupported raw fallocate unexpectedly succeeded")
        if result["call"]["errno"] not in {errno.ENOTSUP, errno.EOPNOTSUPP}:
            raise AssertionError(
                f"{name}: expected ENOTSUP/EOPNOTSUPP, got {result['call']}"
            )

    punch = by_case["punch_hole_keep_size"]
    if punch["call"]["rc"] != 0:
        raise AssertionError(
            f"PUNCH_HOLE|KEEP_SIZE must succeed, got {punch['call']}"
        )
    if punch["fuser_default_fallocate_logged"]:
        raise AssertionError("PUNCH_HOLE|KEEP_SIZE reached default fuser callback")
    if punch["after_file"]["size"] != punch["before_file"]["size"]:
        raise AssertionError("PUNCH_HOLE|KEEP_SIZE changed logical file size")
    if punch["after_db"]["payload_bytes"] >= punch["before_db"]["payload_bytes"]:
        raise AssertionError("aligned PUNCH_HOLE did not release persisted payload")
    if punch["after_file"]["blocks_512"] >= punch["before_file"]["blocks_512"]:
        raise AssertionError("aligned PUNCH_HOLE did not reduce st_blocks")

    legacy = by_case["legacy_posix_fallocate"]
    if legacy["call"]["rc"] != 0:
        raise AssertionError(f"legacy posix_fallocate control failed: {legacy['call']}")

    verify_after_remount(template, references)

    environment = {
        "kernel": platform.release(),
        "machine": platform.machine(),
        "fuse3_pkg_config": command_output(["pkg-config", "--modversion", "fuse3"]),
        "fusermount3": command_output(["fusermount3", "--version"]),
    }

    print("=== F1.2 FALLOCATE CONTRACT ===")
    print(json.dumps(environment, sort_keys=True))

    for result in results:
        compact = {
            "case": result["case"],
            "call_kind": result["call_kind"],
            "mode": result["mode"],
            "rc": result["call"]["rc"],
            "errno": result["call"]["errno"],
            "errno_name": result["call"]["errno_name"],
            "fuser_default_fallocate_logged": result[
                "fuser_default_fallocate_logged"
            ],
            "size_before": result["before_file"]["size"],
            "size_after": result["after_file"]["size"],
            "blocks_before": result["before_file"]["blocks_512"],
            "blocks_after": result["after_file"]["blocks_512"],
            "db_blocks_before": result["before_db"]["block_rows"],
            "db_blocks_after": result["after_db"]["block_rows"],
            "db_payload_before": result["before_db"]["payload_bytes"],
            "db_payload_after": result["after_db"]["payload_bytes"],
        }
        print("F1_CASE " + json.dumps(compact, sort_keys=True))

    compatibility = next(
        (
            line
            for result in results
            for line in result["compatibility_lines"]
            if "FOD FUSE compatibility:" in line
        ),
        "unavailable",
    )
    negotiated = next(
        (
            line
            for result in results
            for line in result["compatibility_lines"]
            if "FOD FUSE negotiated:" in line
        ),
        "unavailable",
    )
    print(f"F1_COMPATIBILITY {compatibility}")
    print(f"F1_NEGOTIATED {negotiated}")
    print(
        "OK fallocate-contract-f1-2 "
        f"cases={len(results)} remount_verified=1 "
        "punch_hole_keep_size=1 unsupported_explicit=1 remount_verified=1"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
