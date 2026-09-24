#!/usr/bin/env python
# -*- coding: utf-8 -*-

import argparse
import sys
from pathlib import Path


def print_err(message):
    print(f"stderr: {message}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(
        description="FOD 3.4.30: align FODMount PostgreSQL env with Rust FOD_PG_* overrides."
    )
    parser.add_argument("--repo", default=str(Path.home() / "git" / "fod"))
    parser.add_argument("--verbose", type=int, choices=(0, 1), default=0)
    args = parser.parse_args()

    repo = Path(args.repo).expanduser().resolve()
    version_path = repo / "fod_version.txt"
    target_path = repo / "tests/integration/fod_mount.py"

    for path in (version_path, target_path):
        if not path.is_file():
            print_err(f"missing required file: {path}")
            return 2

    if version_path.read_text(encoding="utf-8").strip() != "3.4.30":
        print_err("patch requires FOD 3.4.30")
        return 3

    text = target_path.read_text(encoding="utf-8")

    old_init = """        self.root = Path(root)
        self.postgres_db = os.environ.get("POSTGRES_DB", "foddbname")
        self.postgres_user = os.environ.get("POSTGRES_USER", "foduser")
        self.postgres_password = os.environ.get("POSTGRES_PASSWORD", "cichosza")
"""

    new_init = """        self.root = Path(root)
        self.postgres_db = (
            os.environ.get("FOD_PG_DBNAME")
            or os.environ.get("POSTGRES_DB")
            or "foddbname"
        )
        self.postgres_user = (
            os.environ.get("FOD_PG_USER")
            or os.environ.get("POSTGRES_USER")
            or "foduser"
        )
        self.postgres_password = (
            os.environ.get("FOD_PG_PASSWORD")
            or os.environ.get("POSTGRES_PASSWORD")
            or "cichosza"
        )
"""

    old_env = """    def _runtime_env(self) -> dict[str, str]:
        env = os.environ.copy()
        env["POSTGRES_DB"] = self.postgres_db
        env["POSTGRES_USER"] = self.postgres_user
        env["POSTGRES_PASSWORD"] = self.postgres_password
        # Test harness musi uzywac tego samego configu dla mkfs i mountu.
        env["FOD_CONFIG"] = str(self._config_path())
        return env
"""

    new_env = """    def _runtime_env(self) -> dict[str, str]:
        env = os.environ.copy()

        # Python-side helpers historically use POSTGRES_* while the Rust
        # runtime resolves database overrides from FOD_PG_*. Keep both views
        # on the same selected test endpoint.
        env["POSTGRES_DB"] = self.postgres_db
        env["POSTGRES_USER"] = self.postgres_user
        env["POSTGRES_PASSWORD"] = self.postgres_password
        env["FOD_PG_DBNAME"] = self.postgres_db
        env["FOD_PG_USER"] = self.postgres_user
        env["FOD_PG_PASSWORD"] = self.postgres_password

        postgres_host = env.get("FOD_PG_HOST") or env.get("POSTGRES_HOST")
        if postgres_host:
            env["FOD_PG_HOST"] = postgres_host

        postgres_port = env.get("FOD_PG_PORT") or env.get("POSTGRES_PORT")
        if postgres_port:
            env["FOD_PG_PORT"] = postgres_port

        # Test harness musi uzywac tego samego configu dla mkfs i mountu.
        # Database endpoint values above intentionally override [database].
        env["FOD_CONFIG"] = str(self._config_path())
        return env
"""

    if "env[\"FOD_PG_DBNAME\"] = self.postgres_db" in text:
        print_err("FODMount FOD_PG_* mapping already present")
        return 4

    if text.count(old_init) != 1:
        print_err(f"FODMount init anchor count={text.count(old_init)}, expected=1")
        return 5

    if text.count(old_env) != 1:
        print_err(f"FODMount runtime env anchor count={text.count(old_env)}, expected=1")
        return 6

    text = text.replace(old_init, new_init, 1)
    text = text.replace(old_env, new_env, 1)
    target_path.write_text(text, encoding="utf-8")

    if args.verbose:
        print_err(
            "FODMount now maps the selected PostgreSQL test endpoint to native Rust FOD_PG_* overrides"
        )

    print("OK: FODMount uses FOD_PG_* / POSTGRES_* selected database identity")
    print("OK: Rust mkfs/bootstrap receive FOD_PG_DBNAME/FOD_PG_USER/FOD_PG_PASSWORD")
    print("OK: version remains 3.4.30")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
