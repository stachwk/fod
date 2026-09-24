#!/usr/bin/env python
# -*- coding: utf-8 -*-

import argparse
import sys
from pathlib import Path


def print_err(message):
    print(f"stderr: {message}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=str(Path.home() / "git" / "fod"))
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()

    repo = Path(args.repo).expanduser().resolve()
    version_path = repo / "fod_version.txt"
    test_path = repo / "tests/integration/test_runtime_profile.py"

    if not version_path.is_file() or not test_path.is_file():
        print_err("required FOD files are missing")
        return 2

    if version_path.read_text(encoding="utf-8").strip() != "3.4.30":
        print_err("patch requires FOD 3.4.30")
        return 3

    text = test_path.read_text(encoding="utf-8")

    if "def _selected_primary_dsn()" in text:
        print_err("patch already applied")
        return 4

    helper_anchor = """def _docker(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
"""

    helper_replacement = """def _selected_primary_dsn() -> dict[str, str]:
    dsn, _ = load_dsn_from_config(ROOT)

    # Make can select a PostgreSQL endpoint independently of fod_config.ini.
    # Prefer FOD_PG_* and then POSTGRES_* over config-file defaults.
    overrides = {
        "host": ("FOD_PG_HOST", "POSTGRES_HOST"),
        "port": ("FOD_PG_PORT", "POSTGRES_PORT"),
        "dbname": ("FOD_PG_DBNAME", "POSTGRES_DB"),
        "user": ("FOD_PG_USER", "POSTGRES_USER"),
        "password": ("FOD_PG_PASSWORD", "POSTGRES_PASSWORD"),
    }

    for dsn_key, env_keys in overrides.items():
        for env_key in env_keys:
            value = os.environ.get(env_key)
            if value is not None:
                dsn[dsn_key] = value
                break

    return dsn


def _docker(args: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
"""

    write_sig_anchor = """def _write_recovery_config(source_config: Path, destination_dir: Path, port: int) -> Path:
"""
    write_sig_replacement = """def _write_recovery_config(
    source_config: Path,
    destination_dir: Path,
    port: int,
    primary_dsn: dict[str, str],
) -> Path:
"""

    recovery_db_anchor = """    parser["database"]["host"] = "127.0.0.1"
    parser["database"]["port"] = str(port)
    for key in ("sslmode", "sslrootcert", "sslcert", "sslkey"):
"""

    recovery_db_replacement = """    parser["database"]["host"] = "127.0.0.1"
    parser["database"]["port"] = str(port)
    parser["database"]["dbname"] = primary_dsn["dbname"]
    parser["database"]["user"] = primary_dsn["user"]
    parser["database"]["password"] = primary_dsn["password"]
    for key in ("sslmode", "sslrootcert", "sslcert", "sslkey"):
"""

    main_dsn_anchor = """    dsn, _ = load_dsn_from_config(ROOT)
    config_path = ROOT / "fod_config.ini"
"""
    main_dsn_replacement = """    dsn = _selected_primary_dsn()
    config_path = ROOT / "fod_config.ini"
"""

    recovery_call_anchor = """            recovery_config_path = _write_recovery_config(
                config_path,
                Path(standby_config_dir.name),
                recovery_port,
            )
"""

    recovery_call_replacement = """            recovery_config_path = _write_recovery_config(
                config_path,
                Path(standby_config_dir.name),
                recovery_port,
                dsn,
            )
"""

    anchors = (
        ("helper", helper_anchor),
        ("recovery signature", write_sig_anchor),
        ("recovery database", recovery_db_anchor),
        ("main DSN", main_dsn_anchor),
        ("recovery call", recovery_call_anchor),
    )

    for name, anchor in anchors:
        count = text.count(anchor)
        if count != 1:
            print_err(f"{name} anchor count={count}, expected=1")
            return 5

    text = text.replace(helper_anchor, helper_replacement, 1)
    text = text.replace(write_sig_anchor, write_sig_replacement, 1)
    text = text.replace(recovery_db_anchor, recovery_db_replacement, 1)
    text = text.replace(main_dsn_anchor, main_dsn_replacement, 1)
    text = text.replace(recovery_call_anchor, recovery_call_replacement, 1)

    test_path.write_text(text, encoding="utf-8")

    if args.verbose:
        print_err(
            "runtime-profile now uses the selected PostgreSQL database "
            "for primary and recovery paths"
        )

    print("OK runtime-profile selected DSN patch")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
