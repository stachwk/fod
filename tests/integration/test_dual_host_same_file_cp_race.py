#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import hashlib
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tests.integration.fod_mount import FODMount

SOURCE_SIZE = 64 * 1024 * 1024
COPY_TIMEOUT_SECONDS = 120.0
LEASE_CLEANUP_TIMEOUT_SECONDS = 10.0
READBACK_TIMEOUT_SECONDS = 15.0


@dataclass
class CpChild:
    pid: int
    stderr_fd: int
    stamp_fd: int


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(1024 * 1024):
            h.update(chunk)
    return h.hexdigest()


def write_source(path: Path, label: str) -> str:
    seed = hashlib.sha256(
        f"FOD dual-host same-file cp race:{label}".encode()
    ).digest()
    chunk = seed * ((1024 * 1024) // len(seed))
    left = SOURCE_SIZE
    with path.open("wb") as f:
        while left:
            part = chunk[: min(len(chunk), left)]
            f.write(part)
            left -= len(part)
        f.flush()
        os.fsync(f.fileno())
    return sha256_file(path)


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def psql_scalar(launcher: FODMount, sql: str) -> str:
    env = os.environ.copy()
    env["PGPASSWORD"] = launcher.postgres_password
    host = env.get("FOD_PG_HOST") or env.get("POSTGRES_HOST") or "127.0.0.1"
    port = env.get("FOD_PG_PORT") or env.get("POSTGRES_PORT") or "5432"
    p = subprocess.run(
        [
            "psql", "-h", host, "-p", port,
            "-U", launcher.postgres_user,
            "-d", launcher.postgres_db,
            "-At", "-v", "ON_ERROR_STOP=1", "-c", sql,
        ],
        env=env, check=True, capture_output=True, text=True,
    )
    return p.stdout.strip()


def ownership_counts(launcher: FODMount, name: str) -> tuple[int, int]:
    raw = psql_scalar(
        launcher,
        f"""
        SELECT
          (SELECT COUNT(*)
           FROM fod.destination_write_leases
           WHERE parent_key = 0 AND name = {sql_quote(name)})::text || '|' ||
          (SELECT COUNT(*)
           FROM fod.file_write_leases fwl
           JOIN fod.files f ON f.id_file = fwl.file_id
           WHERE f.id_directory IS NULL AND f.name = {sql_quote(name)})::text
        """,
    )
    a, b = raw.split("|", 1)
    return int(a), int(b)


def wait_for_ownership_cleanup(launcher: FODMount, name: str) -> None:
    deadline = time.monotonic() + LEASE_CLEANUP_TIMEOUT_SECONDS
    last = (-1, -1)
    while time.monotonic() < deadline:
        last = ownership_counts(launcher, name)
        if last == (0, 0):
            return
        time.sleep(0.05)
    raise AssertionError(
        f"ownership leak: destination={last[0]} file={last[1]}"
    )


def spawn_stopped_cp(cp_bin: str, source: Path, destination: Path, pgid: int | None) -> CpChild:
    err_r, err_w = os.pipe()
    stamp_r, stamp_w = os.pipe()
    pid = os.fork()

    if pid == 0:
        try:
            os.close(err_r)
            os.close(stamp_r)

            if pgid is None:
                os.setpgid(0, 0)
            else:
                os.setpgid(0, pgid)

            os.dup2(err_w, 2)
            os.close(err_w)

            # Both wrappers stop at exactly the same pre-exec barrier.
            os.kill(os.getpid(), signal.SIGSTOP)

            resumed = time.monotonic_ns()
            os.write(stamp_w, f"{resumed}\n".encode("ascii"))
            os.close(stamp_w)

            env = os.environ.copy()
            env["LC_ALL"] = "C"
            env["LANG"] = "C"
            os.execve(
                cp_bin,
                [
                    cp_bin,
                    "--reflink=never",
                    "--sparse=never",
                    str(source),
                    str(destination),
                ],
                env,
            )
        except BaseException as exc:
            try:
                os.write(2, f"cp wrapper failed: {exc!r}\n".encode())
            except OSError:
                pass
            os._exit(127)

    os.close(err_w)
    os.close(stamp_w)
    return CpChild(pid, err_r, stamp_r)


def wait_stopped(child: CpChild) -> None:
    pid, status = os.waitpid(child.pid, os.WUNTRACED)
    if pid != child.pid or not os.WIFSTOPPED(status) or os.WSTOPSIG(status) != signal.SIGSTOP:
        raise AssertionError(
            f"child did not reach SIGSTOP barrier: pid={pid} status={status}"
        )


def wait_children(children: list[CpChild]) -> dict[int, int]:
    deadline = time.monotonic() + COPY_TIMEOUT_SECONDS
    pending = {c.pid for c in children}
    result: dict[int, int] = {}

    while pending and time.monotonic() < deadline:
        for pid in list(pending):
            waited, status = os.waitpid(pid, os.WNOHANG)
            if waited == 0:
                continue
            pending.remove(pid)
            if os.WIFEXITED(status):
                result[pid] = os.WEXITSTATUS(status)
            elif os.WIFSIGNALED(status):
                result[pid] = 128 + os.WTERMSIG(status)
            else:
                raise AssertionError(f"unexpected child status pid={pid} status={status}")
        if pending:
            time.sleep(0.01)

    if pending:
        for pid in pending:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for pid in pending:
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass
        raise AssertionError(f"cp race timeout; pending={sorted(pending)}")
    return result


def read_fd(fd: int) -> str:
    data = bytearray()
    try:
        while True:
            chunk = os.read(fd, 65536)
            if not chunk:
                break
            data.extend(chunk)
    finally:
        os.close(fd)
    return data.decode(errors="replace")


def read_stamp(fd: int) -> int:
    raw = read_fd(fd).strip()
    if not raw:
        raise AssertionError("missing resume timestamp")
    return int(raw)


def wait_for_hash(path: Path, expected: str) -> str:
    deadline = time.monotonic() + READBACK_TIMEOUT_SECONDS
    last = ""
    while time.monotonic() < deadline:
        try:
            last = sha256_file(path)
        except FileNotFoundError:
            last = ""
        if last == expected:
            return last
        time.sleep(0.05)
    raise AssertionError(
        f"hash did not converge path={path} expected={expected} got={last}"
    )


def main() -> None:
    cp_bin = shutil.which("cp")
    if cp_bin is None:
        raise RuntimeError("cp binary not found")

    suffix = uuid.uuid4().hex[:12]
    name = f"dual-host-cp-race-{suffix}.bin"

    launcher_a = FODMount(str(ROOT))
    launcher_b = FODMount(str(ROOT))
    launcher_a.init_schema()

    child_a: CpChild | None = None
    child_b: CpChild | None = None
    children_reaped = False

    with (
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-dual-src-a-{suffix}.") as src_a_dir,
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-dual-src-b-{suffix}.") as src_b_dir,
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-dual-mnt-a-{suffix}.") as mnt_a_dir,
        tempfile.TemporaryDirectory(prefix=f"/tmp/fod-dual-mnt-b-{suffix}.") as mnt_b_dir,
    ):
        source_a = Path(src_a_dir) / "same.bin"
        source_b = Path(src_b_dir) / "same.bin"
        mount_a = Path(mnt_a_dir)
        mount_b = Path(mnt_b_dir)

        hash_a = write_source(source_a, "A")
        hash_b = write_source(source_b, "B")
        if hash_a == hash_b:
            raise AssertionError(f"source SHA unexpectedly equal: {hash_a}")

        launcher_a.start(str(mount_a), log_prefix="/tmp/fod-dual-cp-a")
        launcher_b.start(str(mount_b), log_prefix="/tmp/fod-dual-cp-b")

        destination_a = mount_a / name
        destination_b = mount_b / name

        try:
            if destination_a.exists() or destination_b.exists():
                raise AssertionError("destination exists before race")

            child_a = spawn_stopped_cp(cp_bin, source_a, destination_a, None)
            wait_stopped(child_a)
            pgid = child_a.pid

            child_b = spawn_stopped_cp(cp_bin, source_b, destination_b, pgid)
            wait_stopped(child_b)

            # One kernel call wakes both stopped wrappers simultaneously.
            release_ns = time.monotonic_ns()
            os.killpg(pgid, signal.SIGCONT)

            exit_codes = wait_children([child_a, child_b])
            children_reaped = True
            resume_a = read_stamp(child_a.stamp_fd)
            resume_b = read_stamp(child_b.stamp_fd)

            stderr_a = read_fd(child_a.stderr_fd)
            stderr_b = read_fd(child_b.stderr_fd)

            rc_a = exit_codes[child_a.pid]
            rc_b = exit_codes[child_b.pid]

            winners = [x for x, rc in (("A", rc_a), ("B", rc_b)) if rc == 0]
            losers = [x for x, rc in (("A", rc_a), ("B", rc_b)) if rc != 0]
            if len(winners) != 1 or len(losers) != 1:
                raise AssertionError(
                    f"expected one winner and one loser: "
                    f"rc_a={rc_a} rc_b={rc_b} "
                    f"stderr_a={stderr_a!r} stderr_b={stderr_b!r}"
                )

            loser = losers[0]
            loser_stderr = stderr_a if loser == "A" else stderr_b
            if "Device or resource busy" in loser_stderr:
                loser_error = "EBUSY"
            elif "Input/output error" in loser_stderr:
                loser_error = "EIO"
            else:
                loser_error = "OTHER"

            skew_us = abs(resume_a - resume_b) / 1000.0
            first_resume_us = (min(resume_a, resume_b) - release_ns) / 1000.0

            print(
                "RACE-EVIDENCE dual-host-same-file-cp-race "
                f"winner={winners[0]} loser={loser} "
                f"rc_a={rc_a} rc_b={rc_b} "
                f"loser_error={loser_error} "
                f"start_skew_us={skew_us:.3f} "
                f"first_resume_after_release_us={first_resume_us:.3f}"
            )

            winner_hash = hash_a if winners[0] == "A" else hash_b
            loser_hash = hash_b if winners[0] == "A" else hash_a

            final_a = wait_for_hash(destination_a, winner_hash)
            final_b = wait_for_hash(destination_b, winner_hash)

            if destination_a.stat().st_size != SOURCE_SIZE:
                raise AssertionError("destination A size mismatch")
            if destination_b.stat().st_size != SOURCE_SIZE:
                raise AssertionError("destination B size mismatch")

            wait_for_ownership_cleanup(launcher_a, name)

            print(
                "OK dual-host-same-file-cp-race "
                f"source_size={SOURCE_SIZE} "
                f"source_sha_a={hash_a} "
                f"source_sha_b={hash_b} "
                f"winner_sha={winner_hash} loser_sha={loser_hash} "
                f"winner={winners[0]} loser={loser} "
                f"rc_a={rc_a} rc_b={rc_b} "
                f"loser_error={loser_error} "
                "release=single_killpg_SIGCONT "
                f"start_skew_us={skew_us:.3f} "
                f"first_resume_after_release_us={first_resume_us:.3f} "
                f"final_sha_a={final_a} final_sha_b={final_b} "
                "ownership_leaks=0"
            )

            if loser_error != "EBUSY":
                raise AssertionError(
                    "race data integrity survived, but loser errno contract is wrong: "
                    f"loser_error={loser_error} stderr={loser_stderr!r}"
                )
        except BaseException:
            print("\n=== MOUNT A LOG ===")
            launcher_a._dump_log()
            print("\n=== MOUNT B LOG ===")
            launcher_b._dump_log()
            raise
        finally:
            for child in (child_a, child_b):
                if child is None:
                    continue
                if not children_reaped:
                    try:
                        os.kill(child.pid, signal.SIGCONT)
                    except ProcessLookupError:
                        pass
                    try:
                        os.kill(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    try:
                        os.waitpid(child.pid, os.WNOHANG)
                    except ChildProcessError:
                        pass
                for fd in (child.stderr_fd, child.stamp_fd):
                    try:
                        os.close(fd)
                    except OSError:
                        pass

            try:
                destination_a.unlink()
            except OSError:
                pass

            launcher_b.stop()
            launcher_a.stop()


if __name__ == "__main__":
    main()
