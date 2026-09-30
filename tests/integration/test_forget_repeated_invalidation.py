#!/usr/bin/env python
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from __future__ import annotations

import os
import re
import tempfile
import time
import uuid
from pathlib import Path

from fod_mount import FODMount


# Test korzysta z tego samego hooka co pojedyncza invalidacja, ale prosi
# runtime o wykonanie wielu cykli w ramach jednego montowania.
CONTROL_DIR_ENV = "FOD_TEST_FORGET_INVALIDATE_DIR"
CONTROL_NAME_ENV = "FOD_TEST_FORGET_INVALIDATE_NAME"
CONTROL_COUNT_ENV = "FOD_TEST_FORGET_INVALIDATE_COUNT"
CYCLES_ENV = "FOD_TEST_FORGET_CYCLES"
RSS_MAX_GROWTH_ENV = "FOD_TEST_FORGET_RSS_MAX_GROWTH_BYTES"

# Domyslnie test pozostaje malym gate C1. Target convergence ustawia wiecej cykli.
DEFAULT_CYCLES = 10
DEFAULT_RSS_MAX_GROWTH_BYTES = 16 * 1024 * 1024
WAIT_SECONDS = 10.0

# Rozszerzony profil pozwala sprawdzic konwergencje lookup_refs, map inode/path i RSS.
# Starszy test pojedynczej invalidacji nadal pasuje do prefiksu tego komunikatu.
FORGET_RE = re.compile(
    r"FOD forget profile: ino=(\d+) nlookup=(\d+) remaining=(\d+) "
    r"evicted=(true|false) lookup_ref_inodes=(\d+) lookup_ref_total=(\d+) "
    r"inode_to_path=(\d+) path_to_inode=(\d+) process_rss_bytes=(\d+)"
)


def positive_int_env(name: str, default: int) -> int:
    """Czyta dodatnia wartosc calkowita z env."""
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    try:
        value = int(raw)
    except ValueError as exc:
        raise AssertionError(f"{name} must be an integer, got {raw!r}") from exc
    if value <= 0:
        raise AssertionError(f"{name} must be greater than zero, got {value}")
    return value


def wait_for_control(control_dir: Path, cycle: int) -> None:
    """Czeka na zakonczenie konkretnego cyklu invalidacji."""
    deadline = time.monotonic() + WAIT_SECONDS
    done = control_dir / f"done.{cycle}"
    error = control_dir / "error"

    while time.monotonic() < deadline:
        if error.exists():
            raise AssertionError(error.read_text(encoding="utf-8").strip())
        if done.exists():
            return
        time.sleep(0.01)

    raise AssertionError(f"forget invalidation hook did not complete cycle={cycle}")


def wait_for_forget(
    log_file: Path,
    target_ino: int,
    expected_count: int,
) -> tuple[int, int, int, bool, int, int, int, int, int]:
    """Czeka na oczekiwany profil FORGET dla badanego inode."""
    deadline = time.monotonic() + WAIT_SECONDS
    last = ""

    while time.monotonic() < deadline:
        try:
            last = log_file.read_text(encoding="utf-8")
        except FileNotFoundError:
            last = ""

        # Filtrujemy po inode, aby obce zdarzenia FORGET nie zaliczyly cyklu.
        matching = [
            match
            for match in FORGET_RE.findall(last)
            if int(match[0]) == target_ino
        ]
        if len(matching) >= expected_count:
            (
                ino,
                nlookup,
                remaining,
                evicted,
                lookup_ref_inodes,
                lookup_ref_total,
                inode_to_path,
                path_to_inode,
                process_rss_bytes,
            ) = matching[expected_count - 1]
            return (
                int(ino),
                int(nlookup),
                int(remaining),
                evicted == "true",
                int(lookup_ref_inodes),
                int(lookup_ref_total),
                int(inode_to_path),
                int(path_to_inode),
                int(process_rss_bytes),
            )
        time.sleep(0.01)

    raise AssertionError(
        "kernel did not emit expected repeated FUSE FORGET "
        f"ino={target_ino} expected_count={expected_count}\n"
        + last[-4000:]
    )


def main() -> None:
    root = Path(__file__).resolve().parents[2]
    fuse_bin = os.environ.get("FOD_RUST_FUSE_BIN", "").strip()
    if not fuse_bin:
        raise AssertionError(
            "FOD_RUST_FUSE_BIN must point to fod-rust-fuse built with "
            "--features integration-test-hooks"
        )

    cycles = positive_int_env(CYCLES_ENV, DEFAULT_CYCLES)
    rss_max_growth_bytes = positive_int_env(
        RSS_MAX_GROWTH_ENV,
        DEFAULT_RSS_MAX_GROWTH_BYTES,
    )

    with tempfile.TemporaryDirectory(prefix="fod-forget-repeated-") as temp_dir:
        temp = Path(temp_dir)
        mountpoint = temp / "mount"
        control_dir = temp / "control"
        control_dir.mkdir()

        launcher = FODMount(str(root))
        launcher.init_schema()

        name = f"forget-repeated-{uuid.uuid4().hex}.bin"
        payload = b"FOD repeated deterministic forget invalidation\n"

        # Przywracamy srodowisko po tescie, aby nie zmieniac kolejnych gate'ow.
        previous = {
            CONTROL_DIR_ENV: os.environ.get(CONTROL_DIR_ENV),
            CONTROL_NAME_ENV: os.environ.get(CONTROL_NAME_ENV),
            CONTROL_COUNT_ENV: os.environ.get(CONTROL_COUNT_ENV),
            "FOD_PROFILE_METADATA_CACHE": os.environ.get("FOD_PROFILE_METADATA_CACHE"),
            "FOD_READDIR_REGISTER_PATHS": os.environ.get("FOD_READDIR_REGISTER_PATHS"),
            "FOD_READDIR_BATCH_METADATA": os.environ.get("FOD_READDIR_BATCH_METADATA"),
            "FOD_LOG_LEVEL": os.environ.get("FOD_LOG_LEVEL"),
        }

        os.environ[CONTROL_DIR_ENV] = str(control_dir)
        os.environ[CONTROL_NAME_ENV] = name
        os.environ[CONTROL_COUNT_ENV] = str(cycles)
        os.environ["FOD_PROFILE_METADATA_CACHE"] = "1"
        os.environ["FOD_READDIR_REGISTER_PATHS"] = "0"
        os.environ["FOD_READDIR_BATCH_METADATA"] = "1"
        os.environ["FOD_LOG_LEVEL"] = "info"

        baseline_lookup_ref_inodes: int | None = None
        baseline_lookup_ref_total: int | None = None
        baseline_inode_to_path: int | None = None
        baseline_path_to_inode: int | None = None
        rss_samples: list[int] = []

        try:
            launcher.start(
                str(mountpoint),
                log_prefix="/tmp/fod-forget-repeated-invalidation",
            )

            target = mountpoint / name
            target.write_bytes(payload)
            initial = target.stat()
            target_ino = initial.st_ino

            if target.read_bytes() != payload:
                raise AssertionError("pre-invalidation payload mismatch")

            # Kazdy cykl wymusza invalidacje dentry, oczekuje FORGET, a potem
            # robi relookup. Punkt po FORGET musi wracac do stalego baseline.
            for cycle in range(1, cycles + 1):
                (control_dir / f"trigger.{cycle}").write_text(
                    "invalidate\n",
                    encoding="utf-8",
                )
                wait_for_control(control_dir, cycle)

                if launcher.config is None:
                    raise AssertionError("mount config unavailable")

                (
                    forget_ino,
                    nlookup,
                    remaining,
                    evicted,
                    lookup_ref_inodes,
                    lookup_ref_total,
                    inode_to_path,
                    path_to_inode,
                    process_rss_bytes,
                ) = wait_for_forget(
                    launcher.config.log_file,
                    target_ino,
                    cycle,
                )

                if forget_ino != target_ino:
                    raise AssertionError(
                        f"forget inode mismatch expected={target_ino} got={forget_ino}"
                    )
                if nlookup <= 0:
                    raise AssertionError(
                        f"invalid forget nlookup={nlookup} ino={forget_ino} cycle={cycle}"
                    )
                if remaining != 0 or not evicted:
                    raise AssertionError(
                        "forget did not retire cached inode path: "
                        f"cycle={cycle} ino={forget_ino} "
                        f"remaining={remaining} evicted={evicted}"
                    )
                # Liczniki lookup sa globalne dla calego mountu. Pierwszy cykl
                # ustala baseline po usunieciu badanego inode; kolejne cykle
                # musza wracac do tego samego stanu, ale niekoniecznie do zera.
                if baseline_lookup_ref_inodes is None:
                    baseline_lookup_ref_inodes = lookup_ref_inodes
                    baseline_lookup_ref_total = lookup_ref_total
                elif (
                    lookup_ref_inodes != baseline_lookup_ref_inodes
                    or lookup_ref_total != baseline_lookup_ref_total
                ):
                    raise AssertionError(
                        "lookup refs did not converge to baseline after forget: "
                        f"cycle={cycle} lookup_ref_inodes={lookup_ref_inodes} "
                        f"lookup_ref_total={lookup_ref_total} "
                        f"baseline_lookup_ref_inodes={baseline_lookup_ref_inodes} "
                        f"baseline_lookup_ref_total={baseline_lookup_ref_total}"
                    )

                if baseline_inode_to_path is None:
                    baseline_inode_to_path = inode_to_path
                    baseline_path_to_inode = path_to_inode
                elif (
                    inode_to_path != baseline_inode_to_path
                    or path_to_inode != baseline_path_to_inode
                ):
                    raise AssertionError(
                        "inode/path cache did not converge to baseline: "
                        f"cycle={cycle} inode_to_path={inode_to_path} "
                        f"path_to_inode={path_to_inode} "
                        f"baseline_inode_to_path={baseline_inode_to_path} "
                        f"baseline_path_to_inode={baseline_path_to_inode}"
                    )

                if inode_to_path != path_to_inode:
                    raise AssertionError(
                        "inode/path cache sizes diverged: "
                        f"cycle={cycle} inode_to_path={inode_to_path} "
                        f"path_to_inode={path_to_inode}"
                    )

                if process_rss_bytes == 0:
                    raise AssertionError(
                        f"RSS sample unavailable cycle={cycle}"
                    )
                rss_samples.append(process_rss_bytes)

                after = target.stat()
                if after.st_ino != target_ino:
                    raise AssertionError(
                        "stable inode changed across repeated forget/relookup: "
                        f"cycle={cycle} before={target_ino} after={after.st_ino}"
                    )
                if target.read_bytes() != payload:
                    raise AssertionError(
                        f"post-invalidation payload mismatch cycle={cycle}"
                    )

            # Pomijamy poczatkowy warmup alokatora przy ocenie stabilizacji RSS.
            warmup_samples = min(max(cycles // 10, 1), 50)
            rss_baseline = rss_samples[warmup_samples - 1]
            rss_tail = rss_samples[warmup_samples:]
            rss_peak = max(rss_tail) if rss_tail else rss_baseline
            rss_end = rss_samples[-1]
            rss_growth = max(0, rss_peak - rss_baseline)

            if rss_growth > rss_max_growth_bytes:
                raise AssertionError(
                    "RSS did not stabilize during forget churn: "
                    f"cycles={cycles} baseline={rss_baseline} peak={rss_peak} "
                    f"end={rss_end} growth={rss_growth} "
                    f"limit={rss_max_growth_bytes}"
                )

            target.unlink()
            print(
                "OK forget-repeated-invalidation "
                f"cycles={cycles} ino={target_ino} "
                "remaining=0 evicted=1 "
                f"lookup_ref_inodes={baseline_lookup_ref_inodes} "
                f"lookup_ref_total={baseline_lookup_ref_total} "
                f"inode_to_path={baseline_inode_to_path} "
                f"path_to_inode={baseline_path_to_inode} "
                f"rss_baseline={rss_baseline} rss_end={rss_end} "
                f"rss_peak={rss_peak} rss_growth={rss_growth} "
                "stable_inode=1 relookup=1"
            )
        except Exception:
            launcher._dump_log()
            raise
        finally:
            launcher.stop()
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value


if __name__ == "__main__":
    main()
