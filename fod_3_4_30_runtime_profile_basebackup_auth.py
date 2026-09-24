#!/usr/bin/env python
# -*- coding: utf-8 -*-

import argparse
import sys
from pathlib import Path


def print_err(message):
    print(f"stderr: {message}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(
        description="FOD 3.4.30: make runtime-profile pg_basebackup authentication non-interactive."
    )
    parser.add_argument("--repo", default=str(Path.home() / "git" / "fod"))
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()

    repo = Path(args.repo).expanduser().resolve()
    version_path = repo / "fod_version.txt"
    test_path = repo / "tests/integration/test_runtime_profile.py"

    for path in (version_path, test_path):
        if not path.is_file():
            print_err(f"missing required file: {path}")
            return 2

    if version_path.read_text(encoding="utf-8").strip() != "3.4.30":
        print_err("patch requires FOD 3.4.30")
        return 3

    text = test_path.read_text(encoding="utf-8")

    if '"PGPASSWORD=' in text and "pg_basebackup -w" in text:
        print_err("pg_basebackup non-interactive authentication patch already applied")
        return 4

    docker_sig_anchor = """def _docker(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["docker", *args],
        capture_output=True,
        text=True,
        check=False,
    )
"""

    docker_sig_replacement = """def _docker(
    args: list[str],
    *,
    check: bool = True,
    timeout_seconds: float | None = None,
) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        ["docker", *args],
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout_seconds,
    )
"""

    basebackup_anchor = """            _docker(
                [
                    "exec",
                    "-u",
                    "postgres",
                    "fod-postgres",
                    "sh",
                    "-lc",
                    f"mkdir -p {standby_backup_path} && "
                    "pg_basebackup -h 127.0.0.1 -p 5432 -U "
                    f"{dsn['user']} -D {standby_backup_path} -Fp -Xs -P",
                ]
            )
"""

    basebackup_replacement = """            _docker(
                [
                    "exec",
                    "-u",
                    "postgres",
                    "-e",
                    f"PGPASSWORD={dsn['password']}",
                    "fod-postgres",
                    "sh",
                    "-lc",
                    f"mkdir -p {standby_backup_path} && "
                    "pg_basebackup -w -h 127.0.0.1 -p 5432 -U "
                    f"{dsn['user']} -D {standby_backup_path} -Fp -Xs -P",
                ],
                timeout_seconds=120.0,
            )
"""

    if text.count(docker_sig_anchor) != 1:
        print_err(f"_docker anchor count={text.count(docker_sig_anchor)}, expected=1")
        return 5

    if text.count(basebackup_anchor) != 1:
        print_err(f"pg_basebackup anchor count={text.count(basebackup_anchor)}, expected=1")
        return 6

    text = text.replace(docker_sig_anchor, docker_sig_replacement, 1)
    text = text.replace(basebackup_anchor, basebackup_replacement, 1)
    test_path.write_text(text, encoding="utf-8")

    if args.verbose:
        print_err(
            "runtime-profile pg_basebackup now receives PGPASSWORD, uses -w, "
            "and has a 120s subprocess timeout"
        )

    print("OK: runtime-profile pg_basebackup authentication is non-interactive")
    print("OK: docker helper supports an optional timeout")
    print("OK: version remains 3.4.30")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
