#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Probe mounted fallback behavior for FUSE protocol additions 7.34-7.39.
# This observation test does not enable any FUSE capability.

from __future__ import annotations

import ctypes
import datetime as dt
import errno
import json
import os
import platform
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount


AT_FDCWD = -100
AT_SYMLINK_NOFOLLOW = 0x100
STATX_BASIC_STATS = 0x07FF
STATX_BTIME = 0x0800
STATX_REQUEST_MASK = STATX_BASIC_STATS | STATX_BTIME
STATX_BUFFER_SIZE = 256
UNSUPPORTED_ERRNOS = {
    errno.ENOSYS,
    errno.EOPNOTSUPP,
    errno.EINVAL,
}


def errno_payload(value: int) -> dict[str, Any]:
    return {
        "errno": value,
        "errno_name": errno.errorcode.get(value, "UNKNOWN"),
        "message": os.strerror(value),
    }


def git_head() -> str:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    return completed.stdout.strip() if completed.returncode == 0 else "unknown"


def probe_syncfs(mountpoint: Path) -> dict[str, Any]:
    libc = ctypes.CDLL(None, use_errno=True)
    syncfs = getattr(libc, "syncfs", None)
    if syncfs is None:
        return {
            "status": "client_unavailable",
            "accepted": True,
            "detail": "libc does not expose syncfs",
        }

    syncfs.argtypes = [ctypes.c_int]
    syncfs.restype = ctypes.c_int

    fd = os.open(
        mountpoint,
        os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_CLOEXEC", 0),
    )
    try:
        ctypes.set_errno(0)
        result = syncfs(fd)
        if result == 0:
            return {
                "status": "success",
                "accepted": True,
            }

        err = ctypes.get_errno()
        payload = errno_payload(err)
        payload.update(
            {
                "status": "unsupported" if err in UNSUPPORTED_ERRNOS else "error",
                "accepted": err in UNSUPPORTED_ERRNOS,
            }
        )
        return payload
    finally:
        os.close(fd)


def probe_tmpfile(mountpoint: Path) -> dict[str, Any]:
    o_tmpfile = getattr(os, "O_TMPFILE", None)
    if o_tmpfile is None:
        return {
            "status": "client_unavailable",
            "accepted": True,
            "detail": "Python/os does not expose O_TMPFILE",
        }

    flags = o_tmpfile | os.O_RDWR | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open(mountpoint, flags, 0o600)
    except OSError as exc:
        err = exc.errno or 0
        payload = errno_payload(err)
        payload.update(
            {
                "status": "unsupported" if err in UNSUPPORTED_ERRNOS else "error",
                "accepted": err in UNSUPPORTED_ERRNOS,
            }
        )
        return payload

    try:
        os.write(fd, b"unexpected tmpfile payload")
        return {
            "status": "unexpected_success",
            "accepted": False,
            "detail": (
                "O_TMPFILE succeeded even though FOD has no approved unnamed-"
                "object lifetime, link, cleanup, and replay contract"
            ),
        }
    finally:
        os.close(fd)


def unpack_u16(buffer: bytes, offset: int) -> int:
    return int.from_bytes(buffer[offset : offset + 2], sys.byteorder, signed=False)


def unpack_u32(buffer: bytes, offset: int) -> int:
    return int.from_bytes(buffer[offset : offset + 4], sys.byteorder, signed=False)


def unpack_u64(buffer: bytes, offset: int) -> int:
    return int.from_bytes(buffer[offset : offset + 8], sys.byteorder, signed=False)


def probe_statx(path: Path) -> dict[str, Any]:
    libc = ctypes.CDLL(None, use_errno=True)
    statx_fn = getattr(libc, "statx", None)
    if statx_fn is None:
        return {
            "status": "client_unavailable",
            "accepted": True,
            "detail": "libc does not expose statx",
        }

    statx_fn.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_uint,
        ctypes.c_void_p,
    ]
    statx_fn.restype = ctypes.c_int

    output = ctypes.create_string_buffer(STATX_BUFFER_SIZE)
    ctypes.set_errno(0)
    result = statx_fn(
        AT_FDCWD,
        os.fsencode(path),
        AT_SYMLINK_NOFOLLOW,
        STATX_REQUEST_MASK,
        ctypes.byref(output),
    )

    if result != 0:
        err = ctypes.get_errno()
        payload = errno_payload(err)
        payload.update(
            {
                "status": "unsupported" if err in UNSUPPORTED_ERRNOS else "error",
                "accepted": err in UNSUPPORTED_ERRNOS,
            }
        )
        return payload

    raw = output.raw
    observed = {
        "mask": unpack_u32(raw, 0),
        "block_size": unpack_u32(raw, 4),
        "nlink": unpack_u32(raw, 16),
        "uid": unpack_u32(raw, 20),
        "gid": unpack_u32(raw, 24),
        "mode": unpack_u16(raw, 28),
        "inode": unpack_u64(raw, 32),
        "size": unpack_u64(raw, 40),
        "blocks": unpack_u64(raw, 48),
        "attributes_mask": unpack_u64(raw, 56),
    }

    normal = os.stat(path, follow_symlinks=False)
    expected = {
        "mode": normal.st_mode,
        "inode": normal.st_ino,
        "size": normal.st_size,
        "uid": normal.st_uid,
        "gid": normal.st_gid,
        "nlink": normal.st_nlink,
    }

    mismatches: dict[str, dict[str, int]] = {}
    for field in ("inode", "size", "uid", "gid", "nlink"):
        if observed[field] != expected[field]:
            mismatches[field] = {
                "statx": observed[field],
                "stat": expected[field],
            }

    if (observed["mode"] & 0o177777) != (expected["mode"] & 0o177777):
        mismatches["mode"] = {
            "statx": observed["mode"],
            "stat": expected["mode"],
        }

    return {
        "status": "success" if not mismatches else "semantic_mismatch",
        "accepted": not mismatches,
        "observed": observed,
        "expected": expected,
        "mismatches": mismatches,
    }


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def main() -> int:
    launcher = FODMount(str(ROOT))
    launcher.init_schema()

    suffix = uuid.uuid4().hex[:8]
    payload: dict[str, Any] = {
        "schema": 1,
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repo_head": git_head(),
        "kernel_release": platform.release(),
        "machine": platform.machine(),
        "fuse_protocol_scope": "7.34-7.39 fallback probes",
        "capabilities_enabled_by_test": [],
    }

    with tempfile.TemporaryDirectory(
        prefix=f"/tmp/fod-post731-probe-{suffix}."
    ) as tmpdir:
        mountpoint = Path(tmpdir)
        launcher.start(str(mountpoint), log_prefix="fod-post731-probe")
        try:
            probe_dir = mountpoint / f"post731_{suffix}"
            probe_dir.mkdir()
            known_file = probe_dir / "known.bin"
            known_data = b"FOD post-7.31 capability fallback probe\n"
            known_file.write_bytes(known_data)

            payload["syncfs"] = probe_syncfs(mountpoint)
            payload["tmpfile"] = probe_tmpfile(probe_dir)
            payload["statx"] = probe_statx(known_file)

            namespace_entries = sorted(item.name for item in probe_dir.iterdir())
            payload["namespace_entries"] = namespace_entries
            payload["namespace_clean"] = namespace_entries == ["known.bin"]
            payload["known_file_bytes_ok"] = known_file.read_bytes() == known_data

            if launcher.config is not None and launcher.config.log_file.exists():
                log_lines = launcher.config.log_file.read_text(
                    encoding="utf-8",
                    errors="replace",
                ).splitlines()
                payload["mount_log_tail"] = log_lines[-40:]
        finally:
            launcher.stop()

    accepted = [
        bool(payload.get("syncfs", {}).get("accepted")),
        bool(payload.get("tmpfile", {}).get("accepted")),
        bool(payload.get("statx", {}).get("accepted")),
        bool(payload.get("namespace_clean")),
        bool(payload.get("known_file_bytes_ok")),
    ]
    payload["result"] = "PASS" if all(accepted) else "FAIL"

    output_path_value = os.environ.get("FOD_POST731_PROBE_OUTPUT", "").strip()
    if output_path_value:
        output_path = Path(output_path_value).expanduser()
        payload["output_path"] = str(output_path)
        write_json(output_path, payload)

    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0 if payload["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
