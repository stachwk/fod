#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""TX03I: PostgreSQL-led recovery after SIGKILL of a FUSE writer.

Uses the established crash-convergence integration scenario, including
live remote unlink, a distinct replacement inode, survivor maintenance,
and open-lease cleanup. Never targets an existing non-test mount.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path


REQUIRED = (
    "postgres_authority=1",
    "lease_survived_crash=1",
    "remote_prune=1",
    "orphan_purge=1",
    "old_generation_removed=1",
    "replacement_isolated=1",
    "hidden_entry_leaks=0",
    "cleanup=1",
    "crash_signal=9",
)


def run_case(root: Path, natural: bool) -> None:
    env = os.environ.copy()
    env["FOD_TEST_FORCE_SESSION_EXPIRY"] = "0" if natural else "1"
    env["PYTHONUNBUFFERED"] = "1"
    scenario = root / "tests/integration/test_unlink_open_writer_crash_convergence.py"
    # The underlying integration test creates two new independent mounts and
    # SIGKILLs only the FUSE PID tied to its own PostgreSQL client session.
    process = subprocess.run(
        [sys.executable, str(scenario)],
        cwd=root,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=180 if natural else 90,
        check=False,
    )
    output = process.stdout
    if process.returncode or "OK unlink-open-writer-crash-convergence " not in output:
        print(output, end="", file=sys.stderr, flush=True)
        raise AssertionError(
            f"TX03I underlying crash convergence failed rc={process.returncode}"
        )
    result = next(
        line for line in output.splitlines()
        if line.startswith("OK unlink-open-writer-crash-convergence ")
    )
    missing = [marker for marker in REQUIRED if marker not in result]
    expected_expiry = (
        "natural_session_expiry=1" if natural else "forced_session_expiry=1"
    )
    if expected_expiry not in result:
        missing.append(expected_expiry)
    old = re.search(r"\bold_file_id=(\d+)\b", result)
    new = re.search(r"\breplacement_file_id=(\d+)\b", result)
    if not old or not new or old.group(1) == new.group(1):
        missing.append("distinct old/replacement file_id")
    if missing:
        raise AssertionError(f"TX03I missing assertions={missing}; result={result}")
    print(
        f"TX03I_CRASH_CONVERGENCE=PASS mode={'natural' if natural else 'forced'} "
        "postgres_authority=1 lease_survived_crash=1 old_generation_removed=1 "
        "replacement_isolated=1 cleanup=1",
        flush=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--natural-ttl", action="store_true",
        help="Wait for real PostgreSQL lease TTL rather than expiring test session",
    )
    args = parser.parse_args()
    dbname = os.environ.get("FOD_PG_DBNAME") or os.environ.get("POSTGRES_DB")
    if not dbname or not dbname.endswith("_test"):
        raise RuntimeError("TX03I requires an explicit dedicated *_test database")
    root = Path(__file__).resolve().parents[2]
    run_case(root, natural=args.natural_ttl)


if __name__ == "__main__":
    main()
